//! The pager's state machine (T-44a-025, T-44a-026, T-44a-027, T-44a-028, B§6). Pure: every method returns the
//! effects the view and the engine must carry out. Nothing here touches GTK.
//!
//! Selection side effects (B§6.4): a settled page hides the menu unless the slider is moving, reports the page,
//! runs the zoom-start hook, and may ask for the next chapter. A transition asks for its destination, and the
//! terminal `Next` shows the menu.
//!
//! Two pages side by side: the list is built page by page (`flat`) and, while spreads are on, paired into spreads
//! (`pair_spreads`). A page found to be wide later re-pairs it. Splitting wide pages is off meanwhile: a wide page shows
//! whole on its own.

use std::collections::{BTreeMap, HashSet};

use super::items::{build_pager_items, pair_spreads, place_insert};
use super::nav::{MoveDecision, PageCmd, decide};
use crate::reader::viewers::effect::Effect;
use crate::reader::viewers::ViewerKind;
use crate::reader::viewers::model::{ChapterKey, ChapterSet, Dir, ItemKey, PageRef, Transition};

/// Settings the pager reads on every decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PagerOptions {
    pub always_show_transition: bool,
    pub use_transitions: bool,
    pub navigate_pan: bool,
}

/// The chapter whose pages are in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CurrentChapter {
    key: ChapterKey,
    page_count: u32,
    loaded: bool,
}

pub struct PagerController {
    kind: ViewerKind,
    opts: PagerOptions,
    /// The list page by page, as built.
    flat: Vec<ItemKey>,
    /// The list the strip shows: `flat`, paired into spreads while `two_pages` is on.
    items: Vec<ItemKey>,
    two_pages: bool,
    /// Pages known to be wider than tall (they stand alone in two-page mode).
    wide: HashSet<PageRef>,
    /// A re-pairing waits for the strip to settle: a page turn gliding toward an index of the old list would land on
    /// another item.
    relayout_pending: bool,
    preprocessed: BTreeMap<u32, PageRef>,
    next: Option<Transition>,
    chapter: Option<CurrentChapter>,
    current: Option<ItemKey>,
    /// The index the strip shows (`pager.currentItem`).
    view_index: usize,
    idle: bool,
    awaiting: Option<ChapterSet>,
    laid_out: bool,
    slider_moving: bool,
}

impl PagerController {
    pub fn new(kind: ViewerKind, opts: PagerOptions) -> Self {
        Self {
            kind,
            opts,
            flat: Vec::new(),
            items: Vec::new(),
            two_pages: false,
            wide: HashSet::new(),
            relayout_pending: false,
            preprocessed: BTreeMap::new(),
            next: None,
            chapter: None,
            current: None,
            view_index: 0,
            idle: true,
            awaiting: None,
            laid_out: false,
            slider_moving: false,
        }
    }

    pub fn set_options(&mut self, opts: PagerOptions) {
        self.opts = opts;
    }

    pub fn items(&self) -> &[ItemKey] {
        &self.items
    }

    /// Shows two pages side by side, or one. The strip stays on the page it shows.
    pub fn set_two_pages(&mut self, on: bool) -> Vec<Effect> {
        if self.two_pages == on {
            return Vec::new();
        }
        self.two_pages = on;
        // a wide page shows whole beside the others: its split halves go
        if on {
            self.flat.retain(|i| !i.page().is_some_and(PageRef::is_insert));
            self.preprocessed.clear();
        }
        self.relayout_when_idle()
    }

    /// `page` turned out wider than tall: in two-page mode it stands alone, and the pages after it pair again.
    pub fn set_wide(&mut self, page: PageRef) -> Vec<Effect> {
        if !self.wide.insert(page) || !self.two_pages {
            return Vec::new();
        }
        self.relayout_when_idle()
    }

    /// [`Self::relayout`] now when the strip is idle, else once it settles (`set_idle`).
    fn relayout_when_idle(&mut self) -> Vec<Effect> {
        if self.idle {
            return self.relayout();
        }
        self.relayout_pending = true;
        Vec::new()
    }

