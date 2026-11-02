//! Legacy key conversions applied before a preference import (T-02a-029). A backup from an older Aniyomi carries keys
//! that were renamed, retired or merged. [`convert`] rewrites them to the current keys and lists what it did, so the
//! restore report can show every change. Keys with no conversion pass through untouched.

use crate::values::PrefValue;

/// Legacy boolean: when false the "ongoing only" restriction of library updates was on in the old split.
pub const LEGACY_UPDATE_ONLY_NON_COMPLETED: &str = "pref_update_only_non_completed_key";
/// Library update interval (hours), `Int`.
pub const UPDATE_INTERVAL: &str = "pref_library_update_interval_key";
/// Library update item restrictions, `Set`.
pub const UPDATE_RESTRICTIONS: &str = "library_update_manga_restriction";
/// The restriction value that "only non-completed" maps to.
pub const ENTRY_NON_COMPLETED: &str = "manga_ongoing";
/// Extension repository sets, handed to the repository importer (area 20) after the write.
pub const EXTENSION_REPO_SETS: [&str; 2] = ["extension_repos", "anime_extension_repos"];

/// One conversion that was applied. Shown in the restore report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conversion {
    /// An interval of 1 or 2 hours (removed in 2.x) became 3.
    IntervalRaised { from: i32 },
    /// A removed quick interval (3, 4, 6 or 8 hours) became 12.
    RetiredIntervalMoved { from: i32 },
    /// `pref_update_only_non_completed_key = false` removed `manga_ongoing` from the update restrictions.
    UpdateRestrictionCombined,
    /// An extension repository set with `count` entries is handed to the repository importer.
    ExtensionReposHanded { key: String, count: usize },
}

fn take(entries: &mut Vec<(String, PrefValue)>, key: &str) -> Option<PrefValue> {
    let at = entries.iter().position(|(k, _)| k == key)?;
    Some(entries.remove(at).1)
}

fn set_value(entries: &mut Vec<(String, PrefValue)>, key: &str, value: PrefValue) {
    match entries.iter_mut().find(|(k, _)| k == key) {
        Some((_, v)) => *v = value,
        None => entries.push((key.to_owned(), value)),
    }
}

