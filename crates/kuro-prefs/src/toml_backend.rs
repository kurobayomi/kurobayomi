//! TOML backend for `config.toml` (normal keys) and `state.toml` (app-state keys, plus the tracker
//! auth-expired flags). Edits go through `toml_edit`, so unknown tables, unknown keys and comments
//! survive a rewrite. Each file is replaced atomically.
//!
//! Layout: a registry row is `[table] field = value`. A family row is `[table] <id> = value`.
//! A key the registry does not know goes to `[extra]` as `key = { t = "long", v = 5 }`, so its
//! kind survives the round trip. Private strings are never written.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

use crate::keys::Namespace;
use crate::registry::{self, Lit, PrefMeta, REGISTRY};
use crate::store::Backend;
use crate::values::PrefValue;
use crate::{PrefError, lock};

const CONFIG_FILE: &str = "config.toml";
const STATE_FILE: &str = "state.toml";
const LOCK_FILE: &str = ".lock";
const EXTRA_TABLE: &str = "extra";
/// Version of the layout written to both files as a top-level `schema = 1`.
pub(crate) const SCHEMA_VERSION: i64 = 1;

/// `config.toml` inside the config directory.
pub fn config_file(config_dir: &Path) -> PathBuf {
    config_dir.join(CONFIG_FILE)
}

/// `state.toml` inside the state directory.
pub(crate) fn state_file(state_dir: &Path) -> PathBuf {
    state_dir.join(STATE_FILE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Config,
    State,
}

/// The file that stores `key`, or `None` when it is never written. A private flag (a bool, such
/// as the tracker auth-expired state) goes to the state file; private strings never do.
fn target(key: &str, is_bool: bool) -> Option<Target> {
    match Namespace::of(key) {
        Namespace::Normal => Some(Target::Config),
        Namespace::AppState => Some(Target::State),
        Namespace::Private => is_bool.then_some(Target::State),
    }
}

#[derive(Default)]
struct Docs {
    config: DocumentMut,
    state: DocumentMut,
}

impl Docs {
    fn get(&self, t: Target) -> &DocumentMut {
        match t {
            Target::Config => &self.config,
            Target::State => &self.state,
        }
    }

    fn get_mut(&mut self, t: Target) -> &mut DocumentMut {
        match t {
            Target::Config => &mut self.config,
            Target::State => &mut self.state,
        }
    }
}

/// Both preference files. Holds the cross-process lock until dropped.
pub struct TomlBackend {
    config: PathBuf,
    state: PathBuf,
    _lock: File,
    docs: Mutex<Docs>,
}

impl TomlBackend {
    /// Takes the lock in `config_dir` and records both file paths. Nothing is read until `load`.
    /// Returns [`PrefError::Locked`] at once when another process holds the lock.
    pub fn open(config_dir: &Path, state_dir: &Path) -> Result<Self, PrefError> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(config_dir.join(LOCK_FILE))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(PrefError::Locked),
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
        Ok(Self {
            config: config_file(config_dir),
            state: state_file(state_dir),
            _lock: lock,
            docs: Mutex::new(Docs::default()),
        })
    }
}

impl Backend for TomlBackend {
    fn load(&self) -> Result<BTreeMap<String, PrefValue>, PrefError> {
        let docs = Docs {
            config: read_doc(&self.config)?,
            state: read_doc(&self.state)?,
        };
        check_schema(&docs.config, &self.config);
        check_schema(&docs.state, &self.state);
        let mut out = BTreeMap::new();
        for t in [Target::Config, Target::State] {
            load_known(docs.get(t), t, &mut out);
            load_extra(docs.get(t), &mut out);
        }
        *lock(&self.docs) = docs;
        Ok(out)
    }

    fn save(&self, all: &BTreeMap<String, PrefValue>) -> Result<(), PrefError> {
        let mut docs = lock(&self.docs);
        for t in [Target::Config, Target::State] {
            stamp_schema(docs.get_mut(t));
            let root = docs.get_mut(t).as_table_mut();
            write_known(root, t, all);
            write_families(root, t, all);
            write_extra(root, t, all);
        }
        // The docs lock stays held through the writes: both files share one fixed temp name, so
        // two overlapping saves could rename each other's half-written temp file into place.
        write_atomic(&self.config, &docs.config.to_string())?;
        write_atomic(&self.state, &docs.state.to_string())
    }
}

