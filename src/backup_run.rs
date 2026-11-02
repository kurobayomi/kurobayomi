//! The GTK-free half of the backup host: the adapters that create and restore a backup over the app's databases and
//! preferences, and the `kurobayomi --run-backup` entry point (T-21a-041, T-22a-041). The GUI half is `backup_host`.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Local;
use kuro_backup::auto::{AutoConfig, AutoHost, OnceOutcome, PowerState, run_once};
use kuro_backup::creator::db::{DbLibrary, DirArtifacts, StorePrefs};
use kuro_backup::creator::{CreateInputs, SourceRegistry};
use kuro_backup::error::BackupError;
use kuro_backup::file::{Target, create_file_report};
use kuro_backup::options::BackupOptions;
use kuro_backup::restore::driver::Targets;
use kuro_backup::restore::job::{RestoreEnd, run_backup};
use kuro_backup::restore::options::RestoreOptions;
use kuro_backup::restore::progress::ProgressEvent;
use kuro_backup::restore::prefs::IntervalScheduler;
use kuro_backup::restore::screen::StopFlag;
use kuro_backup::restore::extensions::legacy_repo_urls;
use kuro_backup::schema::Backup;
use kuro_backup::validate::TrackerRegistry;
use kuro_db::handle::Db;
use kuro_db::schema_unified;
use kuro_domain::extension_repo::{HttpGet, import_legacy_repos};
use kuro_platform::paths::Paths;
use kuro_prefs::pref::Pref;
use kuro_prefs::store::PrefStore;

use crate::pref_model::stores_backend::{Ops as _, RepoOps};

/// `backup_interval` (hours, default 12) and the two app-state stamps of the automatic backup.
pub(crate) const INTERVAL_KEY: &str = "backup_interval";
const LAST_AUTO_KEY: &str = "__APP_STATE_last_auto_backup_timestamp";
pub(crate) const ENABLED_AT_KEY: &str = "__APP_STATE_backup_enabled_at";

/// No tracker is signed in until the tracker area registers its own, so a check lists every tracker of a file as missing.
/// Its source side is not used by the app: backups and checks read sources through `LiveRegistry`.
pub struct NothingInstalled;

impl SourceRegistry for NothingInstalled {
    fn is_installed(&self, _source_id: i64) -> bool {
        false
    }
    fn name_of(&self, _source_id: i64) -> String {
        String::new()
    }
    fn stub_label(&self, source_id: i64) -> String {
        source_id.to_string()
    }
}

impl TrackerRegistry for NothingInstalled {
    fn name(&self, _tracker_id: i32) -> Option<String> {
        None
    }
    fn logged_in(&self, _tracker_id: i32) -> bool {
        false
    }
}

/// The database and the preference store one backup run reads.
#[derive(Clone)]
pub struct Library {
    pub prefs: PrefStore,
    pub db: Arc<Db>,
    /// `<data>/extensions`, where the installed extension packages live.
    pub extensions_dir: PathBuf,
}

impl Library {
    /// Writes one backup to `target`. Returns its path and the extension packages that were skipped.
    pub fn create(&self, opts: &BackupOptions, target: &Target) -> Result<(PathBuf, Vec<String>), BackupError> {
        let library = DbLibrary::new(self.db.clone());
        let prefs = StorePrefs(&self.prefs);
        let artifacts = DirArtifacts { anime_dir: self.extensions_dir.join("anime"), manga_dir: self.extensions_dir.clone() };
        let inputs = CreateInputs {
            library: &library,
            sources: &crate::library_update::live::LiveRegistry,
            prefs: &prefs,
            source_prefs: &prefs,
            // read only when the options ask for extensions, so the default is off (T-21a-019)
            extensions: Some(&artifacts),
        };
        create_file_report(&inputs, opts, target)
    }
}

