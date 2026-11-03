//! Preference restore (area 22, T-22a-009 and T-22a-010). The write rules live in `kuro_prefs::import`; this module
//! converts the backup values, maps the category ids, and re-arms the backup scheduler after an app-preference section.

use std::collections::{BTreeMap, HashMap};

use kuro_prefs::import::{ANIME_CATEGORY_KEYS, CategoryMaps, ImportReport, MANGA_CATEGORY_KEYS, import_preferences};
use kuro_prefs::keys::restored_key;
use kuro_prefs::legacy::{
    EXTENSION_REPO_SETS, LEGACY_UPDATE_ONLY_NON_COMPLETED, UPDATE_INTERVAL, UPDATE_RESTRICTIONS,
};
use kuro_prefs::registry::{self, Kind};
use kuro_prefs::store::PrefStore;
use kuro_prefs::values::PrefValue as Value;
use kuro_db::DbResult;
use rusqlite::Connection;

use crate::schema::{BackupCategory, BackupPreference, BackupSourcePreferences, PrefValue};

/// The scheduler re-arm after app preferences change. Implemented by the owning areas.
pub trait Scheduler {
    /// Reads `backup_interval`. 0 disables the automatic backup.
    fn rearm_auto_backup(&self);
}

/// Reads the backup interval after a restore and hands the timer its state: `None` disables it (interval 0),
/// `Some(hours)` enables it with the restored value (T-22a-010). The callback belongs to the owning area; passing
/// `&|_| {}` makes this a no-op until it lands.
pub struct IntervalScheduler<'a> {
    pub store: &'a PrefStore,
    pub backup_timer: &'a dyn Fn(Option<u32>),
}

impl IntervalScheduler<'_> {
    fn hours(&self, key: &str, default: i32) -> Option<u32> {
        let n = match self.store.get_raw(key) {
            Some(Value::Int(n)) => n,
            _ => default,
        };
        u32::try_from(n).ok().filter(|h| *h > 0)
    }
}

impl Scheduler for IntervalScheduler<'_> {
    fn rearm_auto_backup(&self) {
        (self.backup_timer)(self.hours("backup_interval", 12));
    }
}

fn convert(v: &PrefValue) -> Value {
    match v {
        PrefValue::Int(i) => Value::Int(*i),
        PrefValue::Long(l) => Value::Long(*l),
        PrefValue::Float(f) => Value::Float(*f),
        PrefValue::String(s) => Value::Str(s.clone()),
        PrefValue::Boolean(b) => Value::Bool(*b),
        PrefValue::StringSet(s) => Value::Set(s.clone()),
    }
}

/// The prefix of app-state keys. They are never restored from a backup (T-22a-052).
const APP_STATE_PREFIX: &str = "__APP_STATE_";

/// Legacy names that `legacy::convert` rewrites before the write. They are not in the registry, but a restore must
/// keep them (T-22a-052).
const RESTORABLE_LEGACY_KEYS: [&str; 5] = [
    LEGACY_UPDATE_ONLY_NON_COMPLETED,
    UPDATE_INTERVAL,
    UPDATE_RESTRICTIONS,
    EXTENSION_REPO_SETS[0],
    EXTENSION_REPO_SETS[1],
];

/// The allowlist of an app-preference restore (T-22a-052). A key is written only when the Kurobayomi schema names it or
/// it is a legacy key above. App-state keys and the keys of dropped features (`registry::DROPPED_KEYS`: the installer
/// choice, orientation, secure screen, tablet mode, swipe actions, volume and brightness gestures) are never written;
/// their value is read from the backup and ignored, with no error. Everything else, such as a key the app
/// does not know, is skipped and counted.
fn is_restorable(key: &str) -> bool {
    if key.starts_with(APP_STATE_PREFIX) {
        return false;
    }
    let current = restored_key(key);
    if registry::is_dropped(&current) {
        return false;
    }
    registry::lookup(&current).is_some() || RESTORABLE_LEGACY_KEYS.contains(&current.as_str())
}

/// The two default-category keys hold one id, remapped (T-22a-009).
fn is_default_category(key: &str) -> bool {
    key == "default_category" || key == "default_anime_category"
}

/// The category id-set keys: the twelve category keys minus the two defaults.
fn is_category_set(key: &str) -> bool {
    MANGA_CATEGORY_KEYS[1..].contains(&key) || ANIME_CATEGORY_KEYS[1..].contains(&key)
}

