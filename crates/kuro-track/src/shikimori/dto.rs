//! Shikimori wire shapes (T-31a-004). Unknown keys
//! are ignored. Nullable fields are `Option` and default to `None` when absent.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

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

/// A catalogue score. Shikimori sends it as text (`"score":"9.27"`, and `"0.0"` for a record nobody rated); a number
/// reads too, and a null or absent score is 0. Text that is not a number is a decode error.
fn catalogue_score<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wire {
        Number(f64),
        Text(String),
    }
    match Option::<Wire>::deserialize(deserializer)? {
        None => Ok(0.0),
        Some(Wire::Number(score)) => Ok(score),
        Some(Wire::Text(text)) => match text.trim().parse::<f64>() {
            Ok(score) if score.is_finite() => Ok(score),
            _ => Err(D::Error::custom(format!("score {text:?} is not a number"))),
        },
    }
}

/// A text field that may come as null: null reads as empty, so one record without it does not fail the whole list.
fn text_or_empty<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

/// A catalogue record (`mangas/{id}`, `animes/{id}` and the search list). `chapters`, `episodes` and `aired_on` may be
/// absent or null; `kind` and `status` may be null.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SmEntry {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub chapters: Option<i64>,
    #[serde(default)]
    pub episodes: Option<i64>,
    pub image: SmCover,
    #[serde(default, deserialize_with = "catalogue_score")]
    pub score: f64,
    pub url: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub status: String,
    #[serde(default, deserialize_with = "text_or_empty")]
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

    /// Shikimori's own answer to `mangas?order=popularity&search=Vagabond&limit=20` and `animes?...` (2026-11-05,
    /// trimmed to two records): the score is text. Issue 8: every search failed with "Invalid response".
    #[test]
    fn the_search_answer_of_the_live_site_decodes_with_its_scores_as_text() {
        let mangas: Vec<SmEntry> = serde_json::from_str(
            r#"[{"id":656,"name":"Vagabond","russian":"Бродяга","image":{"original":"/system/mangas/original/656.jpg?1708923143","preview":"/system/mangas/preview/656.jpg?1708923143","x96":"/system/mangas/x96/656.jpg?1708923143","x48":"/system/mangas/x48/656.jpg?1708923143"},"url":"/mangas/656-vagabond","kind":"manga","score":"9.27","status":"paused","volumes":37,"chapters":327,"aired_on":"1998-09-03","released_on":"2015-05-21"},
                {"id":194166,"name":"Burai Heiya","russian":"","image":{"original":"/assets/globals/missing_original.jpg","preview":"/assets/globals/missing_preview.jpg","x96":"/assets/globals/missing_x96.jpg","x48":"/assets/globals/missing_x48.jpg"},"url":"/mangas/194166-burai-heiya","kind":"manga","score":"0.0","status":"released","volumes":1,"chapters":5,"aired_on":null,"released_on":null}]"#,
        )
        .unwrap();
        assert_eq!((mangas[0].score, mangas[0].chapters, mangas[0].kind.as_str()), (9.27, Some(327), "manga"));
        assert_eq!(mangas[1].score, 0.0, "a record nobody rated");
        let animes: Vec<SmEntry> = serde_json::from_str(
            r#"[{"id":53337,"name":"Hyohakusha","russian":"Бродяга","image":{"original":"/system/animes/original/53337.jpg?1711935314","preview":"/system/animes/preview/53337.jpg?1711935314","x96":"/system/animes/x96/53337.jpg?1711935314","x48":"/system/animes/x48/53337.jpg?1711935314"},"url":"/animes/53337-hyohakusha","kind":"music","score":"6.3","status":"released","episodes":1,"episodes_aired":0,"aired_on":"2022-08-23","released_on":null}]"#,
        )
        .unwrap();
        assert_eq!((animes[0].score, animes[0].episodes), (6.3, Some(1)));
    }

    #[test]
    fn a_catalogue_score_reads_as_text_a_number_or_nothing_and_kind_and_status_take_null() {
        let entry = |score: &str| serde_json::from_str::<SmEntry>(&format!(
            r#"{{"id": 1, "name": "N", "image": {{"preview": "/p.jpg"}}, {score} "url": "/mangas/1", "status": null, "kind": null}}"#
        ));
        assert_eq!(entry(r#""score": " 8.5 ","#).unwrap().score, 8.5);
        assert_eq!(entry(r#""score": 7,"#).unwrap().score, 7.0);
        assert_eq!(entry(r#""score": null,"#).unwrap().score, 0.0);
        assert_eq!(entry("").unwrap().score, 0.0, "an absent score");
        let unrated = entry(r#""score": "0.0","#).unwrap();
        assert_eq!((unrated.status.as_str(), unrated.kind.as_str()), ("", ""));
        assert!(entry(r#""score": "n/a","#).is_err(), "text that is no number is a decode error");
        assert!(entry(r#""score": "NaN","#).is_err() && entry(r#""score": "inf","#).is_err(), "nor a non-finite one");
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
