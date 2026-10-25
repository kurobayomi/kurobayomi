//! The browse source screens (`browse_source`, `migrate_source_search`; T-40b-034, T-40b-038, T-41b-014, T-41b-024): one
//! source's listing of either kind, built over that kind's source directory, and the open page that a genre search from an
//! entry page reaches. The page itself is `pages::browse_source`; this builds its dependencies from the app, parks nothing
//! and awaits nothing.
//!
//! A genre or source-line search of an entry page arrives through `entry_core::handoff`. When the page below the entry page
//! is a browse page, that page runs the search on its own listing (`search_open`) and the entry page and the placeholder
//! this builds are popped; with no browse page below, global search runs it (the Aniyomi fallback).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use kuro_domain::source::browse::BrowseQuery;
use kuro_i18n::{MsgKey, tr};
use kuro_source::filter_list::FilterList;
use tokio::sync::watch;

use super::directory::{SourceDirectory, source_or_stub};
use super::entries::FavoriteStore;
use super::filters::{SourceFilters, genre_search};
use super::header::OverflowItem;
use super::listing::{LoadFuture, PageLoader};
use super::listing_view::{ListingDeps, WeakListingView};
use super::pages::{BrowseSourceDeps, FavoritesDeps, SourcePage, browse_source};
use super::pending;
use super::prefs::BrowsePrefs;
use super::primitives::ToastSpec;
use super::route::{BrowseRoute, search_listing};
use kuro_platform::kind::Kind;
use super::sources::LOCAL_SOURCE_ID;
use super::view::file_cover_loader;

/// What a source page needs of the app.
pub struct BrowseEnv {
    /// The catalogue of the page: the duplicate dialog's wording.
    pub kind: Kind,
    pub prefs: Rc<BrowsePrefs>,
    pub directory: Arc<dyn SourceDirectory>,
    pub store: Arc<dyn FavoriteStore>,
    pub open_entry: Rc<dyn Fn(i64)>,
    /// The duplicate dialog's Migrate existing entry.
    pub migrate: Rc<dyn Fn(i64, i64)>,
    pub edit_categories: Rc<dyn Fn()>,
    pub on_back: Rc<dyn Fn()>,
    /// Opens a web address in the system browser: Open in browser of the source's page.
    pub open_url: Rc<dyn Fn(&str)>,
    /// The Settings item of a configurable source.
    pub on_settings: Rc<dyn Fn(i64)>,
    pub toast: Rc<dyn Fn(ToastSpec)>,
    pub now_ms: Rc<dyn Fn() -> i64>,
}

/// The loader of a source that is not installed. The stub page never asks it.
pub(crate) struct NoLoader;

impl PageLoader for NoLoader {
    fn load(&self, _query: BrowseQuery, _filters: FilterList, _page: u32) -> LoadFuture {
        Box::pin(async { Err(tr(MsgKey::unknown_error)) })
    }
}

