//! The `FavoriteStore` over the shared entry repository: the production side of `entries`, one store for both kinds.
//! Every write names the columns it sets (`EntryUpdate` keeps the rest), so flags written by the default-flags step and
//! the stored title and status survive a stale listed snapshot (B-EC-12, E-BR-02). This is the database adapter; the
//! browse pages only see the trait.

use std::collections::BTreeSet;
use std::sync::Arc;

use kuro_data::category::CategoryRepo;
use kuro_data::entry::{Entry, EntryRepo, EntryUpdate};
use kuro_domain::entries::defaults::apply_item_defaults;
use kuro_domain::entries::network_to_local::network_to_local;
use kuro_local::ids::is_local_source;
use kuro_platform::kind::Kind;
use kuro_prefs::pref::Pref;
use kuro_prefs::store::PrefStore;
use tokio::sync::watch;

use super::entries::{CategoryChoice, FavoriteStore, LocalEntry, TrackerBinder};
use super::favorite::FavoriteWrite;

/// Deletes the cached cover files of an entry and returns how many went (the cover cache is the image layer's).
pub type CoverClearer = Arc<dyn Fn(i64) -> usize + Send + Sync>;

/// The store of one kind. The local source keeps its files: removing one of its entries deletes no cover and no
/// background (B§4.8.1). The background only exists for anime.
pub struct RepoStore {
    kind: Kind,
    entries: EntryRepo,
    categories: CategoryRepo,
    default_category: Pref<i32>,
    clear_cover: CoverClearer,
    clear_background: CoverClearer,
    /// The preferences the default item flags come from. `None` skips the defaults.
    defaults: Option<PrefStore>,
    /// Binds the enhanced trackers of an added entry. `None` binds nothing.
    binder: Option<TrackerBinder>,
}

impl RepoStore {
    pub fn new(
        kind: Kind,
        entries: EntryRepo,
        categories: CategoryRepo,
        default_category: Pref<i32>,
        clear_cover: CoverClearer,
        clear_background: CoverClearer,
    ) -> Self {
        Self { kind, entries, categories, default_category, clear_cover, clear_background, defaults: None, binder: None }
    }

    /// Turns on the default item flags of an add, from `defaults`.
    pub fn with_default_flags(mut self, defaults: PrefStore) -> Self {
        self.defaults = Some(defaults);
        self
    }

    /// Turns on the enhanced tracker bindings of an add, through `binder`.
    pub fn with_tracker_binder(mut self, binder: TrackerBinder) -> Self {
        self.binder = Some(binder);
        self
    }

    /// Runs one of the file clearers, unless the entry belongs to the local source (its files are not a cache).
    fn clear_files(&self, id: i64, clear: &CoverClearer) -> usize {
        match self.entries.by_id(id) {
            Ok(entry) if !is_local_source(entry.source) => clear(id),
            Ok(_) => 0,
            Err(error) => {
                tracing::warn!(entry = id, %error, "the files of an entry could not be looked up; none were deleted");
                0
            }
        }
    }

    fn write(&self, update: &EntryUpdate) -> Result<(), String> {
        self.entries.update(update).map_err(text)
    }
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn urls(list: &[Entry]) -> BTreeSet<String> {
    list.iter().map(|e| e.url.clone()).collect()
}

impl FavoriteStore for RepoStore {
    /// A listed entry is found by `(url, source)`, or inserted as a non-favourite row with the `Entry::create` defaults
    /// (`NetworkToLocal`). An existing favourite shows its stored title; an existing non-favourite shows the listed one,
    /// in memory only (E-BR-10).
    fn local_entry(&self, source_id: i64, url: &str, title: &str, cover: Option<&str>) -> Result<LocalEntry, String> {
        let listed = Entry {
            source: source_id,
            url: url.to_owned(),
            title: title.to_owned(),
            thumbnail_url: cover.map(str::to_owned),
            ..Entry::create(self.kind)
        };
        let local = network_to_local(&self.entries, &listed).map_err(text)?;
        // a title Browse shows gets its community score from the score site's bulk lists, when they hold it
        crate::community::browsed(self.kind, local.id, title);
        Ok(LocalEntry { id: local.id, favorite: local.favorite, title: local.title })
    }

