//! The adapter from a viewer engine to the core's viewer contract (T-44a-070). The reader core talks to a
//! [`crate::reader::viewer::Viewer`]; the engines of area 44 are [`ViewerEngine`]s. [`EngineViewer`] is the one place
//! the two meet: the core swaps one adapter for another (`viewer::swap`), which destroys the old engine before the new
//! one is attached.

use std::cell::Cell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use kuro_prefs::reader::ReadingMode;

use super::engine::input_key;
use super::keys::KeyEvent;
use super::{ViewerEngine, ViewerKind};
use crate::reader::model::{ReaderPage, ViewerChapters};
use crate::reader::viewer::Viewer;

/// The viewer for a resolved reading mode. The unresolved default never gets here (the reader resolves it first); it
/// reads left to right, the default preference.
pub fn kind_of(mode: ReadingMode) -> ViewerKind {
    ViewerKind::from_flag(i64::from(mode.flag_value())).unwrap_or(ViewerKind::LeftToRight)
}

/// An engine behind the core's [`Viewer`] contract. The engine's widget is added to `container` with the first
/// chapters and removed again when the viewer is destroyed.
pub struct EngineViewer {
    engine: Box<dyn ViewerEngine>,
    container: gtk::Box,
    destroyed: Cell<bool>,
}

impl EngineViewer {
    pub fn new(engine: Box<dyn ViewerEngine>, container: gtk::Box) -> Self {
        Self { engine, container, destroyed: Cell::new(false) }
    }

    fn attach_widget(&self) {
        let widget = self.engine.widget();
        if widget.parent().is_none() {
            self.container.append(widget);
        }
    }
}

impl Viewer for EngineViewer {
    fn set_chapters(&self, chapters: &ViewerChapters) {
        if self.destroyed.get() {
            return;
        }
        self.attach_widget();
        self.engine.set_chapters(chapters.clone());
    }

    fn move_to_page(&self, page: &Rc<ReaderPage>) {
        if !self.destroyed.get() {
            self.engine.move_to_page(page);
        }
    }

    /// Cancels the loads, drops the pages and sends nothing afterwards (E-77, E-78). Safe to call twice.
    fn destroy(&self) {
        if self.destroyed.replace(true) {
            return;
        }
        self.engine.destroy();
        let widget = self.engine.widget();
        if widget.parent().is_some_and(|p| p == *self.container.upcast_ref::<gtk::Widget>()) {
            self.container.remove(widget);
        }
    }

    fn handle_key(&self, key: gdk::Key, modifiers: gdk::ModifierType) -> bool {
        !self.destroyed.get()
            && self.engine.handle_key(KeyEvent {
                key: input_key(&key, modifiers.contains(gdk::ModifierType::SHIFT_MASK)),
                ctrl: modifiers.contains(gdk::ModifierType::CONTROL_MASK),
                released: false,
            })
    }

    #[cfg(test)]
    fn handle_motion(&self, _dx: f64, dy: f64) -> bool {
        !self.destroyed.get() && self.engine.handle_scroll(dy)
    }
}

impl Drop for EngineViewer {
    fn drop(&mut self) {
        self.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::model::ReaderChapter;
    use crate::reader::viewers::settings::ViewerSettings;
    use crate::reader::viewers::{Emitter, ViewerContext, make_viewer};
    use kuro_data::item::Item;
    use kuro_platform::kind::Kind;
    use kuro_prefs::reader::ReaderPrefs;
    use kuro_prefs::store::PrefStore;
    use std::cell::RefCell;

    fn context(events: Rc<RefCell<Vec<&'static str>>>) -> ViewerContext {
        let sink = Rc::new(move |event: crate::reader::viewer::ViewerEvent| {
            events.borrow_mut().push(match event {
                crate::reader::viewer::ViewerEvent::PageSelected(_) => "selected",
                crate::reader::viewer::ViewerEvent::ToggleMenu => "toggle",
                _ => "other",
            });
        });
        ViewerContext { emit: Emitter::new(sink), settings: ViewerSettings::from_prefs(&ReaderPrefs::new(PrefStore::in_memory())), prefs: None, downloads: Default::default(), menu_open: Default::default() }
    }

    fn chapters() -> ViewerChapters {
        ViewerChapters { curr: ReaderChapter::new(Item::create(Kind::Manga)), prev: None, next: None }
    }

    #[test]
    fn each_reading_mode_picks_its_viewer() {
        assert_eq!(kind_of(ReadingMode::LeftToRight), ViewerKind::LeftToRight);
        assert_eq!(kind_of(ReadingMode::RightToLeft), ViewerKind::RightToLeft);
        assert_eq!(kind_of(ReadingMode::Vertical), ViewerKind::Vertical);
        assert_eq!(kind_of(ReadingMode::Webtoon), ViewerKind::Webtoon);
        assert_eq!(kind_of(ReadingMode::ContinuousVertical), ViewerKind::WebtoonGapped);
        assert_eq!(kind_of(ReadingMode::Default), ViewerKind::LeftToRight, "an unresolved mode is the default preference");
    }

    #[test]
    fn the_engine_widget_goes_into_the_container_with_the_chapters_and_out_on_destroy() {
        crate::testing::with_display(|| {
            for kind in [ViewerKind::LeftToRight, ViewerKind::Webtoon] {
                let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
                let viewer = EngineViewer::new(make_viewer(kind, context(Rc::default())), container.clone());
                assert!(container.first_child().is_none(), "nothing is shown before the chapters arrive");
                viewer.set_chapters(&chapters());
                assert!(container.first_child().is_some(), "{kind:?}");
                viewer.set_chapters(&chapters());
                assert_eq!(container.observe_children().n_items(), 1, "a second hand-over adds nothing");
                viewer.destroy();
                assert!(container.first_child().is_none());
                viewer.destroy();
            }
        });
    }

    #[test]
    fn a_destroyed_viewer_claims_no_key_and_sends_no_event() {
        crate::testing::with_display(|| {
            let events = Rc::new(RefCell::new(Vec::new()));
            let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let viewer = EngineViewer::new(make_viewer(ViewerKind::RightToLeft, context(Rc::clone(&events))), container);
            viewer.set_chapters(&chapters());
            assert!(viewer.handle_key(gdk::Key::Right, gdk::ModifierType::empty()), "an arrow key is the viewer's");
            assert!(!viewer.handle_key(gdk::Key::a, gdk::ModifierType::empty()));
            events.borrow_mut().clear();
            viewer.destroy();
            assert!(!viewer.handle_key(gdk::Key::Right, gdk::ModifierType::empty()));
            assert!(!viewer.handle_motion(0.0, 1.0));
            assert!(events.borrow().is_empty());
        });
    }

    #[test]
    fn dropping_the_adapter_destroys_the_engine() {
        crate::testing::with_display(|| {
            let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
            {
                let viewer = EngineViewer::new(make_viewer(ViewerKind::Vertical, context(Rc::default())), container.clone());
                viewer.set_chapters(&chapters());
                assert!(container.first_child().is_some());
            }
            assert!(container.first_child().is_none());
        });
    }
}
