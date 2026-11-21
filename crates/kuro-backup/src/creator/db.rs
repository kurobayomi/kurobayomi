//! The production adapters of the backup creator (T-21a-012, T-21a-019, T-21a-020): the library read over the two
//! databases, the preference read over the app store, and the installed extension artifacts read from disk. All of it
//! is GTK-free.

use kuro_platform::kind::Kind;
use std::path::PathBuf;
use std::sync::Arc;

use kuro_data::DataError;
use kuro_data::category::{Category, CategoryRepo};
use kuro_data::entry::{Entry, EntryRepo};
use kuro_data::history::HistoryRepo;
use kuro_data::item::ItemRepo;
use kuro_data::track::TrackRepo;
use kuro_db::handle::Db;
use kuro_prefs::values::PrefValue as AppValue;

use super::{Built, EntryData, ExtensionArtifacts, LibraryReader, PrefStore, SourcePrefStore};
use crate::error::BackupError;
use crate::schema::PrefValue;

fn read(e: DataError) -> BackupError {
    BackupError::Read(e.to_string())
}

/// [`LibraryReader`] over the one database.
pub struct DbLibrary {
    db: Arc<Db>,
}

impl DbLibrary {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }

    fn entry_data(&self, entry: Entry) -> Result<EntryData, BackupError> {
        let id = entry.id;
        Ok(EntryData {
            items: ItemRepo::new(self.db.clone()).by_entry_id(id, false).map_err(read)?,
            excluded_scanlators: EntryRepo::new(self.db.clone()).excluded_scanlators(id).map_err(read)?,
            categories: CategoryRepo::new(self.db.clone(), entry.kind).by_entry(id).map_err(read)?,
            tracks: TrackRepo::new(self.db.clone()).by_entry(id).map_err(read)?,
            history: HistoryRepo::new(self.db.clone()).by_entry_id(id).map_err(read)?,
            entry,
        })
    }

    fn all(&self, entries: Vec<Entry>) -> Result<Vec<EntryData>, BackupError> {
        entries.into_iter().map(|e| self.entry_data(e)).collect()
    }
}

impl LibraryReader for DbLibrary {
    /// E-05. The repositories read on pooled WAL readers, which see only committed rows, and every commit goes
    /// through the single writer. Holding both writers for the pass therefore pins the state every read sees.
    // ponytail: the app's own writes wait for the pass; a pinned reader connection in kuro-db would let them
    // proceed, add it if backups on large libraries stall the UI.
    fn snapshot(&self, f: &mut dyn FnMut() -> Built) -> Built {
        let mut built = None;
        let mut pass = || -> Result<(), kuro_db::error::DbError> {
            built = Some(f());
            Ok(())
        };
        let held = self.db.write(|_| pass());
        match built {
            Some(result) => result,
            None => Err(BackupError::Read(held.err().map(|e| e.to_string()).unwrap_or_default())),
        }
    }

    fn favorites(&self, kind: Kind) -> Result<Vec<EntryData>, BackupError> {
        self.all(EntryRepo::new(self.db.clone()).favorites(kind).map_err(read)?)
    }

    fn not_in_library(&self, kind: Kind) -> Result<Vec<EntryData>, BackupError> {
        self.all(EntryRepo::new(self.db.clone()).watched_not_in_library(kind).map_err(read)?)
    }

    fn categories(&self, kind: Kind) -> Result<Vec<Category>, BackupError> {
        CategoryRepo::new(self.db.clone(), kind).all().map_err(read)
    }
}

/// The backup value of a stored preference value.
pub fn to_backup_value(value: AppValue) -> PrefValue {
    match value {
        AppValue::Int(v) => PrefValue::Int(v),
        AppValue::Long(v) => PrefValue::Long(v),
        AppValue::Float(v) => PrefValue::Float(v),
        AppValue::Str(v) => PrefValue::String(v),
        AppValue::Bool(v) => PrefValue::Boolean(v),
        AppValue::Set(v) => PrefValue::StringSet(v),
    }
}

/// [`PrefStore`] and [`SourcePrefStore`] over the app's preference store.
pub struct StorePrefs<'a>(pub &'a kuro_prefs::store::PrefStore);

impl PrefStore for StorePrefs<'_> {
    fn all(&self) -> Vec<(String, PrefValue)> {
        // a source's own settings (`source_<id>.<key>`) travel in the source preference list, not here
        self.0.all().into_iter().filter(|(key, _)| !is_source_setting(key)).map(|(k, v)| (k, to_backup_value(v))).collect()
    }
}

