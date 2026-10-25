//! The navigation overlay (G-27, `ReaderNavigationOverlayView.kt`): the tap zones drawn over the pages, each with its
//! colour and its label, fading in over one second and fading out on its own [`HOLD_MS`] later. A click does not hide it.
//!
//! The overlay is shown when the navigation is set for the first time and the user asked for it (`show_navigation_overlay_on_start`,
//! or the one-time new-user flag), and whenever the navigation changes after that. A disabled navigation never shows it.
//! The zones and the show rule are pure and tested; [`NavOverlay`] is the click-through `DrawingArea` over them.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::glib;
use gtk::prelude::*;
use kuro_prefs::reader::TappingInvertMode;

use super::viewers::ViewerKind;
use super::viewers::nav::{self, Action, NavMode, Rect};
use super::viewers::settings::ViewerSettings;

/// How long a fade takes (`FADE_DURATION`).
pub const FADE_MS: u64 = 1000;
/// How long the overlay stays fully shown before it fades out.
pub const HOLD_MS: u64 = 2000;
/// The label size in pixels (`textSize = 64f`); a zone too narrow for its label shrinks it.
const LABEL_SIZE: f64 = 64.0;

/// One zone of the overlay: where it is (normalised, inversion applied) and what a tap there does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zone {
    pub rect: Rect,
    pub action: Action,
}

/// The zones of a navigation, with the inversion applied. A disabled navigation has none.
pub fn zones(mode: NavMode, invert: TappingInvertMode, kind: ViewerKind) -> Vec<Zone> {
    nav::layout(mode, kind).iter().map(|region| Zone { rect: region.rect.invert(invert), action: region.action }).collect()
}

/// The zones of a viewer for the settings it reads: the pager or the webtoon navigation, as the viewer's family says.
pub fn zones_for(kind: ViewerKind, settings: &ViewerSettings) -> Vec<Zone> {
    if kind.is_pager() {
        zones(settings.nav_mode_pager, settings.tapping_pager, kind)
    } else {
        zones(settings.nav_mode_webtoon, settings.tapping_webtoon, kind)
    }
}

/// Whether setting the navigation fades the overlay in (`setNavigation`). Nothing happens while it is visible, for a disabled
/// navigation, or for the first navigation when it was not asked for.
pub fn should_fade_in(first: bool, visible: bool, wanted_on_start: bool, has_zones: bool) -> bool {
    has_zones && !visible && (wanted_on_start || !first)
}

/// The opacity `elapsed_ms` into a linear fade from `from` to `to`.
pub fn fade_alpha(from: f64, to: f64, elapsed_ms: u64) -> f64 {
    let t = (elapsed_ms as f64 / FADE_MS as f64).clamp(0.0, 1.0);
    from + (to - from) * t
}

/// The font size that fits `text` of `chars` characters into `width` pixels, at most [`LABEL_SIZE`].
fn label_size(width: f64, chars: usize) -> f64 {
    LABEL_SIZE.min((width / (chars.max(1) as f64 * 0.62)).max(8.0))
}

struct State {
    zones: Vec<Zone>,
    /// No navigation has been set yet, so the next one is the first (`firstLaunch`).
    first_set: bool,
    /// Counts fades, so a timer only continues the fade it started.
    generation: u64,
}

/// The overlay widget. It never takes input, and it fades out on its own after it was shown.
pub struct NavOverlay {
    area: gtk::DrawingArea,
    state: Rc<RefCell<State>>,
    shown: Rc<Cell<bool>>,
}

impl NavOverlay {
    /// Adds the overlay over `layer`, hidden. It never takes a click, so every click reaches the viewer.
    pub fn new(layer: &gtk::Overlay) -> Self {
        let state = Rc::new(RefCell::new(State { zones: Vec::new(), first_set: true, generation: 0 }));
        let area = gtk::DrawingArea::new();
        area.set_can_target(false);
        area.set_focusable(false);
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_visible(false);
        area.set_opacity(0.0);
        let drawn = Rc::clone(&state);
        area.set_draw_func(move |_, cr, width, height| draw(&drawn.borrow().zones, cr, f64::from(width), f64::from(height)));
        layer.add_overlay(&area);
        Self { area, state, shown: Rc::new(Cell::new(false)) }
    }

    /// Sets the zones and fades the overlay in when [`should_fade_in`] says so. `wanted_on_start` is the on-start preference
    /// or the new-user flag.
    pub fn set_navigation(&self, zones: Vec<Zone>, wanted_on_start: bool) {
        let (first, has_zones) = {
            let mut state = self.state.borrow_mut();
            let first = std::mem::replace(&mut state.first_set, false);
            let has_zones = !zones.is_empty();
            state.zones = zones;
            (first, has_zones)
        };
        self.area.queue_draw();
        if !has_zones {
            // a disabled navigation takes the overlay away
            fade(&self.area, &self.state, &self.shown, false);
        } else if should_fade_in(first, self.shown.get(), wanted_on_start, has_zones) {
            fade(&self.area, &self.state, &self.shown, true);
        }
    }

    #[cfg(test)]
    /// Whether the overlay is showing or fading in.
    pub fn is_shown(&self) -> bool {
        self.shown.get()
    }
}