/// The page of `source_id` opened on `query`. A source the directory does not know, or one it lists but cannot run, gets
/// the missing-source page. The hide-in-library preference is read here, when the page is made (T-40b-034).
pub fn source_page(env: &BrowseEnv, source_id: i64, query: BrowseQuery) -> SourcePage {
    let listed = env.directory.sources().into_iter().find(|entry| entry.source.id == source_id && !entry.source.is_stub);
    let loader = listed.as_ref().and_then(|_| env.directory.loader(source_id));
    let stub = loader.is_none();
    let source = listed.as_ref().map_or_else(|| source_or_stub(Some(&*env.directory), source_id), |entry| entry.source.clone());
    let home = listed.as_ref().and_then(|entry| entry.home_url.clone()).filter(|url| !url.is_empty());
    let home_url = home.clone();
    let defaults = if stub { FilterList::default() } else { env.directory.filters(source_id).unwrap_or_default() };
    let filters = Rc::new(SourceFilters::new(defaults));
    let (_never_sent, ready) = watch::channel(true);

    // Help is the page's own (the local source's guide); Open in browser needs the source's site
    let on_overflow: Rc<dyn Fn(OverflowItem)> = {
        let (open, settings) = (Rc::clone(&env.open_url), Rc::clone(&env.on_settings));
        Rc::new(move |item| match item {
            OverflowItem::OpenInBrowser => {
                if let Some(url) = &home {
                    open(url);
                }
            }
            OverflowItem::Settings => settings(source_id),
            OverflowItem::Help => {}
        })
    };
    let page = browse_source(BrowseSourceDeps {
        supports_latest: source.supports_latest,
        configurable: env.directory.preferences(source_id).is_some(),
        prefs: Rc::clone(&env.prefs),
        listing: ListingDeps {
            loader: loader.unwrap_or_else(|| Arc::new(NoLoader)),
            prefs: Rc::clone(&env.prefs),
            ready,
            stub,
            is_local: source_id == LOCAL_SOURCE_ID,
            source_id,
            source_name: source.name.clone(),
            source_lang: source.lang.clone(),
            load_cover: file_cover_loader(env.kind),
            open_entry: {
                let open = Rc::clone(&env.open_entry);
                Rc::new(move |item| open(item.id))
            },
            menu: Rc::new(|_: &super::listing::ListItem| Vec::new()),
            favorite_urls: None,
            on_action: Rc::new(|_| {}),
        },
        filters: Rc::clone(&filters),
        favorites: Some(FavoritesDeps {
            store: Arc::clone(&env.store),
            toast: Rc::clone(&env.toast),
            open_entry: Rc::clone(&env.open_entry),
            migrate: Rc::clone(&env.migrate),
            now_ms: Rc::clone(&env.now_ms),
            edit_categories: Rc::clone(&env.edit_categories),
            kind: env.kind,
        }),
        hide_in_library: env.prefs.hide_in_library(),
        initial_query: query,
        on_back: Rc::clone(&env.on_back),
        on_overflow,
        open_url: Rc::clone(&env.open_url),
        home_url,
    });
    // a genre or author search from an entry page runs on this page when it is the one below
    if let SourcePage::Browse(shown) = &page {
        register_page(shown, filters);
    }
    page
}

/// The browse page a genre search can reach: its listing, its filters, its search box and its Popular and Latest chips.
struct OpenBrowse {
    listing: WeakListingView,
    filters: Rc<SourceFilters>,
    search: glib::WeakRef<gtk::SearchEntry>,
    chips: [Option<glib::WeakRef<gtk::ToggleButton>>; 2],
}

thread_local! {
    /// The browse page opened last. It holds weak references only, so it never keeps a popped page alive.
    static OPEN: RefCell<Option<OpenBrowse>> = const { RefCell::new(None) };
}

fn register(page: OpenBrowse) {
    OPEN.with(|slot| *slot.borrow_mut() = Some(page));
}

/// Makes `shown` the page a genre search from an entry page reaches, of either kind: an anime entry's genre or author
/// search runs on the anime listing below it (T-41b-024).
pub fn register_page(shown: &super::pages::BrowseSourcePage, filters: Rc<SourceFilters>) {
    register(OpenBrowse {
        listing: shown.listing.downgrade(),
        filters,
        search: shown.search.downgrade(),
        chips: [Some(shown.popular.downgrade()), shown.latest.as_ref().map(|chip| chip.downgrade())],
    });
}

/// A genre search on the open browse page (`searchGenre` of the page the entry page was opened from, T-40b-038): a genre
/// that a group or select holds sets that filter and searches with no text, any other genre is the search text. The
/// listing restarts and the call returns at once; nothing is awaited. False when no browse page is open.
pub fn search_open(genre: &str) -> bool {
    OPEN.with(|slot| {
        let open = slot.borrow();
        let Some(open) = open.as_ref() else { return false };
        let Some(listing) = open.listing.upgrade() else { return false };
        let (text, list) = genre_search(&open.filters.fresh(), genre);
        open.filters.load(list.clone());
        let text = text.unwrap_or_default();
        if let Some(entry) = open.search.upgrade() {
            entry.set_text(&text);
        }
        for chip in open.chips.iter().flatten().filter_map(|chip| chip.upgrade()) {
            chip.set_active(false);
        }
        listing.restart_with(search_listing(&text), list);
        true
    })
}

/// Whether the page under the one on show is a browse page. `names` are the widget names of the stacked pages and `shown`
/// the index of the page on show (`ui::page_stack`).
pub fn previous_is_browse(names: &[String], shown: usize) -> bool {
    shown > 0 && names.get(shown - 1).is_some_and(|name| name == "screen:browse_source")
}

/// The widget name of the page on show.
pub fn visible_page() -> String {
    let (names, shown) = crate::ui::page_stack();
    names.get(shown).cloned().unwrap_or_default()
}

/// Opens the entry page of `kind` for a library row.
pub fn open_entry(kind: Kind, id: i64) {
    match kind {
        Kind::Manga => crate::entry_core::details::route::open_kind(Kind::Manga, id),
        Kind::Anime => crate::entry_core::details::route::open(id),
    }
}