/// Opens the preference store and the database without GTK, for `--run-backup`.
pub fn open_headless(paths: &dyn Paths) -> Result<Library, String> {
    let prefs = kuro_prefs::settings_load::open(paths).map_err(|e| e.to_string())?;
    let data = paths.data_dir();
    let path = schema_unified::db_path(&data);
    let db = Arc::new(Db::new(&path, schema_unified::open_file(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?);
    Ok(Library { prefs, db, extensions_dir: data.join("extensions") })
}

/// The battery state from `/sys/class/power_supply`: on battery when a battery reports `Discharging`, with the level of
/// the first battery. A machine without a battery is on AC power.
pub fn power_state() -> PowerState {
    power_from(Path::new("/sys/class/power_supply"))
}

fn power_from(root: &Path) -> PowerState {
    let mut batteries: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "Battery"))
        .collect();
    batteries.sort();
    let read = |dir: &Path, file: &str| std::fs::read_to_string(dir.join(file)).ok().map(|s| s.trim().to_owned());
    let on_battery = batteries.iter().any(|b| read(b, "status").as_deref() == Some("Discharging"));
    let percent = batteries.first().and_then(|b| read(b, "capacity")).and_then(|c| c.parse::<u8>().ok());
    PowerState { on_battery, percent }
}

/// [`AutoHost`] for one `--run-backup` invocation: the settings, the databases and the automatic folder.
pub struct CliHost {
    pub library: Library,
    pub auto_dir: PathBuf,
    pub now_ms: Box<dyn Fn() -> i64 + Send + Sync>,
    pub power: PowerState,
}

impl CliHost {
    fn stamp(&self, key: &str) -> i64 {
        self.library.prefs.pref(key.to_owned(), 0_i64).get()
    }

    /// The scheduler settings. The first run counts one interval from the first time the entry point sees the setting
    /// (E-08), so that moment is stored when there is none yet.
    pub fn config(&self) -> AutoConfig {
        let interval = self.library.prefs.pref(INTERVAL_KEY.to_owned(), 12_i32).get();
        let mut enabled_at = self.stamp(ENABLED_AT_KEY);
        if enabled_at == 0 {
            enabled_at = (self.now_ms)();
            if let Err(e) = self.library.prefs.pref(ENABLED_AT_KEY.to_owned(), 0_i64).set(enabled_at) {
                tracing::warn!("could not store when the automatic backup was first seen: {e}");
            }
        }
        AutoConfig {
            interval_hours: kuro_backup::settings::normalize_interval(i64::from(interval)),
            last_auto_epoch_ms: self.stamp(LAST_AUTO_KEY),
            enabled_at_epoch_ms: enabled_at,
            min_battery_percent: kuro_backup::auto::DEFAULT_MIN_BATTERY_PERCENT,
        }
    }
}

impl AutoHost for CliHost {
    fn now_ms(&self) -> i64 {
        (self.now_ms)()
    }

    fn last_auto_ms(&self) -> i64 {
        self.stamp(LAST_AUTO_KEY)
    }

    fn restore_running(&self) -> bool {
        // a restore runs inside the GUI process, which holds the settings lock, so this process never starts beside it
        false
    }

    fn power(&self) -> PowerState {
        self.power
    }

    fn run_backup(&self) -> Result<PathBuf, BackupError> {
        let target = Target::Automatic { dir: self.auto_dir.clone(), now: Local::now() };
        self.library.create(&BackupOptions::default(), &target).map(|(path, _skipped)| path)
    }

    fn save_last_auto_ms(&self, ms: i64) {
        if let Err(e) = self.library.prefs.pref(LAST_AUTO_KEY.to_owned(), 0_i64).set(ms) {
            tracing::warn!("could not store the last automatic backup time: {e}");
        }
    }

    fn report_failure(&self, streak: u32) {
        eprintln!("kurobayomi: the automatic backup failed ({streak} in a row)");
    }
}

/// Exit status of `--run-backup`: 0 when a backup was written or none was due, 1 when the run failed.
pub fn exit_status(outcome: &OnceOutcome) -> u8 {
    match outcome {
        OnceOutcome::Ran(_) | OnceOutcome::Skipped => 0,
        OnceOutcome::Failed(_) => 1,
    }
}

