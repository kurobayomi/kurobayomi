//! Pager item list (T-44a-021, T-44a-022, T-44a-030): the shared flow of `items.rs`, the preprocessed dual-page
//! inserts of the current chapter (B§5.3 step 5, B§5.6), the right-to-left reversal (B§5.3 step 10), and the two-page
//! spreads of a wide window (`pair_spreads`).

use std::collections::BTreeMap;

use crate::reader::viewers::ViewerKind;
use crate::reader::viewers::items::build_flow;
use crate::reader::viewers::model::{ChapterSet, ItemKey, PageKind, PageRef, Transition};

/// The result of one build: the rows, the item to jump to when the last page had a split, and the `Next`
/// transition.
pub struct PagerList {
    pub items: Vec<ItemKey>,
    pub jump: Option<ItemKey>,
    pub next: Transition,
}

/// Builds the pager list (B§5.3). `preprocessed` holds the split pages of neighbour chapters, keyed by parent index;
/// it is consumed here (cleared whatever the current chapter is), as `preprocessed = {}` does upstream.
pub fn build_pager_items(set: &ChapterSet, force: bool, preprocessed: &mut BTreeMap<u32, PageRef>, kind: ViewerKind) -> PagerList {
    let pending = std::mem::take(preprocessed);
    let mut jump = None;
    let (mut items, next) = build_flow(set, force, |pages| {
        let Some(last_index) = pages.last().map(|p| p.index) else { return Vec::new() };
        let mut list = pages;
        for (&key, insert) in pending.iter().rev() {
            if key == last_index {
                jump = Some(ItemKey::Page(insert.clone()));
            }
            // A split for a page past the end cannot be placed; upstream would throw here.
            if (key as usize) < list.len() {
                list.insert(key as usize + 1, insert.clone());
            }
        }
        list.into_iter().map(ItemKey::Page).collect()
    });
    if kind == ViewerKind::RightToLeft {
        items.reverse();
    }
    PagerList { items, jump, next }
}

/// Pairs the pages of `items` (in the order the strip shows them) into two-page spreads, in reading order. Each
/// chapter pairs from its own start: its first page (the cover) stands alone, then pages go in twos, and a wide page
/// (`wide`: a spread already, wider than tall) stands alone and starts the count again after it. So a pair is the same
/// whichever part of the chapter the list holds. Transitions, split inserts and pages of different chapters never pair.
pub fn pair_spreads(items: Vec<ItemKey>, kind: ViewerKind, wide: &dyn Fn(&PageRef) -> bool) -> Vec<ItemKey> {
    let rtl = kind == ViewerKind::RightToLeft;
    let mut reading = items;
    if rtl {
        reading.reverse();
    }
    let mut out = Vec::with_capacity(reading.len());
    let mut rows = reading.into_iter().peekable();
    while let Some(item) = rows.next() {
        if let (ItemKey::Page(first), Some(ItemKey::Page(second))) = (&item, rows.peek())
            && pairs_with(first, second, wide)
        {
            let second = second.clone();
            rows.next();
            out.push(ItemKey::Spread(first.clone(), second));
            continue;
        }
        out.push(item);
    }
    if rtl {
        out.reverse();
    }
    out
}

/// Whether `second` goes beside `first` (see [`pair_spreads`]).
fn pairs_with(first: &PageRef, second: &PageRef, wide: &dyn Fn(&PageRef) -> bool) -> bool {
    if first.is_insert() || second.is_insert() || first.chapter != second.chapter || second.index != first.index + 1 {
        return false;
    }
    if first.index == 0 || wide(first) || wide(second) {
        return false;
    }
    // the count restarts after the cover and after every wide page before `first`
    let page = |index| PageRef { chapter: first.chapter.clone(), index, kind: PageKind::Normal };
    let restart = (1..first.index).rev().find(|&index| wide(&page(index))).unwrap_or(0) + 1;
    (first.index - restart).is_multiple_of(2)
}

