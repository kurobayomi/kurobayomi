//! Viewer item model (T-44a-002): plain snapshots of chapters and pages, the transition type with the symmetric
//! equality of `ChapterTransition.equals` (B§5.1, E-03), and the item key used for diffs and `move_to_page`.
//!
//! The snapshots are taken from the reader core's `ReaderChapter` and `ReaderPage` (`from_reader`), so the list
//! builders below are pure and unit-testable without GTK.

use std::hash::{Hash, Hasher};

use crate::reader::model::{ChapterState, ReaderChapter, ReaderPage, ViewerChapters};

pub use crate::reader::model::ChapterKey;

/// The load state a viewer observes (`ReaderChapter.State` without its payload).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadState {
    Wait,
    Loading,
    Error,
    Loaded,
}

impl LoadState {
    pub fn of(state: &ChapterState) -> Self {
        match state {
            ChapterState::Wait => Self::Wait,
            ChapterState::Loading => Self::Loading,
            ChapterState::Error(_) => Self::Error,
            ChapterState::Loaded(_) => Self::Loaded,
        }
    }
}

/// A chapter as the list builders see it. `number` is the chapter number, `-1` when unknown.
#[derive(Clone, Debug, PartialEq)]
pub struct ChapterSnap {
    pub key: ChapterKey,
    pub number: f64,
    pub state: LoadState,
    /// Pages exist only when `state` is `Loaded`; this count is 0 otherwise.
    pub page_count: u32,
    /// The page the viewer opens on (`requestedPage`).
    pub requested_page: u32,
}

impl ChapterSnap {
    pub fn from_reader(chapter: &ReaderChapter) -> Self {
        let page_count = match &*chapter.state() {
            ChapterState::Loaded(pages) => pages.len() as u32,
            _ => 0,
        };
        Self {
            key: chapter.key(),
            number: chapter.chapter.number,
            state: LoadState::of(&chapter.state()),
            page_count,
            requested_page: chapter.requested_page.get().max(0) as u32,
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.state == LoadState::Loaded
    }

    /// The reference to page `index` of this chapter.
    pub fn page(&self, index: u32) -> PageRef {
        PageRef { chapter: self.key.clone(), index, kind: PageKind::Normal }
    }

    /// Every page of a loaded chapter, in order. Empty for a chapter that is not loaded.
    pub fn pages(&self) -> Vec<PageRef> {
        if self.is_loaded() { (0..self.page_count).map(|i| self.page(i)).collect() } else { Vec::new() }
    }
}

/// The chapter being read and its neighbours (`ViewerChapters`), as snapshots.
#[derive(Clone, Debug, PartialEq)]
pub struct ChapterSet {
    pub curr: ChapterSnap,
    pub prev: Option<ChapterSnap>,
    pub next: Option<ChapterSnap>,
}

impl ChapterSet {
    pub fn from_viewer(chapters: &ViewerChapters) -> Self {
        Self {
            curr: ChapterSnap::from_reader(&chapters.curr),
            prev: chapters.prev.as_deref().map(ChapterSnap::from_reader),
            next: chapters.next.as_deref().map(ChapterSnap::from_reader),
        }
    }
}

/// Normal page or the second half of a dual-page split (`InsertPage`). An insert shares its parent's index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PageKind {
    Normal,
    Insert { parent_index: u32 },
}

/// A page of a chapter. `index` is 0-based; the label is `number()`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PageRef {
    pub chapter: ChapterKey,
    pub index: u32,
    pub kind: PageKind,
}

impl PageRef {
    /// `Page.number`: the 1-based label. An insert has the number of its parent.
    pub fn number(&self) -> u32 {
        self.index + 1
    }

    pub fn is_insert(&self) -> bool {
        matches!(self.kind, PageKind::Insert { .. })
    }

    /// The `InsertPage(parent)` that shares `parent`'s chapter and index.
    pub fn insert_of(parent: &PageRef) -> PageRef {
        PageRef { chapter: parent.chapter.clone(), index: parent.index, kind: PageKind::Insert { parent_index: parent.index } }
    }

    /// The reference for a core page. `None` when its chapter is gone.
    pub fn from_reader(page: &ReaderPage) -> Option<PageRef> {
        let chapter = page.chapter()?;
        let kind = if page.is_insert_page() { PageKind::Insert { parent_index: page.index } } else { PageKind::Normal };
        Some(PageRef { chapter: chapter.key(), index: page.index, kind })
    }
}

/// Which way a transition points.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dir {
    Prev,
    Next,
}

/// A move between chapters. `to` is `None` at the ends of the book.
#[derive(Clone, Debug)]
pub struct Transition {
    pub dir: Dir,
    pub from: ChapterKey,
    pub to: Option<ChapterKey>,
}

/// `ChapterTransition.equals`: the unordered pair, so `Prev(A, B) == Next(B, A)`. The direction is ignored.
impl PartialEq for Transition {
    fn eq(&self, other: &Self) -> bool {
        (self.from == other.from && self.to == other.to)
            || (other.to.as_ref() == Some(&self.from) && self.to.as_ref() == Some(&other.from))
    }
}

impl Eq for Transition {}

