//! The global search and migrate search screens (`global_search`, `migrate_search`; T-40b-060 to T-40b-067): the page of
//! `pages::global_search` over a `SearchRunner`, started from what the caller parked. The start, the sources a search can
//! ask and the routes of its rows are pure and tested here; the page is thin.
//!
//! The Has results toggle is stored as `has_filters_toggle_state` and read again when a page opens, and the migrate search is
//! this same page, so both screens share the key (T-40b-063).

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::{Rc, Weak};
use std::sync::Arc;

use gtk::prelude::*;
use kuro_domain::source::browse::BrowseQuery;
use kuro_i18n::{MsgKey, tr};
use tokio::sync::watch;

use super::browse_screen::open_entry;
use super::directory::SourceDirectory;
use super::entries::FavoriteStore;
use super::favorite_flow::{FavoriteFlow, FavoriteHooks};
use super::listing::ListItem;
use super::global_search::{Candidate, GlobalSearch, MigrateStart, migrate_start, row_text_route};
use super::pages::{GlobalSearchNav, SearchChips, SourceFilter, cover_menu, global_search as search_page_widget};
use super::pending::{self, SearchIntent};
use super::prefs::BrowsePrefs;
use super::route::BrowseRoute;
use kuro_platform::kind::Kind;
use super::search_runner::{Loaders, RunnerHooks, SearchRunner};
use super::dialogs::DialogSlot;
use super::migrate_dialog::{Facts, MigrateHandlers, show};
use super::migrate_engine::{CustomImage, MigrateSteps, migrate};
use super::migrate_live::LiveSteps;
use super::view::file_cover_loader;
use crate::screens::Built;

/// What a search page starts with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    /// The query to search (blank searches nothing), the extension filter of an intent, and the source a migrate search
    /// started from.
    Search { query: String, extension: Option<String>, origin: Option<i64> },
    /// A migrate search whose manga is not in the library (E-MG-02): the page is an error state.
    MigrateMissing,
}

/// The start of a page. A migrate search starts from its manga's title and puts that manga's source first (E-MG-01); a
/// missing manga is an error, not a panic (E-MG-02), and a blank title searches nothing (E-MG-03). Otherwise an intent
/// wins over a parked route, which wins over the text an entry page parked.
pub fn start_for(migrating: bool, migrate_manga: Option<(i64, String)>, intent: Option<SearchIntent>, route: Option<BrowseRoute>, entry_text: Option<String>) -> Start {
    if migrating {
        return match migrate_start(migrate_manga) {
            MigrateStart::Missing => Start::MigrateMissing,
            MigrateStart::Ready { origin, title } => Start::Search { query: title, extension: None, origin: Some(origin) },
        };
    }
    if let Some(SearchIntent { query, extension, .. }) = intent {
        return Start::Search { query, extension, origin: None };
    }
    let query = match route {
        Some(BrowseRoute::GlobalSearch { query, .. }) => query,
        _ => entry_text.unwrap_or_default(),
    };
    Start::Search { query, extension: None, origin: None }
}

/// The sources a search can ask.
pub fn candidates(directory: &dyn SourceDirectory) -> Vec<Candidate> {
    directory.sources().into_iter().map(|entry| Candidate { source: entry.source, package: entry.package }).collect()
}

/// `hidden_catalogues` as source ids; text that is not an id is skipped.
pub fn hidden_ids(disabled: &BTreeSet<String>) -> BTreeSet<i64> {
    disabled.iter().filter_map(|id| id.parse().ok()).collect()
}

/// The arrow of a source row of `kind`: that source's listing, opened with the text in the search box whether or not it
/// was submitted (GS-EC-15). With no text it opens on Popular. A migrate search opens the migrate listing.
pub fn source_route(kind: Kind, source_id: i64, typed: &str, migrating: bool) -> BrowseRoute {
    if typed.trim().is_empty() {
        return BrowseRoute::BrowseSource { kind, source_id, query: BrowseQuery::Popular, migrating };
    }
    match row_text_route(kind, source_id, typed) {
        BrowseRoute::BrowseSource { kind, source_id, query, .. } => BrowseRoute::BrowseSource { kind, source_id, query, migrating },
        other => other,
    }
}

