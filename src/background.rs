//! Work off the GTK thread with its result back on it: the one way the app does it (no screen hand-rolls a thread, a
//! channel and a local future of its own).
//!
//! - [`run`]: blocking work (a blocking network call, a child process to wait for) on a detached thread of its own, so
//!   it never holds a pool or delays the exit. [`blocking`] is short blocking work on GLib's worker pool for code on the
//!   GTK main context that awaits the value and goes on.
//! - [`run_async`] and [`run_async_cancellable`]: a `Send` future on the tokio runtime that `main` enters while the
//!   app runs (source calls, network). The GTK thread never awaits it. [`run_async_on`] runs on a given runtime and
//!   tells `done` when the task died instead of leaving it waiting.
//! - [`run_local_async`]: a future that is not `Send` (the image service's), driven to its end on a blocking-pool thread.
//!
//! `done` always runs on the GTK main context, and only when the work finished: no widget is touched from a worker, as
//! `done` is not `Send`. A long-lived worker (a writer thread, a proxy, the library worker) is a thread of its own, not a
//! job here.

use std::future::Future;

use async_channel::bounded;
use gtk::glib;
use tokio_util::sync::CancellationToken;

/// Runs `call` on the runtime and passes its value to `deliver` on the GTK main context.
pub fn run_async<T, Call, Deliver>(call: Call, deliver: Deliver)
where
    T: Send + 'static,
    Call: Future<Output = T> + Send + 'static,
    Deliver: FnOnce(T) + 'static,
{
    run_async_cancellable(CancellationToken::new(), call, deliver);
}

/// As [`run_async`], but the call is dropped when `cancel` fires, and then `deliver` never runs.
pub fn run_async_cancellable<T, Call, Deliver>(cancel: CancellationToken, call: Call, deliver: Deliver)
where
    T: Send + 'static,
    Call: Future<Output = T> + Send + 'static,
    Deliver: FnOnce(T) + 'static,
{
    let (sender, receiver) = bounded::<T>(1);
    tokio::spawn(async move {
        let value = tokio::select! {
            biased;
            () = cancel.cancelled() => return,
            value = call => value,
        };
        let _ = sender.send(value).await;
    });
    deliver_on_main(receiver, deliver);
}

/// Runs `call` on the runtime `on` (a source's own runtime) and passes its value to `done` on the GTK main context, or
/// `None` when the task ended without one (it panicked), so a spinner or a busy notice is never left waiting.
pub fn run_async_on<T, Call, Deliver>(on: &tokio::runtime::Handle, call: Call, done: Deliver)
where
    T: Send + 'static,
    Call: Future<Output = T> + Send + 'static,
    Deliver: FnOnce(Option<T>) + 'static,
{
    let (sender, receiver) = bounded::<T>(1);
    on.spawn(async move {
        let _ = sender.send(call.await).await;
    });
    glib::MainContext::ref_thread_default().spawn_local(async move {
        done(receiver.recv().await.ok());
    });
}

/// Runs blocking `work` on a thread of its own and passes its value to `done` on the GTK main context. The thread is
/// detached: a long wait (an external player that runs for an hour) holds no pool and never delays the app's exit.
pub fn run<T, Work, Done>(work: Work, done: Done)
where
    T: Send + 'static,
    Work: FnOnce() -> T + Send + 'static,
    Done: FnOnce(T) + 'static,
{
    let (sender, receiver) = bounded::<T>(1);
    std::thread::spawn(move || {
        let _ = sender.send_blocking(work());
    });
    deliver_on_main(receiver, done);
}

/// Blocking `work` on GLib's worker pool, for code on the GTK main context that goes on after it (`blocking(..).await`).
/// For short work (a file, a database read); a long wait goes through [`run`]. `Err` when the work panicked.
pub async fn blocking<T, Work>(work: Work) -> Result<T, Box<dyn std::any::Any + Send + 'static>>
where
    T: Send + 'static,
    Work: FnOnce() -> T + Send + 'static,
{
    gtk::gio::spawn_blocking(work).await
}

/// Runs the future `work` makes, which need not be `Send`, to its end on a blocking-pool thread of the runtime, and
/// passes its value to `done` on the GTK main context. Without an entered runtime nothing runs.
pub fn run_local_async<T, Fut, Work, Done>(work: Work, done: Done)
where
    T: Send + 'static,
    Fut: Future<Output = T>,
    Work: FnOnce() -> Fut + Send + 'static,
    Done: FnOnce(T) + 'static,
{
    let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
    let (sender, receiver) = bounded::<T>(1);
    let driver = runtime.clone();
    runtime.spawn_blocking(move || {
        let _ = sender.send_blocking(driver.block_on(work()));
    });
    deliver_on_main(receiver, done);
}