/// Hashes the unordered pair in a fixed order, so equal transitions hash equally.
impl Hash for Transition {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match &self.to {
            None => {
                0u8.hash(state);
                self.from.hash(state);
            }
            Some(to) => {
                1u8.hash(state);
                let (a, b) = if (self.from.id, &self.from.url) <= (to.id, &to.url) { (&self.from, to) } else { (to, &self.from) };
                a.hash(state);
                b.hash(state);
            }
        }
    }
}

/// One row of a viewer's list: a page, two pages side by side (a spread of the page-by-page modes, in reading order) or a
/// transition. Also the key for diffs and `move_to_page` (`ItemKey`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ItemKey {
    Page(PageRef),
    Spread(PageRef, PageRef),
    Transition(Transition),
}

impl ItemKey {
    /// The page of a page row, or the first page (in reading order) of a spread.
    pub fn page(&self) -> Option<&PageRef> {
        match self {
            Self::Page(p) | Self::Spread(p, _) => Some(p),
            Self::Transition(_) => None,
        }
    }

    /// Whether the row shows `page`.
    pub fn shows(&self, page: &PageRef) -> bool {
        match self {
            Self::Page(p) => p == page,
            Self::Spread(a, b) => a == page || b == page,
            Self::Transition(_) => false,
        }
    }

    /// Whether the row is `key`, or shows the page `key` names (a page asked for that sits in a spread now).
    pub fn holds(&self, key: &ItemKey) -> bool {
        self == key || matches!(key, Self::Page(page) if self.shows(page))
    }

    pub fn transition(&self) -> Option<&Transition> {
        match self {
            Self::Transition(t) => Some(t),
            Self::Page(_) | Self::Spread(..) => None,
        }
    }
}

/// The transition gate (B§5.3 steps 4 and 7): added when there is a gap, when forced, or when the neighbour is
/// absent or not loaded.
pub fn transition_wanted(gap: bool, force: bool, neighbour: Option<&ChapterSnap>) -> bool {
    gap || force || neighbour.is_none_or(|n| !n.is_loaded())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::model::ChapterKey;
    use std::collections::hash_map::DefaultHasher;

    fn key(id: i64) -> ChapterKey {
        ChapterKey { id, url: format!("c{id}") }
    }

    fn tr(dir: Dir, from: i64, to: Option<i64>) -> Transition {
        Transition { dir, from: key(from), to: to.map(key) }
    }

    fn hash_of(t: &Transition) -> u64 {
        let mut h = DefaultHasher::new();
        t.hash(&mut h);
        h.finish()
    }

    #[test]
    fn swapped_pairs_are_equal_and_hash_alike() {
        let a = tr(Dir::Prev, 1, Some(2));
        let b = tr(Dir::Next, 2, Some(1));
        assert_eq!(a, b);
        assert_eq!(hash_of(&a), hash_of(&b));
    }

    #[test]
    fn a_missing_destination_matches_only_the_same_source() {
        assert_eq!(tr(Dir::Prev, 1, None), tr(Dir::Next, 1, None));
        assert_ne!(tr(Dir::Next, 1, None), tr(Dir::Next, 1, Some(2)));
        assert_ne!(tr(Dir::Next, 1, None), tr(Dir::Next, 2, None));
        assert_eq!(hash_of(&tr(Dir::Prev, 1, None)), hash_of(&tr(Dir::Next, 1, None)));
    }

    #[test]
    fn item_keys_tell_pages_and_transitions_apart() {
        let page = PageRef { chapter: key(1), index: 0, kind: PageKind::Normal };
        assert_ne!(ItemKey::Page(page.clone()), ItemKey::Transition(tr(Dir::Next, 1, None)));
        assert_eq!(ItemKey::Page(page.clone()).page(), Some(&page));
    }

    #[test]
    fn page_number_is_index_plus_one_and_inserts_are_flagged() {
        let parent = PageRef { chapter: key(1), index: 4, kind: PageKind::Normal };
        let insert = PageRef::insert_of(&parent);
        assert_eq!(parent.number(), 5);
        assert!(insert.is_insert());
        assert_eq!(insert.kind, PageKind::Insert { parent_index: 4 });
        assert_eq!(insert.number(), parent.number());
    }

    #[test]
    fn only_loaded_chapters_expose_pages() {
        let loaded = ChapterSnap { key: key(1), number: 1.0, state: LoadState::Loaded, page_count: 3, requested_page: 0 };
        let loading = ChapterSnap { state: LoadState::Loading, ..loaded.clone() };
        assert_eq!(loaded.pages().len(), 3);
        assert!(loading.pages().is_empty());
    }

    #[test]
    fn a_missing_or_unloaded_neighbour_always_wants_a_transition() {
        let loaded = ChapterSnap { key: key(1), number: 1.0, state: LoadState::Loaded, page_count: 1, requested_page: 0 };
        let waiting = ChapterSnap { state: LoadState::Wait, ..loaded.clone() };
        assert!(transition_wanted(false, false, None));
        assert!(transition_wanted(false, false, Some(&waiting)));
        assert!(!transition_wanted(false, false, Some(&loaded)));
        assert!(transition_wanted(false, true, Some(&loaded)));
        assert!(transition_wanted(true, false, Some(&loaded)));
    }
}
