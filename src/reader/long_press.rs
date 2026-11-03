//! Long-press tracker (T-43b-012). Pure: no GTK types. The page view feeds it presses, motion,
//! releases and cancels with millisecond timestamps, starts a timer on `Arm`, and calls [`LongPressTracker::fire`]
//! when the timer runs out.
//!
//! Rules (Kotlin `GestureDetectorWithLongTap`): a press arms the long press unless it follows the previous release
//! within the double-tap window (the second tap of a double tap, E-08b). Movement beyond the slop, a release, a
//! cancel or a second pointer disarms it (E-09, E-10, E-11).

/// Movement beyond this many pixels on either axis cancels a pending long press.
pub const SLOP_PX: f64 = 8.0;
/// `gtk-long-press-time` default.
pub const DEFAULT_LONG_PRESS_MS: u32 = 500;
/// `gtk-double-click-time` default.
pub const DEFAULT_DOUBLE_CLICK_MS: u32 = 400;

/// What the page view should do with its timer after an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LongPressAction {
    /// Start a timer that calls `fire` after `after_ms`.
    Arm { after_ms: u32 },
    /// Cancel any pending timer.
    Disarm,
    /// Nothing to do.
    None,
}

#[derive(Clone, Debug)]
pub struct LongPressTracker {
    pub long_press_ms: u32,
    pub double_tap_ms: u32,
    pub slop_px: f64,
    last_up_ms: Option<u32>,
    down: (f64, f64),
    armed: bool,
}

impl LongPressTracker {
    pub fn new(long_press_ms: u32, double_tap_ms: u32) -> Self {
        Self { long_press_ms, double_tap_ms, slop_px: SLOP_PX, last_up_ms: None, down: (0.0, 0.0), armed: false }
    }

    /// A press at `t_ms`. Arms unless it lands inside the double-tap window of the previous release.
    pub fn on_press(&mut self, t_ms: u32, x: f64, y: f64) -> LongPressAction {
        let second_tap = self.last_up_ms.is_some_and(|up| t_ms.saturating_sub(up) <= self.double_tap_ms);
        if second_tap {
            self.armed = false;
            return LongPressAction::None;
        }
        self.down = (x, y);
        self.armed = true;
        LongPressAction::Arm { after_ms: self.long_press_ms }
    }

    /// A pointer move. Disarms when a pending press has moved more than the slop on either axis.
    pub fn on_motion(&mut self, x: f64, y: f64) -> LongPressAction {
        let moved = (x - self.down.0).abs() > self.slop_px || (y - self.down.1).abs() > self.slop_px;
        if self.armed && moved {
            self.armed = false;
            return LongPressAction::Disarm;
        }
        LongPressAction::None
    }

    /// The release at `t_ms`. Records the release time for the double-tap window and disarms.
    pub fn on_release(&mut self, t_ms: u32) -> LongPressAction {
        self.last_up_ms = Some(t_ms);
        self.armed = false;
        LongPressAction::Disarm
    }

    /// A cancelled gesture, or a second pointer going down. Disarms.
    pub fn on_cancel(&mut self) -> LongPressAction {
        self.armed = false;
        LongPressAction::Disarm
    }

    /// Called when the timer runs out. `true` means a long press is confirmed. A timer that outlived its press (the
    /// press was released, moved or cancelled) returns `false`.
    pub fn fire(&mut self) -> bool {
        let confirmed = self.armed;
        self.armed = false;
        confirmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> LongPressTracker {
        LongPressTracker::new(500, 300)
    }

    #[test]
    fn e08_a_press_arms_the_long_press_after_the_configured_delay() {
        let mut t = tracker();
        assert_eq!(t.on_press(1000, 10.0, 10.0), LongPressAction::Arm { after_ms: 500 });
    }

    #[test]
    fn e08b_a_press_inside_the_double_tap_window_does_not_arm() {
        let mut t = tracker();
        t.on_press(900, 10.0, 10.0);
        t.on_release(1000);
        // 200 ms after the release, inside the 300 ms window.
        assert_eq!(t.on_press(1200, 10.0, 10.0), LongPressAction::None);
        assert!(!t.fire());
    }

    #[test]
    fn e08c_a_press_just_outside_the_window_arms() {
        let mut t = tracker();
        t.on_release(1000);
        assert_eq!(t.on_press(1301, 0.0, 0.0), LongPressAction::Arm { after_ms: 500 });
    }

    #[test]
    fn e09_movement_beyond_the_slop_disarms_and_inside_it_keeps_the_press() {
        let mut t = tracker();
        t.on_press(1000, 100.0, 100.0);
        assert_eq!(t.on_motion(107.0, 100.0), LongPressAction::None);
        assert_eq!(t.on_motion(109.0, 100.0), LongPressAction::Disarm);
        assert!(!t.fire());
    }

    #[test]
    fn e09b_vertical_movement_past_the_slop_also_disarms() {
        let mut t = tracker();
        t.on_press(1000, 0.0, 0.0);
        assert_eq!(t.on_motion(0.0, -9.0), LongPressAction::Disarm);
    }

    #[test]
    fn e10_a_release_disarms_and_a_stale_timer_is_ignored() {
        let mut t = tracker();
        t.on_press(1000, 0.0, 0.0);
        assert_eq!(t.on_release(1100), LongPressAction::Disarm);
        assert!(!t.fire());
    }

    #[test]
    fn a_fired_timer_confirms_once() {
        let mut t = tracker();
        t.on_press(1000, 0.0, 0.0);
        assert!(t.fire());
        assert!(!t.fire());
    }

    #[test]
    fn e11_a_cancel_or_second_pointer_disarms() {
        let mut t = tracker();
        t.on_press(1000, 0.0, 0.0);
        assert_eq!(t.on_cancel(), LongPressAction::Disarm);
        assert!(!t.fire());
    }

    #[test]
    fn a_motion_without_a_pending_press_does_nothing() {
        let mut t = tracker();
        assert_eq!(t.on_motion(500.0, 500.0), LongPressAction::None);
    }

    #[test]
    fn the_default_constants_are_the_gtk_defaults() {
        assert_eq!((DEFAULT_LONG_PRESS_MS, DEFAULT_DOUBLE_CLICK_MS), (500, 400));
        assert_eq!(LongPressTracker::new(DEFAULT_LONG_PRESS_MS, DEFAULT_DOUBLE_CLICK_MS).slop_px, 8.0);
    }
}
