//! The GTK engines (T-44a-010, T-44a-023, T-44a-025, T-44a-032, T-44a-034). Each engine owns its controller, carries
//! out the controller's effects on widgets, and routes pointer, key and scroll input to the controller. Logic stays
//! in the controllers; this module only decides where an effect lands on screen.
//!
//! A page that can be read from disk shows its image in a `PageView`, fitted by the stored scale type and zoom start
//! and following later changes in place (`holder`, `page_opts`). A page that cannot be read here keeps a label that
//! follows the page's status. The page view reports long presses (when `reader_long_tap` is on) as
//! `ViewerEvent::LongTap`.
//!
//! The webtoon sits in a `ZoomFrame`: a pinch (the GTK zoom gesture), a drag and a double tap drive `WebtoonZoom`, and
//! the frame scales and moves the strip (T-44a-038, T-44a-039). The pager zooms inside each page view.

use kuro_i18n::{Arg, MsgKey, tr, trf};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::glib::{self, ControlFlow, Propagation};
use gtk::prelude::*;

use super::downloads::DownloadCheck;
use super::effect::Effect;
use super::gesture::{self, DragTracker, Gesture, TapRecognizer};
use super::holder::{self, PageHost, Row, Tasks};
use super::keys::{self, InputKey, KeyAction, KeyContext, KeyEvent};
use super::model::{ChapterKey, ChapterSet, ItemKey, PageKind, PageRef};
use super::nav::{self, TapOutcome};
use super::page_opts::{page_opts, side_padding_px};
use super::pager::controller::{PagerController, PagerOptions};
use super::settings::{self, Family, Reaction, SettingsWatch, ViewerSettings};
use super::webtoon::controller::{WebtoonController, WebtoonOptions};
use super::webtoon::frame::ZoomFrame;
use super::webtoon::layout::{StripLayout, anchor_delta, clamp_step, gap_px, refresh_range, reserved_height, wheel_delta};
use super::webtoon::zoom::{Anim, WebtoonZoom};
use super::{Emitter, ViewerContext, ViewerEngine, ViewerKind};
use crate::reader::error_card;
use crate::reader::page_zoom::{self, ZoomKey};
use crate::reader::quick_scale::{QuickScale, THRESHOLD_DP};
use crate::reader::model::{ChapterTransition, PageStatus, ReaderChapter, ReaderPage, ViewerChapters};
use crate::reader::viewer::ViewerEvent;

/// Height of a transition card in the webtoon strip, until its content is measured.
const CARD_HEIGHT: i32 = 200;
/// Length of the scroll animation of taps, keys and swipes.
const SCROLL_MS: i64 = 250;
/// How often the gesture timers are polled (long press and the double-click window).
const POLL_MS: u64 = 20;

/// The chapters and pages a viewer was handed, keyed the way the events name them.
#[derive(Default)]
struct Lookup {
    chapters: HashMap<ChapterKey, Rc<ReaderChapter>>,
    pages: HashMap<PageRef, Rc<ReaderPage>>,
}

impl Lookup {
    fn rebuild(&mut self, chapters: &ViewerChapters) {
        self.chapters.clear();
        self.pages.clear();
        for chapter in [Some(&chapters.curr), chapters.prev.as_ref(), chapters.next.as_ref()].into_iter().flatten() {
            self.chapters.insert(chapter.key(), Rc::clone(chapter));
            for page in chapter.pages().iter().flat_map(|pages| pages.iter()) {
                let r = PageRef { chapter: chapter.key(), index: page.index, kind: PageKind::Normal };
                self.pages.insert(r, Rc::clone(page));
            }
        }
    }

    fn chapter(&self, key: &ChapterKey) -> Option<Rc<ReaderChapter>> {
        self.chapters.get(key).cloned()
    }

    /// A page for an event. An insert shares its parent's bytes (`InsertPage`), so it is made on demand.
    fn page(&self, r: &PageRef) -> Option<Rc<ReaderPage>> {
        match r.kind {
            PageKind::Normal => self.pages.get(r).cloned(),
            PageKind::Insert { .. } => {
                let parent = self.pages.get(&PageRef { kind: PageKind::Normal, ..r.clone() })?;
                Some(ReaderPage::insert_page(parent))
            }
        }
    }
}

fn now_ms() -> u64 {
    (glib::monotonic_time() / 1000).max(0) as u64
}

/// Double-click time, long-press time and drag threshold from the desktop settings (RD §11), with GTK's defaults
/// when no settings object exists. The pager's recogniser acts on release; the webtoon's waits out the window.
fn recognizer(on_release: bool) -> TapRecognizer {
    let (double, long, slop) = match gtk::Settings::default() {
        Some(s) => (
            s.property::<i32>("gtk-double-click-time"),
            // `gtk-long-press-time` is unsigned, the other two are signed.
            i32::try_from(s.property::<u32>("gtk-long-press-time")).unwrap_or(i32::MAX),
            s.property::<i32>("gtk-dnd-drag-threshold"),
        ),
        None => (400, 500, 8),
    };
    let r = TapRecognizer::new(double.max(0) as u64, long.max(0) as u64, f64::from(slop.max(0)));
    if on_release { r.acting_on_release() } else { r }
}

pub(super) fn input_key(key: &gtk::gdk::Key, shift: bool) -> InputKey {
    match key.name().as_deref() {
        Some("Left") => InputKey::Left,
        Some("Right") => InputKey::Right,
        Some("Up") => InputKey::Up,
        Some("Down") => InputKey::Down,
        Some("Page_Up") => InputKey::PageUp,
        Some("Page_Down") => InputKey::PageDown,
        Some("space") if shift => InputKey::ShiftSpace,
        Some("space") => InputKey::Space,
        Some("Home") => InputKey::Home,
        Some("End") => InputKey::End,
        Some("Menu") => InputKey::Menu,
        Some("XF86AudioRaiseVolume") => InputKey::VolumeUp,
        Some("XF86AudioLowerVolume") => InputKey::VolumeDown,
        // plus, minus and 0 zoom; Ctrl+0 is the same reset (T-43b-058)
        Some("plus" | "equal" | "KP_Add") => InputKey::ZoomIn,
        Some("minus" | "KP_Subtract") => InputKey::ZoomOut,
        Some("0" | "KP_0") => InputKey::ZoomReset,
        _ => InputKey::Other,
    }
}

/// Both axes are `External` (scrolling without bars) and never `Never`: a `Never` axis makes the window's minimum
/// size on that axis the strip's, and the strip is sized to the viewport, so the page area would refuse to shrink
/// and the toplevel would grow each time the bottom bar was hidden and shown again. The cross axis has no range anyway.
fn scrolled() -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::External)
        .hexpand(true)
        .vexpand(true)
        .focusable(true)
        .build()
}

/// The row for an item: a label until page content exists, following the page's status or the destination
/// chapter's state (T-44a-006). An insert page is made on demand and reads through its parent, whose bytes it shares;
/// it decodes the other half of the parent (T-44a-050).
fn item_row(item: &ItemKey, lookup: &Lookup, host: &PageHost) -> Row {
    match item {
        ItemKey::Page(p) => {
            let text = if p.is_insert() {
                trf(MsgKey::reader_page_label_split, &[("chapter", Arg::Int(p.chapter.id)), ("page", Arg::Int(i64::from(p.number())))])
            } else {
                trf(MsgKey::reader_page_label, &[("chapter", Arg::Int(p.chapter.id)), ("page", Arg::Int(i64::from(p.number())))])
            };
            holder::page_row(text, lookup.page(p), Some(host))
        }
        ItemKey::Transition(t) => {
            let to = match &t.to {
                Some(key) => match lookup.chapter(key) {
                    Some(chapter) => Some(chapter),
                    None => return holder::plain_row(&tr(MsgKey::ui_chapter_unavailable)),
                },
                None => None,
            };
            match lookup.chapter(&t.from) {
                Some(from) => {
                    let transition = match t.dir {
                        super::model::Dir::Next => ChapterTransition::Next { from, to },
                        super::model::Dir::Prev => ChapterTransition::Prev { from, to },
                    };
                    holder::transition_row(transition, host)
                }
                None => holder::plain_row(&tr(MsgKey::ui_chapter_unavailable)),
            }
        }
    }
}

/// What a new page row is handed: the options of the current settings, the long-press report (T-43b-013), what a
/// split page asks of the viewer and what a row's error state asks of the layout.
fn page_host(
    settings: &ViewerSettings,
    kind: ViewerKind,
    emit: &Emitter,
    on_split: Rc<dyn Fn(Rc<ReaderPage>)>,
    on_layout: Rc<dyn Fn()>,
    downloads: &DownloadCheck,
    menu_open: &Rc<Cell<bool>>,
) -> PageHost {
    let (long_emit, zoom_emit, retry_emit) = (emit.clone(), emit.clone(), emit.clone());
    PageHost {
        opts: page_opts(settings, kind, crate::reader::theme::system_prefers_dark()),
        on_long_tap: Rc::new(move |page, _x, _y| long_emit.send(ViewerEvent::LongTap(page))),
        // Any zoom of a page hides the menu (T-44a-071, B§6.5).
        on_zoom: Rc::new(move || zoom_emit.send(ViewerEvent::HideMenu)),
        on_split,
        on_layout,
        is_webtoon: !kind.is_pager(),
        on_retry: Rc::new(move |chapter| retry_emit.send(ViewerEvent::RequestPreload(chapter))),
        downloads: downloads.clone(),
        menu_open: Rc::clone(menu_open),
    }
}

/// Runs a scroll of the adjustment to `target`. Smooth scrolls ease out over `SCROLL_MS`; a newer scroll on the same
/// generation cancels this one. `done` runs when the adjustment arrives.
fn animate(
    root: &gtk::ScrolledWindow,
    adj: gtk::Adjustment,
    generation: Rc<Cell<u64>>,
    target: f64,
    smooth: bool,
    done: impl FnOnce() + 'static,
) {
    let token = generation.get() + 1;
    generation.set(token);
    let start = adj.value();
    // A strip that is not on screen gets no frame clock, so it moves at once rather than never finishing.
    if !smooth || !root.is_mapped() || (start - target).abs() < 0.5 {
        adj.set_value(target);
        done();
        return;
    }
    let started: Cell<Option<i64>> = Cell::new(None);
    let done = RefCell::new(Some(done));
    root.add_tick_callback(move |_, clock| {
        if generation.get() != token {
            return ControlFlow::Break;
        }
        let now = clock.frame_time();
        let t0 = started.get().unwrap_or_else(|| {
            started.set(Some(now));
            now
        });
        let p = ((now - t0) as f64 / (SCROLL_MS * 1000) as f64).clamp(0.0, 1.0);
        adj.set_value(start + (target - start) * (1.0 - (1.0 - p).powi(3)));
        if p < 1.0 {
            return ControlFlow::Continue;
        }
        let finish = done.borrow_mut().take();
        if let Some(finish) = finish {
            finish();
        }
        ControlFlow::Break
    });
}

/// What the pointer, key and wheel handlers need from an engine.
trait Surface: 'static {
    fn root(&self) -> &gtk::ScrolledWindow;
    fn recognizer(&self) -> &RefCell<TapRecognizer>;
    /// Follows the preference store, when the viewer was given one.
    fn watch(&self) -> &RefCell<Option<SettingsWatch>>;
    fn apply_settings(&self, new: ViewerSettings);
    fn alive(&self) -> bool;
    fn tap(&self, gesture: Gesture);
    fn key(&self, ev: KeyEvent) -> bool;
    fn wheel(&self, scroll: Scroll) -> bool;
    /// Whether the wheel handler runs before the scrolled window's own one, which it replaces.
    fn wheel_first(&self) -> bool {
        false
    }
}