/// What a search page needs of the app.
pub struct SearchEnv {
    /// Which catalogue the page searches: it reads that kind's languages and routes its rows.
    pub kind: Kind,
    pub prefs: Rc<BrowsePrefs>,
    pub directory: Arc<dyn SourceDirectory>,
    /// The library of `kind`: a card's Add to library and Remove from library.
    pub store: Arc<dyn FavoriteStore>,
    /// A card: the entry's page (library id).
    pub open_entry: Rc<dyn Fn(i64)>,
    /// A card of a migrate search: the dialog of what to migrate. `None` opens the entry, as a long press does.
    pub pick_entry: Option<Rc<dyn Fn(i64)>>,
    /// The single-result auto-open: the entry's page replaces the search.
    pub replace_with_entry: Rc<dyn Fn(i64)>,
    pub open_route: Rc<dyn Fn(BrowseRoute)>,
}

/// The error state of a page that cannot search.
fn error_state(text: &str) -> gtk::Widget {
    crate::widgets::status::error(text, None, None::<fn()>).upcast()
}

#[cfg(test)]
/// The search page for `start`. The runner lives as long as the page; popping it aborts the fetches.
pub fn search_widget(env: &SearchEnv, start: &Start, migrating: bool) -> gtk::Widget {
    search_page(env, start, migrating).0
}