/// Places a dual-page split next to the page at `current` (B§5.6). Returns `false` when the insert would repeat
/// (R2L guard), would sit before an existing insert, or has no slot.
pub fn place_insert(items: &mut Vec<ItemKey>, current: usize, kind: ViewerKind, insert: PageRef) -> bool {
    let place = if kind == ViewerKind::RightToLeft { current } else { current + 1 };
    if kind == ViewerKind::RightToLeft && place >= 1 && is_insert(items.get(place - 1)) {
        return false;
    }
    if place >= items.len() || is_insert(items.get(place)) {
        return false;
    }
    items.insert(place, ItemKey::Page(insert));
    true
}

fn is_insert(item: Option<&ItemKey>) -> bool {
    matches!(item, Some(ItemKey::Page(p)) if p.is_insert())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::viewers::items::tests::{label, snap};
    use crate::reader::viewers::model::{ChapterSnap, LoadState, PageKind};

    fn set(curr: ChapterSnap, prev: Option<ChapterSnap>, next: Option<ChapterSnap>) -> ChapterSet {
        ChapterSet { curr, prev, next }
    }

    fn loaded(id: i64, number: f64, pages: u32) -> ChapterSnap {
        snap(id, number, LoadState::Loaded, pages)
    }

    #[test]
    fn default_settings_give_the_observed_golden_list() {
        // Prev tail, Prev transition, current pages, Next transition, Next head (B§5.3).
        let s = set(loaded(2, 2.0, 3), Some(loaded(1, 1.0, 5)), Some(loaded(3, 3.0, 4)));
        let list = build_pager_items(&s, true, &mut BTreeMap::new(), ViewerKind::LeftToRight);
        assert_eq!(label(&list.items), ["p1:3", "p1:4", "P(2,1)", "p2:0", "p2:1", "p2:2", "N(2,3)", "p3:0", "p3:1"]);
        assert!(list.jump.is_none());
        assert_eq!(list.next.to.as_ref().map(|k| k.id), Some(3));
    }

    #[test]
    fn the_end_of_the_book_keeps_a_terminal_next_transition() {
        let s = set(loaded(2, 2.0, 2), Some(loaded(1, 1.0, 2)), None);
        let list = build_pager_items(&s, true, &mut BTreeMap::new(), ViewerKind::LeftToRight);
        assert_eq!(label(&list.items).last().map(String::as_str), Some("N(2,-)"));
    }

    #[test]
    fn right_to_left_reverses_the_whole_list() {
        let s = set(loaded(2, 2.0, 2), None, Some(loaded(3, 3.0, 2)));
        let ltr = build_pager_items(&s, true, &mut BTreeMap::new(), ViewerKind::LeftToRight);
        let rtl = build_pager_items(&s, true, &mut BTreeMap::new(), ViewerKind::RightToLeft);
        let mut back = rtl.items.clone();
        back.reverse();
        assert_eq!(back, ltr.items);
        assert_eq!(label(&rtl.items)[0], "p3:1");
    }

    #[test]
    fn an_empty_loaded_chapter_does_not_panic_and_still_has_transitions() {
        let s = set(loaded(2, 2.0, 0), None, None);
        let mut pre = BTreeMap::new();
        pre.insert(0, PageRef::insert_of(&loaded(2, 2.0, 1).page(0)));
        let list = build_pager_items(&s, true, &mut pre, ViewerKind::LeftToRight);
        assert_eq!(label(&list.items), ["P(2,-)", "N(2,-)"]);
        assert!(pre.is_empty());
    }

    #[test]
    fn a_split_of_the_last_page_places_the_insert_after_it_and_asks_for_a_jump() {
        let curr = loaded(2, 2.0, 3);
        let s = set(curr.clone(), None, Some(loaded(3, 3.0, 1)));
        let mut pre = BTreeMap::new();
        pre.insert(2, PageRef::insert_of(&curr.page(2)));
        let list = build_pager_items(&s, false, &mut pre, ViewerKind::LeftToRight);
        assert_eq!(label(&list.items), ["P(2,-)", "p2:0", "p2:1", "p2:2", "i2:2", "p3:0"]);
        assert!(matches!(&list.jump, Some(ItemKey::Page(p)) if p.kind == PageKind::Insert { parent_index: 2 }));
    }

    #[test]
    fn a_split_past_the_end_is_dropped_instead_of_panicking() {
        let curr = loaded(2, 2.0, 2);
        let mut pre = BTreeMap::new();
        pre.insert(7, PageRef::insert_of(&curr.page(7)));
        let list = build_pager_items(&set(curr, None, None), true, &mut pre, ViewerKind::LeftToRight);
        assert_eq!(label(&list.items), ["P(2,-)", "p2:0", "p2:1", "N(2,-)"]);
    }

    #[test]
    fn insert_placement_follows_the_direction() {
        let base = vec![ItemKey::Page(loaded(2, 2.0, 3).page(0)), ItemKey::Page(loaded(2, 2.0, 3).page(1))];
        let insert = PageRef::insert_of(&loaded(2, 2.0, 3).page(0));
        let mut ltr = base.clone();
        assert!(place_insert(&mut ltr, 0, ViewerKind::LeftToRight, insert.clone()));
        assert_eq!(label(&ltr), ["p2:0", "i2:0", "p2:1"]);
        let mut rtl = base.clone();
        assert!(place_insert(&mut rtl, 1, ViewerKind::RightToLeft, insert.clone()));
        assert_eq!(label(&rtl), ["p2:0", "i2:0", "p2:1"]);
    }

    #[test]
    fn right_to_left_refuses_an_insert_right_after_another_insert() {
        let curr = loaded(2, 2.0, 3);
        let insert = PageRef::insert_of(&curr.page(1));
        let mut items = vec![ItemKey::Page(curr.page(0)), ItemKey::Page(insert.clone()), ItemKey::Page(curr.page(1))];
        assert!(!place_insert(&mut items, 2, ViewerKind::RightToLeft, insert));
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn no_insert_goes_next_to_an_existing_insert_or_past_the_end() {
        let curr = loaded(2, 2.0, 2);
        let insert = PageRef::insert_of(&curr.page(0));
        let mut items = vec![ItemKey::Page(curr.page(0)), ItemKey::Page(insert.clone())];
        assert!(!place_insert(&mut items, 0, ViewerKind::LeftToRight, insert.clone()));
        let mut at_end = vec![ItemKey::Page(curr.page(0))];
        assert!(!place_insert(&mut at_end, 0, ViewerKind::LeftToRight, insert));
        assert_eq!(at_end.len(), 1);
    }

    fn pages(chapter: i64, count: u32) -> Vec<ItemKey> {
        (0..count).map(|i| ItemKey::Page(loaded(chapter, chapter as f64, count).page(i))).collect()
    }

    #[test]
    fn the_cover_stands_alone_and_the_pages_after_it_go_in_twos() {
        let paired = pair_spreads(pages(2, 6), ViewerKind::LeftToRight, &|_| false);
        assert_eq!(label(&paired), ["p2:0", "s2:1-2", "s2:3-4", "p2:5"]);
    }

    #[test]
    fn a_wide_page_stands_alone_and_the_count_starts_again_after_it() {
        let wide = |p: &PageRef| p.index == 2;
        let paired = pair_spreads(pages(2, 7), ViewerKind::LeftToRight, &wide);
        assert_eq!(label(&paired), ["p2:0", "p2:1", "p2:2", "s2:3-4", "s2:5-6"]);
    }

    #[test]
    fn a_list_holding_part_of_a_chapter_keeps_the_pairs_of_the_whole_chapter() {
        // the tail of a 5-page chapter from page index 2: 2 belongs with 1, which the list does not hold
        let tail: Vec<ItemKey> = pages(1, 5).into_iter().skip(2).collect();
        assert_eq!(label(&pair_spreads(tail, ViewerKind::LeftToRight, &|_| false)), ["p1:2", "s1:3-4"]);
    }

    #[test]
    fn transitions_inserts_and_chapter_edges_never_pair() {
        let s = set(loaded(2, 2.0, 3), Some(loaded(1, 1.0, 5)), Some(loaded(3, 3.0, 4)));
        let list = build_pager_items(&s, true, &mut BTreeMap::new(), ViewerKind::LeftToRight);
        let paired = pair_spreads(list.items, ViewerKind::LeftToRight, &|_| false);
        assert_eq!(label(&paired), ["s1:3-4", "P(2,1)", "p2:0", "s2:1-2", "N(2,3)", "p3:0", "p3:1"]);
        let insert = ItemKey::Page(PageRef::insert_of(&loaded(2, 2.0, 3).page(1)));
        let with_insert = vec![pages(2, 3)[1].clone(), insert, pages(2, 3)[2].clone()];
        assert_eq!(label(&pair_spreads(with_insert, ViewerKind::LeftToRight, &|_| false)), ["p2:1", "i2:1", "p2:2"]);
    }

    #[test]
    fn right_to_left_pairs_in_reading_order_and_keeps_the_strip_reversed() {
        let mut rtl = pages(2, 5);
        rtl.reverse();
        let paired = pair_spreads(rtl, ViewerKind::RightToLeft, &|_| false);
        assert_eq!(label(&paired), ["s2:3-4", "s2:1-2", "p2:0"]);
    }

    /// One golden scenario: the labels of the list a set builds, and the item to jump to when the last page was split.
    fn scenario(about: &str, kind: ViewerKind, force: bool, set: &ChapterSet, pre: &[PageRef]) -> serde_json::Value {
        let mut preprocessed: BTreeMap<u32, PageRef> = pre.iter().map(|p| (p.index, p.clone())).collect();
        let list = build_pager_items(set, force, &mut preprocessed, kind);
        let jump = list.jump.as_ref().map(|j| label(std::slice::from_ref(j)).remove(0));
        serde_json::json!({ "about": about, "viewer": format!("{kind:?}"), "force": force, "items": label(&list.items), "jump": jump })
    }

    /// T-44a-079: the item lists of the pager scenarios (the pager topic, section 8), compared with the golden files.
    #[test]
    fn the_golden_pager_item_lists_match() {
        use crate::reader::golden::check;
        let ltr = ViewerKind::LeftToRight;
        let both = set(loaded(2, 2.0, 3), Some(loaded(1, 1.0, 5)), Some(loaded(3, 3.0, 4)));
        check("pager_prev_next_present.json", &scenario("previous and next loaded, transitions forced", ltr, true, &both, &[]));
        let no_prev = set(loaded(2, 2.0, 3), None, Some(loaded(3, 3.0, 4)));
        check("pager_prev_missing.json", &scenario("no previous chapter: the Prev transition points at nothing", ltr, true, &no_prev, &[]));
        let no_next = set(loaded(2, 2.0, 3), Some(loaded(1, 1.0, 5)), None);
        check("pager_next_missing.json", &scenario("no next chapter", ltr, true, &no_next, &[]));
        let alone = set(loaded(2, 2.0, 2), None, None);
        check("pager_end_of_book.json", &scenario("one chapter, both ends of the book", ltr, false, &alone, &[]));
        let adjacent = set(loaded(2, 2.0, 2), Some(loaded(1, 1.0, 2)), Some(loaded(3, 3.0, 2)));
        check(
            "pager_no_forced_transitions.json",
            &scenario("loaded neighbours with no gap and always-show off: no transitions", ltr, false, &adjacent, &[]),
        );
        let gap = set(loaded(2, 5.0, 2), Some(loaded(1, 2.0, 2)), Some(loaded(3, 9.0, 2)));
        check("pager_gap_forces_transitions.json", &scenario("a chapter gap forces the transition", ltr, false, &gap, &[]));
        check("pager_r2l.json", &scenario("right to left reverses the whole list", ViewerKind::RightToLeft, true, &both, &[]));
        let curr = loaded(2, 2.0, 3);
        let split = set(curr.clone(), None, Some(loaded(3, 3.0, 1)));
        let inserts = [PageRef::insert_of(&curr.page(0)), PageRef::insert_of(&curr.page(2))];
        check("pager_insert_placement.json", &scenario("preprocessed inserts, the last page split", ltr, false, &split, &inserts));
        check("pager_insert_placement_r2l.json", &scenario("the same inserts, right to left", ViewerKind::RightToLeft, false, &split, &inserts));
    }
}