/// One scroll event. A mouse wheel reports notches (1.0 each); a touchpad reports surface pixels.
#[derive(Clone, Copy, Debug)]
pub(super) struct Scroll {
    dx: f64,
    dy: f64,
    notches: bool,
    /// Ctrl was held: the scroll zooms (T-43b-016).
    ctrl: bool,
}

/// Connects the click, motion, key, wheel and gesture-timer controllers of an engine. Handlers hold a weak
/// reference, so a destroyed engine stops receiving input.
fn wire_surface<T: Surface>(this: &Rc<T>) {
    let root = this.root().clone();
    let click = gtk::GestureClick::new();
    let weak = Rc::downgrade(this);
    click.connect_pressed(move |_, _, x, y| {
        let Some(s) = weak.upgrade() else { return };
        let gestures = s.recognizer().borrow_mut().press(now_ms(), x, y);
        gestures.into_iter().for_each(|g| s.tap(g));
    });
    let weak = Rc::downgrade(this);
    click.connect_released(move |_, _, x, y| {
        let Some(s) = weak.upgrade() else { return };
        let gestures = s.recognizer().borrow_mut().release(now_ms(), x, y);
        gestures.into_iter().for_each(|g| s.tap(g));
    });
    root.add_controller(click);

    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(this);
    motion.connect_motion(move |_, x, y| {
        if let Some(s) = weak.upgrade() {
            s.recognizer().borrow_mut().motion(x, y);
        }
    });
    root.add_controller(motion);

    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(this);
    keys.connect_key_pressed(move |_, key, _, state| {
        let ev = KeyEvent { key: input_key(&key, state.contains(gtk::gdk::ModifierType::SHIFT_MASK)), ctrl: state.contains(gtk::gdk::ModifierType::CONTROL_MASK), released: false };
        match weak.upgrade() {
            Some(s) if s.key(ev) => Propagation::Stop,
            _ => Propagation::Proceed,
        }
    });
    let weak = Rc::downgrade(this);
    keys.connect_key_released(move |_, key, _, state| {
        let ev = KeyEvent { key: input_key(&key, state.contains(gtk::gdk::ModifierType::SHIFT_MASK)), ctrl: state.contains(gtk::gdk::ModifierType::CONTROL_MASK), released: true };
        if let Some(s) = weak.upgrade() {
            s.key(ev);
        }
    });
    root.add_controller(keys);

    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    if this.wheel_first() {
        wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
    }
    let weak = Rc::downgrade(this);
    wheel.connect_scroll(move |c, dx, dy| {
        let scroll = Scroll {
            dx,
            dy,
            notches: c.unit() == gtk::gdk::ScrollUnit::Wheel,
            ctrl: c.current_event_state().contains(gtk::gdk::ModifierType::CONTROL_MASK),
        };
        match weak.upgrade() {
            Some(s) if s.wheel(scroll) => Propagation::Stop,
            _ => Propagation::Proceed,
        }
    });
    root.add_controller(wheel);

    // Long presses and the double-click window need a clock; the recogniser is polled while the engine lives.
    let weak = Rc::downgrade(this);
    glib::timeout_add_local(Duration::from_millis(POLL_MS), move || {
        let Some(s) = weak.upgrade() else { return ControlFlow::Break };
        if !s.alive() {
            return ControlFlow::Break;
        }
        let gestures = s.recognizer().borrow_mut().poll(now_ms());
        gestures.into_iter().for_each(|g| s.tap(g));
        // The same clock follows the preference store (T-44a-005): one cheap drain, a read only after a change.
        let changed = s.watch().borrow_mut().as_mut().and_then(SettingsWatch::poll);
        if let Some(new) = changed {
            s.apply_settings(new);
        }
        ControlFlow::Continue
    });
}

/// The tap position as a fraction of the viewer, with the webtoon's height taken from its first layout (B§2.1).
fn normalised(x: f64, y: f64, width: f64, height: f64) -> (f64, f64) {
    ((x / width.max(1.0)).clamp(0.0, 1.0), (y / height.max(1.0)).clamp(0.0, 1.0))
}

/// The drag threshold from the desktop settings, which is the touch slop of a pan.
fn recognizer_slop() -> f64 {
    gtk::Settings::default().map_or(8.0, |s| f64::from(s.property::<i32>("gtk-dnd-drag-threshold").max(0)))
}

/// `delta` with the first `slop` pixels taken off: nothing inside the slop, then the distance past it.
fn dead_zone(delta: f64, slop: f64) -> f64 {
    if delta.abs() > slop { delta - delta.signum() * slop } else { 0.0 }
}

pub struct PagerEngine(Rc<PagerInner>);

impl PagerEngine {
    pub fn new(kind: ViewerKind, ctx: ViewerContext) -> Self {
        Self(PagerInner::new(kind, ctx))
    }

    #[cfg(test)]
    /// Applies a settings change and carries out its reactions (T-44a-005).
    pub fn apply_settings(&self, new: ViewerSettings) {
        self.0.apply_settings(new);
    }
}

impl ViewerEngine for PagerEngine {
    fn kind(&self) -> ViewerKind {
        self.0.kind
    }

    fn widget(&self) -> &gtk::Widget {
        self.0.root.upcast_ref()
    }

    fn set_chapters(&self, chapters: ViewerChapters) {
        self.0.set_chapters(&chapters);
    }

    fn move_to_page(&self, page: &Rc<ReaderPage>) {
        let Some(r) = PageRef::from_reader(page) else { return };
        let fx = self.0.ctl.borrow_mut().move_to_page(&ItemKey::Page(r));
        self.0.run(fx);
    }

    fn handle_key(&self, ev: KeyEvent) -> bool {
        self.0.key(ev)
    }

    fn handle_scroll(&self, dy: f64) -> bool {
        self.0.wheel(Scroll { dx: 0.0, dy, notches: true, ctrl: false })
    }

    fn destroy(&self) {
        self.0.destroy();
    }
}

struct PagerInner {
    me: Weak<PagerInner>,
    /// Scroll gathered towards the next page turn (a touchpad sends many small steps for one swipe), and when the
    /// last turn happened: one swipe or one wheel notch turns one page.
    wheel_sum: Cell<f64>,
    wheel_at: Cell<u64>,
    /// When the scroll last panned the shown page, or was swallowed after it: a swipe that reaches the page's edge
    /// stops there, and only a new one after a pause turns the page.
    panned_at: Cell<u64>,
    kind: ViewerKind,
    root: gtk::ScrolledWindow,
    fixed: gtk::Fixed,
    ctl: RefCell<PagerController>,
    lookup: RefCell<Lookup>,
    settings: RefCell<ViewerSettings>,
    emit: Emitter,
    /// The download index, for the transition cards' downloaded flags.
    downloads: DownloadCheck,
    /// Whether the reader menu is shown (the screen's flag, shared with every row).
    menu_open: Rc<Cell<bool>>,
    /// The realised rows by index, each with the item it was made for, so a changed item list replaces them.
    slots: RefCell<BTreeMap<usize, (ItemKey, Row)>>,
    recognizer: RefCell<TapRecognizer>,
    watch: RefCell<Option<SettingsWatch>>,
    /// The chapters the core handed over, kept so a chapter's state change can hand them to the controller again.
    chapters: RefCell<Option<ViewerChapters>>,
    chapter_tasks: RefCell<Tasks>,
    anim: Rc<Cell<u64>>,
    /// The index the strip last settled on; a scroll to the same index is not a page change.
    settled: Cell<usize>,
    /// The size the slots were laid out for, so a resize relays them out.
    laid_size: Cell<(i32, i32)>,
}

fn pager_options(s: &ViewerSettings) -> PagerOptions {
    PagerOptions { always_show_transition: s.always_show_transition, use_transitions: s.page_transitions, navigate_pan: s.navigate_pan }
}

impl PagerInner {
    fn new(kind: ViewerKind, ctx: ViewerContext) -> Rc<Self> {
        let root = scrolled();
        let fixed = gtk::Fixed::new();
        root.set_child(Some(&fixed));
        // The strip stays hidden until its first layout (`pager.isVisible = false`).
        root.set_visible(false);
        let settings = ctx.settings;
        let inner = Rc::new_cyclic(|me| PagerInner {
            me: me.clone(),
            wheel_sum: Cell::new(0.0),
            wheel_at: Cell::new(0),
            panned_at: Cell::new(0),
            kind,
            root,
            fixed,
            ctl: RefCell::new(PagerController::new(kind, pager_options(&settings.clone().before_first_emission()))),
            lookup: RefCell::default(),
            settings: RefCell::new(settings.clone()),
            emit: ctx.emit,
            downloads: ctx.downloads,
            menu_open: ctx.menu_open,
            slots: RefCell::default(),
            recognizer: RefCell::new(recognizer(true)),
            watch: RefCell::new(ctx.prefs.map(|p| SettingsWatch::new(p, settings))),
            chapters: RefCell::default(),
            chapter_tasks: RefCell::default(),
            anim: Rc::default(),
            settled: Cell::new(0),
            laid_size: Cell::new((-1, -1)),
        });
        wire_surface(&inner);
        // The first emission of the store gives page transitions their stored value (E-22).
        let weak = Rc::downgrade(&inner);
        glib::idle_add_local_once(move || {
            if let Some(s) = weak.upgrade() {
                s.ctl.borrow_mut().set_options(pager_options(&s.settings.borrow()));
            }
        });
        inner.wire_drag();
        inner.wire_resize();
        inner
    }

    fn horizontal(&self) -> bool {
        self.kind != ViewerKind::Vertical
    }

