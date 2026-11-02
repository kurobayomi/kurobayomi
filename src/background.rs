//! Source calls from the GTK thread (T-15a-050). A source call runs on the tokio runtime that `main` enters while the
//! app runs. Its value comes back on the GTK main context, where widgets may be touched. The GTK thread never awaits
//! a source, and no widget is touched from a tokio task: `deliver` is not `Send`, so it cannot be moved there.

use std::future::Future;

use async_channel::bounded;
use gtk::glib;
use tokio_util::sync::CancellationToken;

/// Runs `call` on the runtime and passes its value to `deliver` on the GTK main context.
// No call site until the source screens land (T-01a-037 wiring).
pub fn call_on_main<T, Call, Deliver>(call: Call, deliver: Deliver)
where
    T: Send + 'static,
    Call: Future<Output = T> + Send + 'static,
    Deliver: FnOnce(T) + 'static,
{
    call_cancellable(CancellationToken::new(), call, deliver);
}

/// As [`call_on_main`], but the call is dropped when `cancel` fires, and then `deliver` never runs.
// No call site until the source screens land (T-01a-037 wiring).
pub fn call_cancellable<T, Call, Deliver>(cancel: CancellationToken, call: Call, deliver: Deliver)
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
    // The thread-default context: the GTK main context in the app, and a private one in the tests.
    glib::MainContext::ref_thread_default().spawn_local(async move {
        if let Ok(value) = receiver.recv().await {
            deliver(value);
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
                call_on_main(
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
                call_cancellable(
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
}
