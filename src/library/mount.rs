//! Mounts the library view in the window (T-37a-061): builds the view's dependencies from the app data, registers
//! the views so the shell can reach them, and exposes what the shell needs. The two kinds share one sidebar
//! destination (`library`) with an Anime and a Manga page; each kind still has its own sidebar index per nav style.
//!
//! Decision (T-37a-045, replaces B-14.11): with `MOVE_MANGA_TO_MORE` the manga library is a page under More, at index
//! 5. Aniyomi jumps from it to the anime tab; here Back returns to the tab it was opened from, and to More when it was
//! entered from the More list. The sidebar index below is the part this module owns; the Back target is the shell's
//! (`src/shell/tabs.rs`, `TabState::back`), which has to remember the origin tab when the manga page opens.
//!
//! - Other screens can fill the search box ([`search_library`] and [`inject_search`], B-1.3).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use kuro_data::entry::{EntryRepo, EntryUpdate};
use kuro_data::item::ItemRepo;
use kuro_data::source::StubSourceRepo;
use kuro_db::handle::Db;
use kuro_platform::kind::Kind;
use kuro_i18n::{MsgKey, tr};
use kuro_images::art::{ArtKind, ArtRef};
use kuro_images::cover_file::delete_from_cache;
use kuro_images::invalidate::remove_covers;
use kuro_platform::system_paths::SystemPaths;
use kuro_platform::paths::Paths;
use kuro_prefs::library::LibraryPrefs;

use crate::library::actions::delete_plan;
use crate::library::trackers::Logins;
use crate::library::download_facts;
use crate::library::query::{Facts, SourceInfo};
use crate::library::view::{Deps, LibraryView, Nav};
use crate::browse::route::{BrowseKind, BrowseRoute};
use crate::screens::Built;

thread_local! {
    static VIEWS: RefCell<Vec<(Kind, LibraryView)>> = const { RefCell::new(Vec::new()) };
    /// The kinds whose database watch is bound (one bit per kind): one watch serves every view of a kind.
    static WATCHED: Cell<u8> = const { Cell::new(0) };
    /// Searches injected before the view of their kind was built; each is applied when that view mounts.
    static PENDING_SEARCH: RefCell<Vec<(Kind, String)>> = const { RefCell::new(Vec::new()) };
    /// What the refresh button does (the update runner in `crate::misc` registers here).
    static UPDATE_HOST: RefCell<Option<UpdateHost>> = const { RefCell::new(None) };
    static DOWNLOAD_SINK: RefCell<Option<Rc<dyn Fn(Kind, Vec<i64>, DownloadAmount)>>> = const { RefCell::new(None) };
}

pub use crate::library::actions::DownloadAmount;

/// What the library's refresh button asks of the app (T-26a-102).
#[derive(Clone)]
pub struct UpdateHost {
    /// Starts a pass of one category of a kind; `false` when a pass of that kind is already running.
    pub start: Rc<dyn Fn(Kind, Option<i64>) -> bool>,
    /// Whether a pass of that kind is running.
    pub running: Rc<dyn Fn(Kind) -> bool>,
}

/// Registers the update runner behind the refresh button. Replaces an earlier one.
pub fn on_update(host: UpdateHost) {
    UPDATE_HOST.with(|h| *h.borrow_mut() = Some(host));
}

/// Registers what the Download action of a selection does. Replaces an earlier one.
pub fn on_download(sink: impl Fn(Kind, Vec<i64>, DownloadAmount) + 'static) {
    DOWNLOAD_SINK.with(|s| *s.borrow_mut() = Some(Rc::new(sink)));
}

const fn bit(kind: Kind) -> u8 {
    match kind {
        Kind::Anime => 1,
        Kind::Manga => 2,
    }
}

/// The view of `kind` that is on screen, or the first one when none is mapped yet.
fn visible(kind: Option<Kind>) -> Option<LibraryView> {
    VIEWS.with(|views| {
        let views = views.borrow();
        views
            .iter()
            .find(|(k, v)| kind.is_none_or(|want| want == *k) && v.widget().is_mapped())
            .or_else(|| views.iter().find(|(k, _)| kind.is_none_or(|want| want == *k)))
            .map(|(_, v)| v.clone())
    })
}

