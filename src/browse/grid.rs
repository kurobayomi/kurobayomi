//! Grid and list cells, and the page loader state (T-40a-004, T-40a-049). The pager keeps one request in flight and drops
//! pages from an older generation (R§3, E-40 R-40-09, E-PG-03).

/// Padding around the grid content and between cells (B§2.7).
pub const GRID_PADDING_PX: i32 = 8;
/// The container carries the padding minus half a gap and every cell carries half a gap on each side (the shared
/// `cover_grid::CELL_MARGIN_PX`), so the cells sit 4 px apart and the content 8 px from the edge (B§2.7).
pub use crate::widgets::cover_grid::CELL_MARGIN_PX;
pub const CONTAINER_MARGIN_PX: i32 = GRID_PADDING_PX - CELL_MARGIN_PX;
/// Alpha of a favourite's cover; non-favourites are drawn at full strength (B§2.7, D§13).
pub const FAVORITE_ALPHA: f64 = 0.34;
/// Items per page and the first load's size (B§4.3).
pub const PAGE_SIZE: usize = 25;

pub fn cell_alpha(favorite: bool) -> f64 {
    if favorite { FAVORITE_ALPHA } else { 1.0 }
}

/// The in-library bookmark badge shows only for favourites (B§2.10).
pub fn badge_visible(favorite: bool) -> bool {
    favorite
}

/// List mode fills the window below the top bar (B§2.8). Never negative.
pub fn list_height(container_px: i32, top_bar_px: i32) -> i32 {
    (container_px - top_bar_px).max(0)
}

/// The next page is requested when the last visible index plus one page reaches the total (B§4.3).
pub fn prefetch_due(last_visible: usize, total: usize) -> bool {
    last_visible + PAGE_SIZE >= total
}

/// Index of the last cell on screen, estimated from the scroll position (`value`, `page` and `upper` of the vertical
/// adjustment) and the number of cells. Content that fits the window counts as fully visible.
pub fn last_visible_index(value: f64, page: f64, upper: f64, cells: usize) -> usize {
    if cells == 0 {
        return 0;
    }
    if upper <= 0.0 || upper <= page {
        return cells - 1;
    }
    let seen = ((value + page) / upper).clamp(0.0, 1.0);
    (((cells as f64) * seen).ceil() as usize).saturating_sub(1).min(cells - 1)
}

/// The recycle guard of a grid cell (edge R10): every bind of the cell takes a new ticket, and a cover that arrives for an
/// older bind is dropped, so a recycled cell never shows another item's picture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoverBind {
    current: u64,
}

impl CoverBind {
    /// The cell is bound to an item: returns the ticket of that item's cover load.
    pub fn bind(&mut self) -> u64 {
        self.current += 1;
        self.current
    }

    /// Whether a cover that finished under `ticket` may still be applied to the cell.
    pub fn accepts(&self, ticket: u64) -> bool {
        ticket == self.current
    }
}

/// The `CoverBind` of every recycled cell of one factory, keyed by the cell object (edge R10).
#[derive(Debug, Default)]
pub struct CoverBinds {
    cells: std::collections::HashMap<usize, CoverBind>,
}

impl CoverBinds {
    /// The cell `key` is bound to an item: the ticket of that item's cover load.
    pub fn bind(&mut self, key: usize) -> u64 {
        self.cells.entry(key).or_default().bind()
    }

    /// Whether a cover that finished under `ticket` may still be applied to the cell.
    pub fn accepts(&self, key: usize, ticket: u64) -> bool {
        self.cells.get(&key).is_some_and(|bind| bind.accepts(ticket))
    }

    /// The cell was torn down.
    pub fn release(&mut self, key: usize) {
        self.cells.remove(&key);
    }
}

/// A request for the next page, tagged with the generation it belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageAsk {
    pub generation: u64,
    pub page: u32,
}

/// Pages of one listing. At most one request is in flight; a reset starts a new generation and late pages of the old
/// one are dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pager {
    generation: u64,
    next: u32,
    in_flight: bool,
    done: bool,
}

impl Pager {
    pub fn new() -> Self {
        Self { generation: 0, next: 1, in_flight: false, done: false }
    }

    /// The next page to request, or `None` while one is in flight or the listing has ended.
    pub fn ask(&mut self) -> Option<PageAsk> {
        if self.in_flight || self.done {
            return None;
        }
        self.in_flight = true;
        Some(PageAsk { generation: self.generation, page: self.next })
    }

