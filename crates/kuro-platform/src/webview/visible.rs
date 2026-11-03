//! The rules of the visible challenge window (T-12a-032), without GTK. The hidden solve is
//! tried first; when it times out and the host can show a window, the same challenge runs again in a 960 x 720 window
//! titled `cloudflare_check_title` with the hint `cloudflare_check_hint` and a Cancel button. The window closes by itself
//! when the check clears. Cancel, Escape and the window's close button all cancel the solve. A person needs longer than a
//! script, so the deadline is 120 s instead of 30 s. The window itself is GTK and lives in the app.

use std::time::Duration;

use super::{SolveFailure, SolveOutcome, SolveRequest};

pub const WINDOW_WIDTH: i32 = 960;
pub const WINDOW_HEIGHT: i32 = 720;
/// The deadline of a solve in the visible window.
pub const VISIBLE_SOLVE_TIMEOUT: Duration = Duration::from_secs(120);

/// The ways a person ends the visible window without clearing the check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dismissal {
    CancelButton,
    Escape,
    CloseButton,
}

impl Dismissal {
    /// Every dismissal is the same cancel (`Failed(Cancelled)`).
    pub fn outcome(self) -> SolveOutcome {
        SolveOutcome::Failed { reason: SolveFailure::Cancelled, outdated: false }
    }
}

impl SolveRequest {
    /// The same challenge for the visible window: the longer deadline and the `visible` flag.
    pub fn into_visible(self) -> Self {
        Self { visible: true, timeout: VISIBLE_SOLVE_TIMEOUT, ..self }
    }
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::*;

    fn request() -> SolveRequest {
        SolveRequest {
            url: Url::parse("https://site.test/").unwrap(),
            headers: Vec::new(),
            user_agent: "UA".into(),
            cookies: Vec::new(),
            old_clearance: Some("old".into()),
            timeout: Duration::from_secs(30),
            cookie_names: &["cf_clearance"],
            visible: false,
        }
    }

    #[test]
    fn the_window_is_960_by_720_and_the_deadline_120_seconds() {
        assert_eq!((WINDOW_WIDTH, WINDOW_HEIGHT), (960, 720));
        assert_eq!(VISIBLE_SOLVE_TIMEOUT, Duration::from_secs(120));
    }

    #[test]
    fn a_visible_request_keeps_the_challenge_and_gets_the_long_deadline() {
        let visible = request().into_visible();
        assert!(visible.visible);
        assert_eq!(visible.timeout, Duration::from_secs(120));
        assert_eq!((visible.url.as_str(), visible.user_agent.as_str(), visible.old_clearance.as_deref()), ("https://site.test/", "UA", Some("old")));
    }

    #[test]
    fn cancel_escape_and_the_close_button_are_all_a_cancel() {
        for dismissal in [Dismissal::CancelButton, Dismissal::Escape, Dismissal::CloseButton] {
            assert_eq!(dismissal.outcome(), SolveOutcome::Failed { reason: SolveFailure::Cancelled, outdated: false });
        }
    }
}
