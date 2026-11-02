//! The list shape both viewers share (B§5.3 steps 1-7 and 8, B§5.5 steps 1-7): the previous chapter's last two
//! pages, the Prev transition gate, the current chapter's pages, the Next transition gate and the next chapter's
//! first two pages. The viewer-specific parts (insert pages, reversal) are added by the pager builder.

use super::gap::calculate_chapter_gap;
use super::model::{ChapterSet, ChapterSnap, Dir, ItemKey, PageRef, Transition, transition_wanted};

/// Builds the shared list. `body` turns the current chapter's pages (empty unless it is loaded) into its rows.
/// Returns the rows and the `Next` transition, which the viewer keeps even when it is not in the list (it is the
/// preload destination).
pub fn build_flow(
    set: &ChapterSet,
    force: bool,
    body: impl FnOnce(Vec<PageRef>) -> Vec<ItemKey>,
) -> (Vec<ItemKey>, Transition) {
    let curr = &set.curr;
    let prev_gap = set.prev.as_ref().is_some_and(|p| calculate_chapter_gap(Some(curr.number), Some(p.number)) > 0);
    let next_gap = set.next.as_ref().is_some_and(|n| calculate_chapter_gap(Some(n.number), Some(curr.number)) > 0);

    let mut items = Vec::new();
    if let Some(prev) = set.prev.as_ref().filter(|p| p.is_loaded()) {
        let pages = prev.pages();
        let from = pages.len().saturating_sub(2);
        items.extend(pages[from..].iter().cloned().map(ItemKey::Page));
    }
    if transition_wanted(prev_gap, force, set.prev.as_ref()) {
        items.push(ItemKey::Transition(Transition { dir: Dir::Prev, from: curr.key.clone(), to: set.prev.as_ref().map(|p| p.key.clone()) }));
    }
    items.extend(body(curr.pages()));

    let next_transition = Transition { dir: Dir::Next, from: curr.key.clone(), to: set.next.as_ref().map(|n| n.key.clone()) };
    if transition_wanted(next_gap, force, set.next.as_ref()) {
        items.push(ItemKey::Transition(next_transition.clone()));
    }
    if let Some(next) = set.next.as_ref().filter(|n| n.is_loaded()) {
        items.extend(head(next, 2).into_iter().map(ItemKey::Page));
    }
    (items, next_transition)
}

/// The first `n` pages of a loaded chapter.
fn head(chapter: &ChapterSnap, n: u32) -> Vec<PageRef> {
    (0..n.min(chapter.page_count)).map(|i| chapter.page(i)).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::reader::viewers::model::{ChapterKey, LoadState};

    pub fn snap(id: i64, number: f64, state: LoadState, pages: u32) -> ChapterSnap {
        ChapterSnap { key: ChapterKey { id, url: format!("c{id}") }, number, state, page_count: pages, requested_page: 0 }
    }

    /// Renders a list as compact labels: `p<chapter>:<index>`, `P(<from>,<to>)` for Prev, `N(...)` for Next.
    pub fn label(items: &[ItemKey]) -> Vec<String> {
        items
            .iter()
            .map(|i| match i {
                ItemKey::Page(p) if p.is_insert() => format!("i{}:{}", p.chapter.id, p.index),
                ItemKey::Page(p) => format!("p{}:{}", p.chapter.id, p.index),
                ItemKey::Spread(a, b) => format!("s{}:{}-{}", a.chapter.id, a.index, b.index),
                ItemKey::Transition(t) => {
                    let to = t.to.as_ref().map_or("-".to_string(), |k| k.id.to_string());
                    let dir = if t.dir == Dir::Prev { "P" } else { "N" };
                    format!("{dir}({},{to})", t.from.id)
                }
            })
            .collect()
    }

    fn body(pages: Vec<PageRef>) -> Vec<ItemKey> {
        pages.into_iter().map(ItemKey::Page).collect()
    }

    #[test]
    fn all_loaded_neighbours_give_tails_and_heads_with_transitions_in_between() {
        let set = ChapterSet {
            curr: snap(2, 2.0, LoadState::Loaded, 3),
            prev: Some(snap(1, 1.0, LoadState::Loaded, 5)),
            next: Some(snap(3, 3.0, LoadState::Loaded, 4)),
        };
        let (items, next) = build_flow(&set, true, body);
        assert_eq!(label(&items), ["p1:3", "p1:4", "P(2,1)", "p2:0", "p2:1", "p2:2", "N(2,3)", "p3:0", "p3:1"]);
        assert_eq!(next.to.unwrap().id, 3);
    }

    #[test]
    fn a_loaded_neighbour_with_no_gap_and_no_force_has_no_transition() {
        let set = ChapterSet {
            curr: snap(2, 2.0, LoadState::Loaded, 2),
            prev: Some(snap(1, 1.0, LoadState::Loaded, 2)),
            next: Some(snap(3, 3.0, LoadState::Loaded, 2)),
        };
        let (items, _) = build_flow(&set, false, body);
        assert_eq!(label(&items), ["p1:0", "p1:1", "p2:0", "p2:1", "p3:0", "p3:1"]);
    }

    #[test]
    fn the_first_chapter_of_the_book_starts_with_a_prev_transition_to_nothing() {
        let set = ChapterSet { curr: snap(2, 2.0, LoadState::Loaded, 1), prev: None, next: None };
        let (items, next) = build_flow(&set, false, body);
        assert_eq!(label(&items), ["P(2,-)", "p2:0", "N(2,-)"]);
        assert!(next.to.is_none());
    }

    #[test]
    fn a_gap_forces_the_transition_even_when_the_neighbour_is_loaded() {
        let set = ChapterSet {
            curr: snap(2, 5.0, LoadState::Loaded, 1),
            prev: Some(snap(1, 2.0, LoadState::Loaded, 1)),
            next: None,
        };
        let (items, _) = build_flow(&set, false, body);
        assert_eq!(label(&items), ["p1:0", "P(2,1)", "p2:0", "N(2,-)"]);
    }

    #[test]
    fn an_empty_loaded_chapter_adds_no_pages_and_does_not_panic() {
        let set = ChapterSet { curr: snap(2, 2.0, LoadState::Loaded, 0), prev: None, next: None };
        let (items, _) = build_flow(&set, true, body);
        assert_eq!(label(&items), ["P(2,-)", "N(2,-)"]);
    }
}
