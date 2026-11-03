//! Shikimori errors. The display texts are for
//! logs; the user sees the message keys they convert to (T-31a-041).

use kuro_platform::secrets::SecretError;

use crate::error::TrackError;
use crate::messages::{MsgKey, TrackDetail};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShikimoriError {
    #[error("Not authenticated with Shikimori")]
    NotAuthenticated,
    /// The refresh was refused (400 or 401), or the stored login has no refresh token (D1, D3).
    #[error("Shikimori session has expired, log in again")]
    AuthExpired,
    #[error("HTTP error {0}")]
    Http(u16),
    #[error("{0}")]
    Transport(String),
    #[error("Failed to decode Shikimori response: {0}")]
    Decode(String),
    #[error("Unknown status: {0}")]
    UnknownStatus(String),
    /// Two library rows for one title. Manga and anime share the text (D13).
    #[error("Too many entries in response")]
    TooManyEntries,
    #[error("Could not find {0}")]
    NotFound(&'static str),
    /// A track with no library-entry id cannot be deleted remotely (D18).
    #[error("Missing library id")]
    NoLibraryId,
    /// No client id and secret in the user's configuration (D-OPEN-1).
    #[error("Shikimori is not configured")]
    NotConfigured,
    #[error("Secret storage is unavailable")]
    SecretStorage,
}

impl From<reqwest::Error> for ShikimoriError {
    fn from(e: reqwest::Error) -> Self {
        match e.status() {
            Some(code) => ShikimoriError::Http(code.as_u16()),
            None => ShikimoriError::Transport(e.to_string()),
        }
    }
}

impl From<serde_json::Error> for ShikimoriError {
    fn from(e: serde_json::Error) -> Self {
        ShikimoriError::Decode(e.to_string())
    }
}

impl From<SecretError> for ShikimoriError {
    fn from(_: SecretError) -> Self {
        ShikimoriError::SecretStorage
    }
}

impl From<ShikimoriError> for TrackError {
    /// The toast of a failure (T-31a-041). The expired login (D1), the unset client and a server status have their
    /// own keys. Every other failure is `track_error` with a fixed detail key, never raw text.
    fn from(e: ShikimoriError) -> Self {
        let detail = match e {
            ShikimoriError::AuthExpired | ShikimoriError::Http(401) => {
                return TrackError::Key(MsgKey::LoginExpired);
            }
            ShikimoriError::NotConfigured => {
                return TrackError::Key(MsgKey::TrackerNotConfigured(super::NAME.to_owned()));
            }
            ShikimoriError::Http(code) => return TrackError::Key(MsgKey::HttpStatus(code)),
            ShikimoriError::NotAuthenticated => TrackDetail::NotAuthenticated,
            ShikimoriError::NotFound("anime") => TrackDetail::NotFoundAnime,
            ShikimoriError::NotFound(_) => TrackDetail::NotFoundManga,
            ShikimoriError::UnknownStatus(_) => TrackDetail::UnknownStatus,
            ShikimoriError::Decode(_) => TrackDetail::InvalidResponse,
            ShikimoriError::Transport(_) => TrackDetail::Network,
            ShikimoriError::SecretStorage => TrackDetail::SecretStorage,
            ShikimoriError::TooManyEntries => TrackDetail::TooManyEntries,
            ShikimoriError::NoLibraryId => TrackDetail::NoLibraryId,
        };
        TrackError::Key(MsgKey::TrackerDetail {
            tracker: super::NAME.to_owned(),
            detail,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_31a_005_user_visible_texts() {
        assert_eq!(
            ShikimoriError::NotAuthenticated.to_string(),
            "Not authenticated with Shikimori"
        );
        assert_eq!(ShikimoriError::Http(404).to_string(), "HTTP error 404");
        assert_eq!(
            ShikimoriError::TooManyEntries.to_string(),
            "Too many entries in response"
        );
        assert_eq!(
            ShikimoriError::NotFound("anime").to_string(),
            "Could not find anime"
        );
    }

    #[test]
    fn t_31a_041_each_failure_maps_to_a_key_and_never_to_raw_text() {
        let key = |e: ShikimoriError| match TrackError::from(e) {
            TrackError::Key(k) => k,
            other => panic!("{other:?}"),
        };
        assert_eq!(key(ShikimoriError::AuthExpired), MsgKey::LoginExpired);
        assert_eq!(key(ShikimoriError::Http(401)), MsgKey::LoginExpired);
        assert_eq!(
            key(ShikimoriError::NotConfigured),
            MsgKey::TrackerNotConfigured("Shikimori".into())
        );
        assert_eq!(key(ShikimoriError::Http(404)), MsgKey::HttpStatus(404));
        assert_eq!(
            key(ShikimoriError::TooManyEntries),
            MsgKey::TrackerDetail {
                tracker: "Shikimori".into(),
                detail: TrackDetail::TooManyEntries
            }
        );
        assert_eq!(
            key(ShikimoriError::NotFound("anime")),
            MsgKey::TrackerDetail {
                tracker: "Shikimori".into(),
                detail: TrackDetail::NotFoundAnime
            }
        );
        // The text of the inner error never reaches the key.
        let decode = key(ShikimoriError::Decode("secret body text".into()));
        assert!(!crate::messages::english(&decode).contains("secret body text"));
        let all = [
            ShikimoriError::NotAuthenticated,
            ShikimoriError::Transport("x".into()),
            ShikimoriError::UnknownStatus("9".into()),
            ShikimoriError::NoLibraryId,
            ShikimoriError::SecretStorage,
        ];
        for e in all {
            assert!(matches!(key(e), MsgKey::TrackerDetail { .. }));
        }
    }
}
