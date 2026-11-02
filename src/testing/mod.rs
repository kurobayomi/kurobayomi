//! The one GTK harness of the test binary. GTK binds to the thread that starts it and takes the default GLib main
//! context with it, while `cargo test` runs every test on a thread of its own. So every test that creates a widget or
//! touches the default main context runs its body through [`with_display`]: the first call starts one dedicated thread,
//! starts GTK there, and from then on all such bodies run on that thread, one at a time.
//!
//! Without a display (a headless CI container) GTK cannot start, and [`with_display`] skips the body with a note on
//! stderr, so the test passes. Pure-logic tests do not use this module and keep running in parallel.

use std::any::Any;
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};

type Job = Box<dyn FnOnce() + Send>;

struct Harness {
    /// Send side of the GTK thread's queue; the mutex serialises the callers and makes the handle `Sync`.
    jobs: Mutex<Sender<Job>>,
    display: bool,
}

fn harness() -> &'static Harness {
    static HARNESS: OnceLock<Harness> = OnceLock::new();
    HARNESS.get_or_init(|| {
        let (jobs, queue): (Sender<Job>, Receiver<Job>) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("gtk-test-thread".into())
            .spawn(move || {
                // ponytail: GTK tests run on Unix only; on the Windows runner some wait forever for a window event and,
                // holding the one GTK thread, stall the rest. Port them in their own step, then drop the `cfg!`.
                let display = cfg!(unix) && adw::init().is_ok();
                ready_tx.send(display).expect("the first caller waits for the answer");
                if display {
                    for job in queue {
                        job();
                    }
                }
            })
            .expect("spawn the GTK test thread");
        Harness { jobs: Mutex::new(jobs), display: ready_rx.recv().expect("the GTK test thread answers") }
    })
}

/// Whether GTK started, which means a display is available.
pub fn has_display() -> bool {
    harness().display
}

/// Runs `body` on the GTK thread and returns its result, or `None` when there is no display. A panic in the body
/// reaches the calling test, so the assertion messages stay with the test that made them.
pub fn on_gtk<R: Send + 'static>(body: impl FnOnce() -> R + Send + 'static) -> Option<R> {
    let harness = harness();
    if !harness.display {
        return None;
    }
    let (result_tx, result_rx) = mpsc::sync_channel::<Result<R, Box<dyn Any + Send>>>(1);
    let job: Job = Box::new(move || {
        let _ = result_tx.send(panic::catch_unwind(AssertUnwindSafe(body)));
    });
    // The lock is held until the body is done, so display tests never overlap.
    let jobs = harness.jobs.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    jobs.send(job).expect("the GTK test thread is running");
    match result_rx.recv().expect("the GTK test thread returns the result") {
        Ok(value) => Some(value),
        Err(payload) => {
            drop(jobs);
            // Panic again here, so the message is printed (and captured) on the test's own thread.
            let message = match payload.downcast::<String>() {
                Ok(message) => *message,
                Err(payload) => payload.downcast::<&str>().map_or_else(|_| "a non-text panic".to_owned(), |m| (*m).to_owned()),
            };
            panic!("{message}")
        }
    }
}

/// The body of a test that needs a display: runs on the GTK thread, or says why it was skipped.
pub fn with_display(body: impl FnOnce() + Send + 'static) {
    if on_gtk(body).is_none() {
        eprintln!("skipped: no display for this GTK test");
    }
}

/// The navigation view and window of the whole test binary, emptied on every call. `ui::nav_init` keeps the first view
/// it is given for the life of its thread, and every display test now shares the GTK thread, so they all take this one
/// instead of making their own. The first page a test pushes is its root. Call it only from inside [`with_display`] or
/// [`on_gtk`].
pub fn nav() -> (adw::NavigationView, gtk::ApplicationWindow) {
    use gtk::prelude::*;

    thread_local! {
        static NAV: (adw::Application, gtk::ApplicationWindow, adw::NavigationView) = {
            let app = adw::Application::builder()
                .application_id("io.github.anas1412.KurobayomiTest")
                .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
                .build();
            app.register(gtk::gio::Cancellable::NONE).expect("register the test application");
            // an `adw::ApplicationWindow`, as the app's: dialogs open inside it
            let adw_window = adw::ApplicationWindow::new(&app);
            let nav = adw::NavigationView::new();
            adw::prelude::AdwApplicationWindowExt::set_content(&adw_window, Some(&nav));
            let window: gtk::ApplicationWindow = adw_window.upcast();
            crate::ui::nav_init(&nav, &window);
            (app, window, nav)
        };
    }
    NAV.with(|(_, window, nav)| {
        nav.replace(&[]);
        (nav.clone(), window.clone())
    })
}

#[cfg(test)]
mod tests {
    use std::thread::{self, ThreadId};


    use super::*;

    fn gtk_thread_id() -> Option<ThreadId> {
        on_gtk(|| thread::current().id())
    }

    #[test]
    fn display_tests_all_run_on_one_thread_that_is_not_the_test_thread() {
        let Some(first) = gtk_thread_id() else { return };
        assert_ne!(first, thread::current().id());
        let handles: Vec<_> = (0..4).map(|_| thread::spawn(gtk_thread_id)).collect();
        for handle in handles {
            assert_eq!(handle.join().expect("the caller finishes"), Some(first));
        }
    }

    #[test]
    fn bodies_never_overlap() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::Duration;

        if !has_display() {
            return;
        }
        let inside = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let inside = Arc::clone(&inside);
                thread::spawn(move || {
                    on_gtk(move || {
                        assert_eq!(inside.fetch_add(1, Ordering::SeqCst), 0, "another body is still running");
                        thread::sleep(Duration::from_millis(5));
                        inside.fetch_sub(1, Ordering::SeqCst);
                    })
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("the body passed");
        }
    }

    #[test]
    fn a_widget_can_be_made_on_the_gtk_thread() {
        with_display(|| {
            let label = gtk::Label::new(Some("hello"));
            assert_eq!(label.text().as_str(), "hello");
            assert!(gtk::is_initialized_main_thread());
        });
    }

    #[test]
    fn a_panic_in_a_body_reaches_the_caller_and_the_thread_survives() {
        if !has_display() {
            return;
        }
        let caught = panic::catch_unwind(|| with_display(|| panic!("boom")));
        assert!(caught.is_err(), "the panic reaches the caller");
        assert_eq!(on_gtk(|| 7), Some(7), "the thread still runs bodies");
    }
}

/// An absolute path on this system from a Unix-style one: `/x` stays `/x`, and is `C:/x` on Windows, where a path
/// without a drive is not absolute.
pub fn abs(unix: &str) -> String {
    if cfg!(windows) { format!("C:{unix}") } else { unix.to_owned() }
}

/// The `file://` link of [`abs`]`(unix)`, with `unix` kept as written (escapes included): `file:///x`, or
/// `file:///C:/x` on Windows.
pub fn file_link(unix: &str) -> String {
    if cfg!(windows) { format!("file:///C:{unix}") } else { format!("file://{unix}") }
}
