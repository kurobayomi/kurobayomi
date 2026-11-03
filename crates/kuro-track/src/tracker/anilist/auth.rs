//! OAuth for AniList: the implicit
//! grant, the redirect fragment, token expiry in seconds, and the state of the login.

use std::fmt;

use url::Url;

use super::score::ScoreFormat;

/// Lifetime assumed when the redirect carries no `expires_in`: 365 days, in seconds.
pub const DEFAULT_LIFETIME_S: i64 = 31_536_000;
/// Stored values above this are epoch milliseconds from the previous build (`10^12` ms is 2001).
const LEGACY_MS_THRESHOLD: i64 = 1_000_000_000_000;

/// The bearer token. `Debug` never prints it, and there is no `Display`.
#[derive(Clone, PartialEq, Eq)]
pub struct AccessToken(String);

impl AccessToken {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// The raw token. Only the HTTP layer and the secret store read it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AccessToken(<redacted>)")
    }
}

/// The result of reading the redirect URI the browser sends back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectOutcome {
    Token {
        token: AccessToken,
        /// Seconds, when AniList sent `expires_in`.
        expires_in: Option<i64>,
    },
    /// No `access_token` in the fragment: the user denied access, or the redirect is not a login.
    Denied,
}

/// The browser login URL. Query order is `client_id` then `response_type`, as the Kotlin builder produced.
pub const AUTHORIZE_URL: &str =
    "https://anilist.co/api/v2/oauth/authorize?client_id=53424&response_type=token";

/// Reads `access_token` and `expires_in` from the fragment as form data. No trailing `&` is needed (E-02).
pub fn parse_redirect(uri: &Url) -> RedirectOutcome {
    let fragment = uri.fragment().unwrap_or_default();
    let mut token = None;
    let mut expires_in = None;
    for (key, value) in url::form_urlencoded::parse(fragment.as_bytes()) {
        match key.as_ref() {
            "access_token" if !value.is_empty() => token = Some(value.into_owned()),
            "expires_in" => expires_in = value.parse::<i64>().ok(),
            _ => {}
        }
    }
    match token {
        Some(t) => RedirectOutcome::Token {
            token: AccessToken::new(t),
            expires_in,
        },
        None => RedirectOutcome::Denied,
    }
}

/// Stored expiry to unix seconds. Values above `10^12` were milliseconds and are divided by 1000.
pub fn normalize_expiry_s(stored: i64) -> i64 {
    if stored > LEGACY_MS_THRESHOLD {
        stored / 1000
    } else {
        stored
    }
}

/// The same comparison as the Kotlin `isExpired`: a token is expired only once `now` is past its expiry.
pub fn is_expired(expires_at_s: i64, now_s: i64) -> bool {
    now_s > expires_at_s
}

/// The login state. `Expired` keeps the viewer id so that the UI can ask for a new login.
#[derive(Clone, PartialEq, Eq)]
pub enum AuthState {
    LoggedOut,
    LoggedIn {
        token: AccessToken,
        viewer_id: i64,
        /// Unix seconds.
        expires_at: i64,
        score_format: ScoreFormat,
    },
    Expired {
        viewer_id: i64,
    },
}

impl fmt::Debug for AuthState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthState::LoggedOut => f.write_str("LoggedOut"),
            AuthState::LoggedIn {
                viewer_id,
                expires_at,
                score_format,
                ..
            } => f
                .debug_struct("LoggedIn")
                .field("viewer_id", viewer_id)
                .field("expires_at", expires_at)
                .field("score_format", score_format)
                .finish_non_exhaustive(),
            AuthState::Expired { viewer_id } => f
                .debug_struct("Expired")
                .field("viewer_id", viewer_id)
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn t_28a_008_authorize_url_is_exact() {
        assert_eq!(
            AUTHORIZE_URL,
            "https://anilist.co/api/v2/oauth/authorize?client_id=53424&response_type=token"
        );
    }

    #[test]
    fn t_28a_008_token_is_read_when_it_is_the_last_fragment_value() -> TestResult {
        let uri = Url::parse("kurobayomi://anilist-auth#access_token=abc")?;
        assert_eq!(
            parse_redirect(&uri),
            RedirectOutcome::Token {
                token: AccessToken::new("abc"),
                expires_in: None
            },
            "E-02"
        );
        Ok(())
    }

    #[test]
    fn t_28a_008_missing_token_is_denied() -> TestResult {
        let uri = Url::parse("kurobayomi://anilist-auth#error=access_denied")?;
        assert_eq!(parse_redirect(&uri), RedirectOutcome::Denied);
        let bare = Url::parse("kurobayomi://anilist-auth")?;
        assert_eq!(parse_redirect(&bare), RedirectOutcome::Denied);
        Ok(())
    }

    #[test]
    fn t_28a_008_expires_in_is_parsed_from_the_fragment() -> TestResult {
        let uri = Url::parse(
            "kurobayomi://anilist-auth#access_token=tok&token_type=Bearer&expires_in=31536000",
        )?;
        assert_eq!(
            parse_redirect(&uri),
            RedirectOutcome::Token {
                token: AccessToken::new("tok"),
                expires_in: Some(31_536_000)
            }
        );
        Ok(())
    }

    #[test]
    fn t_28a_008_token_debug_is_redacted() {
        let token = AccessToken::new("SECRET-TOKEN-VALUE");
        let shown = format!("{token:?}");
        assert!(!shown.contains("SECRET-TOKEN-VALUE"), "{shown}");
        let state = AuthState::LoggedIn {
            token,
            viewer_id: 1,
            expires_at: 2,
            score_format: ScoreFormat::Point10,
        };
        assert!(!format!("{state:?}").contains("SECRET-TOKEN-VALUE"));
    }

    #[test]
    fn t_28a_009_expiry_uses_seconds_and_strict_greater_than() {
        assert!(is_expired(100, 101), "past expiry is expired");
        assert!(
            !is_expired(100, 100),
            "equal is still valid, as isExpired uses >"
        );
        assert!(!is_expired(100, 99));
    }

    #[test]
    fn t_28a_009_legacy_millisecond_expiry_is_divided_by_1000() {
        assert_eq!(normalize_expiry_s(1_700_000_000_000), 1_700_000_000);
        assert_eq!(normalize_expiry_s(1_700_000_000), 1_700_000_000);
    }
}
