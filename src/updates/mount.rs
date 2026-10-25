//! Mounts the History and Updates tabs (T-42a-021, T-42a-025): one screen per kind under an Anime | Manga switcher,
//! and the header actions. The screens are the live ones of [`history_screen`] and [`updates_screen`]; the pages call
//! [`history`] and [`updates`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::Box as GBox;
use kuro_platform::kind::Kind;
use gtk::glib;
use kuro_i18n::{MsgKey, tr};

use crate::screens::Built;
use crate::data;
use crate::misc::diet_defaults as diet;
use crate::updates::download_host;
use crate::updates::library_trigger::{self, Outcome};
use crate::updates::history_screen::HistoryScreen;
use crate::updates::qualities;
use crate::updates::storage::{self, StorageRow};
use crate::updates::storage_data;
use crate::updates::storage_screen::StorageScreen;
use crate::updates::updates_screen::UpdatesScreen;

#[cfg(test)]
/// The stack page names of the kind switcher; anime comes first.
const ANIME: &str = "anime";
const MANGA: &str = "manga";

thread_local! {
    /// The History screens, kept alive with the page like the Updates ones.
    static HISTORY: RefCell<Vec<Rc<HistoryScreen>>> = const { RefCell::new(Vec::new()) };
    /// The Updates screens; their widgets only hold weak references, so this keeps them alive.
    static UPDATES: RefCell<Vec<Rc<UpdatesScreen>>> = const { RefCell::new(Vec::new()) };
    /// The Storage screens of the last page built, with the tasks that feed them.
    static STORAGE: RefCell<Vec<(Rc<StorageScreen>, Vec<glib::JoinHandle<()>>)>> = const { RefCell::new(Vec::new()) };
}

fn kind_of(name: Option<&str>) -> Kind {
    if name == Some(MANGA) { Kind::Manga } else { Kind::Anime }
}

/// Which of the Updates refresh buttons (anime first, as the screens are built) belongs to `kind`.
fn refresh_index(kind: Kind) -> usize {
    usize::from(kind == Kind::Manga)
}

/// A tab with the Library's one-row header (docs/design/other-tabs.md): the Anime | Manga switch, the visible kind's
/// search (when it has one) and its actions; no page title. Returns the page and the stack of the two kinds.
fn tab_page(views: [Built; 2]) -> (GBox, gtk::Stack) {
    let page = crate::screens::kind_page(views, false);
    let stack = page.last_child().and_downcast::<gtk::Stack>().expect("the kinds stack follows the header");
    (page, stack)
}

/// The History page: both kinds, each with its search and its Clear history button in the header. `None` before the
/// app data is open.
pub fn history() -> Option<Built> {
    if !crate::data::is_installed() {
        return None;
    }
    let (anime, manga) = (HistoryScreen::new(Kind::Anime), HistoryScreen::new(Kind::Manga));
    let view = |screen: &Rc<HistoryScreen>| {
        let target = Rc::clone(screen);
        let clear = crate::ui::icon_button("user-trash-symbolic", &tr(MsgKey::pref_clear_history));
        clear.connect_clicked(move |_| target.ask_clear());
        let target = Rc::clone(screen);
        let refresh = crate::ui::refresh_button(&tr(MsgKey::action_webview_refresh));
        refresh.connect_clicked(move |button| {
            crate::ui::spin_once(button);
            target.reload();
        });
        let end = crate::ui::HeaderActions { search: Some(screen.search().clone()), refresh: Some(refresh.upcast()), extra: vec![clear.upcast()], ..Default::default() }.widgets();
        Built { content: screen.widget().clone().upcast(), end }
    };
    let (page, _) = tab_page([view(&anime), view(&manga)]);
    HISTORY.with(|h| *h.borrow_mut() = vec![anime, manga]);
    Some(Built { content: page.upcast(), end: vec![] })
}

/// The spec label of the toolbar button that starts a library update.
const UPDATE_LIBRARY: &str = "Update library";
/// The spec label of the toolbar button that opens the Upcoming page.
const VIEW_UPCOMING: &str = "View Upcoming Updates";

thread_local! {
    /// The kind whose Upcoming page the toolbar button last asked for; `screens::build("upcoming")` reads it.
    static UPCOMING_KIND: std::cell::Cell<Kind> = const { std::cell::Cell::new(Kind::Anime) };
}

/// The Upcoming page of the kind the Updates toolbar asked for (anime until it asks). `None` before the app data is open.
/// The page is built by `ui::open("upcoming")` through `screens::build`.
pub fn upcoming() -> Option<GBox> {
    crate::updates::upcoming::page::build(UPCOMING_KIND.with(|k| k.get()))
}