/// The search page and the widgets its header carries (the search toggle and the options).
pub fn search_page(env: &SearchEnv, start: &Start, migrating: bool) -> (gtk::Widget, Vec<gtk::Widget>) {
    let Start::Search { query, extension, origin } = start else { return (error_state(&tr(MsgKey::unknown_error)), Vec::new()) };
    let sources = &env.prefs.sources;
    // the has-results toggle is cut from the settings: it starts off on every open
    let only_has_results = false;
    let directory = Arc::clone(&env.directory);
    // the hide set is cut: every source of an enabled language is searched
    let hidden: BTreeSet<String> = BTreeSet::new();
    let engine = GlobalSearch::new(candidates(&*directory), sources.enabled_languages.get(), hidden_ids(&hidden), *origin, only_has_results);
    // the page and the runner reach each other: the page calls the runner through a weak slot
    let slot: Rc<RefCell<Weak<SearchRunner>>> = Rc::default();
    let with_runner = |action: fn(&Rc<SearchRunner>, &str)| {
        let slot = Rc::clone(&slot);
        move |text: &str| {
            let runner = slot.borrow().upgrade();
            if let Some(runner) = runner {
                action(&runner, text);
            }
        }
    };
    let submit = with_runner(|runner, text| runner.submit(text));
    let (typed_slot, open_route) = (Rc::clone(&slot), Rc::clone(&env.open_route));
    let open_entry = env.pick_entry.clone().unwrap_or_else(|| Rc::clone(&env.open_entry));
    let kind = env.kind;
    // a card's menu: Open (the entry's page), the library flow of the browse pages, Open in browser on the source's site.
    // The cards are drawn again from the results, so a flag the flow flipped is kept here.
    let flags: Rc<RefCell<HashMap<i64, bool>>> = Rc::default();
    let flow_slot: Rc<RefCell<Option<Rc<FavoriteFlow>>>> = Rc::default();
    let homes: HashMap<i64, String> = directory.sources().into_iter().filter_map(|entry| Some((entry.source.id, entry.home_url?))).collect();
    let menu = {
        let (flags, flow_slot, open) = (Rc::clone(&flags), Rc::clone(&flow_slot), Rc::clone(&env.open_entry));
        move |source_id: i64, item: &ListItem| {
            let mut item = item.clone();
            if let Some(&flag) = flags.borrow().get(&item.id) {
                item.favorite = flag;
            }
            let flow = flow_slot.borrow().clone();
            let web = super::nav::entry_web_url(homes.get(&source_id).map(String::as_str), &item.url);
            let open = Rc::clone(&open);
            cover_menu(&item, Rc::new(move |item: &ListItem| open(item.id)), flow, web, Rc::new(|url: &str| crate::ui::open_link(url, None)))
        }
    };
    let nav = GlobalSearchNav {
        search: Rc::new(submit),
        open_source: Rc::new(move |id, typed| open_route(source_route(kind, id, typed, migrating))),
        open_entry: Rc::new(move |_, item| open_entry(item.id)),
        menu: Rc::new(menu),
        load_cover: file_cover_loader(kind),
        chips: SearchChips {
            filter: SourceFilter::All,
            only_has_results,
        },
        on_chips: Rc::new(move |chips| {
            // the filter searches again; the toggle is not stored (the setting is cut)
            let runner = typed_slot.borrow().upgrade();
            if let Some(runner) = runner {
                runner.set_only_has_results(chips.only_has_results);
                runner.set_filter(chips.filter);
            }
        }),
    };
    let (_never_sent, ready) = watch::channel(true);
    let page = search_page_widget(nav, ready);
    let hooks = FavoriteHooks {
        set_flag: Rc::new(move |id, flag| {
            flags.borrow_mut().insert(id, flag);
        }),
        toast: Rc::new(|spec: super::primitives::ToastSpec| crate::toast::notify(spec.message)),
        open_entry: Rc::clone(&env.open_entry),
        migrate: Rc::new(move |old, new| (migrate_dialog(kind, old))(new)),
        now_ms: Rc::new(super::browse_screen::now_ms),
        edit_categories: Rc::new(move || super::glue::open_categories(kind)),
    };
    *flow_slot.borrow_mut() = Some(FavoriteFlow::new_for(Arc::clone(&env.store), page.widget(), hooks, kind));
    page.set_hint(&super::texts::global_search_empty_hint());
    let hooks = RunnerHooks { changed: Rc::new(page.updater()), open_entry: Rc::clone(&env.replace_with_entry) };
    let loaders: Loaders = {
        let directory = Arc::clone(&directory);
        Arc::new(move |id| directory.loader(id))
    };
    let runner = SearchRunner::new(engine, loaders, hooks);
    let filters = Arc::clone(&directory);
    runner.set_default_filters(Arc::new(move |id| filters.filters(id)));
    // a pasted link resolves through the sources' `uri_type` before any catalogue search (T-20a-031)
    runner.set_link_resolver(Arc::new(move |uri| directory.resolve_link(uri)));
    *slot.borrow_mut() = Rc::downgrade(&runner);
    runner.attach(page.widget());
    if query.trim().is_empty() {
        runner.refresh();
    } else {
        runner.request(query, extension.as_deref());
    }
    (page.widget().clone(), page.actions())
}

/// The app side of a search page of `kind`. `migrate_from` is the entry a migrate search started from: a card opens the
/// migrate dialog (T-41a-041, T-40a-015) instead of the entry.
fn production_env(kind: Kind, migrate_from: Option<i64>) -> SearchEnv {
    SearchEnv {
        kind,
        prefs: Rc::new(BrowsePrefs::new(crate::data::app().base.store().clone(), kind)),
        directory: super::directory::current(kind),
        store: crate::updates::history_add::store(kind),
        open_entry: Rc::new(move |id| open_entry(kind, id)),
        pick_entry: migrate_from.map(|old| migrate_dialog(kind, old)),
        replace_with_entry: Rc::new(move |id| {
            crate::ui::pop();
            open_entry(kind, id);
        }),
        open_route: Rc::new(pending::open),
    }
}

/// The migration steps of `kind` over the app: its database, its source directory, the one download service and the
/// preferences.
fn live_steps(kind: Kind) -> LiveSteps {
    let app = crate::data::app();
    let downloads = crate::shell::host::Shell::get().and_then(|shell| shell.download_service());
    LiveSteps::new(kind, app.db.clone(), super::directory::current(kind), downloads, app.base.store().clone())
}

