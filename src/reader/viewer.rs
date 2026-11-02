//! The viewer contract (T-043a-087). The reader core calls a viewer only through the five methods of [`Viewer`];
//! a viewer reports back through [`ViewerEvent`]. The viewer code itself belongs to area 44 (`reader-viewers`).
//!
//! [`swap`] is the one place a viewer is replaced: the previous viewer is destroyed and the container is cleared
//! before the new viewer is attached.

use std::rc::Rc;

use super::model::{ReaderChapter, ReaderPage, ViewerChapters};

/// The calls the core makes on a viewer.
pub trait Viewer {
    /// Hands over the current, previous and next chapters. Replaces what the viewer showed before.
    fn set_chapters(&self, chapters: &ViewerChapters);
    /// Shows `page`. The core uses it for the saved page, the slider and the chapter buttons.
    fn move_to_page(&self, page: &Rc<ReaderPage>);
    /// Releases everything the viewer holds. Called once, before a replacement viewer is attached.
    fn destroy(&self);
    /// A key press the viewer owns (arrow and page keys). `true` when it consumed the key.
    fn handle_key(&self, key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool;
    #[cfg(test)]
    /// A scroll or pointer motion the viewer owns, as deltas. `true` when it consumed the event.
    fn handle_motion(&self, dx: f64, dy: f64) -> bool;
}

/// What a viewer reports back. Delivered on the GTK thread, in order.
#[derive(Clone)]
pub enum ViewerEvent {
    /// A page became the visible one.
    PageSelected(Rc<ReaderPage>),
    /// A chapter transition card became the visible item (no page is on screen).
    TransitionShown,
    /// A tap toggled the chrome.
    ToggleMenu,
    ShowMenu,
    HideMenu,
    /// The viewer wants the next chapter loaded in the background.
    RequestPreload(Rc<ReaderChapter>),
    /// A long press (or right-click) on a page.
    LongTap(Rc<ReaderPage>),
}

/// The viewer currently attached to the reader's container, if any.
pub type ViewerSlot = Option<Box<dyn Viewer>>;

/// Replaces the viewer in `slot` with `next`. The order is fixed: destroy the previous viewer, clear the
/// container, attach the new one, then store it.
pub fn swap(slot: &mut ViewerSlot, next: Box<dyn Viewer>, clear_container: impl FnOnce(), attach: impl FnOnce(&dyn Viewer)) {
    if let Some(previous) = slot.take() {
        previous.destroy();
    }
    clear_container();
    attach(next.as_ref());
    *slot = Some(next);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Clone)]
    struct Log(Rc<RefCell<Vec<String>>>, &'static str);

    impl Viewer for Log {
        fn set_chapters(&self, _: &ViewerChapters) {}
        fn move_to_page(&self, _: &Rc<ReaderPage>) {}
        fn destroy(&self) {
            self.0.borrow_mut().push(format!("destroy {}", self.1));
        }
        fn handle_key(&self, _: gtk::gdk::Key, _: gtk::gdk::ModifierType) -> bool {
            false
        }
        fn handle_motion(&self, _: f64, _: f64) -> bool {
            false
        }
    }

    #[test]
    fn swap_destroys_the_old_viewer_then_clears_then_attaches_the_new_one() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut slot: ViewerSlot = Some(Box::new(Log(Rc::clone(&log), "old")));
        let (a, b) = (Rc::clone(&log), Rc::clone(&log));
        swap(
            &mut slot,
            Box::new(Log(Rc::clone(&log), "new")),
            move || a.borrow_mut().push("clear".into()),
            move |_| b.borrow_mut().push("attach".into()),
        );
        assert_eq!(log.borrow().as_slice(), ["destroy old", "clear", "attach"]);
        assert!(slot.is_some());
    }

    #[test]
    fn the_first_viewer_has_nothing_to_destroy_but_still_clears_and_attaches() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut slot: ViewerSlot = None;
        let (a, b) = (Rc::clone(&log), Rc::clone(&log));
        swap(
            &mut slot,
            Box::new(Log(Rc::clone(&log), "only")),
            move || a.borrow_mut().push("clear".into()),
            move |_| b.borrow_mut().push("attach".into()),
        );
        assert_eq!(log.borrow().as_slice(), ["clear", "attach"]);
    }
}