    /// Pairs `flat` into the shown list again; `Rebuilt` when that changed it. When the item on screen changed with it
    /// (a page now beside another, or alone again), that item is selected, so a page that joined the screen counts as
    /// read.
    fn relayout(&mut self) -> Vec<Effect> {
        let items = self.paired();
        if items == self.items {
            return Vec::new();
        }
        self.items = items;
        self.reanchor();
        let mut fx = vec![Effect::Rebuilt];
        if self.current.is_some() {
            fx.extend(self.on_page_change(self.view_index));
        }
        fx
    }

    fn paired(&self) -> Vec<ItemKey> {
        if self.two_pages { pair_spreads(self.flat.clone(), self.kind, &|page| self.wide.contains(page)) } else { self.flat.clone() }
    }

    /// The item the reader has settled on, if any (the engine's settle check reads it).
    pub fn current(&self) -> Option<&ItemKey> {
        self.current.as_ref()
    }

    pub fn view_index(&self) -> usize {
        self.view_index
    }

    /// The `Next` transition's destination, the preload target (`adapter.nextTransition.to`).
    fn next_to(&self) -> Option<&ChapterKey> {
        self.next.as_ref().and_then(|t| t.to.as_ref())
    }

    /// `setChapters`: applied now when the strip is idle, otherwise kept until it settles (E-75: last one wins).
    pub fn set_chapters(&mut self, set: ChapterSet) -> Vec<Effect> {
        if self.idle {
            self.apply(set)
        } else {
            self.awaiting = Some(set);
            Vec::new()
        }
    }

    /// The strip's idle state. Becoming idle applies a pending chapter set, and a single-page chapter then asks
    /// for the next chapter (E-76).
    pub fn set_idle(&mut self, idle: bool) -> Vec<Effect> {
        self.idle = idle;
        if !idle {
            return Vec::new();
        }
        let mut fx = if std::mem::take(&mut self.relayout_pending) { self.relayout() } else { Vec::new() };
        let Some(set) = self.awaiting.take() else { return fx };
        fx.extend(self.apply(set));
        if let (Some(chapter), Some(to)) = (&self.chapter, self.next_to()) {
            if chapter.loaded && chapter.page_count == 1 {
                fx.push(Effect::Preload(to.clone()));
            }
        }
        fx
    }

    fn apply(&mut self, set: ChapterSet) -> Vec<Effect> {
        let force = self.opts.always_show_transition || self.current.as_ref().is_some_and(|c| c.transition().is_some());
        let list = build_pager_items(&set, force, &mut self.preprocessed, self.kind);
        self.flat = list.items;
        self.items = self.paired();
        self.next = Some(list.next);
        self.chapter = Some(CurrentChapter {
            key: set.curr.key.clone(),
            page_count: set.curr.page_count,
            loaded: set.curr.is_loaded(),
        });
        self.reanchor();
        let mut fx = vec![Effect::Rebuilt];
        if let Some(jump) = list.jump {
            fx.extend(self.move_to_page(&jump));
        }
        // The first layout waits for a loaded chapter (`curr.pages ?: return`), then shows the strip. A chapter
        // with no pages shows the strip on its transition without a move (E-01).
        if !self.laid_out && set.curr.is_loaded() {
            if set.curr.page_count > 0 {
                let index = set.curr.requested_page.min(set.curr.page_count - 1);
                fx.extend(self.move_to_page(&ItemKey::Page(set.curr.page(index))));
            }
            self.laid_out = true;
            fx.push(Effect::Show);
        }
        fx
    }

    /// Keeps the strip on the item it shows when the list changes: a neighbour chapter that finished loading adds
    /// pages in front of it, so the index alone would point at another item (`ViewPager.getItemPosition`).
    fn reanchor(&mut self) {
        let last = self.items.len().saturating_sub(1);
        // a page and a spread holding it are the same place: the spread re-paired or split around it
        let kept = self.current.as_ref().and_then(|c| self.items.iter().position(|i| i == c || c.page().is_some_and(|p| i.shows(p))));
        self.view_index = kept.unwrap_or_else(|| self.view_index.min(last));
    }

    pub fn is_idle(&self) -> bool {
        self.idle
    }

