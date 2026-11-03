//! Bangumi errors. The display texts are for
//! logs; the user sees the message keys they convert to (T-31a-041).

use kuro_platform::secrets::SecretError;

use crate::error::TrackError;
use crate::messages::{MsgKey, TrackDetail};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BangumiError {
    #[error("Not authenticated with Bangumi")]
    NotAuthenticated,
    /// The refresh was refused (400 or 401), or the stored login has no refresh token (D1, EC-B-02).
    #[error("Bangumi session has expired, log in again")]
    AuthExpired,
    #[error("HTTP error {0}")]
    Http(u16),
    #[error("{0}")]
    Transport(String),
    #[error("Failed to decode Bangumi response: {0}")]
    Decode(String),
    #[error("Unknown status: {0}")]
    UnknownStatus(String),
    #[error("Could not find {0}")]
    NotFound(&'static str),
    /// No client id and secret in the user's configuration (D-OPEN-1).
    #[error("Bangumi is not configured")]
    NotConfigured,
    #[error("Secret storage is unavailable")]
    SecretStorage,
}

impl From<reqwest::Error> for BangumiError {
    fn from(e: reqwest::Error) -> Self {
        match e.status() {
            Some(code) => BangumiError::Http(code.as_u16()),
            None => BangumiError::Transport(e.to_string()),
        }
    }
}

impl From<serde_json::Error> for BangumiError {
    fn from(e: serde_json::Error) -> Self {
        BangumiError::Decode(e.to_string())
    }
}

impl From<SecretError> for BangumiError {
    fn from(_: SecretError) -> Self {
        BangumiError::SecretStorage
    }
}

impl From<BangumiError> for TrackError {
    /// The toast of a failure (T-31a-041). The expired login (D1), the unset client and a server status have their
    /// own keys. Every other failure is `track_error` with a fixed detail key, never raw text.
    fn from(e: BangumiError) -> Self {
        let detail = match e {
            BangumiError::AuthExpired | BangumiError::Http(401) => {
                return TrackError::Key(MsgKey::LoginExpired);
            }
            BangumiError::NotConfigured => {
                return TrackError::Key(MsgKey::TrackerNotConfigured(super::NAME.to_owned()));
            }
            BangumiError::Http(code) => return TrackError::Key(MsgKey::HttpStatus(code)),
            BangumiError::NotAuthenticated => TrackDetail::NotAuthenticated,
            BangumiError::NotFound("anime") => TrackDetail::NotFoundAnime,
            BangumiError::NotFound(_) => TrackDetail::NotFoundManga,
            BangumiError::UnknownStatus(_) => TrackDetail::UnknownStatus,
            BangumiError::Decode(_) => TrackDetail::InvalidResponse,
            BangumiError::Transport(_) => TrackDetail::Network,
            BangumiError::SecretStorage => TrackDetail::SecretStorage,
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
    fn t_31a_034_user_visible_texts() {
        assert_eq!(
            BangumiError::NotAuthenticated.to_string(),
            "Not authenticated with Bangumi"
        );
        assert_eq!(BangumiError::Http(403).to_string(), "HTTP error 403");
        assert_eq!(
            BangumiError::NotFound("manga").to_string(),
            "Could not find manga"
        );
    }

    #[test]
    fn t_31a_041_each_failure_maps_to_a_key_and_never_to_raw_text() {
        let key = |e: BangumiError| match TrackError::from(e) {
            TrackError::Key(k) => k,
            other => panic!("{other:?}"),
        };
        assert_eq!(key(BangumiError::AuthExpired), MsgKey::LoginExpired);
        assert_eq!(key(BangumiError::Http(401)), MsgKey::LoginExpired);
        assert_eq!(
            key(BangumiError::NotConfigured),
            MsgKey::TrackerNotConfigured("Bangumi".into())
        );
        assert_eq!(key(BangumiError::Http(404)), MsgKey::HttpStatus(404));
        assert_eq!(
            key(BangumiError::NotFound("manga")),
            MsgKey::TrackerDetail {
                tracker: "Bangumi".into(),
                detail: TrackDetail::NotFoundManga
            }
        );
        let decode = key(BangumiError::Decode("secret body text".into()));
        assert!(!crate::messages::english(&decode).contains("secret body text"));
        let all = [
            BangumiError::NotAuthenticated,
            BangumiError::Transport("x".into()),
            BangumiError::UnknownStatus("9".into()),
            BangumiError::SecretStorage,
        ];
        for e in all {
            assert!(matches!(key(e), MsgKey::TrackerDetail { .. }));
        }
    }
}
