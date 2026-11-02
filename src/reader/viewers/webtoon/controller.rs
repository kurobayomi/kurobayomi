//! The webtoon's state machine (T-44a-034, B§7.2-7.5, B§5.5, E-27, E-29). Pure: every method returns effects.
//! Changes apply at once (no deferral, B§7.5). Scroll position is supplied by the view as the bottom-most
//! visible item and the first visible item, since the layout owns the geometry.

use super::items::build_webtoon_items;
use crate::reader::viewers::effect::Effect;
use crate::reader::viewers::model::{ChapterKey, ChapterSet, Dir, ItemKey, PageRef, Transition};

/// Settings the webtoon reads on every decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WebtoonOptions {
    pub always_show_transition: bool,
    pub use_transitions: bool,
    /// The scroll distance that hides the menu (`ReaderHideThreshold`, read live).
    pub hide_threshold: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CurrentChapter {
    key: ChapterKey,
    page_count: u32,
}

pub struct WebtoonController {
    opts: WebtoonOptions,
    items: Vec<ItemKey>,
    chapter: Option<CurrentChapter>,
    current: Option<ItemKey>,
    laid_out: bool,
    /// The item a jump went to, reported by the next scroll when no item ends inside the view (a page taller than
    /// the view fills it), as upstream's `moveToPage` does when `findLastEndVisibleItemPosition` finds none.
    jumped: Option<usize>,
}

impl WebtoonController {
    pub fn new(opts: WebtoonOptions) -> Self {
        Self { opts, items: Vec::new(), chapter: None, current: None, laid_out: false, jumped: None }
    }

    pub fn set_options(&mut self, opts: WebtoonOptions) {
        self.opts = opts;
    }

    pub fn items(&self) -> &[ItemKey] {
        &self.items
    }

    #[cfg(test)]
    pub fn current(&self) -> Option<&ItemKey> {
        self.current.as_ref()
    }

