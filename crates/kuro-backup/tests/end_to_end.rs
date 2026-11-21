//! Integration tests (area 21, T-21a-040 section 4.11): a fake library, real files in a temp folder, and the decoder.

use kuro_backup::creator::{CreateInputs, EntryData, LibraryReader, PrefStore, SourcePrefStore, SourceRegistry};
use kuro_backup::error::BackupError;
use kuro_backup::file::{Target, create_file};
use kuro_backup::format::decode_file;
use kuro_backup::options::BackupOptions;
use kuro_backup::schema::PrefValue;
use kuro_data::category::Category;
use kuro_data::entry::Entry;
use kuro_data::item::Item;
use kuro_platform::kind::Kind;

struct Library {
    manga: Vec<EntryData>,
}

impl LibraryReader for Library {
    fn favorites(&self, kind: Kind) -> Result<Vec<EntryData>, BackupError> {
        Ok(if kind.is_anime() { Vec::new() } else { self.manga.clone() })
    }
    fn not_in_library(&self, _: Kind) -> Result<Vec<EntryData>, BackupError> {
        Ok(Vec::new())
    }
    fn categories(&self, kind: Kind) -> Result<Vec<Category>, BackupError> {
        Ok(if kind.is_anime() { Vec::new() } else { vec![Category { id: 4, name: "Reading".into(), order: 1, flags: 0, hidden: false }] })
    }
}

struct NoSources;
impl SourceRegistry for NoSources {
    fn is_installed(&self, _: i64) -> bool {
        false
    }
    fn name_of(&self, id: i64) -> String {
        format!("source {id}")
    }
    fn stub_label(&self, id: i64) -> String {
        id.to_string()
    }
}

struct NoPrefs;
impl PrefStore for NoPrefs {
    fn all(&self) -> Vec<(String, PrefValue)> {
        Vec::new()
    }
}
impl SourcePrefStore for NoPrefs {
    fn configurable(&self, _: Kind) -> Vec<(String, Vec<(String, PrefValue)>)> {
        Vec::new()
    }
}

fn one_manga(chapters: usize) -> EntryData {
    let mut m = Entry::create(Kind::Manga);
    m.id = 1;
    m.source = 7;
    m.url = "/m/1".into();
    m.title = "Title".into();
    m.favorite = true;
    let chapters = (0..chapters)
        .map(|i| {
            let mut c = Item::create(Kind::Manga);
            c.id = 10 + i as i64;
            c.entry_id = 1;
            c.url = format!("/c/{i}");
            c.name = format!("Chapter {i}");
            c
        })
        .collect();
    EntryData {
        entry: m,
        items: chapters,
        excluded_scanlators: Default::default(),
        categories: vec![Category { id: 4, name: "Reading".into(), order: 1, flags: 0, hidden: false }],
        tracks: Vec::new(),
        history: Vec::new(),
    }
}

fn inputs<'a>(lib: &'a Library, sources: &'a NoSources, prefs: &'a NoPrefs) -> CreateInputs<'a> {
    CreateInputs { library: lib, sources, prefs, source_prefs: prefs, extensions: None }
}

#[test]
fn manual_backup_round_trips_through_the_decoder() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library { manga: vec![one_manga(2)] };
    let target = dir.path().join("manual.tachibk");
    let written = create_file(&inputs(&lib, &NoSources, &NoPrefs), &BackupOptions::default(), &Target::Manual(target.clone())).unwrap();
    assert_eq!(written, target);
    let decoded = decode_file(&target).unwrap();
    assert_eq!(decoded.manga.len(), 1);
    assert_eq!(decoded.manga[0].chapters.len(), 2);
    assert_eq!(decoded.manga[0].categories, vec![1]);
    assert_eq!(decoded.categories.len(), 1);
    assert!(!decoded.is_legacy);
    let names: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec!["manual.tachibk".to_string()], "no .part file is left behind");
}

struct Prefs;
impl PrefStore for Prefs {
    fn all(&self) -> Vec<(String, PrefValue)> {
        vec![
            ("pref_theme_mode_key".into(), PrefValue::String("SYSTEM".into())),
            ("__PRIVATE_track_token_2".into(), PrefValue::String("secret".into())),
            ("__APP_STATE_last_auto_backup_timestamp".into(), PrefValue::Long(5)),
        ]
    }
}
impl SourcePrefStore for Prefs {
    fn configurable(&self, kind: Kind) -> Vec<(String, Vec<(String, PrefValue)>)> {
        if kind.is_anime() {
            Vec::new()
        } else {
            vec![("source_7".into(), vec![("quality".into(), PrefValue::Int(2))])]
        }
    }
}

/// T-21a-044: default options write a backup that carries the app and source preferences; `__PRIVATE_` keys appear
/// only when the sensitive-settings option is on; `__APP_STATE_` keys never appear.
#[test]
fn default_options_write_preferences_and_private_keys_follow_the_option() {
    let dir = tempfile::tempdir().unwrap();
    let lib = Library { manga: vec![one_manga(1)] };
    let inputs = CreateInputs { library: &lib, sources: &NoSources, prefs: &Prefs, source_prefs: &Prefs, extensions: None };

    let plain = dir.path().join("plain.tachibk");
    create_file(&inputs, &BackupOptions::default(), &Target::Manual(plain.clone())).unwrap();
    let decoded = decode_file(&plain).unwrap();
    let keys: Vec<&str> = decoded.preferences.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, vec!["pref_theme_mode_key"]);
    assert_eq!(decoded.preferences[0].value, PrefValue::String("SYSTEM".into()));
    assert_eq!(decoded.source_preferences.len(), 1);
    assert_eq!(decoded.source_preferences[0].prefs[0].value, PrefValue::Int(2));

    let sensitive = dir.path().join("sensitive.tachibk");
    let opts = BackupOptions { private_settings: true, ..BackupOptions::default() };
    create_file(&inputs, &opts, &Target::Manual(sensitive.clone())).unwrap();
    let decoded = decode_file(&sensitive).unwrap();
    let keys: Vec<&str> = decoded.preferences.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, vec!["pref_theme_mode_key", "__PRIVATE_track_token_2"]);
}