fn schema_of(doc: &DocumentMut) -> Option<i64> {
    doc.get("schema")?.as_integer()
}

/// A file from a newer app is still loaded for the fields this build knows; its unknown fields are left alone.
fn check_schema(doc: &DocumentMut, file: &Path) {
    if let Some(found) = schema_of(doc).filter(|found| *found > SCHEMA_VERSION) {
        tracing::warn!(
            file = %file.display(),
            found,
            supported = SCHEMA_VERSION,
            "preferences file has a newer schema; loading the known fields only"
        );
    }
}

/// Writes `schema = 1` at the top of the file. A newer schema already in the file is kept, never lowered.
fn stamp_schema(doc: &mut DocumentMut) {
    let version = schema_of(doc).unwrap_or(0).max(SCHEMA_VERSION);
    if schema_of(doc) != Some(version) {
        doc.as_table_mut().insert("schema", Item::Value(Value::from(version)));
    }
}

fn lit_is_bool(lit: Lit) -> bool {
    matches!(lit, Lit::Bool(_))
}

fn value_is_bool(v: &PrefValue) -> bool {
    matches!(v, PrefValue::Bool(_))
}

/// Registry rows (not families) that belong in `t`.
fn fixed_rows(t: Target) -> impl Iterator<Item = &'static PrefMeta> {
    REGISTRY
        .iter()
        .filter(move |m| !m.is_family() && target(m.key, lit_is_bool(m.default)) == Some(t))
}

/// Family rows that belong in `t`.
fn family_rows(t: Target) -> impl Iterator<Item = &'static PrefMeta> {
    REGISTRY
        .iter()
        .filter(move |m| m.is_family() && target(m.key, lit_is_bool(m.default)) == Some(t))
}

fn load_known(doc: &DocumentMut, t: Target, out: &mut BTreeMap<String, PrefValue>) {
    for meta in fixed_rows(t).chain(family_rows(t)) {
        let Some(table) = table_at(doc.as_table(), meta.table) else {
            continue;
        };
        if meta.is_family() {
            for (id, item) in table.iter() {
                insert_decoded(out, &meta.key.replace("{id}", id), item, meta.default);
            }
        } else if let Some(item) = table.get(meta.field) {
            insert_decoded(out, meta.key, item, meta.default);
        }
    }
}

fn insert_decoded(out: &mut BTreeMap<String, PrefValue>, key: &str, item: &Item, default: Lit) {
    match item
        .as_value()
        .and_then(|v| decode_as(v, tag_of_lit(default)))
    {
        Some(v) => {
            out.entry(key.to_owned()).or_insert(v);
        }
        // A field of the wrong TOML type resets only that field, to its default.
        None => tracing::warn!("Invalid value for {key}; using the default"),
    }
}

fn load_extra(doc: &DocumentMut, out: &mut BTreeMap<String, PrefValue>) {
    let Some(extra) = table_at(doc.as_table(), EXTRA_TABLE) else {
        return;
    };
    for (key, item) in extra.iter() {
        let decoded = item
            .as_value()
            .and_then(Value::as_inline_table)
            .and_then(|t| {
                let tag = t.get("t")?.as_str()?;
                decode_as(t.get("v")?, tag)
            });
        match decoded {
            Some(v) => {
                out.entry(key.to_owned()).or_insert(v);
            }
            None => tracing::warn!("Invalid value for {key} in [extra]; ignoring"),
        }
    }
}

/// Sets `field` to `value`. An existing key keeps its decor, so comments above it and an inline
/// comment after it stay.
fn set_field(table: &mut Table, field: &str, mut value: Value) {
    match table.get_mut(field) {
        Some(item) => {
            if let Some(old) = item.as_value() {
                *value.decor_mut() = old.decor().clone();
            }
            *item = Item::Value(value);
        }
        None => {
            table.insert(field, Item::Value(value));
        }
    }
}

fn write_known(root: &mut Table, t: Target, all: &BTreeMap<String, PrefValue>) {
    for meta in fixed_rows(t) {
        match all.get(meta.key) {
            Some(v) => {
                if let Some(table) = table_mut(root, meta.table) {
                    set_field(table, meta.field, to_value(v));
                }
            }
            None => {
                if let Some(table) = table_at_mut(root, meta.table) {
                    table.remove(meta.field);
                }
            }
        }
    }
}

