//! Single tap, double tap and long press recognition (T-44a-011, B§3.1, E-20, E-22, E-23).
//!
//! Pure over timestamps in milliseconds, so the GTK layer feeds it press, motion and release events plus a timer
//! poll. A single tap is confirmed only once the double-click window has passed without a second press (Android
//! parity, and E-20: a double tap produces no tap action). A long press starting inside the window of the previous
//! release is ignored (E-22). The release after a long press is not a tap.

/// What the recogniser reports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    Tap { x: f64, y: f64 },
    DoubleTap { x: f64, y: f64 },
    LongPress { x: f64, y: f64 },
}

#[derive(Clone, Copy, Debug)]
struct Press {
    /// Where the timer-armed press started. Refreshed only when a long-press timer starts (E-23).
    x: f64,
    y: f64,
    /// When the long press fires, if one is armed.
    long_at: Option<u64>,
    /// This press is the second half of a double tap.
    double: bool,
    /// A long press fired during this press, so its release is not a tap.
    long_fired: bool,
    /// The pointer left the slop, so the release is a drag, not a tap.
    moved: bool,
}

#[derive(Debug)]
pub struct TapRecognizer {
    /// `gtk-double-click-time`.
    double_click_ms: u64,
    /// `gtk-long-press-time`.
    long_press_ms: u64,
    /// `gtk-dnd-drag-threshold`, the movement that cancels a long press.
    slop: f64,
    last_up: Option<u64>,
    press: Option<Press>,
    /// A release waiting out the double-click window: x, y and the release time.
    pending_tap: Option<(f64, f64, u64)>,
    /// The pager's mode: a single tap acts on release and a double tap acts on nothing (E-20). The webtoon waits
    /// out the window so that a double tap can zoom instead.
    on_release: bool,
}

impl TapRecognizer {
    pub fn new(double_click_ms: u64, long_press_ms: u64, slop: f64) -> Self {
        Self { double_click_ms, long_press_ms, slop, last_up: None, press: None, pending_tap: None, on_release: false }
    }

    /// The pager's mode (T-44a-011): a single tap is reported on release, and the second tap of a double tap is
    /// swallowed rather than reported as a [`Gesture::DoubleTap`].
    pub fn acting_on_release(mut self) -> Self {
        self.on_release = true;
        self
    }

    /// A press at `t` (ms). Timers due by `t` are settled first.
    pub fn press(&mut self, t: u64, x: f64, y: f64) -> Vec<Gesture> {
        let out = self.poll(t);
        let within = self.last_up.is_some_and(|up| t.saturating_sub(up) <= self.double_click_ms);
        if within {
            self.pending_tap = None;
            // A double-tap press keeps the old coordinates and arms no long press (E-22, E-23).
            let (px, py) = self.press.map_or((x, y), |p| (p.x, p.y));
            self.press = Some(Press { x: px, y: py, long_at: None, double: true, long_fired: false, moved: false });
        } else {
            self.press = Some(Press { x, y, long_at: Some(t + self.long_press_ms), double: false, long_fired: false, moved: false });
        }
        out
    }

    /// Whether the press in progress is the second half of a double tap. A drag from it is a quick scale (T-44a-040).
    pub fn in_double_press(&self) -> bool {
        self.press.is_some_and(|p| p.double)
    }

    /// Pointer motion. Movement past the slop cancels an armed long press.
    pub fn motion(&mut self, x: f64, y: f64) {
        if let Some(press) = self.press.as_mut() {
            if (x - press.x).abs() > self.slop || (y - press.y).abs() > self.slop {
                press.moved = true;
                press.long_at = None;
            }
        }
    }

    /// A release at `t`. Returns a double tap, or leaves a single tap pending.
    pub fn release(&mut self, t: u64, x: f64, y: f64) -> Vec<Gesture> {
        let mut out = self.poll(t);
        let Some(press) = self.press.take() else { return out };
        if press.long_fired || press.moved {
            // a drag or a long press is no first tap: the next press starts afresh. Counted as one, a second swipe
            // soon after the first was the second half of a double tap, and quick-scaled (or zoomed) the strip.
            self.last_up = None;
            return out;
        }
        self.last_up = Some(t);
        if press.double {
            if !self.on_release {
                out.push(Gesture::DoubleTap { x: press.x, y: press.y });
            }
        } else if self.on_release {
            out.push(Gesture::Tap { x, y });
        } else {
            self.pending_tap = Some((x, y, t));
        }
        out
    }

    /// Settles timers at `t`: a long press that has come due, or a single tap whose window has passed.
    pub fn poll(&mut self, t: u64) -> Vec<Gesture> {
        let mut out = Vec::new();
        if let Some(press) = self.press.as_mut() {
            if press.long_at.is_some_and(|at| t >= at) {
                press.long_at = None;
                press.long_fired = true;
                out.push(Gesture::LongPress { x: press.x, y: press.y });
            }
        }
        if let Some((x, y, up)) = self.pending_tap {
            if t >= up + self.double_click_ms {
                self.pending_tap = None;
                out.push(Gesture::Tap { x, y });
            }
        }
        out
    }
}

