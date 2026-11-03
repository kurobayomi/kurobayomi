//! The Cloudflare solver state machine, without WebKit. The GTK side feeds it the page and
//! cookie events and runs the 30 s deadline; this type decides when the solve is over. It has no GTK types,
//! so every row of the table is a unit test.

use super::SolveFailure;

/// One input to the machine. `clearance` is the current `cf_clearance` value for the original URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SolveEvent {
    /// `load-changed(COMMITTED)`: the document for the original URL (fragment ignored) and its HTTP status.
    Committed { is_original: bool, status: u16 },
    /// `load-changed(FINISHED)`.
    Finished { is_original: bool, clearance: Option<String> },
    /// A cookie-store change or the 500 ms poll.
    CookieCheck { clearance: Option<String> },
    /// `load-failed` for the main frame. `is_original` is true when it is the original URL.
    LoadFailed { is_original: bool, message: String },
    /// The 30 s deadline ran out before anything finished.
    Deadline,
    /// The caller cancelled the solve.
    Cancelled,
}

/// How the solve ended. The driver adds the cookies (for `Cleared`) and the `outdated` flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MachineStep {
    Cleared,
    NoChallenge,
    Failed(SolveFailure),
}

/// Tracks one solve. Once a step is returned, every later event returns `None`.
#[derive(Debug)]
pub struct SolveMachine {
    old_clearance: Option<String>,
    challenge_seen: bool,
    done: bool,
}

impl SolveMachine {
    /// `old_clearance` is the `cf_clearance` value that was removed before the solve (the "differs" test).
    pub fn new(old_clearance: Option<String>) -> Self {
        Self {
            old_clearance,
            challenge_seen: false,
            done: false,
        }
    }

    /// A value is cleared when it exists and differs from the old one (Kotlin `it != oldCookie`).
    fn cleared(&self, clearance: Option<&String>) -> bool {
        clearance.is_some_and(|value| self.old_clearance.as_ref() != Some(value))
    }

    /// Applies one event and returns the step when the solve ends on it.
    pub fn on_event(&mut self, event: SolveEvent) -> Option<MachineStep> {
        if self.done {
            return None;
        }
        let step = match event {
            SolveEvent::Committed { is_original, status } => {
                if is_original && matches!(status, 403 | 503) {
                    self.challenge_seen = true;
                }
                None
            }
            SolveEvent::Finished { is_original, clearance } => {
                if self.cleared(clearance.as_ref()) {
                    Some(MachineStep::Cleared)
                } else if is_original && !self.challenge_seen {
                    Some(MachineStep::NoChallenge)
                } else {
                    None
                }
            }
            SolveEvent::CookieCheck { clearance } => {
                self.cleared(clearance.as_ref()).then_some(MachineStep::Cleared)
            }
            SolveEvent::LoadFailed { is_original, message } => {
                (is_original && !self.challenge_seen)
                    .then_some(MachineStep::Failed(SolveFailure::LoadError(message)))
            }
            SolveEvent::Deadline => Some(MachineStep::Failed(SolveFailure::Timeout)),
            SolveEvent::Cancelled => Some(MachineStep::Failed(SolveFailure::Cancelled)),
        };
        if step.is_some() {
            self.done = true;
        }
        step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cf(value: &str) -> Option<String> {
        Some(value.to_owned())
    }

    #[test]
    fn a_new_clearance_value_clears_on_finish() {
        let mut m = SolveMachine::new(cf("old"));
        let step = m.on_event(SolveEvent::Finished {
            is_original: true,
            clearance: cf("new"),
        });
        assert_eq!(step, Some(MachineStep::Cleared));
    }

    #[test]
    fn the_same_value_as_before_is_not_cleared() {
        let mut m = SolveMachine::new(cf("same"));
        assert_eq!(
            m.on_event(SolveEvent::CookieCheck { clearance: cf("same") }),
            None
        );
        assert_eq!(
            m.on_event(SolveEvent::Finished {
                is_original: false,
                clearance: cf("same"),
            }),
            None
        );
    }

    #[test]
    fn a_first_value_when_none_existed_clears() {
        let mut m = SolveMachine::new(None);
        assert_eq!(
            m.on_event(SolveEvent::CookieCheck { clearance: cf("fresh") }),
            Some(MachineStep::Cleared)
        );
    }

    #[test]
    fn finish_on_the_original_url_without_a_challenge_is_no_challenge() {
        let mut m = SolveMachine::new(None);
        m.on_event(SolveEvent::Committed { is_original: true, status: 200 });
        assert_eq!(
            m.on_event(SolveEvent::Finished {
                is_original: true,
                clearance: None,
            }),
            Some(MachineStep::NoChallenge)
        );
    }

    #[test]
    fn a_challenge_status_keeps_the_solve_running_until_clearance_or_deadline() {
        let mut m = SolveMachine::new(None);
        m.on_event(SolveEvent::Committed { is_original: true, status: 503 });
        assert_eq!(
            m.on_event(SolveEvent::Finished {
                is_original: true,
                clearance: None,
            }),
            None
        );
        assert_eq!(m.on_event(SolveEvent::Deadline), Some(MachineStep::Failed(SolveFailure::Timeout)));
    }

    #[test]
    fn a_403_on_another_url_does_not_count_as_the_challenge() {
        let mut m = SolveMachine::new(None);
        m.on_event(SolveEvent::Committed { is_original: false, status: 403 });
        assert_eq!(
            m.on_event(SolveEvent::Finished {
                is_original: true,
                clearance: None,
            }),
            Some(MachineStep::NoChallenge)
        );
    }

    #[test]
    fn a_main_frame_load_error_fails_only_before_a_challenge_is_seen() {
        let mut m = SolveMachine::new(None);
        assert_eq!(
            m.on_event(SolveEvent::LoadFailed {
                is_original: true,
                message: "dns".into(),
            }),
            Some(MachineStep::Failed(SolveFailure::LoadError("dns".into())))
        );

        let mut seen = SolveMachine::new(None);
        seen.on_event(SolveEvent::Committed { is_original: true, status: 403 });
        assert_eq!(
            seen.on_event(SolveEvent::LoadFailed {
                is_original: true,
                message: "reset".into(),
            }),
            None
        );
    }

    #[test]
    fn cancel_finishes_with_cancelled_and_later_events_are_ignored() {
        let mut m = SolveMachine::new(None);
        assert_eq!(
            m.on_event(SolveEvent::Cancelled),
            Some(MachineStep::Failed(SolveFailure::Cancelled))
        );
        assert_eq!(m.on_event(SolveEvent::Deadline), None);
        assert_eq!(
            m.on_event(SolveEvent::CookieCheck { clearance: cf("x") }),
            None
        );
    }
}