fn write_families(root: &mut Table, t: Target, all: &BTreeMap<String, PrefValue>) {
    for meta in family_rows(t) {
        let wanted: Vec<(&str, &PrefValue)> = all
            .iter()
            .filter_map(|(key, v)| match registry::lookup(key) {
                Some((m, Some(id))) if m.key == meta.key => Some((id, v)),
                _ => None,
            })
            .collect();
        if let Some(table) = table_at_mut(root, meta.table) {
            let stale: Vec<String> = table
                .iter()
                .map(|(k, _)| k.to_owned())
                .filter(|k| !wanted.iter().any(|(id, _)| *id == k.as_str()))
                .collect();
            for k in stale {
                table.remove(&k);
            }
        }
        for (id, v) in wanted {
            if let Some(table) = table_mut(root, meta.table) {
                set_field(table, id, to_value(v));
            }
        }
    }
}

/// Updates `[extra]` in place, so its comments survive. Only keys that are no longer wanted go.
fn write_extra(root: &mut Table, t: Target, all: &BTreeMap<String, PrefValue>) {
    let wanted: Vec<(&String, &PrefValue)> = all
        .iter()
        .filter(|(key, v)| {
            registry::lookup(key).is_none() && target(key, value_is_bool(v)) == Some(t)
        })
        .collect();
    if wanted.is_empty() && table_at(root, EXTRA_TABLE).is_none() {
        return;
    }
    let Some(extra) = table_mut(root, EXTRA_TABLE) else {
        return;
    };
    let stale: Vec<String> = extra
        .iter()
        .map(|(k, _)| k.to_owned())
        .filter(|k| !wanted.iter().any(|(key, _)| *key == k))
        .collect();
    for k in stale {
        extra.remove(&k);
    }
    for (key, v) in wanted {
        set_field(extra, key, Value::InlineTable(tagged(v)));
    }
}

fn tag_of_lit(lit: Lit) -> &'static str {
    match lit {
        Lit::Bool(_) => "bool",
        Lit::Int(_) => "int",
        Lit::Long(_) => "long",
        Lit::Float(_) => "float",
        Lit::Str(_) => "string",
        Lit::Set(_) => "set",
    }
}

fn tag_of_value(v: &PrefValue) -> &'static str {
    match v {
        PrefValue::Bool(_) => "bool",
        PrefValue::Int(_) => "int",
        PrefValue::Long(_) => "long",
        PrefValue::Float(_) => "float",
        PrefValue::Str(_) => "string",
        PrefValue::Set(_) => "set",
    }
}

/// Decodes a TOML value as the kind named by `tag`. `None` when the TOML type does not match.
pub(crate) fn decode_as(v: &Value, tag: &str) -> Option<PrefValue> {
    match tag {
        "bool" => v.as_bool().map(PrefValue::Bool),
        "int" => v
            .as_integer()
            .and_then(|i| i32::try_from(i).ok())
            .map(PrefValue::Int),
        "long" => v.as_integer().map(PrefValue::Long),
        "float" => v.as_float().map(|f| PrefValue::Float(f as f32)),
        "string" => v.as_str().map(|s| PrefValue::Str(s.to_owned())),
        "set" => v
            .as_array()?
            .iter()
            .map(|x| x.as_str().map(str::to_owned))
            .collect::<Option<std::collections::BTreeSet<String>>>()
            .map(PrefValue::Set),
        _ => None,
    }
}

fn to_value(v: &PrefValue) -> Value {
    match v {
        PrefValue::Bool(b) => Value::from(*b),
        PrefValue::Int(i) => Value::from(i64::from(*i)),
        PrefValue::Long(l) => Value::from(*l),
        // The shortest text that reads back as this f32, so 0.85 is written as 0.85.
        PrefValue::Float(f) => Value::from(f.to_string().parse::<f64>().unwrap_or(f64::from(*f))),
        PrefValue::Str(s) => Value::from(s.as_str()),
        PrefValue::Set(items) => Value::Array(items.iter().map(String::as_str).collect::<Array>()),
    }
}