/// Release velocity below which a short swipe snaps back (TP section 5), in pixels per second.
pub const FLING_VELOCITY: f64 = 300.0;
/// A fling shorter than this is a stray touch, not a page turn (the ViewPager minimum fling distance).
const FLING_MIN_DISTANCE: f64 = 16.0;
/// Samples older than this before the release do not count towards the release velocity.
const VELOCITY_WINDOW_MS: u64 = 100;

/// Timed offsets of a drag, for the release velocity (T-44a-023). Pure over timestamps in milliseconds.
#[derive(Debug, Default)]
pub struct DragTracker {
    samples: Vec<(u64, f64)>,
}

impl DragTracker {
    pub fn begin(&mut self) {
        self.samples.clear();
    }

    /// The drag offset at time `t`.
    pub fn push(&mut self, t: u64, offset: f64) {
        self.samples.push((t, offset));
    }

    /// The velocity in px/s over the last [`VELOCITY_WINDOW_MS`] before the last sample, signed like the offset.
    /// A drag that paused before its release has no velocity.
    pub fn velocity(&self) -> f64 {
        let Some(&(end, last)) = self.samples.last() else { return 0.0 };
        let Some(&(start, first)) = self.samples.iter().find(|(t, _)| end - t <= VELOCITY_WINDOW_MS) else { return 0.0 };
        if end == start { 0.0 } else { (last - first) / (end - start) as f64 * 1000.0 }
    }
}

