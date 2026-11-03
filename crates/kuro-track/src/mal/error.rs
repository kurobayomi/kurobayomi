//! Errors of the MAL tracker. The texts come from [`super::messages`]. The first
//! four are the Kotlin literals, kept verbatim so logs match upstream.

use super::messages as text;

#[derive(Debug, thiserror::Error)]
pub enum MalError {
    #[error("{}", text::NOT_AUTHENTICATED)]
    NotAuthenticated,
    #[error("{}", text::REFRESH_FAILED)]
    RefreshFailed,
    #[error("{}", text::LOGIN_EXPIRED)]
    LoginExpired,
    #[error("{prefix} {0}", prefix = text::HTTP_ERROR)]
    Http(u16),
    #[error("network error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("unexpected response: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("invalid url: {0}")]
    Url(#[from] url::ParseError),
    /// A local status code that MAL's list does not have. Refused before any PUT (D-23).
    #[error("{prefix} {0}", prefix = text::UNKNOWN_STATUS)]
    UnknownStatus(i64),
    /// A score label outside the picker list. Refused, the score is left unchanged (D-12).
    #[error("{prefix} {0}", prefix = text::UNKNOWN_SCORE)]
    UnknownScore(String),
    /// No client id in the config file or `KUROBAYOMI_MAL_CLIENT_ID` (T-29a-040). Login is not started.
    #[error("{}", text::NO_CLIENT_ID)]
    NoClientId,
    /// A redirect or paste arrived without a started login (E-LOGIN-4).
    #[error("{}", text::NO_PENDING_LOGIN)]
    NoPendingLogin,
    /// The user refused consent, or the redirect carried no code. The existing login stays (D-04).
    #[error("{}", text::LOGIN_NOT_COMPLETED)]
    LoginNotCompleted,
    /// No redirect within five minutes. The paste field still works (E-LOGIN-5).
    #[error("{}", text::LOGIN_TIMED_OUT)]
    LoginTimedOut,
    /// Pasted text that is neither a redirect URL nor a code (E-LOGIN-9).
    #[error("{}", text::PASTE_INVALID)]
    PasteInvalid,
    #[error("store: {0}")]
    Store(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_exact() {
        assert_eq!(
            MalError::NotAuthenticated.to_string(),
            "MAL: User is not authenticated"
        );
        assert_eq!(
            MalError::RefreshFailed.to_string(),
            "MAL: Failed to refresh account token"
        );
        assert_eq!(MalError::LoginExpired.to_string(), "MAL: Login has expired");
        assert_eq!(MalError::Http(404).to_string(), "HTTP error 404");
        assert_eq!(
            MalError::UnknownStatus(11).to_string(),
            "MAL: Unknown status code 11"
        );
        assert_eq!(
            MalError::UnknownScore("11".into()).to_string(),
            "MAL: Unknown score 11"
        );
        assert_eq!(
            MalError::NoClientId.to_string(),
            "Set a MyAnimeList client id to log in"
        );
        assert_eq!(
            MalError::LoginNotCompleted.to_string(),
            "Login was not completed"
        );
        assert_eq!(
            MalError::PasteInvalid.to_string(),
            "Paste the full redirect URL or the code"
        );
    }
}