/// The dialog of what to include for moving `old` to a card's entry of `kind`. Show manga, Show anime and Show seasons
/// open the card's page, where a seasons title lists its seasons. The options come from the old entry (its custom images
/// and downloads) and start with every option selected; the choice is not stored. Copy and Migrate run the migration
/// engine on the main context; a success pops the search and pushes the new entry, a failure shows its reason in the
/// dialog. The duplicate dialog of a browse page opens it at once with the duplicate as `old`.
pub(super) fn migrate_dialog(kind: Kind, old: i64) -> Rc<dyn Fn(i64)> {
    let slot = DialogSlot::new();
    Rc::new(move |new: i64| {
        let steps = Rc::new(live_steps(kind));
        let (Some(old_entry), Some(new_entry)) = (steps.entry(old), steps.entry(new)) else {
            tracing::error!(old, new, "an entry of the migrate dialog could not be read");
            crate::toast::notify(tr(MsgKey::internal_error));
            return;
        };
        let facts = Facts {
            kind,
            old: steps.fetch_type(old),
            new: steps.fetch_type(new),
            custom_cover: steps.has_custom(CustomImage::Cover, &old_entry),
            custom_background: steps.has_custom(CustomImage::Background, &old_entry),
            downloads: steps.has_downloads(&old_entry),
        };
        let handlers = MigrateHandlers {
            show_entry: Rc::new(move || open_entry(kind, new)),
            run: Rc::new(move |plan, done| {
                let (steps, old_entry, new_entry) = (Rc::clone(&steps), old_entry.clone(), new_entry.clone());
                gtk::glib::spawn_future_local(async move {
                    done(match migrate(&*steps, &old_entry, &new_entry, plan).await {
                        Ok(outcome) => {
                            if !outcome.warnings.is_empty() {
                                crate::toast::notify(tr(MsgKey::internal_error));
                            }
                            Ok(())
                        }
                        Err(error) => Err(error.to_string()),
                    });
                });
            }),
            // the migrate page is popped, which leaves the page it was opened from, and the new entry is pushed
            finished: Rc::new(move || {
                crate::ui::pop();
                open_entry(kind, new);
            }),
        };
        show(&slot, &crate::ui::window(), facts, crate::misc::diet_defaults::MIGRATE_FLAGS_ALL, handlers);
    })
}

/// The entry a migrate request names: its source and title. An entry that is not in the library is `None`.
fn migrate_origin(kind: Kind, entry_id: i64) -> Option<(i64, String)> {
    live_steps(kind).entry(entry_id).map(|entry| (entry.source, entry.title))
}