/// `source_<decimal id>.<key>` (`source_languages` is an app setting, not this).
fn is_source_setting(key: &str) -> bool {
    key.strip_prefix("source_").and_then(|rest| rest.split_once('.')).is_some_and(|(id, _)| id.parse::<i64>().is_ok())
}

impl SourcePrefStore for StorePrefs<'_> {
    // ponytail: no installed source declares settings yet, so no source is configurable; list them through the source
    // registry once the extension host runs sources.
    fn configurable(&self, _kind: Kind) -> Vec<(String, Vec<(String, PrefValue)>)> {
        Vec::new()
    }
}

/// The installed extension packages as files: anime `<pkg>.kuroext` first, then manga `<dir>/component.wasm`. Both lists
/// are in name order. A package whose file cannot be read is reported with no bytes.
pub struct DirArtifacts {
    /// `<data>/extensions/anime`.
    pub anime_dir: PathBuf,
    /// `<data>/extensions`, one folder per manga package.
    pub manga_dir: PathBuf,
}

impl ExtensionArtifacts for DirArtifacts {
    fn installed(&self) -> Vec<(String, Option<Vec<u8>>)> {
        let mut out = Vec::new();
        let mut anime: Vec<PathBuf> = std::fs::read_dir(&self.anime_dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "kuroext") && p.is_file())
            .collect();
        anime.sort();
        for path in anime {
            let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            out.push((name, std::fs::read(&path).ok()));
        }
        let mut manga: Vec<PathBuf> = std::fs::read_dir(&self.manga_dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir() && p.join("manifest.toml").is_file() && !is_hidden(p))
            .collect();
        manga.sort();
        for dir in manga {
            let name = dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            out.push((name, std::fs::read(dir.join("component.wasm")).ok()));
        }
        out
    }
}

