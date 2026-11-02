//! Window-size rules shared by the layouts (T-34a-062, `WindowSize.kt` and `DisplayExtensions.kt`).

use std::cell::RefCell;

use gtk::prelude::*;

use crate::entry_core::constants::two_pane;

/// The smaller side, in logical pixels, from which the tablet layouts apply (`TABLET_UI_REQUIRED_SCREEN_WIDTH_DP`).
pub const TABLET_UI_MIN: i32 = 720;

/// The size a new window opens at: 16:9, the shape of an episode, so the player fills it with no bars (user,
/// 2026-11-02), and exactly [`TABLET_UI_MIN`] tall so the wide layouts apply.
pub const DEFAULT_SIZE: (i32, i32) = (1280, 720);

/// True when `min(width, height)` is at least 720 logical pixels. Android uses the smallest screen width, so a
/// window that is wide but short stays on the phone layouts.
pub fn is_tablet_ui(width: i32, height: i32) -> bool {
    width.min(height) >= TABLET_UI_MIN
}

/// The window's current size: the real one once it is shown, else the default one.
pub fn size_of(window: &impl IsA<gtk::Window>) -> (i32, i32) {
    let window = window.upcast_ref::<gtk::Window>();
    (window.width().max(window.default_width()), window.height().max(window.default_height()))
}

/// Remembers the last answers of both breakpoints (tablet UI and two panes) so a resize reports only when one flips.
#[derive(Default)]
pub struct Crossing(Option<(bool, bool)>);

impl Crossing {
    /// `Some(tablet)` when the window crossed the tablet or the two-pane threshold since the last call. The reported answer
    /// is the tablet one, so a listener that needs the two-pane answer works it out from the width. The first call only
    /// sets the baseline.
    pub fn update(&mut self, width: i32, height: i32) -> Option<bool> {
        let now = (is_tablet_ui(width, height), two_pane(width));
        let before = self.0.replace(now);
        before.filter(|b| *b != now).map(|_| now.0)
    }
}

/// A layout that depends on [`is_tablet_ui`]. It gets the new answer and returns `false` once it is gone.
type Listener = Box<dyn Fn(bool) -> bool>;

thread_local! {
    static CROSSING: RefCell<Crossing> = RefCell::new(Crossing::default());
    static LISTENERS: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
}

/// Runs `f` with the new answer every time a resize crosses the 720 px or the 900 px threshold, until it returns `false`.
pub fn on_tablet_change(f: impl Fn(bool) -> bool + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push(Box::new(f)));
}

fn check(window: &gtk::Window) {
    let (width, height) = size_of(window);
    let Some(tablet) = CROSSING.with(|c| c.borrow_mut().update(width, height)) else { return };
    // a listener may rebuild a page that registers another listener, so run a taken copy and merge back
    let mut run = LISTENERS.with(|l| std::mem::take(&mut *l.borrow_mut()));
    run.retain(|f| f(tablet));
    LISTENERS.with(|l| {
        let mut added = l.borrow_mut();
        run.append(&mut added);
        *added = run;
    });
}

/// Starts watching `window`: a change of its size or maximized state re-evaluates the breakpoint. Call once.
pub fn watch(window: &impl IsA<gtk::Window>) {
    let window = window.upcast_ref::<gtk::Window>();
    check(window);
    window.connect_default_width_notify(check);
    window.connect_default_height_notify(check);
    window.connect_maximized_notify(check);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_window_has_the_shape_of_an_episode_and_the_wide_layouts() {
        let (w, h) = DEFAULT_SIZE;
        assert_eq!(w * 9, h * 16, "16:9");
        assert!(is_tablet_ui(w, h));
    }

    #[test]
    fn the_smaller_side_decides_with_719_off_and_720_on() {
        assert!(!is_tablet_ui(719, 719));
        assert!(is_tablet_ui(720, 720));
        assert!(!is_tablet_ui(1800, 719), "wide but short");
        assert!(!is_tablet_ui(719, 1800), "tall but narrow");
        assert!(is_tablet_ui(1800, 720));
        assert!(is_tablet_ui(720, 1800));
    }

    #[test]
    fn crossing_fires_on_the_two_pane_breakpoint_as_well() {
        let mut c = Crossing::default();
        assert_eq!(c.update(1000, 800), None, "the first call is the baseline");
        assert_eq!(c.update(899, 800), Some(true), "still a tablet, but the two-pane width is crossed");
        assert_eq!(c.update(900, 800), Some(true), "back over 900 px");
        assert_eq!(c.update(900, 800), None, "no change, no report");
    }

    #[test]
    fn crossing_reports_only_a_flip() {
        let mut c = Crossing::default();
        assert_eq!(c.update(1180, 800), None, "the first call is the baseline");
        assert_eq!(c.update(1000, 800), None, "still a tablet");
        assert_eq!(c.update(1000, 719), Some(false));
        assert_eq!(c.update(900, 600), None, "still a phone");
        assert_eq!(c.update(720, 720), Some(true));
    }

    #[test]
    fn resizing_across_the_threshold_flips_the_answer() {
        let sizes = [(1180, 800), (1180, 719), (1180, 720), (719, 800), (720, 800), (1180, 800)];
        let answers: Vec<bool> = sizes.iter().map(|&(w, h)| is_tablet_ui(w, h)).collect();
        assert_eq!(answers, [true, false, true, false, true, true]);
    }
}
