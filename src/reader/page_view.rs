//! The page view (T-43b-013, T-43b-014, T-43b-016, T-43b-024, T-44a-044, T-44a-049, T-44a-059). One page: the bytes decode
//! on the blocking pool into a pyramid of tiles, the tiles that the view shows become `gdk::Texture`s on the GTK thread,
//! and `snapshot` draws them at the zoom and centre that the pure model in `page_zoom` gives. Loading shows the indicator
//! and hides the page; a failed load emits `image-load-error`.
//!
//! Landscape auto-zoom (T-43b-018): once a wide page is fitted, 500 ms after it loads on screen or is selected, it eases to
//! the view height over 500 ms. Pan (T-43b-019): `pan_left` and `pan_right` move one view width over 250 ms. Both are
//! animations of the one zoom state, so a pinch, drag, wheel, key zoom or double tap stops them.
//!
//! Signals: `image-loaded`, `image-load-error`, `scale-changed` (f64), `view-clicked`, `long-tap` (f64, f64).
//! The long press is the tracker in `long_press`, armed by the GTK click gesture. It is reported only while
//! [`PageView::set_long_tap_enabled`] is on (`reader_long_tap`).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::subclass::prelude::ObjectSubclassIsExt;
use gtk::{gdk, gio, glib, graphene};

use crate::reader::crop_borders;
use crate::reader::decode_guard::{self, Ticket};
use kuro_core::image::background::Background;
use crate::reader::decoded_lru::{ByteLru, DEFAULT_BUDGET_BYTES};
use crate::reader::long_press::{DEFAULT_DOUBLE_CLICK_MS, DEFAULT_LONG_PRESS_MS, LongPressAction, LongPressTracker, SLOP_PX};
use crate::reader::page_decode::{self, DecodeError, LoadState, MIN_DELAY_MS};
use crate::reader::page_transform::{self, DecodeIntent};
use crate::reader::page_zoom::{self as zoom, PageConfig, Size, ZoomKey, ZoomState};
use crate::reader::progress::LoadingIndicator;
use crate::reader::page_profile;
use crate::reader::tiles::{Pyramid, Tile, current_tile_edge};
use crate::reader::viewers::page_bytes::ByteSource;
use crate::reader::viewers::pager::nav::Phys;
use crate::reader::viewers::visible;

/// The result of a decode: the pyramid of the page and whether a transform changed it, or why it failed.
/// The pyramids of a page's frames, each with its delay in milliseconds, and whether a transform changed the page.
type Frames = Vec<(Pyramid, u32)>;

type Decoded = std::thread::Result<Result<(Frames, bool, Option<u32>), DecodeError>>;

/// Decodes every frame of `bytes`, applies the transform of `intent`, cuts the uniform border when `crop` is on (T-43b-020),
/// and builds a pyramid per frame. A webtoon page is cut to `width` after that (never enlarged). A page with an edge past
/// the D-9 limit of 16384 px is shrunk to it before the pyramid is built. All of it runs on the worker; the textures are
/// made later on the GTK thread.
fn decode_animation(bytes: &[u8], width: Option<u32>, intent: DecodeIntent, crop: bool) -> Result<(Frames, bool, Option<u32>), DecodeError> {
    let mut changed = false;
    let mut frames = Vec::new();
    // read once per page: a change of the threshold applies to pages decoded after it (T-44a-060)
    let tile_edge = current_tile_edge();
    for (index, frame) in page_decode::decode_frames(bytes)?.into_iter().enumerate() {
        let (page, split) = page_transform::apply(page_profile::apply(frame.page), intent);
        if index == 0 {
            changed = split;
        }
        let page = if crop { crop_borders::crop_borders(page) } else { page };
        let page = match width {
            Some(width) => page_decode::fit_to_width(page, width),
            None => page,
        };
        let page = page_decode::limit_edge(page, page_decode::MAX_TEXTURE_EDGE);
        frames.push((Pyramid::new(page, tile_edge), frame.delay_ms));
    }
    Ok((frames, changed, page_decode::loop_passes(bytes)))
}

/// The first frame of [`decode_animation`] as a still page.
#[cfg(test)]
fn decode_pyramid(bytes: &[u8], width: Option<u32>, intent: DecodeIntent, crop: bool) -> Result<(Pyramid, bool), DecodeError> {
    let (mut frames, changed, _) = decode_animation(bytes, width, intent, crop)?;
    Ok((frames.swap_remove(0).0, changed))
}

/// What identifies a decode in the cache: where the bytes come from, what was done to them, whether the border was cut,
/// and the width they were cut to (T-44a-069).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct DecodeKey {
    source: ByteSource,
    intent: DecodeIntent,
    crop: bool,
    width: Option<u32>,
}

/// A cached decode. A page that shows it holds its own share of the pyramid, which is how the cache knows the entry is
/// in use.
#[derive(Clone)]
struct Cached {
    /// The first frame, which the page shows until the frames start to turn.
    pyramid: Rc<Pyramid>,
    /// Every frame with its delay (one entry for a still page).
    frames: Vec<(Rc<Pyramid>, u32)>,
    /// How many times the frames play through; `None` loops forever.
    passes: Option<u32>,
    split: bool,
}

thread_local! {
    /// The decoded pages kept by byte budget. A page that leaves the realised window drops its widget but not its pixels.
    static DECODED: RefCell<ByteLru<DecodeKey, Cached>> = RefCell::new(ByteLru::new(DEFAULT_BUDGET_BYTES));
}

fn cache_get(key: &DecodeKey) -> Option<Cached> {
    DECODED.with(|c| c.borrow_mut().get(key))
}

fn cache_put(key: DecodeKey, cached: &Cached) {
    let bytes = cached.frames.iter().map(|(pyramid, _)| pyramid.byte_size()).sum();
    DECODED.with(|c| c.borrow_mut().insert(key, cached.clone(), bytes, |e| Rc::strong_count(&e.pyramid) > 1));
}

/// Forgets every decoded page. A new reader session starts with none.
pub fn clear_decoded_cache() {
    DECODED.with(|c| c.borrow_mut().clear());
}

/// Paints a background pick: `Solid` as one colour, `VerticalBands` as a four-stop gradient at 0, 1/3, 2/3 and 1.
fn paint_fill(snapshot: &gtk::Snapshot, fill: &Background, bounds: &graphene::Rect) {
    let rgba = |c: kuro_core::image::background::Rgba| gdk::RGBA::new(f32::from(c.0) / 255.0, f32::from(c.1) / 255.0, f32::from(c.2) / 255.0, f32::from(c.3) / 255.0);
    match fill {
        Background::Solid(c) => snapshot.append_color(&rgba(*c), bounds),
        Background::VerticalBands { .. } => {
            let stops = fill.stops().map(|(offset, c)| gtk::gsk::ColorStop::new(offset, rgba(c)));
            snapshot.append_linear_gradient(bounds, &graphene::Point::new(bounds.x(), bounds.y()), &graphene::Point::new(bounds.x(), bounds.y() + bounds.height()), &stops);
        }
    }
}

/// Uploads one tile as a texture. `None` for a tile outside its level.
fn upload(pyramid: &Pyramid, tile: Tile) -> Option<gdk::Texture> {
    let bytes = pyramid.tile_bytes(tile);
    if bytes.is_empty() {
        return None;
    }
    let stride = tile.w as usize * 4;
    let bytes = glib::Bytes::from_owned(bytes);
    let texture = gdk::MemoryTexture::new(tile.w as i32, tile.h as i32, gdk::MemoryFormat::R8g8b8a8, &bytes, stride);
    Some(texture.upcast::<gdk::Texture>())
}