    /// The strip settled on `index` (ViewPager `onPageSelected`): hides the menu unless the slider is moving. The
    /// slider flag is spent on this selection (E-82 proposal), so later swipes hide the menu again.
    pub fn on_settled(&mut self, index: usize) -> Vec<Effect> {
        self.view_index = index;
        let mut fx = Vec::new();
        if !self.slider_moving {
            fx.push(Effect::HideMenu);
        }
        self.slider_moving = false;
        fx.extend(self.on_page_change(index));
        fx
    }

    #[cfg(test)]
    /// Set while the page slider drags, so the page it lands on keeps the menu open.
    pub fn set_slider_moving(&mut self, moving: bool) {
        self.slider_moving = moving;
    }

    /// `onPageChange` (B§6.4): a new item becomes the current one.
    pub fn on_page_change(&mut self, index: usize) -> Vec<Effect> {
        let Some(item) = self.items.get(index).cloned() else { return Vec::new() };
        if self.current.as_ref() == Some(&item) {
            return Vec::new();
        }
        let allow = allow_preload(&item, self.current.as_ref(), self.next_to());
        let forward = forward_flag(self.current.as_ref(), &item);
        self.current = Some(item.clone());
        match &item {
            ItemKey::Page(page) => self.on_page_selected(page, allow, forward),
            // both pages are read: the first is reported, then the second runs the selection (progress, preload)
            ItemKey::Spread(first, second) => {
                let mut fx = vec![Effect::PageSelected(first.clone())];
                fx.extend(self.on_page_selected(second, allow, forward));
                fx.push(Effect::ShownBeside(first.clone()));
                fx
            }
            ItemKey::Transition(t) => self.on_transition_selected(t),
        }
    }

    fn on_page_selected(&self, page: &PageRef, allow: bool, forward: bool) -> Vec<Effect> {
        let mut fx = vec![Effect::PageSelected(page.clone()), Effect::ZoomStart { page: page.clone(), forward }];
        tracing::debug!(page = page.number(), "pager onPageSelected");
        if page.is_insert() {
            return fx;
        }
        let Some(chapter) = &self.chapter else { return fx };
        // The last five pages of the current chapter (`size - number < 5`).
        let in_range = i64::from(chapter.page_count) - i64::from(page.number()) < 5;
        if in_range && allow && chapter.key == page.chapter {
            tracing::debug!(page = page.number(), total = chapter.page_count, "pager requests preload of the next chapter");
            if let Some(to) = self.next_to() {
                fx.push(Effect::Preload(to.clone()));
            }
        }
        fx
    }

    fn on_transition_selected(&self, t: &Transition) -> Vec<Effect> {
        let mut fx = vec![Effect::TransitionSelected];
        match (&t.to, t.dir) {
            (Some(to), _) => fx.push(Effect::Preload(to.clone())),
            (None, Dir::Next) => fx.push(Effect::ShowMenu),
            (None, Dir::Prev) => {}
        }
        fx
    }

    /// `moveToPage` (B§6.6). A move to the index already shown runs the page change directly, since no
    /// settle will follow.
    pub fn move_to_page(&mut self, key: &ItemKey) -> Vec<Effect> {
        let Some(index) = self.items.iter().position(|i| i.holds(key)) else { return Vec::new() };
        if self.view_index == index {
            return self.on_page_change(index);
        }
        self.view_index = index;
        vec![Effect::ScrollTo { index, smooth: self.opts.use_transitions }]
    }

    /// A move command from a key, tap or wheel (B§6.3). `can_pan` is the current page's pan capability in the
    /// direction the command moves.
    pub fn command(&mut self, cmd: PageCmd, can_pan: bool) -> Vec<Effect> {
        match decide(self.kind, cmd, self.view_index, self.items.len(), self.opts.navigate_pan, can_pan, self.opts.use_transitions) {
            MoveDecision::Pan(dir) => vec![Effect::Pan(dir)],
            MoveDecision::Turn { index, smooth } => {
                self.view_index = index;
                vec![Effect::ScrollTo { index, smooth }]
            }
            MoveDecision::Nothing => Vec::new(),
        }
    }

