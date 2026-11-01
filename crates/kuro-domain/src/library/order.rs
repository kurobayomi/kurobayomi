//! Library sort engine (`applySort`, behaviour 2.5). Pure: every page is sorted with the sort of its own category,
//! the Random sort shuffles with a seeded ChaCha8 generator, and alphabetical ties use the shared collator.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use rand::SeedableRng;
use rand::rngs::ChaCha8Rng;
use rand::seq::SliceRandom;

use super::model::LibraryRow;
use super::page::LibraryItem;
use kuro_platform::sort::{Direction, LibrarySort, SortType};
use crate::entries::flags::AIRING_TIME_MASK;
use crate::items::sorter::collate;

/// The tracker value of an entry with no usable score, and of a NaN mean.
pub const UNSCORED: f64 = -1.0;

/// Mean of the 10-point scores of one entry's tracks. No scores, or a NaN mean, count as [`UNSCORED`].
pub fn tracker_mean(scores: &[f64]) -> f64 {
    if scores.is_empty() {
        return UNSCORED;
    }
    let mean = scores.iter().sum::<f64>() / scores.len() as f64;
    if mean.is_nan() { UNSCORED } else { mean }
}

/// What the comparators read besides the rows.
#[derive(Clone, Copy, Debug)]
pub struct SortContext<'a> {
    /// The 10-point scores of each entry's tracks, by entry id. Missing entries are unscored.
    pub tracker_scores: &'a BTreeMap<i64, Vec<f64>>,
    /// `library_random_*_sort_seed` of the kind being sorted.
    pub random_seed: i32,
}

impl SortContext<'_> {
    fn tracker_value(&self, entry_id: i64) -> f64 {
        self.tracker_scores.get(&entry_id).map_or(UNSCORED, |s| tracker_mean(s))
    }
}

/// A fresh Random seed (`Random.nextInt()`), drawn when the Random sort is picked.
pub fn new_seed() -> i32 {
    rand::random::<i32>()
}

/// `shuffled(Random(seed))`: the same seed always gives the same order.
pub fn shuffle_with_seed<T>(items: &mut [T], seed: i32) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed as i64 as u64);
    items.shuffle(&mut rng);
}

/// Zero-last comparison used by the unread, unseen and airing sorts: equal values are equal, a zero sorts after
/// any non-zero value when ascending and before it when descending (the caller reverses the whole comparison
/// for descending sorts, so zeros end up last in both directions).
fn zero_last(a: i64, b: i64, ascending: bool) -> Ordering {
    if a == b {
        Ordering::Equal
    } else if a == 0 {
        if ascending { Ordering::Greater } else { Ordering::Less }
    } else if b == 0 {
        if ascending { Ordering::Less } else { Ordering::Greater }
    } else {
        a.cmp(&b)
    }
}

/// Airing time of an anime row: bits 24 to 55 of `viewer_flags`, epoch seconds (0 when none).
fn airing_at(row: &LibraryRow) -> i64 {
    (row.viewer_flags & AIRING_TIME_MASK) >> 24
}

/// The comparator of one type, before the direction is applied. `titles` holds the lowercase titles of the page,
/// computed once per item.
fn type_cmp(
    ty: SortType,
    (a, b): (usize, usize),
    items: &[LibraryItem],
    titles: &[String],
    ctx: &SortContext<'_>,
    asc: bool,
) -> Ordering {
    let (x, y) = (&items[a].row, &items[b].row);
    match ty {
        SortType::Alphabetical => collate(&titles[a], &titles[b]),
        SortType::LastActivity => x.last_activity.cmp(&y.last_activity),
        SortType::LastUpdate => x.last_update.cmp(&y.last_update),
        SortType::UnreadCount => zero_last(x.pending(), y.pending(), asc),
        SortType::TotalItems => x.total.cmp(&y.total),
        SortType::LatestItem => x.latest_upload.cmp(&y.latest_upload),
        SortType::ItemFetchDate => x.item_fetched_at.cmp(&y.item_fetched_at),
        SortType::DateAdded => x.date_added.cmp(&y.date_added),
        SortType::TrackerMean => ctx.tracker_value(x.id).total_cmp(&ctx.tracker_value(y.id)),
        SortType::AiringTime => x.pending().cmp(&y.pending()).then_with(|| zero_last(airing_at(x), airing_at(y), asc)),
        SortType::Random => Ordering::Equal,
    }
}

