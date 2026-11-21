//! Preference import from a backup (T-02a-026). The backup reader (area 22) supplies the entries and the
//! category id maps built from the restored categories; this module decides what is written.

use std::collections::{BTreeMap, BTreeSet};

use crate::keys::Namespace;
use crate::registry::{self, Kind};
use crate::store::PrefStore;
use crate::tracking::{CredentialError, TrackingCredentials};
use crate::values::PrefValue;

/// Old category id to new category id, one map for manga and one for anime (matched by category name).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CategoryMaps {
    pub manga: BTreeMap<i64, i64>,
    pub anime: BTreeMap<i64, i64>,
}

/// The six manga-side category-id keys (library and download): `category_keys(Kind::Manga).all()`.
pub const MANGA_CATEGORY_KEYS: [&str; 6] = crate::category::category_keys(kuro_platform::kind::Kind::Manga).all();

/// The six anime-side category-id keys (library and download): `category_keys(Kind::Anime).all()`.
pub const ANIME_CATEGORY_KEYS: [&str; 6] = crate::category::category_keys(kuro_platform::kind::Kind::Anime).all();

/// Schedules that must be recomputed after an import, because their preferences may have changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Schedule {
    MangaLibraryUpdate,
    AnimeLibraryUpdate,
}

/// What the import did. The restore dialog shows these counts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub written: usize,
    pub kind_mismatch: usize,
    pub unknown_kept: usize,
    /// Keys of dropped features (`registry::DROPPED_KEYS`): not written, not kept in `[extra]`.
    pub dropped_skipped: usize,
    pub app_state_skipped: usize,
    pub private_skipped: usize,
    pub private_restored: usize,
    pub unmapped_categories: usize,
    pub schedules: Vec<Schedule>,
    /// Legacy key conversions that ran before the write (see `legacy::convert`).
    pub conversions: Vec<crate::legacy::Conversion>,
}

fn kind_of(v: &PrefValue) -> Kind {
    match v {
        PrefValue::Int(_) => Kind::Int,
        PrefValue::Long(_) => Kind::Long,
        PrefValue::Float(_) => Kind::Float,
        PrefValue::Str(_) => Kind::Str,
        PrefValue::Bool(_) => Kind::Bool,
        PrefValue::Set(_) => Kind::Set,
    }
}

fn same_kind(a: &Kind, b: &Kind) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

/// The new id for one old category id. `-1` (none) and `0` (uncategorised) map to themselves.
fn remap(map: &BTreeMap<i64, i64>, old: i64) -> Option<i64> {
    if old == -1 || old == 0 {
        Some(old)
    } else {
        map.get(&old).copied()
    }
}

