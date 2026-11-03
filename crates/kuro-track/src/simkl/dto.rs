//! Simkl wire shapes. Nullable keys are `Option`, so a missing key
//! reads as absent; the required keys are the ones the Kotlin declares without a default.

use serde::{Deserialize, Serialize};

/// `SimklOAuth`: the access token. The stored form is `{"access_token":"<token>"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimklOAuth {
    pub access_token: String,
}

/// `SimklUser` (`GET /users/settings`). `account` is required, so a reply without it is a decode error and never id 0.
#[derive(Debug, Clone, Deserialize)]
pub struct SimklUser {
    pub account: SimklAccount,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct SimklAccount {
    pub id: i64,
}

/// `SimklSyncWatched` (`POST /sync/watched`, one element per requested id).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SimklSyncWatched {
    pub result: Option<bool>,
    pub last_watched: Option<String>,
    /// The Simkl wire status of the title in the user's list, for example `watching`.
    pub list: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct SimklIds {
    pub simkl: i64,
}

/// The `show` or `movie` part of a sync item.
#[derive(Debug, Clone, Deserialize)]
pub struct SimklSyncResultItem {
    pub title: String,
    pub ids: SimklIds,
}

/// `SimklSyncItem`: one title of the user's list.
#[derive(Debug, Clone, Deserialize)]
pub struct SimklSyncItem {
    pub show: Option<SimklSyncResultItem>,
    pub movie: Option<SimklSyncResultItem>,
    pub total_episodes_count: Option<i64>,
    pub watched_episodes_count: Option<f64>,
    pub user_rating: Option<i32>,
}

impl SimklSyncItem {
    /// The part for `kind`: `show` or `movie`. Any other name has no part.
    pub fn result(&self, kind: &str) -> Option<&SimklSyncResultItem> {
        match kind {
            "show" => self.show.as_ref(),
            "movie" => self.movie.as_ref(),
            _ => None,
        }
    }
}

/// `SimklSyncResult` (`GET /sync/all-items/...`). Only the three response keys exist.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SimklSyncResult {
    pub anime: Option<Vec<SimklSyncItem>>,
    pub tv: Option<Vec<SimklSyncItem>>,
    pub movies: Option<Vec<SimklSyncItem>>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct SimklSearchIds {
    pub simkl_id: i64,
}

/// `SimklSearchResult` (`GET /search/<kind>`).
#[derive(Debug, Clone, Deserialize)]
pub struct SimklSearchResult {
    pub ids: SimklSearchIds,
    pub title_romaji: Option<String>,
    pub title: Option<String>,
    pub ep_count: Option<i64>,
    pub poster: Option<String>,
    pub all_titles: Option<Vec<String>>,
    /// The absolute page URL, for example `https://simkl.com/anime/1234/slug`.
    pub url: String,
    pub status: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub year: Option<i32>,
}

/// Request body element of `POST /sync/watched`.
#[derive(Serialize)]
pub struct WatchedQuery {
    pub simkl: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_32a_034_the_token_json_is_compact_and_only_the_access_token() {
        let json = serde_json::to_string(&SimklOAuth {
            access_token: "t".into(),
        })
        .unwrap();
        assert_eq!(json, r#"{"access_token":"t"}"#);
    }

    #[test]
    fn t_32a_033_a_user_reply_without_account_is_a_parse_error() {
        assert!(serde_json::from_str::<SimklUser>("{}").is_err());
        let ok: SimklUser = serde_json::from_str(r#"{"account":{"id":7}}"#).unwrap();
        assert_eq!(ok.account.id, 7);
    }

    #[test]
    fn t_32a_039_a_watched_reply_with_missing_keys_reads_them_as_absent() {
        let w: SimklSyncWatched = serde_json::from_str(r#"{"result":true}"#).unwrap();
        assert_eq!(w.result, Some(true));
        assert!(w.last_watched.is_none() && w.list.is_none());
    }

    #[test]
    fn t_32a_039_the_tv_key_is_read_from_the_sync_result() {
        let json = r#"{"tv":[{"show":{"title":"Show","ids":{"simkl":5}},"total_episodes_count":12,"watched_episodes_count":3.0,"user_rating":null}]}"#;
        let r: SimklSyncResult = serde_json::from_str(json).unwrap();
        assert!(r.anime.is_none() && r.movies.is_none());
        let item = &r.tv.unwrap()[0];
        assert_eq!(item.result("show").unwrap().ids.simkl, 5);
        assert_eq!(item.total_episodes_count, Some(12));
        assert!(item.result("movie").is_none());
    }

    #[test]
    fn t_32a_037_a_search_result_reads_type_from_the_type_key() {
        let json =
            r#"{"ids":{"simkl_id":9},"url":"https://simkl.com/tv/9/x","type":"tv","year":2001}"#;
        let r: SimklSearchResult = serde_json::from_str(json).unwrap();
        assert_eq!(r.ids.simkl_id, 9);
        assert_eq!(r.kind.as_deref(), Some("tv"));
        assert_eq!(r.year, Some(2001));
        assert!(r.title.is_none() && r.poster.is_none());
    }

    #[test]
    fn t_32a_037_a_search_result_without_url_is_a_parse_error() {
        assert!(serde_json::from_str::<SimklSearchResult>(r#"{"ids":{"simkl_id":1}}"#).is_err());
    }
}
