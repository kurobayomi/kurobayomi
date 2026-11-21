//! Preference export into backup entries (T-02a-025).
//!
//! `export_app` and `export_sources` decide which stored values go into a backup and under which key. Only
//! explicitly stored values are exported (defaults are never written), app state never is, and private values
//! only when the user asked for them. Keys come out sorted, so the same store always gives the same entries.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::keys::{backup_key, Namespace};
use crate::source_file::{SourceFilePrefs, SOURCES_DIR};
use crate::store::PrefStore;
use crate::values::PrefValue;

/// Whether a stored key goes into a backup. App state never does; private keys only with `include_private`.
fn exported(key: &str, include_private: bool) -> bool {
    match Namespace::of(key) {
        Namespace::Normal => true,
        Namespace::Private => include_private,
        Namespace::AppState => false,
    }
}

/// The stored values of `store` that a backup carries, each under its backup key, sorted by key.
fn entries(store: &PrefStore, include_private: bool) -> Vec<(String, PrefValue)> {
    let mut out: BTreeMap<String, PrefValue> = BTreeMap::new();
    for (key, value) in store.all() {
        if exported(&key, include_private) {
            out.insert(backup_key(&key), value);
        }
    }
    out.into_iter().collect()
}

/// The app preferences for a backup: explicitly stored normal keys under their Aniyomi names (Kurobayomi-only
/// keys get the `kuro_` prefix), sorted by key. `include_private` adds the `__PRIVATE_` keys.
pub fn export_app(store: &PrefStore, include_private: bool) -> Vec<(String, PrefValue)> {
    entries(store, include_private)
}

/// The source preferences for a backup: one `(source id, entries)` per source file that holds at least one
/// exported key, in ascending id order. Files are found under `<config_dir>/sources`.
pub fn export_sources(config_dir: &Path, include_private: bool) -> Vec<(i64, Vec<(String, PrefValue)>)> {
    let Ok(read) = fs::read_dir(config_dir.join(SOURCES_DIR)) else {
        return Vec::new();
    };
    let mut ids: Vec<i64> = read
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_prefix("source_")?.strip_suffix(".toml")?.parse::<i64>().ok()
        })
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .filter_map(|id| {
            let file = SourceFilePrefs::open(config_dir, id).ok()?;
            let kept = entries(&file, include_private);
            (!kept.is_empty()).then_some((id, kept))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with(pairs: &[(&str, PrefValue)]) -> PrefStore {
        let store = PrefStore::in_memory();
        for (key, value) in pairs {
            store.set_raw(key, value.clone());
        }
        store
    }

    fn keys(entries: &[(String, PrefValue)]) -> Vec<&str> {
        entries.iter().map(|(k, _)| k.as_str()).collect()
    }

    #[test]
    fn app_export_skips_app_state_and_private_unless_asked() {
        let store = store_with(&[
            ("pref_default_reading_mode_key", PrefValue::Str("dark".into())),
            ("__APP_STATE_last_app_check", PrefValue::Long(5)),
            ("__PRIVATE_track_token_2", PrefValue::Str("secret".into())),
            ("kuro_only_setting", PrefValue::Int(3)),
        ]);
        assert_eq!(keys(&export_app(&store, false)), vec!["kuro_kuro_only_setting", "pref_default_reading_mode_key"]);
        assert_eq!(
            keys(&export_app(&store, true)),
            vec!["__PRIVATE_track_token_2", "kuro_kuro_only_setting", "pref_default_reading_mode_key"]
        );
    }

    #[test]
    fn app_export_keeps_every_kind_and_is_sorted() {
        let store = store_with(&[
            ("show_nsfw_source", PrefValue::Bool(true)),
            ("source_languages", PrefValue::Float(1.5)),
            ("__APP_STATE_incognito_mode", PrefValue::Bool(true)),
        ]);
        let out = export_app(&store, false);
        assert_eq!(keys(&out), vec!["show_nsfw_source", "source_languages"]);
        assert_eq!(out[0].1, PrefValue::Bool(true));
        assert_eq!(out[1].1, PrefValue::Float(1.5));
    }

    #[test]
    fn source_export_lists_only_files_with_keys_in_ascending_id_order() {
        let dir = tempfile::TempDir::new().unwrap();
        for (id, key) in [(9_i64, "show_nsfw_source"), (2, "pref_default_reading_mode_key"), (5, "custom_x")] {
            let file = SourceFilePrefs::open(dir.path(), id).unwrap();
            file.set_raw(key, PrefValue::Int(id as i32));
            file.flush().unwrap();
        }
        let empty = SourceFilePrefs::open(dir.path(), 7).unwrap();
        empty.set_raw("__APP_STATE_only", PrefValue::Bool(true));
        empty.flush().unwrap();
        let out = export_sources(dir.path(), false);
        let ids: Vec<i64> = out.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![2, 5, 9], "file 7 holds only app state and is left out");
        assert_eq!(out[0].1, vec![("pref_default_reading_mode_key".to_string(), PrefValue::Int(2))]);
        assert_eq!(out[1].1, vec![("kuro_custom_x".to_string(), PrefValue::Int(5))], "a key outside the registry gets the kuro prefix");
    }

    #[test]
    fn missing_sources_folder_exports_nothing() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(export_sources(dir.path(), true).is_empty());
    }
}