    fn duplicates(&self, id: i64, title: &str) -> Result<Vec<i64>, String> {
        Ok(self.entries.duplicates(self.kind, id, title).map_err(text)?.into_iter().map(|e| e.id).collect())
    }

    fn categories(&self) -> Result<Vec<CategoryChoice>, String> {
        let all = self.categories.all().map_err(text)?;
        Ok(all.into_iter().filter(|c| c.id != 0).map(|c| CategoryChoice { id: c.id, name: c.name }).collect())
    }

    fn default_category(&self) -> i64 {
        i64::from(self.default_category.get())
    }

    fn write_favorite(&self, id: i64, write: FavoriteWrite) -> Result<(), String> {
        self.write(&EntryUpdate { favorite: Some(write.favorite), date_added: Some(write.date_added), ..EntryUpdate::new(id) })
    }

    fn set_categories(&self, id: i64, ids: &[i64]) -> Result<(), String> {
        self.entries.set_categories(id, ids).map_err(text)
    }

    fn entry_categories(&self, id: i64) -> Result<Vec<i64>, String> {
        Ok(self.categories.by_entry(id).map_err(text)?.into_iter().filter(|c| c.id != 0).map(|c| c.id).collect())
    }

    fn clear_cover(&self, id: i64) -> usize {
        self.clear_files(id, &self.clear_cover)
    }

    fn stamp_cover(&self, id: i64, now_ms: i64) -> Result<(), String> {
        self.write(&EntryUpdate { cover_last_modified: Some(now_ms), ..EntryUpdate::new(id) })
    }

    fn clear_background(&self, id: i64) -> usize {
        if self.kind.is_anime() { self.clear_files(id, &self.clear_background) } else { 0 }
    }

    fn stamp_background(&self, id: i64, now_ms: i64) -> Result<(), String> {
        if !self.kind.is_anime() {
            return Ok(());
        }
        self.write(&EntryUpdate { background_last_modified: Some(now_ms), ..EntryUpdate::new(id) })
    }

    fn after_add(&self, id: i64) -> Result<(), String> {
        let entry = self.entries.by_id(id).map_err(text)?;
        let flags = match &self.defaults {
            Some(prefs) => apply_item_defaults(&self.entries, prefs, &entry).map(|_| ()).map_err(text),
            None => Ok(()),
        };
        // the trackers are asked even when the flags failed: they do not depend on each other
        if let Some(bind) = &self.binder {
            bind(entry.id, &entry.title, entry.source);
        }
        // the new title gets its community score without waiting for the hourly check
        crate::community::wake();
        flags
    }

