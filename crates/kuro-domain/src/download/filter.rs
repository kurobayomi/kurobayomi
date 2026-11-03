//! Auto-download of new items (`FilterChaptersForDownload.kt`, `FilterEpisodesForDownload.kt`). One algorithm serves
//! manga chapters and anime episodes through the [`NewItem`] and [`ReadState`] traits.

use std::collections::BTreeSet;

use kuro_data::item::Item;

/// `UNCATEGORIZED_ID`: the category of an entry that has no category. Used when an entry has none (E-DL-06).
pub const DEFAULT_CATEGORY_ID: i64 = 0;

/// An item that may be downloaded: its number, with a negative number meaning "not recognised".
pub trait NewItem {
    fn number(&self) -> f64;

    /// `chapterNumber >= 0` / `episodeNumber >= 0`.
    fn is_recognized(&self) -> bool {
        self.number() >= 0.0
    }
}

/// Whether an existing item counts as read: `done`, which is `read` for a chapter and `seen` for an episode.
pub trait ReadState {
    fn is_read(&self) -> bool;
}

impl NewItem for Item {
    fn number(&self) -> f64 {
        self.number
    }
}

impl ReadState for Item {
    fn is_read(&self) -> bool {
        self.done
    }
}

/// The include and exclude category sets of one kind of entry (`downloadNew…Categories` and `…Exclude`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CategoryDecision {
    pub included: BTreeSet<i64>,
    pub excluded: BTreeSet<i64>,
}

/// Behaviour 5.2: whether new items of an entry in `categories` may be queued. An entry with no category counts as
/// being in [`DEFAULT_CATEGORY_ID`]. Exclusion wins over inclusion.
pub fn should_download_new(favorite: bool, categories: &[i64], rule: &CategoryDecision) -> bool {
    if !favorite {
        return false;
    }
    let default = [DEFAULT_CATEGORY_ID];
    let cats: &[i64] = if categories.is_empty() { &default } else { categories };
    match (rule.included.is_empty(), rule.excluded.is_empty()) {
        (true, true) => true,
        _ if cats.iter().any(|c| rule.excluded.contains(c)) => false,
        _ if rule.included.is_empty() => true,
        _ => cats.iter().any(|c| rule.included.contains(c)),
    }
}

/// The key of a number in the "already read" set. `+ 0.0` turns `-0.0` into `0.0`, so the two compare equal.
fn number_key(n: f64) -> u64 {
    (n + 0.0).to_bits()
}

