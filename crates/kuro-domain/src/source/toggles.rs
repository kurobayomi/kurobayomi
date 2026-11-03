//! The pure toggles of the source lists (`ToggleMangaSource`, `ToggleLanguage`;
//! T-17a-019, T-17a-020, T-17a-022). Each function changes one set and returns the new
//! membership, so the UI can update without reading the set again. A set holds decimal source ids.
//! The naming trap is gone: a "disabled" set is never read through a helper called `is_enabled`.

use std::collections::BTreeSet;

use crate::source::model::Id;

/// Hides source `id` when it is shown, shows it when it is hidden (`ToggleMangaSource.await(source)`). Returns `true` when
/// the source is hidden (disabled) after the flip.
pub fn flip_disabled(disabled: &mut BTreeSet<String>, id: Id) -> bool {
    toggle_member(disabled, id)
}

/// Hides (`disable = true`) or shows (`false`) every source in `ids` in one change. Returns `disable`, the membership every
/// id has afterwards. An empty `ids` changes nothing.
pub fn set_disabled(disabled: &mut BTreeSet<String>, ids: &[Id], disable: bool) -> bool {
    for id in ids {
        if disable {
            disabled.insert(id.to_string());
        } else {
            disabled.remove(&id.to_string());
        }
    }
    disable
}

/// Adds a language code that is absent, removes one that is present (`ToggleLanguage`). Codes are compared exactly.
/// Returns `true` when the language is enabled after the call. No per-source set is touched.
pub fn toggle_language(languages: &mut BTreeSet<String>, language: &str) -> bool {
    if languages.remove(language) {
        false
    } else {
        languages.insert(language.to_owned());
        true
    }
}

/// Flips membership of the decimal form of `id`, and returns whether it is a member afterwards.
fn toggle_member(set: &mut BTreeSet<String>, id: Id) -> bool {
    let key = id.to_string();
    if set.remove(&key) {
        false
    } else {
        set.insert(key);
        true
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_flip_reports_the_new_membership() {
        let mut disabled = set(&[]);
        assert!(flip_disabled(&mut disabled, 5), "now disabled");
        assert_eq!(disabled, set(&["5"]));
        assert!(!flip_disabled(&mut disabled, 5), "now enabled");
        assert!(disabled.is_empty());
    }

    #[test]
    fn a_bulk_set_writes_every_id_and_returns_the_applied_state() {
        let mut disabled = set(&["9"]);
        assert!(set_disabled(&mut disabled, &[1, 2, 9], true));
        assert_eq!(disabled, set(&["1", "2", "9"]));
        assert!(!set_disabled(&mut disabled, &[1, 9], false));
        assert_eq!(disabled, set(&["2"]));
        assert!(!set_disabled(&mut disabled, &[], false), "no ids changes nothing");
        assert_eq!(disabled, set(&["2"]));
    }

    #[test]
    fn a_language_toggle_adds_and_removes_and_codes_are_exact() {
        let mut languages = set(&["en"]);
        assert!(toggle_language(&mut languages, "fr"));
        assert!(!toggle_language(&mut languages, "fr"));
        assert!(!toggle_language(&mut languages, "en"), "en is removed");
        assert!(toggle_language(&mut languages, "en"), "and added back");
        assert!(toggle_language(&mut languages, "EN") && languages.contains("EN"), "codes are not normalised");
    }

    #[test]
    fn toggling_a_language_twice_restores_the_original_set() {
        let original = set(&["all", "en", "ja"]);
        let mut languages = original.clone();
        toggle_language(&mut languages, "en");
        toggle_language(&mut languages, "en");
        assert_eq!(languages, original);
    }

    #[test]
    fn a_language_toggle_leaves_the_disabled_set_alone() {
        let disabled = set(&["3"]);
        let mut languages = set(&["en"]);
        toggle_language(&mut languages, "ja");
        toggle_language(&mut languages, "en");
        assert_eq!(disabled, set(&["3"]));
    }

    proptest! {
        /// Flipping an id twice restores the set it started from, whatever the set holds (T-17a-019).
        #[test]
        fn flipping_twice_restores_the_set(ids in proptest::collection::btree_set(any::<i64>(), 0..16), id in any::<i64>()) {
            let original: BTreeSet<String> = ids.iter().map(i64::to_string).collect();
            let mut working = original.clone();
            flip_disabled(&mut working, id);
            flip_disabled(&mut working, id);
            prop_assert_eq!(working, original);
        }

        /// A bulk set followed by its opposite restores the ids it touched, in one write each (T-17a-019).
        #[test]
        fn a_bulk_set_then_its_opposite_restores_the_members(ids in proptest::collection::vec(any::<i64>(), 0..16)) {
            let mut working = BTreeSet::new();
            set_disabled(&mut working, &ids, true);
            prop_assert!(ids.iter().all(|id| working.contains(&id.to_string())));
            set_disabled(&mut working, &ids, false);
            prop_assert!(working.is_empty());
        }
    }
}
