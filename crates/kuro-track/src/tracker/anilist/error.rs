//! Typed failures of the AniList tracker.

use std::time::Duration;

use kuro_platform::secrets::SecretError;

#[derive(Debug, thiserror::Error)]
pub enum AniListError {
    #[error("Not authenticated with AniList")]
    NotAuthenticated,
    /// HTTP 401, or a stored token past its expiry. The token is cleared before this is returned.
    #[error("AniList session expired")]
    AuthExpired,
    #[error("AniList rate limit reached")]
    RateLimited { retry_after: Option<Duration> },
    #[error("HTTP error {0}")]
    Http(u16),
    #[error("AniList server error")]
    Transient,
    /// Messages of the GraphQL `errors` array, joined with `"; "`. Sent on HTTP 200 too.
    #[error("AniList error: {0}")]
    GraphQl(String),
    #[error("could not read AniList response")]
    Decode(#[source] serde_json::Error),
    #[error("no network")]
    Offline,
    #[error("entry not found on your AniList list")]
    NotInList,
    #[error("unknown AniList status {0}")]
    UnknownStatus(String),
    #[error("unknown AniList score format {0}")]
    UnknownScoreFormat(String),
    #[error("score index out of range")]
    ScoreOutOfRange,
    #[error(transparent)]
    Transport(reqwest::Error),
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error(transparent)]
    Prefs(#[from] kuro_prefs::PrefError),
    /// A failure of the shared credential state (`BaseTracker`).
    #[error(transparent)]
    Track(#[from] crate::error::TrackError),
}

impl From<AniListError> for crate::error::TrackError {
    fn from(e: AniListError) -> Self {
        match e {
            AniListError::Track(t) => t,
            AniListError::Prefs(p) => crate::error::TrackError::Prefs(p),
            AniListError::Secret(s) => crate::error::TrackError::from(s),
            other => crate::error::TrackError::Key(other.message()),
        }
    }
}

impl AniListError {
    /// The user-facing message of this failure (T-28a-045). Never the `Display` or `Debug` text: those are for logs.
    /// The GraphQL text is the server's own message and is shown as sent.
    pub fn message(&self) -> crate::messages::MsgKey {
        use crate::messages::MsgKey;
        match self {
            AniListError::NotAuthenticated => MsgKey::AniListNotAuthenticated,
            AniListError::AuthExpired => MsgKey::AniListSessionExpired,
            AniListError::NotInList => MsgKey::AniListNotInList,
            AniListError::RateLimited { .. } => MsgKey::AniListRateLimited,
            AniListError::Offline => MsgKey::AniListOffline,
            AniListError::UnknownStatus(_) => MsgKey::AniListUnknownStatus,
            AniListError::GraphQl(text) => MsgKey::TrackerError {
                tracker: "AniList".to_owned(),
                detail: text.clone(),
            },
            AniListError::Track(t) => MsgKey::Raw(t.to_string()),
            AniListError::Prefs(_)
            | AniListError::Secret(_)
            | AniListError::Http(_)
            | AniListError::Transient
            | AniListError::Decode(_)
            | AniListError::UnknownScoreFormat(_)
            | AniListError::ScoreOutOfRange
            | AniListError::Transport(_) => MsgKey::UnknownError,
        }
    }

    /// Maps a transport failure. A failed connection is reported as [`AniListError::Offline`].
    pub fn from_transport(e: reqwest::Error) -> Self {
        if e.is_connect() {
            AniListError::Offline
        } else {
            AniListError::Transport(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{MsgKey, english};

    #[test]
    fn t_28a_045_every_failure_has_a_user_message_and_no_debug_text() {
        let cases = [
            AniListError::NotAuthenticated,
            AniListError::AuthExpired,
            AniListError::NotInList,
            AniListError::RateLimited { retry_after: None },
            AniListError::Offline,
            AniListError::UnknownStatus("X".into()),
            AniListError::Transient,
        ];
        for e in cases {
            let key = e.message();
            let text = english(&key);
            for debug in [
                "AuthExpired",
                "NotInList",
                "RateLimited",
                "UnknownStatus",
                "NotAuthenticated",
            ] {
                assert!(!text.contains(debug), "{text}");
            }
            let tracked: crate::error::TrackError = e.into();
            assert_eq!(tracked.to_string(), text);
            assert_eq!(tracked.toast("AniList"), key);
        }
        assert_eq!(
            AniListError::AuthExpired.message(),
            MsgKey::AniListSessionExpired
        );
    }

    #[test]
    fn t_28a_042_not_in_list_and_expired_login_are_not_queued() {
        use crate::error::TrackError;
        assert!(TrackError::from(AniListError::NotInList).is_retry_futile());
        assert!(TrackError::from(AniListError::AuthExpired).is_retry_futile());
        assert!(!TrackError::from(AniListError::Offline).is_retry_futile());
        assert!(!TrackError::from(AniListError::Transient).is_retry_futile());
        assert!(
            !TrackError::from(AniListError::RateLimited { retry_after: None }).is_retry_futile()
        );
    }
}