/// Milliseconds since the epoch.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
}

/// The app side of a source page of `kind`, over that kind's library.
fn production_env(kind: Kind) -> BrowseEnv {
    use crate::entry_core::handoff::{self, SourcePrefsRequest};

    BrowseEnv {
        kind,
        prefs: Rc::new(BrowsePrefs::new(crate::data::app().base.store().clone(), kind)),
        directory: super::directory::current(kind),
        store: crate::updates::history_add::store(kind),
        open_entry: Rc::new(move |id| open_entry(kind, id)),
        // the migrate dialog opens at once with old = the duplicate and new = the browsed entry (T-40b-046)
        migrate: Rc::new(move |old, new| (super::search_screen::migrate_dialog(kind, old))(new)),
        edit_categories: Rc::new(move || super::glue::open_categories(kind)),
        on_back: Rc::new(crate::ui::pop),
        open_url: Rc::new(|url| crate::ui::open_link(url, None)),
        on_settings: Rc::new(move |source_id| handoff::open_source_preferences(SourcePrefsRequest { source_id, kind })),
        toast: Rc::new(|spec| crate::toast::notify(spec.message)),
        now_ms: Rc::new(now_ms),
    }
}

/// The page for `browse_source` or `migrate_source_search`, as `screens::build` asks for it. `None` leaves the screen to its
/// sample page: no app data, or nothing was parked for this screen.
pub fn page(screen: &str) -> Option<gtk::Box> {
    if !crate::data::is_installed() {
        return None;
    }
    // a Sources row, a global search row or an entry page parked the route of its kind
    if let Some(BrowseRoute::BrowseSource { kind, source_id, query, .. }) = pending::take_for(screen) {
        let root = source_page(&production_env(kind), source_id, query).widget().clone();
        crate::ui::tag(&root, screen);
        return Some(root);
    }
    entry_search(screen)
}

/// The entry page on show that a parked genre or title search comes from, and the kind of its catalogue.
pub(super) fn search_caller() -> Option<Kind> {
    match visible_page().as_str() {
        "screen:manga_entry_details" => Some(Kind::Manga),
        "screen:anime_entry_details" => Some(Kind::Anime),
        _ => None,
    }
}