/// Imports `entries` into `store` as one batch (one `Changed { key: None }`). Returns the report.
///
/// Rules: app-state keys are skipped; private keys go to the secret store only when `include_private` is set
/// (`creds` must be given for that); keys the registry knows are written only when the value kind matches the
/// registry kind; keys of dropped features are counted and ignored; unknown keys are kept, so they land in `[extra]`. Category-id keys are remapped through the
/// manga or anime map. A set is unioned with the device set, and ids that do not map are dropped.
pub fn import_preferences(
    store: &PrefStore,
    mut entries: Vec<(String, PrefValue)>,
    maps: &CategoryMaps,
    creds: Option<&TrackingCredentials>,
    include_private: bool,
) -> Result<ImportReport, CredentialError> {
    let mut report = ImportReport { conversions: crate::legacy::convert(&mut entries), ..ImportReport::default() };
    let mut writes: Vec<(String, PrefValue)> = Vec::new();
    for (key, value) in entries {
        let key = crate::keys::restored_key(&key);
        if registry::is_dropped(&key) {
            report.dropped_skipped += 1;
            continue;
        }
        match Namespace::of(&key) {
            Namespace::AppState => report.app_state_skipped += 1,
            Namespace::Private => match creds {
                Some(c) if include_private && c.restore_private(&key, &value)? => report.private_restored += 1,
                _ => report.private_skipped += 1,
            },
            Namespace::Normal => match registry::lookup(&key) {
                None => {
                    report.unknown_kept += 1;
                    writes.push((key, value));
                }
                Some((meta, _)) if !same_kind(&meta.kind, &kind_of(&value)) => report.kind_mismatch += 1,
                Some(_) => {
                    let manga = MANGA_CATEGORY_KEYS.contains(&key.as_str());
                    let anime = ANIME_CATEGORY_KEYS.contains(&key.as_str());
                    if !manga && !anime {
                        writes.push((key, value));
                        continue;
                    }
                    let map = if manga { &maps.manga } else { &maps.anime };
                    match value {
                        PrefValue::Int(old) => match remap(map, i64::from(old)) {
                            Some(new) => writes.push((key, PrefValue::Int(new as i32))),
                            None => report.unmapped_categories += 1,
                        },
                        PrefValue::Set(ids) => {
                            let mut merged: BTreeSet<String> = match store.get_raw(&key) {
                                Some(PrefValue::Set(existing)) => existing,
                                _ => BTreeSet::new(),
                            };
                            for id in ids {
                                match id.parse::<i64>().ok().and_then(|old| remap(map, old)) {
                                    Some(new) => {
                                        merged.insert(new.to_string());
                                    }
                                    None => report.unmapped_categories += 1,
                                }
                            }
                            writes.push((key, PrefValue::Set(merged)));
                        }
                        other => writes.push((key, other)),
                    }
                }
            },
        }
    }
    report.written = writes.len();
    store.replace_many(writes);
    report.schedules = vec![Schedule::MangaLibraryUpdate, Schedule::AnimeLibraryUpdate];
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::testutil::MemorySecrets;

    fn maps() -> CategoryMaps {
        CategoryMaps {
            manga: BTreeMap::from([(5, 9), (6, 10)]),
            anime: BTreeMap::from([(5, 20)]),
        }
    }

    fn set(v: &[&str]) -> PrefValue {
        PrefValue::Set(v.iter().map(|s| (*s).to_owned()).collect())
    }

    fn run(store: &PrefStore, entries: Vec<(&str, PrefValue)>) -> ImportReport {
        let entries = entries.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
        import_preferences(store, entries, &maps(), None, false).unwrap()
    }

    #[test]
    fn known_keys_with_the_registry_kind_are_written() {
        let store = PrefStore::in_memory();
        let report = run(&store, vec![("show_nsfw_source", PrefValue::Bool(true))]);
        assert_eq!(store.get_raw("show_nsfw_source"), Some(PrefValue::Bool(true)));
        assert_eq!(report.written, 1);
    }

    #[test]
    fn kind_mismatch_is_skipped_and_counted() {
        let store = PrefStore::in_memory();
        let report = run(&store, vec![("show_nsfw_source", PrefValue::Str("yes".into()))]);
        assert_eq!(store.get_raw("show_nsfw_source"), None);
        assert_eq!(report.kind_mismatch, 1);
    }

    #[test]
    fn unknown_keys_are_kept_and_app_state_is_skipped() {
        let store = PrefStore::in_memory();
        let report = run(
            &store,
            vec![
                ("some_future_key", PrefValue::Long(7)),
                ("__APP_STATE_incognito_mode", PrefValue::Bool(true)),
            ],
        );
        assert_eq!(store.get_raw("some_future_key"), Some(PrefValue::Long(7)));
        assert_eq!(store.get_raw("__APP_STATE_incognito_mode"), None);
        assert_eq!((report.unknown_kept, report.app_state_skipped), (1, 1));
    }

    #[test]
    fn keys_of_dropped_features_are_ignored_not_kept() {
        let store = PrefStore::in_memory();
        let report = run(
            &store,
            vec![
                ("tablet_ui_mode", PrefValue::Str("ALWAYS".into())),
                ("secure_screen", PrefValue::Bool(true)),
                ("secure_screen_v2", PrefValue::Str("ALWAYS".into())),
                ("pref_chapter_swipe_end_action", PrefValue::Str("Download".into())),
                ("kuro_pref_gesture_volume_brightness", PrefValue::Bool(true)),
                ("show_nsfw_source", PrefValue::Bool(true)),
            ],
        );
        assert_eq!((report.dropped_skipped, report.unknown_kept, report.written), (5, 0, 1));
        for key in ["tablet_ui_mode", "secure_screen", "secure_screen_v2", "pref_chapter_swipe_end_action", "pref_gesture_volume_brightness"] {
            assert_eq!(store.get_raw(key), None, "{key}");
        }
        assert_eq!(store.get_raw("show_nsfw_source"), Some(PrefValue::Bool(true)));
    }

    #[test]
    fn private_entries_need_the_private_choice_and_a_keyring() {
        let store = PrefStore::in_memory();
        let creds = TrackingCredentials::new(store.clone(), Arc::new(MemorySecrets::default()));
        let entries = || vec![("__PRIVATE_track_token_4".to_owned(), PrefValue::Str("tok".into()))];

        let skipped = import_preferences(&store, entries(), &maps(), Some(&creds), false).unwrap();
        assert_eq!((skipped.private_skipped, skipped.private_restored), (1, 0));
        assert_eq!(creds.track_token(4).get().unwrap(), "");

        let restored = import_preferences(&store, entries(), &maps(), Some(&creds), true).unwrap();
        assert_eq!(restored.private_restored, 1);
        assert_eq!(creds.track_token(4).get().unwrap(), "tok");
    }

    #[test]
    fn default_category_is_remapped_by_name_and_unmapped_values_are_not_written() {
        let store = PrefStore::in_memory();
        run(&store, vec![("default_category", PrefValue::Int(5))]);
        assert_eq!(store.get_raw("default_category"), Some(PrefValue::Int(9)));

        let report = run(&store, vec![("default_category", PrefValue::Int(7))]);
        assert_eq!(store.get_raw("default_category"), Some(PrefValue::Int(9)), "unmapped does not overwrite");
        assert_eq!(report.unmapped_categories, 1);

        run(&store, vec![("default_category", PrefValue::Int(-1))]);
        assert_eq!(store.get_raw("default_category"), Some(PrefValue::Int(-1)));
        run(&store, vec![("default_category", PrefValue::Int(0))]);
        assert_eq!(store.get_raw("default_category"), Some(PrefValue::Int(0)));
    }

    #[test]
    fn anime_default_uses_the_anime_map() {
        let store = PrefStore::in_memory();
        run(&store, vec![("default_anime_category", PrefValue::Int(5))]);
        assert_eq!(store.get_raw("default_anime_category"), Some(PrefValue::Int(20)));
    }

    #[test]
    fn category_set_keys_are_cut_so_an_import_skips_them_and_leaves_the_device_set() {
        let store = PrefStore::in_memory();
        store.set_raw("library_update_categories", set(&["1"]));
        let report = run(&store, vec![("library_update_categories", set(&["5", "6", "99"]))]);
        assert_eq!(store.get_raw("library_update_categories"), Some(set(&["1"])));
        assert_eq!(report.dropped_skipped, 1);
    }

    #[test]
    fn a_set_where_nothing_maps_changes_nothing() {
        let store = PrefStore::in_memory();
        store.set_raw("download_new_categories", set(&["1"]));
        run(&store, vec![("download_new_categories", set(&["77"]))]);
        assert_eq!(store.get_raw("download_new_categories"), Some(set(&["1"])));
    }

    #[test]
    fn one_change_event_and_the_same_result_on_a_second_import() {
        let store = PrefStore::in_memory();
        let rx = store.subscribe();
        let entries = || {
            vec![
                ("show_nsfw_source".to_owned(), PrefValue::Bool(true)),
                ("default_category".to_owned(), PrefValue::Int(5)),
            ]
        };
        import_preferences(&store, entries(), &maps(), None, false).unwrap();
        let first = store.all();
        import_preferences(&store, entries(), &maps(), None, false).unwrap();
        assert_eq!(store.all(), first, "importing twice gives the same values");
        let events: Vec<_> = rx.try_iter().collect();
        assert_eq!(events.len(), 1, "one event for the first import; the second changes nothing");
        assert!(events.iter().all(|c| c.key.is_none()));
    }

    #[test]
    fn schedules_are_reported_for_recomputation() {
        let store = PrefStore::in_memory();
        let report = run(&store, vec![]);
        assert_eq!(
            report.schedules,
            [Schedule::MangaLibraryUpdate, Schedule::AnimeLibraryUpdate]
        );
    }

    #[test]
    fn the_key_lists_hold_twelve_distinct_category_keys_and_only_the_two_defaults_are_registered() {
        let all: BTreeSet<&str> = MANGA_CATEGORY_KEYS.iter().chain(ANIME_CATEGORY_KEYS.iter()).copied().collect();
        assert_eq!(all.len(), 12);
        for key in all {
            let kept = key == "default_category" || key == "default_anime_category";
            assert_eq!(registry::lookup(key).is_some(), kept, "{key}");
            assert_eq!(registry::is_dropped(key), !kept, "{key}");
        }
    }
}
