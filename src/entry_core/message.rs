//! The snackbar, toast and message-key layer of the entry pages (T-39b-019). Upstream: `AnimeScreenModel.kt` (the
//! `showSnackbar` and toast calls). A [`Notice`] is what both entry pages
//! hand to the app's one notification layer (`crate::toast`).
//!
//! Decision, queue versus replace (E-UI-3): a new message replaces the one on screen and its action is dropped
//! (Compose queues them; a documented difference), while the first draft of this file queued them. The page shows what just happened, so a new message replaces:
//! the toast layer keeps one.
//!
//! The "16 keys" of the task: the message table has 28 rows. Twelve of them are messages with text of
//! their own, and they are the variants of [`UserMessage`]. The other sixteen are labels of buttons, titles and
//! texts of dialogs that other modules own (the tracking sheet, the date remover, the share chooser, Cancel, OK,
//! Delete and Add), so there are twelve message keys, not sixteen.

use std::cell::RefCell;
use std::time::Duration;

use kuro_i18n::{MsgKey, tr};

/// Every user-visible message of the details pages that carries text of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserMessage {
    DeleteDownloadsForAnime,
    /// A text that is not a key of its own: an exception message from a refresh.
    Plain(String),
}

/// The English text of the message, with its arguments substituted.
pub fn text(message: &UserMessage) -> String {
    match message {
        UserMessage::DeleteDownloadsForAnime => tr(MsgKey::delete_downloads_for_anime),
        UserMessage::Plain(text) => text.clone(),
    }
}

/// `Short` is the snackbar default (4 s).
pub const SNACK_DURATION: Duration = Duration::from_secs(4);

/// The button a snackbar offers besides the message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnackAction {
    Delete,
    Retry,
}

impl SnackAction {
    pub fn label(self) -> String {
        match self {
            Self::Delete => tr(MsgKey::action_delete),
            Self::Retry => tr(MsgKey::action_retry),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snack {
    pub message: UserMessage,
    pub action: Option<SnackAction>,
    pub duration: Duration,
}

/// How a message is shown: a snackbar with its button rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Presentation {
    Snack(Snack),
}

/// The presentation of each message: deleting the downloads asks with a Delete button, every
/// other message is a plain snackbar.
pub fn present(message: UserMessage) -> Presentation {
    let action = matches!(message, UserMessage::DeleteDownloadsForAnime).then_some(SnackAction::Delete);
    Presentation::Snack(Snack { message, action, duration: SNACK_DURATION })
}

/// A failed refresh's snack: its text with Retry and a dismiss button.
pub fn with_retry(message: UserMessage) -> Presentation {
    Presentation::Snack(Snack { message, action: Some(SnackAction::Retry), duration: SNACK_DURATION })
}

/// What an entry page puts on the app's notification layer: the text and, for a snack with a button, the button's label
/// and the page command it runs. A notice with a button stays until it is acted on or dismissed (T-38a-034); a plain
/// one times out after `duration`. A new notice replaces the one on screen, and its action is gone (E-UI-3).
#[derive(Debug)]
pub struct Notice<A> {
    pub text: String,
    pub action: Option<(String, A)>,
    pub duration: Duration,
}

impl<A: 'static> Notice<A> {

    /// The notice of a presentation. `action` is what its button runs; a toast has no button and a snack without an
    /// action label ignores it.
    pub fn of(presentation: &Presentation, action: Option<A>) -> Self {
        let (message, label, duration) = match presentation {
            Presentation::Snack(snack) => (&snack.message, snack.action.map(SnackAction::label), snack.duration),
        };
        Self { text: text(message), action: label.zip(action), duration }
    }

    /// Shows the notice on the toast layer. `run` takes the command when its button is pressed.
    pub fn show(self, run: impl Fn(A) + 'static) {
        match self.action {
            Some((label, command)) => {
                let command = RefCell::new(Some(command));
                crate::toast::notify_action(&self.text, &label, None, move || {
                    if let Some(command) = command.borrow_mut().take() {
                        run(command);
                    }
                });
            }
            None => crate::toast::notify_for(&self.text, self.duration),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_renders_its_text() {
        assert_eq!(text(&UserMessage::DeleteDownloadsForAnime), "Delete downloaded episodes?");
        assert_eq!(text(&UserMessage::Plain("boom".into())), "boom");
    }

    #[test]
    fn only_the_delete_question_has_a_button_and_a_failed_refresh_has_retry() {
        let Presentation::Snack(delete) = present(UserMessage::DeleteDownloadsForAnime);
        assert_eq!(delete.action, Some(SnackAction::Delete));
        assert_eq!(delete.duration, SNACK_DURATION);
        let Presentation::Snack(plain) = present(UserMessage::Plain("x".into()));
        assert_eq!(plain.action, None);
        let Presentation::Snack(failed) = with_retry(UserMessage::Plain("HTTP error 503".into()));
        assert_eq!(failed.action.map(SnackAction::label), Some("Retry".to_owned()));
    }

    #[test]
    fn a_notice_has_a_button_only_when_the_presentation_has_a_label_and_a_command_comes_with_it() {
        let delete = Notice::of(&present(UserMessage::DeleteDownloadsForAnime), Some("delete"));
        assert_eq!(delete.text, "Delete downloaded episodes?");
        assert_eq!(delete.action, Some(("Delete".to_owned(), "delete")));
        let plain = Notice::of(&present(UserMessage::Plain("x".into())), Some("x"));
        assert!(plain.action.is_none(), "no label, so no action even when one is passed");
        assert_eq!(plain.duration, SNACK_DURATION, "a snack is 4 s");
        let retry = Notice::of(&with_retry(UserMessage::Plain("HTTP error 503".into())), Some("retry"));
        assert_eq!(retry.action, Some(("Retry".to_owned(), "retry")));
    }
}