mod imp {
    use super::*;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub struct PageView {
        /// The decoded page as a pyramid of tiles (level 0 is the full page). Only the visible tiles are uploaded.
        pub pyramid: RefCell<Option<Rc<Pyramid>>>,
        /// Uploaded tiles. A tile that leaves the view is dropped on the next frame (T-44a-049).
        pub tiles: RefCell<HashMap<Tile, gdk::Texture>>,
        /// Every frame of an animated page with its delay in ms; a still page holds one (T-44a-051).
        pub frames: RefCell<Vec<(Rc<Pyramid>, u32)>>,
        /// The index of the frame on screen.
        pub frame_index: Cell<usize>,
        /// Bumped when the frames change or the page is dropped, so a frame timer left from before does nothing.
        pub frame_token: Cell<u64>,
        /// How many times the frames play through (`None` is forever), and how many passes have finished.
        pub passes: Cell<Option<u32>>,
        pub passes_done: Cell<u32>,
        pub src: Cell<Size>,
        pub state: Cell<LoadState>,
        /// Bumped by every load, recycle and cancelled timer, so a late result or timer is dropped.
        pub generation: Cell<u64>,
        pub config: Cell<PageConfig>,
        /// The view size the zoom was last fitted to. `None` means the zoom resets to the fit and the anchor.
        pub fitted: Cell<Option<(i32, i32)>>,
        /// The zoom of the page: scale, centre, fitted minimum, maximum and double-tap level (T-44a-044).
        pub zoom: Cell<ZoomState>,
        /// Whether a decode is cut to the view width first (webtoon pages, B§8.4).
        pub decode_to_view: Cell<bool>,
        /// Whether a decode cuts the uniform border off the page (`crop_borders`, `crop_borders_webtoon`).
        pub crop_borders: Cell<bool>,
        /// Whether a wide page zooms to the view height after it loads (`landscape_zoom`).
        pub landscape_zoom: Cell<bool>,
        /// Set for a webtoon page: it does not zoom on its own, the strip does (T-44a-039).
        pub zoom_locked: Cell<bool>,
        pub background: RefCell<Option<gdk::RGBA>>,
        /// The theme is Automatic: a loaded page paints what `choose_background` picks for it (T-13a-029).
        pub auto_background: Cell<bool>,
        /// The pick for the page on show, when `auto_background` is on.
        pub page_fill: RefCell<Option<Background>>,
        pub long: RefCell<Option<LongPressTracker>>,
        pub long_gen: Cell<u64>,
        pub long_fired: Cell<bool>,
        pub long_tap_enabled: Cell<bool>,
        /// The reader menu's flag; a long press reports while it is shown, whatever `long_tap_enabled` says (E-21).
        pub menu_open: RefCell<Rc<Cell<bool>>>,
        pub press_at: Cell<(f64, f64)>,
        pub pointer: Cell<Option<(f64, f64)>>,
        pub pinch_start: Cell<f64>,
        pub drag_start: Cell<(f64, f64)>,
        pub indicator: RefCell<Option<LoadingIndicator>>,
        /// Bumped to stop a running zoom animation.
        pub anim_gen: Cell<u64>,
        pub double_tap_ms: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PageView {
        const NAME: &'static str = "KuroReaderPageView";
        type Type = super::PageView;
        type ParentType = gtk::Widget;

        fn class_init(class: &mut Self::Class) {
            class.set_layout_manager_type::<gtk::BinLayout>();
        }
    }

    impl ObjectImpl for PageView {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_hexpand(true);
            obj.set_vexpand(true);
            let indicator = LoadingIndicator::new();
            indicator.widget().set_parent(&*obj);
            *self.indicator.borrow_mut() = Some(indicator);
            *self.long.borrow_mut() = Some(long_press_tracker());
            self.install_controllers();
        }

        fn dispose(&self) {
            if let Some(indicator) = self.indicator.take() {
                indicator.widget().unparent();
            }
        }

        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<glib::subclass::Signal>> = std::sync::OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("image-loaded").build(),
                    glib::subclass::Signal::builder("image-load-error").build(),
                    glib::subclass::Signal::builder("page-split").build(),
                    glib::subclass::Signal::builder("scale-changed").param_types([glib::Type::F64]).build(),
                    glib::subclass::Signal::builder("view-clicked").build(),
                    glib::subclass::Signal::builder("long-tap").param_types([glib::Type::F64, glib::Type::F64]).build(),
                ]
            })
        }
    }

    impl WidgetImpl for PageView {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let (w, h) = (f64::from(obj.width()), f64::from(obj.height()));
            if w > 0.0 && h > 0.0 {
                self.fit_to(Size { w, h });
            }
            if self.state.get() == LoadState::Ready {
                self.draw_page(snapshot, w, h);
            }
            if let Some(child) = obj.first_child() {
                obj.snapshot_child(&child, snapshot);
            }
        }
    }

    impl PageView {
        pub(super) fn view(&self) -> Size {
            let obj = self.obj();
            Size { w: f64::from(obj.width()), h: f64::from(obj.height()) }
        }

        /// Draws the visible tiles at the zoom. Tiles that are not visible are released first (T-44a-049).
        fn draw_page(&self, snapshot: &gtk::Snapshot, w: f64, h: f64) {
            let pyramid = self.pyramid.borrow();
            let Some(pyramid) = pyramid.as_ref() else { return };
            let z = self.zoom.get();
            let bounds = graphene::Rect::new(0.0, 0.0, w as f32, h as f32);
            snapshot.push_clip(&bounds);
            // A webtoon page (it never zooms on its own) is stretched onto its row, which the strip sized from the
            // page's own aspect: off by under a pixel, so nothing shows, and no background line is left at its ends
            // (issue 4). The page covers the row, so no background is painted under it either.
            let src = self.src.get();
            let fill = (self.zoom_locked.get() && src.w > 0.0 && src.h > 0.0).then(|| (w / src.w, h / src.h));
            match (fill, self.page_fill.borrow().as_ref(), *self.background.borrow()) {
                (Some(_), _, _) => {}
                (None, Some(fill), _) => paint_fill(snapshot, fill, &bounds),
                (None, None, Some(background)) => snapshot.append_color(&background, &bounds),
                (None, None, None) => {}
            }
            let (_, visible) = pyramid.visible_tiles(z.scale, z.center, (w, h));
            let mut uploaded = self.tiles.borrow_mut();
            uploaded.retain(|tile, _| visible.contains(tile));
            for tile in visible {
                if !uploaded.contains_key(&tile) {
                    if let Some(texture) = upload(pyramid, tile) {
                        uploaded.insert(tile, texture);
                    }
                }
                let Some(texture) = uploaded.get(&tile) else { continue };
                let (x, y, tw, th) = match fill {
                    Some((sx, sy)) => Pyramid::fill_rect(tile, sx, sy),
                    None => Pyramid::screen_rect(tile, z.scale, z.center, (w, h)),
                };
                snapshot.append_texture(texture, &graphene::Rect::new(x as f32, y as f32, tw as f32, th as f32));
            }
            snapshot.pop();
        }

        /// Fits the zoom to the view. A new page or a reset starts at the fit and the zoom-start anchor. A resize keeps
        /// a fitted page fitted and otherwise keeps the zoom, clamped to the new range.
        fn fit_to(&self, view: Size) {
            if self.state.get() != LoadState::Ready {
                return;
            }
            let key = (view.w as i32, view.h as i32);
            let src = self.src.get();
            let config = self.config.get();
            let min = zoom::min_scale(config.scale, view, src);
            let before = self.zoom.get();
            let mut z = match self.fitted.get() {
                Some(previous) if previous == key => return,
                Some(_) => {
                    let scale = if before.is_fitted() { min } else { before.scale };
                    let kept = ZoomState { scale, center: before.center, min, max: min * zoom::MAX_ZOOM_FACTOR, double_tap: zoom::double_tap_level(min) };
                    ZoomState { scale: kept.clamp(kept.scale), ..kept }
                }
                None => ZoomState::fitted(min, zoom::anchor_center(config.zoom_start, src)),
            };
            z.center = zoom::clamp_center(z.center, view, src, z.scale);
            self.zoom.set(z);
            self.fitted.set(Some(key));
            if (z.scale - before.scale).abs() > 1e-9 {
                // Every scale change reports, the first fit included (behaviour section 9.1). Deferred, because this
                // runs inside a snapshot.
                let weak = self.obj().downgrade();
                glib::idle_add_local_once(move || {
                    on_page(&weak, |p| {
                        let scale = p.zoom.get().scale;
                        p.obj().emit_by_name::<()>("scale-changed", &[&scale]);
                    });
                });
            }
        }

        pub(super) fn set_state(&self, state: LoadState) {
            self.state.set(state);
            if let Some(indicator) = self.indicator.borrow().as_ref() {
                if state == LoadState::Loading {
                    indicator.show();
                } else {
                    indicator.hide();
                }
            }
            self.obj().queue_draw();
        }

        /// Drops the pyramid and every uploaded tile.
        pub(super) fn drop_pages(&self) {
            self.pyramid.replace(None);
            self.tiles.borrow_mut().clear();
            self.frames.borrow_mut().clear();
            self.frame_token.set(self.frame_token.get() + 1);
        }

        /// Schedules the frame after the one on screen, after the current frame's delay. A still page schedules nothing.
        fn schedule_frame(&self) {
            let delay = {
                let frames = self.frames.borrow();
                if frames.len() < 2 {
                    return;
                }
                frames[self.frame_index.get() % frames.len()].1.max(MIN_DELAY_MS)
            };
            let (weak, token) = (self.obj().downgrade(), self.frame_token.get());
            glib::timeout_add_local_once(Duration::from_millis(u64::from(delay)), move || {
                if let Some(page) = weak.upgrade() {
                    page.imp().advance_frame(token);
                }
            });
        }

        /// Shows the frame after the one on screen, looping back to the first, and schedules the next (T-44a-051).
        fn advance_frame(&self, token: u64) {
            if token != self.frame_token.get() {
                return;
            }
            let next = match self.frames.borrow().len() {
                n if n > 1 => (self.frame_index.get() + 1) % n,
                _ => return,
            };
            if next == 0 {
                self.passes_done.set(self.passes_done.get() + 1);
                if self.passes.get().is_some_and(|passes| self.passes_done.get() >= passes) {
                    return;
                }
            }
            let pyramid = Rc::clone(&self.frames.borrow()[next].0);
            self.frame_index.set(next);
            self.pyramid.replace(Some(pyramid));
            self.tiles.borrow_mut().clear();
            self.obj().queue_draw();
            self.schedule_frame();
        }

        /// Finishes a decode started by `set_image`. A result whose ticket is no longer current (another session, or
        /// the page was loaded again) is dropped without touching anything (E-40, T-43b-059). A good result goes to the
        /// decoded cache under `key` before the page shows it.
        pub(super) fn finish_load(&self, ticket: Ticket, decoded: Decoded, key: Option<DecodeKey>) {
            if !decode_guard::accepts(ticket, decode_guard::current_session(), self.generation.get()) {
                return;
            }
            match decoded {
                Ok(Ok((frames, split, passes))) if frames.first().is_some_and(|(p, _)| p.size().0 > 0 && p.size().1 > 0) => {
                    let frames: Vec<(Rc<Pyramid>, u32)> = frames.into_iter().map(|(p, delay)| (Rc::new(p), delay)).collect();
                    let cached = Cached { pyramid: Rc::clone(&frames[0].0), frames, passes, split };
                    if let Some(key) = key {
                        cache_put(key, &cached);
                    }
                    self.show_pyramid(cached);
                }
                _ => {
                    self.drop_pages();
                    self.set_state(LoadState::Error);
                    self.obj().emit_by_name::<()>("image-load-error", &[]);
                }
            }
        }

        /// Picks the page's own background when the theme is Automatic, from the decoded page (level 0) and the
        /// viewport orientation; otherwise clears it so the theme colour shows.
        pub(super) fn refresh_page_fill(&self) {
            // the automatic background reads a static page only (B§9.3), so an animated page keeps the theme colour
            let fill = if self.auto_background.get() && self.frames.borrow().len() < 2 {
                self.pyramid.borrow().as_ref().map(|pyramid| {
                    let obj = self.obj();
                    pyramid.choose_background(obj.width() > obj.height())
                })
            } else {
                None
            };
            self.page_fill.replace(fill);
        }

        /// Shows a decoded page: sizes the zoom to it and reports `image-loaded`, and `page-split` when a transform
        /// split it.
        pub(super) fn show_pyramid(&self, cached: Cached) {
            let (w, h) = cached.pyramid.size();
            self.src.set(Size { w: f64::from(w), h: f64::from(h) });
            self.pyramid.replace(Some(cached.pyramid));
            self.frames.replace(cached.frames);
            self.frame_index.set(0);
            self.passes.set(cached.passes);
            self.passes_done.set(0);
            self.frame_token.set(self.frame_token.get() + 1);
            self.schedule_frame();
            self.refresh_page_fill();
            self.tiles.borrow_mut().clear();
            self.fitted.set(None);
            self.set_state(LoadState::Ready);
            let obj = self.obj();
            obj.emit_by_name::<()>("image-loaded", &[]);
            if cached.split {
                obj.emit_by_name::<()>("page-split", &[]);
            }
            // a page that loads on screen is read forward (`onImageLoaded`)
            self.schedule_landscape_zoom(true, true);
        }

        /// Schedules the landscape auto-zoom 500 ms from now (`landscapeZoom`, T-43b-018). Nothing is scheduled when the
        /// preference is off. A user zoom or pan in the meantime bumps `anim_gen` and cancels it. `on_screen` asks for the
        /// page to be visible then, as a page that loads off screen is not zoomed.
        pub(super) fn schedule_landscape_zoom(&self, forward: bool, on_screen: bool) {
            if !self.landscape_zoom.get() {
                return;
            }
            self.stop_animation();
            let generation = self.anim_gen.get();
            let weak = self.obj().downgrade();
            glib::timeout_add_local_once(Duration::from_millis(u64::from(zoom::LANDSCAPE_ZOOM_DELAY_MS)), move || {
                on_page(&weak, |p| p.landscape_zoom(generation, forward, on_screen));
            });
        }

        /// The zoom itself: when all five preconditions hold now, eases to the view height over 500 ms.
        fn landscape_zoom(&self, generation: u64, forward: bool, on_screen: bool) {
            if self.anim_gen.get() != generation {
                return;
            }
            let (view, src, z) = (self.view(), self.src.get(), self.zoom.get());
            let ready = self.state.get() == LoadState::Ready && self.fitted.get().is_some() && view.h > 0.0;
            let config = self.config.get();
            if !zoom::landscape_zoom_applies(self.landscape_zoom.get(), config.scale, src, z.scale, z.min, ready) {
                return;
            }
            if on_screen && !visible::landscape_zoom_on_load(&*self.obj(), true, (src.w as i32, src.h as i32)) {
                return;
            }
            let centre = zoom::landscape_zoom_centre(config.zoom_start, forward, src, z.center);
            self.animate_view(zoom::landscape_zoom_scale(view, src), centre, zoom::LANDSCAPE_ZOOM_MS, zoom::ease_in_out);
        }

        /// How far the page can pan in each direction, in screen px, `(left, right)`.
        pub(super) fn pan_room(&self) -> (f64, f64) {
            if self.state.get() != LoadState::Ready || self.fitted.get().is_none() {
                return (0.0, 0.0);
            }
            let z = self.zoom.get();
            zoom::pan_remaining(z.center, self.view(), self.src.get(), z.scale)
        }

        /// The centre after moving the view by `(dx, dy)` screen px, held inside the page. `None` when the page would
        /// move by half a pixel or less, is not ready, or does not pan on its own (a webtoon page).
        pub(super) fn pan_target(&self, dx: f64, dy: f64) -> Option<(f64, f64)> {
            let z = self.zoom.get();
            if self.state.get() != LoadState::Ready || self.fitted.get().is_none() || self.zoom_locked.get() || z.scale <= 0.0 {
                return None;
            }
            let (cx, cy) = z.center;
            let to = zoom::clamp_center((cx + dx / z.scale, cy + dy / z.scale), self.view(), self.src.get(), z.scale);
            let moved = ((to.0 - cx) * z.scale).abs().max(((to.1 - cy) * z.scale).abs());
            (moved > 0.5).then_some(to)
        }

        /// Moves the page one view width toward `dir` over 250 ms, easing out (T-43b-019). Does nothing before the page is
        /// ready. Any zoom or drag that follows stops it.
        pub(super) fn pan(&self, dir: Phys) {
            if self.state.get() != LoadState::Ready || self.fitted.get().is_none() {
                return;
            }
            let z = self.zoom.get();
            let sign = if dir == Phys::Right { 1.0 } else { -1.0 };
            let centre = zoom::pan_step_centre(z.center, self.view(), z.scale, sign);
            self.animate_view(z.scale, centre, zoom::PAN_MS, zoom::ease_out_quad);
        }

        /// Eases the zoom to `scale` and the centre to `centre` over `duration_ms` with `ease`. The centre is held inside
        /// the page on every frame, so a pan stops at the edge. `scale-changed` is reported while the scale moves.
        pub(super) fn animate_view(&self, scale: f64, centre: (f64, f64), duration_ms: u32, ease: fn(f64) -> f64) {
            self.stop_animation();
            let generation = self.anim_gen.get();
            let (view, src) = (self.view(), self.src.get());
            let start = self.zoom.get();
            let to = start.clamp(scale);
            let began = std::time::Instant::now();
            let weak = self.obj().downgrade();
            glib::timeout_add_local(Duration::from_millis(16), move || {
                let Some(page) = weak.upgrade() else { return glib::ControlFlow::Break };
                let p = page.imp();
                if p.anim_gen.get() != generation {
                    return glib::ControlFlow::Break;
                }
                let t = began.elapsed().as_secs_f64() * 1000.0 / f64::from(duration_ms.max(1));
                let eased = ease(t);
                let now = start.scale + (to - start.scale) * eased;
                let at = (start.center.0 + (centre.0 - start.center.0) * eased, start.center.1 + (centre.1 - start.center.1) * eased);
                let before = p.zoom.get().scale;
                p.zoom.set(ZoomState { scale: now, center: zoom::clamp_center(at, view, src, now), ..p.zoom.get() });
                page.queue_draw();
                if (now - before).abs() > 1e-9 {
                    page.emit_by_name::<()>("scale-changed", &[&now]);
                }
                if t >= 1.0 { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
            });
        }

        fn install_controllers(&self) {
            let obj = self.obj();

            let click = gtk::GestureClick::new();
            let weak = obj.downgrade();
            click.connect_pressed(move |g, n, x, y| {
                let t = g.current_event_time();
                on_page(&weak, |p| p.press(n, x, y, t));
            });
            let weak = obj.downgrade();
            click.connect_released(move |g, n, x, y| {
                let t = g.current_event_time();
                on_page(&weak, |p| p.release(n, x, y, t));
            });
            let weak = obj.downgrade();
            click.connect_cancel(move |_, _| on_page(&weak, |p| p.cancel()));
            obj.add_controller(click);

            // a right-click opens the page actions like a long press does (T-43a-045); it is deliberate, so it does
            // not wait for the long-press setting
            let secondary = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
            let weak = obj.downgrade();
            secondary.connect_pressed(move |_, _, x, y| {
                on_page(&weak, |p| p.obj().emit_by_name::<()>("long-tap", &[&x, &y]));
            });
            obj.add_controller(secondary);

            let motion = gtk::EventControllerMotion::new();
            let weak = obj.downgrade();
            motion.connect_motion(move |_, x, y| on_page(&weak, |p| p.motion(x, y)));
            obj.add_controller(motion);

            let drag = gtk::GestureDrag::new();
            let weak = obj.downgrade();
            drag.connect_drag_begin(move |_, _, _| on_page(&weak, |p| p.drag_begin()));
            let weak = obj.downgrade();
            drag.connect_drag_update(move |_, dx, dy| on_page(&weak, |p| p.drag_update(dx, dy)));
            obj.add_controller(drag);

            let pinch = gtk::GestureZoom::new();
            let weak = obj.downgrade();
            pinch.connect_begin(move |_, _| on_page(&weak, |p| p.pinch_begin()));
            let weak = obj.downgrade();
            pinch.connect_scale_changed(move |_, factor| on_page(&weak, |p| p.pinch(factor)));
            obj.add_controller(pinch);

            let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
            let weak = obj.downgrade();
            scroll.connect_scroll(move |c, _, dy| {
                if !c.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                    return glib::Propagation::Proceed;
                }
                on_page(&weak, |p| p.wheel(dy < 0.0));
                glib::Propagation::Stop
            });
            obj.add_controller(scroll);
        }

        pub(super) fn press(&self, n: i32, x: f64, y: f64, t: u32) {
            self.press_at.set((x, y));
            self.pointer.set(Some((x, y)));
            if n >= 2 {
                // The second tap of a double tap: no long press, and the pending timer (if any) is dropped.
                self.long_action(|tr| tr.on_cancel());
                self.bump_long();
                if !self.zoom_locked.get() {
                    self.double_tap(x, y);
                }
                return;
            }
            self.long_fired.set(false);
            if let LongPressAction::Arm { after_ms } = self.long_action(|tr| tr.on_press(t, x, y)) {
                self.arm(after_ms);
            }
        }

        fn release(&self, n: i32, x: f64, y: f64, t: u32) {
            self.long_action(|tr| tr.on_release(t));
            self.bump_long();
            let (px, py) = self.press_at.get();
            let moved = (x - px).abs() > SLOP_PX || (y - py).abs() > SLOP_PX;
            if n == 1 && !self.long_fired.get() && !moved {
                self.obj().emit_by_name::<()>("view-clicked", &[]);
            }
        }

        fn cancel(&self) {
            self.long_action(|tr| tr.on_cancel());
            self.bump_long();
        }

        fn motion(&self, x: f64, y: f64) {
            self.pointer.set(Some((x, y)));
            if self.long_action(|tr| tr.on_motion(x, y)) == LongPressAction::Disarm {
                self.bump_long();
            }
        }

        fn long_action(&self, f: impl FnOnce(&mut LongPressTracker) -> LongPressAction) -> LongPressAction {
            self.long.borrow_mut().as_mut().map_or(LongPressAction::None, f)
        }

        fn bump_long(&self) {
            self.long_gen.set(self.long_gen.get() + 1);
        }

        fn arm(&self, after_ms: u32) {
            let generation = self.long_gen.get() + 1;
            self.long_gen.set(generation);
            let weak = self.obj().downgrade();
            glib::timeout_add_local_once(Duration::from_millis(u64::from(after_ms)), move || {
                on_page(&weak, |p| p.long_timer(generation));
            });
        }

        fn long_timer(&self, generation: u64) {
            if self.long_gen.get() != generation {
                return;
            }
            let confirmed = self.long.borrow_mut().as_mut().is_some_and(|tr| tr.fire());
            if !confirmed {
                return;
            }
            self.long_fired.set(true);
            if self.long_tap_enabled.get() || self.menu_open.borrow().get() {
                let (x, y) = self.press_at.get();
                self.obj().emit_by_name::<()>("long-tap", &[&x, &y]);
            }
        }

        fn double_tap(&self, x: f64, y: f64) {
            if self.state.get() != LoadState::Ready {
                return;
            }
            let z = self.zoom.get();
            // an animated page toggles between the fit and twice the fit (PhotoView levels 1, 2, 5, B§10.6)
            let target = if self.frames.borrow().len() > 1 {
                zoom::animated_double_tap_target(z.scale / z.min) * z.min
            } else {
                zoom::double_tap_target(z.scale, z.min)
            };
            let enabled = gtk::Settings::default().is_none_or(|s| s.is_gtk_enable_animations());
            self.animate_zoom(target, (x, y), zoom::animation_ms(self.double_tap_ms.get(), enabled));
        }

        pub(super) fn wheel(&self, up: bool) {
            if self.state.get() != LoadState::Ready || self.zoom_locked.get() {
                return;
            }
            let focus = self.pointer.get().unwrap_or_else(|| {
                let view = self.view();
                (view.w / 2.0, view.h / 2.0)
            });
            let z = self.zoom.get();
            let target = zoom::wheel_zoom(z.scale, z.min, up);
            self.set_zoom(target, focus);
        }

        pub(super) fn pinch_begin(&self) {
            if self.zoom_locked.get() {
                return;
            }
            self.stop_animation();
            self.pinch_start.set(self.zoom.get().scale);
            self.cancel();
        }

        pub(super) fn pinch(&self, factor: f64) {
            if self.state.get() != LoadState::Ready || self.zoom_locked.get() {
                return;
            }
            let view = self.view();
            self.set_zoom(zoom::pinch_scale(self.pinch_start.get(), factor, &self.zoom.get()), (view.w / 2.0, view.h / 2.0));
        }

        fn drag_begin(&self) {
            if self.zoom_locked.get() {
                return;
            }
            self.stop_animation();
            self.drag_start.set(self.zoom.get().center);
        }

        fn drag_update(&self, dx: f64, dy: f64) {
            let mut z = self.zoom.get();
            if self.state.get() != LoadState::Ready || z.scale <= 0.0 || self.zoom_locked.get() {
                return;
            }
            let (sx, sy) = self.drag_start.get();
            let view = self.view();
            z.center = zoom::clamp_center((sx - dx / z.scale, sy - dy / z.scale), view, self.src.get(), z.scale);
            self.zoom.set(z);
            self.obj().queue_draw();
        }

        pub(super) fn stop_animation(&self) {
            self.anim_gen.set(self.anim_gen.get() + 1);
        }

        /// Zooms to `target` over `duration_ms` (T-43b-016), the source point under `focus` staying put. A duration of
        /// 1 ms or less zooms at once.
        pub(super) fn animate_zoom(&self, target: f64, focus: (f64, f64), duration_ms: u32) {
            if duration_ms <= 1 {
                self.set_zoom(target, focus);
                return;
            }
            self.stop_animation();
            let generation = self.anim_gen.get();
            let (view, src) = (self.view(), self.src.get());
            let start = self.zoom.get();
            let (from, begin) = (start.scale, start.center);
            let to = start.clamp(target);
            let began = std::time::Instant::now();
            let weak = self.obj().downgrade();
            glib::timeout_add_local(Duration::from_millis(16), move || {
                let Some(page) = weak.upgrade() else { return glib::ControlFlow::Break };
                let p = page.imp();
                if p.anim_gen.get() != generation {
                    return glib::ControlFlow::Break;
                }
                let elapsed = began.elapsed().as_secs_f64() * 1000.0;
                let done = elapsed >= f64::from(duration_ms);
                let scale = zoom::zoom_step(from, to, elapsed, duration_ms);
                let center = zoom::clamp_center(zoom::zoom_about(begin, view, from, scale, focus), view, src, scale);
                p.zoom.set(ZoomState { scale, center, ..p.zoom.get() });
                page.queue_draw();
                page.emit_by_name::<()>("scale-changed", &[&scale]);
                if done { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
            });
        }

        /// Sets the zoom to `target` (clamped to the page's range) while the source point under `focus` stays put.
        pub(super) fn set_zoom(&self, target: f64, focus: (f64, f64)) {
            self.stop_animation();
            let view = self.view();
            let src = self.src.get();
            let z = self.zoom.get();
            let new = z.clamp(target);
            let center = zoom::zoom_about(z.center, view, z.scale, new, focus);
            self.zoom.set(ZoomState { scale: new, center: zoom::clamp_center(center, view, src, new), ..z });
            let obj = self.obj();
            obj.queue_draw();
            obj.emit_by_name::<()>("scale-changed", &[&new]);
        }
    }

    fn long_press_tracker() -> LongPressTracker {
        let settings = gtk::Settings::default();
        let long_ms = settings.as_ref().and_then(|s| u32::try_from(s.gtk_long_press_time()).ok()).unwrap_or(DEFAULT_LONG_PRESS_MS);
        let double_ms = settings.as_ref().and_then(|s| u32::try_from(s.gtk_double_click_time()).ok()).unwrap_or(DEFAULT_DOUBLE_CLICK_MS);
        LongPressTracker::new(long_ms.max(1), double_ms)
    }

    /// Runs `f` on the page if it is still alive. Signal handlers hold weak references so that a page closed while a
    /// timer or decode is pending is not kept alive by them.
    pub(super) fn on_page(weak: &glib::WeakRef<super::PageView>, f: impl FnOnce(&PageView)) {
        if let Some(page) = weak.upgrade() {
            f(page.imp());
        }
    }
}