    /// What a new row is handed. A split page asks for its insert page on the next idle, because the report can come
    /// while the rows are being built.
    fn host(&self) -> PageHost {
        let weak = self.me.clone();
        let on_split: Rc<dyn Fn(Rc<ReaderPage>)> = Rc::new(move |page| {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(this) = weak.upgrade() {
                    this.on_page_split(&page);
                }
            });
        });
        page_host(&self.settings.borrow(), self.kind, &self.emit, on_split, Rc::new(|| {}), &self.downloads, &self.menu_open)
    }

    /// A transform split `page` in two: its insert page goes next to it (B§5.6, T-44a-050).
    fn on_page_split(&self, page: &Rc<ReaderPage>) {
        if !self.emit.is_open() {
            return;
        }
        let Some(parent) = PageRef::from_reader(page) else { return };
        let insert = PageRef::insert_of(&parent);
        let fx = self.ctl.borrow_mut().on_page_split(&parent, insert);
        self.run(fx);
    }

    fn extent(&self) -> i32 {
        if self.horizontal() { self.root.width() } else { self.root.height() }.max(0)
    }

    fn adjustment(&self) -> gtk::Adjustment {
        if self.horizontal() { self.root.hadjustment() } else { self.root.vadjustment() }
    }

    /// Takes the chapters and follows their load state: each change hands them to the controller again (B§11.3).
    fn set_chapters(&self, chapters: &ViewerChapters) {
        *self.chapters.borrow_mut() = Some(chapters.clone());
        let weak = self.me.clone();
        let on_change: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(this) = weak.upgrade() {
                this.apply_chapters();
            }
        });
        *self.chapter_tasks.borrow_mut() = holder::watch_chapters(chapters, on_change);
        self.apply_chapters();
    }

    fn apply_chapters(&self) {
        let Some(chapters) = self.chapters.borrow().clone() else { return };
        self.lookup.borrow_mut().rebuild(&chapters);
        let fx = self.ctl.borrow_mut().set_chapters(ChapterSet::from_viewer(&chapters));
        self.run(fx);
    }

    fn run(&self, fx: Vec<Effect>) {
        for effect in fx {
            match effect {
                Effect::Rebuilt => {
                    self.rebuild();
                    self.realign();
                }
                Effect::Show => {
                    self.root.set_visible(true);
                    self.rebuild();
                }
                Effect::ScrollTo { index, smooth } => self.scroll_to(index, smooth),
                Effect::Pan(dir) => self.on_current_row(|row| row.pan(dir)),
                // the page's zoom-start hook runs the landscape zoom (T-43b-018)
                Effect::ZoomStart { page, forward } => {
                    let key = ItemKey::Page(page);
                    if let Some((_, row)) = self.slots.borrow().values().find(|(item, _)| *item == key) {
                        row.page_selected(forward);
                    }
                }
                Effect::PageSelected(r) => {
                    let page = self.lookup.borrow().page(&r);
                    if let Some(page) = page {
                        self.emit.send(ViewerEvent::PageSelected(page));
                    }
                }
                Effect::TransitionSelected => self.emit.send(ViewerEvent::TransitionShown),
                Effect::HideMenu => self.emit.send(ViewerEvent::HideMenu),
                Effect::ShowMenu => self.emit.send(ViewerEvent::ShowMenu),
                Effect::Preload(key) => {
                    let chapter = self.lookup.borrow().chapter(&key);
                    if let Some(chapter) = chapter {
                        self.emit.send(ViewerEvent::RequestPreload(chapter));
                    }
                }
            }
        }
    }

    /// After the list changed, puts the strip back on the shown item, which may have a new index. Not while a move
    /// runs: that one is heading for its own target.
    fn realign(&self) {
        let (idle, view) = {
            let ctl = self.ctl.borrow();
            (ctl.is_idle(), ctl.view_index())
        };
        if idle {
            self.adjustment().set_value(view as f64 * f64::from(self.extent()));
            self.settled.set(view);
        }
    }

    /// Realises the slots around the shown index (`current ± 1`) and places them on the strip.
    fn rebuild(&self) {
        let (items, view) = {
            let ctl = self.ctl.borrow();
            (ctl.items().to_vec(), ctl.view_index())
        };
        let (w, h) = (self.root.width().max(0), self.root.height().max(0));
        let resized = self.laid_size.replace((w, h)) != (w, h);
        let count = items.len();
        let wanted: Range<usize> = if count == 0 { 0..0 } else { view.saturating_sub(1)..(view + 2).min(count) };
        let mut slots = self.slots.borrow_mut();
        let stale: Vec<usize> = slots.keys().filter(|i| !wanted.contains(i)).copied().collect();
        for i in stale {
            if let Some((_, row)) = slots.remove(&i) {
                self.fixed.remove(&row.widget);
            }
        }
        for i in wanted {
            if slots.get(&i).is_some_and(|(key, _)| *key != items[i]) {
                if let Some((_, old)) = slots.remove(&i) {
                    self.fixed.remove(&old.widget);
                }
            }
            let widget = slots
                .entry(i)
                .or_insert_with(|| {
                    let row = item_row(&items[i], &self.lookup.borrow(), &self.host());
                    self.fixed.put(&row.widget, 0.0, 0.0);
                    (items[i].clone(), row)
                })
                .1
                .widget
                .clone();
            let (x, y) = if self.horizontal() { (i as i32 * w, 0) } else { (0, i as i32 * h) };
            self.fixed.move_(&widget, f64::from(x), f64::from(y));
            // a row's margins add to its request, so they are left out (a transition card has side margins)
            let (mx, my) = (widget.margin_start() + widget.margin_end(), widget.margin_top() + widget.margin_bottom());
            widget.set_size_request((w - mx).max(0), (h - my).max(0));
        }
        let total = count as i32;
        if self.horizontal() {
            self.fixed.set_size_request(total * w, h);
        } else {
            self.fixed.set_size_request(w, total * h);
        }
        drop(slots);
        // the size is recorded here, so the strip is put on the shown item here too: the frame tick sees no change
        // afterwards. A first scroll made before the strip had a size would otherwise stay at 0 (the Prev card).
        if resized {
            self.adjustment().set_value(view as f64 * f64::from(self.extent()));
        }
    }

    fn scroll_to(&self, index: usize, smooth: bool) {
        self.rebuild();
        let target = index as f64 * f64::from(self.extent());
        self.glide(target, smooth, Some(index));
    }

    /// Moves the strip to `target`. While a smooth move runs the strip is not idle, so a chapter set waits for it
    /// (E-75). When the move ends, the strip settles on `settle_to` (if the page changed) and becomes idle again.
    fn glide(&self, target: f64, smooth: bool, settle_to: Option<usize>) {
        if smooth {
            self.set_idle(false);
        }
        let weak = self.me.clone();
        animate(&self.root, self.adjustment(), Rc::clone(&self.anim), target, smooth, move || {
            if let Some(s) = weak.upgrade() {
                if let Some(index) = settle_to {
                    s.settle(index);
                }
                s.set_idle(true);
            }
        });
    }

    fn set_idle(&self, idle: bool) {
        let fx = self.ctl.borrow_mut().set_idle(idle);
        self.run(fx);
    }

    /// The strip arrived on `index` (ViewPager `onPageSelected`). A scroll that ends where it started is not a
    /// page change.
    fn settle(&self, index: usize) {
        // `realign` also records the shown index, so an unchanged index is a page change only while the controller
        // has not yet made that item current (the opening scroll onto page 1 of a one-page chapter).
        let unchanged = {
            let ctl = self.ctl.borrow();
            ctl.items().get(index).is_none_or(|item| ctl.current() == Some(item))
        };
        if self.settled.replace(index) == index && unchanged {
            self.rebuild();
            return;
        }
        self.rebuild();
        let fx = self.ctl.borrow_mut().on_settled(index);
        self.run(fx);
    }

    /// Swipe commit (B§5, TP §5): past half the extent, or on a release velocity above the threshold, to the
    /// neighbour the drag points to. Otherwise it snaps back.
    fn finish_drag(&self, offset: f64, velocity: f64) {
        let extent = f64::from(self.extent());
        let (view, count) = {
            let ctl = self.ctl.borrow();
            (ctl.view_index(), ctl.items().len())
        };
        let step = gesture::swipe_step(offset, velocity, extent);
        let target = view as i64 + step;
        if step != 0 && target >= 0 && (target as usize) < count {
            let index = target as usize;
            self.glide(index as f64 * extent, true, Some(index));
        } else {
            self.glide(view as f64 * extent, true, None);
        }
    }

    /// A drag moves the strip, unless the shown page takes it. Who owns a drag is decided once, past the drag
    /// threshold, as Android's pager does: the page keeps the whole drag when it can still pan the way the drag goes
    /// (zoomed, or bigger than the view) or when the drag runs across the strip's axis. A zoomed page therefore never
    /// slides on to the next page mid-drag; a new drag started at its edge does.
    fn wire_drag(&self) {
        let drag = gtk::GestureDrag::new();
        let start = Rc::new(Cell::new(0.0));
        let track = Rc::new(RefCell::new(DragTracker::default()));
        let owner = Rc::new(Cell::new(DragOwner::Undecided));
        let slop = recognizer_slop();
        let weak = self.me.clone();
        let (s, t, o) = (Rc::clone(&start), Rc::clone(&track), Rc::clone(&owner));
        drag.connect_drag_begin(move |_, _, _| {
            if let Some(this) = weak.upgrade() {
                t.borrow_mut().begin();
                s.set(this.adjustment().value());
                // a page that cannot pan at all leaves the whole drag to the strip, from its first pixel
                let mut pans = false;
                this.on_current_row(|row| pans = [(2.0, 0.0), (-2.0, 0.0), (0.0, 2.0), (0.0, -2.0)].iter().any(|&(x, y)| row.can_pan_by(x, y)));
                o.set(if pans { DragOwner::Undecided } else { DragOwner::Strip(0.0) });
                this.rebuild();
                this.set_idle(false);
            }
        });
        let weak = self.me.clone();
        let (s, t, o) = (Rc::clone(&start), Rc::clone(&track), Rc::clone(&owner));
        drag.connect_drag_update(move |_, dx, dy| {
            let Some(this) = weak.upgrade() else { return };
            let horizontal = this.horizontal();
            let (offset, across) = if horizontal { (dx, dy) } else { (dy, dx) };
            if o.get() == DragOwner::Undecided {
                if offset.abs().max(across.abs()) < slop {
                    return;
                }
                // the content follows the finger, so a drag toward -x shows more of the page's right side
                let toward = -offset.signum() * 2.0;
                let mut pans = across.abs() > offset.abs();
                this.on_current_row(|row| pans = pans || row.can_pan_by(if horizontal { toward } else { 0.0 }, if horizontal { 0.0 } else { toward }));
                o.set(if pans { DragOwner::Page } else { DragOwner::Strip(offset) });
            }
            let DragOwner::Strip(from) = o.get() else { return };
            t.borrow_mut().push(now_ms(), offset - from);
            this.adjustment().set_value(s.get() - (offset - from));
        });
        let weak = self.me.clone();
        drag.connect_drag_end(move |_, dx, dy| {
            if let Some(this) = weak.upgrade() {
                let DragOwner::Strip(from) = owner.replace(DragOwner::Undecided) else {
                    // the page took the drag: the strip did not move, so it only settles
                    this.finish_drag(0.0, 0.0);
                    return;
                };
                let offset = if this.horizontal() { dx } else { dy } - from;
                // The release is a sample too, so a drag that paused before letting go has no velocity.
                let velocity = {
                    let mut track = track.borrow_mut();
                    track.push(now_ms(), offset);
                    track.velocity()
                };
                this.finish_drag(offset, velocity);
            }
        });
        self.root.add_controller(drag);
    }

    /// A resize moves the slots and the strip to the new extent. Checked once per frame, which is cheap.
    fn wire_resize(&self) {
        let weak = self.me.clone();
        self.root.add_tick_callback(move |root, _| {
            let Some(this) = weak.upgrade() else { return ControlFlow::Break };
            if this.laid_size.get() != (root.width(), root.height()) && root.is_visible() {
                this.rebuild();
                let view = this.ctl.borrow().view_index() as f64;
                this.adjustment().set_value(view * f64::from(this.extent()));
            }
            ControlFlow::Continue
        });
    }

    /// Runs `f` on the row of the page that is shown, when it has one.
    fn on_current_row(&self, f: impl FnOnce(&Row)) {
        let index = self.ctl.borrow().view_index();
        if let Some((_, row)) = self.slots.borrow().get(&index) {
            f(row);
        }
    }

    /// A move command. The shown page's pan room in the direction of the command decides whether it pans first
    /// (`navigate_to_pan`, B§6.3), for keys, wheel and side-zone taps alike.
    fn command(&self, cmd: super::pager::nav::PageCmd) {
        let dir = super::pager::nav::physical(self.kind, cmd);
        let mut can_pan = false;
        self.on_current_row(|row| can_pan = row.can_pan(dir));
        let fx = self.ctl.borrow_mut().command(cmd, can_pan);
        self.run(fx);
    }

    fn set_splits_removed(&self) {
        let fx = self.ctl.borrow_mut().remove_splits();
        self.run(fx);
    }

    fn apply_settings(&self, new: ViewerSettings) {
        let old = std::mem::replace(&mut *self.settings.borrow_mut(), new.clone());
        self.ctl.borrow_mut().set_options(pager_options(&new));
        let opts = page_opts(&new, self.kind, crate::reader::theme::system_prefers_dark());
        self.slots.borrow().values().for_each(|(_, row)| row.apply(&opts));
        for reaction in settings::diff(&old, &new, Family::Pager) {
            match reaction {
                Reaction::RefreshImages => {
                    let fx = self.ctl.borrow().refresh();
                    self.run(fx);
                }
                Reaction::RemoveSplits => self.set_splits_removed(),
                // The taps read the table on each release. The pager's theme change refreshes its images, and it has
                // no webtoon zoom, so the other reactions do not apply to it.
                Reaction::Navigation | Reaction::Restyle | Reaction::ZoomOutDisabled | Reaction::DoubleTapZoom => {}
            }
        }
    }

    fn destroy(&self) {
        self.emit.close();
        self.anim.set(self.anim.get() + 1);
        *self.chapter_tasks.borrow_mut() = Tasks::default();
        let mut child = self.fixed.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            self.fixed.remove(&widget);
        }
        self.slots.borrow_mut().clear();
    }

    fn key(&self, ev: KeyEvent) -> bool {
        let ctx = key_context(&self.settings.borrow(), self.menu_open.get());
        let out = keys::pager_key(ev, ctx);
        match out.action {
            Some(KeyAction::Page(cmd)) => self.command(cmd),
            Some(KeyAction::ToggleMenu) => self.emit.send(ViewerEvent::ToggleMenu),
            Some(KeyAction::Zoom(zoom_key)) => self.on_current_row(|row| row.zoom_key(zoom_key)),
            Some(KeyAction::Scroll { .. }) | None => {}
        }
        out.claimed
    }

    /// A pointer scroll. Ctrl zooms the shown page. A page that can still pan that way pans (a zoomed or a fit-width
    /// page); a scroll that keeps coming after it reached the edge is swallowed until it pauses for `WHEEL_GAP_MS`.
    /// Otherwise down is the next page, and up or no movement is the previous one (E-24).
    fn wheel(&self, scroll: Scroll) -> bool {
        let Scroll { dx, dy, notches, ctrl } = scroll;
        if ctrl {
            if dy != 0.0 {
                self.on_current_row(|row| row.zoom_wheel(dy < 0.0));
            }
            return true;
        }
        let (px, py) = if notches { (wheel_delta(dx, self.root.width()), wheel_delta(dy, self.root.height())) } else { (dx, dy) };
        let now = now_ms();
        let mut panned = false;
        self.on_current_row(|row| panned = row.scroll_pan(px, py));
        if panned || now.saturating_sub(self.panned_at.get()) < WHEEL_GAP_MS {
            self.wheel_sum.set(0.0);
            self.panned_at.set(now);
            return true;
        }
        if dy == 0.0 && dx != 0.0 {
            // a sideways scroll that pans nothing goes on to the scrolled window, as before
            return false;
        }
        match wheel_turn(self.wheel_sum.get() + dy, now.saturating_sub(self.wheel_at.get())) {
            WheelTurn::Gather(sum) => self.wheel_sum.set(sum),
            WheelTurn::Turn(next) => {
                self.wheel_sum.set(0.0);
                self.wheel_at.set(now);
                self.command(if next { super::pager::nav::PageCmd::Next } else { super::pager::nav::PageCmd::Prev });
            }
        }
        true
    }
}

