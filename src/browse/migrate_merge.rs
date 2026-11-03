//! The item-state copy of a migration (behavior 15 step 2; T-40a-016), for both kinds: `merge_item_state` carries the fetch
//! date and the bookmark of the old item over to the new item with the same number (an anime item also its resume point
//! and filler mark), and marks the new items done up to the highest done number of the old ones. Pure: the engine reads
//! the rows and writes the result.

use kuro_data::item::{Item, ItemUpdate};
use kuro_domain::items::model::ItemExt;

/// The new items with the old state merged in. The done flag is never copied: it is derived from the old done point
/// (`<=` the highest done number). An item whose number is not recognised (below zero) is left as it is, and the first
/// old item with the same number supplies the state. The resume point and the filler mark are copied for anime only.
pub fn merge_item_state(prev: &[Item], new: &[Item]) -> Vec<Item> {
    let max_done = prev.iter().filter(|i| i.done).map(|i| i.number).reduce(f64::max);
    new.iter()
        .map(|i| {
            let mut merged = i.clone();
            if i.is_recognized_number() {
                if let Some(old) = prev.iter().find(|p| p.is_recognized_number() && p.number == i.number) {
                    merged.date_fetch = old.date_fetch;
                    merged.bookmark = old.bookmark;
                    if i.kind.is_anime() {
                        merged.progress = old.progress;
                        merged.fillermark = old.fillermark;
                    }
                }
                if max_done.is_some_and(|max| merged.number <= max) {
                    merged.done = true;
                }
            }
            merged
        })
        .collect()
}

/// The updates that carry the old state over: only the rows the merge changed.
pub fn item_updates(current: &[Item], merged: &[Item]) -> Vec<ItemUpdate> {
    let state = |i: &Item| (i.done, i.bookmark, i.fillermark, i.progress, i.date_fetch);
    current
        .iter()
        .zip(merged)
        .filter(|(before, after)| state(before) != state(after))
        .map(|(_, after)| ItemUpdate {
            done: Some(after.done),
            bookmark: Some(after.bookmark),
            fillermark: Some(after.fillermark),
            progress: Some(after.progress),
            date_fetch: Some(after.date_fetch),
            ..ItemUpdate::new(after.id)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use kuro_platform::kind::Kind;

    use super::*;

    fn item(kind: Kind, number: f64, done: bool) -> Item {
        Item { number, done, ..Item::create(kind) }
    }

    #[test]
    fn a_new_item_takes_the_fetch_date_and_bookmark_of_the_old_one() {
        for kind in [Kind::Manga, Kind::Anime] {
            let old = [Item { date_fetch: 500, bookmark: true, ..item(kind, 3.0, false) }];
            let merged = merge_item_state(&old, &[item(kind, 3.0, false)]);
            assert_eq!((merged[0].date_fetch, merged[0].bookmark), (500, true), "TC-40-020 {kind}");
        }
    }

    #[test]
    fn items_at_or_below_the_highest_done_number_are_marked_done() {
        for kind in [Kind::Manga, Kind::Anime] {
            let old = [item(kind, 3.0, true), item(kind, 5.0, false)];
            let merged = merge_item_state(&old, &[item(kind, 2.0, false), item(kind, 3.0, false), item(kind, 4.0, false)]);
            assert_eq!(merged.iter().map(|c| c.done).collect::<Vec<_>>(), [true, true, false], "TC-40-021, E-40-80: equal is done ({kind})");
        }
    }

    #[test]
    fn the_first_old_item_with_the_same_number_wins() {
        let old = [Item { date_fetch: 100, bookmark: true, ..item(Kind::Manga, 5.0, false) }, Item { date_fetch: 200, bookmark: false, ..item(Kind::Manga, 5.0, false) }];
        let merged = merge_item_state(&old, &[item(Kind::Manga, 5.0, false)]);
        assert_eq!((merged[0].date_fetch, merged[0].bookmark), (100, true), "TC-40-022, E-40-81");
    }

    #[test]
    fn an_unrecognised_number_is_left_untouched() {
        for kind in [Kind::Manga, Kind::Anime] {
            let old = [Item { date_fetch: 9, bookmark: true, ..item(kind, -1.0, true) }];
            let merged = merge_item_state(&old, &[Item { date_fetch: 1, ..item(kind, -1.0, false) }]);
            assert_eq!((merged[0].date_fetch, merged[0].bookmark, merged[0].done), (1, false, false), "TC-40-023: -1 is not matched and not marked ({kind})");
        }
    }

    #[test]
    fn the_done_flag_is_derived_and_never_copied_from_the_old_item() {
        let old = [item(Kind::Manga, 9.0, false)];
        let merged = merge_item_state(&old, &[item(Kind::Manga, 9.0, false)]);
        assert!(!merged[0].done, "TC-40-024: no done point in the old list means nothing is marked");
        let no_old = merge_item_state(&[], &[item(Kind::Manga, 2.0, false)]);
        assert_eq!((no_old[0].date_fetch, no_old[0].bookmark, no_old[0].done), (0, false, false), "E-40-77: no old items copy nothing");
    }

    #[test]
    fn the_resume_point_and_filler_mark_follow_the_match_for_anime_only() {
        let old = |kind| [Item { progress: 90, fillermark: true, ..item(kind, 1.0, false) }];
        let anime = merge_item_state(&old(Kind::Anime), &[item(Kind::Anime, 1.0, false)]);
        assert_eq!((anime[0].progress, anime[0].fillermark), (90, true), "E20: copied with the match");
        let manga = merge_item_state(&old(Kind::Manga), &[item(Kind::Manga, 1.0, false)]);
        assert_eq!((manga[0].progress, manga[0].fillermark), (0, false), "a manga page is not carried over");
    }

    #[test]
    fn only_changed_rows_are_written() {
        for kind in [Kind::Manga, Kind::Anime] {
            let chapter = |id: i64, number: f64, done: bool| Item { id, ..item(kind, number, done) };
            let current = [chapter(2, 2.0, false), chapter(3, 4.0, false)];
            let merged = merge_item_state(&[chapter(1, 3.0, true)], &current);
            let updates = item_updates(&current, &merged);
            assert_eq!(updates.len(), 1, "{kind}: item 2 is below the done point, item 4 is not");
            assert_eq!((updates[0].id, updates[0].done), (2, Some(true)));
        }
    }

    #[test]
    fn the_done_rule_and_the_matched_state_carry_over_from_the_old_anime_items() {
        let episode = |id: i64, number: f64, done: bool| Item { id, ..item(Kind::Anime, number, done) };
        let prev = [
            Item { bookmark: true, date_fetch: 500, progress: 90, fillermark: true, ..episode(1, 1.0, true) },
            episode(2, 2.0, false),
            episode(3, 5.0, true),
        ];
        let current = [episode(11, 1.0, false), episode(12, 2.0, false), episode(13, 4.0, false), episode(14, -1.0, false)];
        let updates = item_updates(&current, &merge_item_state(&prev, &current));
        let row = |id: i64| updates.iter().find(|u| u.id == id);
        let first = row(11).expect("matched row");
        assert_eq!((first.done, first.bookmark, first.date_fetch), (Some(true), Some(true), Some(500)), "matched by number, done by the highest done number");
        assert_eq!((first.progress, first.fillermark), (Some(90), Some(true)));
        assert_eq!(row(12).map(|u| u.done), Some(Some(true)), "2.0 is under the highest done 5.0");
        assert_eq!(row(13).map(|u| u.done), Some(Some(true)), "4.0 is under 5.0 even with no matching episode");
        assert!(row(14).is_none(), "unrecognised numbers are left unchanged");
    }
}
