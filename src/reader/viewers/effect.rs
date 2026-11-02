//! The work a viewer's state machine asks the view and the engine to do (shared by the pager and the webtoon).

use super::model::{ChapterKey, PageRef};
use super::pager::nav::Phys;

/// Work for the view and the engine, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// The item list changed; rebuild the slots and keep the current page.
    Rebuilt,
    /// The strip becomes visible after the first layout.
    Show,
    /// Move the strip to item `index`. The view reports `on_settled` when it arrives.
    ScrollTo { index: usize, smooth: bool },
    /// Pan the current page one viewport width.
    Pan(Phys),
    /// A page became the visible one (`ViewerEvent::PageSelected`).
    PageSelected(PageRef),
    /// The page selected last shows beside this one, a two-page spread (`ViewerEvent::ShownBeside`).
    ShownBeside(PageRef),
    /// The page's zoom-start hook (B§10.3). `forward` is the selection direction.
    ZoomStart { page: PageRef, forward: bool },
    /// A chapter transition card became the visible item; the page counter shows no number on it.
    TransitionSelected,
    HideMenu,
    ShowMenu,
    /// Ask for a chapter to load in the background.
    Preload(ChapterKey),
}