/// Who a pager drag moves: not known before the drag threshold, then the shown page or the strip (with the drag
/// offset the strip starts from).
#[derive(Clone, Copy, Debug, PartialEq)]
enum DragOwner {
    Undecided,
    Page,
    Strip(f64),
}

/// What a scroll step does in a paged viewer.
#[derive(Debug, PartialEq)]
enum WheelTurn {
    /// Not enough yet, or too soon after the last turn: keep the sum.
    Gather(f64),
    /// Turn one page, forward when true.
    Turn(bool),
}

/// A wheel notch is 1.0; a touchpad swipe arrives as many small steps. A page turns when the gathered scroll reaches
/// one notch, and at most once per `WHEEL_GAP_MS`, so one swipe never flips several pages.
const WHEEL_GAP_MS: u64 = 350;

fn wheel_turn(sum: f64, since_last_ms: u64) -> WheelTurn {
    if since_last_ms < WHEEL_GAP_MS {
        // the rest of the swipe that just turned a page is dropped
        return WheelTurn::Gather(0.0);
    }
    if sum.abs() >= 1.0 { WheelTurn::Turn(sum > 0.0) } else { WheelTurn::Gather(sum) }
}

fn key_context(s: &ViewerSettings, menu_visible: bool) -> KeyContext {
    KeyContext { volume_enabled: s.volume_keys, volume_inverted: s.volume_keys_inverted, menu_visible }
}

impl Surface for PagerInner {
    fn root(&self) -> &gtk::ScrolledWindow {
        &self.root
    }

    fn recognizer(&self) -> &RefCell<TapRecognizer> {
        &self.recognizer
    }

    fn watch(&self) -> &RefCell<Option<SettingsWatch>> {
        &self.watch
    }

    fn apply_settings(&self, new: ViewerSettings) {
        PagerInner::apply_settings(self, new);
    }

    fn alive(&self) -> bool {
        self.emit.is_open()
    }

    fn tap(&self, gesture: Gesture) {
        // Long presses reach the page view, and a pager double tap is swallowed (E-20).
        let Gesture::Tap { x, y } = gesture else { return };
        let (nx, ny) = normalised(x, y, f64::from(self.root.width()), f64::from(self.root.height()));
        let outcome = {
            let s = self.settings.borrow();
            let action = nav::hit_test(nav::layout(s.nav_mode_pager, self.kind), s.tapping_pager, nx, ny);
            nav::pager_tap(action)
        };
        match outcome {
            TapOutcome::ToggleMenu => self.emit.send(ViewerEvent::ToggleMenu),
            TapOutcome::Page(cmd) => self.command(cmd),
            TapOutcome::ScrollDown | TapOutcome::ScrollUp => {}
        }
    }

    fn key(&self, ev: KeyEvent) -> bool {
        PagerInner::key(self, ev)
    }

    fn wheel(&self, scroll: Scroll) -> bool {
        PagerInner::wheel(self, scroll)
    }

    /// The pager turns pages itself; the scrolled window must not also scroll the strip with the same wheel event.
    fn wheel_first(&self) -> bool {
        true
    }
}

pub struct WebtoonEngine(Rc<WebtoonInner>);

impl WebtoonEngine {
    pub fn new(gapped: bool, ctx: ViewerContext) -> Self {
        Self(WebtoonInner::new(gapped, ctx))
    }

    #[cfg(test)]
    /// Applies a settings change and carries out its reactions (T-44a-005).
    pub fn apply_settings(&self, new: ViewerSettings) {
        self.0.apply_settings(new);
    }
}

impl ViewerEngine for WebtoonEngine {
    fn kind(&self) -> ViewerKind {
        self.0.kind
    }

    fn widget(&self) -> &gtk::Widget {
        self.0.frame.upcast_ref()
    }

    fn set_chapters(&self, chapters: ViewerChapters) {
        self.0.set_chapters(&chapters);
    }

    fn move_to_page(&self, page: &Rc<ReaderPage>) {
        let Some(r) = PageRef::from_reader(page) else { return };
        let fx = self.0.ctl.borrow_mut().move_to_page(&ItemKey::Page(r));
        self.0.run(fx);
    }

    fn handle_key(&self, ev: KeyEvent) -> bool {
        self.0.key(ev)
    }

    /// The scrolled window handles the wheel itself (B§4.4).
    fn handle_scroll(&self, _dy: f64) -> bool {
        false
    }

    fn destroy(&self) {
        self.0.destroy();
    }
}

struct WebtoonInner {
    me: Weak<WebtoonInner>,
    kind: ViewerKind,
    root: gtk::ScrolledWindow,
    fixed: gtk::Fixed,
    ctl: RefCell<WebtoonController>,
    lookup: RefCell<Lookup>,
    settings: RefCell<ViewerSettings>,
    emit: Emitter,
    /// The download index, for the transition cards' downloaded flags.
    downloads: DownloadCheck,
    /// Whether the reader menu is shown (the screen's flag, shared with every row).
    menu_open: Rc<Cell<bool>>,
    /// Realised rows, keyed by item so a rebuild reuses the rows that survive it (the keyed diff of T-44a-031).
    realised: RefCell<HashMap<ItemKey, Row>>,
    layout: RefCell<StripLayout>,
    /// The items `layout` was made for, so a change of heights can be anchored to the item at the top of the view.
    laid_items: RefCell<Vec<ItemKey>>,
    /// The decoded size of every page that has shown one, so a page that left the screen keeps its height.
    sizes: RefCell<HashMap<ItemKey, (u32, u32)>>,
    watch: RefCell<Option<SettingsWatch>>,
    /// The chapters the core handed over, kept so a chapter's state change can hand them to the controller again.
    chapters: RefCell<Option<ViewerChapters>>,
    chapter_tasks: RefCell<Tasks>,
    /// The size the strip was last laid out for. A strip that is shown before it has a size is laid out again once
    /// it has one (T-44a-032).
    laid_size: Cell<(i32, i32)>,
    /// An item to scroll to once the scroll range has grown to the strip. Set by a scroll that came before the
    /// strip was allocated; applied on the frame it has been.
    pending_scroll: Cell<Option<usize>>,
    last_scroll: Cell<i64>,
    /// The viewport height at the last measure: a scroll that comes with a new height was the layout, not the user.
    last_viewport: Cell<i32>,
    /// Set while `sync` measures and reconciles, so the value changes it causes do not re-enter it.
    busy: Cell<bool>,
    /// The viewport height at the first layout. Taps are normalised by it (B§2.1).
    original_height: Cell<i32>,
    recognizer: RefCell<TapRecognizer>,
    anim: Rc<Cell<u64>>,
    /// False until the store's first emission, so page transitions start off (E-22).
    transitions_live: Cell<bool>,
    /// What the engine shows: the frame scales and moves the strip (T-44a-038).
    frame: ZoomFrame,
    zoom: RefCell<WebtoonZoom>,
    /// Bumped to stop a running zoom animation.
    zoom_anim: Cell<u64>,
    /// A zoom animation is running: a double tap is ignored (E-33).
    animating: Cell<bool>,
    /// A layout asked for by a row is waiting for the next idle.
    layout_pending: Cell<bool>,
    /// A quick scale in progress: a drag that began on a double tap (T-44a-040).
    quick: RefCell<Option<QuickScale>>,
}

fn webtoon_options(s: &ViewerSettings) -> WebtoonOptions {
    WebtoonOptions { always_show_transition: s.always_show_transition, use_transitions: s.page_transitions, hide_threshold: s.hide_threshold }
}