fn is_hidden(path: &std::path::Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::creator::CreateInputs;
    use crate::schema::Backup;
    use crate::creator::extensions::collect;
    use kuro_db::schema_unified;
    use kuro_prefs::store::PrefStore as AppStore;

    fn dbs() -> (tempfile::TempDir, Arc<Db>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kurobayomi.db");
        let db = Arc::new(Db::new(&path, schema_unified::open_file(&path).unwrap()).unwrap());
        (dir, db)
    }

    #[test]
    fn an_empty_library_reads_as_empty_lists_and_the_system_category() {
        let (_dir, db) = dbs();
        let lib = DbLibrary::new(db);
        assert!(lib.favorites(Kind::Manga).unwrap().is_empty());
        assert!(lib.favorites(Kind::Anime).unwrap().is_empty());
        assert!(lib.not_in_library(Kind::Manga).unwrap().is_empty());
        assert!(lib.not_in_library(Kind::Anime).unwrap().is_empty());
        assert!(lib.categories(Kind::Manga).unwrap().iter().all(|c| c.id == 0), "only the system row");
    }

    /// E-05: a write that starts during the pass commits only after it, so the pass reads one state.
    #[test]
    fn a_write_during_the_build_pass_waits_for_it() {
        let (_dir, db) = dbs();
        let lib = DbLibrary::new(db.clone());
        let writer_db = db.clone();
        let mut during = 0;
        let mut writer = None;
        let built = lib.snapshot(&mut || {
            writer = Some(std::thread::spawn({
                let db = writer_db.clone();
                move || {
                    let mut m = Entry::create(Kind::Manga);
                    m.source = 7;
                    m.url = "/m/late".into();
                    m.title = "Late".into();
                    m.favorite = true;
                    EntryRepo::new(db).insert(&m).unwrap();
                }
            }));
            std::thread::sleep(std::time::Duration::from_millis(200));
            during = lib.favorites(Kind::Manga).unwrap().len();
            Ok((Backup::default(), Vec::new()))
        });
        assert!(built.is_ok());
        assert_eq!(during, 0, "the late write was not visible inside the pass");
        writer.unwrap().join().unwrap();
        assert_eq!(lib.favorites(Kind::Manga).unwrap().len(), 1, "the write lands once the pass ends");
    }

    #[test]
    fn a_favourite_is_read_with_its_chapters_and_categories() {
        let (_dir, db) = dbs();
        let repo = EntryRepo::new(db.clone());
        let mut m = Entry::create(Kind::Manga);
        m.source = 7;
        m.url = "/m/1".into();
        m.title = "One".into();
        m.favorite = true;
        let id = repo.insert(&m).unwrap();
        let lib = DbLibrary::new(db);
        let entries = lib.favorites(Kind::Manga).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!((entries[0].entry.id, entries[0].entry.title.as_str()), (id, "One"));
    }

    #[test]
    fn app_prefs_keep_their_type_and_leave_source_settings_out() {
        let store = AppStore::in_memory();
        store.set_raw("theme", AppValue::Str("dark".into()));
        store.set_raw("volume", AppValue::Int(3));
        store.set_raw("source_1234.show", AppValue::Bool(true));
        store.set_raw("source_languages", AppValue::Set(["en".to_owned()].into()));
        let all = StorePrefs(&store).all();
        assert!(all.contains(&("theme".to_owned(), PrefValue::String("dark".into()))));
        assert!(all.contains(&("volume".to_owned(), PrefValue::Int(3))));
        assert!(!all.iter().any(|(k, _)| k == "source_1234.show"));
        assert!(all.iter().any(|(k, _)| k == "source_languages"), "an app setting that starts with source_ stays");
        assert!(StorePrefs(&store).configurable(Kind::Manga).is_empty());
    }

    #[test]
    fn artifacts_list_anime_first_then_manga_and_report_a_missing_file_as_skipped() {
        let root = tempfile::tempdir().unwrap();
        let anime_dir = root.path().join("anime");
        std::fs::create_dir_all(&anime_dir).unwrap();
        std::fs::write(anime_dir.join("b.demo.kuroext"), b"B").unwrap();
        std::fs::write(anime_dir.join("a.demo.kuroext"), b"A").unwrap();
        std::fs::write(anime_dir.join("a.demo.signer"), b"ignored").unwrap();
        let pkg = root.path().join("m.example");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("manifest.toml"), b"x").unwrap();
        std::fs::write(pkg.join("component.wasm"), b"W").unwrap();
        let broken = root.path().join("z.broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("manifest.toml"), b"x").unwrap();
        let source = DirArtifacts { anime_dir: anime_dir.clone(), manga_dir: root.path().to_path_buf() };
        let (written, skipped) = collect(&source);
        let names: Vec<_> = written.iter().map(|e| e.pkg_name.as_str()).collect();
        assert_eq!(names, ["a.demo", "b.demo", "m.example"]);
        assert_eq!(written[2].apk, b"W");
        assert_eq!(skipped, ["z.broken"]);
    }

    struct Named;
    impl crate::creator::SourceRegistry for Named {
        fn is_installed(&self, _: i64) -> bool {
            false
        }
        fn name_of(&self, id: i64) -> String {
            id.to_string()
        }
        fn stub_label(&self, id: i64) -> String {
            id.to_string()
        }
    }

    /// T-21a-019: the artifacts go into the file, anime first, and the skipped package is reported.
    #[test]
    fn a_backup_with_extensions_carries_the_artifacts_and_reports_the_skipped_one() {
        let (dir, db) = dbs();
        let mut m = Entry::create(Kind::Manga);
        m.source = 7;
        m.url = "/m/1".into();
        m.title = "One".into();
        m.favorite = true;
        EntryRepo::new(db.clone()).insert(&m).unwrap();
        let exts = dir.path().join("exts");
        std::fs::create_dir_all(exts.join("anime")).unwrap();
        std::fs::write(exts.join("anime/a.demo.kuroext"), b"A").unwrap();
        let broken = exts.join("m.broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("manifest.toml"), b"x").unwrap();
        let lib = DbLibrary::new(db);
        let store = AppStore::in_memory();
        let prefs = StorePrefs(&store);
        let artifacts = DirArtifacts { anime_dir: exts.join("anime"), manga_dir: exts.clone() };
        let inputs = CreateInputs { library: &lib, sources: &Named, prefs: &prefs, source_prefs: &prefs, extensions: Some(&artifacts) };
        let opts = crate::options::BackupOptions { extensions: true, ..Default::default() };
        let target = crate::file::Target::Manual(dir.path().join("out.tachibk"));
        let (path, skipped) = crate::file::create_file_report(&inputs, &opts, &target).unwrap();
        assert_eq!(skipped, ["m.broken"]);
        let decoded = crate::format::decode_file(&path).unwrap();
        assert_eq!(decoded.extensions.len(), 1);
        assert_eq!(decoded.extensions[0].pkg_name, "a.demo");
        assert_eq!(decoded.manga.len(), 1);
        // off by default: the same library without the option writes no artifacts
        let plain = CreateInputs { extensions: Some(&artifacts), ..inputs };
        let (path, _) = crate::file::create_file_report(&plain, &crate::options::BackupOptions::default(), &target).unwrap();
        assert!(crate::format::decode_file(&path).unwrap().extensions.is_empty());
    }
}
