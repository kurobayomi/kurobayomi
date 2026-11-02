//! Q-21 and X-112 (area 22, T-22a-042, T-22a-047): a backup the Create side writes passes the restore preflight and
//! restores, for the full default set and for each single option that `can_create` allows.

use chrono::{Local, TimeZone};
use kuro_backup::creator::{CreateInputs, EntryData, LibraryReader, PrefStore, SourcePrefStore, SourceRegistry};
use kuro_backup::error::BackupError;
use kuro_backup::file::{Target, create_file};
use kuro_backup::format::decode_file;
use kuro_backup::options::BackupOptions;
use kuro_backup::restore::driver::{Targets, restore_backup};
use kuro_backup::restore::options::RestoreOptions;
use kuro_backup::restore::prefs::IntervalScheduler;
use kuro_backup::schema::PrefValue;
use kuro_backup::validate::{TrackerRegistry, preflight_file};
use kuro_data::category::Category;
use kuro_data::entry::Entry;
use kuro_data::item::Item;
use kuro_db::schema_unified;
use kuro_platform::kind::Kind;
use kuro_prefs::store::PrefStore as Store;
use kuro_prefs::values::PrefValue as Stored;
use rusqlite::Connection;

fn category() -> Category {
    Category { id: 4, name: "Reading".into(), order: 1, flags: 0, hidden: false }
}

fn manga_entry() -> EntryData {
    let mut m = Entry::create(Kind::Manga);
    m.id = 1;
    m.source = 7;
    m.url = "/m/1".into();
    m.title = "Manga".into();
    m.favorite = true;
    let mut c = Item::create(Kind::Manga);
    c.id = 10;
    c.entry_id = 1;
    c.url = "/m/1/c".into();
    c.name = "Chapter".into();
    EntryData { entry: m, items: vec![c], excluded_scanlators: Default::default(), categories: vec![category()], tracks: Vec::new(), history: Vec::new() }
}

fn anime_entry() -> EntryData {
    let mut a = Entry::create(Kind::Anime);
    a.id = 2;
    a.source = 8;
    a.url = "/a/1".into();
    a.title = "Anime".into();
    a.favorite = true;
    EntryData { entry: a, items: Vec::new(), excluded_scanlators: Default::default(), categories: Vec::new(), tracks: Vec::new(), history: Vec::new() }
}

struct Library;

impl LibraryReader for Library {
    fn favorites(&self, kind: Kind) -> Result<Vec<EntryData>, BackupError> {
        Ok(vec![if kind.is_anime() { anime_entry() } else { manga_entry() }])
    }
    fn not_in_library(&self, _: Kind) -> Result<Vec<EntryData>, BackupError> {
        Ok(Vec::new())
    }
    fn categories(&self, kind: Kind) -> Result<Vec<Category>, BackupError> {
        Ok(vec![if kind.is_anime() { Category { id: 5, name: "Watching".into(), order: 1, flags: 0, hidden: false } } else { category() }])
    }
}

struct Sources;
impl SourceRegistry for Sources {
    fn is_installed(&self, _: i64) -> bool {
        true
    }
    fn name_of(&self, id: i64) -> String {
        format!("source {id}")
    }
    fn stub_label(&self, id: i64) -> String {
        id.to_string()
    }
}

struct Trackers;
impl TrackerRegistry for Trackers {
    fn name(&self, _: i32) -> Option<String> {
        None
    }
    fn logged_in(&self, _: i32) -> bool {
        true
    }
}

struct Prefs;
impl PrefStore for Prefs {
    fn all(&self) -> Vec<(String, PrefValue)> {
        vec![("show_nsfw_source".into(), PrefValue::Boolean(true))]
    }
}
impl SourcePrefStore for Prefs {
    fn configurable(&self, kind: Kind) -> Vec<(String, Vec<(String, PrefValue)>)> {
        if kind.is_anime() { Vec::new() } else { vec![("1234".into(), vec![("show_nsfw".into(), PrefValue::Boolean(true))])] }
    }
}

/// Every option off; a test turns on the one it checks.
fn none() -> BackupOptions {
    BackupOptions {
        library_entries: false,
        categories: false,
        chapters: false,
        tracking: false,
        history: false,
        read_entries: false,
        app_settings: false,
        extension_stores: false,
        custom_button: false,
        source_settings: false,
        private_settings: false,
        extensions: false,
    }
}