/// Reloads the library of `kind`, for changes made outside it such as a local folder scan.
pub fn refresh(kind: Kind) {
    if let Some(view) = visible(Some(kind)) {
        view.refresh();
    }
}

/// Puts `query` in the search box of one kind's library (B-1.3). A view that is not built yet gets it on mount.
pub fn inject_search(kind: Kind, query: &str) {
    match visible(Some(kind)) {
        Some(view) => view.set_query(query),
        None => PENDING_SEARCH.with(|p| p.borrow_mut().push((kind, query.to_owned()))),
    }
}

/// Opens the Library destination on `kind`'s page with `query` in its search box. This is the entry point for
/// screens that search the library (B-1.3).
pub fn search_library(kind: Kind, query: &str) {
    crate::ui::pop_to_root();
    crate::ui::open("library");
    inject_search(kind, query);
    show_kind(kind);
}

/// Switches the Library tab to `kind`'s page (T-36a-060: a deep link to an entry selects the matching library).
pub fn show_kind(kind: Kind) {
    if let Some(view) = visible(Some(kind)) {
        let page = kind.as_str();
        if let Some(stack) = view.widget().ancestor(gtk::Stack::static_type()).and_downcast::<gtk::Stack>() {
            stack.set_visible_child_name(page);
        }
    }
}

/// The route of "search globally" from the library of `kind`: the global search page, with `query` in its box.
fn global_search_route(kind: Kind, query: &str) -> BrowseRoute {
    let kind = if kind == Kind::Anime { BrowseKind::Anime } else { BrowseKind::Manga };
    BrowseRoute::GlobalSearch { kind, source_id: None, query: query.to_owned() }
}

/// Opens the global search page with `query` in its search box (T-37a-022). The request is parked first, because the
/// page builds itself from it (`browse::search_screen::body`).
fn open_global_search(kind: Kind, query: &str) {
    crate::browse::pending::open(global_search_route(kind, query));
}

/// `(id, is_local)` for each entry. An entry that cannot be read counts as local, so its files are never deleted.
fn local_flags(db: &Arc<Db>, ids: &[i64]) -> Vec<(i64, bool)> {
    let repo = EntryRepo::new(db.clone());
    ids.iter().map(|id| (*id, repo.by_id(*id).map_or(true, |e| e.source == 0))).collect()
}

/// Source name and language for the search and the language badge. A source with no stored row shows its id.
pub fn source_info(db: &Arc<Db>, kind: Kind, id: i64) -> SourceInfo {
    let found = StubSourceRepo::new(db.clone(), kind).get(id).ok().flatten().map(|s| (s.name, s.lang));
    match found {
        Some((name, language)) => SourceInfo { name, language },
        None => SourceInfo { name: id.to_string(), language: String::new() },
    }
}

/// Takes the selected entries out of the library: favorite off, and the cached covers (an anime's background too)
/// deleted. A cover timestamp moves only when a file was deleted, as Kotlin's `removeCovers` does, and it is written
/// with the removal (E-27). Downloads stay (T2-DEL-01, T2-DEL-03). Local entries keep their files.
pub fn remove_from_library(paths: &dyn Paths, db: &Arc<Db>, ids: &[i64]) -> usize {
    let now = kuro_data::time::now_millis();
    ids.iter()
        .filter(|id| {
            let done = remove_entry(paths, db, **id, now).map_err(|e| e.to_string());
            done.inspect_err(|e| tracing::error!(error = %e, entry = id, "removing from the library failed")).is_ok()
        })
        .count()
}