/// A genre or source-line search of the manga or anime page on show.
fn entry_search(screen: &str) -> Option<gtk::Box> {
    let kind = search_caller()?;
    let request = crate::entry_core::handoff::take_search()?;
    let (names, shown) = crate::ui::page_stack();
    if previous_is_browse(&names, shown) && search_open(&request.query) {
        // the search ran on the page below: this placeholder and the manga page go
        glib::idle_add_local_once(|| {
            crate::ui::pop();
            crate::ui::pop();
        });
        let placeholder = crate::ui::page("", &gtk::Box::new(gtk::Orientation::Vertical, 0), Vec::new(), true);
        crate::ui::tag(&placeholder, screen);
        return Some(placeholder);
    }
    pending::park(super::global_search::deep_link_fallback_for(kind, &request.query));
    Some(crate::screens::build("global_search"))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Mutex;

    use kuro_prefs::store::PrefStore;

    use super::*;
    use crate::browse::directory::tests::{FakeDirectory, entry};
    use crate::browse::entries::fake::FakeStore;
    use crate::browse::listing::ListItem;

    fn pump_until(mut done: impl FnMut() -> bool) -> bool {
        let context = glib::MainContext::default();
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(3) {
            while context.iteration(false) {}
            if done() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        done()
    }

    struct Rig {
        env: BrowseEnv,
        failing: Arc<std::sync::atomic::AtomicBool>,
        asked: Arc<Mutex<Vec<BrowseQuery>>>,
        log: Rc<RefCell<Vec<String>>>,
        _entered: tokio::runtime::EnterGuard<'static>,
    }

    /// A directory with the source 1 (supports no latest) whose listings hold a favourite and a new entry.
    fn rig() -> Rig {
        let rt: &'static tokio::runtime::Runtime =
            Box::leak(Box::new(tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_time().build().expect("runtime")));
        let entered = rt.enter();
        let asked: Arc<Mutex<Vec<BrowseQuery>>> = Arc::default();
        let log_asked = Arc::clone(&asked);
        let failing = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fails = Arc::clone(&failing);
        let mut alpha = entry(1, "Alpha", "en", "ext.a");
        alpha.home_url = Some("https://alpha.example".into());
        let directory = Arc::new(FakeDirectory {
            entries: vec![alpha],
            answers: Arc::new(move |_, query| {
                log_asked.lock().expect("lock").push(query.clone());
                if fails.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err("offline".into());
                }
                Ok(vec![
                    ListItem { id: 1, title: "mine".into(), url: "/mine".into(), favorite: true, ..ListItem::default() },
                    ListItem { id: 2, title: "new".into(), url: "/new".into(), ..ListItem::default() },
                ])
            }),
        });
        let log: Rc<RefCell<Vec<String>>> = Rc::default();
        let (opened, web, back, settings) = (Rc::clone(&log), Rc::clone(&log), Rc::clone(&log), Rc::clone(&log));
        let env = BrowseEnv {
            kind: Kind::Manga,
            prefs: Rc::new(BrowsePrefs::new(PrefStore::in_memory(), Kind::Manga)),
            directory,
            store: Arc::new(FakeStore::default()),
            open_entry: Rc::new(move |id| opened.borrow_mut().push(format!("entry {id}"))),
            migrate: Rc::new(|_, _| {}),
            edit_categories: Rc::new(|| {}),
            on_back: Rc::new(move || back.borrow_mut().push("back".into())),
            open_url: Rc::new(move |url| web.borrow_mut().push(format!("browser {url}"))),
            on_settings: Rc::new(move |id| settings.borrow_mut().push(format!("settings {id}"))),
            toast: Rc::new(|_| {}),
            now_ms: Rc::new(|| 1),
        };
        Rig { env, failing, asked, log, _entered: entered }
    }

    fn browse(page: &SourcePage) -> &crate::browse::pages::BrowseSourcePage {
        match page {
            SourcePage::Browse(page) => page,
            SourcePage::Stub(_) => panic!("a browse page"),
        }
    }

    fn a_source_the_directory_does_not_know_gets_the_missing_source_page() {
        let rig = rig();
        let page = source_page(&rig.env, 99, BrowseQuery::Popular);
        assert!(matches!(page, SourcePage::Stub(_)), "an unknown id is a stub page");
        assert!(rig.asked.lock().expect("lock").is_empty(), "and never loads");
    }

    fn a_page_opened_on_a_search_starts_with_that_search_and_shows_the_text() {
        let rig = rig();
        let page = source_page(&rig.env, 1, BrowseQuery::Search("one piece".into()));
        let shown = browse(&page);
        assert!(pump_until(|| !rig.asked.lock().expect("lock").is_empty()));
        assert_eq!(rig.asked.lock().expect("lock")[0], BrowseQuery::Search("one piece".into()), "the first load is the search, not Popular");
        assert_eq!(shown.search.text().as_str(), "one piece");
        assert!(!shown.popular.is_active(), "no chip claims a search");
    }

    fn a_genre_search_reaches_the_open_page_and_restarts_its_listing_at_once() {
        let rig = rig();
        let page = source_page(&rig.env, 1, BrowseQuery::Popular);
        let shown = browse(&page);
        assert!(pump_until(|| !rig.asked.lock().expect("lock").is_empty()));
        assert!(shown.popular.is_active());
        assert!(search_open("Drama"), "the open page takes the search");
        assert_eq!(shown.listing.query(), BrowseQuery::Search("Drama".into()), "no group holds the genre: its name is the text");
        assert!(!shown.popular.is_active(), "Popular no longer applies");
        assert_eq!(shown.search.text().as_str(), "Drama");
        assert!(pump_until(|| rig.asked.lock().expect("lock").contains(&BrowseQuery::Search("Drama".into()))), "the listing loads the search");
        drop(page);
    }

    fn a_failed_first_page_offers_only_retry_and_retry_asks_again() {
        use super::super::primitives::EmptyAction;
        let rig = rig();
        rig.failing.store(true, std::sync::atomic::Ordering::SeqCst);
        let page = source_page(&rig.env, 1, BrowseQuery::Popular);
        let error_buttons = || {
            let mut buttons = Vec::new();
            fn walk(widget: &gtk::Widget, out: &mut Vec<gtk::Button>) {
                if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                    out.push(button.clone());
                }
                let mut child = widget.first_child();
                while let Some(next) = child {
                    walk(&next, out);
                    child = next.next_sibling();
                }
            }
            walk(browse(&page).listing.widget().upcast_ref(), &mut buttons);
            buttons.into_iter().filter(|b| b.tooltip_text().is_some()).collect::<Vec<_>>()
        };
        assert!(pump_until(|| !error_buttons().is_empty()), "a failed first page shows the error actions");
        let buttons = error_buttons();
        assert_eq!(buttons.len(), 1, "retry is the only button");
        assert_eq!(buttons[0].tooltip_text().as_deref(), Some(EmptyAction::Retry.label().as_str()));
        rig.failing.store(false, std::sync::atomic::Ordering::SeqCst);
        let asked = rig.asked.lock().expect("lock").len();
        buttons[0].emit_clicked();
        assert!(pump_until(|| rig.asked.lock().expect("lock").len() > asked), "retry asks the first page again");
        assert!(rig.log.borrow().is_empty(), "and opens nothing in the web view");
    }

    /// The rig over `directory::tests::catalogues` of `kind`: the source 42 runs, the source 43 is only listed.
    fn rig_of(kind: Kind) -> Rig {
        let mut rig = rig();
        rig.env.kind = kind;
        rig.env.prefs = Rc::new(BrowsePrefs::new(PrefStore::in_memory(), kind));
        rig.env.directory = Arc::new(crate::browse::directory::tests::catalogues(kind));
        rig
    }

    fn a_listed_source_that_cannot_run_and_an_unknown_one_get_the_missing_source_page_for_either_kind() {
        for kind in [Kind::Manga, Kind::Anime] {
            let rig = rig_of(kind);
            assert!(matches!(source_page(&rig.env, 43, BrowseQuery::Popular), SourcePage::Stub(_)), "{kind:?}: listed, but nothing to ask");
            assert!(matches!(source_page(&rig.env, 99, BrowseQuery::Popular), SourcePage::Stub(_)), "{kind:?}");
        }
    }

    fn a_runnable_source_of_either_kind_lists_offers_latest_and_opens_its_site_in_the_browser() {
        for kind in [Kind::Manga, Kind::Anime] {
            let rig = rig_of(kind);
            let page = source_page(&rig.env, 42, BrowseQuery::Popular);
            let shown = browse(&page);
            assert!(shown.latest.is_some(), "{kind:?}: the source states it supports latest");
            assert!(pump_until(|| shown.listing.loaded_count() > 0), "{kind:?}: the listing loads");
            let overflow = shown.header.overflow.as_ref().expect("the overflow menu");
            let labels = crate::widgets::view_options::menu_labels(&overflow.menu_model().expect("a menu"));
            let label = tr(MsgKey::action_open_in_browser);
            let item = labels.iter().position(|l| *l == label).expect("item");
            overflow.activate_action(&format!("more.e{item}"), None).expect("the item's action");
            assert_eq!(*rig.log.borrow(), ["browser https://src.example"], "{kind:?}");
        }
    }

    fn a_route_of_either_kind_is_taken_for_the_browse_source_screen() {
        for kind in [Kind::Manga, Kind::Anime] {
            let route = BrowseRoute::BrowseSource { kind, source_id: 42, query: BrowseQuery::Popular, migrating: false };
            crate::browse::pending::park(route.clone());
            assert_eq!(crate::browse::pending::take_for("browse_source"), Some(route));
        }
    }

    pub(crate) fn smoke() {
        a_listed_source_that_cannot_run_and_an_unknown_one_get_the_missing_source_page_for_either_kind();
        a_runnable_source_of_either_kind_lists_offers_latest_and_opens_its_site_in_the_browser();
        a_route_of_either_kind_is_taken_for_the_browse_source_screen();
        a_source_the_directory_does_not_know_gets_the_missing_source_page();
        a_page_opened_on_a_search_starts_with_that_search_and_shows_the_text();
        a_genre_search_reaches_the_open_page_and_restarts_its_listing_at_once();
        a_failed_first_page_offers_only_retry_and_retry_asks_again();
    }

    #[test]
    fn the_previous_page_is_a_browse_page_only_when_the_one_below_the_shown_page_is() {
        let stack = |names: &[&str]| names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
        let names = stack(&["root", "screen:browse_source", "screen:manga_entry_details"]);
        assert!(previous_is_browse(&names, 2));
        assert!(!previous_is_browse(&names, 1), "a browse page on show has the root below it");
        assert!(!previous_is_browse(&names, 0), "nothing is below the root");
        let other = stack(&["root", "screen:library", "screen:manga_entry_details"]);
        assert!(!previous_is_browse(&other, 2));
    }

    #[test]
    fn a_genre_search_with_no_open_page_reports_it() {
        OPEN.with(|slot| *slot.borrow_mut() = None);
        assert!(!search_open("Drama"), "the caller falls back to global search");
    }
}
