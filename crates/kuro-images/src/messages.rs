//! Result messages of the save, share and cover actions. The UI shows each one as a
//! snackbar or a desktop notice with the text returned here.

use kuro_i18n::{MsgKey, tr};

/// One message key of the save, share and cover actions, plus the notice texts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    CoverSaved,
    ErrorSavingCover,
    ErrorSharingCover,
    CoverUpdated,
    FailedToUpdateCover,
    BackgroundSaved,
    ErrorSavingBackground,
    ErrorSharingBackground,
    BackgroundUpdated,
    FailedToUpdateBackground,
    /// Title of the save notice on success.
    PictureSaved,
    /// Title of the save notice on failure.
    Error,
    /// Body of the save notice when the error has no text.
    UnknownError,
}

impl Message {
    /// The text of the message in the active language, from the `kuro-i18n` catalogue. `Error` has no catalogue
    /// key of its own and stays English.
    pub fn text(self) -> String {
        let key = match self {
            Message::CoverSaved => MsgKey::cover_saved,
            Message::ErrorSavingCover => MsgKey::error_saving_cover,
            Message::ErrorSharingCover => MsgKey::error_sharing_cover,
            Message::CoverUpdated => MsgKey::cover_updated,
            Message::FailedToUpdateCover => MsgKey::notification_cover_update_failed,
            Message::BackgroundSaved => MsgKey::background_saved,
            Message::ErrorSavingBackground => MsgKey::error_saving_background,
            Message::ErrorSharingBackground => MsgKey::error_sharing_background,
            Message::BackgroundUpdated => MsgKey::background_updated,
            Message::FailedToUpdateBackground => MsgKey::notification_background_update_failed,
            Message::PictureSaved => MsgKey::picture_saved,
            Message::UnknownError => MsgKey::unknown_error,
            Message::Error => return "Error".to_owned(),
        };
        tr(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snackbar_texts_match_behavior_23_1() {
        let rows = [
            (Message::CoverSaved, "Cover saved"),
            (Message::ErrorSavingCover, "Error saving cover"),
            (Message::ErrorSharingCover, "Error sharing cover"),
            (Message::CoverUpdated, "Cover updated"),
            (Message::FailedToUpdateCover, "Failed to update cover"),
            (Message::BackgroundSaved, "Background saved"),
            (Message::ErrorSavingBackground, "Error saving background"),
            (Message::ErrorSharingBackground, "Error sharing background"),
            (Message::BackgroundUpdated, "Background updated"),
            (
                Message::FailedToUpdateBackground,
                "Failed to update background",
            ),
        ];
        for (message, text) in rows {
            assert_eq!(message.text(), text);
        }
    }

    #[test]
    fn the_notice_texts_match_behavior_23_2() {
        assert_eq!(Message::PictureSaved.text(), "Picture saved");
        assert_eq!(Message::Error.text(), "Error");
        assert_eq!(Message::UnknownError.text(), "Unknown error");
    }
}