/// Deletes the cached cover (`ArtKind::Cover`) or background of an entry that just left the library, for the screens that
/// write the favorite themselves (Browse, History, the entry page). The local source keeps its files. Returns how many
/// files went; a background exists for anime only.
pub fn clear_cached(db: &Arc<Db>, id: i64, art: ArtKind) -> usize {
    // a test database has small ids that the real cover cache of the machine also uses: tests delete no files
    if cfg!(test) {
        return 0;
    }
    let Ok(entry) = EntryRepo::new(db.clone()).by_id(id) else { return 0 };
    if kuro_local::ids::is_local_source(entry.source) || (art == ArtKind::Background && !entry.kind.is_anime()) {
        return 0;
    }
    let Ok(paths) = SystemPaths::system() else { return 0 };
    let image = ArtRef {
        owner: entry.kind,
        entry_id: id,
        source_id: entry.source,
        kind: art,
        url: if art == ArtKind::Background { entry.background_url.clone() } else { entry.thumbnail_url.clone() },
        favorite: entry.favorite,
        last_modified: if art == ArtKind::Background { entry.background_last_modified() } else { entry.cover_last_modified },
    };
    delete_from_cache(&paths, &image, false)
}

/// One entry of either kind: favorite off and its cached cover (an anime's background too) deleted.
fn remove_entry(paths: &dyn Paths, db: &Arc<Db>, id: i64, now: i64) -> kuro_data::DataResult<()> {
    let repo = EntryRepo::new(db.clone());
    let entry = repo.by_id(id)?;
    let cover = ArtRef {
        owner: entry.kind,
        entry_id: id,
        source_id: entry.source,
        kind: ArtKind::Cover,
        url: entry.thumbnail_url.clone(),
        favorite: entry.favorite,
        last_modified: entry.cover_last_modified,
    };
    let cover_after = remove_covers(paths, &cover, now);
    let background_last_modified = entry.kind.is_anime().then(|| {
        let background = ArtRef {
            kind: ArtKind::Background,
            url: entry.background_url.clone(),
            last_modified: entry.background_last_modified(),
            ..cover.clone()
        };
        (remove_covers(paths, &background, now) != background).then_some(now)
    });
    repo.update(&EntryUpdate {
        favorite: Some(false),
        cover_last_modified: (cover_after != cover).then_some(now),
        background_last_modified: background_last_modified.flatten(),
        ..EntryUpdate::new(id)
    })
}

fn nav(kind: Kind, db: Arc<Db>) -> Nav {
    // every entry opens by its id: the page for its kind, and the reader or the player for a chapter or an episode
    let open = Rc::new(move |id| crate::screens::open_entry(kind, id));
    // one launcher hook for every opener: the external player when the preference says so and a launcher is installed
    let resume = Rc::new(move |r: crate::library::view::Resume| {
        crate::updates::open_route::open(kind, r.entry, r.item, crate::updates::open_route::route(kind, r.external, false))
    });
    let delete = {
        let db = db.clone();
        Rc::new(move |ids: Vec<i64>| {
            let db = db.clone();
            let flags = local_flags(&db, &ids);
            let offered = !flags.iter().any(|(_, local)| *local);
            crate::library::delete_dialog::show(kind, offered, move |choice| {
                let plan = delete_plan(&flags, choice);
                let Ok(paths) = SystemPaths::system() else {
                    tracing::error!("no data paths; the selected entries stay in the library");
                    return;
                };
                let done = remove_from_library(&paths, &db, &plan.unfavorite);
                if let Some(text) = crate::library::actions::partial_failure_toast(done, plan.unfavorite.len()) {
                    crate::toast::notify_for(&text, crate::library::TOAST_TIME);
                }
                if !plan.delete_downloads.is_empty() {
                    // the one download service deletes each entry's folder and its queued units, for both kinds
                    crate::updates::download_host::spawn_detached(move || {
                        let backend = crate::updates::download_host::current();
                        for id in plan.delete_downloads {
                            backend.delete_entry(kind, id);
                        }
                    });
                }
                if let Some(view) = visible(Some(kind)) {
                    view.refresh();
                }
            });
        })
    };
    Nav {
        open,
        resume,
        // the selection bar's Download menu picks the amount; the registered manager queues the items
        download: Rc::new(move |ids, amount| {
            let sink = DOWNLOAD_SINK.with(|s| s.borrow().clone());
            match sink {
                Some(sink) => sink(kind, ids, amount),
                None => tracing::debug!(?ids, "no download manager is registered for the library"),
            }
        }),
        delete,
        global_search: Rc::new(move |query| open_global_search(kind, query)),
        edit_categories: Rc::new(|| crate::ui::open("categories")),
        update: Rc::new(move |category| match UPDATE_HOST.with(|h| h.borrow().clone()) {
            Some(host) => (host.start)(kind, category),
            None => {
                tracing::debug!(?category, "no update runner is registered for the library");
                false
            }
        }),
        update_running: Rc::new(move || UPDATE_HOST.with(|h| h.borrow().clone()).is_some_and(|host| (host.running)(kind))),
    }
}

