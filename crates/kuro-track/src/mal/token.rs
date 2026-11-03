//! OAuth token model and the expiry rule.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::dates::now_secs;

/// Early expiry margin in seconds (`MALOAuth.kt:22`).
pub const EXPIRY_MARGIN_S: i64 = 60;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MalToken {
    #[serde(default)]
    pub token_type: String,
    pub refresh_token: String,
    pub access_token: String,
    pub expires_in: i64,
    /// Epoch seconds. The token endpoint does not send it, so it defaults to the parse time (E-AUTH-8).
    #[serde(default = "now_secs")]
    pub created_at: i64,
}

impl MalToken {
    /// True when `now_s` is past `created_at + (expires_in - margin)`, strictly (`MALOAuth.kt:22-24`).
    pub fn is_expired(&self, now_s: i64) -> bool {
        let usable = (self.expires_in - margin(self.expires_in)).max(0);
        self.created_at + usable < now_s
    }
}

/// The early-expiry margin. Sixty seconds normally. Below sixty seconds of lifetime it is half the lifetime, so
/// such a token is not expired from the moment it arrives (D-19).
pub fn margin(expires_in: i64) -> i64 {
    if expires_in < EXPIRY_MARGIN_S {
        (expires_in / 2).max(0)
    } else {
        EXPIRY_MARGIN_S
    }
}

/// Keeps the secrets out of `Debug` output and logs.
impl fmt::Debug for MalToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MalToken")
            .field("token_type", &self.token_type)
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("expires_in", &self.expires_in)
            .field("created_at", &self.created_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(expires_in: i64, created_at: i64) -> MalToken {
        MalToken {
            token_type: "Bearer".into(),
            refresh_token: "r".into(),
            access_token: "a".into(),
            expires_in,
            created_at,
        }
    }

    #[test]
    fn token_expiry_margin() {
        let t = token(100, 0);
        assert!(!t.is_expired(40));
        assert!(t.is_expired(41));
    }

    #[test]
    fn token_expiry_short_lifetime_is_clamped() {
        let t = token(30, 0);
        assert_eq!(margin(30), 15);
        assert!(!t.is_expired(15));
        assert!(t.is_expired(16));
    }

    #[test]
    fn token_created_default() {
        let json = r#"{"token_type":"Bearer","expires_in":2592000,"access_token":"x","refresh_token":"y"}"#;
        let before = now_secs();
        let t: MalToken = serde_json::from_str(json).expect("decodes without created_at");
        let after = now_secs();
        assert!(t.created_at >= before && t.created_at <= after);
    }

    #[test]
    fn token_debug_hides_secrets() {
        let text = format!("{:?}", token(100, 0));
        assert!(!text.contains("\"a\"") && !text.contains("\"r\""));
    }
}