/// Applies every conversion to `entries` in place and returns the ones that ran, in a fixed order.
pub fn convert(entries: &mut Vec<(String, PrefValue)>) -> Vec<Conversion> {
    let mut done = Vec::new();

    if let Some(PrefValue::Int(hours)) = entries.iter().find(|(k, _)| k == UPDATE_INTERVAL).map(|(_, v)| v.clone()) {
        let next = match hours {
            1 | 2 => Some(3),
            3 | 4 | 6 | 8 => Some(12),
            _ => None,
        };
        if let Some(next) = next {
            set_value(entries, UPDATE_INTERVAL, PrefValue::Int(next));
            done.push(if next == 3 {
                Conversion::IntervalRaised { from: hours }
            } else {
                Conversion::RetiredIntervalMoved { from: hours }
            });
        }
    }

    if let Some(non_completed) = take(entries, LEGACY_UPDATE_ONLY_NON_COMPLETED) {
        if non_completed == PrefValue::Bool(false) {
            let restrictions = entries.iter().find(|(k, _)| k == UPDATE_RESTRICTIONS).map(|(_, v)| v.clone());
            if let Some(PrefValue::Set(mut set)) = restrictions {
                if set.remove(ENTRY_NON_COMPLETED) {
                    set_value(entries, UPDATE_RESTRICTIONS, PrefValue::Set(set));
                    done.push(Conversion::UpdateRestrictionCombined);
                }
            }
        }
    }

    for key in EXTENSION_REPO_SETS {
        if let Some((_, PrefValue::Set(repos))) = entries.iter().find(|(k, _)| k == key) {
            if !repos.is_empty() {
                done.push(Conversion::ExtensionReposHanded { key: key.to_owned(), count: repos.len() });
            }
        }
    }
    done
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::import::{CategoryMaps, import_preferences};
    use crate::store::PrefStore;

    fn set(items: &[&str]) -> PrefValue {
        PrefValue::Set(items.iter().map(|s| (*s).to_owned()).collect::<BTreeSet<_>>())
    }

    fn entry(key: &str, value: PrefValue) -> (String, PrefValue) {
        (key.to_owned(), value)
    }

    #[test]
    fn one_and_two_hour_intervals_become_three() {
        for from in [1, 2] {
            let mut e = vec![entry(UPDATE_INTERVAL, PrefValue::Int(from))];
            assert_eq!(convert(&mut e), vec![Conversion::IntervalRaised { from }]);
            assert_eq!(e, vec![entry(UPDATE_INTERVAL, PrefValue::Int(3))]);
        }
    }

    #[test]
    fn the_retired_quick_intervals_become_twelve() {
        for from in [3, 4, 6, 8] {
            let mut e = vec![entry(UPDATE_INTERVAL, PrefValue::Int(from))];
            assert_eq!(convert(&mut e), vec![Conversion::RetiredIntervalMoved { from }]);
            assert_eq!(e, vec![entry(UPDATE_INTERVAL, PrefValue::Int(12))]);
        }
    }

    #[test]
    fn a_supported_interval_and_a_zero_backup_interval_pass_through() {
        let mut e = vec![entry(UPDATE_INTERVAL, PrefValue::Int(12)), entry("backup_interval", PrefValue::Int(0))];
        let before = e.clone();
        assert!(convert(&mut e).is_empty());
        assert_eq!(e, before, "backup_interval 0 stays 0: the auto-backup migration is not run on import");
    }

    #[test]
    fn non_completed_off_removes_ongoing_from_the_restrictions() {
        let mut e = vec![
            entry(LEGACY_UPDATE_ONLY_NON_COMPLETED, PrefValue::Bool(false)),
            entry(UPDATE_RESTRICTIONS, set(&["manga_ongoing", "manga_started"])),
        ];
        assert_eq!(convert(&mut e), vec![Conversion::UpdateRestrictionCombined]);
        assert_eq!(e, vec![entry(UPDATE_RESTRICTIONS, set(&["manga_started"]))]);
    }

    #[test]
    fn non_completed_on_changes_no_restriction() {
        let mut e = vec![
            entry(LEGACY_UPDATE_ONLY_NON_COMPLETED, PrefValue::Bool(true)),
            entry(UPDATE_RESTRICTIONS, set(&["manga_ongoing"])),
        ];
        assert!(convert(&mut e).is_empty());
        assert_eq!(e, vec![entry(UPDATE_RESTRICTIONS, set(&["manga_ongoing"]))]);
    }

    #[test]
    fn extension_repo_sets_are_reported_for_the_repository_importer() {
        let mut e = vec![
            entry("extension_repos", set(&["https://a.example/index.min.json"])),
            entry("anime_extension_repos", set(&[])),
        ];
        assert_eq!(
            convert(&mut e),
            vec![Conversion::ExtensionReposHanded { key: "extension_repos".into(), count: 1 }]
        );
        assert_eq!(e.len(), 2, "the sets are still written: the importer reads them from the store");
    }

    #[test]
    fn the_import_report_lists_the_conversions_and_skips_the_cut_interval() {
        let store = PrefStore::in_memory();
        let entries = vec![entry(UPDATE_INTERVAL, PrefValue::Int(1))];
        let report = import_preferences(&store, entries, &CategoryMaps::default(), None, false).unwrap();
        assert_eq!(report.conversions, vec![Conversion::IntervalRaised { from: 1 }]);
        assert_eq!(store.get_raw(UPDATE_INTERVAL), None, "the interval is cut (DROPPED_KEYS), so the new value is not written");
    }
}