/// Waits on the GTK main context for the one value of `receiver` and passes it to `done`; a dropped sender passes
/// nothing.
fn deliver_on_main<T: 'static>(receiver: async_channel::Receiver<T>, done: impl FnOnce(T) + 'static) {
    // the thread-default context: the GTK main context in the app, and a private one in the tests
    glib::MainContext::ref_thread_default().spawn_local(async move {
        if let Ok(value) = receiver.recv().await {
            done(value);
        }
    });
}

#[cfg(test)]
/// The request of the browse listing on screen. A new listing cancels the one before it (T-15a-050).
// No call site until the source screens land (T-01a-037 wiring).
#[derive(Default)]
pub struct ListingRequest {
    current: Option<CancellationToken>,
}

#[cfg(test)]
impl ListingRequest {
    #[cfg(test)]
    /// The token for a new listing request. The previous request, if any, is cancelled.
    pub fn next(&mut self) -> CancellationToken {
        if let Some(previous) = self.current.take() {
            previous.cancel();
        }
        let token = CancellationToken::new();
        self.current = Some(token.clone());
        token
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn a_new_listing_cancels_the_one_before_it() {
        let mut listing = ListingRequest::default();
        let first = listing.next();
        assert!(!first.is_cancelled());
        let second = listing.next();
        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());
    }

    /// A source that sleeps two seconds must not hold up the main context: a 10 ms timer on it fires in under 50 ms.
    /// The value then arrives on the main context. The test uses its own context, so tests do not share one.
    #[test]
    fn a_slow_source_call_leaves_the_main_context_responsive() {
        let rt = kuro_core::runtime::build().expect("runtime");
        let _entered = rt.enter();
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let (done_tx, done_rx) = mpsc::channel::<u32>();
                run_async(
                    async {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        42_u32
                    },
                    move |value| {
                        let _ = done_tx.send(value);
                    },
                );
                context.block_on(async {
                    let before = Instant::now();
                    glib::timeout_future(Duration::from_millis(10)).await;
                    assert!(before.elapsed() < Duration::from_millis(50), "the main context stalled");
                });
                assert!(done_rx.try_recv().is_err(), "the value must not arrive before the call finishes");
                context.block_on(glib::timeout_future(Duration::from_millis(2_300)));
                assert_eq!(done_rx.try_recv(), Ok(42));
            })
            .expect("the test thread takes its own main context");
    }

    #[test]
    fn a_cancelled_call_never_delivers() {
        let rt = kuro_core::runtime::build().expect("runtime");
        let _entered = rt.enter();
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| {
                let (done_tx, done_rx) = mpsc::channel::<u32>();
                let token = CancellationToken::new();
                run_async_cancellable(
                    token.clone(),
                    async {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        7_u32
                    },
                    move |value| {
                        let _ = done_tx.send(value);
                    },
                );
                token.cancel();
                context.block_on(glib::timeout_future(Duration::from_millis(100)));
                assert!(done_rx.try_recv().is_err());
            })
            .expect("the test thread takes its own main context");
    }
    /// The guard of this module: no code outside it hands off background work its own way. A one-shot hand-off (a
    /// spawned thread or task whose single value is awaited on the GTK side) and a direct GLib worker-pool call are
    /// this module's job; a stream of progress (a `while` loop over a channel) and a long-lived worker are not. Reads the
    /// source files, so it needs no display.
    #[test]
    fn background_work_goes_through_this_module() {
        fn rust_files(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("a source directory") {
                let path = entry.expect("a directory entry").path();
                if path.is_dir() {
                    rust_files(&path, found);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    found.push(path);
                }
            }
        }
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        let mut offenders = Vec::new();
        for file in files.into_iter().filter(|f| !f.ends_with("background.rs")) {
            let text = std::fs::read_to_string(&file).expect("a readable source file");
            // the production part: a test module may wire its own threads
            let code = text.split("#[cfg(test)]\nmod tests").next().unwrap_or_default();
            let spawns = ["thread::spawn", "thread::Builder::new", "spawn_blocking(", ".spawn(async move"].iter().any(|s| code.contains(s));
            let one_shot = code.lines().any(|line| line.contains(".recv().await") && !line.contains("while "));
            if code.contains("gio::spawn_blocking") || (spawns && one_shot) {
                offenders.push(file.strip_prefix(&src).unwrap_or(&file).display().to_string());
            }
        }
        assert!(offenders.is_empty(), "hand background work to crate::background (run, run_async, run_async_on, blocking): {offenders:?}");
    }
}