    /// `setChapters` (B§7.5): applied immediately; the first layout scrolls to the requested page and shows.
    pub fn set_chapters(&mut self, set: ChapterSet) -> Vec<Effect> {
        let force = self.opts.always_show_transition || self.current.as_ref().is_some_and(|c| c.transition().is_some());
        let (items, _next) = build_webtoon_items(&set, force);
        self.items = items;
        self.chapter = Some(CurrentChapter { key: set.curr.key.clone(), page_count: set.curr.page_count });
        let mut fx = vec![Effect::Rebuilt];
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

    /// `moveToPage` (B§7.5). The view scrolls without animation and then reports a scroll with `dy == 0`.
    pub fn move_to_page(&mut self, key: &ItemKey) -> Vec<Effect> {
        match self.items.iter().position(|i| i == key) {
            Some(index) => {
                self.jumped = Some(index);
                vec![Effect::ScrollTo { index, smooth: false }]
            }
            None => Vec::new(),
        }
    }

    /// A scroll step (B§7.2). `last_visible` is the bottom-most visible item, `first_visible` the top one.
    /// `menu_visible` gates the hide rule; `dy` is the scroll delta in pixels (positive is down).
    pub fn on_scroll(&mut self, dy: i32, menu_visible: bool, last_visible: Option<usize>, first_visible: Option<usize>) -> Vec<Effect> {
        let mut fx = Vec::new();
        if let Some(pos) = last_visible.or(self.jumped.take()) {
            fx.extend(self.report_current(pos));
        }
        if (dy > self.opts.hide_threshold || dy < -self.opts.hide_threshold) && menu_visible {
            fx.push(Effect::HideMenu);
        }
        if dy < 0 {
            if let Some(ItemKey::Transition(t)) = first_visible.and_then(|i| self.items.get(i)) {
                if let (Dir::Prev, Some(to)) = (t.dir, &t.to) {
                    fx.push(Effect::Preload(to.clone()));
                }
            }
        }
        if let Some(ItemKey::Transition(t)) = last_visible.and_then(|i| self.items.get(i)) {
            if t.dir == Dir::Next && t.to.is_none() {
                fx.push(Effect::ShowMenu);
            }
        }
        fx
    }

    /// `onScrolled(pos)` (B§7.3): a changed current item is reported once.
    fn report_current(&mut self, pos: usize) -> Vec<Effect> {
        let Some(item) = self.items.get(pos).cloned() else { return Vec::new() };
        if self.current.as_ref() == Some(&item) {
            return Vec::new();
        }
        let allow = self.allow_preload(&item);
        self.current = Some(item.clone());
        match &item {
            ItemKey::Page(page) | ItemKey::Spread(page, _) => self.on_page_selected(page, allow),
            ItemKey::Transition(t) => self.on_transition_selected(t),
        }
    }

    /// `checkAllowPreload` (B§7.3): pages of the current page's chapter or of the chapter the list ends in.
    /// Transitions and a missing current item always allow it.
    fn allow_preload(&self, item: &ItemKey) -> bool {
        let (Some(current), ItemKey::Page(page)) = (self.current.as_ref(), item) else { return true };
        let last_chapter = self.last_chapter();
        current.page().is_some_and(|c| c.chapter == page.chapter) || last_chapter.as_ref() == Some(&page.chapter)
    }

    /// The chapter the list ends in: a `Next` destination, or the chapter of a last page (B§7.3).
    fn last_chapter(&self) -> Option<ChapterKey> {
        match self.items.last()? {
            ItemKey::Transition(Transition { dir: Dir::Next, to, .. }) => to.clone(),
            ItemKey::Transition(_) => None,
            ItemKey::Page(p) | ItemKey::Spread(p, _) => Some(p.chapter.clone()),
        }
    }

    /// A page became current. Quirk of B§7.3, reproduced: the preload target is the chapter the list ends in, which is
    /// the chapter of the last page when the list ends in a page, not necessarily the chapter of this page.
    fn on_page_selected(&self, page: &PageRef, allow: bool) -> Vec<Effect> {
        let mut fx = vec![Effect::PageSelected(page.clone())];
        // `logcat` in WebtoonViewer.onPageSelected (T-44a-083)
        tracing::debug!(page = page.number(), "webtoon onPageSelected");
        let Some(chapter) = &self.chapter else { return fx };
        if i64::from(chapter.page_count) - i64::from(page.number()) < 5 && allow && chapter.key == page.chapter {
            tracing::debug!(page = page.number(), total = chapter.page_count, "webtoon requests preload of the next chapter");
            if let Some(to) = self.last_chapter() {
                fx.push(Effect::Preload(to));
            }
        }
        fx
    }

    fn on_transition_selected(&self, t: &Transition) -> Vec<Effect> {
        std::iter::once(Effect::TransitionSelected).chain(t.to.iter().cloned().map(Effect::Preload)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::viewers::items::tests::snap;
    use crate::reader::viewers::model::{ChapterSnap, LoadState};

    const OPTS: WebtoonOptions = WebtoonOptions { always_show_transition: true, use_transitions: true, hide_threshold: 31 };

    fn loaded(id: i64, number: f64, pages: u32) -> ChapterSnap {
        snap(id, number, LoadState::Loaded, pages)
    }

    fn set(curr: ChapterSnap, next: Option<ChapterSnap>) -> ChapterSet {
        ChapterSet { curr, prev: None, next }
    }

    fn preloads(fx: &[Effect]) -> Vec<i64> {
        fx.iter().filter_map(|e| if let Effect::Preload(k) = e { Some(k.id) } else { None }).collect()
    }

    /// Chapter 2 with 20 pages and chapter 3 loaded after it. With no previous chapter the list is `P(2,-)`, the 20
    /// pages, `N(2,3)`, then the first two pages of chapter 3.
    fn strip() -> WebtoonController {
        let mut c = WebtoonController::new(OPTS);
        c.set_chapters(set(loaded(2, 2.0, 20), Some(loaded(3, 3.0, 4))));
        c
    }

    fn index_of_page(c: &WebtoonController, chapter: i64, index: u32) -> usize {
        c.items().iter().position(|i| i.page().is_some_and(|p| p.chapter.id == chapter && p.index == index)).unwrap()
    }

    #[test]
    fn the_first_layout_scrolls_to_the_requested_page_without_animation() {
        let mut curr = loaded(2, 2.0, 5);
        curr.requested_page = 2;
        let mut c = WebtoonController::new(OPTS);
        let fx = c.set_chapters(set(curr, None));
        let target = index_of_page(&c, 2, 2);
        assert!(fx.contains(&Effect::ScrollTo { index: target, smooth: false }));
        assert!(fx.contains(&Effect::Show));
    }

    #[test]
    fn an_empty_first_chapter_shows_without_scrolling() {
        let mut c = WebtoonController::new(OPTS);
        let fx = c.set_chapters(set(loaded(2, 2.0, 0), None));
        assert!(fx.contains(&Effect::Show));
        assert!(!fx.iter().any(|e| matches!(e, Effect::ScrollTo { .. })));
    }

    #[test]
    fn the_last_five_pages_preload_the_chapter_the_list_ends_in() {
        let mut c = strip();
        let mut seen = Vec::new();
        for page in 0..20 {
            let fx = c.on_scroll(0, false, Some(index_of_page(&c, 2, page)), None);
            if !preloads(&fx).is_empty() {
                seen.push(page + 1);
            }
        }
        assert_eq!(seen, vec![16, 17, 18, 19, 20]);
    }

    #[test]
    fn a_menu_hides_only_past_the_threshold_boundary() {
        let mut c = strip();
        assert!(!c.on_scroll(31, true, None, None).contains(&Effect::HideMenu));
        assert!(!c.on_scroll(-31, true, None, None).contains(&Effect::HideMenu));
        assert!(c.on_scroll(32, true, None, None).contains(&Effect::HideMenu));
        assert!(c.on_scroll(-32, true, None, None).contains(&Effect::HideMenu));
        assert!(!c.on_scroll(40, false, None, None).contains(&Effect::HideMenu));
    }

    #[test]
    fn scrolling_up_past_a_previous_transition_preloads_its_destination() {
        let mut c = WebtoonController::new(OPTS);
        c.set_chapters(set(loaded(2, 2.0, 2), None));
        let mut prev_set = set(loaded(2, 2.0, 2), None);
        prev_set.prev = Some(loaded(1, 1.0, 3));
        c.set_chapters(prev_set);
        let first = c.items().iter().position(|i| i.transition().is_some_and(|t| t.dir == Dir::Prev)).unwrap();
        assert_eq!(preloads(&c.on_scroll(-10, false, None, Some(first))), vec![1]);
        assert!(preloads(&c.on_scroll(10, false, None, Some(first))).is_empty());
    }

    #[test]
    fn the_end_of_the_book_shows_the_menu_when_the_last_visible_item_is_the_terminal_next() {
        let mut c = WebtoonController::new(OPTS);
        c.set_chapters(set(loaded(2, 2.0, 2), None));
        let last = c.items().len() - 1;
        assert!(c.on_scroll(10, false, Some(last), None).contains(&Effect::ShowMenu));
        assert!(!c.on_scroll(10, false, Some(0), None).contains(&Effect::ShowMenu));
    }

    #[test]
    fn a_changed_current_item_is_reported_once() {
        let mut c = strip();
        let first = index_of_page(&c, 2, 0);
        assert!(c.on_scroll(0, false, Some(first), None).contains(&Effect::PageSelected(loaded(2, 2.0, 20).page(0))));
        assert!(!c.on_scroll(0, false, Some(first), None).iter().any(|e| matches!(e, Effect::PageSelected(_))));
    }

    #[test]
    fn a_jump_to_a_page_taller_than_the_view_reports_that_page() {
        let mut c = strip();
        let target = index_of_page(&c, 2, 3);
        assert!(!c.move_to_page(&c.items()[target].clone()).is_empty());
        assert!(c.on_scroll(0, false, None, Some(target)).contains(&Effect::PageSelected(loaded(2, 2.0, 20).page(3))));
        assert!(!c.on_scroll(0, false, None, Some(target)).iter().any(|e| matches!(e, Effect::PageSelected(_))), "only once");
    }

    #[test]
    fn a_move_to_an_unknown_item_does_nothing() {
        let mut c = strip();
        assert!(c.move_to_page(&ItemKey::Page(loaded(9, 9.0, 1).page(0))).is_empty());
    }

    #[test]
    fn a_card_on_screen_is_reported_as_a_card_and_not_as_a_page() {
        let mut c = strip();
        let next = c.items().iter().position(|i| i.transition().is_some_and(|t| t.dir == Dir::Next)).unwrap();
        let fx = c.on_scroll(10, false, Some(next), None);
        assert!(fx.contains(&Effect::TransitionSelected));
        assert!(!fx.iter().any(|e| matches!(e, Effect::PageSelected(_))));
    }
}
