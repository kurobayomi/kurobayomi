//! The page viewers (area 44). The reader core calls a viewer only through [`ViewerEngine`]; a viewer reports back
//! through the core's [`ViewerEvent`]. Each viewer is split in two: a pure controller (`pager::controller`,
//! `webtoon::controller`) that decides what happens and returns [`effect::Effect`]s, and a thin GTK engine that
//! carries them out. Everything decided here is unit-tested without GTK.
//!
//! `make_viewer` builds the engine for a reading mode (B§0). `ViewerKind::from_flag` rejects the unresolved mode 0.

pub mod adapter;
pub mod downloads;
pub mod effect;
pub mod engine;
pub mod gap;
pub mod gesture;
pub mod holder;
pub mod items;
pub mod keys;
pub mod model;
pub mod nav;
pub mod page_bytes;
pub mod page_opts;
pub mod pager;
pub mod settings;
pub mod status;
pub mod visible;
pub mod webtoon;

use std::cell::Cell;
use std::fmt;
use std::rc::Rc;

use crate::reader::model::{ReaderPage, ViewerChapters};
use crate::reader::viewer::ViewerEvent;
use keys::KeyEvent;
use kuro_prefs::reader::MangaReaderPrefs;
use settings::ViewerSettings;

/// The five reading modes a viewer is built for (B§0, flag values 1..5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewerKind {
    LeftToRight,
    RightToLeft,
    Vertical,
    Webtoon,
    WebtoonGapped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewerError {
    /// Flag value 0 (the default) must be resolved to a concrete mode before a viewer is built.
    UnresolvedMode(i64),
}

impl fmt::Display for ViewerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnresolvedMode(v) => write!(f, "Preference value must be resolved: {v}"),
        }
    }
}

impl std::error::Error for ViewerError {}

impl ViewerKind {
    /// The viewer for a reading-mode value (`viewer_flags & 0x7`): 1 L2R, 2 R2L, 3 vertical, 4 webtoon,
    /// 5 webtoon with gaps. Any other value, including 0, is an error.
    pub fn from_flag(flag: i64) -> Result<Self, ViewerError> {
        match flag & 0x7 {
            1 => Ok(Self::LeftToRight),
            2 => Ok(Self::RightToLeft),
            3 => Ok(Self::Vertical),
            4 => Ok(Self::Webtoon),
            5 => Ok(Self::WebtoonGapped),
            _ => Err(ViewerError::UnresolvedMode(flag)),
        }
    }

    /// Whether the viewer is a paged strip (L2R, R2L, vertical) rather than the webtoon.
    pub fn is_pager(self) -> bool {
        matches!(self, Self::LeftToRight | Self::RightToLeft | Self::Vertical)
    }
}

/// The calls the core makes on a viewer. `widget` is the viewer's root, which the container shows.
pub trait ViewerEngine {
    fn kind(&self) -> ViewerKind;
    fn widget(&self) -> &gtk::Widget;
    /// Hands over the current, previous and next chapters. The viewer keeps what it needs.
    fn set_chapters(&self, chapters: ViewerChapters);
    /// Shows `page`, as the saved page, the slider and the chapter buttons do.
    fn move_to_page(&self, page: &Rc<ReaderPage>);
    /// A key the viewer owns, on either edge. `true` when it claimed the key.
    fn handle_key(&self, ev: KeyEvent) -> bool;
    /// A pointer scroll, `dy > 0` for scrolling down. `true` when the viewer used it.
    fn handle_scroll(&self, dy: f64) -> bool;
    /// Releases everything. No event is sent after this (E-78). A viewer with nothing to release keeps the default.
    fn destroy(&self) {}
}

/// Sends viewer events to the core, and nothing once the viewer is destroyed (E-78).
#[derive(Clone)]
pub struct Emitter {
    sink: Rc<dyn Fn(ViewerEvent)>,
    alive: Rc<Cell<bool>>,
}

impl Emitter {
    pub fn new(sink: Rc<dyn Fn(ViewerEvent)>) -> Self {
        Self { sink, alive: Rc::new(Cell::new(true)) }
    }

    pub fn send(&self, event: ViewerEvent) {
        if self.alive.get() {
            (self.sink)(event);
        }
    }

    pub fn close(&self) {
        self.alive.set(false);
    }

    pub fn is_open(&self) -> bool {
        self.alive.get()
    }
}