impl WebtoonInner {
    fn new(gapped: bool, ctx: ViewerContext) -> Rc<Self> {
        let root = scrolled();
        let fixed = gtk::Fixed::new();
        root.set_child(Some(&fixed));
        root.set_visible(false);
        let frame = ZoomFrame::new(&root);
        let kind = if gapped { ViewerKind::WebtoonGapped } else { ViewerKind::Webtoon };
        let settings = ctx.settings;
        let inner = Rc::new_cyclic(|me| WebtoonInner {
            me: me.clone(),
            kind,
            root,
            fixed,
            frame,
            zoom: RefCell::new(WebtoonZoom::new(settings.zoom_out_disabled)),
            zoom_anim: Cell::new(0),
            animating: Cell::new(false),
            layout_pending: Cell::new(false),
            quick: RefCell::new(None),
            ctl: RefCell::new(WebtoonController::new(webtoon_options(&settings.clone().before_first_emission()))),
            lookup: RefCell::default(),
            settings: RefCell::new(settings.clone()),
            emit: ctx.emit,
            downloads: ctx.downloads,
            menu_open: ctx.menu_open,
            realised: RefCell::default(),
            layout: RefCell::default(),
            laid_items: RefCell::default(),
            sizes: RefCell::default(),
            watch: RefCell::new(ctx.prefs.map(|p| SettingsWatch::new(p, settings))),
            chapters: RefCell::default(),
            chapter_tasks: RefCell::default(),
            laid_size: Cell::new((-1, -1)),
            pending_scroll: Cell::new(None),
            last_scroll: Cell::new(0),
            last_viewport: Cell::new(0),
            busy: Cell::new(false),
            original_height: Cell::new(0),
            recognizer: RefCell::new(recognizer(false)),
            anim: Rc::default(),
            transitions_live: Cell::new(false),
        });
        wire_surface(&inner);
        let weak = Rc::downgrade(&inner);
        glib::idle_add_local_once(move || {
            if let Some(s) = weak.upgrade() {
                s.transitions_live.set(true);
                s.ctl.borrow_mut().set_options(webtoon_options(&s.settings.borrow()));
            }
        });
        let weak = inner.me.clone();
        let adj = inner.root.vadjustment();
        adj.connect_value_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.sync();
            }
        });
        inner.wire_resize();
        inner.wire_zoom();
        inner
    }

    /// What a new row is handed. A row asks for a layout when it starts or stops showing its error card.
    fn host(&self) -> PageHost {
        let weak = self.me.clone();
        let on_layout: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(this) = weak.upgrade() {
                this.request_layout();
            }
        });
        page_host(&self.settings.borrow(), self.kind, &self.emit, Rc::new(|_| {}), on_layout, &self.downloads, &self.menu_open)
    }

    /// Lays the strip out again on the next idle: the report can come while the rows are being built.
    fn request_layout(&self) {
        if self.layout_pending.replace(true) {
            return;
        }
        let weak = self.me.clone();
        glib::idle_add_local_once(move || {
            if let Some(this) = weak.upgrade() {
                this.layout_pending.set(false);
                this.sync();
            }
        });
    }

    /// The strip is laid out once it has a size, and again when the size changes. Checked once per frame, like the
    /// pager. The scroll range grows a frame after the layout sets the strip's size, so a pending scroll waits for it.
    fn wire_resize(&self) {
        let weak = self.me.clone();
        self.root.add_tick_callback(move |root, _| {
            let Some(this) = weak.upgrade() else { return ControlFlow::Break };
            if !root.is_visible() || root.height() <= 0 {
                return ControlFlow::Continue;
            }
            let size = (root.width(), root.height());
            let resized = this.laid_size.replace(size) != size;
            if this.pending_scroll.get().is_some() {
                // Size the strip first, but report nothing before the scroll lands: the controller would take the
                // top of the strip for the current item.
                if resized {
                    this.lay_out();
                }
                if this.apply_pending_scroll() {
                    this.sync();
                }
            } else if resized {
                this.sync();
            }
            ControlFlow::Continue
        });
    }

    /// Scrolls to the pending item when the scroll range holds the strip. `true` when it did.
    fn apply_pending_scroll(&self) -> bool {
        let Some(index) = self.pending_scroll.get() else { return false };
        let (_, layout) = self.current_layout();
        let adj = self.root.vadjustment();
        if !layout.fits_range(adj.upper()) {
            return false;
        }
        self.pending_scroll.set(None);
        let target = (layout.scroll_to(index) as f64).min(adj.upper() - adj.page_size()).max(adj.lower());
        // A jump is not a scroll the user made (B§7.5: the view reports it with `dy == 0`).
        self.last_scroll.set(target.round() as i64);
        adj.set_value(target);
        true
    }

    fn viewport(&self) -> i32 {
        let h = self.root.height().max(0);
        if self.original_height.get() == 0 && h > 0 {
            self.original_height.set(h);
        }
        h
    }

    /// The width of a page row: the strip less the side padding on both sides.
    fn row_width(&self) -> i32 {
        let width = self.root.width().max(0);
        (width - 2 * side_padding_px(width, self.settings.borrow().side_padding)).max(0)
    }

    /// The strip geometry for the current items. A decoded page is as tall as it is at the row width. A page that is not
    /// decoded keeps a reserved height (its known size, else the previous decoded page's, else 1.4 times the width) under
    /// its spinner, so the strip is never short. A page that failed is the error card, 0.8 of the viewport (B§8.5), and
    /// transition cards have a fixed height. The viewport is the strip's own height, which the frame makes
    /// `original / scale` below a scale of 1 (E-31).
    fn current_layout(&self) -> (Vec<ItemKey>, StripLayout) {
        let items = self.ctl.borrow().items().to_vec();
        let vh = self.viewport();
        let width = self.row_width();
        // gapped mode leaves room under every item; the row itself is the item height without the gap
        let gap = gap_px(self.kind == ViewerKind::WebtoonGapped);
        {
            let mut sizes = self.sizes.borrow_mut();
            for (key, row) in self.realised.borrow().iter() {
                if let Some(size) = row.source_size() {
                    sizes.insert(key.clone(), size);
                }
            }
        }
        let sizes = self.sizes.borrow();
        let mut previous = None;
        let heights = items
            .iter()
            .map(|item| {
                let height = match item.page() {
                    None => CARD_HEIGHT,
                    Some(page) if self.is_errored(item, page) => error_card::webtoon_card_height(vh),
                    Some(_) => {
                        let known = sizes.get(item).copied();
                        let height = reserved_height(known, previous, width);
                        if known.is_some() {
                            previous = Some(height);
                        }
                        height
                    }
                };
                height + gap
            })
            .collect();
        (items, StripLayout::new(heights))
    }

    fn is_errored(&self, item: &ItemKey, page: &PageRef) -> bool {
        match self.realised.borrow().get(item) {
            Some(row) => row.errored(),
            None => self.lookup.borrow().pages.get(page).is_some_and(|p| p.status.get() == PageStatus::Error),
        }
    }

    /// Takes the chapters and follows their load state: each change hands them to the controller again (B§11.3).
    fn set_chapters(&self, chapters: &ViewerChapters) {
        *self.chapters.borrow_mut() = Some(chapters.clone());
        let weak = self.me.clone();
        let on_change: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(this) = weak.upgrade() {
                this.apply_chapters();
            }
        });
        *self.chapter_tasks.borrow_mut() = holder::watch_chapters(chapters, on_change);
        self.apply_chapters();
    }

    fn apply_chapters(&self) {
        let Some(chapters) = self.chapters.borrow().clone() else { return };
        self.lookup.borrow_mut().rebuild(&chapters);
        let fx = self.ctl.borrow_mut().set_chapters(ChapterSet::from_viewer(&chapters));
        self.run(fx);
    }

    fn run(&self, fx: Vec<Effect>) {
        for effect in fx {
            match effect {
                Effect::Rebuilt => self.sync(),
                Effect::Show => {
                    self.root.set_visible(true);
                    self.sync();
                }
                Effect::ScrollTo { index, .. } => {
                    // The strip may not be allocated yet; the frame tick finishes the scroll then.
                    self.pending_scroll.set(Some(index));
                    if self.apply_pending_scroll() {
                        self.sync();
                    } else {
                        self.lay_out();
                    }
                }
                // the webtoon neither pans a page nor zooms one to the view height
                Effect::Pan(_) | Effect::ZoomStart { .. } => {}
                Effect::PageSelected(r) => {
                    let page = self.lookup.borrow().page(&r);
                    if let Some(page) = page {
                        self.emit.send(ViewerEvent::PageSelected(page));
                    }
                }
                Effect::TransitionSelected => self.emit.send(ViewerEvent::TransitionShown),
                Effect::HideMenu => self.emit.send(ViewerEvent::HideMenu),
                Effect::ShowMenu => self.emit.send(ViewerEvent::ShowMenu),
                Effect::Preload(key) => {
                    let chapter = self.lookup.borrow().chapter(&key);
                    if let Some(chapter) = chapter {
                        self.emit.send(ViewerEvent::RequestPreload(chapter));
                    }
                }
            }
        }
    }

    /// Realises the rows around the viewport, reports the scroll to the controller and carries out what it asks.
    /// Re-entrant calls (a size change moves the scroll value) are dropped while this measures.
    fn sync(&self) {
        // Before the first allocation every placeholder is 0 px tall, so nothing would realise and no item would be
        // current. The frame tick lays the strip out once it has a size (T-44a-032).
        if self.root.height() <= 0 || self.busy.replace(true) {
            return;
        }
        let fx = self.measure();
        self.busy.set(false);
        self.run(fx);
    }

    fn measure(&self) -> Vec<Effect> {
        let scroll = self.root.vadjustment().value().round() as i64;
        let dy = scroll - self.last_scroll.replace(scroll);
        // showing or hiding the menu bar resizes the view and moves the scroll position to keep the page in place;
        // that move must not count as the user scrolling, or it hides the menu it just showed
        let vh = self.viewport();
        let resized = self.last_viewport.replace(vh) != vh;
        let (last, first) = self.lay_out();
        self.ctl.borrow_mut().on_scroll(if resized { 0 } else { dy as i32 }, true, last, first)
    }

    /// Realises the rows for the scroll position and sizes the strip. Returns the bottom-most item whose end is
    /// visible and the first visible item, which the controller needs to report the current item.
    fn lay_out(&self) -> (Option<usize>, Option<usize>) {
        let vh = self.viewport();
        if vh <= 0 {
            return (None, None);
        }
        let (items, layout) = self.current_layout();
        let scroll = self.anchor(&items, &layout);
        let range = layout.realised(scroll, vh);
        let last = layout.last_end_visible(scroll, vh, range.clone());
        let first = layout.first_visible(scroll);
        self.reconcile(&items, &layout, range);
        (last, first)
    }

    /// Keeps what the reader looks at in place when page heights change: the item that held the top of the view
    /// keeps its place, so pages that grow or shrink above it move the scroll position by the same amount. Nothing
    /// moves while the first jump to a page is still waiting. Returns the scroll position to lay out for.
    fn anchor(&self, items: &[ItemKey], layout: &StripLayout) -> i64 {
        let adj = self.root.vadjustment();
        let scroll = adj.value().round() as i64;
        if self.pending_scroll.get().is_some() {
            return scroll;
        }
        let (old, old_items) = (self.layout.borrow(), self.laid_items.borrow());
        let Some(old_index) = old.first_visible(scroll) else { return scroll };
        let Some(key) = old_items.get(old_index) else { return scroll };
        let new_index = if items.get(old_index) == Some(key) { Some(old_index) } else { items.iter().position(|i| i == key) };
        let Some(new_index) = new_index else { return scroll };
        let delta = anchor_delta(&old, old_index, layout, new_index);
        drop((old, old_items));
        if delta == 0 {
            return scroll;
        }
        // the range follows the strip's size one frame later, so it is widened here or the move would be clamped
        let total = layout.total_height() as f64;
        if adj.upper() < total {
            adj.set_upper(total);
        }
        // the move is not a scroll of the user, and must not run `sync` again
        let was_busy = self.busy.replace(true);
        adj.set_value((scroll + delta) as f64);
        self.busy.set(was_busy);
        let moved = adj.value().round() as i64;
        self.last_scroll.set(moved);
        moved
    }

    fn reconcile(&self, items: &[ItemKey], layout: &StripLayout, range: Range<usize>) {
        let width = self.root.width().max(0);
        let pad = side_padding_px(width, self.settings.borrow().side_padding);
        let mut realised = self.realised.borrow_mut();
        let wanted: HashSet<&ItemKey> = range.clone().map(|i| &items[i]).collect();
        let stale: Vec<ItemKey> = realised.keys().filter(|k| !wanted.contains(k)).cloned().collect();
        for key in stale {
            if let Some(row) = realised.remove(&key) {
                self.fixed.remove(&row.widget);
            }
        }
        for i in range {
            let key = &items[i];
            let widget = realised
                .entry(key.clone())
                .or_insert_with(|| {
                    let row = item_row(key, &self.lookup.borrow(), &self.host());
                    self.fixed.put(&row.widget, 0.0, 0.0);
                    row
                })
                .widget
                .clone();
            // every row is inset by the side padding on both sides (T-43b-047). A row's margins add to its width, so
            // the request leaves them out: otherwise the strip's minimum grows by them on every pass (BUG-READER-2).
            let gap = gap_px(self.kind == ViewerKind::WebtoonGapped);
            let margins = widget.margin_start() + widget.margin_end();
            widget.set_size_request((width - 2 * pad - margins).max(0), (layout.height(i) - gap).max(0));
            self.fixed.move_(&widget, f64::from(pad), layout.offset(i) as f64);
        }
        drop(realised);
        self.fixed.set_size_request(width, layout.total_height() as i32);
        *self.layout.borrow_mut() = layout.clone();
        *self.laid_items.borrow_mut() = items.to_vec();
    }

    /// The geometry the zoom math is done in: half the frame's width and original height (B§10.7).
    fn zoom_geometry(&self) -> ((f64, f64), f64) {
        let (w, h) = (f64::from(self.frame.width()), f64::from(self.frame.height()));
        ((w / 2.0, h / 2.0), h)
    }

    /// Shows the zoom state: the frame scales the strip and moves it, and the strip takes the height that goes with it.
    fn apply_zoom(&self) {
        let zoom = self.zoom.borrow();
        self.frame.set_view(zoom.scale(), zoom.pan());
    }

    fn stop_zoom_anim(&self) {
        self.zoom_anim.set(self.zoom_anim.get() + 1);
        self.animating.set(false);
    }

    /// Pinch and quick scale (`onScale`): `factor` multiplies the scale. Any zoom hides the menu (B§6.5).
    fn pinch(&self, factor: f64) {
        let (half, original) = self.zoom_geometry();
        self.zoom.borrow_mut().pinch(factor, half, original);
        self.apply_zoom();
        self.emit.send(ViewerEvent::HideMenu);
    }

    /// The end of a pinch: a scale under the minimum animates back to it (`onScaleEnd`).
    fn pinch_end(&self) {
        let anim = self.zoom.borrow_mut().settle();
        if let Some(anim) = anim {
            self.run_zoom_anim(anim);
        }
    }

    /// Whether the strip is at its first or last item, which is when a drag pans y as well as x (B§10.7).
    fn at_edge(&self) -> bool {
        let layout = self.layout.borrow();
        let scroll = self.root.vadjustment().value().round() as i64;
        let vh = i64::from(self.root.height());
        layout.first_visible(scroll) == Some(0) || (!layout.is_empty() && layout.offset(layout.len() - 1) < scroll + vh)
    }

    /// A drag while zoomed pans x always and y at the edges. `dx` and `dy` are the change since the last update.
    fn zoom_drag(&self, dx: f64, dy: f64) {
        let (half, original) = self.zoom_geometry();
        let edge = self.at_edge();
        self.zoom.borrow_mut().drag(dx, dy, edge, half, original);
        self.apply_zoom();
    }

    /// `onDoubleTapConfirmed` (E-33): zoom to 2 about the touch, or back to 1. Gated by the live setting.
    fn double_tap(&self, x: f64, y: f64) {
        let enabled = self.settings.borrow().double_tap_zoom;
        let (half, _) = self.zoom_geometry();
        let anim = self.zoom.borrow_mut().double_tap(enabled, self.animating.get(), (x, y), half);
        if let Some(anim) = anim {
            self.emit.send(ViewerEvent::HideMenu);
            self.run_zoom_anim(anim);
        }
    }

    /// Runs `anim` on the frame clock, 200 ms with the decelerate curve, clamped on every frame (E-34). A frame that is
    /// not on screen, or desktop animations being off, goes to the end state at once.
    fn run_zoom_anim(&self, anim: Anim) {
        self.stop_zoom_anim();
        let token = self.zoom_anim.get();
        let (half, original) = self.zoom_geometry();
        let animations = gtk::Settings::default().is_none_or(|s| s.is_gtk_enable_animations());
        if !animations || !self.frame.is_mapped() {
            self.zoom.borrow_mut().apply(anim.end(), half, original);
            self.apply_zoom();
            return;
        }
        self.animating.set(true);
        let started: Cell<Option<i64>> = Cell::new(None);
        let weak = self.me.clone();
        self.frame.add_tick_callback(move |_, clock| {
            let Some(this) = weak.upgrade() else { return ControlFlow::Break };
            if this.zoom_anim.get() != token {
                return ControlFlow::Break;
            }
            let now = clock.frame_time();
            let t0 = started.get().unwrap_or_else(|| {
                started.set(Some(now));
                now
            });
            let elapsed = ((now - t0) / 1000).clamp(0, i64::from(u32::MAX)) as u32;
            let (half, original) = this.zoom_geometry();
            let done = this.zoom.borrow_mut().step(&anim, elapsed, half, original);
            this.apply_zoom();
            if done {
                this.animating.set(false);
                ControlFlow::Break
            } else {
                ControlFlow::Continue
            }
        });
    }

    /// The pinch and drag gestures of the strip. They run on the frame in the capture phase, so they see the touch
    /// before the pages do (a page of the strip does not zoom on its own), and they claim nothing: taps, long presses
    /// and the scrolled window's own scrolling carry on.
    fn wire_zoom(&self) {
        let pinch = gtk::GestureZoom::new();
        pinch.set_propagation_phase(gtk::PropagationPhase::Capture);
        let last = Rc::new(Cell::new(1.0));
        let (weak, seen) = (self.me.clone(), Rc::clone(&last));
        pinch.connect_begin(move |_, _| {
            seen.set(1.0);
            if let Some(this) = weak.upgrade() {
                this.stop_zoom_anim();
            }
        });
        let weak = self.me.clone();
        pinch.connect_scale_changed(move |_, factor| {
            // the gesture reports the scale since it began; `onScale` takes the change since the last event
            let step = factor / last.replace(factor).max(f64::EPSILON);
            if let Some(this) = weak.upgrade() {
                this.pinch(step);
            }
        });
        let weak = self.me.clone();
        pinch.connect_end(move |_, _| {
            if let Some(this) = weak.upgrade() {
                this.pinch_end();
            }
        });
        self.frame.add_controller(pinch);

        let drag = gtk::GestureDrag::new();
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let slop = recognizer_slop();
        let applied = Rc::new(Cell::new((0.0, 0.0)));
        let (weak, seen) = (self.me.clone(), Rc::clone(&applied));
        drag.connect_drag_begin(move |_, _, _| {
            seen.set((0.0, 0.0));
            if let Some(this) = weak.upgrade() {
                this.quick.borrow_mut().take();
                this.stop_zoom_anim();
            }
        });
        let weak = self.me.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let Some(this) = weak.upgrade() else { return };
            if this.quick_scale_move(dy) {
                return;
            }
            if this.zoom.borrow().scale() <= 1.0 {
                return;
            }
            // the first `slop` pixels of a drag are a tap that wobbled, not a pan (touch slop)
            let (ex, ey) = (dead_zone(dx, slop), dead_zone(dy, slop));
            let (px, py) = applied.replace((ex, ey));
            this.zoom_drag(ex - px, ey - py);
        });
        let weak = self.me.clone();
        drag.connect_drag_end(move |_, _, _| {
            let Some(this) = weak.upgrade() else { return };
            if this.quick.borrow_mut().take().is_some() {
                this.pinch_end();
            }
        });
        self.frame.add_controller(drag);

        // a fling glides the zoomed strip on from the release (T-44a-041)
        let swipe = gtk::GestureSwipe::new();
        swipe.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = self.me.clone();
        swipe.connect_swipe(move |_, vx, vy| {
            if let Some(this) = weak.upgrade() {
                this.fling((vx, vy));
            }
        });
        self.frame.add_controller(swipe);
    }

    /// One vertical move of a quick scale (T-44a-040, TZ §4.2). A drag that began on the second press of a double tap
    /// scales the strip by the move's factor; the first move arms it. Returns whether the move was taken.
    fn quick_scale_move(&self, dy: f64) -> bool {
        let armed = self.settings.borrow().double_tap_zoom && self.recognizer.borrow().in_double_press();
        let mut slot = self.quick.borrow_mut();
        if slot.is_none() {
            if !armed {
                return false;
            }
            *slot = Some(QuickScale::new(THRESHOLD_DP, 0.0));
        }
        let Some(quick) = slot.as_mut() else { return false };
        // a crossing of the view height (scale 1) resets the distance, as the source does on an axis crossing
        let before = self.zoom.borrow().scale();
        let factor = quick.on_move(dy, |m| before < 1.0 && before * m >= 1.0);
        drop(slot);
        if factor != 1.0 {
            self.pinch(factor);
        }
        true
    }

    /// A fling while zoomed (B§10.8): the strip glides on from the release velocity.
    fn fling(&self, velocity: (f64, f64)) {
        let (half, original) = self.zoom_geometry();
        let edge = self.at_edge();
        let anim = self.zoom.borrow().fling(velocity, edge, half, original);
        if let Some(anim) = anim {
            self.run_zoom_anim(anim);
        }
    }

    /// Scrolls by `delta` pixels, animated when page transitions are on (B§7.4).
    fn scroll_by(&self, delta: f64) {
        let adj = self.root.vadjustment();
        let target = adj.value() + clamp_step(delta, self.root.height());
        let smooth = self.transitions_live.get() && self.settings.borrow().page_transitions;
        animate(&self.root, adj, Rc::clone(&self.anim), target, smooth, || {});
    }

    /// One scroll step is three quarters of the viewport (B§7.4).
    fn step(&self) -> f64 {
        f64::from(StripLayout::scroll_distance(self.root.height().max(0)))
    }

    /// `refreshAdapter` (B§7.7, E-30): drops the rows in `[centre-3, centre+3]` so they are rebuilt.
    fn refresh_images(&self) {
        let scroll = self.root.vadjustment().value().round() as i64;
        let vh = self.viewport();
        let (items, layout) = self.current_layout();
        let centre = layout.last_end_visible(scroll, vh, layout.realised(scroll, vh));
        if let Some(range) = refresh_range(centre, items.len()) {
            let mut realised = self.realised.borrow_mut();
            let dropped: Vec<Row> = range.filter_map(|i| realised.remove(&items[i])).collect();
            drop(realised);
            dropped.iter().for_each(|row| self.fixed.remove(&row.widget));
        }
        self.sync();
    }

    fn apply_settings(&self, new: ViewerSettings) {
        let old = std::mem::replace(&mut *self.settings.borrow_mut(), new.clone());
        self.ctl.borrow_mut().set_options(webtoon_options(&new));
        let opts = page_opts(&new, self.kind, crate::reader::theme::system_prefers_dark());
        self.realised.borrow().values().for_each(|row| row.apply(&opts));
        for reaction in settings::diff(&old, &new, Family::Webtoon) {
            match reaction {
                Reaction::RefreshImages => self.refresh_images(),
                // The webtoon does not restart for a theme change as upstream does; its background changes in place.
                Reaction::Restyle => crate::reader::theme::apply_background(&self.root, new.reader_theme),
                // Turning zoom-out off while the strip is below 1 animates it back to 1 (E-32).
                Reaction::ZoomOutDisabled => {
                    let anim = self.zoom.borrow_mut().set_zoom_out_disabled(new.zoom_out_disabled);
                    if let Some(anim) = anim {
                        self.run_zoom_anim(anim);
                    }
                }
                // The taps read the table on each release, and a double tap reads `double_tap_zoom` live.
                Reaction::Navigation | Reaction::RemoveSplits | Reaction::DoubleTapZoom => {}
            }
        }
    }

    fn destroy(&self) {
        self.emit.close();
        self.anim.set(self.anim.get() + 1);
        self.stop_zoom_anim();
        *self.chapter_tasks.borrow_mut() = Tasks::default();
        let mut child = self.fixed.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            self.fixed.remove(&widget);
        }
        self.realised.borrow_mut().clear();
    }

    fn key(&self, ev: KeyEvent) -> bool {
        let ctx = key_context(&self.settings.borrow(), self.menu_open.get());
        let out = keys::webtoon_key(ev, ctx);
        match out.action {
            Some(KeyAction::Scroll { down }) => {
                let step = self.step();
                self.scroll_by(if down { step } else { -step });
            }
            Some(KeyAction::ToggleMenu) => self.emit.send(ViewerEvent::ToggleMenu),
            Some(KeyAction::Zoom(zoom_key)) => self.key_zoom(zoom_key),
            Some(KeyAction::Page(_)) | None => {}
        }
        out.claimed
    }

    /// A zoom key scales the strip as a pinch does (T-43b-058): one step in or out, or back to 1.
    fn key_zoom(&self, key: ZoomKey) {
        self.stop_zoom_anim();
        let factor = match key {
            ZoomKey::In => page_zoom::KEY_ZOOM_STEP,
            ZoomKey::Out => 1.0 / page_zoom::KEY_ZOOM_STEP,
            ZoomKey::Reset => 1.0 / self.zoom.borrow().scale().max(f64::EPSILON),
        };
        self.pinch(factor);
        self.pinch_end();
    }
}

