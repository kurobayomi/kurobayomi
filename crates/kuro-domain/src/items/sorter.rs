//! Item and season comparators (`ChapterSorter.kt`, `EpisodeSorter.kt`, `SeasonSorter.kt`) and the collator used for
//! alphabetical order (`SortUtil.kt`).
//!
//! One comparator serves both kinds: the sort value is the unified `item_flags` group (`kuro_db::flags::item`).
//!
//! Numbers compare with `f64::total_cmp` (the same total order as `Double.compare`). Sorts are stable.
//! An unknown sorting value falls back to SOURCE with a warning; the Kotlin code throws there.

use std::cmp::Ordering;
use std::sync::LazyLock;

use crate::entries::flags::SeasonFlags;
use icu_collator::options::{CollatorOptions, Strength};
use icu_collator::{Collator, CollatorBorrowed, CollatorPreferences};
use icu_locale::Locale;
use kuro_data::entry::{Entry, SeasonEntry};
use kuro_data::item::Item;
use kuro_db::flags::{item, season as season_flags};

/// The four item sort keys of both kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Source,
    Number,
    UploadDate,
    Alphabet,
}

/// The fields the item comparator reads.
pub trait SortItem {
    fn source_order(&self) -> i64;
    fn number(&self) -> f64;
    fn date_upload(&self) -> i64;
    fn name(&self) -> &str;
}