fn create(dir: &std::path::Path, opts: &BackupOptions) -> std::path::PathBuf {
    let inputs = CreateInputs { library: &Library, sources: &Sources, prefs: &Prefs, source_prefs: &Prefs, extensions: None };
    create_file(&inputs, opts, &Target::Manual(dir.join("out.tachibk"))).unwrap()
}

struct Dbs {
    db: Connection,
    app: Store,
    source: Store,
}

fn fresh() -> Dbs {
    Dbs { db: schema_unified::open_in_memory().unwrap(), app: Store::in_memory(), source: Store::in_memory() }
}

fn restore(dbs: &mut Dbs, path: &std::path::Path) {
    let backup = decode_file(path).unwrap();
    let nothing = |_: Option<u32>| {};
    let sched = IntervalScheduler { store: &dbs.app, backup_timer: &nothing };
    let source = dbs.source.clone();
    let open = move |_: &str| source.clone();
    let targets = Targets { db: &mut dbs.db, app_prefs: &dbs.app, source_prefs: &open, scheduler: &sched };
    let now = Local.with_ymd_and_hms(2026, 10, 9, 12, 0, 0).single().unwrap();
    let run = restore_backup(backup, RestoreOptions::from_bits([true; 7]), now, targets, &|| false, &mut |_| {});
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    assert_eq!(run.stop, None);
}

fn rows(c: &Connection, table: &str) -> i64 {
    c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).unwrap()
}

fn entries(c: &Connection, kind: &str) -> i64 {
    c.query_row("SELECT count(*) FROM entries WHERE kind = ?1", [kind], |r| r.get(0)).unwrap()
}

#[test]
fn x112_default_backup_restores_into_an_empty_database_with_the_same_counts() {
    let dir = tempfile::tempdir().unwrap();
    let path = create(dir.path(), &BackupOptions::default());
    preflight_file(&path, &Sources, &Trackers).unwrap();
    let mut dbs = fresh();
    let seeded = rows(&dbs.db, "categories");
    restore(&mut dbs, &path);
    assert_eq!(entries(&dbs.db, "manga"), 1);
    assert_eq!(rows(&dbs.db, "items"), 1);
    assert_eq!(entries(&dbs.db, "anime"), 1);
    assert_eq!(rows(&dbs.db, "categories") - seeded, 2, "one manga and one anime category");
    assert_eq!(dbs.app.get_raw("show_nsfw_source"), Some(Stored::Bool(true)));
    assert_eq!(dbs.source.get_raw("show_nsfw"), Some(Stored::Bool(true)));
}

/// Each option that `can_create` accepts on its own writes a file that the preflight accepts and the restore applies.
#[test]
fn every_single_option_the_create_side_allows_restores() {
    let dir = tempfile::tempdir().unwrap();
    let singles: [(&str, BackupOptions, fn(&Dbs) -> bool); 4] = [
        ("library entries", BackupOptions { library_entries: true, ..none() }, |d| entries(&d.db, "manga") == 1 && entries(&d.db, "anime") == 1),
        ("categories", BackupOptions { categories: true, ..none() }, |d| rows(&d.db, "categories") > 2),
        ("app settings", BackupOptions { app_settings: true, ..none() }, |d| d.app.get_raw("show_nsfw_source").is_some()),
        ("source settings", BackupOptions { source_settings: true, ..none() }, |d| d.source.get_raw("show_nsfw").is_some()),
    ];
    // Custom buttons are cut: that option alone would write an empty file, so it does not enable Create.
    assert!(!BackupOptions { custom_button: true, ..none() }.can_create());
    for (name, opts, applied) in singles {
        assert!(opts.can_create(), "{name}");
        let path = create(dir.path(), &opts);
        preflight_file(&path, &Sources, &Trackers).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut dbs = fresh();
        restore(&mut dbs, &path);
        // Extension stores alone are not listed: the creator writes no repo rows any more (the links travel in the prefs).
        assert!(applied(&dbs), "{name}: the restore wrote what the file carries");
    }
}