pub(crate) fn tagged(v: &PrefValue) -> InlineTable {
    let mut t = InlineTable::new();
    t.insert("t", Value::from(tag_of_value(v)));
    t.insert("v", to_value(v));
    t
}

/// `path` is dotted, e.g. `player.gestures`.
fn table_at<'a>(root: &'a Table, path: &str) -> Option<&'a Table> {
    path.split('.')
        .try_fold(root, |t, seg| t.get(seg)?.as_table())
}

fn table_at_mut<'a>(root: &'a mut Table, path: &str) -> Option<&'a mut Table> {
    path.split('.')
        .try_fold(root, |t, seg| t.get_mut(seg)?.as_table_mut())
}

/// Like [`table_at_mut`], creating each missing table on the way.
fn table_mut<'a>(root: &'a mut Table, path: &str) -> Option<&'a mut Table> {
    path.split('.').try_fold(root, |t, seg| {
        if !t.contains_key(seg) {
            t.insert(seg, Item::Table(Table::new()));
        }
        t.get_mut(seg)?.as_table_mut()
    })
}

/// Reads a settings file. A missing file is empty. An unreadable file is renamed aside and an
/// empty document is used.
fn read_doc(path: &Path) -> Result<DocumentMut, PrefError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(DocumentMut::new()),
        Err(e) => return Err(e.into()),
    };
    let parsed = String::from_utf8(bytes)
        .ok()
        .and_then(|text| text.parse::<DocumentMut>().ok());
    if let Some(doc) = parsed {
        return Ok(doc);
    }
    quarantine(path);
    Ok(DocumentMut::new())
}

/// Renames `path` to `<name>.corrupt-<unix seconds>`.
fn quarantine(path: &Path) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut to = path.as_os_str().to_owned();
    to.push(format!(".corrupt-{stamp}"));
    match fs::rename(path, &to) {
        Ok(()) => {
            tracing::warn!(path = %path.display(), "settings file is unreadable; defaults used, old file kept")
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "settings file is unreadable and could not be moved")
        }
    }
}