impl SortItem for Item {
    fn source_order(&self) -> i64 {
        self.source_order
    }
    fn number(&self) -> f64 {
        self.number
    }
    fn date_upload(&self) -> i64 {
        self.date_upload
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// The user's locale for collation: `LC_COLLATE`, then `LANG`, converted to a BCP 47 tag. Anything that
/// does not parse (including `C` and `POSIX`) is the root locale.
fn system_locale() -> Locale {
    let raw = ["LC_COLLATE", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default();
    let base = raw
        .split(['.', '@'])
        .next()
        .unwrap_or_default()
        .replace('_', "-");
    Locale::try_from_str(&base).unwrap_or(Locale::UNKNOWN)
}

/// The primary-strength collator, built once. `a`, `A` and `á` compare equal; digits compare one character
/// at a time, so `Chapter 10` sorts before `Chapter 2`.
static COLLATOR: LazyLock<CollatorBorrowed<'static>> = LazyLock::new(|| {
    let mut options = CollatorOptions::default();
    options.strength = Some(Strength::Primary);
    let prefs = CollatorPreferences::from(&system_locale());
    // The compiled data covers every locale, so the root collator is the fallback when the lookup fails.
    Collator::try_new(prefs, options)
        .or_else(|_| Collator::try_new(CollatorPreferences::default(), options))
        .expect("compiled collation data is built in")
});

/// `compareToWithCollator`.
pub fn collate(a: &str, b: &str) -> Ordering {
    COLLATOR.compare(a, b)
}

/// The key of a sorting value (`item_flags & SORTING_MASK`, unified layout). Unknown values are `Source`.
pub fn item_key(sorting: i64) -> SortKey {
    match sorting {
        item::SORTING_SOURCE => SortKey::Source,
        item::SORTING_NUMBER => SortKey::Number,
        item::SORTING_UPLOAD_DATE => SortKey::UploadDate,
        item::SORTING_ALPHABET => SortKey::Alphabet,
        other => unknown_key(other),
    }
}

fn unknown_key(sorting: i64) -> SortKey {
    tracing::warn!(sorting, "unknown item sorting, using source order");
    SortKey::Source
}

/// Compares two items by `key`. Number, upload date and name are ascending when not descending. Source
/// order is the inverse: a source lists newest first, so descending means ascending `source_order`.
pub fn compare_items<T: SortItem>(key: SortKey, descending: bool, a: &T, b: &T) -> Ordering {
    let ascending = match key {
        SortKey::Source => a.source_order().cmp(&b.source_order()),
        SortKey::Number => a.number().total_cmp(&b.number()),
        SortKey::UploadDate => a.date_upload().cmp(&b.date_upload()),
        SortKey::Alphabet => collate(a.name(), b.name()),
    };
    let reversed = match key {
        SortKey::Source => !descending,
        _ => descending,
    };
    if reversed {
        ascending.reverse()
    } else {
        ascending
    }
}

/// The comparator of an entry's item list: `sorting` is `Entry::sorting()` (unified layout).
pub fn item_cmp(sorting: i64, descending: bool) -> impl Fn(&Item, &Item) -> Ordering {
    let key = item_key(sorting);
    move |a, b| compare_items(key, descending, a, b)
}

/// The fields the season comparator reads.
pub trait SortSeason {
    fn season_number(&self) -> f64;
    fn season_source_order(&self) -> i64;
    fn title(&self) -> &str;
    fn latest_upload(&self) -> i64;
    fn pending(&self) -> i64;
    fn last_seen(&self) -> i64;
    fn fetched_at(&self) -> i64;
}

impl SortSeason for SeasonEntry {
    fn season_number(&self) -> f64 {
        self.entry.season_number()
    }
    fn season_source_order(&self) -> i64 {
        self.entry.season_source_order()
    }
    fn title(&self) -> &str {
        &self.entry.title
    }
    fn latest_upload(&self) -> i64 {
        self.latest_upload
    }
    fn pending(&self) -> i64 {
        SeasonEntry::pending(self)
    }
    fn last_seen(&self) -> i64 {
        self.last_activity
    }
    fn fetched_at(&self) -> i64 {
        self.fetched_at
    }
}

/// Ascending by the season sort code (`season_flags & SORT_MASK`). Unknown codes (0xE000) fall back to SOURCE with a
/// warning.
fn season_cmp_by<T: SortSeason>(sorting: i64) -> impl Fn(&T, &T) -> Ordering {
    move |a: &T, b: &T| match sorting {
        season_flags::SORT_SEASON => a.season_number().total_cmp(&b.season_number()),
        season_flags::SORT_UPLOAD => a.latest_upload().cmp(&b.latest_upload()),
        season_flags::SORT_ALPHABET => collate(&a.title().to_lowercase(), &b.title().to_lowercase()),
        season_flags::SORT_COUNT => a.pending().cmp(&b.pending()),
        season_flags::SORT_LAST_SEEN => a.last_seen().cmp(&b.last_seen()),
        season_flags::SORT_FETCHED => a.fetched_at().cmp(&b.fetched_at()),
        season_flags::SORT_SOURCE => a.season_source_order().cmp(&b.season_source_order()),
        other => {
            tracing::warn!(sorting = other, "unknown season sorting, using source order");
            a.season_source_order().cmp(&b.season_source_order())
        }
    }
}

/// `getSeasonSortComparator(anime)`: ascending by the season sort code of the parent.
pub fn season_cmp<T: SortSeason>(parent: &Entry) -> impl Fn(&T, &T) -> Ordering {
    season_cmp_by(SeasonFlags(parent.season_flags()).sorting())
}

/// `sort_seasons`: the key comparator, reversed when the season sort is descending, then the alphabetical title as a
/// tie-break that is never reversed.
pub fn sort_seasons<T: SortSeason>(list: &mut [T], parent: &Entry) {
    let flags = SeasonFlags(parent.season_flags());
    let by_key = season_cmp_by::<T>(flags.sorting());
    let descending = flags.sort_descending();
    list.sort_by(|a, b| {
        let ord = by_key(a, b);
        let ord = if descending { ord.reverse() } else { ord };
        ord.then_with(|| collate(&a.title().to_lowercase(), &b.title().to_lowercase()))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_platform::kind::Kind;

    fn chapter(name: &str, source_order: i64, number: f64, upload: i64) -> Item {
        Item { name: name.into(), source_order, number, date_upload: upload, ..Item::create(Kind::Manga) }
    }

    fn names<T: SortItem>(items: &[T]) -> Vec<String> {
        items.iter().map(|i| i.name().to_owned()).collect()
    }

    fn sorted(mut items: Vec<Item>, sorting: i64, descending: bool) -> Vec<String> {
        items.sort_by(item_cmp(sorting, descending));
        names(&items)
    }

    #[test]
    fn source_row_is_inverted_against_the_other_keys() {
        let items = vec![chapter("s0", 0, 0.0, 0), chapter("s1", 1, 0.0, 0), chapter("s2", 2, 0.0, 0)];
        assert_eq!(sorted(items.clone(), item::SORTING_SOURCE, true), ["s0", "s1", "s2"]);
        assert_eq!(sorted(items, item::SORTING_SOURCE, false), ["s2", "s1", "s0"]);
    }

    #[test]
    fn numbers_use_total_order_with_unrecognised_first_ascending() {
        let items = vec![chapter("five", 0, 5.0, 0), chapter("unknown", 1, -1.0, 0), chapter("zero", 2, 0.0, 0), chapter("two-and-half", 3, 2.5, 0)];
        assert_eq!(sorted(items.clone(), item::SORTING_NUMBER, false), ["unknown", "zero", "two-and-half", "five"]);
        assert_eq!(sorted(items, item::SORTING_NUMBER, true), ["five", "two-and-half", "zero", "unknown"]);
        assert_eq!((-0.0f64).total_cmp(&0.0), Ordering::Less);
    }

    #[test]
    fn upload_date_follows_the_same_direction_rule() {
        let items = vec![chapter("old", 0, 0.0, 10), chapter("new", 1, 0.0, 20)];
        assert_eq!(sorted(items.clone(), item::SORTING_UPLOAD_DATE, true), ["new", "old"]);
        assert_eq!(sorted(items, item::SORTING_UPLOAD_DATE, false), ["old", "new"]);
    }

    #[test]
    fn sorts_are_stable_for_equal_keys() {
        let items = vec![chapter("first", 0, 3.0, 0), chapter("second", 1, 3.0, 0), chapter("third", 2, 3.0, 0)];
        assert_eq!(sorted(items.clone(), item::SORTING_NUMBER, false), ["first", "second", "third"]);
        assert_eq!(sorted(items, item::SORTING_NUMBER, true), ["first", "second", "third"]);
    }

    #[test]
    fn collator_is_primary_strength_and_not_natural_order() {
        assert_eq!(collate("a", "A"), Ordering::Equal);
        assert_eq!(collate("á", "a"), Ordering::Equal);
        assert_eq!(collate("Chapter 10", "Chapter 2"), Ordering::Less);
        assert_eq!(collate("b", "A"), Ordering::Greater);
        assert_eq!(collate("alpha", "Beta"), Ordering::Less, "T-17a-045: alphabetical, not by code point");
    }

    #[test]
    fn primary_equal_names_keep_input_order() {
        let items = vec![chapter("Chapter 2A", 0, 0.0, 0), chapter("chapter 2a", 1, 0.0, 0)];
        assert_eq!(sorted(items, item::SORTING_ALPHABET, false), ["Chapter 2A", "chapter 2a"]);
    }

    #[test]
    fn descending_false_is_playback_order_for_every_sorting() {
        // Source: the highest source_order is the oldest item, so ascending playback lists it first.
        let items = vec![chapter("n", 0, 2.0, 30), chapter("o", 1, 1.0, 10), chapter("p", 2, 3.0, 20)];
        assert_eq!(sorted(items.clone(), item::SORTING_SOURCE, false), ["p", "o", "n"]);
        assert_eq!(sorted(items.clone(), item::SORTING_NUMBER, false), ["o", "n", "p"]);
        assert_eq!(sorted(items.clone(), item::SORTING_UPLOAD_DATE, false), ["o", "p", "n"]);
        assert_eq!(sorted(items, item::SORTING_ALPHABET, false), ["n", "o", "p"]);
    }

    #[test]
    fn unknown_sorting_falls_back_to_source_without_panicking() {
        let items = vec![chapter("s0", 0, 9.0, 0), chapter("s1", 1, 1.0, 0)];
        assert_eq!(item_key(0x800), SortKey::Source);
        assert_eq!(sorted(items.clone(), 0x800, true), sorted(items, item::SORTING_SOURCE, true));
    }

    #[test]
    fn the_keys_of_the_four_sortings() {
        assert_eq!(item_key(item::SORTING_NUMBER), SortKey::Number);
        assert_eq!(item_key(item::SORTING_ALPHABET), SortKey::Alphabet);
        assert_eq!(item_key(item::SORTING_UPLOAD_DATE), SortKey::UploadDate);
    }

    fn season(title: &str, source_order: i64, number: f64) -> SeasonEntry {
        SeasonEntry {
            entry: Entry {
                title: title.into(),
                season_source_order: Some(source_order),
                season_number: Some(number),
                ..Entry::create(Kind::Anime)
            },
            total: 1,
            done: 0,
            bookmarks: 0,
            fillermarks: 0,
            latest_upload: 0,
            fetched_at: 0,
            last_activity: 0,
        }
    }

    fn titles(list: &[SeasonEntry]) -> Vec<String> {
        list.iter().map(|s| s.entry.title.clone()).collect()
    }

    fn parent(flags: i64) -> Entry {
        Entry { season_flags: Some(flags), ..Entry::create(Kind::Anime) }
    }

    #[test]
    fn default_word_zero_lists_highest_source_order_first() {
        let mut list = vec![season("a", 0, 1.0), season("b", 1, 2.0), season("c", 2, 3.0)];
        sort_seasons(&mut list, &parent(0));
        assert_eq!(titles(&list), ["c", "b", "a"]);
    }

    #[test]
    fn ascending_flag_reverses_back_to_source_order() {
        let mut list = vec![season("a", 0, 1.0), season("b", 1, 2.0), season("c", 2, 3.0)];
        sort_seasons(&mut list, &parent(season_flags::SORT_ASC));
        assert_eq!(titles(&list), ["a", "b", "c"]);
    }

    #[test]
    fn alphabetical_tie_break_is_not_reversed() {
        let tied = || vec![season("Zeta", 0, 2.0), season("Alpha", 1, 2.0)];
        let mut asc = tied();
        sort_seasons(&mut asc, &parent(season_flags::SORT_ASC | season_flags::SORT_SEASON));
        assert_eq!(titles(&asc), ["Alpha", "Zeta"]);
        let mut desc = tied();
        sort_seasons(&mut desc, &parent(season_flags::SORT_SEASON));
        assert_eq!(titles(&desc), ["Alpha", "Zeta"], "descending reverses the key, not the name tie-break");
    }

    #[test]
    fn code_0xe000_falls_back_to_source() {
        let items = vec![season("a", 0, 1.0), season("b", 1, 2.0)];
        let flagged = parent(0xE000);
        let cmp = season_cmp(&flagged);
        let mut sorted_items = items.clone();
        sorted_items.sort_by(&cmp);
        assert_eq!(titles(&sorted_items), ["a", "b"]);
    }
}