/// `kurobayomi --run-backup`: at most one automatic run, then the exit status (T-21a-041).
pub fn run_backup_cli() -> u8 {
    let paths = match kuro_platform::system_paths::SystemPaths::system() {
        Ok(paths) => paths,
        Err(e) => {
            eprintln!("kurobayomi: {e}");
            return 1;
        }
    };
    let library = match open_headless(&paths) {
        Ok(library) => library,
        Err(e) if e.to_lowercase().contains("lock") => {
            // the GUI owns the settings and runs its own schedule
            eprintln!("kurobayomi: the app is open, so no automatic backup runs from here");
            return 0;
        }
        Err(e) => {
            eprintln!("kurobayomi: cannot open the library: {e}");
            return 1;
        }
    };
    let auto_dir = match crate::settings_ui::storage::manager(&library.prefs).map(|storage| storage.backups_dir()) {
        Some(Ok(dir)) => dir,
        Some(Err(e)) => {
            eprintln!("kurobayomi: the backups folder is not available: {e}");
            return 1;
        }
        None => {
            eprintln!("kurobayomi: the storage folder could not be resolved");
            return 1;
        }
    };
    let host = CliHost {
        library,
        auto_dir: auto_dir.clone(),
        now_ms: Box::new(|| Local::now().timestamp_millis()),
        power: power_state(),
    };
    let outcome = run_once(&host.config(), &host, &auto_dir);
    if let OnceOutcome::Failed(e) = &outcome {
        eprintln!("kurobayomi: automatic backup failed: {e}");
    }
    exit_status(&outcome)
}

/// What a finished restore also imports: the old repo addresses of the backup, as repository links, after the user
/// chose to restore the extension stores (T-19a-022, T-18a-019).
pub struct LegacyImport<'a> {
    pub links: Pref<BTreeSet<String>>,
    pub repo_http: &'a dyn HttpGet,
}

/// Runs one restore on this thread over its own connection to the database file. Source settings land in the app
/// store as `source_<id>.<key>`. Returns how the run ended.
pub fn run_restore(
    library: &Library,
    data_dir: &Path,
    log_dir: &Path,
    backup: Backup,
    opts: RestoreOptions,
    stop: &StopFlag,
    emit: &mut dyn FnMut(ProgressEvent),
    legacy: Option<LegacyImport<'_>>,
) -> Result<RestoreEnd, String> {
    let legacy_urls = (opts.extension_stores).then(|| legacy_repo_urls(&backup));
    let (manga_entries, anime_entries) = (backup.manga.len(), backup.anime.len());
    let mut conn = schema_unified::open_file(&schema_unified::db_path(data_dir)).map_err(|e| e.to_string())?;
    let held: RefCell<HashMap<String, PrefStore>> = RefCell::default();
    let source_prefs = |name: &str| -> PrefStore {
        held.borrow_mut()
            .entry(name.to_owned())
            .or_insert_with(|| {
                // seeded with what the app already stores for this source, so a restore only overrides what it names
                let store = PrefStore::in_memory();
                let prefix = format!("{name}.");
                for (key, value) in library.prefs.all() {
                    if let Some(rest) = key.strip_prefix(&prefix) {
                        store.set_raw(rest, value);
                    }
                }
                store
            })
            .clone()
    };
    let scheduler = IntervalScheduler { store: &library.prefs, backup_timer: &|_| {} };
    let end = run_backup(
        backup,
        opts,
        Local::now(),
        Targets { db: &mut conn, app_prefs: &library.prefs, source_prefs: &source_prefs, scheduler: &scheduler },
        &|| stop.requested(),
        emit,
        log_dir,
    );
    if matches!(end, RestoreEnd::Completed(_)) {
        // a big restore leaves the planner without statistics for the new rows (T-03a-028)
        if let Err(e) = kuro_db::maintenance::after_restore(&conn, manga_entries + anime_entries) {
            tracing::warn!("ANALYZE after the restore failed: {e}");
        }
    }
    // the restore wrote through its own connection, which tells no one: mark every table changed on the app's handle
    // so the screens already open (the library of the other kind, history, updates) read the restored rows
    // (a cancelled restore may have written part of the rows too)
    {
        use kuro_db::TxExt;
        if let Err(e) = library.db.write(|tx| tx.touch(kuro_db::DB_TABLES)) {
            tracing::warn!("announcing the restored rows failed: {e}");
        }
    }
    for (name, store) in held.borrow().iter() {
        for (key, value) in store.all() {
            library.prefs.set_raw(&format!("{name}.{key}"), value);
        }
    }
    if let (Some(legacy), Some(urls), RestoreEnd::Completed(_)) = (legacy, legacy_urls, &end) {
        if !import_legacy_repos(&legacy.links, &urls, legacy.repo_http).is_empty() {
            // the imported links are fetched now, so their sources are listed without waiting for the daily check
            RepoOps::new(legacy.links).refresh();
        }
    }
    // an Aniyomi backup's disabled sources become the source switches on the next load (`source_switch::settle`)
    if matches!(end, RestoreEnd::Completed(_)) {
        crate::recipe_sources::reload();
    }
    Ok(end)
}

