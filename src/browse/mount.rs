//! Mounts the Browse tab (T-41a-011, T-40a-034): the anime and manga source lists behind the shell, and the pages the tab
//! opens (the language filter, global search, the repository links). The screen registry calls [`browse`]; it returns
//! `None` before the app data is open, so the sample page of `entry` stays until then.
//!
//! The Sources pages list what the source directory of their kind has. The tab has no extension or migrate pages; a
//! repository link adds its sources, and the language filter decides what shows.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use gtk::prelude::*;
use kuro_domain::source::model::Source;
use kuro_prefs::pref::Pref;
use kuro_prefs::source::SourcePrefs;
use tokio::sync::watch;

use super::glue;
use super::route::{BrowsePage, BrowseRoute};
use kuro_platform::kind::Kind;
use super::shell::{BrowseShell, ShellDeps};
use super::sources_page::{SourcesNav, SourcesSnapshot};
use crate::screens::Built;

thread_local! {
    /// The mounted tab, kept alive with the window.
    static SHELL: RefCell<Option<Rc<BrowseShell>>> = const { RefCell::new(None) };
}

/// Fetches every repository link again and reloads the sources (the Repositories page's refresh), off the GTK thread;
/// the button stays off until it ends.
fn refresh_repositories(button: &gtk::Button) {
    use crate::pref_model::stores_backend::{Ops as _, RepoOps};
    let links = SourcePrefs::new(crate::data::app().base.store().clone()).manga_extension_repos;
    let ops = RepoOps::new(links);
    crate::ui::set_refreshing(button, true);
    let button = button.clone();
    gtk::glib::spawn_future_local(async move {
        let _ = gtk::gio::spawn_blocking(move || ops.refresh()).await;
        crate::ui::set_refreshing(&button, false);
    });
}

/// Opens the one repository list, which holds the links of both kinds. The empty Sources pages and the header menu
/// both lead here.
pub fn open_repos() {
    crate::ui::open("settings_extension_repos");
}

thread_local! {
    static ADD_REQUEST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Opens the repository list with its add dialog already open: the first-run button, where the user came to add one.
pub fn open_repos_to_add() {
    ADD_REQUEST.with(|r| r.set(true));
    open_repos();
}

/// Whether the repository list should open its add dialog, handed out once.
pub fn take_add_request() -> bool {
    ADD_REQUEST.with(std::cell::Cell::take)
}

/// The installed sources of a kind, as the Sources filter and the Sources page list them.
pub fn installed_sources(kind: Kind) -> Vec<Source> {
    super::directory::current(kind).sources().into_iter().map(|entry| entry.source).collect()
}

/// What a Sources page shows of its preferences (T-40b-015) and the sources the directory of its kind lists. The pin and
/// hide sets are cut, so the lists get none of them: the language filter alone decides which sources show. The extension
/// host's anime packages ship icon files; the repository sources draw theirs from `recipe_sources::icon_path`.
fn sources_snapshot(kind: Kind, prefs: &SourcePrefs) -> SourcesSnapshot {
    let listed = super::directory::current(kind).sources();
    let home_urls = listed.iter().filter_map(|entry| Some((entry.source.id, entry.home_url.clone()?))).collect();
    SourcesSnapshot {
        sources: listed.into_iter().map(|entry| entry.source).collect(),
        enabled_languages: prefs.enabled_languages.get(),
        disabled: BTreeSet::new(),
        last_used: crate::misc::diet_defaults::BROWSE_LAST_USED_SOURCE,
        icon_files: BTreeMap::new(),
        home_urls,
    }
}

/// Calls `show` each time a load of the repository sources ends, until `widget` is gone: the open Sources tabs and the
/// language filter read their lists again, so a source that finished loading appears without reopening anything.
fn follow_loads(widget: &gtk::Box, show: impl Fn() + 'static) {
    let weak = widget.downgrade();
    glue::follow_changes(crate::recipe_sources::changes(), move |_| {
        if weak.upgrade().is_none() {
            return false;
        }
        show();
        true
    });
}

/// One preference's change check for [`glue::poll_pref_changes`]: true once when the value changed since the last tick.
fn watch_check<T: kuro_prefs::values::PrefType + 'static>(pref: &Pref<T>) -> Box<dyn Fn() -> bool> {
    let watch = pref.watch();
    Box::new(move || watch.try_changed().is_some())
}

/// Opens the screen of a route with its request parked for the screen that builds (`pending`).
fn open_route(route: &BrowseRoute) {
    super::pending::open(route.clone());
}

/// The Sources page of `kind` over `snapshot`, which reads the directory and the prefs. It reads again each time the page
/// is shown: the tab is built before the repository sources load, so the first read lists none of them. A change to the
/// languages also reaches the open page (T-40b-015), and so does each finished load.
fn sources_page(kind: Kind, prefs: &Rc<SourcePrefs>, ready: watch::Receiver<bool>, snapshot: Rc<dyn Fn(&SourcePrefs) -> SourcesSnapshot>) -> super::sources_page::SourcesPage {
    let nav = SourcesNav {
        open_listing: Rc::new(move |source_id, query| open_route(&BrowseRoute::BrowseSource { kind, source_id, query, migrating: false })),
        open_url: Rc::new(|url: &str| crate::ui::open_link(url, None)),
        kind,
    };
    let page = super::pages::sources(nav, ready);
    let show = {
        let (page, prefs) = (page.clone(), Rc::clone(prefs));
        move || page.update(snapshot(&prefs))
    };
    show();
    let shown = show.clone();
    page.widget().connect_map(move |_| shown());
    let checks: Vec<Box<dyn Fn() -> bool>> = vec![watch_check(&prefs.enabled_languages)];
    let alive = {
        let widget = page.widget().downgrade();
        move || widget.upgrade().is_some()
    };
    glue::poll_pref_changes(checks, alive, show.clone());
    follow_loads(page.widget(), show);
    page
}

