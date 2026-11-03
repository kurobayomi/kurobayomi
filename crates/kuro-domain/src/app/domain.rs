//! The domain container (`DomainModule.kt`, `SYDomainModule.kt`; T-10a-001, PARTIAL). It owns the handles the interactors
//! share and gives each one out by a cheap clone, so nothing is a global.
//!
//! Built: the source, backup and download preferences, the storage manager, the source toggles, the entry and item
//! repositories (accessors above). The construction test uses `kuro-testkit` fakes.
//!
//! Deferred, with the exact gap:
//! - None of the 47 `DomainModule` rows owned by area 10 has its accessor on `Domain` yet. Some
//!   of their behaviour is built under other names and is not wired here: `TrustExtension` (`extension/trust.rs`).
//!   Per-extension incognito is cut (settings diet); only the global incognito flag exists. Most of the remaining rows have no type in the workspace yet.
//! - `Domain::start(replay)` is built (T-10a-029): it schedules the delayed tracking worker when the stored queue is
//!   non-empty. The store is one file with a `manga` and an `anime` map, so it is one handle, not two singletons.
//! - `TrackPreferences` is area 27's, and its task is T-10a-026 (milestone M08).

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use kuro_data::entry::EntryRepo;
use kuro_data::item::ItemRepo;
use kuro_db::handle::Db;
use kuro_prefs::source::SourcePrefs;
use kuro_prefs::store::PrefStore;

use super::source::SourceToggles;
use crate::backup::BackupPrefs;
use crate::download::prefs::DownloadPrefs;
use crate::storage::StorageManager;
use crate::track::delayed::{DelayedTrackingStore, DelayedTrackingWorker, QUEUE_FILE, TrackReplay};

/// What a [`Domain`] is built from: the preference store, the database, and the base storage directory used until
/// the user picks one.
pub struct DomainDeps {
    pub prefs: PrefStore,
    pub db: Arc<Db>,
    /// The platform data directory plus `kurobayomi`, from `kuro_platform::paths::Paths::data_dir`.
    pub default_storage: PathBuf,
}

/// The shared domain handles. Clones are cheap: every field is an `Arc` or a handle that holds one.
#[derive(Clone)]
pub struct Domain {
    source_prefs: SourcePrefs,
    backup_prefs: BackupPrefs,
    download_prefs: DownloadPrefs,
    storage: Arc<StorageManager>,
    toggles: Arc<SourceToggles>,
    entries: EntryRepo,
    items: ItemRepo,
    db: Arc<Db>,
    delayed_store: Arc<DelayedTrackingStore>,
    delayed_worker: Arc<OnceLock<Arc<DelayedTrackingWorker>>>,
}

impl Domain {
    /// Builds the container. Nothing is read from disk here; the storage tree is created on first use (see
    /// `StorageManager::ensure_tree`).
    pub fn new(deps: DomainDeps) -> Self {
        let DomainDeps { prefs, db, default_storage } = deps;
        Self {
            source_prefs: SourcePrefs::new(prefs.clone()),
            backup_prefs: BackupPrefs::new(&prefs),
            download_prefs: DownloadPrefs::new(&prefs),
            storage: Arc::new(StorageManager::open(&prefs, default_storage.clone())),
            toggles: Arc::new(SourceToggles::default()),
            entries: EntryRepo::new(Arc::clone(&db)),
            items: ItemRepo::new(Arc::clone(&db)),
            db,
            delayed_store: Arc::new(DelayedTrackingStore::new(default_storage.join(QUEUE_FILE))),
            delayed_worker: Arc::new(OnceLock::new()),
        }
    }

    pub fn source_prefs(&self) -> &SourcePrefs {
        &self.source_prefs
    }

    pub fn backup_prefs(&self) -> &BackupPrefs {
        &self.backup_prefs
    }

    pub fn download_prefs(&self) -> &DownloadPrefs {
        &self.download_prefs
    }

    pub fn storage(&self) -> Arc<StorageManager> {
        Arc::clone(&self.storage)
    }

    pub fn source_toggles(&self) -> Arc<SourceToggles> {
        Arc::clone(&self.toggles)
    }

    /// The shared entry repository, both kinds.
    pub fn entries(&self) -> &EntryRepo {
        &self.entries
    }