#[cfg(test)]
mod tests {
    use kuro_prefs::source::SourcePrefs;
    use std::sync::Mutex;

    use kuro_backup::schema::{BackupManga, BackupPreference, PrefValue};

    use super::*;

    fn library(dir: &Path) -> Library {
        let path = schema_unified::db_path(dir);
        let db = Arc::new(Db::new(&path, schema_unified::open_file(&path).unwrap()).unwrap());
        Library { prefs: PrefStore::in_memory(), db, extensions_dir: dir.join("extensions") }
    }

    fn host(dir: &Path, now: i64) -> CliHost {
        CliHost {
            library: library(dir),
            auto_dir: dir.join("autobackup"),
            now_ms: Box::new(move || now),
            power: PowerState { on_battery: false, percent: None },
        }
    }

    fn files(dir: &Path) -> usize {
        std::fs::read_dir(dir).map(|d| d.filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().ends_with(".tachibk")).count()).unwrap_or(0)
    }

    /// T-21a-041: past the due time the entry point writes exactly one file and exits 0; the next call is not due.
    #[test]
    fn run_backup_writes_one_file_when_due_and_none_straight_after() {
        let dir = tempfile::tempdir().unwrap();
        let host = host(dir.path(), 1_000_000_000_000);
        // seed one library entry so the backup is not empty
        let mut m = kuro_data::entry::Entry::create(kuro_platform::kind::Kind::Manga);
        m.source = 1;
        m.url = "/m".into();
        m.title = "T".into();
        m.favorite = true;
        kuro_data::entry::EntryRepo::new(host.library.db.clone()).insert(&m).unwrap();
        host.library.prefs.pref("__APP_STATE_backup_enabled_at".to_owned(), 0_i64).set(1).unwrap();
        let first = run_once(&host.config(), &host, &host.auto_dir);
        assert!(matches!(first, OnceOutcome::Ran(_)), "{first:?}");
        assert_eq!(exit_status(&first), 0);
        assert_eq!(files(&host.auto_dir), 1);
        assert_eq!(host.last_auto_ms(), 1_000_000_000_000, "the run is recorded");
        let second = run_once(&host.config(), &host, &host.auto_dir);
        assert!(matches!(second, OnceOutcome::Skipped));
        assert_eq!(exit_status(&second), 0);
        assert_eq!(files(&host.auto_dir), 1, "at most one run");
    }

    #[test]
    fn a_failed_run_exits_1_and_the_first_sight_of_the_setting_is_not_due() {
        assert_eq!(exit_status(&OnceOutcome::Failed(BackupError::CreateFile)), 1);
        let dir = tempfile::tempdir().unwrap();
        let host = host(dir.path(), 5_000_000);
        let config = host.config();
        assert_eq!(config.enabled_at_epoch_ms, 5_000_000, "stored on first sight");
        assert!(matches!(run_once(&config, &host, &host.auto_dir), OnceOutcome::Skipped), "one interval from now");
        assert_eq!(files(&host.auto_dir), 0);
    }

    #[test]
    fn the_battery_is_read_from_the_power_supply_folder() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(power_from(root.path()), PowerState { on_battery: false, percent: None }, "no battery means AC");
        let bat = root.path().join("BAT0");
        std::fs::create_dir_all(&bat).unwrap();
        std::fs::write(bat.join("type"), "Battery\n").unwrap();
        std::fs::write(bat.join("status"), "Discharging\n").unwrap();
        std::fs::write(bat.join("capacity"), "42\n").unwrap();
        let mains = root.path().join("AC");
        std::fs::create_dir_all(&mains).unwrap();
        std::fs::write(mains.join("type"), "Mains\n").unwrap();
        assert_eq!(power_from(root.path()), PowerState { on_battery: true, percent: Some(42) });
    }

    struct NoNet;
    impl HttpGet for NoNet {
        fn get_text(&self, _url: &str) -> Result<String, String> {
            Err("offline".into())
        }
    }

    /// T-22a-041 and T-19a-022: a restore over the files writes the entry, and with the stores option the old repo
    /// lists of the backup become rows that fail their refresh quietly.
    #[test]
    fn a_restore_writes_the_library_and_imports_the_old_repo_lists_once_the_stores_option_is_on() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(dir.path());
        let backup = Backup {
            manga: vec![BackupManga { source: 7, url: "/m".into(), title: "Restored".into(), version: 4, ..Default::default() }],
            preferences: vec![
                BackupPreference { key: "anime_extension_repos".into(), value: PrefValue::StringSet(["https://a.test/index.min.json".to_owned()].into()) },
                BackupPreference { key: "extension_repos".into(), value: PrefValue::StringSet(["https://m.test".to_owned()].into()) },
            ],
            ..Default::default()
        };
        assert_eq!(legacy_repo_urls(&backup).len(), 2);
        let events = Mutex::new(Vec::new());
        let links = SourcePrefs::new(lib.prefs.clone()).manga_extension_repos;
        let legacy = LegacyImport { links: links.clone(), repo_http: &NoNet };
        let opts = RestoreOptions { app_settings: false, ..RestoreOptions::default() };
        let end = run_restore(&lib, dir.path(), &dir.path().join("logs"), backup, opts, &StopFlag::default(), &mut |e| events.lock().unwrap().push(e.title), Some(legacy)).unwrap();
        assert!(matches!(end, RestoreEnd::Completed(_)), "{end:?}");
        assert!(kuro_data::entry::EntryRepo::new(lib.db.clone()).by_url_and_source(kuro_platform::kind::Kind::Manga, "/m", 7).unwrap().is_some());
        // only the address that is a repository link is listed; the other one is ignored without error
        assert_eq!(links.get().into_iter().collect::<Vec<_>>(), ["https://a.test/index.min.json"]);
    }

    #[test]
    fn without_the_stores_option_the_old_repo_lists_are_not_imported() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(dir.path());
        let backup = Backup {
            manga: vec![BackupManga { source: 7, url: "/m".into(), title: "R".into(), version: 4, ..Default::default() }],
            preferences: vec![BackupPreference { key: "anime_extension_repos".into(), value: PrefValue::StringSet(["https://a.test".to_owned()].into()) }],
            ..Default::default()
        };
        let links = SourcePrefs::new(lib.prefs.clone()).manga_extension_repos;
        let legacy = LegacyImport { links: links.clone(), repo_http: &NoNet };
        let opts = RestoreOptions { extension_stores: false, ..RestoreOptions::default() };
        run_restore(&lib, dir.path(), &dir.path().join("logs"), backup, opts, &StopFlag::default(), &mut |_| {}, Some(legacy)).unwrap();
        assert!(links.get().is_empty());
    }

    #[test]
    fn source_settings_of_a_backup_are_stored_under_the_source_namespace() {
        use kuro_backup::schema::BackupSourcePreferences;
        let dir = tempfile::tempdir().unwrap();
        let lib = library(dir.path());
        lib.prefs.set_raw("source_1234.keep", kuro_prefs::values::PrefValue::Int(1));
        let backup = Backup {
            manga: vec![BackupManga { source: 7, url: "/m".into(), title: "R".into(), version: 4, ..Default::default() }],
            source_preferences: vec![BackupSourcePreferences {
                source_key: "1234".into(),
                prefs: vec![BackupPreference { key: "show".into(), value: PrefValue::Boolean(true) }],
            }],
            ..Default::default()
        };
        run_restore(&lib, dir.path(), &dir.path().join("logs"), backup, RestoreOptions::default(), &StopFlag::default(), &mut |_| {}, None).unwrap();
        assert_eq!(lib.prefs.get_raw("source_1234.show"), Some(kuro_prefs::values::PrefValue::Bool(true)));
        assert_eq!(lib.prefs.get_raw("source_1234.keep"), Some(kuro_prefs::values::PrefValue::Int(1)));
    }
}
