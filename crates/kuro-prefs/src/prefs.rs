//! The preference aggregate: every section built once over one shared store.
//!
//! Built in the application context at startup, before any widget is created. Sections that are
//! not in this file yet (tracking, downloads, backup, storage, the subtitle, audio and
//! advanced player sections) are added with their own tasks.

use std::sync::Arc;

use kuro_platform::paths::Paths;
use kuro_platform::secrets::SecretStore;

use crate::PrefError;
use crate::backup::{BackupPrefs, StoragePrefs};
use crate::base::BasePrefs;
use crate::library::LibraryPrefs;
use crate::migrate::{self, Migration};
use crate::network::NetworkPrefs;
use crate::player::PlayerPrefs;
use crate::reader::ReaderPrefs;
use crate::settings_load;
use crate::source::SourcePrefs;
use crate::store::PrefStore;
use crate::ui::UiPrefs;

/// Every preference section. Clones of a section share the store.
pub struct Prefs {
    pub base: BasePrefs,
    pub ui: UiPrefs,
    pub sources: SourcePrefs,
    pub library: LibraryPrefs,
    pub network: NetworkPrefs,
    pub reader: ReaderPrefs,
    pub player: PlayerPrefs,
    /// `[backup]`: the automatic backup interval and its app state (T-02a-024).
    pub backup: BackupPrefs,
    /// `[storage]`: the chosen base storage folder (T-02a-024).
    pub storage: StoragePrefs,
    /// Credential storage for tracker secrets (area 02, part 2 reads through it).
    pub secrets: Arc<dyn SecretStore>,
    store: PrefStore,
}

impl Prefs {
    /// Opens `config.toml` and `state.toml` from `paths`, runs the startup migrations for
    /// `version_code` on the raw store, builds every section and repairs invalid stored enum and
    /// object values to their defaults.
    pub fn open(paths: &dyn Paths, secrets: Arc<dyn SecretStore>, version_code: i32) -> Result<Self, PrefError> {
        Self::open_with(paths, secrets, version_code, migrate::list::MIGRATIONS)
    }

    /// [`Prefs::open`] with an explicit step list. The steps see the raw keys before any section exists.
    pub(crate) fn open_with(
        paths: &dyn Paths,
        secrets: Arc<dyn SecretStore>,
        version_code: i32,
        steps: &[Migration],
    ) -> Result<Self, PrefError> {
        let store = settings_load::open(paths)?;
        migrate::run(&store, version_code, steps)?;
        let prefs = Self::with_store(store, secrets);
        prefs.repair()?;
        Ok(prefs)
    }

    /// Builds the sections over an existing store. Nothing is repaired.
    pub fn with_store(store: PrefStore, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            base: BasePrefs::new(store.clone()),
            ui: UiPrefs::new(store.clone()),
            sources: SourcePrefs::new(store.clone()),
            library: LibraryPrefs::new(store.clone()),
            network: NetworkPrefs::new(store.clone()),
            reader: ReaderPrefs::new(store.clone()),
            player: PlayerPrefs::new(store.clone()),
            backup: BackupPrefs::new(store.clone()),
            storage: StoragePrefs::new(store.clone()),
            secrets,
            store,
        }
    }

    /// Every section over an in-memory store and an in-memory secret store (T-02a-033). No file and no thread is
    /// involved, so a test or preview needs no runtime.
    pub fn in_memory() -> Self {
        Self::with_store(PrefStore::in_memory(), Arc::new(crate::memory::MemorySecrets::default()))
    }

    /// The store every section reads and writes.
    pub fn store(&self) -> &PrefStore {
        &self.store
    }

    /// Startup repair: each section rewrites its invalid stored values to the defaults.
    fn repair(&self) -> Result<(), PrefError> {
        self.base.repair()?;
        self.ui.repair()?;
        self.sources.repair()?;
        self.library.repair()?;
        self.network.repair()?;
        self.reader.repair()?;
        self.player.repair()?;
        self.backup.repair()?;
        self.storage.repair()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{MemorySecrets, TestDir, TestPaths};
    use crate::values::PrefValue;

    fn secrets() -> Arc<dyn SecretStore> {
        Arc::new(MemorySecrets::default())
    }

    #[test]
    fn sections_share_one_store() {
        let prefs = Prefs::with_store(PrefStore::in_memory(), secrets());
        prefs.base.incognito_mode.set(true).unwrap();
        assert_eq!(
            prefs.store().get_raw("__APP_STATE_incognito_mode"),
            Some(PrefValue::Bool(true))
        );
        prefs.reader.default_reading_mode.set(4).unwrap();
        assert_eq!(prefs.reader.default_reading_mode.get(), 4);
        assert_eq!(prefs.store().get_raw("pref_default_reading_mode_key"), Some(PrefValue::Int(4)));
    }

    #[test]
    fn in_memory_prefs_keep_values_and_publish_change_events() {
        let prefs = Prefs::in_memory();
        let changes = prefs.store().subscribe();
        prefs.base.incognito_mode.set(true).unwrap();
        assert!(prefs.base.incognito_mode.get());
        assert!(changes.try_recv().is_ok(), "a set publishes a change event");
    }

    #[test]
    fn open_builds_defaults_on_first_run() {
        let paths = TestPaths(TestDir::new("prefs-first-run"));
        Prefs::open(&paths, secrets(), 1).unwrap();
    }




}