/// Sorts one page. Ties on the main key fall back to the lowercase title in ascending collation order, even when
/// the direction is descending. The Random sort shuffles instead of comparing.
pub fn sort_page(items: Vec<LibraryItem>, sort: LibrarySort, ctx: &SortContext<'_>) -> Vec<LibraryItem> {
    let mut items = items;
    if sort.ty == SortType::Random {
        shuffle_with_seed(&mut items, ctx.random_seed);
        return items;
    }
    let asc = sort.dir == Direction::Ascending;
    let titles: Vec<String> = items.iter().map(|i| i.row.title.to_lowercase()).collect();
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| {
        let main = type_cmp(sort.ty, (a, b), &items, &titles, ctx, asc);
        let main = if asc { main } else { main.reverse() };
        main.then_with(|| collate(&titles[a], &titles[b]))
    });
    let mut slots: Vec<Option<LibraryItem>> = items.into_iter().map(Some).collect();
    order.into_iter().map(|i| slots[i].take().expect("each index is used once")).collect()
}

/// What tapping a sort type in the settings sheet does: the new sort, and whether the Random seed is re-rolled.
/// Tapping the current type flips the direction, tapping another type keeps the direction, and Random always
/// becomes ascending with a new seed.
pub fn click(current: LibrarySort, tapped: SortType) -> (LibrarySort, bool) {
    if tapped == SortType::Random {
        return (LibrarySort { ty: SortType::Random, dir: Direction::Ascending }, true);
    }
    if tapped == current.ty {
        let dir = match current.dir {
            Direction::Ascending => Direction::Descending,
            Direction::Descending => Direction::Ascending,
        };
        (LibrarySort { ty: tapped, dir }, false)
    } else {
        (LibrarySort { ty: tapped, dir: current.dir }, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_platform::sort::SortType::*;

    fn item(id: i64, title: &str, total: i64, done: i64, last_read: i64, latest: i64) -> LibraryItem {
        LibraryItem {
            row: LibraryRow {
                id,
                source: 7,
                title: title.into(),
                category: 0,
                total,
                done,
                bookmarks: 0,
                fillermarks: 0,
                latest_upload: latest,
                item_fetched_at: 0,
                last_activity: last_read,
                last_update: 0,
                date_added: id,
                status: 1,
                fetch_interval: 0,
                viewer_flags: 0,
                always_update: true,
                next_update: 0,
            },
            download_count: 0,
            unread_count: 0,
            is_local: false,
            source_language: String::new(),
        }
    }

    /// The four-entry fixture of `p1-edge-cases-and-tests.md` 2.2 (ids 1, 2, 3, 4 with their titles).
    fn fixture() -> Vec<LibraryItem> {
        vec![
            item(1, "Alpha", 10, 10, 500, 100),
            item(2, "beta", 8, 2, 900, 300),
            item(3, "Gamma", 5, 0, 0, 200),
            item(4, "Émile", 3, 3, 100, 50),
        ]
    }

    fn ids(items: &[LibraryItem]) -> Vec<i64> {
        items.iter().map(|i| i.row.id).collect()
    }

    fn sorted(items: Vec<LibraryItem>, ty: SortType, dir: Direction) -> Vec<i64> {
        let scores = BTreeMap::new();
        let ctx = SortContext { tracker_scores: &scores, random_seed: 0 };
        ids(&sort_page(items, LibrarySort { ty, dir }, &ctx))
    }

    #[test]
    fn alphabetical_uses_collation_and_ties_stay_alphabetical_when_descending() {
        assert_eq!(sorted(fixture(), Alphabetical, Direction::Ascending), vec![1, 2, 4, 3]);
        assert_eq!(sorted(fixture(), Alphabetical, Direction::Descending), vec![3, 4, 2, 1]);
    }

    #[test]
    fn unread_count_places_zero_last_in_both_directions() {
        let page = vec![fixture()[0].clone(), fixture()[1].clone(), fixture()[3].clone()];
        assert_eq!(sorted(page.clone(), UnreadCount, Direction::Ascending), vec![2, 1, 4]);
        assert_eq!(sorted(page, UnreadCount, Direction::Descending), vec![2, 1, 4]);
        let hold = vec![fixture()[1].clone(), fixture()[2].clone()];
        assert_eq!(sorted(hold.clone(), UnreadCount, Direction::Ascending), vec![3, 2]);
        assert_eq!(sorted(hold, UnreadCount, Direction::Descending), vec![2, 3]);
    }

    #[test]
    fn last_read_orders_the_default_page_by_value() {
        let default_page = vec![fixture()[0].clone(), fixture()[1].clone(), fixture()[3].clone()];
        assert_eq!(sorted(default_page.clone(), LastActivity, Direction::Ascending), vec![4, 1, 2]);
        assert_eq!(sorted(default_page, LastActivity, Direction::Descending), vec![2, 1, 4]);
    }

    #[test]
    fn equal_keys_break_ties_by_title_even_when_descending() {
        let page = vec![item(5, "zeta", 1, 0, 7, 0), item(6, "Alpha", 1, 0, 7, 0)];
        assert_eq!(sorted(page, LastActivity, Direction::Descending), vec![6, 5]);
    }

    #[test]
    fn tracker_mean_uses_minus_one_for_unscored_and_nan() {
        assert_eq!(tracker_mean(&[]), UNSCORED);
        assert_eq!(tracker_mean(&[f64::NAN]), UNSCORED);
        assert_eq!(tracker_mean(&[8.0, 9.0]), 8.5);
        let scores = BTreeMap::from([(2, vec![8.5]), (3, vec![9.0]), (1, vec![f64::NAN])]);
        let ctx = SortContext { tracker_scores: &scores, random_seed: 0 };
        let order = ids(&sort_page(fixture(), LibrarySort { ty: TrackerMean, dir: Direction::Ascending }, &ctx));
        assert_eq!(order, vec![1, 4, 2, 3]);
    }

    #[test]
    fn random_is_seeded_and_a_permutation() {
        let long: Vec<LibraryItem> = (0..20).map(|i| item(i, &format!("t{i}"), 1, 0, 0, 0)).collect();
        let scores = BTreeMap::new();
        let a = sort_page(
            long.clone(),
            LibrarySort { ty: Random, dir: Direction::Ascending },
            &SortContext { tracker_scores: &scores, random_seed: 42 },
        );
        let b = sort_page(
            long.clone(),
            LibrarySort { ty: Random, dir: Direction::Ascending },
            &SortContext { tracker_scores: &scores, random_seed: 42 },
        );
        let c = sort_page(
            long.clone(),
            LibrarySort { ty: Random, dir: Direction::Ascending },
            &SortContext { tracker_scores: &scores, random_seed: 43 },
        );
        assert_eq!(ids(&a), ids(&b), "same seed, same order");
        assert_ne!(ids(&a), ids(&c), "new seed, new order");
        let mut back = ids(&a);
        back.sort_unstable();
        assert_eq!(back, (0..20).collect::<Vec<i64>>(), "a permutation");
    }

    #[test]
    fn airing_time_sorts_by_pending_then_airing_with_zero_last() {
        let mut a = item(1, "a", 5, 0, 0, 0);
        let mut b = item(2, "b", 5, 0, 0, 0);
        let mut c = item(3, "c", 5, 0, 0, 0);
        let mut d = item(4, "d", 9, 0, 0, 0);
        a.row.viewer_flags = 200 << 24;
        b.row.viewer_flags = 0;
        c.row.viewer_flags = 100 << 24;
        d.row.viewer_flags = 1 << 24;
        let page = vec![a, b, c, d];
        assert_eq!(sorted(page.clone(), AiringTime, Direction::Ascending), vec![3, 1, 2, 4]);
        assert_eq!(sorted(page, AiringTime, Direction::Descending), vec![4, 1, 3, 2]);
    }

    #[test]
    fn click_flips_the_current_type_and_keeps_direction_for_others() {
        let current = LibrarySort { ty: LastActivity, dir: Direction::Descending };
        assert_eq!(click(current, LastActivity), (LibrarySort { ty: LastActivity, dir: Direction::Ascending }, false));
        assert_eq!(click(current, DateAdded), (LibrarySort { ty: DateAdded, dir: Direction::Descending }, false));
        assert_eq!(click(current, Random), (LibrarySort { ty: Random, dir: Direction::Ascending }, true));
    }
}