    /// The first-layout and refresh rebuild (B§6.7): keeps the index and recreates the visible slots.
    pub fn refresh(&self) -> Vec<Effect> {
        vec![Effect::Rebuilt, Effect::ScrollTo { index: self.view_index, smooth: false }]
    }

    /// Removes every dual-page insert (`cleanupPageSplit`, B§5.6), used when splitting is switched off.
    pub fn remove_splits(&mut self) -> Vec<Effect> {
        let before = self.flat.len();
        self.flat.retain(|i| !i.page().is_some_and(PageRef::is_insert));
        self.preprocessed.clear();
        if self.flat.len() == before {
            return Vec::new();
        }
        self.items = self.paired();
        self.reanchor();
        vec![Effect::Rebuilt]
    }

    /// A dual-page split of `parent` (B§5.6). A split of a neighbour's page waits in `preprocessed`.
    pub fn on_page_split(&mut self, parent: &PageRef, insert: PageRef) -> Vec<Effect> {
        // two pages side by side show a wide page whole
        if self.two_pages {
            return Vec::new();
        }
        let is_current = self.chapter.as_ref().is_some_and(|c| c.key == parent.chapter);
        if !is_current {
            self.preprocessed.insert(insert.index, insert);
            return Vec::new();
        }
        let Some(index) = self.flat.iter().position(|i| i.page() == Some(parent)) else { return Vec::new() };
        if !place_insert(&mut self.flat, index, self.kind, insert) {
            return Vec::new();
        }
        self.items = self.paired();
        self.reanchor();
        vec![Effect::Rebuilt]
    }
}

/// `checkAllowPreload` (B§6.4). Transitions and a missing current page always allow it.
fn allow_preload(item: &ItemKey, current: Option<&ItemKey>, next_to: Option<&ChapterKey>) -> bool {
    let (Some(current), Some(page)) = (current, item.page()) else { return true };
    let from_next = current.transition().filter(|t| t.dir == Dir::Next).and_then(|t| t.to.as_ref());
    Some(&page.chapter) == from_next
        || current.page().is_some_and(|c| c.chapter == page.chapter)
        || next_to == Some(&page.chapter)
}