/// Behaviour 5.1: the new items to queue.
///
/// `enabled` is the `download_new` preference of the kind, and `eligible` is the result of [`should_download_new`]
/// for the entry. The caller resolves the categories first, so this function is pure. With `unread_only`, an item is
/// dropped when a read, recognised item has the same number (Q-DL-1: numbers, not ids, are compared).
pub fn filter_new<T, R>(enabled: bool, eligible: bool, unread_only: bool, new_items: &[T], existing: &[R]) -> Vec<T>
where
    T: NewItem + Clone,
    R: NewItem + ReadState,
{
    if new_items.is_empty() || !enabled || !eligible {
        return Vec::new();
    }
    if !unread_only {
        return new_items.to_vec();
    }
    let read: BTreeSet<u64> =
        existing.iter().filter(|e| e.is_read() && e.is_recognized()).map(|e| number_key(e.number())).collect();
    new_items.iter().filter(|n| !read.contains(&number_key(n.number()))).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use kuro_platform::kind::Kind;

    fn item(number: f64, done: bool) -> Item {
        Item { number, done, ..Item::create(Kind::Manga) }
    }

    fn set(ids: &[i64]) -> BTreeSet<i64> {
        ids.iter().copied().collect()
    }

    fn rule(included: &[i64], excluded: &[i64]) -> CategoryDecision {
        CategoryDecision { included: set(included), excluded: set(excluded) }
    }

    #[test]
    fn should_download_new_follows_the_five_rows_of_behaviour_5_2() {
        // (favorite, categories, included, excluded, expected)
        type Row<'a> = (bool, &'a [i64], &'a [i64], &'a [i64], bool);
        let table: [Row; 10] = [
            (false, &[1], &[], &[], false),
            (true, &[1], &[], &[], true),
            (true, &[], &[], &[], true),
            (true, &[3], &[], &[3], false),
            (true, &[3, 5], &[], &[5], false),
            (true, &[3], &[], &[5], true),
            (true, &[3], &[3], &[], true),
            (true, &[4], &[3], &[], false),
            (true, &[3, 4], &[3], &[], true),
            (true, &[3], &[3], &[3], false),
        ];
        for (favorite, cats, included, excluded, want) in table {
            assert_eq!(
                should_download_new(favorite, cats, &rule(included, excluded)),
                want,
                "favorite {favorite}, categories {cats:?}, include {included:?}, exclude {excluded:?}"
            );
        }
    }

    #[test]
    fn an_entry_with_no_categories_is_in_the_default_category() {
        assert!(should_download_new(true, &[], &rule(&[0], &[])), "E-DL-06: default id 0 is included");
        assert!(!should_download_new(true, &[], &rule(&[], &[0])), "E-DL-06: default id 0 is excluded");
        assert!(!should_download_new(true, &[], &rule(&[7], &[])));
    }

    #[test]
    fn the_default_category_zero_is_a_valid_set_member() {
        assert!(should_download_new(true, &[0], &rule(&[0], &[])), "E-DL-05");
    }

    #[test]
    fn filter_new_returns_nothing_when_disabled_empty_or_not_eligible() {
        let new = [item(1.0, false)];
        assert!(filter_new(false, true, false, &new, &[] as &[Item]).is_empty(), "TC-DL-03");
        assert!(filter_new(true, false, false, &new, &[] as &[Item]).is_empty(), "favorite or category check failed");
        assert!(filter_new(true, true, false, &[] as &[Item], &[] as &[Item]).is_empty());
    }

    #[test]
    fn without_unread_only_every_new_item_is_returned_unchanged() {
        let new = [item(1.0, false), item(2.0, false)];
        let existing = [item(1.0, true)];
        assert_eq!(filter_new(true, true, false, &new, &existing), new.to_vec());
    }

    #[test]
    fn a_read_item_with_the_same_number_suppresses_the_new_one_for_both_kinds() {
        let new = [item(5.0, false), item(6.0, false)];
        // TC-DL-04: read chapter 5 suppresses the new 5; an unread 5 does not.
        assert_eq!(filter_new(true, true, true, &new, &[item(5.0, true)]), vec![item(6.0, false)]);
        assert_eq!(filter_new(true, true, true, &new, &[item(5.0, false)]), new.to_vec());
    }

    #[test]
    fn unrecognised_numbers_never_suppress_anything() {
        let new = [item(-1.0, false), item(3.0, false)];
        // TC-DL-05: a read item with -1 is not in the set; a new -1 is kept.
        assert_eq!(filter_new(true, true, true, &new, &[item(-1.0, true)]), new.to_vec());
    }

    #[test]
    fn decimal_numbers_compare_by_exact_bits() {
        let new = [item(10.5, false), item(10.0, false)];
        // TC-DL-06: a read 10.5 suppresses only 10.5.
        assert_eq!(filter_new(true, true, true, &new, &[item(10.5, true)]), vec![item(10.0, false)]);
    }

    #[test]
    fn negative_zero_and_zero_are_the_same_number() {
        let new = [item(0.0, false)];
        assert!(filter_new(true, true, true, &new, &[item(-0.0, true)]).is_empty());
    }

    #[test]
    fn an_item_of_either_kind_uses_its_number_and_done_flag() {
        for kind in Kind::ALL {
            let it = Item { number: 4.0, done: true, ..Item::create(kind) };
            assert_eq!((it.number(), it.is_read(), it.is_recognized()), (4.0, true, true), "{kind}");
        }
        assert!(!Item { number: -1.0, ..Item::create(Kind::Anime) }.is_recognized());
    }
}