/// The Updates page: both kinds under the spec toolbar, each with a search that filters its feed by entry title (as History
/// and the Library have one). `None` before the app data is open.
/// The new-updates counters reset when the page is shown and when it is left (T-42b-020).
pub fn updates() -> Option<Built> {
    if !crate::data::is_installed() {
        return None;
    }
    // before the screens: their rows offer Show qualities only while a sheet is registered
    qualities::install_episode_options();
    let screens = [UpdatesScreen::new(Kind::Anime), UpdatesScreen::new(Kind::Manga)];
    let spec = crate::ui::screen("updates");
    let mut refresh: Vec<gtk::Button> = Vec::new();
    let views = screens.each_ref().map(|screen| {
        let kind = screen.kind();
        let mut actions = crate::ui::HeaderActions { search: Some(screen.search().clone()), ..Default::default() };
        for item in &spec.toolbar {
            if item.label == UPDATE_LIBRARY {
                let button = crate::ui::refresh_button(&item.label);
                button.connect_clicked(move |button| update_library(button, kind));
                refresh.push(button.clone());
                actions.refresh = Some(button.upcast());
            } else if item.label == VIEW_UPCOMING {
                // each kind has its own Upcoming page, as in the source (the anime tab opens the anime one)
                let button = crate::ui::icon_button(&item.icon, &tr(MsgKey::label_upcoming));
                button.connect_clicked(move |_| {
                    UPCOMING_KIND.with(|k| k.set(kind));
                    crate::ui::open("upcoming");
                });
                actions.extra.push(button.upcast());
            } else {
                let button = crate::ui::icon_button(&item.icon, &item.label);
                let label = item.label.clone();
                button.connect_clicked(move |_| {
                    crate::ui::resolve("updates", &label);
                });
                actions.extra.push(button.upcast());
            }
        }
        Built { content: screen.widget().clone().upcast(), end: actions.widgets() }
    });
    let (page, stack) = tab_page(views);
    page.connect_map(|_| reset_new_updates());
    page.connect_unmap(|_| reset_new_updates());
    if refresh.len() == 2 {
        // an empty feed offers the same pass as its kind's refresh button, which then spins
        for screen in &screens {
            let action = gtk::Button::with_label(&tr(MsgKey::action_update_library));
            let kind = screen.kind();
            let button = refresh[refresh_index(kind)].clone();
            action.connect_clicked(move |_| update_library(&button, kind));
            screen.set_empty_action(&action);
        }
        // F5 starts the same pass as the Refresh button of the visible kind (T-42b-042)
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, _| {
            if library_trigger::is_refresh_key(key) {
                let kind = kind_of(stack.visible_child_name().as_deref());
                update_library(&refresh[refresh_index(kind)], kind);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        page.add_controller(keys);
    }
    UPDATES.with(|u| *u.borrow_mut() = screens.into());
    Some(Built { content: page.upcast(), end: vec![] })
}

/// A header button with an icon and a word; `tip` is the longer tooltip.
/// Clears the two new-updates counters (T-42b-020).
fn reset_new_updates() {
    let library = &data::app().library;
    let _ = library.new_manga_updates_count.set(0);
    let _ = library.new_anime_updates_count.set(0);
}

/// The refresh button of `kind`: starts a library pass for it, shows the outcome on that kind's page, and spins
/// until the pass ends (T-42a-033). A refused start shows its toast and no spinner.
fn update_library(button: &gtk::Button, kind: Kind) {
    let outcome = library_trigger::trigger(kind);
    UPDATES.with(|u| {
        if let Some(screen) = u.borrow().iter().find(|screen| screen.kind() == kind) {
            screen.show_toast(&library_trigger::message(outcome));
        }
    });
    if outcome == Outcome::Started {
        spin_until_done(button, kind);
    }
}

/// Turns the button's arrow until the library pass of `kind` is no longer running.
fn spin_until_done(button: &gtk::Button, kind: Kind) {
    crate::ui::set_refreshing(button, true);
    let weak = button.downgrade();
    glib::timeout_add_local(Duration::from_millis(500), move || {
        let Some(button) = weak.upgrade() else { return glib::ControlFlow::Break };
        if library_trigger::running(kind) {
            return glib::ControlFlow::Continue;
        }
        crate::ui::set_refreshing(&button, false);
        glib::ControlFlow::Break
    });
}

/// The Storage page: one page per kind, with the shared kind header (switch and title search). It follows the download cache: the
/// first snapshot comes at once and a new one after each change, with `initializing` taken from the cache (T-42a-054).
pub fn storage() -> Option<Built> {
    if !crate::data::is_installed() {
        return None;
    }
    let screens = [Kind::Anime, Kind::Manga].map(|kind| storage_screen(kind));
    let views = screens.each_ref().map(|(screen, _)| Built { content: crate::ui::scrolled(screen.widget()).upcast(), end: vec![screen.search().clone().upcast()] });
    let page = crate::screens::kind_page(views, true);
    STORAGE.with(|s| *s.borrow_mut() = screens.into());
    Some(Built { content: page.upcast(), end: vec![] })
}

/// One kind's Storage page and its watch on the download cache.
fn storage_screen(kind: Kind) -> (Rc<StorageScreen>, Vec<glib::JoinHandle<()>>) {
    let screen = StorageScreen::new(kind);
    // entry id -> source id of the rows on screen, for the delete hook
    let sources: Rc<RefCell<HashMap<i64, i64>>> = Rc::default();
    let known = Rc::clone(&sources);
    screen.connect_delete(move |row: &StorageRow| {
        let backend = download_host::current();
        let installed = known.borrow().get(&row.entry_id).is_some_and(|source| backend.source_installed(kind, *source));
        let outcome = storage::delete_outcome(installed);
        if installed {
            let entry_id = row.entry_id;
            download_host::spawn_detached(move || {
                backend.delete_entry(kind, entry_id);
            });
        }
        outcome
    });
    let reload = {
        let (weak, sources) = (Rc::downgrade(&screen), sources);
        move || {
            let (weak, sources) = (weak.clone(), Rc::clone(&sources));
            glib::spawn_future_local(async move {
                let Some(screen) = weak.upgrade() else { return };
                let db = crate::data::app().db.clone();
                let hide_hidden = diet::LIBRARY_HIDE_HIDDEN_CATEGORIES;
                let (backend, initializing) = (download_host::current(), *download_host::hub().cache.initializing().borrow());
                let built = download_host::blocking(move || storage_data::input(&db, kind, &*backend, hide_hidden, initializing)).await;
                match built {
                    Some(Ok((input, by_source))) => {
                        *sources.borrow_mut() = by_source;
                        screen.set_input(input);
                    }
                    Some(Err(message)) => tracing::warn!("the storage page could not load: {message}"),
                    None => tracing::warn!("the storage page task was dropped"),
                }
            });
        }
    };
    reload();
    let hub = download_host::hub();
    let (mut changes, mut initializing) = (hub.cache.subscribe(), hub.cache.initializing());
    let (on_change, on_init) = (reload.clone(), reload);
    let watch_changes = glib::spawn_future_local(async move {
        use tokio::sync::broadcast::error::RecvError;
        loop {
            match changes.recv().await {
                Ok(()) | Err(RecvError::Lagged(_)) => on_change(),
                Err(RecvError::Closed) => return,
            }
        }
    });
    let watch_init = glib::spawn_future_local(async move {
        while initializing.changed().await.is_ok() {
            on_init();
        }
    });
    (screen, vec![watch_changes, watch_init])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// docs/design/other-tabs.md: Updates and History use the Library's one-row header, and the stack the page hands
    /// back is the one the switch drives, so the header actions act on the visible kind.
    #[test]
    fn updates_and_history_have_the_library_header_and_follow_the_switch() {
        crate::testing::with_display(|| {
            let view = |word: &str| Built {
                content: gtk::Label::new(Some(word)).upcast(),
                end: vec![gtk::SearchEntry::new().upcast(), crate::ui::icon_button("edit-clear-all-symbolic", "Clear history").upcast()],
            };
            let (page, stack) = tab_page([view("anime list"), view("manga list")]);
            let header = page.first_child().and_downcast::<gtk::HeaderBar>().expect("the header row comes first");
            assert!(header.next_sibling().and_then(|row| row.next_sibling()).and_then(|bars| bars.next_sibling()).is_some_and(|w| w == stack), "then the narrow switch row, the search bars, then the two kinds");
            assert_eq!(kind_of(stack.visible_child_name().as_deref()), Kind::Anime, "Anime shows first");
            stack.set_visible_child_name(MANGA);
            assert_eq!(kind_of(stack.visible_child_name().as_deref()), Kind::Manga);
        });
    }

    /// F5 and the empty feed's Update library press the refresh button of their own kind: anime's is built first.
    #[test]
    fn f5_and_the_empty_feed_use_the_refresh_button_of_their_kind() {
        assert_eq!(refresh_index(kind_of(Some(ANIME))), 0);
        assert_eq!(refresh_index(kind_of(Some(MANGA))), 1);
        assert_eq!(refresh_index(kind_of(None)), 0, "before the stack has a page, anime");
    }
}