/// Writes `text` to `<path>.tmp` beside `path`, syncs it, then renames it over `path`.
pub(crate) fn write_atomic(path: &Path, text: &str) -> Result<(), PrefError> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let _ = fs::remove_file(&tmp);
    {
        let mut file = create_private(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn create_private(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::testutil::TestDir;

    fn backend(dir: &TestDir) -> Result<TomlBackend, PrefError> {
        let config = dir.path().join("config");
        let state = dir.path().join("state");
        fs::create_dir_all(&config)?;
        fs::create_dir_all(&state)?;
        TomlBackend::open(&config, &state)
    }

    fn file(dir: &TestDir, name: &str) -> String {
        let folder = if name == CONFIG_FILE {
            "config"
        } else {
            "state"
        };
        fs::read_to_string(dir.path().join(folder).join(name)).unwrap_or_default()
    }

    fn write_file(dir: &TestDir, name: &str, text: &str) {
        let folder = if name == CONFIG_FILE {
            "config"
        } else {
            "state"
        };
        fs::create_dir_all(dir.path().join(folder)).unwrap();
        fs::write(dir.path().join(folder).join(name), text).unwrap();
    }

    fn sample() -> BTreeMap<String, PrefValue> {
        let mut m = BTreeMap::new();
        m.insert(
            "pref_display_mode_library".into(),
            PrefValue::Str("LIST".into()),
        );
        m.insert("crop_borders".into(), PrefValue::Bool(true));
        m.insert(
            "library_unread_updates_count".into(),
            PrefValue::Int(12),
        );
        m.insert("pref_progress_preference".into(), PrefValue::Float(0.85));
        m.insert(
            "source_languages".into(),
            PrefValue::Set(BTreeSet::from(["all".to_owned(), "en".to_owned()])),
        );
        m.insert(
            "__APP_STATE_library_update_last_timestamp".into(),
            PrefValue::Long(1_700_000_000_000),
        );
        m.insert("__APP_STATE_incognito_mode".into(), PrefValue::Bool(true));
        m.insert(
            "pref_filter_library_tracked_7_v2".into(),
            PrefValue::Str("ENABLED_IS".into()),
        );
        m.insert(
            "__PRIVATE_pref_tracker_auth_expired_3".into(),
            PrefValue::Bool(true),
        );
        m.insert("legacy_long".into(), PrefValue::Long(5));
        m.insert("legacy_flag".into(), PrefValue::Bool(true));
        m
    }

    #[test]
    fn both_files_carry_schema_one_after_a_save() {
        let dir = TestDir::new("schema-stamp");
        let b = backend(&dir).unwrap();
        b.save(&sample()).unwrap();
        assert!(file(&dir, CONFIG_FILE).starts_with("schema = 1"), "{}", file(&dir, CONFIG_FILE));
        assert!(file(&dir, STATE_FILE).starts_with("schema = 1"), "{}", file(&dir, STATE_FILE));
    }

    #[test]
    fn a_newer_schema_loads_known_fields_and_is_not_lowered_on_save() {
        let dir = TestDir::new("schema-newer");
        write_file(&dir, CONFIG_FILE, "schema = 2\n[player]\nsome_future_field = 1\n");
        let b = backend(&dir).unwrap();
        let loaded = b.load().unwrap();
        assert!(loaded.is_empty(), "unknown fields are not loaded as values: {loaded:?}");
        b.save(&sample()).unwrap();
        assert!(file(&dir, CONFIG_FILE).starts_with("schema = 2"), "{}", file(&dir, CONFIG_FILE));
        assert!(file(&dir, CONFIG_FILE).contains("some_future_field = 1"));
    }

    #[test]
    fn a_config_written_before_a_feature_was_dropped_still_loads_without_its_keys() {
        let dir = TestDir::new("dropped-keys");
        write_file(
            &dir,
            CONFIG_FILE,
            "schema = 1\n[ui]\ntablet_ui_mode = \"ALWAYS\"\ntheme_mode = \"DARK\"\n[security]\nsecure_screen = \"NEVER\"\n\
             [library]\nswipe_left_chapter = \"Download\"\n[reader]\ndefault_orientation_type = 16\n\
             [player]\ndefault_player_orientation_type = \"Free\"\nremember_player_brightness = true\n\
             [player.gestures]\ngesture_volume_brightness = false\n",
        );
        let b = backend(&dir).unwrap();
        let loaded = b.load().unwrap();
        assert_eq!(loaded.len(), 1, "only the kept field is a value: {loaded:?}");
        assert_eq!(loaded.get("pref_theme_mode_key"), Some(&PrefValue::Str("DARK".into())));
        b.save(&loaded).unwrap();
        assert!(file(&dir, CONFIG_FILE).contains("theme_mode = \"DARK\""));
    }

    #[test]
    fn values_round_trip_through_a_reopen() {
        let dir = TestDir::new("roundtrip");
        let b = backend(&dir).unwrap();
        assert!(b.load().unwrap().is_empty());
        b.save(&sample()).unwrap();
        drop(b);
        let reopened = backend(&dir).unwrap();
        assert_eq!(reopened.load().unwrap(), sample());
    }

    #[test]
    fn values_land_in_the_right_file_and_table() {
        let dir = TestDir::new("layout");
        let b = backend(&dir).unwrap();
        b.load().unwrap();
        b.save(&sample()).unwrap();
        let config = file(&dir, CONFIG_FILE);
        let state = file(&dir, STATE_FILE);
        assert!(
            config.contains("[library]") && config.contains("display_mode = \"LIST\""),
            "{config}"
        );
        assert!(
            config.contains("[tracking.auth_expired]") || state.contains("[tracking.auth_expired]")
        );
        assert!(state.contains("incognito_mode = true"));
        assert!(
            state.contains("[library]") && state.contains("last_updated_timestamp = 1700000000000")
        );
        assert!(
            config.contains("[library.filter_tracked_manga]")
                && config.contains("7 = \"ENABLED_IS\"")
        );
        assert!(state.contains("[tracking.auth_expired]") && state.contains("3 = true"));
        assert!(
            config.contains("[extra]") && config.contains("legacy_long = { t = \"long\", v = 5 }")
        );
    }

    #[test]
    fn only_explicitly_set_keys_are_written() {
        let dir = TestDir::new("explicit");
        let b = backend(&dir).unwrap();
        b.load().unwrap();
        let mut only = BTreeMap::new();
        only.insert("crop_borders".to_owned(), PrefValue::Bool(true));
        b.save(&only).unwrap();
        let config = file(&dir, CONFIG_FILE);
        assert!(config.contains("crop_borders = true"));
        assert!(!config.contains("display_mode"));
        assert!(!config.contains("new_manga_updates_count"));
    }

    #[test]
    fn unknown_tables_keys_and_comments_survive_a_rewrite() {
        let dir = TestDir::new("unknown");
        write_file(
            &dir,
            CONFIG_FILE,
            "# my comment\nfoo = 1\n\n[mystery]\nx = \"y\"\n\n[reader]\n# keep me\ncrop_borders = true\n",
        );
        let b = backend(&dir).unwrap();
        let loaded = b.load().unwrap();
        assert_eq!(
            loaded.get("crop_borders"),
            Some(&PrefValue::Bool(true))
        );
        assert!(!loaded.contains_key("foo"));
        let mut next = loaded.clone();
        next.insert("crop_borders".into(), PrefValue::Bool(false));
        b.save(&next).unwrap();
        let config = file(&dir, CONFIG_FILE);
        assert!(config.contains("# my comment"));
        assert!(config.contains("foo = 1"));
        assert!(config.contains("[mystery]") && config.contains("x = \"y\""));
        assert!(config.contains("# keep me"));
        assert!(config.contains("crop_borders = false"));
    }

    #[test]
    fn deleting_a_key_removes_it_from_the_file() {
        let dir = TestDir::new("delete");
        let b = backend(&dir).unwrap();
        b.load().unwrap();
        b.save(&sample()).unwrap();
        let mut without = sample();
        without.remove("pref_display_mode_library");
        without.remove("legacy_long");
        b.save(&without).unwrap();
        let config = file(&dir, CONFIG_FILE);
        assert!(!config.contains("display_mode"));
        assert!(!config.contains("legacy_long"));
        assert!(config.contains("crop_borders"));
    }

    #[test]
    fn private_strings_never_reach_either_file() {
        let dir = TestDir::new("private");
        let b = backend(&dir).unwrap();
        b.load().unwrap();
        let mut m = BTreeMap::new();
        m.insert(
            "__PRIVATE_track_token_3".to_owned(),
            PrefValue::Str("hunter2".into()),
        );
        m.insert(
            "__PRIVATE_pref_mangasync_password_3".to_owned(),
            PrefValue::Str("pw-secret".into()),
        );
        b.save(&m).unwrap();
        assert!(!file(&dir, CONFIG_FILE).contains("hunter2"));
        assert!(!file(&dir, STATE_FILE).contains("pw-secret"));
        assert!(!file(&dir, STATE_FILE).contains("hunter2"));
        assert!(b.load().unwrap().is_empty());
    }

    #[test]
    fn save_leaves_no_temp_file_behind() {
        let dir = TestDir::new("atomic");
        let b = backend(&dir).unwrap();
        b.load().unwrap();
        b.save(&sample()).unwrap();
        for folder in ["config", "state"] {
            for entry in fs::read_dir(dir.path().join(folder)).unwrap() {
                let name = entry.unwrap().file_name().to_string_lossy().into_owned();
                assert!(!name.ends_with(".tmp"), "{name} left behind");
            }
        }
        assert!(dir.path().join("config").join(CONFIG_FILE).is_file());
    }

    #[test]
    fn an_inline_comment_survives_a_changed_value() {
        let dir = TestDir::new("inline-comment");
        write_file(
            &dir,
            CONFIG_FILE,
            "[reader]\ncrop_borders = true # inline note\n",
        );
        let b = backend(&dir).unwrap();
        let mut next = b.load().unwrap();
        next.insert("crop_borders".into(), PrefValue::Bool(false));
        b.save(&next).unwrap();
        assert!(file(&dir, CONFIG_FILE).contains("crop_borders = false # inline note"));
    }

    #[test]
    fn comments_inside_and_above_extra_survive_a_rewrite() {
        let dir = TestDir::new("extra-comments");
        write_file(
            &dir,
            CONFIG_FILE,
            "# above extra\n[extra]\n# keep this note\nlegacy_long = { t = \"long\", v = 5 }\n",
        );
        let b = backend(&dir).unwrap();
        let mut next = b.load().unwrap();
        next.insert("legacy_long".into(), PrefValue::Long(6));
        b.save(&next).unwrap();
        let config = file(&dir, CONFIG_FILE);
        assert!(config.contains("# above extra"), "{config}");
        assert!(config.contains("# keep this note"), "{config}");
        assert!(config.contains("v = 6"), "{config}");
        assert_eq!(
            b.load().unwrap().get("legacy_long"),
            Some(&PrefValue::Long(6))
        );
    }

    #[test]
    fn a_failed_write_leaves_the_previous_file_intact() {
        let dir = TestDir::new("fault");
        let b = backend(&dir).unwrap();
        b.load().unwrap();
        b.save(&sample()).unwrap();
        let before = file(&dir, CONFIG_FILE);
        // A directory in the way of the temp file makes the write fail before the rename.
        fs::create_dir(dir.path().join("config").join("config.toml.tmp")).unwrap();
        let mut next = sample();
        next.insert("crop_borders".into(), PrefValue::Bool(false));
        assert!(b.save(&next).is_err());
        assert_eq!(file(&dir, CONFIG_FILE), before);
    }

    #[cfg(unix)]
    #[test]
    fn files_are_written_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new("mode");
        let b = backend(&dir).unwrap();
        b.load().unwrap();
        b.save(&sample()).unwrap();
        let mode = fs::metadata(dir.path().join("config").join(CONFIG_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_field_with_the_wrong_toml_type_resets_only_that_field() {
        let dir = TestDir::new("wrongtype");
        write_file(
            &dir,
            CONFIG_FILE,
            "[library]\nnew_manga_updates_count = \"twelve\"\n\n[reader]\ncrop_borders = false\n",
        );
        let loaded = backend(&dir).unwrap().load().unwrap();
        assert!(!loaded.contains_key("library_unread_updates_count"));
        assert_eq!(
            loaded.get("crop_borders"),
            Some(&PrefValue::Bool(false))
        );
    }

    #[test]
    fn an_int_outside_i32_is_treated_as_the_wrong_type() {
        let dir = TestDir::new("overflow");
        write_file(
            &dir,
            CONFIG_FILE,
            "[library]\nnew_manga_updates_count = 3000000000\n",
        );
        let loaded = backend(&dir).unwrap().load().unwrap();
        assert!(!loaded.contains_key("library_unread_updates_count"));
    }

    #[test]
    fn an_unreadable_file_is_renamed_and_defaults_are_used() {
        let dir = TestDir::new("corrupt");
        write_file(&dir, CONFIG_FILE, "[library\nthis is not toml");
        let loaded = backend(&dir).unwrap().load().unwrap();
        assert!(loaded.is_empty());
        assert!(!dir.path().join("config").join(CONFIG_FILE).exists());
        let renamed: Vec<String> = fs::read_dir(dir.path().join("config"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("config.toml.corrupt-"))
            .collect();
        assert_eq!(renamed.len(), 1);
    }

    #[test]
    fn a_second_process_gets_locked_instead_of_blocking() {
        let dir = TestDir::new("lock");
        let first = backend(&dir).unwrap();
        let config = dir.path().join("config");
        let state = dir.path().join("state");
        match TomlBackend::open(&config, &state) {
            Err(PrefError::Locked) => {}
            Err(other) => panic!("expected Locked, got {other}"),
            Ok(_) => panic!("second open should be locked"),
        }
        drop(first);
        assert!(TomlBackend::open(&config, &state).is_ok());
    }

    #[test]
    fn concurrent_saves_always_leave_a_complete_file() {
        let dir = TestDir::new("concurrent");
        let b = std::sync::Arc::new(backend(&dir).unwrap());
        b.load().unwrap();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let b = b.clone();
                std::thread::spawn(move || {
                    for _ in 0..25 {
                        b.save(&sample()).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        drop(b);
        assert_eq!(backend(&dir).unwrap().load().unwrap(), sample());
        let names: Vec<_> = fs::read_dir(dir.path().join("config")).unwrap().flatten().collect();
        assert!(names.iter().all(|e| !e.file_name().to_string_lossy().contains("corrupt")));
    }
}