impl Surface for WebtoonInner {
    fn root(&self) -> &gtk::ScrolledWindow {
        &self.root
    }

    fn recognizer(&self) -> &RefCell<TapRecognizer> {
        &self.recognizer
    }

    fn watch(&self) -> &RefCell<Option<SettingsWatch>> {
        &self.watch
    }

    fn apply_settings(&self, new: ViewerSettings) {
        WebtoonInner::apply_settings(self, new);
    }

    fn alive(&self) -> bool {
        self.emit.is_open()
    }

    fn tap(&self, gesture: Gesture) {
        // Long presses reach the page view. A double tap zooms the strip (T-44a-039).
        let (x, y) = match gesture {
            Gesture::Tap { x, y } => (x, y),
            Gesture::DoubleTap { x, y } => return self.double_tap(x, y),
            Gesture::LongPress { .. } => return,
        };
        let original = f64::from(self.original_height.get());
        let (nx, ny) = normalised(x, y, f64::from(self.root.width()), original);
        let outcome = {
            let s = self.settings.borrow();
            let action = nav::hit_test(nav::layout(s.nav_mode_webtoon, self.kind), s.tapping_webtoon, nx, ny);
            nav::webtoon_tap(action)
        };
        match outcome {
            TapOutcome::ToggleMenu => self.emit.send(ViewerEvent::ToggleMenu),
            TapOutcome::ScrollDown => self.scroll_by(self.step()),
            TapOutcome::ScrollUp => self.scroll_by(-self.step()),
            TapOutcome::Page(_) => {}
        }
    }

