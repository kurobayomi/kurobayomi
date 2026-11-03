//! Shikimori wire shapes (T-31a-004). Unknown keys
//! are ignored. Nullable fields are `Option` and default to `None` when absent.

use serde::{Deserialize, Serialize};

/// Rejects `NaN` and infinite numbers after decoding (T-31a-043). serde_json already refuses them in JSON text, so this
/// guards the values that reach the rest of the code by another route. The error text names the first bad value.
pub fn ensure_finite(values: &[f64]) -> Result<(), String> {
    match values.iter().find(|v| !v.is_finite()) {
        Some(bad) => Err(format!("non-finite number {bad}")),
        None => Ok(()),
    }
}

/// The OAuth token. Stored whole, as JSON, so a rotated refresh token is kept (T-31a-005).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmOAuth {
    pub access_token: String,
    pub token_type: String,
    /// Unix seconds, from the server.
    pub created_at: i64,
    /// Seconds.
    pub expires_in: i64,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

/// `GET users/whoami`. The id is the user id that user rates are keyed by.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SmUser {
    pub id: i64,
}

/// `POST v2/user_rates` response: the user-rate id.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SmAddEntryResponse {
    pub id: i64,
}

/// A catalogue record (`mangas/{id}`, `animes/{id}` and the search list). `chapters`, `episodes` and `aired_on` may be
/// absent or null.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SmEntry {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub chapters: Option<i64>,
    #[serde(default)]
    pub episodes: Option<i64>,
    pub image: SmCover,
    pub score: f64,
    pub url: String,
    pub status: String,
    pub kind: String,
    #[serde(default)]
    pub aired_on: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SmCover {
    pub preview: String,
}

/// One row of `v2/user_rates`: the user's own rating of one catalogue record.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SmUserListEntry {
    /// The user-rate (library-entry) id.
    pub id: i64,
    pub chapters: f64,
    pub episodes: f64,
    pub score: i64,
    pub status: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_31a_004_entry_decodes_without_chapters_episodes_and_aired_on() {
        let entry: SmEntry = serde_json::from_str(
            r#"{"id": 1, "name": "N", "image": {"preview": "/p.jpg"}, "score": 7.5,
                "url": "/mangas/1", "status": "ongoing", "kind": "manga", "extra": true}"#,
        )
        .unwrap();
        assert_eq!(entry.chapters, None);
        assert_eq!(entry.episodes, None);
        assert_eq!(entry.aired_on, None);
        assert_eq!(entry.image.preview, "/p.jpg");
    }

    #[test]
    fn t_31a_004_user_rate_score_is_an_integer_and_counts_are_floats() {
        let row: SmUserListEntry = serde_json::from_str(
            r#"{"id": 9, "chapters": 3.5, "episodes": 0.0, "score": 8, "status": "watching"}"#,
        )
        .unwrap();
        assert_eq!(row.score, 8);
        assert_eq!(row.chapters, 3.5);
        assert!(
            serde_json::from_str::<SmUserListEntry>(
                r#"{"id": 9, "chapters": 1, "episodes": 0, "score": 8.5, "status": "x"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn t_31a_005_oauth_refresh_token_is_optional() {
        let t: SmOAuth = serde_json::from_str(
            r#"{"access_token": "a", "token_type": "Bearer", "created_at": 10, "expires_in": 86400}"#,
        )
        .unwrap();
        assert_eq!(t.refresh_token, None);
    }

    #[test]
    fn t_31a_043_every_shikimori_dto_ignores_unknown_keys_and_takes_absent_optionals() {
        let user: SmUser = serde_json::from_str(r#"{"id": 1, "extra": 2}"#).unwrap();
        let added: SmAddEntryResponse = serde_json::from_str(r#"{"id": 3, "x": null}"#).unwrap();
        let cover: SmCover =
            serde_json::from_str(r#"{"preview": "/p", "original": "/o"}"#).unwrap();
        let row: SmUserListEntry = serde_json::from_str(
            r#"{"id": 9, "chapters": 1, "episodes": 0, "score": 8, "status": "x", "extra": true}"#,
        )
        .unwrap();
        let token: SmOAuth = serde_json::from_str(
            r#"{"access_token": "a", "token_type": "Bearer", "created_at": 1, "expires_in": 2, "scope": "x"}"#,
        )
        .unwrap();
        assert_eq!((user.id, added.id, cover.preview.as_str()), (1, 3, "/p"));
        assert_eq!((row.id, row.score), (9, 8));
        assert_eq!(token.refresh_token, None);
    }

    #[test]
    fn t_31a_043_a_missing_required_key_fails() {
        assert!(serde_json::from_str::<SmUser>("{}").is_err());
        assert!(
            serde_json::from_str::<SmOAuth>(
                r#"{"token_type": "Bearer", "created_at": 1, "expires_in": 2}"#
            )
            .is_err()
        );
    }

    #[test]
    fn t_31a_043_non_finite_numbers_are_rejected() {
        assert!(ensure_finite(&[0.0, 9.5]).is_ok());
        assert!(ensure_finite(&[1.0, f64::NAN]).is_err());
        assert!(ensure_finite(&[f64::INFINITY]).is_err());
    }
}