/// The app's secret store, opened on the worker thread of [`Logins`]. Without data paths nobody is logged in.
fn open_secrets() -> Arc<dyn kuro_platform::secrets::SecretStore> {
    match SystemPaths::system() {
        Ok(paths) => Arc::new(kuro_platform::secrets::open(&paths)),
        Err(_) => Arc::new(kuro_prefs::memory::MemorySecrets::default()),
    }
}

/// Whether a chapter or an episode is on disk, for the continue button's downloaded filter. A local entry has every unit.
fn item_downloaded(kind: Kind, db: Arc<Db>, index: Option<kuro_downloads::index::DownloadIndex>) -> Rc<dyn Fn(i64) -> bool> {
    Rc::new(move |id| {
        let Some(item) = ItemRepo::new(db.clone()).by_id(id).ok().flatten() else { return false };
        let Ok(entry) = EntryRepo::new(db.clone()).by_id(item.entry_id) else { return false };
        entry.source == 0
            || index.as_ref().is_some_and(|i| i.is_downloaded(kind, entry.source, &entry.title, &item.name, item.scanlator.as_deref()))
    })
}

fn deps(kind: Kind) -> Deps {
    let app = crate::data::app();
    let db = app.db.clone();
    let store = app.base.store().clone();
    let prefs = Rc::new(LibraryPrefs::new(store.clone()));
    let categories = Rc::new(kuro_domain::category::service::CategoryService::new(
        db.clone(),
        kind,
        Arc::new(LibraryPrefs::new(store)),
    ));
    let source_db = db.clone();
    let downloaded_only = app.base.downloaded_only.clone();
    // the per-row download count comes from the one download index; the tracker logins from the secret store
    let index = crate::shell::host::Shell::get().and_then(|shell| shell.download_service()).map(|service| service.index().clone());
    let item_downloaded = item_downloaded(kind, db.clone(), index.clone());
    let logins = Rc::new(Logins::new(kind, open_secrets, move || {
        glib::MainContext::default().invoke(move || {
            if let Some(view) = visible(Some(kind)) {
                view.refresh();
            }
        });
    }));
    Deps {
        kind,
        db,
        prefs,
        categories,
        facts: Facts {
            downloads: download_facts::lookup(kind, index),
            source: Arc::new(move |id| source_info(&source_db, kind, id)),
        },
        downloaded_only: Rc::new(move || downloaded_only.get()),
        logged_in: Rc::new(move || logins.get()),
        always_external: Rc::new(|| crate::misc::diet_defaults::PLAYER_ALWAYS_EXTERNAL),
        item_downloaded,
    }
}

/// Marks `kind` as watched in `slot`; `false` when it already was.
fn claim_watch(slot: &Cell<u8>, kind: Kind) -> bool {
    let fresh = slot.get() & bit(kind) == 0;
    slot.set(slot.get() | bit(kind));
    fresh
}