    /// A page arrived. Returns `false` (and changes nothing) when the page belongs to an older generation.
    pub fn deliver(&mut self, generation: u64, has_next: bool) -> bool {
        if generation != self.generation {
            return false;
        }
        self.in_flight = false;
        self.next += 1;
        self.done = !has_next;
        true
    }

    /// A page failed. The same page may be asked again. Returns `false` for a stale generation.
    pub fn fail(&mut self, generation: u64) -> bool {
        if generation != self.generation {
            return false;
        }
        self.in_flight = false;
        true
    }

    /// Starts over: a new search or a refresh. Pages still in flight from before are dropped.
    pub fn reset(&mut self) {
        self.generation += 1;
        self.next = 1;
        self.in_flight = false;
        self.done = false;
    }

    pub fn is_done(&self) -> bool {
        self.done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_metrics_and_favourite_alpha_match_the_design() {
        assert_eq!((GRID_PADDING_PX, CELL_MARGIN_PX * 2), (8, 4));
        assert_eq!(cell_alpha(true), 0.34);
        assert_eq!(cell_alpha(false), 1.0);
        assert!(badge_visible(true));
        assert!(!badge_visible(false));
    }

    #[test]
    fn a_stale_cover_never_reaches_a_recycled_cell() {
        let mut binds = CoverBinds::default();
        let first = binds.bind(1);
        let second = binds.bind(1);
        let other = binds.bind(2);
        assert!(!binds.accepts(1, first), "the cell was rebound to another item");
        assert!(binds.accepts(1, second));
        assert!(binds.accepts(2, other), "cells do not share tickets");
        binds.release(1);
        assert!(!binds.accepts(1, second), "a torn-down cell takes nothing");
    }

    #[test]
    fn list_height_is_the_container_minus_the_top_bar_and_never_negative() {
        assert_eq!(list_height(800, 56), 744);
        assert_eq!(list_height(40, 56), 0);
    }

    #[test]
    fn prefetch_is_due_when_the_last_visible_index_plus_a_page_reaches_the_total() {
        assert!(!prefetch_due(0, 100));
        assert!(prefetch_due(75, 100));
        assert!(prefetch_due(99, 100));
    }

    #[test]
    fn the_last_visible_cell_follows_the_scroll_position() {
        assert_eq!(last_visible_index(0.0, 500.0, 500.0, 40), 39, "content that fits is all visible");
        assert_eq!(last_visible_index(0.0, 100.0, 1000.0, 100), 9);
        assert_eq!(last_visible_index(900.0, 100.0, 1000.0, 100), 99);
        assert_eq!(last_visible_index(2000.0, 100.0, 1000.0, 100), 99, "overscroll is clamped");
        assert_eq!(last_visible_index(0.0, 100.0, 1000.0, 0), 0);
    }

    #[test]
    fn a_cover_for_a_recycled_cell_is_dropped() {
        let mut cell = CoverBind::default();
        let first = cell.bind();
        assert!(cell.accepts(first));
        let second = cell.bind();
        assert!(!cell.accepts(first), "the earlier item's cover never lands on the recycled cell");
        assert!(cell.accepts(second));
    }

    #[test]
    fn only_one_page_request_is_in_flight_at_a_time() {
        let mut pager = Pager::new();
        let first = pager.ask().expect("first page");
        assert_eq!(first.page, 1);
        assert_eq!(pager.ask(), None, "a second ask while one is in flight is refused");
        assert!(pager.deliver(first.generation, true));
        assert_eq!(pager.ask().map(|a| a.page), Some(2));
    }

    #[test]
    fn a_page_from_an_older_generation_is_dropped() {
        let mut pager = Pager::new();
        let old = pager.ask().expect("page");
        pager.reset();
        assert!(!pager.deliver(old.generation, true), "stale page dropped");
        assert!(!pager.fail(old.generation), "stale failure dropped");
        let fresh = pager.ask().expect("fresh page after reset");
        assert_eq!(fresh.generation, old.generation + 1);
        assert_eq!(fresh.page, 1);
    }

    #[test]
    fn a_listing_without_a_next_page_ends_and_stops_asking() {
        let mut pager = Pager::new();
        let ask = pager.ask().expect("page");
        pager.deliver(ask.generation, false);
        assert!(pager.is_done());
        assert_eq!(pager.ask(), None);
    }

    #[test]
    fn a_failed_page_can_be_asked_again() {
        let mut pager = Pager::new();
        let ask = pager.ask().expect("page");
        assert!(pager.fail(ask.generation));
        assert_eq!(pager.ask().map(|a| a.page), Some(1));
    }
}
