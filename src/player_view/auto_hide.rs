//! The player's bars hide by themselves (user, 2026-10-14; `auto_hide_bars`, on by default): while a video plays, the
//! top bar, the control bar and the cursor fade out after [`IDLE`] without the mouse, a click or a key, and come back on
//! any of them. They stay while paused or while one of their menus is open. Both bars float over the video, so
//! hiding them never resizes it. With the setting off the header sits above the video and nothing hides.

use std::cell::Cell;
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
        let this = Rc::new(Self { enabled, header: header_revealer, bar: bar_revealer, area: area.clone(), last_input: Rc::new(Cell::new(Instant::now())) });
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

    /// Shows or hides the bars and the cursor for this moment. `paused` is the player's state.
    pub fn tick(&self, paused: bool) {
        let menu_open = [self.header.upcast_ref::<gtk::Widget>(), self.bar.upcast_ref()].into_iter().any(has_open_menu);
        let show = shown(self.enabled, paused, menu_open, self.last_input.get().elapsed());
        if self.header.reveals_child() != show {
            self.header.set_reveal_child(show);
            self.bar.set_reveal_child(show);
            self.area.set_cursor_from_name(if show { None } else { Some("none") });
        }
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
}