/// Turns the backup entries into the writes that `import_preferences` performs. Three rules sit on top of it:
/// an unmapped default id (including -1) is dropped, so the setting stays; a set with no mapped id is dropped; and a
/// mapped set is cleared first, so the import replaces it instead of uniting it with the device set (D12).
fn prepare(store: &PrefStore, prefs: &[BackupPreference], maps: &CategoryMaps) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for p in prefs {
        let key = p.key.as_str();
        let value = convert(&p.value);
        // A stored value of another type wins, for registered and unregistered keys alike (E-53). An absent key is written.
        if store.get_raw(key).is_some_and(|stored| std::mem::discriminant(&stored) != std::mem::discriminant(&value)) {
            continue;
        }
        if is_default_category(key) {
            let map = map_for(key, maps);
            match value {
                Value::Int(old) if !map.contains_key(&i64::from(old)) => continue,
                _ => out.push((p.key.clone(), value)),
            }
        } else if is_category_set(key) {
            if let Value::Set(ids) = &value {
                let map = map_for(key, maps);
                let mapped = ids.iter().any(|id| id.parse::<i64>().is_ok_and(|old| map.contains_key(&old)));
                if !mapped {
                    continue;
                }
                if matches!(registry::lookup(key).map(|(m, _)| &m.kind), Some(Kind::Set)) {
                    store.remove(key);
                }
            }
            out.push((p.key.clone(), value));
        } else {
            out.push((p.key.clone(), value));
        }
    }
    out
}

fn map_for<'a>(key: &str, maps: &'a CategoryMaps) -> &'a BTreeMap<i64, i64> {
    if MANGA_CATEGORY_KEYS.contains(&key) { &maps.manga } else { &maps.anime }
}

/// What an app-preference restore did: the import report, and how many backup keys the allowlist skipped (T-22a-052).
#[derive(Debug, Default)]
pub struct AppPrefsRestore {
    pub report: ImportReport,
    pub skipped: usize,
}

/// Restores the app preferences (T-22a-009), writes `categorized_display` when the categories section computed it,
/// then re-arms the backup scheduler once (T-22a-010, X-57). Keys the allowlist refuses are skipped and counted.
pub fn restore_app_prefs(
    store: &PrefStore,
    prefs: &[BackupPreference],
    maps: &CategoryMaps,
    categorized_display: Option<bool>,
    scheduler: &dyn Scheduler,
) -> AppPrefsRestore {
    let allowed: Vec<BackupPreference> = prefs.iter().filter(|p| is_restorable(&p.key)).cloned().collect();
    let skipped = prefs.len() - allowed.len();
    let entries = prepare(store, &allowed, maps);
    // No credentials are passed, so `import_preferences` has no error path. Private keys are counted as skipped.
    let report = import_preferences(store, entries, maps, None, false).unwrap_or_default();
    if let Some(flag) = categorized_display {
        store.set_raw("categorized_display", Value::Bool(flag));
    }
    scheduler.rearm_auto_backup();
    AppPrefsRestore { report, skipped }
}

/// Restores each source's preferences into the store `source_<sourceId>` (T-22a-009, E-52). `open` returns the store
/// for a name. Source files carry no category ids, so no maps apply.
pub fn restore_source_prefs(
    open: &dyn Fn(&str) -> PrefStore,
    sources: &[BackupSourcePreferences],
) -> Vec<ImportReport> {
    let none = CategoryMaps::default();
    sources
        .iter()
        .map(|s| {
            let store = open(&format!("source_{}", s.source_key));
            let entries = prepare(&store, &s.prefs, &none);
            import_preferences(&store, entries, &none, None, false).unwrap_or_default()
        })
        .collect()
}