/// What a new viewer starts from.
pub struct ViewerContext {
    pub emit: Emitter,
    pub settings: ViewerSettings,
    /// The store the settings came from. With it the viewer follows later changes (T-44a-005); without it the
    /// settings stay as given.
    pub prefs: Option<MangaReaderPrefs>,
    /// The download index, for the transition cards' downloaded flags (T-43b-032).
    pub downloads: downloads::DownloadCheck,
    /// Whether the reader menu is shown. The screen owns it and sets it on each toggle; the long tap and the volume
    /// keys read it (E-21, E-25).
    pub menu_open: Rc<Cell<bool>>,
}

/// The engine for a reading mode (B§0): the pager for the three paged modes, the webtoon for the two strips.
pub fn make_viewer(kind: ViewerKind, ctx: ViewerContext) -> Box<dyn ViewerEngine> {
    match kind {
        ViewerKind::LeftToRight | ViewerKind::RightToLeft | ViewerKind::Vertical => Box::new(engine::PagerEngine::new(kind, ctx)),
        ViewerKind::Webtoon => Box::new(engine::WebtoonEngine::new(false, ctx)),
        ViewerKind::WebtoonGapped => Box::new(engine::WebtoonEngine::new(true, ctx)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::prelude::Cast;

    #[test]
    fn reading_mode_values_map_to_the_five_viewers() {
        assert_eq!(ViewerKind::from_flag(1), Ok(ViewerKind::LeftToRight));
        assert_eq!(ViewerKind::from_flag(2), Ok(ViewerKind::RightToLeft));
        assert_eq!(ViewerKind::from_flag(3), Ok(ViewerKind::Vertical));
        assert_eq!(ViewerKind::from_flag(4), Ok(ViewerKind::Webtoon));
        assert_eq!(ViewerKind::from_flag(5), Ok(ViewerKind::WebtoonGapped));
    }

    #[test]
    fn the_unresolved_default_and_unknown_modes_are_rejected() {
        assert_eq!(ViewerKind::from_flag(0), Err(ViewerError::UnresolvedMode(0)));
        assert!(ViewerKind::from_flag(6).is_err());
        // Other flag bits do not change the mode.
        assert_eq!(ViewerKind::from_flag(0x38 | 2), Ok(ViewerKind::RightToLeft));
    }

    #[test]
    fn the_family_split_is_pager_then_webtoon() {
        assert!(ViewerKind::Vertical.is_pager());
        assert!(!ViewerKind::WebtoonGapped.is_pager());
    }

    /// A minimal viewer: it claims the right arrow key and records what it was sent (T-43b-034).
    struct Probe {
        root: gtk::Box,
        keys: Cell<u32>,
    }

    impl ViewerEngine for Probe {
        fn kind(&self) -> ViewerKind {
            ViewerKind::LeftToRight
        }
        fn widget(&self) -> &gtk::Widget {
            self.root.upcast_ref()
        }
        fn set_chapters(&self, _chapters: ViewerChapters) {}
        fn move_to_page(&self, _page: &Rc<ReaderPage>) {}
        fn handle_key(&self, ev: KeyEvent) -> bool {
            self.keys.set(self.keys.get() + 1);
            ev.key == keys::InputKey::Right
        }
        fn handle_scroll(&self, _dy: f64) -> bool {
            false
        }
    }

    #[test]
    fn a_test_viewer_receives_a_key_and_reports_it_handled() {
        crate::testing::with_display(|| {
            let viewer: Box<dyn ViewerEngine> = Box::new(Probe { root: gtk::Box::new(gtk::Orientation::Vertical, 0), keys: Cell::new(0) });
            assert!(viewer.handle_key(KeyEvent { key: keys::InputKey::Right, ctrl: false, released: false }));
            assert!(!viewer.handle_key(KeyEvent { key: keys::InputKey::Left, ctrl: false, released: false }));
            viewer.destroy();
            assert_eq!(viewer.kind(), ViewerKind::LeftToRight);
        });
    }

    #[test]
    fn an_emitter_sends_until_it_is_closed() {
        let seen = Rc::new(Cell::new(0));
        let counter = Rc::clone(&seen);
        let emit = Emitter::new(Rc::new(move |_| counter.set(counter.get() + 1)));
        emit.send(ViewerEvent::ToggleMenu);
        emit.close();
        emit.send(ViewerEvent::ToggleMenu);
        assert_eq!(seen.get(), 1);
    }
}
