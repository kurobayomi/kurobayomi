//! External player result handling (Kotlin `MainActivity.externalPlayerResult`, T-36a-059).
//!
//! Android started the player for a result and ran the progress callback on `RESULT_OK`. Here the player is a child process:
//! a success exit runs the progress callback with the saved anime and episode ids; any other exit does nothing.

use std::process::Child;

/// The episode handed to the external player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExternalPlayback {
    pub anime_id: i64,
    pub episode_id: i64,
}

/// Runs `on_progress` with the saved ids when the player exited with success. A failed exit, or ids that were not saved,
/// run nothing.
pub fn handle_exit(success: bool, saved: Option<ExternalPlayback>, on_progress: impl FnOnce(ExternalPlayback)) -> bool {
    match saved.filter(|_| success) {
        Some(playback) => {
            on_progress(playback);
            true
        }
        None => false,
    }
}

/// Waits for `child` on a worker thread and runs [`handle_exit`] on the GTK thread when it ends.
pub fn watch(mut child: Child, saved: ExternalPlayback, on_progress: impl FnOnce(ExternalPlayback) + 'static) {
    crate::background::run(
        move || child.wait().is_ok_and(|status| status.success()),
        move |success| {
            handle_exit(success, Some(saved), on_progress);
        },
    );
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    const SAVED: ExternalPlayback = ExternalPlayback { anime_id: 4, episode_id: 9 };

    #[test]
    fn a_success_exit_runs_the_callback_with_the_saved_ids() {
        let seen = RefCell::new(None);
        assert!(handle_exit(true, Some(SAVED), |p| *seen.borrow_mut() = Some(p)));
        assert_eq!(*seen.borrow(), Some(SAVED));
    }

    #[test]
    fn a_failed_exit_does_nothing() {
        let seen = RefCell::new(None);
        assert!(!handle_exit(false, Some(SAVED), |p| *seen.borrow_mut() = Some(p)));
        assert!(seen.borrow().is_none());
    }

    #[test]
    fn a_success_without_saved_ids_does_nothing() {
        let seen = RefCell::new(None);
        assert!(!handle_exit(true, None, |p| *seen.borrow_mut() = Some(p)));
        assert!(seen.borrow().is_none());
    }

    #[test]
    fn a_real_child_exit_is_reported_through_the_main_loop() {
        crate::testing::with_display(|| {
            let ctx = gtk::glib::MainContext::default();
            for (program, expect) in [("true", true), ("false", false)] {
                let Ok(child) = std::process::Command::new(program).spawn() else { return };
                let ran = std::rc::Rc::new(std::cell::Cell::new(false));
                let flag = ran.clone();
                watch(child, SAVED, move |_| flag.set(true));
                // a success ends the wait early; a failure is given the same time to (not) call back
                let end = std::time::Instant::now() + std::time::Duration::from_millis(1500);
                while std::time::Instant::now() < end && !ran.get() {
                    ctx.iteration(false);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                assert_eq!(ran.get(), expect, "{program}");
            }
        });
    }
}