/// How many items a swipe moves: `+1` towards the next, `-1` towards the previous, `0` to snap back. It commits
/// past half the `extent`, or when the release `velocity` in the drag's direction passes [`FLING_VELOCITY`] after a
/// drag longer than [`FLING_MIN_DISTANCE`] (TP section 5).
pub fn swipe_step(offset: f64, velocity: f64, extent: f64) -> i64 {
    if extent <= 0.0 {
        return 0;
    }
    let towards_next = offset < 0.0;
    if offset.abs() > extent / 2.0 {
        return if towards_next { 1 } else { -1 };
    }
    let flung = offset.abs() > FLING_MIN_DISTANCE && velocity.abs() >= FLING_VELOCITY && (velocity < 0.0) == towards_next;
    match (flung, towards_next) {
        (false, _) => 0,
        (true, true) => 1,
        (true, false) => -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pager_tap_acts_on_release() {
        let mut r = recognizer().acting_on_release();
        assert!(r.press(0, 1.0, 2.0).is_empty());
        assert_eq!(r.release(50, 1.0, 2.0), vec![Gesture::Tap { x: 1.0, y: 2.0 }]);
        assert!(r.poll(10_000).is_empty(), "nothing is left pending");
    }

    #[test]
    fn a_pager_double_tap_reports_no_double_tap_and_no_second_tap() {
        // E-20: the first tap already acted on release; the second press inside the window acts on nothing.
        let mut r = recognizer().acting_on_release();
        r.press(0, 5.0, 5.0);
        assert_eq!(r.release(50, 5.0, 5.0).len(), 1);
        assert!(r.press(200, 5.0, 5.0).is_empty());
        assert!(r.release(230, 5.0, 5.0).is_empty());
        assert!(r.poll(10_000).is_empty());
    }

    #[test]
    fn a_pager_drag_or_long_press_is_not_a_tap() {
        let mut dragged = recognizer().acting_on_release();
        dragged.press(0, 0.0, 0.0);
        dragged.motion(20.0, 0.0);
        assert!(dragged.release(100, 20.0, 0.0).is_empty());
        let mut long = recognizer().acting_on_release();
        long.press(0, 0.0, 0.0);
        assert_eq!(long.poll(500), vec![Gesture::LongPress { x: 0.0, y: 0.0 }]);
        assert!(long.release(600, 0.0, 0.0).is_empty());
    }

    #[test]
    fn a_swipe_commits_past_half_the_extent() {
        assert_eq!(swipe_step(-401.0, 0.0, 800.0), 1);
        assert_eq!(swipe_step(401.0, 0.0, 800.0), -1);
        assert_eq!(swipe_step(-400.0, 0.0, 800.0), 0);
    }

    #[test]
    fn a_fast_short_flick_commits_in_its_direction() {
        assert_eq!(swipe_step(-100.0, -300.0, 800.0), 1);
        assert_eq!(swipe_step(100.0, 900.0, 800.0), -1);
        assert_eq!(swipe_step(-100.0, -299.0, 800.0), 0, "below the threshold it snaps back");
    }

    #[test]
    fn a_flick_against_the_drag_or_a_stray_touch_snaps_back() {
        assert_eq!(swipe_step(-100.0, 900.0, 800.0), 0, "the finger came back");
        assert_eq!(swipe_step(-10.0, -2000.0, 800.0), 0, "too short to be a page turn");
        assert_eq!(swipe_step(-100.0, -900.0, 0.0), 0);
    }

    #[test]
    fn the_release_velocity_is_measured_over_the_last_100_ms() {
        let mut d = DragTracker::default();
        d.push(0, 0.0);
        d.push(50, -5.0);
        d.push(150, -15.0);
        d.push(200, -45.0);
        // From t=100 on: samples at 150 and 200 -> -30 px in 50 ms.
        assert_eq!(d.velocity(), -600.0);
    }

    #[test]
    fn a_paused_drag_has_no_release_velocity() {
        let mut d = DragTracker::default();
        d.push(0, 0.0);
        d.push(50, -80.0);
        d.push(400, -80.0);
        assert_eq!(d.velocity(), 0.0);
        d.begin();
        assert_eq!(d.velocity(), 0.0);
    }

    fn recognizer() -> TapRecognizer {
        TapRecognizer::new(400, 500, 8.0)
    }

    #[test]
    fn a_single_tap_is_confirmed_after_the_double_click_window() {
        let mut r = recognizer();
        assert!(r.press(0, 1.0, 2.0).is_empty());
        assert!(r.release(50, 1.0, 2.0).is_empty());
        assert!(r.poll(449).is_empty());
        assert_eq!(r.poll(450), vec![Gesture::Tap { x: 1.0, y: 2.0 }]);
    }

    #[test]
    fn a_double_tap_produces_no_single_tap() {
        // E-20: the second press inside the window cancels the pending tap.
        let mut r = recognizer();
        r.press(0, 5.0, 5.0);
        r.release(50, 5.0, 5.0);
        assert!(r.press(200, 5.0, 5.0).is_empty());
        assert_eq!(r.release(230, 5.0, 5.0), vec![Gesture::DoubleTap { x: 5.0, y: 5.0 }]);
        assert!(r.poll(10_000).is_empty());
    }

    #[test]
    fn a_long_press_fires_once_and_its_release_is_not_a_tap() {
        let mut r = recognizer();
        r.press(0, 3.0, 4.0);
        assert!(r.poll(499).is_empty());
        assert_eq!(r.poll(500), vec![Gesture::LongPress { x: 3.0, y: 4.0 }]);
        assert!(r.poll(600).is_empty());
        assert!(r.release(700, 3.0, 4.0).is_empty());
        assert!(r.poll(2000).is_empty());
    }

    #[test]
    fn movement_past_the_slop_cancels_the_long_press() {
        let mut within_slop = recognizer();
        within_slop.press(0, 0.0, 0.0);
        within_slop.motion(8.0, 0.0);
        assert_eq!(within_slop.poll(600), vec![Gesture::LongPress { x: 0.0, y: 0.0 }]);

        let mut past_slop = recognizer();
        past_slop.press(0, 0.0, 0.0);
        past_slop.motion(9.0, 0.0);
        assert!(past_slop.poll(600).is_empty());
        past_slop.release(100, 9.0, 0.0);
        assert!(past_slop.poll(600).is_empty(), "a drag past the slop is not a tap");
    }

    #[test]
    fn a_press_soon_after_a_swipe_or_a_long_press_is_a_first_press() {
        // two quick swipes: the second is a drag of its own, not the second half of a double tap (a quick scale)
        let mut r = recognizer();
        r.press(0, 10.0, 10.0);
        r.motion(10.0, 200.0);
        assert!(r.release(150, 10.0, 200.0).is_empty());
        r.press(250, 10.0, 10.0);
        assert!(!r.in_double_press(), "no quick scale");
        assert!(r.release(280, 10.0, 10.0).is_empty(), "a tap after a swipe is no double tap");
        assert_eq!(r.poll(700), vec![Gesture::Tap { x: 10.0, y: 10.0 }], "it is a tap");
        // the pager: a tap right after a swipe acts, it is not swallowed as a second tap
        let mut pager = recognizer().acting_on_release();
        pager.press(0, 0.0, 0.0);
        pager.motion(200.0, 0.0);
        pager.release(150, 200.0, 0.0);
        pager.press(250, 5.0, 5.0);
        assert_eq!(pager.release(280, 5.0, 5.0), vec![Gesture::Tap { x: 5.0, y: 5.0 }]);
        // nor after a long press
        let mut long = recognizer();
        long.press(0, 0.0, 0.0);
        long.poll(600);
        long.release(700, 0.0, 0.0);
        long.press(800, 0.0, 0.0);
        assert!(!long.in_double_press());
    }

    #[test]
    fn a_long_press_inside_the_double_tap_window_does_not_start() {
        // E-22: the press follows a release by 100 ms, so no long press is armed.
        let mut r = recognizer();
        r.press(0, 1.0, 1.0);
        r.release(20, 1.0, 1.0);
        r.press(120, 1.0, 1.0);
        assert!(r.poll(2000).is_empty());
    }

    #[test]
    fn a_drag_after_a_double_tap_is_not_a_long_press() {
        // E-23: the double-tap press keeps stale coordinates and arms nothing.
        let mut r = recognizer();
        r.press(0, 0.0, 0.0);
        r.release(20, 0.0, 0.0);
        r.press(100, 50.0, 50.0);
        r.motion(200.0, 50.0);
        assert!(r.poll(3000).is_empty());
    }
}