/// Reloads the library of `kind` whenever the database changes under it: a chapter read, an entry added from Browse, a
/// library update, a category renamed (T-08a-032). The first publication of each watch is the state the view loads
/// itself, so it is skipped; an error publication is logged and the next change tries again. Bound once per kind:
/// `refresh` reloads the visible view, so a second watch (another build of the page) would only reload it twice.
fn watch_database(kind: Kind) {
    use crate::watch::bind_watch;
    if !WATCHED.with(|w| claim_watch(w, kind)) {
        return;
    }
    let app = crate::data::app();
    let rows = on_change("library", move || refresh(kind));
    let categories = on_change("categories", move || refresh(kind));
    bind_watch(EntryRepo::new(app.db.clone()).subscribe_library(kind), move |p| rows(p.and_then(|s| (*s).as_ref().map(drop).map_err(String::clone))));
    bind_watch(app.categories(kind).repo().watch_all(), move |p| categories(p.map(drop)));
}

/// A watch callback: the first publication is ignored (the view loaded that state itself), an error is logged, and
/// every later change runs `reload`.
fn on_change(label: &'static str, reload: impl Fn() + 'static) -> impl Fn(Result<(), String>) {
    let first = Cell::new(true);
    move |published| match published {
        Err(error) => tracing::warn!(%error, watch = label, "the library watch published an error"),
        Ok(()) if first.replace(false) => {}
        Ok(()) => reload(),
    }
}

/// The library page of one kind: the view, and for the page header its search entry, its refresh button and the one
/// menu (docs/design/main-window.md). `None` when the worker thread cannot start, so the caller keeps its fallback.
pub fn build(kind: Kind) -> Option<Built> {
    let app = crate::data::app();
    // subscribed before the view's first load, so a write between the two is not missed
    watch_database(kind);
    let view = match LibraryView::new(deps(kind), nav(kind, app.db.clone())) {
        Ok(view) => view,
        Err(e) => {
            tracing::error!(error = %e, "the library view could not start");
            return None;
        }
    };
    VIEWS.with(|views| views.borrow_mut().push((kind, view.clone())));
    let pending: Vec<String> = PENDING_SEARCH.with(|p| {
        let mut p = p.borrow_mut();
        let mine = p.iter().filter(|(k, _)| *k == kind).map(|(_, q)| q.clone()).collect();
        p.retain(|(k, _)| *k != kind);
        mine
    });
    if let Some(query) = pending.last() {
        view.set_query(query);
    }
    // the view options button carries its own dot while a filter is on (T-37a-056)
    let (search, refresh, options) = view.controls();
    // search, Refresh, then the page's own buttons, as on every page (user, 2026-10-25): Categories first of them
    let extra = std::iter::once(categories_button().upcast()).chain(local_folder_buttons(kind).map(|b| b.upcast::<gtk::Widget>())).collect();
    let end = crate::ui::HeaderActions { search: Some(search), refresh: Some(refresh.upcast()), extra, options: Some(options), ..Default::default() }.widgets();
    Some(Built { content: view.widget(), end })
}

/// The Categories button, after Refresh: opens the categories page.
fn categories_button() -> gtk::Button {
    let button = crate::ui::icon_button("tag-symbolic", &tr(MsgKey::categories));
    button.connect_clicked(|_| crate::ui::open("categories"));
    button
}

/// The local folder buttons of the header: link a folder, open the Local folder. Updating one category is F5 and a
/// category tab's right-click menu; the refresh button updates the whole library, local titles included.
fn local_folder_buttons(kind: Kind) -> [gtk::Button; 2] {
    let (link, open) = local_folder_actions(kind);
    // the button opens a folder chooser, so its name ends in "…"
    let label = format!("{}…", tr(if kind == Kind::Anime { MsgKey::ui_add_local_anime_folder } else { MsgKey::ui_add_local_manga_folder }));
    let link_button = crate::ui::icon_button("folder-new-symbolic", &label);
    link_button.connect_clicked(move |_| link());
    let open_button = crate::ui::icon_button("document-open-symbolic", &tr(MsgKey::ui_open_local_folder));
    open_button.connect_clicked(move |_| open());
    [link_button, open_button]
}