/// Builds the tab. `None` before the app data is open.
pub fn browse() -> Option<Built> {
    if !crate::data::is_installed() {
        return None;
    }
    let store = crate::data::app().base.store().clone();
    let prefs = Rc::new(SourcePrefs::new(store));
    // the Sources tabs show a spinner while the repository sources load
    let ready = crate::recipe_sources::ready();

    // Sources: the lists wait for the repository sources; the language filter decides which sources show
    let kind_page = |kind: Kind| sources_page(kind, &prefs, ready.clone(), Rc::new(move |prefs: &SourcePrefs| sources_snapshot(kind, prefs)));
    let (anime_sources, manga_sources) = (kind_page(Kind::Anime), kind_page(Kind::Manga));

    let pages: Vec<(BrowsePage, gtk::Widget)> = vec![
        (BrowsePage::AnimeSources, anime_sources.widget().clone().upcast()),
        (BrowsePage::MangaSources, manga_sources.widget().clone().upcast()),
    ];
    let (lang_prefs, toggle_prefs) = (Rc::clone(&prefs), Rc::clone(&prefs));
    let deps = ShellDeps {
        pages,
        on_route: Rc::new(|route| open_route(&route)),
        languages: Rc::new(move |kind| {
            let groups = super::lang_pages::sources_filter_groups(&installed_sources(kind), &lang_prefs.enabled_languages.get());
            groups.into_iter().map(|group| (group.lang, group.title, group.enabled)).collect()
        }),
        on_language: Rc::new(move |lang, on| {
            let mut set = toggle_prefs.enabled_languages.get();
            if set.contains(&lang) != on {
                kuro_domain::source::toggles::toggle_language(&mut set, &lang);
                if let Err(error) = toggle_prefs.enabled_languages.set(set) {
                    tracing::warn!(%error, "the source languages could not be stored");
                }
            }
        }),
        on_repos: Rc::new(|_kind| open_repos()),
        on_refresh: Rc::new(refresh_repositories),
        on_leave: Rc::new(crate::ui::pop),
    };
    let shell = Rc::new(BrowseShell::new(deps));
    let built = Built { content: shell.root.clone().upcast(), end: shell.end.clone() };
    SHELL.with(|s| *s.borrow_mut() = Some(shell));
    Some(built)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_i18n::{MsgKey, tr};

    /// The page is built before the sources start (main builds the tabs first): a source that arrives afterwards is
    /// listed once the page is shown, with no pref written in between.
    #[test]
    fn the_manga_sources_page_lists_sources_that_arrive_after_it_was_built() {
        crate::testing::with_display(|| {
            let prefs = Rc::new(SourcePrefs::new(kuro_prefs::store::PrefStore::in_memory()));
            let arrived: Rc<RefCell<Vec<Source>>> = Rc::default();
            let read = {
                let arrived = Rc::clone(&arrived);
                Rc::new(move |_: &SourcePrefs| SourcesSnapshot {
                    sources: arrived.borrow().clone(),
                    enabled_languages: BTreeSet::from(["en".to_owned()]),
                    last_used: -1,
                    ..SourcesSnapshot::default()
                })
            };
            let (_ready_tx, ready) = watch::channel(true);
            let page = sources_page(Kind::Manga, &prefs, ready, read);
            arrived.borrow_mut().push(Source::new(kuro_source::generate_id("Fixture", "en", 1), "Fixture", "en"));
            let window = gtk::Window::builder().default_width(600).default_height(600).child(page.widget()).build();
            window.present();
            let lists_it = || {
                let mut stack = vec![page.widget().clone().upcast::<gtk::Widget>()];
                while let Some(widget) = stack.pop() {
                    if widget.downcast_ref::<gtk::Label>().is_some_and(|l| l.label() == "Fixture") {
                        return true;
                    }
                    let mut child = widget.first_child();
                    while let Some(c) = child {
                        child = c.next_sibling();
                        stack.push(c);
                    }
                }
                false
            };
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while std::time::Instant::now() < deadline && !lists_it() {
                gtk::glib::MainContext::default().iteration(false);
            }
            assert!(lists_it(), "the source that arrived after the build is listed once the page is shown");
            window.close();
        });
    }

    /// The empty Sources page of a kind has one action, the repository links of that kind (the first-run step of the
    /// app flow).
    #[test]
    fn the_empty_sources_action_names_the_repository_links() {
        crate::testing::with_display(|| {
            for kind in [Kind::Manga, Kind::Anime] {
                let empty = super::super::sources_page::sources_empty();
                let action = empty.child().and_then(|row| row.first_child()).and_downcast::<gtk::Button>().expect("the one action");
                assert_eq!(action.label().as_deref(), Some(tr(MsgKey::label_extension_repos).as_str()), "{kind:?}");
            }
        });
    }
}