/// The body of `global_search` and `migrate_search`, as `screens::body` asks for it. `None` leaves the screen to its sample
/// page: no app data, or nothing was parked for this screen. The catalogue is the migrate request's, the parked route's, or
/// the entry page's that asked for a title or genre search; an intent carries its own.
pub fn body(migrating: bool) -> Option<Built> {
    if !crate::data::is_installed() {
        return None;
    }
    let screen = if migrating { "migrate_search" } else { "global_search" };
    let (kind, migrate_from, start) = if migrating {
        let request = crate::entry_core::handoff::take_migrate()?;
        let kind = request.kind;
        (kind, Some(request.entry_id), start_for(true, migrate_origin(kind, request.entry_id), None, None, None))
    } else {
        let intent = pending::take_intent();
        let route = pending::take_for(screen);
        let caller = super::browse_screen::search_caller();
        let entry_text = (intent.is_none() && route.is_none() && caller.is_some())
            .then(crate::entry_core::handoff::take_search)
            .flatten()
            .map(|request| request.query);
        if intent.is_none() && route.is_none() && entry_text.is_none() {
            return None;
        }
        let kind = match (&intent, &route) {
            (Some(intent), _) => intent.kind,
            (None, Some(BrowseRoute::GlobalSearch { kind, .. })) => *kind,
            _ => caller.unwrap_or(Kind::Manga),
        };
        (kind, None, start_for(false, None, intent, route, entry_text))
    };
    let (content, end) = search_page(&production_env(kind, migrate_from), &start, migrating);
    Some(Built { content, end })
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Mutex;

    use gtk::glib;
    use kuro_prefs::store::PrefStore;

    use super::*;
    use crate::browse::directory::tests::{FakeDirectory, entry};
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

    #[test]
    fn a_migrate_search_starts_from_its_manga_and_a_missing_one_is_an_error_state() {
        assert_eq!(
            start_for(true, Some((5, "Berserk".into())), None, None, None),
            Start::Search { query: "Berserk".into(), extension: None, origin: Some(5) },
            "E-MG-01: the title is searched and its source is the origin"
        );
        assert_eq!(start_for(true, None, None, None, None), Start::MigrateMissing, "E-MG-02: a missing manga is an error, not a panic");
        assert_eq!(
            start_for(true, Some((5, String::new())), None, None, None),
            Start::Search { query: String::new(), extension: None, origin: Some(5) },
            "E-MG-03: a blank title is a blank query, which searches nothing"
        );
    }

    #[test]
    fn an_intent_wins_over_a_route_and_a_route_over_the_text_an_entry_parked() {
        let intent = SearchIntent { query: "one".into(), extension: Some("eu.a".into()), kind: Kind::Manga };
        let route = BrowseRoute::GlobalSearch { kind: Kind::Manga, source_id: None, query: "two".into() };
        assert_eq!(
            start_for(false, None, Some(intent), Some(route.clone()), Some("three".into())),
            Start::Search { query: "one".into(), extension: Some("eu.a".into()), origin: None }
        );
        assert_eq!(start_for(false, None, None, Some(route), Some("three".into())), Start::Search { query: "two".into(), extension: None, origin: None });
        assert_eq!(start_for(false, None, None, None, Some("three".into())), Start::Search { query: "three".into(), extension: None, origin: None });
        assert_eq!(start_for(false, None, None, None, None), Start::Search { query: String::new(), extension: None, origin: None });
    }

    #[test]
    fn a_source_row_of_either_kind_opens_its_own_listing_with_the_typed_text_or_on_popular() {
        for kind in [Kind::Manga, Kind::Anime] {
            assert_eq!(
                source_route(kind, 7, "typed", false),
                BrowseRoute::BrowseSource { kind, source_id: 7, query: BrowseQuery::Search("typed".into()), migrating: false },
                "GS-EC-15: unsubmitted text goes with the tap"
            );
            assert_eq!(source_route(kind, 7, "  ", false), BrowseRoute::BrowseSource { kind, source_id: 7, query: BrowseQuery::Popular, migrating: false });
            assert!(matches!(source_route(kind, 7, "x", true), BrowseRoute::BrowseSource { migrating: true, .. }), "a migrate search opens the migrate listing");
        }
    }

    #[test]
    fn candidates_carry_their_package_and_the_hidden_ids_skip_text_that_is_no_id() {
        let directory = FakeDirectory { entries: vec![entry(1, "A", "en", "ext.a"), entry(2, "B", "ja", "")], answers: Arc::new(|_, _| Ok(Vec::new())) };
        let found = candidates(&directory);
        assert_eq!(found.iter().map(|c| c.source.id).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(found[0].package, "ext.a");
        assert_eq!(hidden_ids(&BTreeSet::from(["3".to_owned(), "x".to_owned()])), BTreeSet::from([3]));
    }

    struct Rig {
        env: SearchEnv,
        asked: Arc<Mutex<Vec<BrowseQuery>>>,
        log: Rc<RefCell<Vec<String>>>,
        _entered: tokio::runtime::EnterGuard<'static>,
    }

    /// Two sources, 1 (ext.a) and 2 (ext.b), that each answer a search with one entry.
    fn rig() -> Rig {
        let rt: &'static tokio::runtime::Runtime =
            Box::leak(Box::new(tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_time().build().expect("runtime")));
        let entered = rt.enter();
        let asked: Arc<Mutex<Vec<BrowseQuery>>> = Arc::default();
        let record = Arc::clone(&asked);
        let directory = Arc::new(FakeDirectory {
            entries: vec![entry(1, "Alpha", "en", "ext.a"), entry(2, "Beta", "en", "ext.b")],
            answers: Arc::new(move |id, query| {
                record.lock().expect("lock").push(query.clone());
                Ok(vec![ListItem { id: 40 + id, source_id: id, title: format!("Found {id}"), url: format!("/{id}"), ..ListItem::default() }])
            }),
        });
        let prefs = Rc::new(BrowsePrefs::new(PrefStore::in_memory(), Kind::Manga));
        let log: Rc<RefCell<Vec<String>>> = Rc::default();
        let (entry_log, replace_log, route_log) = (Rc::clone(&log), Rc::clone(&log), Rc::clone(&log));
        let env = SearchEnv {
            kind: Kind::Manga,
            prefs,
            directory,
            store: Arc::new(crate::browse::entries::fake::FakeStore::default()),
            pick_entry: None,
            open_entry: Rc::new(move |id| entry_log.borrow_mut().push(format!("entry {id}"))),
            replace_with_entry: Rc::new(move |id| replace_log.borrow_mut().push(format!("replace {id}"))),
            open_route: Rc::new(move |route| route_log.borrow_mut().push(format!("route {route:?}"))),
        };
        Rig { env, asked, log, _entered: entered }
    }

    fn labels(root: &gtk::Widget) -> Vec<String> {
        fn walk(widget: &gtk::Widget, out: &mut Vec<String>) {
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                out.push(label.label().to_string());
            }
            let mut child = widget.first_child();
            while let Some(next) = child {
                walk(&next, out);
                child = next.next_sibling();
            }
        }
        let mut out = Vec::new();
        walk(root, &mut out);
        out
    }

    fn a_page_with_a_query_searches_every_enabled_source_and_draws_their_rows() {
        let rig = rig();
        let start = Start::Search { query: "one".into(), extension: None, origin: None };
        let page = search_widget(&rig.env, &start, false);
        assert!(pump_until(|| rig.asked.lock().expect("lock").len() == 2), "both sources are asked");
        assert!(rig.asked.lock().expect("lock").iter().all(|q| *q == BrowseQuery::Search("one".into())));
        assert!(pump_until(|| {
            let text = labels(&page);
            text.iter().any(|t| t == "Found 1") && text.iter().any(|t| t == "Found 2")
        }), "the results are drawn: {:?}", labels(&page));
    }

    fn the_has_results_toggle_filters_the_open_page_and_is_not_stored() {
        // the search options menu: its button, and the dot that shows while a Show only filter is on
        fn options(root: &gtk::Widget) -> Option<(gtk::MenuButton, gtk::Widget)> {
            if let Some(button) = root.downcast_ref::<gtk::MenuButton>().filter(|b| b.tooltip_text().as_deref() == Some(super::super::pages::search_options_label().as_str())) {
                let dot = button.parent()?.last_child()?;
                return Some((button.clone(), dot));
            }
            let mut child = root.first_child();
            while let Some(next) = child {
                if let Some(found) = options(&next) {
                    return Some(found);
                }
                child = next.next_sibling();
            }
            None
        }
        let rig = rig();
        let start = Start::Search { query: String::new(), extension: None, origin: None };
        // the options button is a header action of the page
        let (_page, actions) = search_page(&rig.env, &start, false);
        let in_header = |actions: &[gtk::Widget]| actions.iter().find_map(options);
        assert!(pump_until(|| in_header(&actions).is_some()), "the search options menu");
        let (button, dot) = in_header(&actions).expect("menu");
        assert!(!dot.is_visible(), "Has results starts off on every open");
        button.activate_action("viewopt.has_results", None).expect("the Has results check");
        assert!(dot.is_visible(), "Has results is on in this page");
        let (_again, again) = search_page(&rig.env, &start, true);
        assert!(pump_until(|| in_header(&again).is_some()));
        assert!(!in_header(&again).expect("menu").1.is_visible(), "the next page does not read it back");
    }
    fn a_source_arrow_opens_its_listing_with_the_typed_text() {
        let rig = rig();
        let start = Start::Search { query: "one".into(), extension: None, origin: None };
        let page = search_widget(&rig.env, &start, false);
        assert!(pump_until(|| labels(&page).iter().any(|t| t == "Found 1")));
        let search = {
            fn find(widget: &gtk::Widget) -> Option<gtk::SearchEntry> {
                if let Some(entry) = widget.downcast_ref::<gtk::SearchEntry>() {
                    return Some(entry.clone());
                }
                let mut child = widget.first_child();
                while let Some(next) = child {
                    if let Some(found) = find(&next) {
                        return Some(found);
                    }
                    child = next.next_sibling();
                }
                None
            }
            find(&page).expect("search entry")
        };
        search.set_text("typed but never submitted");
        let arrow = {
            fn find(widget: &gtk::Widget) -> Option<gtk::Button> {
                if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                    if button.icon_name().as_deref() == Some("go-next-symbolic") {
                        return Some(button.clone());
                    }
                }
                let mut child = widget.first_child();
                while let Some(next) = child {
                    if let Some(found) = find(&next) {
                        return Some(found);
                    }
                    child = next.next_sibling();
                }
                None
            }
            find(&page).expect("source arrow")
        };
        arrow.emit_clicked();
        let last = rig.log.borrow().last().cloned().unwrap_or_default();
        assert!(last.contains("Search(\"typed but never submitted\")"), "GS-EC-15: {last}");
    }

    fn a_card_opens_its_entry_and_one_result_with_an_extension_filter_replaces_the_page() {
        let rig = rig();
        let start = Start::Search { query: "one".into(), extension: Some("ext.a".into()), origin: None };
        let _page = search_widget(&rig.env, &start, false);
        assert!(pump_until(|| rig.log.borrow().contains(&"replace 41".to_owned())), "GS-EC-13: one source, one result, replaced by its page: {:?}", rig.log.borrow());
        assert_eq!(rig.asked.lock().expect("lock").len(), 1, "only the extension's source is asked");
    }

    fn a_migrate_search_with_a_missing_manga_is_an_error_state_that_searches_nothing() {
        let rig = rig();
        let page = search_widget(&rig.env, &Start::MigrateMissing, true);
        assert!(labels(&page).contains(&tr(MsgKey::unknown_error)), "E-MG-02");
        assert!(rig.asked.lock().expect("lock").is_empty());
    }

    fn an_anime_page_searches_its_enabled_sources_and_sends_its_rows_to_the_anime_page() {
        let mut rig = rig();
        rig.env.kind = Kind::Anime;
        let page = search_widget(&rig.env, &Start::Search { query: "one".into(), extension: None, origin: None }, false);
        assert!(pump_until(|| labels(&page).iter().any(|t| t == "Found 2")), "the anime source answers: {:?}", labels(&page));
        assert_eq!(rig.asked.lock().expect("lock").len(), 2, "every enabled anime source is asked");
        let search = {
            fn find(widget: &gtk::Widget) -> Option<gtk::SearchEntry> {
                if let Some(entry) = widget.downcast_ref::<gtk::SearchEntry>() {
                    return Some(entry.clone());
                }
                let mut child = widget.first_child();
                while let Some(next) = child {
                    if let Some(found) = find(&next) {
                        return Some(found);
                    }
                    child = next.next_sibling();
                }
                None
            }
            find(&page).expect("search entry")
        };
        assert_eq!(search.placeholder_text().as_deref(), Some(crate::browse::texts::global_search_empty_hint()).as_deref(), "T-41a-027: the anime hint");
    }

    pub(crate) fn smoke() {
        an_anime_page_searches_its_enabled_sources_and_sends_its_rows_to_the_anime_page();
        a_page_with_a_query_searches_every_enabled_source_and_draws_their_rows();
        the_has_results_toggle_filters_the_open_page_and_is_not_stored();
        a_source_arrow_opens_its_listing_with_the_typed_text();
        a_card_opens_its_entry_and_one_result_with_an_extension_filter_replaces_the_page();
        a_migrate_search_with_a_missing_manga_is_an_error_state_that_searches_nothing();
    }
}