/// Link a folder and open the Local folder, from the local folder module, for `kind`.
fn local_folder_actions(kind: Kind) -> (Box<dyn Fn()>, Box<dyn Fn()>) {
    let (link, open) = match kind {
        Kind::Manga => ("manga link", "manga open"),
        Kind::Anime => ("anime link", "anime open"),
    };
    (
        Box::new(move || run_local(link, || crate::local_flow::ui::link_folder(kind))),
        Box::new(move || run_local(open, || crate::local_flow::ui::open_local_folder(kind))),
    )
}

#[cfg(test)]
thread_local! {
    static LOCAL_CALLS: RefCell<Option<Vec<&'static str>>> = const { RefCell::new(None) };
}

fn run_local(name: &'static str, run: impl FnOnce()) {
    #[cfg(test)]
    if LOCAL_CALLS.with(|c| c.borrow_mut().as_mut().map(|calls| calls.push(name)).is_some()) {
        return;
    }
    let _ = name;
    run();
}

#[cfg(test)]
mod tests {
    use kuro_data::entry::Entry;
    use kuro_testkit::TempPaths;

    use super::*;

    #[test]
    fn the_database_watch_binds_once_per_kind() {
        let slot = Cell::new(0);
        assert!(claim_watch(&slot, Kind::Manga), "the first build binds it");
        assert!(!claim_watch(&slot, Kind::Manga), "a second build of the page does not");
        assert!(claim_watch(&slot, Kind::Anime), "the other kind has its own");
    }

    /// Needs a display and runs in a child process with fresh app data (see `screens::alone`).
    #[test]
    fn a_favourite_added_to_the_database_shows_in_the_mounted_library() {
        crate::screens::alone::run(module_path!(), "a_favourite_added_to_the_database_shows_in_the_mounted_library", |ctx| {
            let (_stack, _window) = crate::testing::nav();
            let built = build(Kind::Manga).expect("the library mounts");
            build(Kind::Manga).expect("a second build of the page");
            let view = visible(Some(Kind::Manga)).expect("the manga view is registered");
            assert!(crate::screens::alone::pump(ctx, || view.cover_count() == 0), "an empty library loads");
            let repo = EntryRepo::new(crate::data::app().db.clone());
            let id = repo.insert(&Entry { source: 7, url: "/b".into(), title: "Berserk".into(), ..Entry::create(Kind::Manga) }).expect("insert");
            repo.update(&EntryUpdate { favorite: Some(true), ..EntryUpdate::new(id) }).expect("favourite");
            assert!(crate::screens::alone::pump(ctx, || view.cover_count() == 1), "the favourite shows without a refresh");
            drop(built);
        });
    }

    fn test_db() -> (tempfile::TempDir, Arc<Db>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("test.db");
        let writer = kuro_db::schema_unified::open_file(&path).expect("open manga db");
        (dir, Arc::new(Db::new(&path, writer).expect("manga db handle")))
    }

    #[test]
    fn t2_del_01_removal_clears_the_favorite_and_leaves_the_rest() {
        let (_dir, db) = test_db();
        let repo = EntryRepo::new(db.clone());
        let id = repo.insert(&Entry { favorite: true, source: 7, url: "u".into(), title: "Berserk".into(), ..Entry::create(Kind::Manga) }).expect("insert");
        assert_eq!(repo.by_id(id).expect("stored").title, "Berserk");
        let paths = TempPaths::new().expect("temp paths");
        assert_eq!(remove_from_library(&paths, &db, &[id]), 1);
        assert!(!repo.by_id(id).expect("still stored").favorite);
    }