/// The `forward` flag of a selection (B§6.4 step 3, E-11, E-12).
fn forward_flag(current: Option<&ItemKey>, item: &ItemKey) -> bool {
    match (current, item.page()) {
        (Some(ItemKey::Transition(t)), Some(_)) => t.dir != Dir::Prev,
        (Some(c), Some(p)) => match c.page() {
            Some(c) if p.number() == c.number() => p.is_insert(),
            Some(c) => p.number() > c.number(),
            None => true,
        },
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::viewers::items::tests::snap;
    use crate::reader::viewers::model::{ChapterSnap, LoadState};

    const OPTS: PagerOptions = PagerOptions { always_show_transition: true, use_transitions: true, navigate_pan: false };

    fn loaded(id: i64, number: f64, pages: u32) -> ChapterSnap {
        snap(id, number, LoadState::Loaded, pages)
    }

    fn set(curr: ChapterSnap, prev: Option<ChapterSnap>, next: Option<ChapterSnap>) -> ChapterSet {
        ChapterSet { curr, prev, next }
    }

    fn selected(fx: &[Effect]) -> Vec<String> {
        fx.iter()
            .filter_map(|e| match e {
                Effect::PageSelected(p) => Some(format!("sel:{}:{}", p.chapter.id, p.index)),
                Effect::Preload(k) => Some(format!("preload:{}", k.id)),
                Effect::ShowMenu => Some("show".into()),
                Effect::HideMenu => Some("hide".into()),
                _ => None,
            })
            .collect()
    }

    /// A pager on a 20-page chapter 2 with chapter 3 loaded next, settled on its first page.
    fn settled_pager(kind: ViewerKind) -> PagerController {
        let mut c = PagerController::new(kind, OPTS);
        c.set_chapters(set(loaded(2, 2.0, 20), None, Some(loaded(3, 3.0, 4))));
        c.on_settled(0);
        c
    }

    #[test]
    fn the_first_layout_moves_to_the_requested_page_clamped_to_the_last() {
        let mut curr = loaded(2, 2.0, 5);
        curr.requested_page = 99;
        let mut c = PagerController::new(ViewerKind::LeftToRight, OPTS);
        let fx = c.set_chapters(set(curr, None, None));
        assert!(fx.contains(&Effect::Show));
        let target = c.items().iter().position(|i| i == &ItemKey::Page(snap(2, 2.0, LoadState::Loaded, 5).page(4))).unwrap();
        assert!(fx.contains(&Effect::ScrollTo { index: target, smooth: true }));
    }

    #[test]
    fn an_empty_chapter_shows_the_strip_without_a_move() {
        let mut c = PagerController::new(ViewerKind::LeftToRight, OPTS);
        let fx = c.set_chapters(set(loaded(2, 2.0, 0), None, None));
        assert!(fx.contains(&Effect::Show));
        assert!(!fx.iter().any(|e| matches!(e, Effect::ScrollTo { .. })));
    }

    #[test]
    fn a_chapter_set_during_settling_is_kept_and_the_last_one_wins() {
        let mut c = PagerController::new(ViewerKind::LeftToRight, OPTS);
        c.set_chapters(set(loaded(2, 2.0, 3), None, None));
        c.set_idle(false);
        assert!(c.set_chapters(set(loaded(5, 5.0, 2), None, None)).is_empty());
        assert!(c.set_chapters(set(loaded(7, 7.0, 4), None, None)).is_empty());
        let fx = c.set_idle(true);
        assert!(fx.contains(&Effect::Rebuilt));
        assert_eq!(c.items().iter().find_map(ItemKey::page).map(|p| p.chapter.id), Some(7));
    }

    #[test]
    fn a_single_page_chapter_on_idle_preloads_the_next_chapter() {
        let mut c = PagerController::new(ViewerKind::LeftToRight, OPTS);
        c.set_idle(false);
        c.set_chapters(set(loaded(2, 2.0, 1), None, Some(loaded(3, 3.0, 1))));
        let fx = c.set_idle(true);
        assert!(selected(&fx).contains(&"preload:3".to_string()));
    }

    #[test]
    fn the_last_five_pages_preload_the_next_chapter_and_earlier_pages_do_not() {
        let mut c = settled_pager(ViewerKind::LeftToRight);
        let mut preloads = Vec::new();
        for page in 0..20usize {
            let index = c.items().iter().position(|i| i.page().is_some_and(|p| p.index as usize == page && p.chapter.id == 2)).unwrap();
            let fx = c.on_page_change(index);
            if selected(&fx).contains(&"preload:3".to_string()) {
                preloads.push(page + 1);
            }
        }
        assert_eq!(preloads, vec![16, 17, 18, 19, 20]);
    }

    #[test]
    fn a_next_transition_with_no_destination_shows_the_menu_and_a_prev_one_does_nothing() {
        let mut c = PagerController::new(ViewerKind::LeftToRight, OPTS);
        c.set_chapters(set(loaded(2, 2.0, 1), None, None));
        let last = c.items().len() - 1;
        assert!(selected(&c.on_page_change(last)).contains(&"show".to_string()));
        let mut d = PagerController::new(ViewerKind::LeftToRight, OPTS);
        d.set_chapters(set(loaded(2, 2.0, 1), None, None));
        assert!(!selected(&d.on_page_change(0)).contains(&"show".to_string()));
        assert!(!selected(&d.on_page_change(0)).contains(&"hide".to_string()));
    }

    #[test]
    fn a_transition_with_a_destination_preloads_it() {
        let mut c = settled_pager(ViewerKind::LeftToRight);
        let next = c.items().iter().position(|i| i.transition().is_some_and(|t| t.dir == Dir::Next)).unwrap();
        assert_eq!(selected(&c.on_page_change(next)), ["preload:3".to_string()]);
    }

    #[test]
    fn a_settled_page_hides_the_menu_except_while_the_slider_moves_once() {
        let mut c = settled_pager(ViewerKind::LeftToRight);
        assert!(selected(&c.on_settled(2)).contains(&"hide".to_string()));
        c.set_slider_moving(true);
        assert!(!selected(&c.on_settled(3)).contains(&"hide".to_string()));
        assert!(selected(&c.on_settled(4)).contains(&"hide".to_string()));
    }

    #[test]
    fn the_forward_flag_follows_split_pairs_and_transitions() {
        let page = |i: u32| ItemKey::Page(loaded(2, 2.0, 3).page(i));
        let insert = ItemKey::Page(PageRef::insert_of(&loaded(2, 2.0, 3).page(1)));
        assert!(forward_flag(Some(&page(1)), &insert));
        assert!(!forward_flag(Some(&insert), &page(1)));
        let prev = ItemKey::Transition(Transition { dir: Dir::Prev, from: loaded(2, 2.0, 1).key, to: None });
        assert!(!forward_flag(Some(&prev), &page(0)));
        assert!(forward_flag(None, &page(0)));
    }

    #[test]
    fn a_split_of_the_current_page_is_placed_and_a_neighbours_is_kept_for_later() {
        let mut c = settled_pager(ViewerKind::LeftToRight);
        let parent = loaded(2, 2.0, 20).page(0);
        assert_eq!(c.on_page_split(&parent, PageRef::insert_of(&parent)), vec![Effect::Rebuilt]);
        assert!(c.items().iter().any(|i| i.page().is_some_and(PageRef::is_insert)));
        let neighbour = loaded(9, 9.0, 4).page(1);
        assert!(c.on_page_split(&neighbour, PageRef::insert_of(&neighbour)).is_empty());
    }

    #[test]
    fn moves_at_the_boundary_do_nothing_and_turns_use_the_transition_setting() {
        let mut c = settled_pager(ViewerKind::LeftToRight);
        assert!(c.command(PageCmd::Prev, false).is_empty());
        let fx = c.command(PageCmd::Next, false);
        assert_eq!(fx, vec![Effect::ScrollTo { index: 1, smooth: true }]);
    }

    #[test]
    fn switching_splits_off_removes_only_the_inserts() {
        let mut c = settled_pager(ViewerKind::LeftToRight);
        let parent = loaded(2, 2.0, 20).page(0);
        c.on_page_split(&parent, PageRef::insert_of(&parent));
        assert_eq!(c.remove_splits(), vec![Effect::Rebuilt]);
        assert!(!c.items().iter().any(|i| i.page().is_some_and(PageRef::is_insert)));
        assert!(c.remove_splits().is_empty());
    }

    #[test]
    fn move_to_the_shown_index_runs_the_page_change_without_scrolling() {
        let mut c = settled_pager(ViewerKind::LeftToRight);
        let first = c.items()[0].clone();
        assert!(c.move_to_page(&first).is_empty());
        let second = c.items()[1].clone();
        assert_eq!(c.move_to_page(&second), vec![Effect::ScrollTo { index: 1, smooth: true }]);
    }
    fn at(c: &PagerController, chapter: i64, index: u32) -> usize {
        c.items().iter().position(|i| i.page().is_some_and(|p| p.chapter.id == chapter && p.index == index && !p.is_insert())).unwrap()
    }

    /// One user step: the command, then the strip settling where it went.
    fn turn(c: &mut PagerController, cmd: PageCmd) -> Vec<Effect> {
        let moved = c.command(cmd, false);
        assert!(moved.iter().any(|e| matches!(e, Effect::ScrollTo { .. })), "the step moves: {moved:?}");
        let index = c.view_index();
        c.on_settled(index)
    }

    /// Chapter 2 (24 pages) with chapter 1 (5 pages) before it and chapter 3 after it, opened and settled on page 1.
    fn opened(kind: ViewerKind) -> PagerController {
        let mut c = PagerController::new(kind, OPTS);
        let fx = c.set_chapters(set(loaded(2, 2.0, 24), Some(loaded(1, 1.0, 5)), Some(loaded(3, 3.0, 4))));
        assert!(fx.contains(&Effect::ScrollTo { index: at(&c, 2, 0), smooth: true }));
        c.on_settled(at(&c, 2, 0));
        c
    }

    #[test]
    fn two_pages_pair_the_strip_and_keep_the_page_on_show() {
        let mut c = opened(ViewerKind::LeftToRight);
        let page = |i| loaded(2, 2.0, 24).page(i);
        let on_three = c.items().iter().position(|i| i.shows(&page(2))).unwrap();
        c.on_settled(on_three);
        assert_eq!(selected(&c.set_two_pages(true)), ["sel:2:1", "sel:2:2"], "the page joining the screen counts as read");
        assert_eq!(c.items()[c.view_index()], ItemKey::Spread(page(1), page(2)), "still on page 3, now beside page 2");
        let fx = c.move_to_page(&ItemKey::Page(page(4)));
        assert!(matches!(fx.as_slice(), [Effect::ScrollTo { index, .. }] if c.items()[*index] == ItemKey::Spread(page(3), page(4))), "a page asked for finds its spread");
        assert_eq!(c.set_wide(page(1))[0], Effect::Rebuilt);
        assert!(c.items().contains(&ItemKey::Page(page(1))) && c.items().contains(&ItemKey::Spread(page(2), page(3))), "a wide page stands alone and the pairs after it move");
        assert!(c.set_wide(page(1)).is_empty(), "known already");
        assert_eq!(c.set_two_pages(false)[0], Effect::Rebuilt);
        assert!(!c.items().iter().any(|i| matches!(i, ItemKey::Spread(..))));
    }

    #[test]
    fn pairing_waits_while_a_page_turn_glides_and_runs_once_the_strip_settles() {
        let mut c = opened(ViewerKind::RightToLeft);
        let before = c.items().to_vec();
        c.set_idle(false);
        assert!(c.set_two_pages(true).is_empty(), "nothing moves under the glide");
        assert_eq!(c.items(), before.as_slice());
        assert_eq!(c.set_idle(true)[0], Effect::Rebuilt);
        assert!(c.items().iter().any(|i| matches!(i, ItemKey::Spread(..))));
        let page = |i| loaded(2, 2.0, 24).page(i);
        c.set_idle(false);
        assert!(c.set_wide(page(3)).is_empty());
        assert_eq!(c.set_idle(true)[0], Effect::Rebuilt);
        assert!(c.items().contains(&ItemKey::Page(page(3))));
    }

    #[test]
    fn a_spread_reports_both_its_pages_and_splitting_is_off() {
        let mut c = opened(ViewerKind::RightToLeft);
        c.set_two_pages(true);
        let page = |i| loaded(2, 2.0, 24).page(i);
        let index = c.items().iter().position(|i| i == &ItemKey::Spread(page(1), page(2))).unwrap();
        assert_eq!(selected(&c.on_settled(index)), ["hide", "sel:2:1", "sel:2:2"]);
        assert!(c.on_page_split(&page(3), PageRef::insert_of(&page(3))).is_empty());
    }

    #[test]
    fn opening_lands_on_page_one_after_the_leading_prev_card_and_tail() {
        for kind in [ViewerKind::LeftToRight, ViewerKind::RightToLeft, ViewerKind::Vertical] {
            let c = opened(kind);
            assert_eq!(c.view_index(), at(&c, 2, 0), "{kind:?}");
            assert_eq!(c.current(), Some(&ItemKey::Page(loaded(2, 2.0, 24).page(0))), "{kind:?}");
        }
        let c = opened(ViewerKind::LeftToRight);
        assert_eq!(c.view_index(), 3, "two tail pages and the card come first");
    }

    #[test]
    fn a_neighbour_that_loads_later_keeps_the_strip_on_the_page_shown() {
        let mut c = PagerController::new(ViewerKind::LeftToRight, OPTS);
        c.set_chapters(set(loaded(2, 2.0, 24), Some(snap(1, 1.0, LoadState::Loading, 0)), None));
        c.on_settled(at(&c, 2, 0));
        assert_eq!(c.view_index(), 1);
        let fx = c.set_chapters(set(loaded(2, 2.0, 24), Some(loaded(1, 1.0, 5)), None));
        assert_eq!(c.view_index(), at(&c, 2, 0));
        assert_eq!(c.view_index(), 3, "two tail pages were added in front");
        assert!(!fx.iter().any(|e| matches!(e, Effect::ScrollTo { .. } | Effect::PageSelected(_))));
    }

    #[test]
    fn back_from_page_one_shows_the_prev_card_and_a_card_has_no_page() {
        let mut c = opened(ViewerKind::LeftToRight);
        let fx = turn(&mut c, PageCmd::Prev);
        assert_eq!(c.view_index(), 2);
        assert!(matches!(c.current(), Some(ItemKey::Transition(t)) if t.dir == Dir::Prev));
        assert!(fx.contains(&Effect::TransitionSelected));
        assert!(!fx.iter().any(|e| matches!(e, Effect::PageSelected(_))));
        assert_eq!(selected(&fx), ["hide".to_string(), "preload:1".to_string()]);
    }

    #[test]
    fn back_from_the_prev_card_lands_on_the_previous_chapters_last_page_without_a_reload() {
        let mut c = opened(ViewerKind::LeftToRight);
        turn(&mut c, PageCmd::Prev);
        let fx = turn(&mut c, PageCmd::Prev);
        assert_eq!(c.view_index(), at(&c, 1, 4));
        assert!(fx.contains(&Effect::PageSelected(loaded(1, 1.0, 5).page(4))));
        assert!(!fx.contains(&Effect::Rebuilt));
        // the screen switches the current chapter: the strip stays on that page and moves nothing
        let fx = c.set_chapters(set(loaded(1, 1.0, 5), None, Some(loaded(2, 2.0, 24))));
        assert_eq!(c.view_index(), at(&c, 1, 4));
        assert_eq!(c.current(), Some(&ItemKey::Page(loaded(1, 1.0, 5).page(4))));
        assert!(!fx.iter().any(|e| matches!(e, Effect::ScrollTo { .. } | Effect::PageSelected(_) | Effect::Show)));
    }

    #[test]
    fn a_step_waits_on_the_card_while_the_previous_chapter_loads() {
        let mut c = PagerController::new(ViewerKind::LeftToRight, OPTS);
        c.set_chapters(set(loaded(2, 2.0, 3), Some(snap(1, 1.0, LoadState::Loading, 0)), None));
        c.on_settled(1);
        turn(&mut c, PageCmd::Prev);
        assert!(matches!(c.current(), Some(ItemKey::Transition(t)) if t.dir == Dir::Prev));
        assert!(c.command(PageCmd::Prev, false).is_empty(), "nothing before the card yet");
        c.set_chapters(set(loaded(2, 2.0, 3), Some(loaded(1, 1.0, 5)), None));
        assert!(c.items()[c.view_index()].transition().is_some(), "the strip still shows the card");
        turn(&mut c, PageCmd::Prev);
        assert_eq!(c.view_index(), at(&c, 1, 4));
    }

    #[test]
    fn forward_from_the_last_page_shows_the_next_card_then_the_next_chapters_first_page() {
        let mut c = opened(ViewerKind::LeftToRight);
        c.on_settled(at(&c, 2, 23));
        let fx = turn(&mut c, PageCmd::Next);
        assert!(matches!(c.current(), Some(ItemKey::Transition(t)) if t.dir == Dir::Next));
        assert_eq!(selected(&fx), ["hide".to_string(), "preload:3".to_string()]);
        assert!(!fx.iter().any(|e| matches!(e, Effect::PageSelected(_))));
        let fx = turn(&mut c, PageCmd::Next);
        assert_eq!(c.view_index(), at(&c, 3, 0));
        assert!(fx.contains(&Effect::PageSelected(loaded(3, 3.0, 4).page(0))));
        let fx = c.set_chapters(set(loaded(3, 3.0, 4), Some(loaded(2, 2.0, 24)), None));
        assert_eq!(c.view_index(), at(&c, 3, 0));
        assert!(!fx.iter().any(|e| matches!(e, Effect::ScrollTo { .. })));
    }

    #[test]
    fn a_right_to_left_pager_steps_back_through_the_cards_the_same_way() {
        let mut c = opened(ViewerKind::RightToLeft);
        turn(&mut c, PageCmd::Prev);
        assert!(matches!(c.current(), Some(ItemKey::Transition(t)) if t.dir == Dir::Prev));
        turn(&mut c, PageCmd::Prev);
        assert_eq!(c.view_index(), at(&c, 1, 4));
    }
}