    /// The shared item repository, both kinds.
    pub fn items(&self) -> &ItemRepo {
        &self.items
    }

    /// The delayed tracking queue (`<data dir>/tracking_queue.json`).
    pub fn delayed_tracking(&self) -> Arc<DelayedTrackingStore> {
        Arc::clone(&self.delayed_store)
    }

    /// The delayed tracking worker, once [`Domain::start`] has built it.
    pub fn delayed_worker(&self) -> Option<Arc<DelayedTrackingWorker>> {
        self.delayed_worker.get().cloned()
    }

    /// Starts the domain's background work: builds the delayed tracking worker over `replay` and schedules it when the
    /// stored queue is non-empty (a replacement for WorkManager keeping the job across restarts). A second call keeps the
    /// first worker and schedules it again if the queue is non-empty. Needs a tokio runtime.
    pub fn start(&self, replay: Arc<dyn TrackReplay>) {
        let worker = self
            .delayed_worker
            .get_or_init(|| Arc::new(DelayedTrackingWorker::new(Arc::clone(&self.delayed_store), replay)));
        if !self.delayed_store.is_empty() {
            worker.schedule();
        }
    }

    /// The database handle.
    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }
}

#[cfg(test)]
mod tests {
    use kuro_testkit::{InMemorySettings, TempDir, TestDb};

    use kuro_platform::kind::Kind;
    use super::*;

    /// The container's inputs built from `kuro-testkit` fakes: in-memory settings, and the migrated database in `dir`.
    fn deps(dir: &TempDir, settings: &InMemorySettings) -> DomainDeps {
        DomainDeps {
            prefs: PrefStore::clone(settings),
            db: TestDb::handle(dir),
            default_storage: dir.path().join("no-such-base"),
        }
    }

    #[test]
    fn a_domain_builds_from_testkit_fakes_and_shares_its_handles() {
        let dir = TempDir::new().expect("temp dir");
        let settings = InMemorySettings::new();
        let domain = Domain::new(deps(&dir, &settings));
        assert!(domain.source_prefs().enabled_languages.get().contains("en"));
        assert_eq!(domain.backup_prefs().interval.get(), 12);
        let clone = domain.clone();
        assert!(Arc::ptr_eq(&domain.storage(), &clone.storage()), "clones share the storage manager");
        assert!(Arc::ptr_eq(&domain.source_toggles(), &clone.source_toggles()));
        assert!(Arc::ptr_eq(domain.db(), clone.db()), "clones share the database");
        assert!(domain.entries().by_id(1).is_err() && domain.items().by_id(1).unwrap().is_none());
    }

    struct Online;

    #[async_trait::async_trait]
    impl TrackReplay for Online {
        async fn wait_online(&self) {}
        async fn replay(&self, _kind: Kind, _track_id: i64, _progress: f32) -> crate::track::delayed::ReplayOutcome {
            crate::track::delayed::ReplayOutcome::Done
        }
    }

    #[tokio::test]
    async fn start_schedules_the_worker_only_when_the_stored_queue_is_not_empty() {
        let dir = TempDir::new().expect("temp dir");
        let settings = InMemorySettings::new();
        let domain = Domain::new(deps(&dir, &settings));
        domain.start(Arc::new(Online));
        let worker = domain.delayed_worker().expect("built by start");
        assert!(!worker.is_running(), "an empty queue schedules nothing");

        domain.delayed_tracking().add(Kind::Manga, 1, 3.0).expect("queue");
        let again = Domain::new(deps(&dir, &settings));
        again.start(Arc::new(Online));
        for _ in 0..200 {
            if again.delayed_tracking().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(again.delayed_tracking().is_empty(), "the stored item was replayed after the restart");
    }

    #[test]
    fn the_container_reads_the_same_preference_store_it_was_built_from() {
        let dir = TempDir::new().expect("temp dir");
        let settings = InMemorySettings::new();
        let domain = Domain::new(deps(&dir, &settings));
        let _ = domain.backup_prefs().interval.set(0);
        assert_eq!(settings.get_raw("backup_interval"), Some(kuro_prefs::values::PrefValue::Int(0)));
        // The default base is created on first use, so the user is never asked for a folder.
        assert_eq!(domain.storage().base().unwrap(), dir.path().join("no-such-base"));
    }
}