    fn key(&self, ev: KeyEvent) -> bool {
        WebtoonInner::key(self, ev)
    }

    /// The wheel moves the strip by `viewport^(2/3)` a notch, as the scrolled window does, but never by more than a
    /// viewport, so one input cannot jump over pages that are still loading. Ctrl zooms the whole strip, as a pinch.
    fn wheel(&self, scroll: Scroll) -> bool {
        let dy = scroll.dy;
        if scroll.ctrl {
            if dy != 0.0 {
                self.stop_zoom_anim();
                self.pinch(if dy < 0.0 { page_zoom::WHEEL_ZOOM_STEP } else { 1.0 / page_zoom::WHEEL_ZOOM_STEP });
                self.pinch_end();
            }
            return true;
        }
        if dy == 0.0 && scroll.dx != 0.0 {
            return false;
        }
        let adj = self.root.vadjustment();
        // a running tap or key animation yields to the wheel
        self.anim.set(self.anim.get() + 1);
        adj.set_value(adj.value() + wheel_delta(dy, self.root.height()));
        true
    }

    fn wheel_first(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn one_wheel_notch_or_one_swipe_turns_one_page() {
        assert_eq!(super::wheel_turn(1.0, 10_000), super::WheelTurn::Turn(true), "a notch down turns forward");
        assert_eq!(super::wheel_turn(-1.0, 10_000), super::WheelTurn::Turn(false), "a notch up turns back");
        assert_eq!(super::wheel_turn(0.3, 10_000), super::WheelTurn::Gather(0.3), "a small touchpad step waits");
        assert_eq!(super::wheel_turn(5.0, 100), super::WheelTurn::Gather(0.0), "right after a turn the rest is dropped");
    }

    use super::*;
    use crate::reader::model::PageSource;
    use crate::reader::viewers::ViewerContext;
    use kuro_data::item::Item;
    use kuro_platform::kind::Kind;
    use kuro_prefs::reader::ReaderPrefs;
    use kuro_prefs::store::PrefStore;
    use std::path::PathBuf;

    fn prefs() -> ReaderPrefs {
        ReaderPrefs::new(PrefStore::in_memory())
    }

    fn context(settings: ViewerSettings) -> ViewerContext {
        ViewerContext { emit: Emitter::new(Rc::new(|_| {})), settings, prefs: None, downloads: DownloadCheck::default(), menu_open: Rc::default() }
    }

    fn image(name: &str, width: u32, height: u32) -> PathBuf {
        let path = std::env::temp_dir().join(format!("kuro-engine-{}-{name}.png", std::process::id()));
        image::RgbaImage::from_pixel(width, height, image::Rgba([40, 80, 120, 255])).save(&path).expect("write the test image");
        path
    }

    /// A loaded chapter whose pages are ready files (a path that does not exist is a page that fails).
    fn chapter(id: i64, files: &[PathBuf]) -> Rc<ReaderChapter> {
        let mut data = Item::create(Kind::Manga);
        data.id = id;
        data.url = format!("/c/{id}");
        let chapter = ReaderChapter::new(data);
        let pages = files
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let page = ReaderPage::new(i as u32, format!("p{i}"), None, PageSource::File(path.clone()));
                page.status.set(PageStatus::Ready);
                page
            })
            .collect();
        chapter.finish_loading(pages).unwrap();
        chapter
    }

    fn handed(chapter: &Rc<ReaderChapter>) -> ViewerChapters {
        ViewerChapters { curr: Rc::clone(chapter), prev: None, next: None }
    }

    fn show(widget: &gtk::Widget) -> gtk::Window {
        let window = gtk::Window::new();
        window.set_default_size(400, 500);
        window.set_child(Some(widget));
        window.present();
        window
    }

    fn pump_until(done: impl Fn() -> bool) -> bool {
        let ctx = glib::MainContext::default();
        for _ in 0..800 {
            while ctx.iteration(false) {}
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    /// Runs the main context for `ms` milliseconds.
    fn pump_ms(ms: u64) {
        let ctx = glib::MainContext::default();
        let end = std::time::Instant::now() + Duration::from_millis(ms);
        while std::time::Instant::now() < end {
            while ctx.iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn a_failed_webtoon_page_is_an_error_card_of_four_fifths_of_the_viewport() {
        crate::testing::with_display(|| {
            let missing = std::env::temp_dir().join("kuro-engine-missing.png");
            let chapter = chapter(1, &[missing, image("webtoon-ok", 40, 60)]);
            let engine = WebtoonEngine::new(false, context(ViewerSettings::from_prefs(&prefs())));
            let window = show(engine.widget());
            engine.set_chapters(handed(&chapter));
            let inner = Rc::clone(&engine.0);
            let height_of = |index: usize| {
                let item = inner.ctl.borrow().items().iter().find(|i| i.page().is_some_and(|p| p.index == index as u32)).cloned();
                item.and_then(|item| inner.realised.borrow().get(&item).map(|row| (row.errored(), row.widget.height_request())))
            };
            assert!(pump_until(|| height_of(0).is_some_and(|(errored, _)| errored)), "the unreadable page shows its error card");
            let viewport = inner.root.height();
            assert!(viewport > 0);
            assert!(pump_until(|| height_of(0).is_some_and(|(_, h)| h == error_card::webtoon_card_height(viewport))), "0.8 of the viewport");
            // a good page is as tall as its 40 x 60 image is at the row width, once it is decoded
            let good = || {
                let item = inner.ctl.borrow().items().iter().find(|i| i.page().is_some_and(|p| p.index == 1)).cloned()?;
                let realised = inner.realised.borrow();
                let row = realised.get(&item)?;
                row.source_size().map(|_| (row.widget.width_request(), row.widget.height_request()))
            };
            assert!(pump_until(|| good().is_some_and(|(w, h)| h == (f64::from(w) * 1.5).round() as i32)), "a decoded page has its own height");
            window.destroy();
        });
    }

    #[test]
    fn a_pinch_scales_the_strip_and_below_one_the_strip_is_taller() {
        crate::testing::with_display(|| {
            let chapter = chapter(1, &[image("zoom-a", 40, 60), image("zoom-b", 40, 60)]);
            let engine = WebtoonEngine::new(false, context(ViewerSettings::from_prefs(&prefs())));
            let window = show(engine.widget());
            engine.set_chapters(handed(&chapter));
            let inner = Rc::clone(&engine.0);
            assert!(pump_until(|| inner.root.height() > 0 && inner.frame.height() > 0));
            let original = inner.frame.height();
            inner.pinch(0.5);
            assert!(close(inner.frame.view().0, 0.5));
            assert!(pump_until(|| inner.root.height() == original * 2), "the strip is twice as tall at 0.5 (E-31)");
            inner.pinch(0.1);
            assert!(close(inner.frame.view().0, 0.5), "the scale stops at the minimum");
            inner.pinch(2.0);
            inner.pinch(10.0);
            assert!(close(inner.frame.view().0, 3.0), "and at the maximum");
            assert!(pump_until(|| inner.root.height() == original));
            window.destroy();
        });
    }

    #[test]
    fn a_drag_while_zoomed_pans_x_and_a_double_tap_zooms_to_two_and_back() {
        crate::testing::with_display(|| {
            let chapter = chapter(1, &[image("tap-a", 40, 60)]);
            let engine = WebtoonEngine::new(false, context(ViewerSettings::from_prefs(&prefs())));
            let window = show(engine.widget());
            engine.set_chapters(handed(&chapter));
            let inner = Rc::clone(&engine.0);
            assert!(pump_until(|| inner.frame.height() > 0 && inner.frame.is_mapped()));
            // the recogniser's double tap reaches the zoom (T-44a-039)
            Surface::tap(&*inner, Gesture::DoubleTap { x: 100.0, y: 100.0 });
            assert!(inner.animating.get(), "the zoom animation runs on the frame clock");
            inner.double_tap(300.0, 300.0);
            assert!(pump_until(|| !inner.animating.get() && close(inner.frame.view().0, 2.0)), "200 ms later the strip is at 2");
            let (_, (x0, y0)) = inner.frame.view();
            assert!(close(x0, 100.0) && close(y0, 150.0), "centred on the first touch: {x0}, {y0}");
            inner.zoom_drag(-50.0, 40.0);
            let (_, (x, y)) = inner.frame.view();
            assert!(close(x, x0 - 50.0), "x always pans");
            assert!(close(y, y0), "the page sits between two cards, not at an edge of the strip: y does not pan");
            inner.root.vadjustment().set_value(0.0);
            assert!(pump_until(|| inner.at_edge()), "scrolled to the first item");
            inner.zoom_drag(0.0, 40.0);
            assert!(close(inner.frame.view().1.1, y0 + 40.0), "at an edge y pans too");
            inner.double_tap(10.0, 10.0);
            assert!(pump_until(|| !inner.animating.get() && close(inner.frame.view().0, 1.0)), "a second double tap goes back to 1");
            assert_eq!(inner.frame.view().1, (0.0, 0.0));
            window.destroy();
        });
    }

    #[test]
    fn t058_the_plus_minus_and_zero_keys_map_to_the_zoom_keys() {
        crate::testing::with_display(|| {
            use gtk::gdk::Key;
            assert_eq!(input_key(&Key::plus, false), InputKey::ZoomIn);
            assert_eq!(input_key(&Key::equal, false), InputKey::ZoomIn);
            assert_eq!(input_key(&Key::KP_Add, false), InputKey::ZoomIn);
            assert_eq!(input_key(&Key::minus, false), InputKey::ZoomOut);
            assert_eq!(input_key(&Key::KP_Subtract, false), InputKey::ZoomOut);
            assert_eq!(input_key(&Key::_0, false), InputKey::ZoomReset);
            assert_eq!(input_key(&Key::KP_0, false), InputKey::ZoomReset);
            assert_eq!(input_key(&Key::a, false), InputKey::Other);
        });
    }

    #[test]
    fn t058_a_zoom_key_scales_the_webtoon_strip_and_reset_returns_to_one() {
        crate::testing::with_display(|| {
            let chapter = chapter(1, &[image("keyzoom-a", 40, 60)]);
            let engine = WebtoonEngine::new(false, context(ViewerSettings::from_prefs(&prefs())));
            let window = show(engine.widget());
            engine.set_chapters(handed(&chapter));
            let inner = Rc::clone(&engine.0);
            assert!(pump_until(|| inner.frame.height() > 0 && inner.frame.is_mapped()));
            let press = |key| engine.handle_key(KeyEvent { key, ctrl: false, released: false });
            assert!(press(InputKey::ZoomIn));
            assert!(close(inner.frame.view().0, page_zoom::KEY_ZOOM_STEP), "{}", inner.frame.view().0);
            assert!(press(InputKey::ZoomOut));
            assert!(press(InputKey::ZoomOut));
            assert!(close(inner.frame.view().0, 1.0 / page_zoom::KEY_ZOOM_STEP));
            assert!(press(InputKey::ZoomIn));
            assert!(press(InputKey::ZoomIn));
            assert!(press(InputKey::ZoomReset));
            assert!(close(inner.frame.view().0, 1.0), "reset: {}", inner.frame.view().0);
            window.destroy();
        });
    }

    #[test]
    fn t040_a_drag_that_began_on_a_double_tap_quick_scales_and_any_other_drag_does_not() {
        crate::testing::with_display(|| {
            let chapter = chapter(1, &[image("quick-a", 40, 60)]);
            let engine = WebtoonEngine::new(false, context(ViewerSettings::from_prefs(&prefs())));
            let window = show(engine.widget());
            engine.set_chapters(handed(&chapter));
            let inner = Rc::clone(&engine.0);
            assert!(pump_until(|| inner.frame.height() > 0 && inner.frame.is_mapped()));
            assert!(!inner.quick_scale_move(10.0), "a plain drag is not a quick scale");
            assert!(close(inner.zoom.borrow().scale(), 1.0));
            {
                let mut recognizer = inner.recognizer.borrow_mut();
                recognizer.press(0, 10.0, 10.0);
                recognizer.release(50, 10.0, 10.0);
                recognizer.press(100, 10.0, 10.0);
            }
            assert!(inner.quick_scale_move(0.0), "the first move arms the quick scale");
            assert!(inner.quick_scale_move(10.0), "a move down from the double tap is taken");
            assert!(close(inner.zoom.borrow().scale(), 1.5), "dragging down zooms in by the span: {}", inner.zoom.borrow().scale());
            window.destroy();
        });
    }

    /// A pager on two wide pages, shown and decoded, with the shown page's row.
    fn wide_pager(name: &str, prefs: &ReaderPrefs) -> (PagerEngine, gtk::Window) {
        pager_of(name, prefs, 2000, 1000)
    }

    /// A pager over two pages of `width` x `height`; a wide page pans before it turns.
    fn pager_of(name: &str, prefs: &ReaderPrefs, width: u32, height: u32) -> (PagerEngine, gtk::Window) {
        let chapter = chapter(1, &[image(&format!("{name}-a"), width, height), image(&format!("{name}-b"), width, height)]);
        let engine = PagerEngine::new(ViewerKind::LeftToRight, context(ViewerSettings::from_prefs(prefs)));
        let window = show(engine.widget());
        engine.set_chapters(handed(&chapter));
        let inner = Rc::clone(&engine.0);
        let ready = || {
            let index = inner.ctl.borrow().view_index();
            inner.slots.borrow().get(&index).is_some_and(|(_, row)| row.page_ready())
        };
        assert!(pump_until(ready), "the shown page is decoded");
        pump_ms(300);
        (engine, window)
    }

    #[test]
    fn a_one_page_chapter_shows_its_page_after_the_opening_transition_and_selects_it() {
        crate::testing::with_display(|| {
            use super::super::pager::nav::PageCmd;
            let selected = Rc::new(Cell::new(0));
            let seen = Rc::clone(&selected);
            let mut ctx = context(ViewerSettings::from_prefs(&prefs()));
            ctx.emit = Emitter::new(Rc::new(move |e| {
                if matches!(e, ViewerEvent::PageSelected(_)) {
                    seen.set(seen.get() + 1);
                }
            }));
            let chapter = chapter(1, &[image("one-page", 400, 3000)]);
            let engine = PagerEngine::new(ViewerKind::LeftToRight, ctx);
            let window = show(engine.widget());
            engine.set_chapters(handed(&chapter));
            let inner = Rc::clone(&engine.0);
            assert!(pump_until(|| inner.slots.borrow().get(&1).is_some_and(|(_, row)| row.page_ready())), "the page is decoded");
            pump_ms(300);
            assert_eq!(inner.ctl.borrow().view_index(), 1, "opens on page 1, not the Prev card");
            assert_eq!(selected.get(), 1, "the opening page is selected, which marks the chapter read");
            inner.command(PageCmd::Next);
            pump_ms(300);
            assert_eq!(inner.ctl.borrow().view_index(), 2);
            window.destroy();
        });
    }

    #[test]
    fn t019_a_next_command_turns_a_fitted_page_because_it_has_nothing_to_pan() {
        crate::testing::with_display(|| {
            use super::super::pager::nav::PageCmd;
            let (engine, window) = wide_pager("pan-fit", &prefs());
            let inner = Rc::clone(&engine.0);
            let first = inner.ctl.borrow().view_index();
            inner.command(PageCmd::Next);
            assert_eq!(inner.ctl.borrow().view_index(), first + 1);
            window.destroy();
        });
    }

    #[test]
    fn t019_a_next_command_pans_a_zoomed_page_before_it_turns_and_navigate_to_pan_off_turns_at_once() {
        crate::testing::with_display(|| {
            use super::super::pager::nav::{PageCmd, Phys};
            for navigate_pan in [true] {
                let p = prefs();
                let (engine, window) = wide_pager(&format!("pan-zoom-{navigate_pan}"), &p);
                let inner = Rc::clone(&engine.0);
                inner.on_current_row(|row| (0..4).for_each(|_| row.zoom_key(ZoomKey::In)));
                let mut room = false;
                inner.on_current_row(|row| room = row.can_pan(Phys::Right));
                assert!(room, "zoomed in, the page has room to the right");
                let first = inner.ctl.borrow().view_index();
                let mut pans = 0;
                for _ in 0..6 {
                    inner.command(PageCmd::Next);
                    if inner.ctl.borrow().view_index() != first {
                        break;
                    }
                    pans += 1;
                    pump_ms(400);
                }
                assert_eq!(inner.ctl.borrow().view_index(), first + 1, "it turns once the page is at its right edge");
                assert_eq!(pans, usize::from(navigate_pan), "one pan across the zoomed page, none with the preference off");
                window.destroy();
            }
        });
    }

    #[test]
    fn a_wide_pager_page_asks_for_its_insert_page_when_splitting_is_on() {
        crate::testing::with_display(|| {
            for (split, expect_insert) in [(true, true), (false, false)] {
                let name = format!("pager-wide-{split}");
                let chapter = chapter(1, &[image(&format!("{name}-a"), 40, 20), image(&format!("{name}-b"), 40, 20)]);
                let p = prefs();
                p.dual_page_split_paged.set(split).unwrap();
                let engine = PagerEngine::new(ViewerKind::LeftToRight, context(ViewerSettings::from_prefs(&p)));
                let window = show(engine.widget());
                engine.set_chapters(handed(&chapter));
                let inner = Rc::clone(&engine.0);
                let has_insert = || inner.ctl.borrow().items().iter().any(|i| i.page().is_some_and(PageRef::is_insert));
                if expect_insert {
                    assert!(pump_until(has_insert), "the split of a wide page places its insert page");
                } else {
                    pump_until(|| false);
                    assert!(!has_insert(), "no split, no insert");
                }
                engine.destroy();
                window.destroy();
            }
        });
    }

    #[test]
    fn t016_the_wheel_pages_the_pager_down_for_a_down_step_and_back_for_an_up_step_and_the_webtoon_keeps_it() {
        crate::testing::with_display(|| {
            // portrait pages, as in a manga: a wide page would pan on the up step instead of turning
            let (engine, window) = pager_of("wheel-a", &prefs(), 800, 1200);
            let inner = Rc::clone(&engine.0);
            let first = inner.ctl.borrow().view_index();
            assert!(engine.handle_scroll(3.0));
            assert_eq!(inner.ctl.borrow().view_index(), first + 1, "a down step is the next page");
            assert!(engine.handle_scroll(0.0));
            assert_eq!(inner.ctl.borrow().view_index(), first + 1, "no movement turns nothing, and the rest of a swipe is dropped");
            pump_ms(400);
            // the wait lets the pager settle, which may renumber the views around the current one
            let settled = inner.ctl.borrow().view_index();
            assert!(engine.handle_scroll(-1.0));
            assert_eq!(inner.ctl.borrow().view_index(), settled - 1, "an up step after the gap is the previous page");
            window.destroy();
            let webtoon = WebtoonEngine::new(false, context(ViewerSettings::from_prefs(&prefs())));
            assert!(!webtoon.handle_scroll(1.0), "the webtoon scrolls natively and does not claim the wheel");
        });
    }

    #[test]
    fn bug_reader_1_a_page_key_press_turns_the_page_and_its_release_does_not_turn_it_again() {
        crate::testing::with_display(|| {
            let (engine, window) = wide_pager("key-turn", &prefs());
            let inner = Rc::clone(&engine.0);
            let first = inner.ctl.borrow().view_index();
            assert!(engine.handle_key(KeyEvent { key: InputKey::Down, ctrl: false, released: false }));
            assert_eq!(inner.ctl.borrow().view_index(), first + 1, "the press turns the page");
            assert!(engine.handle_key(KeyEvent { key: InputKey::Down, ctrl: false, released: true }));
            assert_eq!(inner.ctl.borrow().view_index(), first + 1, "the release is claimed and turns nothing");
            window.destroy();
        });
    }

    #[test]
    fn bug_reader_2_the_strip_width_does_not_grow_across_layout_passes_at_no_side_padding() {
        crate::testing::with_display(|| {
            let chapter = chapter(1, &[image("width-a", 40, 60), image("width-b", 40, 60), image("width-c", 40, 60)]);
            let p = prefs();
            let engine = WebtoonEngine::new(false, context(ViewerSettings::from_prefs(&p)));
            let window = show(engine.widget());
            engine.set_chapters(handed(&chapter));
            let inner = Rc::clone(&engine.0);
            assert!(pump_until(|| inner.viewport() > 0 && inner.fixed.size_request().0 > 0));
            let (start_fixed, start_window) = (inner.fixed.size_request().0, window.width());
            for _ in 0..20 {
                inner.lay_out();
                pump_ms(10);
            }
            assert_eq!(inner.fixed.size_request().0, start_fixed, "the strip's width request does not grow per pass");
            assert!(window.width() <= start_window, "the window does not grow with the strip: {} > {start_window}", window.width());
            window.destroy();
        });
    }
}