/// Fades the overlay to visible or hidden over [`FADE_MS`], from its present opacity. A shown overlay fades out
/// [`HOLD_MS`] after it is fully visible, unless another fade started meanwhile.
fn fade(area: &gtk::DrawingArea, state: &Rc<RefCell<State>>, shown: &Rc<Cell<bool>>, show: bool) {
    if shown.get() == show {
        return;
    }
    shown.set(show);
    let generation = {
        let mut state = state.borrow_mut();
        state.generation += 1;
        state.generation
    };
    let (from, to) = (area.opacity(), if show { 1.0 } else { 0.0 });
    if show {
        area.set_visible(true);
    }
    // Reduced motion (T-44a-074): when the desktop turns animations off, the fade lasts one frame.
    if !gtk::Settings::default().is_none_or(|s| s.is_gtk_enable_animations()) {
        area.set_opacity(to);
        area.set_visible(show);
        if show {
            hide_later(area, state, shown, generation);
        }
        return;
    }
    let (area, state, shown) = (area.clone(), Rc::clone(state), Rc::clone(shown));
    let started = Instant::now();
    glib::timeout_add_local(Duration::from_millis(16), move || {
        if state.borrow().generation != generation {
            return glib::ControlFlow::Break;
        }
        let elapsed = started.elapsed().as_millis() as u64;
        area.set_opacity(fade_alpha(from, to, elapsed));
        if elapsed < FADE_MS {
            return glib::ControlFlow::Continue;
        }
        if show {
            hide_later(&area, &state, &shown, generation);
        } else {
            area.set_visible(false);
        }
        glib::ControlFlow::Break
    });
}

/// Fades the overlay out [`HOLD_MS`] from now, when fade `generation` is still the last one.
fn hide_later(area: &gtk::DrawingArea, state: &Rc<RefCell<State>>, shown: &Rc<Cell<bool>>, generation: u64) {
    let (area, state, shown) = (area.clone(), Rc::clone(state), Rc::clone(shown));
    glib::timeout_add_local_once(Duration::from_millis(HOLD_MS), move || {
        if state.borrow().generation == generation {
            fade(&area, &state, &shown, false);
        }
    });
}

/// Paints the zones: a translucent fill and the label at the zone's centre, white with a black outline.
fn draw(zones: &[Zone], cr: &gtk::cairo::Context, width: f64, height: f64) {
    for zone in zones {
        let (left, top) = (zone.rect.left * width, zone.rect.top * height);
        let (w, h) = ((zone.rect.right - zone.rect.left) * width, (zone.rect.bottom - zone.rect.top) * height);
        let argb = nav::colors::of(zone.action);
        let channel = |shift: u32| f64::from((argb >> shift) & 0xFF) / 255.0;
        cr.set_source_rgba(channel(16), channel(8), channel(0), channel(24));
        cr.rectangle(left, top, w, h);
        let _ = cr.fill();

        let text = nav::label(zone.action).1;
        cr.set_font_size(label_size(w, text.chars().count()));
        let Ok(extents) = cr.text_extents(text) else { continue };
        cr.move_to(left + w / 2.0 - extents.width() / 2.0 - extents.x_bearing(), top + h / 2.0 - extents.height() / 2.0 - extents.y_bearing());
        cr.text_path(text);
        cr.set_source_rgb(0.0, 0.0, 0.0);
        cr.set_line_width(8.0);
        let _ = cr.stroke_preserve();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        let _ = cr.fill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zones_follow_the_layout() {
        let plain = zones(NavMode::LShaped, TappingInvertMode::NONE, ViewerKind::Webtoon);
        assert_eq!(plain.len(), 4);
        assert_eq!(plain[0], Zone { rect: Rect::new(0.0, 0.33, 0.33, 0.66), action: Action::Prev });
    }

    #[test]
    fn the_settings_pick_the_pager_or_the_webtoon_navigation() {
        let settings = ViewerSettings::from_prefs(&kuro_prefs::reader::ReaderPrefs::new(kuro_prefs::store::PrefStore::in_memory()));
        assert_eq!(zones_for(ViewerKind::RightToLeft, &settings).len(), 2);
        assert_eq!(zones_for(ViewerKind::Webtoon, &settings).len(), 4);
    }

    #[test]
    fn the_default_navigation_depends_on_the_viewer() {
        assert!(zones(NavMode::Default, TappingInvertMode::NONE, ViewerKind::LeftToRight).iter().all(|z| matches!(z.action, Action::Left | Action::Right)));
        assert!(zones(NavMode::Default, TappingInvertMode::NONE, ViewerKind::Webtoon).iter().all(|z| matches!(z.action, Action::Prev | Action::Next)));
    }

    #[test]
    fn the_first_navigation_shows_only_when_asked_and_a_later_one_always_does() {
        assert!(!should_fade_in(true, false, false, true), "first launch, not asked");
        assert!(should_fade_in(true, false, true, true), "first launch, asked");
        assert!(should_fade_in(false, false, false, true), "a change after the first");
        assert!(!should_fade_in(false, true, true, true), "already visible");
        assert!(!should_fade_in(true, false, true, false), "disabled navigation");
    }

    #[test]
    fn the_fade_is_linear_over_one_second() {
        assert_eq!(fade_alpha(0.0, 1.0, 0), 0.0);
        assert!((fade_alpha(0.0, 1.0, 500) - 0.5).abs() < 1e-9);
        assert_eq!(fade_alpha(0.0, 1.0, 5_000), 1.0);
        assert!((fade_alpha(0.5, 0.0, 500) - 0.25).abs() < 1e-9, "a fade out starts from where the fade in got to");
    }

    #[test]
    fn a_narrow_zone_shrinks_its_label_and_a_wide_one_keeps_64() {
        assert_eq!(label_size(2000.0, 4), LABEL_SIZE);
        assert!(label_size(100.0, 4) < LABEL_SIZE);
        assert!(label_size(1.0, 4) >= 8.0);
    }
}