    /// Needs a tokio runtime: the follower task that republishes the subscription runs on it.
    fn favorite_urls(&self, source_id: i64) -> watch::Receiver<BTreeSet<String>> {
        let mut subscription = self.entries.subscribe_favorites_by_source(self.kind, source_id);
        let initial = subscription.current().and_then(|p| p.ok()).map(|list| urls(&list)).unwrap_or_default();
        let (tx, rx) = watch::channel(initial);
        tokio::spawn(async move {
            while let Some(published) = subscription.changed().await {
                let Ok(list) = published else { continue };
                if tx.send(urls(&list)).is_err() {
                    break;
                }
            }
        });
        rx
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use kuro_db::handle::Db;
    use kuro_domain::category::target::AddTarget;
    use kuro_domain::entries::defaults::item_default_word;

    use super::*;
    use crate::browse::entries::{add_entry, remove_entry};
    use crate::browse::favorite::{AddOutcome, toggle_write};

    const KINDS: [Kind; 2] = [Kind::Manga, Kind::Anime];

    struct Rig {
        _dir: tempfile::TempDir,
        store: RepoStore,
        entries: EntryRepo,
        categories: CategoryRepo,
        db: Arc<Db>,
        default: Pref<i32>,
    }

    fn rig_with(kind: Kind, covers: Arc<AtomicUsize>, backgrounds: Arc<AtomicUsize>, default: i32) -> Rig {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("test.db");
        let writer = kuro_db::schema_unified::open_file(&path).expect("schema");
        let db = Arc::new(Db::new(&path, writer).expect("db"));
        let entries = EntryRepo::new(Arc::clone(&db));
        let categories = CategoryRepo::new(Arc::clone(&db), kind);
        let default = PrefStore::in_memory().pref("default_category", default);
        let clear_cover: CoverClearer = Arc::new(move |_| covers.load(Ordering::SeqCst));
        let clear_background: CoverClearer = Arc::new(move |_| backgrounds.load(Ordering::SeqCst));
        let store = RepoStore::new(kind, entries.clone(), categories.clone(), default.clone(), clear_cover, clear_background);
        Rig { _dir: dir, store, entries, categories, db, default }
    }

    fn rig(kind: Kind) -> Rig {
        rig_with(kind, Arc::default(), Arc::default(), 0)
    }

    fn entry(rig: &Rig, source: i64, url: &str, title: &str) -> i64 {
        rig.store.local_entry(source, url, title, None).expect("entry").id
    }

    fn category(rig: &Rig, name: &str) -> i64 {
        kuro_testkit::insert_category(&rig.db, rig.categories.kind(), name)
    }

    #[test]
    fn an_unknown_entry_is_inserted_as_a_non_favourite_and_found_again() {
        for kind in KINDS {
            let rig = rig(kind);
            let first = rig.store.local_entry(5, "/series/1", "One", Some("http://c/1.png")).expect("entry");
            assert!(!first.favorite, "{kind}");
            let again = rig.store.local_entry(5, "/series/1", "Renamed", None).expect("entry");
            assert_eq!(again.id, first.id, "{kind}: the same (url, source) is one row");
            assert_eq!(rig.entries.by_id(first.id).expect("row").thumbnail_url.as_deref(), Some("http://c/1.png"), "{kind}");
        }
    }

    #[test]
    fn adding_writes_the_favourite_and_its_date_and_nothing_else() {
        for kind in KINDS {
            let rig = rig(kind);
            let id = entry(&rig, 5, "/series/1", "Stored title");
            rig.entries.update(&EntryUpdate { status: Some(2), ..EntryUpdate::new(id) }).expect("status");
            rig.entries.update_item_flags(id, |_| 0b1010).expect("flags");

            rig.store.write_favorite(id, toggle_write(false, 4_000)).expect("favourite");

            let row = rig.entries.by_id(id).expect("row");
            assert!(row.favorite, "{kind}");
            assert_eq!(row.date_added, 4_000);
            assert_eq!(row.title, "Stored title", "a listed title does not overwrite the stored one (E-BR-02)");
            assert_eq!(row.status, 2, "the stored status survives the write");
            assert_eq!(row.item_flags, 0b1010, "the flags written earlier survive");
        }
    }

    #[test]
    fn a_listed_favourite_keeps_its_stored_title() {
        for kind in KINDS {
            let rig = rig(kind);
            let id = entry(&rig, 5, "/series/1", "Stored");
            rig.store.write_favorite(id, toggle_write(false, 1)).expect("favourite");

            let again = rig.store.local_entry(5, "/series/1", "Renamed upstream", None).expect("entry");
            assert_eq!((again.id, again.favorite, again.title.as_str()), (id, true, "Stored"), "{kind}");
        }
    }

    #[test]
    fn removing_stamps_the_cover_when_files_went_and_the_background_for_anime_only() {
        for kind in KINDS {
            let rig = rig_with(kind, Arc::new(AtomicUsize::new(1)), Arc::new(AtomicUsize::new(1)), 0);
            let id = entry(&rig, 5, "/series/1", "One");
            add_entry(&rig.store, id, 4_000);

            remove_entry(&rig.store, id, 9_000).expect("removed");

            let row = rig.entries.by_id(id).expect("row");
            assert!(!row.favorite, "{kind}");
            assert_eq!(row.date_added, 0);
            assert_eq!(row.cover_last_modified, 9_000, "a deleted cover file stamps the row");
            let expected = if kind.is_anime() { 9_000 } else { 0 };
            assert_eq!(row.background_last_modified(), expected, "{kind}: only anime has a background");
        }
    }

    #[test]
    fn no_files_going_means_no_stamp() {
        for kind in KINDS {
            let rig = rig(kind);
            let id = entry(&rig, 5, "/series/1", "One");
            add_entry(&rig.store, id, 4_000);
            remove_entry(&rig.store, id, 9_000).expect("removed");
            let row = rig.entries.by_id(id).expect("row");
            assert_eq!((row.cover_last_modified, row.background_last_modified()), (0, 0), "{kind}");
        }
    }

    #[test]
    fn the_local_source_deletes_no_files_on_remove() {
        for kind in KINDS {
            let rig = rig_with(kind, Arc::new(AtomicUsize::new(3)), Arc::default(), 0);
            let id = entry(&rig, 0, "/local/1", "Local one");
            add_entry(&rig.store, id, 4_000);

            remove_entry(&rig.store, id, 9_000).expect("removed");

            assert_eq!(rig.entries.by_id(id).expect("row").cover_last_modified, 0, "{kind}: the local source's files are not a cache");
        }
    }

    #[test]
    fn an_unset_default_category_asks_and_a_set_one_is_written_before_the_favourite() {
        for kind in KINDS {
            let rig = rig_with(kind, Arc::default(), Arc::default(), -1);
            let action = category(&rig, "Action");
            let id = entry(&rig, 5, "/series/1", "One");

            assert_eq!(add_entry(&rig.store, id, 1), AddOutcome::Chooser, "{kind}: default -1 opens the chooser");
            assert!(!rig.entries.by_id(id).expect("row").favorite, "the chooser writes nothing");

            rig.default.set(action as i32).expect("default");
            assert_eq!(add_entry(&rig.store, id, 2), AddOutcome::Added { category_error: None });
            let links: Vec<i64> = rig.categories.by_entry(id).expect("links").into_iter().map(|c| c.id).collect();
            assert_eq!(links, vec![action]);
            assert!(rig.entries.by_id(id).expect("row").favorite);
        }
    }

    #[test]
    fn an_add_with_the_default_target_writes_no_categories_and_then_the_favourite() {
        // AddTarget::Categories(vec![]) is the Default category: the link set is emptied, then the favourite is written.
        for kind in KINDS {
            let rig = rig(kind);
            let id = entry(&rig, 5, "/series/1", "One");
            let outcome = crate::browse::favorite::add(
                &AddTarget::Categories(vec![]),
                7,
                |ids| rig.store.set_categories(id, ids),
                |write| rig.store.write_favorite(id, write),
            );
            assert_eq!(outcome, AddOutcome::Added { category_error: None }, "{kind}");
            assert!(rig.entries.by_id(id).expect("row").favorite);
        }
    }

    #[test]
    fn a_duplicate_is_another_entry_with_the_same_title_ignoring_case() {
        for kind in KINDS {
            let rig = rig(kind);
            let first = entry(&rig, 5, "/series/1", "Same Show");
            let second = entry(&rig, 6, "/other/1", "same show");
            rig.store.write_favorite(first, toggle_write(false, 1)).expect("favourite");

            assert_eq!(rig.store.duplicates(second, "same show").expect("duplicates"), vec![first], "{kind}");
            assert!(rig.store.duplicates(first, "Same Show").expect("duplicates").is_empty(), "an entry is not its own duplicate");
        }
    }

    #[test]
    fn the_entry_categories_are_the_user_category_links_of_the_row() {
        for kind in KINDS {
            let rig = rig(kind);
            let id = entry(&rig, 5, "/series/1", "One");
            let reading = category(&rig, "Reading");
            assert_eq!(rig.store.entry_categories(id), Ok(Vec::new()), "{kind}: a new entry has no links");
            rig.store.set_categories(id, &[reading]).expect("links");
            assert_eq!(rig.store.entry_categories(id), Ok(vec![reading]));
            let rows = crate::browse::entries::chooser_rows(&rig.store.categories().expect("list"), &rig.store.entry_categories(id).expect("links"));
            assert_eq!(rows.iter().map(|(c, on)| (c.id, *on)).collect::<Vec<_>>(), [(reading, true)]);
        }
    }

    #[test]
    fn the_favourite_urls_of_a_source_follow_the_library() {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_time().build().expect("runtime");
        let _entered = rt.enter();
        for kind in KINDS {
            let rig = rig(kind);
            let id = entry(&rig, 5, "/series/1", "One");
            let mut urls = rig.store.favorite_urls(5);
            assert!(urls.borrow().is_empty());
            add_entry(&rig.store, id, 1);
            let seen = rt.block_on(async { tokio::time::timeout(Duration::from_secs(3), urls.wait_for(|set| set.contains("/series/1"))).await });
            assert!(matches!(seen, Ok(Ok(_))), "{kind}: the added entry reaches the follower");
        }
    }

    #[test]
    fn an_add_applies_the_default_item_flags_and_binds_the_enhanced_trackers() {
        for kind in KINDS {
            let rig = rig(kind);
            let prefs = PrefStore::in_memory();
            let word = item_default_word(&prefs, kind);
            let bound: Arc<std::sync::Mutex<Vec<(i64, String, i64)>>> = Arc::default();
            let log = Arc::clone(&bound);
            let binder: TrackerBinder = Arc::new(move |id, title, source| log.lock().expect("lock").push((id, title.to_owned(), source)));
            let store = rig.store.with_default_flags(prefs).with_tracker_binder(binder);
            let id = store.local_entry(5, "/series/1", "One", None).expect("entry").id;
            rig.entries.update_item_flags(id, |_| word ^ 0b1000).expect("scrambled flags");
            assert_ne!(rig.entries.by_id(id).expect("row").item_flags, word);

            assert_eq!(add_entry(&store, id, 4_000), AddOutcome::Added { category_error: None });

            let row = rig.entries.by_id(id).expect("row");
            assert!(row.favorite, "{kind}");
            assert_eq!(row.date_added, 4_000);
            assert_eq!(row.item_flags, word, "the default item flags are written");
            assert_eq!(*bound.lock().expect("lock"), [(id, "One".to_owned(), 5)], "the trackers are asked once, with the source id");
        }
    }

    #[test]
    fn a_chooser_that_has_not_confirmed_binds_nothing() {
        let rig = rig_with(Kind::Manga, Arc::default(), Arc::default(), -1);
        let reading = category(&rig, "Reading");
        let bound: Arc<std::sync::Mutex<Vec<i64>>> = Arc::default();
        let log = Arc::clone(&bound);
        let store = rig.store.with_tracker_binder(Arc::new(move |id, _, _| log.lock().expect("lock").push(id)));
        let id = store.local_entry(5, "/series/1", "One", None).expect("entry").id;
        assert_eq!(add_entry(&store, id, 1), AddOutcome::Chooser);
        assert!(bound.lock().expect("lock").is_empty(), "the add asks first; the confirm path binds");
        crate::browse::entries::add_entry_to(&store, id, 2, vec![reading]);
        assert_eq!(*bound.lock().expect("lock"), [id], "the confirm path reaches after_add");
    }
}
