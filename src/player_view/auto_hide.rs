//! The player's bars hide by themselves (user, 2026-10-14; `auto_hide_bars`, on by default): while a video plays, the
//! top bar, the control bar and the cursor fade out after [`IDLE`] without the mouse, a click or a key, and come back on
//! any of them. They stay while paused or while one of their menus is open. Both bars float over the video, so
//! hiding them never resizes it. With the setting off the header sits above the video and nothing hides.
//!
//! The side panels over the video (the episode list, the comments) sit between the bars, not under them
//! ([`AutoHide::hold_for`]): their top starts under a floating header and their foot above the control bar, and the
//! bars stay while one is open, as they do for an open menu (user, 2026-11-21: their title rows and their foot were
//! hidden under the bars).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;

/// How long the bars stay without any input while a video plays.
pub const IDLE: Duration = Duration::from_secs(3);

/// Whether the bars show: always without auto-hide, while paused or a menu is open, and within [`IDLE`] of the last input.
pub fn shown(enabled: bool, paused: bool, menu_open: bool, idle: Duration) -> bool {
    !enabled || paused || menu_open || idle < IDLE
}

pub struct AutoHide {
    enabled: bool,
    header: gtk::Revealer,
    bar: gtk::Revealer,
    area: gtk::GLArea,
    last_input: Rc<Cell<Instant>>,
    /// The side panels over the video, kept between the bars ([`Self::hold_for`]).
    panels: RefCell<Vec<gtk::Revealer>>,
}

/// The room a side panel leaves at its top and its foot: the header's height when it floats over the video (auto-hide
/// on), nothing when it sits above it; the control bar's height, which always floats.
pub fn panel_insets(enabled: bool, header_height: i32, bar_height: i32) -> (i32, i32) {
    (if enabled { header_height.max(0) } else { 0 }, bar_height.max(0))
}

impl AutoHide {
    /// Lays out `header` and `bar` on `page` and `overlay` (which holds `area`, the video) and follows the input.
    pub fn attach(enabled: bool, page: &gtk::Box, overlay: &gtk::Overlay, header: &adw::HeaderBar, bar: &gtk::Box, area: &gtk::GLArea) -> Rc<Self> {
        let reveal = |child: &gtk::Widget, transition, align| {
            let revealer = gtk::Revealer::builder().transition_type(transition).transition_duration(200).reveal_child(true).valign(align).build();
            revealer.set_child(Some(child));
            revealer
        };
        let header_revealer = reveal(header.upcast_ref(), gtk::RevealerTransitionType::Crossfade, gtk::Align::Start);
        let bar_revealer = reveal(bar.upcast_ref(), gtk::RevealerTransitionType::Crossfade, gtk::Align::End);
        if enabled {
            overlay.add_overlay(&header_revealer);
        } else {
            page.append(&header_revealer);
        }
        overlay.add_overlay(&bar_revealer);
        let this = Rc::new(Self {
            enabled,
            header: header_revealer,
            bar: bar_revealer,
            area: area.clone(),
            last_input: Rc::new(Cell::new(Instant::now())),
            panels: RefCell::new(Vec::new()),
        });
        if enabled {
            // a pointer that really moved, a click or a key counts as input (GTK also reports motion when the layout
            // changes under a still pointer, which would keep the bars up for ever)
            let motion = gtk::EventControllerMotion::new();
            let (last, at) = (Rc::clone(&this.last_input), Cell::new((f64::NAN, f64::NAN)));
            motion.connect_motion(move |_, x, y| {
                if at.replace((x, y)) != (x, y) {
                    last.set(Instant::now());
                }
            });
            page.add_controller(motion);
            let click = gtk::GestureClick::new();
            click.set_propagation_phase(gtk::PropagationPhase::Capture);
            let last = Rc::clone(&this.last_input);
            click.connect_pressed(move |_, _, _, _| last.set(Instant::now()));
            page.add_controller(click);
            let keys = gtk::EventControllerKey::new();
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            let last = Rc::clone(&this.last_input);
            keys.connect_key_pressed(move |_, _, _, _| {
                last.set(Instant::now());
                gtk::glib::Propagation::Proceed
            });
            page.add_controller(keys);
        }
        this
    }

    /// Keeps `panel` (a side panel laid over the video, aligned to its edge) between the bars, and the bars up while it
    /// is open.
    pub fn hold_for(&self, panel: &gtk::Revealer) {
        self.panels.borrow_mut().push(panel.clone());
        self.fit_panels();
    }

    /// Gives every held panel the room of the bars, measured while they show (a hidden bar is not measured).
    fn fit_panels(&self) {
        if !self.header.reveals_child() {
            return;
        }
        let (top, bottom) = panel_insets(self.enabled, self.header.height(), self.bar.height());
        for panel in self.panels.borrow().iter() {
            if panel.margin_top() != top {
                panel.set_margin_top(top);
            }
            if panel.margin_bottom() != bottom {
                panel.set_margin_bottom(bottom);
            }
        }
    }

    /// Shows or hides the bars and the cursor for this moment. `paused` is the player's state. An open side panel keeps
    /// them, as an open menu does.
    pub fn tick(&self, paused: bool) {
        let menu_open = [self.header.upcast_ref::<gtk::Widget>(), self.bar.upcast_ref()].into_iter().any(has_open_menu);
        let panel_open = self.panels.borrow().iter().any(|panel| panel.reveals_child());
        let show = shown(self.enabled, paused, menu_open || panel_open, self.last_input.get().elapsed());
        if self.header.reveals_child() != show {
            self.header.set_reveal_child(show);
            self.bar.set_reveal_child(show);
            self.area.set_cursor_from_name(if show { None } else { Some("none") });
        }
        self.fit_panels();
    }
}

/// Whether a menu of `root` is open (a pressed menu button).
fn has_open_menu(root: &gtk::Widget) -> bool {
    if root.downcast_ref::<gtk::MenuButton>().is_some_and(|m| m.is_active()) {
        return true;
    }
    let mut child = root.first_child();
    while let Some(c) = child {
        if has_open_menu(&c) {
            return true;
        }
        child = c.next_sibling();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bars_hide_only_while_playing_untouched_and_with_no_menu_open() {
        let quiet = IDLE + Duration::from_millis(1);
        assert!(!shown(true, false, false, quiet), "playing, untouched for 3 s: hidden");
        assert!(shown(true, false, false, Duration::from_secs(1)), "a recent move shows them");
        assert!(shown(true, true, false, quiet), "paused: shown");
        assert!(shown(true, false, true, quiet), "a menu open: shown");
        assert!(shown(false, false, false, quiet), "auto-hide off: always shown");
    }

    #[test]
    fn a_panel_starts_under_a_floating_header_and_ends_above_the_control_bar() {
        assert_eq!(panel_insets(true, 46, 52), (46, 52), "auto-hide: both bars float over the video");
        assert_eq!(panel_insets(false, 46, 52), (0, 52), "the header sits above the video: no room needed at the top");
        assert_eq!(panel_insets(true, -1, 0), (0, 0), "an unmeasured bar takes no room");
    }
}
