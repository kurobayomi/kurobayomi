//! The toast service (area 12 section 3.4, T-12a-014). `Toaster::show` is callable from any thread. The messages
//! go into a bounded channel; the GTK overlay in the root crate takes them one at a time with [`ToastFeed::next`]
//! and shows each for its [`ToastLength::duration`]. Five messages may wait; a sixth is dropped while five wait.

use std::time::Duration;

/// How many messages may wait at once.
pub const MAX_QUEUED: usize = 5;

/// `Toast.LENGTH_SHORT` and `Toast.LENGTH_LONG` on Android.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastLength {
    Short,
    Long,
}

impl ToastLength {
    pub fn duration(self) -> Duration {
        match self {
            ToastLength::Short => Duration::from_millis(2000),
            ToastLength::Long => Duration::from_millis(3500),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToastMessage {
    pub text: String,
    pub length: ToastLength,
}

/// The sending side. Cheap to clone; held by `AppContext` and by the network stack.
#[derive(Clone, Debug)]
pub struct Toaster {
    tx: async_channel::Sender<ToastMessage>,
}

/// The receiving side, owned by the overlay.
#[derive(Debug)]
pub struct ToastFeed {
    rx: async_channel::Receiver<ToastMessage>,
}

/// A connected pair: the sender for producers and the feed for the overlay.
pub fn toast_channel() -> (Toaster, ToastFeed) {
    let (tx, rx) = async_channel::bounded(MAX_QUEUED);
    (Toaster { tx }, ToastFeed { rx })
}

impl Toaster {
    /// Queues a message. Empty text is ignored. Returns `false` when the message was not queued (empty, or
    /// five messages already wait). Never blocks.
    pub fn show(&self, text: impl Into<String>, length: ToastLength) -> bool {
        let text = text.into();
        if text.is_empty() {
            return false;
        }
        match self.tx.try_send(ToastMessage { text, length }) {
            Ok(()) => true,
            Err(async_channel::TrySendError::Full(dropped)) => {
                tracing::debug!(text = %dropped.text, "toast dropped: queue is full");
                false
            }
            Err(async_channel::TrySendError::Closed(_)) => false,
        }
    }
}

impl ToastFeed {
    /// Waits for the next message. `None` once every `Toaster` is gone.
    pub async fn next(&self) -> Option<ToastMessage> {
        self.rx.recv().await.ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_lasts_two_seconds_and_long_three_and_a_half() {
        assert_eq!(ToastLength::Short.duration(), Duration::from_millis(2000));
        assert_eq!(ToastLength::Long.duration(), Duration::from_millis(3500));
    }

    #[test]
    fn empty_text_is_ignored() {
        let (toaster, _feed) = toast_channel();
        assert!(!toaster.show("", ToastLength::Short));
    }

    #[test]
    fn a_sixth_message_is_dropped_while_five_wait() {
        let (toaster, feed) = toast_channel();
        for i in 0..MAX_QUEUED {
            assert!(toaster.show(format!("m{i}"), ToastLength::Short), "message {i}");
        }
        assert!(!toaster.show("m5", ToastLength::Long));
        drop(feed);
    }

    #[tokio::test(start_paused = true)]
    async fn messages_come_out_in_order_and_a_slot_frees_when_one_is_taken() {
        let (toaster, feed) = toast_channel();
        toaster.show("first", ToastLength::Short);
        toaster.show("second", ToastLength::Long);
        assert_eq!(feed.next().await.unwrap().text, "first");
        assert!(toaster.show("third", ToastLength::Short));
        let second = feed.next().await.unwrap();
        assert_eq!(second.text, "second");
        assert_eq!(second.length, ToastLength::Long);
    }

    #[tokio::test]
    async fn next_ends_when_every_sender_is_gone() {
        let (toaster, feed) = toast_channel();
        drop(toaster);
        assert_eq!(feed.next().await, None);
    }
}