glib::wrapper! {
    pub struct PageView(ObjectSubclass<imp::PageView>) @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for PageView {
    fn default() -> Self {
        Self::new()
    }
}

impl PageView {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Starts decoding `bytes` for this page. A page still loading is replaced, and its result is dropped. The decode
    /// belongs to the current reader session; a result from any other session is dropped (T-43b-059).
    pub fn set_image(&self, bytes: Vec<u8>, config: PageConfig) {
        self.load(bytes, config, DecodeIntent::NONE, None);
    }

    /// Like [`set_image`], with the transform to apply on the worker. The decoded page is kept in the byte-budget cache
    /// under `source` and `intent`, so [`show_cached`] finds it again (T-44a-069).
    pub fn set_image_from(&self, source: ByteSource, bytes: Vec<u8>, config: PageConfig, intent: DecodeIntent) {
        let key = self.cache_key(source, intent);
        self.load(bytes, config, intent, Some(key));
    }

    /// Whether the cache key of this page is settled: a webtoon page is keyed by the view width, which is 0 until the
    /// page has been allocated.
    pub fn cache_key_ready(&self) -> bool {
        !self.imp().decode_to_view.get() || self.width() > 0
    }

    /// Shows the page from the decoded cache when it is there, with no decode. `false` when it is not.
    pub fn show_cached(&self, source: ByteSource, config: PageConfig, intent: DecodeIntent) -> bool {
        let Some(cached) = cache_get(&self.cache_key(source, intent)) else { return false };
        let imp = self.imp();
        imp.stop_animation();
        imp.config.set(config);
        // a decode still running for this page is stale now
        imp.generation.set(imp.generation.get() + 1);
        imp.show_pyramid(cached);
        true
    }

    fn cache_key(&self, source: ByteSource, intent: DecodeIntent) -> DecodeKey {
        let width = self.imp().decode_to_view.get().then(|| self.width().max(0) as u32).filter(|w| *w > 0);
        DecodeKey { source, intent, crop: self.imp().crop_borders.get(), width }
    }

    fn load(&self, bytes: Vec<u8>, config: PageConfig, intent: DecodeIntent, key: Option<DecodeKey>) {
        let imp = self.imp();
        imp.stop_animation();
        imp.config.set(config);
        imp.fitted.set(None);
        imp.drop_pages();
        let generation = imp.generation.get() + 1;
        imp.generation.set(generation);
        imp.set_state(LoadState::Loading);
        let ticket = Ticket::now(generation);
        let width = key.as_ref().map_or_else(|| imp.decode_to_view.get().then(|| self.width().max(0) as u32).filter(|w| *w > 0), |k| k.width);
        let crop = imp.crop_borders.get();
        let weak = self.downgrade();
        glib::spawn_future_local(async move {
            let decoded = gio::spawn_blocking(move || decode_animation(&bytes, width, intent, crop)).await;
            imp::on_page(&weak, |p| p.finish_load(ticket, decoded, key));
        });
    }

    /// Drops the pyramid and any pending decode, and shows nothing until the next `set_image`.
    pub fn recycle(&self) {
        let imp = self.imp();
        imp.stop_animation();
        imp.generation.set(imp.generation.get() + 1);
        imp.drop_pages();
        imp.fitted.set(None);
        imp.set_state(LoadState::Empty);
    }

    /// Applies new display settings to the shown page without reloading its texture (D-11).
    pub fn set_config(&self, config: PageConfig) {
        let imp = self.imp();
        imp.stop_animation();
        imp.config.set(config);
        imp.fitted.set(None);
        self.queue_draw();
    }

    /// The double-tap zoom duration in ms (`pref_double_tap_anim_speed`), before the GTK animation setting scales it.
    pub fn set_double_tap_anim_ms(&self, ms: u32) {
        self.imp().double_tap_ms.set(ms);
    }

    /// Whether the page zooms on its own. A webtoon page does not: the strip does (T-44a-039).
    pub fn set_zoomable(&self, zoomable: bool) {
        self.imp().zoom_locked.set(!zoomable);
    }

    /// Cuts decodes to the view width first, as a webtoon page does (B§8.4).
    pub fn set_decode_to_view(&self, enabled: bool) {
        self.imp().decode_to_view.set(enabled);
    }

    /// Cuts the uniform border off pages decoded from now on (`crop_borders`, `crop_borders_webtoon`). A shown page is
    /// decoded again by the caller.
    pub fn set_crop_borders(&self, on: bool) {
        self.imp().crop_borders.set(on);
    }

    /// Whether a wide fitted page zooms to the view height after it loads (`landscape_zoom`).
    pub fn set_landscape_zoom(&self, on: bool) {
        self.imp().landscape_zoom.set(on);
    }

    /// The page became the selected one: the landscape zoom runs 500 ms later (`onPageSelected`). `forward` is the
    /// direction of the selection, which decides the end of the page the zoom starts at.
    pub fn on_page_selected(&self, forward: bool) {
        self.imp().schedule_landscape_zoom(forward, false);
    }

    /// Whether the page has more than 1 px left to pan to the left (`canPanLeft`).
    pub fn can_pan_left(&self) -> bool {
        zoom::can_pan(self.imp().pan_room().0)
    }

    /// Whether the page has more than 1 px left to pan to the right (`canPanRight`).
    pub fn can_pan_right(&self) -> bool {
        zoom::can_pan(self.imp().pan_room().1)
    }

    /// Whether moving the view by `(dx, dy)` screen px would pan the page (it has room that way). A webtoon page never
    /// pans on its own.
    pub fn can_pan_by(&self, dx: f64, dy: f64) -> bool {
        self.imp().pan_target(dx, dy).is_some()
    }

    /// Moves the view by `(dx, dy)` screen px at once, as a scroll does; positive shows more of the right and bottom.
    /// Returns whether the page moved: `false` at its edge, so the viewer can turn the page instead.
    pub fn scroll_pan(&self, dx: f64, dy: f64) -> bool {
        let imp = self.imp();
        let Some(center) = imp.pan_target(dx, dy) else { return false };
        imp.stop_animation();
        imp.zoom.set(ZoomState { center, ..imp.zoom.get() });
        self.queue_draw();
        true
    }

    /// One notch of Ctrl+wheel zoom about the pointer (T-43b-016).
    pub fn zoom_wheel(&self, up: bool) {
        self.imp().wheel(up);
    }

    /// Pans one view width to the left over 250 ms. Does nothing before the page is ready.
    pub fn pan_left(&self) {
        self.imp().pan(Phys::Left);
    }

    /// Pans one view width to the right over 250 ms. Does nothing before the page is ready.
    pub fn pan_right(&self) {
        self.imp().pan(Phys::Right);
    }

    /// Zooms for a zoom key (T-43b-058), about the view centre. A webtoon page does not zoom: the strip does.
    pub fn zoom_by_key(&self, key: ZoomKey) {
        let imp = self.imp();
        if imp.state.get() != LoadState::Ready || imp.fitted.get().is_none() || imp.zoom_locked.get() {
            return;
        }
        let (z, view) = (imp.zoom.get(), imp.view());
        imp.set_zoom(zoom::key_zoom_target(z.scale, z.min, key), (view.w / 2.0, view.h / 2.0));
    }

    /// Whether `long-tap` reports (`reader_long_tap`).
    pub fn long_tap_enabled(&self) -> bool {
        self.imp().long_tap_enabled.get()
    }

    pub fn state(&self) -> LoadState {
        self.imp().state.get()
    }

    /// The size in pixels of the decoded page on show, `None` until it is ready.
    pub fn source_size(&self) -> Option<(u32, u32)> {
        let imp = self.imp();
        let src = imp.src.get();
        (imp.state.get() == LoadState::Ready && src.w >= 1.0 && src.h >= 1.0).then(|| (src.w as u32, src.h as u32))
    }

    pub fn scale(&self) -> f64 {
        self.imp().zoom.get().scale
    }

    /// The page background, drawn only once a page has loaded.
    pub fn set_page_background(&self, colour: Option<gdk::RGBA>) {
        self.imp().background.replace(colour);
        self.queue_draw();
    }

    /// Whether the page paints the background picked from its own pixels (reader theme Automatic).
    pub fn set_auto_background(&self, on: bool) {
        self.imp().auto_background.set(on);
        self.imp().refresh_page_fill();
        self.queue_draw();
    }

    /// Whether a confirmed long press reports `long-tap` (`reader_long_tap`).
    pub fn set_long_tap_enabled(&self, enabled: bool) {
        self.imp().long_tap_enabled.set(enabled);
    }

    /// The reader menu's flag, read on each long press (E-21).
    pub fn set_menu_open(&self, open: Rc<Cell<bool>>) {
        self.imp().menu_open.replace(open);
    }

    pub fn connect_image_loaded<F: Fn() + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("image-loaded", false, move |_| {
            f();
            None
        })
    }

    pub fn connect_image_load_error<F: Fn() + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("image-load-error", false, move |_| {
            f();
            None
        })
    }

    pub fn connect_scale_changed<F: Fn(f64) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("scale-changed", false, move |values| {
            f(values[1].get::<f64>().unwrap_or_default());
            None
        })
    }

    /// Reports that a transform split the page in two, so the viewer places its insert page.
    pub fn connect_page_split<F: Fn() + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("page-split", false, move |_| {
            f();
            None
        })
    }

    pub fn connect_view_clicked<F: Fn() + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("view-clicked", false, move |_| {
            f();
            None
        })
    }

    /// Reports a confirmed long press with its press position.
    pub fn connect_long_tap<F: Fn(f64, f64) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("long-tap", false, move |values| {
            f(values[1].get::<f64>().unwrap_or_default(), values[2].get::<f64>().unwrap_or_default());
            None
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(width, height, image::Rgba([200, 30, 30, 255]));
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut out, image::ImageFormat::Png).expect("encode");
        out.into_inner()
    }

    /// Runs the main context for a while, so frames are drawn and fitted.
    fn pump(ctx: &glib::MainContext) {
        for _ in 0..100 {
            while ctx.iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Runs the main context until the page leaves `Loading`, or gives up after a few seconds.
    fn settle(page: &PageView) {
        let ctx = glib::MainContext::default();
        for _ in 0..2000 {
            if page.state() != LoadState::Loading {
                return;
            }
            ctx.iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_page_loads_and_recycle_drops_it_then_a_new_one_loads() {
        crate::testing::with_display(|| {
            let page = PageView::new();
            assert_eq!(page.state(), LoadState::Empty);
            page.set_image(png(3, 2), PageConfig::default());
            assert_eq!(page.state(), LoadState::Loading);
            settle(&page);
            assert_eq!(page.state(), LoadState::Ready);
            page.recycle();
            assert_eq!(page.state(), LoadState::Empty);
            page.set_image(png(4, 4), PageConfig::default());
            settle(&page);
            assert_eq!(page.state(), LoadState::Ready);
        });
    }

    #[test]
    fn a_corrupt_page_enters_the_error_state_and_emits_the_signal() {
        crate::testing::with_display(|| {
            let page = PageView::new();
            let failed = std::rc::Rc::new(Cell::new(false));
            let seen = std::rc::Rc::clone(&failed);
            page.connect_image_load_error(move || seen.set(true));
            page.set_image(b"not an image".to_vec(), PageConfig::default());
            settle(&page);
            assert_eq!(page.state(), LoadState::Error);
            assert!(failed.get());
        });
    }

    #[test]
    fn e01_a_tall_page_is_fitted_to_the_view_when_shown() {
        crate::testing::with_display(|| {
            let page = PageView::new();
            let window = gtk::Window::new();
            window.set_default_size(800, 600);
            window.set_child(Some(&page));
            window.present();
            page.set_image(png(1000, 2000), PageConfig::default());
            settle(&page);
            let ctx = glib::MainContext::default();
            pump(&ctx);
            // FitScreen on a 1000x2000 page in an 800x600 view: min(0.8, 0.3) = 0.3.
            assert!((page.scale() - 0.3).abs() < 0.01, "scale {}", page.scale());
            window.destroy();
        });
    }

    #[test]
    fn e01_a_wide_page_is_fitted_to_the_view_when_shown() {
        crate::testing::with_display(|| {
            let page = PageView::new();
            let window = gtk::Window::new();
            window.set_default_size(800, 600);
            window.set_child(Some(&page));
            let reported = std::rc::Rc::new(Cell::new(0.0));
            let seen = std::rc::Rc::clone(&reported);
            page.connect_scale_changed(move |s| seen.set(s));
            window.present();
            page.set_image(png(2000, 1000), PageConfig::default());
            settle(&page);
            let ctx = glib::MainContext::default();
            pump(&ctx);
            // FitScreen on a 2000x1000 page in an 800x600 view: min(0.4, 0.6) = 0.4, reported by the initial fit.
            assert!((page.scale() - 0.4).abs() < 0.01, "scale {}", page.scale());
            assert!((reported.get() - 0.4).abs() < 0.01, "scale-changed reported {}", reported.get());
            window.destroy();
        });
    }

    #[test]
    fn changing_the_config_refits_without_reloading_the_texture() {
        crate::testing::with_display(|| {
            let page = PageView::new();
            let window = gtk::Window::new();
            window.set_default_size(800, 600);
            window.set_child(Some(&page));
            window.present();
            page.set_image(png(2000, 1000), PageConfig::default());
            settle(&page);
            let ctx = glib::MainContext::default();
            pump(&ctx);
            page.set_config(PageConfig { scale: zoom::ImageScaleType::Stretch, ..PageConfig::default() });
            pump(&ctx);
            assert_eq!(page.state(), LoadState::Ready, "no reload");
            assert!((page.scale() - 0.6).abs() < 0.01, "stretch fills the larger fit: {}", page.scale());
            page.set_config(PageConfig { scale: zoom::ImageScaleType::FitWidth, ..PageConfig::default() });
            pump(&ctx);
            assert!((page.scale() - 0.4).abs() < 0.01, "fit width of 2000 in 800: {}", page.scale());
            window.destroy();
        });
    }

    #[test]
    fn a_result_from_an_old_generation_or_session_is_not_applied() {
        // The guard that finish_load uses: a ticket applies only to the session and generation it was started for.
        let ticket = Ticket { session: 1, generation: 2 };
        assert!(decode_guard::accepts(ticket, 1, 2));
        assert!(!decode_guard::accepts(ticket, 1, 3));
        assert!(!decode_guard::accepts(ticket, 2, 2));
    }

    fn wide_png() -> Vec<u8> {
        png(400, 200)
    }

    fn settle_all() {
        pump(&glib::MainContext::default());
    }

    fn file(name: &str) -> ByteSource {
        ByteSource::File(std::path::PathBuf::from(format!("/page-view-tests/{name}")))
    }

    #[test]
    fn a_decoded_page_is_kept_in_the_cache_and_a_second_view_shows_it_without_decoding() {
        crate::testing::with_display(|| {
            let (source, config) = (file("cached.png"), PageConfig::default());
            let first = PageView::new();
            assert!(!first.show_cached(source.clone(), config, DecodeIntent::NONE), "nothing is cached yet");
            first.set_image_from(source.clone(), png(30, 20), config, DecodeIntent::NONE);
            settle(&first);
            assert_eq!(first.state(), LoadState::Ready);
            let second = PageView::new();
            assert!(second.show_cached(source.clone(), config, DecodeIntent::NONE));
            assert_eq!(second.state(), LoadState::Ready, "shown at once, with no decode");
            assert_eq!(second.imp().src.get(), Size { w: 30.0, h: 20.0 });
            let split = DecodeIntent { transform: page_transform::PageTransform::SplitLeft, only_if_wide: true };
            assert!(!PageView::new().show_cached(source, config, split), "another transform is another entry");
        });
    }

    #[test]
    fn a_view_holds_a_share_of_its_cached_pyramid_which_is_what_pins_it_against_eviction() {
        crate::testing::with_display(|| {
            let view = PageView::new();
            view.set_image_from(file("pinned.png"), png(8, 8), PageConfig::default(), DecodeIntent::NONE);
            settle(&view);
            let key = DecodeKey { source: file("pinned.png"), intent: DecodeIntent::NONE, crop: false, width: None };
            let cached = cache_get(&key).expect("cached");
            // each holder keeps the page twice: as the first-frame pyramid and as frame 0
            assert_eq!(Rc::strong_count(&cached.pyramid), 6, "the cache, the view and this handle");
            view.recycle();
            assert_eq!(Rc::strong_count(&cached.pyramid), 4, "the view let go; the pixels stay cached for the way back");
        });
    }

    #[test]
    fn the_transform_is_applied_on_the_worker_and_a_split_is_reported() {
        crate::testing::with_display(|| {
            let page = PageView::new();
            let splits = Rc::new(Cell::new(0));
            let seen = Rc::clone(&splits);
            page.connect_page_split(move || seen.set(seen.get() + 1));
            let split = DecodeIntent { transform: page_transform::PageTransform::SplitLeft, only_if_wide: true };
            page.set_image_from(file("wide.png"), wide_png(), PageConfig::default(), split);
            settle(&page);
            assert_eq!(page.imp().src.get(), Size { w: 200.0, h: 200.0 }, "the left half of a 400 by 200 page");
            assert_eq!(splits.get(), 1);
            // a tall page is not wide: it is kept whole and nothing is reported
            page.set_image_from(file("tall.png"), png(100, 300), PageConfig::default(), split);
            settle(&page);
            assert_eq!(page.imp().src.get(), Size { w: 100.0, h: 300.0 });
            assert_eq!(splits.get(), 1);
            // the cached page reports its split again for a row that is built later
            let again = PageView::new();
            let seen = Rc::clone(&splits);
            again.connect_page_split(move || seen.set(seen.get() + 1));
            assert!(again.show_cached(file("wide.png"), PageConfig::default(), split));
            assert_eq!(splits.get(), 2);
        });
    }

    /// A PNG of `width` by `height` with a white border of `margin` px around red ink.
    fn bordered_png(width: u32, height: u32, margin: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(width, height, |x, y| {
            let inside = x >= margin && x < width - margin && y >= margin && y < height - margin;
            if inside { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([255, 255, 255, 255]) }
        });
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut out, image::ImageFormat::Png).expect("encode");
        out.into_inner()
    }

    #[test]
    fn t020_the_decode_cuts_a_twenty_pixel_white_border_only_when_crop_is_on() {
        let bytes = bordered_png(120, 100, 20);
        let (uncropped, _) = decode_pyramid(&bytes, None, DecodeIntent::NONE, false).expect("decode");
        assert_eq!(uncropped.size(), (120, 100));
        let (cropped, _) = decode_pyramid(&bytes, None, DecodeIntent::NONE, true).expect("decode");
        assert_eq!(cropped.size(), (80, 60));
        // a solid red page is both dark and light to the border detector, so only an all-white page has no border (a margin as wide as half the page leaves no ink)
        let white = bordered_png(120, 100, 60);
        let (plain, _) = decode_pyramid(&white, None, DecodeIntent::NONE, true).expect("decode");
        assert_eq!(plain.size(), (120, 100), "a page with no uniform border is unchanged");
        let (webtoon, _) = decode_pyramid(&bytes, Some(40), DecodeIntent::NONE, true).expect("decode");
        assert_eq!(webtoon.size(), (40, 30), "the border is cut first, then the page is cut to the view width");
    }

    #[test]
    fn t020_a_page_cropped_and_one_not_are_separate_cache_entries() {
        crate::testing::with_display(|| {
            let (source, config) = (file("crop.png"), PageConfig::default());
            let page = PageView::new();
            page.set_crop_borders(true);
            page.set_image_from(source.clone(), bordered_png(120, 100, 20), config, DecodeIntent::NONE);
            settle(&page);
            assert_eq!(page.imp().src.get(), Size { w: 80.0, h: 60.0 });
            let other = PageView::new();
            assert!(!other.show_cached(source.clone(), config, DecodeIntent::NONE), "crop is off here, so the cropped decode is not used");
            other.set_crop_borders(true);
            assert!(other.show_cached(source, config, DecodeIntent::NONE));
        });
    }

    /// Runs the main context for `ms` milliseconds.
    fn run_for(ms: u64) {
        let ctx = glib::MainContext::default();
        let end = std::time::Instant::now() + Duration::from_millis(ms);
        while std::time::Instant::now() < end {
            while ctx.iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A shown 800 by 600 window with a 2000 by 1000 page loaded in it, fitted (scale 0.4).
    fn wide_page_in_window(landscape: bool) -> (PageView, gtk::Window) {
        let page = PageView::new();
        page.set_landscape_zoom(landscape);
        let window = gtk::Window::new();
        window.set_default_size(800, 600);
        window.set_child(Some(&page));
        window.present();
        run_for(300);
        page.set_image(png(2000, 1000), PageConfig::default());
        settle(&page);
        (page, window)
    }

    #[test]
    fn t018_a_wide_fitted_page_zooms_to_the_view_height_after_the_delay() {
        crate::testing::with_display(|| {
            let (page, window) = wide_page_in_window(true);
            run_for(200);
            assert!((page.scale() - 0.4).abs() < 0.01, "still at the fit before the 500 ms delay: {}", page.scale());
            run_for(1400);
            assert!((page.scale() - 0.6).abs() < 0.01, "the 600 px view height over the 1000 px page: {}", page.scale());
            window.destroy();
        });
    }

    #[test]
    fn t018_the_zoom_does_not_run_with_the_preference_off_or_after_a_user_zoom() {
        crate::testing::with_display(|| {
            let (off, window) = wide_page_in_window(false);
            run_for(1600);
            assert!((off.scale() - 0.4).abs() < 0.01, "preference off: {}", off.scale());
            window.destroy();
            let (user, window) = wide_page_in_window(true);
            run_for(200);
            user.zoom_by_key(ZoomKey::In);
            let zoomed = user.scale();
            run_for(1600);
            assert!((user.scale() - zoomed).abs() < 1e-9, "a user zoom cancels the pending landscape zoom: {}", user.scale());
            window.destroy();
        });
    }

    #[test]
    fn t019_a_zoomed_page_pans_one_view_width_and_stops_at_the_edge() {
        crate::testing::with_display(|| {
            let (page, window) = wide_page_in_window(false);
            run_for(300);
            assert!(!page.can_pan_left() && !page.can_pan_right(), "a fitted page has nothing to pan");
            for _ in 0..3 {
                page.zoom_by_key(ZoomKey::In);
            }
            assert!(page.can_pan_left() && page.can_pan_right(), "zoomed in on its centre, both sides have room");
            let before = page.imp().zoom.get();
            page.pan_right();
            run_for(150);
            let mid = page.imp().zoom.get().center.0;
            assert!(mid > before.center.0 && mid < 1900.0, "moving: {mid}");
            run_for(400);
            // one view width is 800 / 0.78125 = 1024 source px, which is past the right edge: the pan stops there
            let edge = 2000.0 - 800.0 / before.scale / 2.0;
            assert!((page.imp().zoom.get().center.0 - edge).abs() < 1.0, "at the right edge: {}", page.imp().zoom.get().center.0);
            assert!(!page.can_pan_right() && page.can_pan_left());
            page.pan_left();
            run_for(500);
            assert!((page.imp().zoom.get().center.0 - (edge - 1024.0).max(800.0 / before.scale / 2.0)).abs() < 1.0);
            window.destroy();
        });
    }

    #[test]
    fn issue_4_a_scroll_pans_a_zoomed_page_until_its_edge_and_a_webtoon_page_never() {
        crate::testing::with_display(|| {
            let (page, window) = wide_page_in_window(false);
            run_for(300);
            assert!(!page.can_pan_by(50.0, 0.0) && !page.scroll_pan(50.0, 0.0), "a fitted page has nothing to pan");
            for _ in 0..3 {
                page.zoom_by_key(ZoomKey::In);
            }
            let before = page.imp().zoom.get().center;
            assert!(page.can_pan_by(0.0, 50.0) && page.scroll_pan(0.0, 50.0), "zoomed, a scroll down pans down");
            assert!(page.imp().zoom.get().center.1 > before.1);
            while page.scroll_pan(0.0, 50.0) {}
            assert!(!page.can_pan_by(0.0, 50.0) && page.can_pan_by(0.0, -50.0), "stopped at the bottom edge only");
            page.set_zoomable(false);
            assert!(!page.can_pan_by(0.0, -50.0) && !page.scroll_pan(0.0, -50.0), "a webtoon page leaves panning to the strip");
            window.destroy();
        });
    }

    #[test]
    fn t019_a_pan_is_interruptible_and_does_nothing_before_the_page_is_ready() {
        crate::testing::with_display(|| {
            let idle = PageView::new();
            idle.pan_left();
            idle.pan_right();
            assert!(!idle.can_pan_left() && !idle.can_pan_right());
            let (page, window) = wide_page_in_window(false);
            run_for(300);
            for _ in 0..3 {
                page.zoom_by_key(ZoomKey::In);
            }
            page.pan_right();
            run_for(80);
            page.imp().stop_animation();
            let held = page.imp().zoom.get().center.0;
            run_for(400);
            assert!((page.imp().zoom.get().center.0 - held).abs() < 1e-9, "a stopped pan stays where it was stopped");
            window.destroy();
        });
    }

    #[test]
    fn t025_the_automatic_theme_paints_a_colour_computed_from_the_loaded_page_and_other_themes_do_not() {
        crate::testing::with_display(|| {
            let auto = PageView::new();
            auto.set_auto_background(true);
            assert!(auto.imp().page_fill.borrow().is_none(), "nothing is computed before a page has loaded");
            auto.set_image(png(60, 40), PageConfig::default());
            settle(&auto);
            assert!(auto.imp().page_fill.borrow().is_some(), "after load the page's own colour is picked");
            let themed = PageView::new();
            themed.set_auto_background(false);
            themed.set_image(png(60, 40), PageConfig::default());
            settle(&themed);
            assert!(themed.imp().page_fill.borrow().is_none(), "the theme colour shows");
            auto.set_auto_background(false);
            assert!(auto.imp().page_fill.borrow().is_none(), "leaving Automatic clears the computed colour");
        });
    }

    #[test]
    fn t058_zoom_keys_step_the_scale_and_reset_returns_to_the_fit() {
        crate::testing::with_display(|| {
            let (page, window) = wide_page_in_window(false);
            run_for(300);
            let fit = page.scale();
            page.zoom_by_key(ZoomKey::In);
            assert!((page.scale() - fit * zoom::KEY_ZOOM_STEP).abs() < 1e-9);
            page.zoom_by_key(ZoomKey::Out);
            page.zoom_by_key(ZoomKey::Out);
            assert!((page.scale() - fit).abs() < 1e-9, "never under the fit");
            page.zoom_by_key(ZoomKey::In);
            page.zoom_by_key(ZoomKey::Reset);
            assert!((page.scale() - fit).abs() < 1e-9);
            page.set_zoomable(false);
            page.zoom_by_key(ZoomKey::In);
            assert!((page.scale() - fit).abs() < 1e-9, "a webtoon page leaves zoom to the strip");
            window.destroy();
        });
    }

    #[test]
    fn a_webtoon_page_does_not_zoom_on_its_own() {
        crate::testing::with_display(|| {
            let page = PageView::new();
            let window = gtk::Window::new();
            window.set_default_size(800, 600);
            window.set_child(Some(&page));
            window.present();
            page.set_image(png(1000, 2000), PageConfig::default());
            settle(&page);
            settle_all();
            let fit = page.scale();
            page.set_zoomable(false);
            page.imp().pinch_begin();
            page.imp().pinch(3.0);
            assert!((page.scale() - fit).abs() < 1e-9, "locked: the pinch changes nothing");
            page.imp().press(2, 400.0, 300.0, 0);
            settle_all();
            assert!((page.scale() - fit).abs() < 1e-9, "locked: a double tap does not zoom");
            page.set_zoomable(true);
            page.imp().pinch_begin();
            page.imp().pinch(2.0);
            assert!((page.scale() - fit * 2.0).abs() < 1e-9, "unlocked: the pinch scales from the fit");
            window.destroy();
        });
    }
}