/// The category maps of the preference restore (T-22a-009): backup category id (the device id) to the database id of
/// the same name, once for manga and once for anime. Both lists come from the backup's manga categories.
pub fn category_maps(c: &Connection, backup: &[BackupCategory]) -> DbResult<CategoryMaps> {
    let by_name = |kind: kuro_platform::kind::Kind| -> DbResult<HashMap<String, i64>> {
        Ok(kuro_db::tables::categories::all(c, kind)?.into_iter().map(|r| (r.name, r.id)).collect())
    };
    let manga_by_name = by_name(kuro_platform::kind::Kind::Manga)?;
    let anime_by_name = by_name(kuro_platform::kind::Kind::Anime)?;
    let mut maps = CategoryMaps::default();
    for b in backup {
        if let Some(&id) = manga_by_name.get(&b.name) {
            maps.manga.insert(b.id, id);
        }
        if let Some(&id) = anime_by_name.get(&b.name) {
            maps.anime.insert(b.id, id);
        }
    }
    Ok(maps)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::BTreeSet;

    use super::*;

    #[derive(Default)]
    struct Count {
        backup: Cell<u32>,
    }

    impl Scheduler for Count {
        fn rearm_auto_backup(&self) {
            self.backup.set(self.backup.get() + 1);
        }
    }

    fn pref(key: &str, value: PrefValue) -> BackupPreference {
        BackupPreference { key: key.into(), value }
    }

    fn set(ids: &[&str]) -> PrefValue {
        PrefValue::StringSet(ids.iter().map(|s| (*s).to_owned()).collect::<BTreeSet<_>>())
    }

    fn maps() -> CategoryMaps {
        CategoryMaps { manga: BTreeMap::from([(5, 9), (6, 10)]), anime: BTreeMap::from([(5, 20)]) }
    }

    #[test]
    fn stored_type_mismatch_is_skipped_and_absent_key_is_written() {
        let store = PrefStore::in_memory();
        store.set_raw("show_nsfw_source", Value::Bool(false));
        let sched = Count::default();
        let prefs = vec![pref("show_nsfw_source", PrefValue::String("yes".into()))];
        restore_app_prefs(&store, &prefs, &CategoryMaps::default(), None, &sched);
        assert_eq!(store.get_raw("show_nsfw_source"), Some(Value::Bool(false)));
    }

    #[test]
    fn unregistered_keys_are_skipped_and_counted_by_the_allowlist() {
        let store = PrefStore::in_memory();
        store.set_raw("not_in_registry", Value::Bool(true));
        let prefs = vec![
            pref("not_in_registry", PrefValue::String("x".into())),
            pref("other_unknown", PrefValue::String("y".into())),
        ];
        let outcome = restore_app_prefs(&store, &prefs, &CategoryMaps::default(), None, &Count::default());
        assert_eq!(store.get_raw("not_in_registry"), Some(Value::Bool(true)));
        assert_eq!(store.get_raw("other_unknown"), None, "an unknown key is never written");
        assert_eq!(outcome.skipped, 2);
    }

    #[test]
    fn keys_outside_the_schema_and_app_state_keys_are_skipped_and_counted() {
        let store = PrefStore::in_memory();
        let prefs = vec![
            pref("other_unknown", PrefValue::String("y".into())),
            pref("__APP_STATE_last_auto_backup_timestamp", PrefValue::Long(5)),
            pref("extension_installer", PrefValue::String("SHIZUKU".into())),
            pref("show_nsfw_source", PrefValue::Boolean(true)),
        ];
        let outcome = restore_app_prefs(&store, &prefs, &CategoryMaps::default(), None, &Count::default());
        assert_eq!(outcome.skipped, 3);
        assert_eq!(outcome.report.unknown_kept, 0, "nothing unknown reaches the write");
        assert_eq!(store.get_raw("extension_installer"), None, "the removed installer preference is ignored, with no error");
        assert_eq!(store.get_raw("show_nsfw_source"), Some(Value::Bool(true)));
    }

    #[test]
    fn keys_of_removed_desktop_features_are_skipped_without_error() {
        let store = PrefStore::in_memory();
        let prefs = vec![
            pref("tablet_ui_mode", PrefValue::String("ALWAYS".into())),
            pref("pref_default_orientation_type_key", PrefValue::Int(16)),
            pref("pref_default_player_orientation_type_key", PrefValue::String("Free".into())),
            pref("secure_screen_v2", PrefValue::String("ALWAYS".into())),
            pref("secure_screen", PrefValue::Boolean(true)),
            pref("pref_chapter_swipe_end_action", PrefValue::String("Download".into())),
            pref("pref_episode_swipe_start_action", PrefValue::String("Disabled".into())),
            pref("pref_gesture_volume_brightness", PrefValue::Boolean(false)),
            pref("pref_remember_brightness", PrefValue::Boolean(true)),
            pref("pref_app_theme", PrefValue::String("MONET".into())),
            pref("show_nsfw_source", PrefValue::Boolean(true)),
        ];
        let outcome = restore_app_prefs(&store, &prefs, &CategoryMaps::default(), None, &Count::default());
        assert_eq!(outcome.skipped, 10, "the ten dropped keys, the old app theme among them");
        assert_eq!(store.all().len(), 1, "only the flag is written");
        assert_eq!(store.get_raw("show_nsfw_source"), Some(Value::Bool(true)));
    }

    #[test]
    fn a_backup_with_only_known_keys_skips_nothing() {
        let store = PrefStore::in_memory();
        let outcome = restore_app_prefs(
            &store,
            &[pref("show_nsfw_source", PrefValue::Boolean(true))],
            &CategoryMaps::default(),
            None,
            &Count::default(),
        );
        assert_eq!(outcome.skipped, 0);
        assert_eq!(store.get_raw("show_nsfw_source"), Some(Value::Bool(true)));
    }

    #[test]
    fn a_legacy_key_passes_the_allowlist_so_its_conversion_still_runs() {
        let store = PrefStore::in_memory();
        let outcome = restore_app_prefs(
            &store,
            &[pref("extension_repos", set(&["https://r/one"]))],
            &CategoryMaps::default(),
            None,
            &Count::default(),
        );
        assert_eq!(outcome.skipped, 0);
        assert!(!outcome.report.conversions.is_empty(), "the repository set is handed on, not dropped");
    }

    #[test]
    fn absent_registry_key_with_matching_kind_is_written() {
        let store = PrefStore::in_memory();
        restore_app_prefs(
            &store,
            &[pref("show_nsfw_source", PrefValue::Boolean(true))],
            &CategoryMaps::default(),
            None,
            &Count::default(),
        );
        assert_eq!(store.get_raw("show_nsfw_source"), Some(Value::Bool(true)));
    }

    #[test]
    fn default_category_is_remapped_and_unmapped_or_minus_one_keeps_the_setting() {
        let store = PrefStore::in_memory();
        store.set_raw("default_category", Value::Int(3));
        let sched = Count::default();
        restore_app_prefs(&store, &[pref("default_category", PrefValue::Int(5))], &maps(), None, &sched);
        assert_eq!(store.get_raw("default_category"), Some(Value::Int(9)));

        restore_app_prefs(&store, &[pref("default_category", PrefValue::Int(-1))], &maps(), None, &sched);
        restore_app_prefs(&store, &[pref("default_category", PrefValue::Int(77))], &maps(), None, &sched);
        assert_eq!(store.get_raw("default_category"), Some(Value::Int(9)));
    }

    #[test]
    fn anime_default_uses_the_anime_map() {
        let store = PrefStore::in_memory();
        restore_app_prefs(
            &store,
            &[pref("default_anime_category", PrefValue::Int(5))],
            &maps(),
            None,
            &Count::default(),
        );
        assert_eq!(store.get_raw("default_anime_category"), Some(Value::Int(20)));
    }

    #[test]
    fn category_set_keys_are_cut_so_a_restore_skips_them_and_leaves_the_device_set() {
        let store = PrefStore::in_memory();
        store.set_raw("library_update_categories", Value::Set(BTreeSet::from(["1".to_string()])));
        let outcome = restore_app_prefs(
            &store,
            &[pref("library_update_categories", set(&["5", "6"]))],
            &maps(),
            None,
            &Count::default(),
        );
        assert_eq!(outcome.skipped, 1);
        assert_eq!(store.get_raw("library_update_categories"), Some(Value::Set(BTreeSet::from(["1".to_string()]))));
    }

    #[test]
    fn the_backup_scheduler_is_rearmed_exactly_once_per_app_section() {
        let store = PrefStore::in_memory();
        let sched = Count::default();
        restore_app_prefs(&store, &[], &CategoryMaps::default(), None, &sched);
        assert_eq!(sched.backup.get(), 1);
    }

    #[test]
    fn interval_zero_disables_the_timer_and_non_zero_enables_it() {
        let store = PrefStore::in_memory();
        let calls = std::cell::RefCell::new(Vec::new());
        let backup = |h: Option<u32>| calls.borrow_mut().push(h);
        let sched = IntervalScheduler { store: &store, backup_timer: &backup };
        restore_app_prefs(&store, &[pref("backup_interval", PrefValue::Int(0))], &CategoryMaps::default(), None, &sched);
        restore_app_prefs(&store, &[pref("backup_interval", PrefValue::Int(6))], &CategoryMaps::default(), None, &sched);
        assert_eq!(calls.into_inner(), vec![None, Some(6)]);
    }

    #[test]
    fn absent_backup_interval_uses_the_default_of_twelve_hours() {
        let store = PrefStore::in_memory();
        let got = std::cell::Cell::new(None);
        let backup = |h: Option<u32>| got.set(h);
        IntervalScheduler { store: &store, backup_timer: &backup }.rearm_auto_backup();
        assert_eq!(got.get(), Some(12));
    }

    #[test]
    fn categorized_display_is_written_only_when_computed() {
        let store = PrefStore::in_memory();
        restore_app_prefs(&store, &[], &CategoryMaps::default(), Some(true), &Count::default());
        assert_eq!(store.get_raw("categorized_display"), Some(Value::Bool(true)));
    }

    #[test]
    fn source_preferences_go_to_the_store_named_for_each_source() {
        let names = std::cell::RefCell::new(Vec::new());
        let open = |name: &str| {
            names.borrow_mut().push(name.to_owned());
            PrefStore::in_memory()
        };
        let sources = vec![
            BackupSourcePreferences { source_key: "1234".into(), prefs: vec![pref("show_nsfw_source", PrefValue::Boolean(true))] },
            BackupSourcePreferences { source_key: "99".into(), prefs: vec![] },
        ];
        let reports = restore_source_prefs(&open, &sources);
        assert_eq!(names.into_inner(), vec!["source_1234".to_string(), "source_99".to_string()]);
        assert_eq!(reports.len(), 2);
    }
}