    #[test]
    fn t2_del_03_removal_deletes_the_cover_and_stamps_its_timestamp() {
        let (_dir, db) = test_db();
        let paths = TempPaths::new().expect("temp paths");
        let repo = EntryRepo::new(db.clone());
        let id = repo
            .insert(&Entry { favorite: true, source: 7, url: "u".into(), title: "Berserk".into(), thumbnail_url: Some("https://img.test/c.jpg".into()), ..Entry::create(Kind::Manga) })
            .expect("insert");
        let art = ArtRef {
            owner: Kind::Manga,
            entry_id: id,
            source_id: 7,
            kind: ArtKind::Cover,
            url: Some("https://img.test/c.jpg".into()),
            favorite: true,
            last_modified: 0,
        };
        let file = kuro_images::cover_paths::cover_file_for(&paths, &art).expect("a cover path");
        std::fs::create_dir_all(file.parent().expect("a directory")).expect("cover dir");
        std::fs::write(&file, b"png").expect("cover file");
        remove_from_library(&paths, &db, &[id]);
        assert!(!file.exists(), "the cached cover is deleted");
        assert!(repo.by_id(id).expect("stored").cover_last_modified > 0);
    }

    #[test]
    fn a_removal_with_no_cached_cover_leaves_the_cover_timestamp_alone() {
        // Kotlin's removeCovers moves the timestamp only when it deleted a file.
        let (_dir, db) = test_db();
        let paths = TempPaths::new().expect("temp paths");
        let repo = EntryRepo::new(db.clone());
        let id = repo.insert(&Entry { favorite: true, source: 7, url: "u".into(), title: "Berserk".into(), ..Entry::create(Kind::Manga) }).expect("insert");
        remove_from_library(&paths, &db, &[id]);
        assert_eq!(repo.by_id(id).expect("stored").cover_last_modified, 0);
    }

    #[test]
    fn t_08a_032_a_library_watch_skips_its_first_publication_and_reloads_on_each_change_after_it() {
        let reloads = Rc::new(Cell::new(0));
        let seen = reloads.clone();
        let watch = on_change("library", move || seen.set(seen.get() + 1));
        watch(Ok(()));
        assert_eq!(reloads.get(), 0, "the state the view already loaded");
        watch(Ok(()));
        watch(Err("database is locked".into()));
        watch(Ok(()));
        assert_eq!(reloads.get(), 2, "each change reloads; an error only logs and the watcher goes on");
    }

    #[test]
    fn t_37a_061_a_search_for_an_unbuilt_view_waits_for_its_mount() {
        PENDING_SEARCH.with(|p| p.borrow_mut().clear());
        inject_search(Kind::Manga, "one piece");
        let waiting = PENDING_SEARCH.with(|p| p.borrow().clone());
        assert_eq!(waiting, vec![(Kind::Manga, "one piece".to_owned())]);
        PENDING_SEARCH.with(|p| p.borrow_mut().clear());
    }

    #[test]
    fn an_unknown_source_shows_its_id_and_no_language() {
        let (_dir, db) = test_db();
        assert_eq!(source_info(&db, Kind::Manga, 42), SourceInfo { name: "42".into(), language: String::new() });
    }

    /// The header buttons: the local folders, each reaching the local flow, and Categories. Update category is not one
    /// (F5 and the tab's right-click menu).
    #[test]
    fn the_header_buttons_reach_the_local_flow() {
        crate::testing::with_display(|| {
            for (kind, word) in [(Kind::Manga, "manga"), (Kind::Anime, "anime")] {
                let [link, open] = local_folder_buttons(kind);
                let article = if kind == Kind::Anime { "an" } else { "a" };
                assert_eq!(link.tooltip_text().as_deref(), Some(format!("Link {article} {word} folder…").as_str()));
                assert_eq!(open.tooltip_text().as_deref(), Some("Open the Local folder"));
                LOCAL_CALLS.with(|c| *c.borrow_mut() = Some(Vec::new()));
                link.emit_clicked();
                open.emit_clicked();
                let calls = LOCAL_CALLS.with(|c| c.borrow_mut().take()).unwrap_or_default();
                assert_eq!(calls, [format!("{word} link"), format!("{word} open")]);
            }
            let button = categories_button();
            assert_eq!(button.tooltip_text().as_deref(), Some(tr(MsgKey::categories).as_str()));
            assert_eq!(button.icon_name().as_deref(), Some("tag-symbolic"));
        });
    }
}
