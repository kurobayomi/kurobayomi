//! Bangumi wire shapes (T-31a-023, T-31a-024, T-31a-025).
//! Unknown keys are ignored. Nullable fields are `Option` and default to `None` when absent.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Unix seconds now. The default of `created_at` when a token response has none (EC-B-04).
pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The OAuth token, stored whole as JSON (T-31a-025, T-31a-034). `user_id` is never read; it is kept when present
/// and omitted when absent, so the stored JSON round-trips.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BgmOAuth {
    pub access_token: String,
    pub token_type: String,
    /// Unix seconds. Set to the decode time when the response has none.
    #[serde(default = "now_secs")]
    pub created_at: i64,
    /// Seconds (604800 for Bangumi).
    pub expires_in: i64,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<i64>,
}

/// `GET /v0/me`. The login name (not the nickname) is the user name the collection paths use.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BgmUser {
    pub username: String,
}

/// `GET /v0/users/{username}/collections/{subject_id}` (T-31a-023). Every field defaults when absent.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BgmCollection {
    /// 0 to 10, where 0 means no rating.
    #[serde(default)]
    pub rate: Option<i64>,
    /// The collection type, 1 to 5 (see `BgmStatus`).
    #[serde(default, rename = "type")]
    pub kind: Option<i64>,
    #[serde(default)]
    pub ep_status: Option<i64>,
    #[serde(default)]
    pub vol_status: Option<i64>,
    /// Parsed and never mapped to the track (D14).
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub subject: Option<BgmSlimSubject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BgmSlimSubject {
    #[serde(default)]
    pub volumes: Option<i64>,
    #[serde(default)]
    pub eps: Option<i64>,
}

/// `GET /v0/episodes?subject_id=` (T-31a-044). Only the fields the progress write needs. `type` 0 is a main episode,
/// and `sort` orders them. Both default when absent.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BgmEpisode {
    pub id: i64,
    #[serde(default, rename = "type")]
    pub kind: i64,
    #[serde(default)]
    pub sort: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BgmEpisodeList {
    #[serde(default)]
    pub data: Vec<BgmEpisode>,
}

/// `POST /v0/search/subjects` (T-31a-024).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BgmSearchResult {
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
    #[serde(default)]
    pub data: Vec<BgmSubject>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BgmSubject {
    pub id: i64,
    /// Chinese name. Blank is allowed.
    pub name_cn: String,
    /// Original name.
    pub name: String,
    #[serde(default)]
    pub summary: Option<String>,
    /// `YYYY-MM-DD`.
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub images: Option<BgmImages>,
    #[serde(default)]
    pub volumes: i64,
    /// Episodes, or chapters for a manga.
    #[serde(default)]
    pub eps: i64,
    #[serde(default)]
    pub rating: Option<BgmRating>,
    /// `漫画` for manga. Null for most anime. Manga search keeps only null or `漫画`.
    #[serde(default)]
    pub platform: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BgmImages {
    #[serde(default)]
    pub common: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BgmRating {
    #[serde(default)]
    pub score: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_31a_025_a_token_without_created_at_gets_the_decode_time() {
        let before = now_secs();
        let t: BgmOAuth = serde_json::from_str(
            r#"{"access_token": "a", "token_type": "Bearer", "expires_in": 604800, "refresh_token": "r"}"#,
        )
        .unwrap();
        assert!(t.created_at >= before && t.created_at <= now_secs());
        assert_eq!(t.user_id, None);
    }

    #[test]
    fn t_31a_025_the_stored_json_round_trips_and_omits_an_absent_user_id() {
        let t: BgmOAuth = serde_json::from_str(
            r#"{"access_token": "a", "token_type": "Bearer", "created_at": 5, "expires_in": 604800, "refresh_token": "r", "user_id": 12345}"#,
        )
        .unwrap();
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<BgmOAuth>(&json).unwrap(), t);
        assert!(json.contains("\"user_id\":12345"));

        let mut absent = t.clone();
        absent.user_id = None;
        assert!(!serde_json::to_string(&absent).unwrap().contains("user_id"));
    }

    #[test]
    fn t_31a_023_a_collection_with_nothing_but_the_type_decodes_with_defaults() {
        let c: BgmCollection = serde_json::from_str(r#"{"type": 3}"#).unwrap();
        assert_eq!(c.kind, Some(3));
        assert_eq!((c.rate, c.ep_status, c.subject), (None, None, None));
        assert!(!c.private);
    }

    #[test]
    fn t_31a_024_a_search_subject_needs_its_names_and_id() {
        let s: BgmSubject = serde_json::from_str(
            r#"{"id": 1, "name_cn": "", "name": "Bleach", "platform": "漫画"}"#,
        )
        .unwrap();
        assert_eq!((s.volumes, s.eps), (0, 0));
        assert!(serde_json::from_str::<BgmSubject>(r#"{"id": 1, "name": "Bleach"}"#).is_err());
    }

    #[test]
    fn t_31a_043_every_bangumi_dto_ignores_unknown_keys_and_takes_absent_optionals() {
        let user: BgmUser =
            serde_json::from_str(r#"{"username": "Ann", "nickname": "A"}"#).unwrap();
        let result: BgmSearchResult =
            serde_json::from_str(r#"{"total": 0, "limit": 20, "offset": 0, "extra": 1}"#).unwrap();
        assert!(result.data.is_empty());
        let subject: BgmSubject = serde_json::from_str(
            r#"{"id": 1, "name_cn": "", "name": "B", "rating": {"score": null, "total": 0}, "images": {}}"#,
        )
        .unwrap();
        assert_eq!(subject.rating.unwrap().score, None);
        let episodes: BgmEpisodeList =
            serde_json::from_str(r#"{"data": [{"id": 5, "sort": 1, "name": "x"}], "total": 1}"#)
                .unwrap();
        assert_eq!((episodes.data[0].id, episodes.data[0].kind), (5, 0));
        assert_eq!(user.username, "Ann");
        assert!(serde_json::from_str::<BgmUser>("{}").is_err());
    }
}
