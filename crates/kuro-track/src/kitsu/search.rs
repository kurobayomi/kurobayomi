//! Kitsu search through Algolia (T-30a-017, T-30a-018, T-30a-028). The key comes from Kitsu with the bearer token,
//! then the query goes to Algolia without one.

use kuro_platform::kind::Kind;
use chrono::{DateTime, Local};
use reqwest::Method;
use reqwest::header::CONTENT_TYPE;
use serde_json::{Value, json};

use super::api::{KitsuApi, read_success};
use super::dto::{AlgoliaHit, AlgoliaKeyResponse, AlgoliaResponse};
use super::error::KitsuError;
use crate::model::search::{AnimeTrackSearch, MangaTrackSearch};
use crate::model::TrackerId;

/// The Algolia application id (`X-Algolia-Application-Id`).
pub const APP_ID: &str = "AWQO5J657S";

/// The manga search filter, sent verbatim after `query=...` (`ALGOLIA_FILTER`).
pub const FILTER_MANGA: &str = "&facetFilters=%5B%22kind%3Amanga%22%5D&attributesToRetrieve=%5B%22synopsis%22%2C%22averageRating%22%2C%22canonicalTitle%22%2C%22chapterCount%22%2C%22posterImage%22%2C%22startDate%22%2C%22subtype%22%2C%22endDate%22%2C%20%22id%22%5D";
/// The anime search filter (`ALGOLIA_FILTER_ANIME`).
pub const FILTER_ANIME: &str = "&facetFilters=%5B%22kind%3Aanime%22%5D&attributesToRetrieve=%5B%22synopsis%22%2C%22averageRating%22%2C%22canonicalTitle%22%2C%22episodeCount%22%2C%22posterImage%22%2C%22startDate%22%2C%22subtype%22%2C%22endDate%22%2C%20%22id%22%5D";

fn filter(kind: Kind) -> &'static str {
    match kind {
        Kind::Manga => FILTER_MANGA,
        Kind::Anime => FILTER_ANIME,
    }
}

/// Percent-encodes like Java's `URLEncoder` (UTF-8): alphanumerics and `*-._` stay, a space becomes `+`, and every
/// other byte becomes `%XX` in upper case.
pub fn encode_query(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    for byte in query.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The tracking URL of a media id (`https://kitsu.app/<kind>/<id>`).
pub fn media_url(kind: Kind, media_id: i64) -> String {
    format!("https://kitsu.app/{}/{media_id}", kind.as_str())
}

/// The day of an Algolia start date (epoch seconds) in the host's zone, `yyyy-MM-dd` (E-56, a recorded decision).
pub fn start_date_text(epoch_secs: i64) -> String {
    DateTime::from_timestamp(epoch_secs, 0)
        .map(|at| at.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// Drops `novel` hits (exact match) and skips a hit that does not decode, with a log line (KT-72, KT-79). A null
/// subtype is kept. The order of the rest is kept.
pub fn decode_hits(hits: Vec<Value>) -> Vec<AlgoliaHit> {
    hits.into_iter()
        .filter(|raw| raw.get("subtype").and_then(Value::as_str) != Some("novel"))
        .filter_map(|raw| match serde_json::from_value::<AlgoliaHit>(raw) {
            Ok(hit) => Some(hit),
            Err(e) => {
                tracing::warn!("Kitsu search hit skipped: {e}");
                None
            }
        })
        .collect()
}

/// The fields that both kinds map the same way.
struct Common {
    title: String,
    total: i64,
    score: Option<f64>,
    cover_url: String,
    summary: String,
    publishing_status: String,
    publishing_type: String,
    start_date: String,
}

fn common(kind: Kind, hit: &AlgoliaHit) -> Common {
    Common {
        title: hit.canonical_title.clone(),
        total: match kind {
            Kind::Manga => hit.chapter_count,
            Kind::Anime => hit.episode_count,
        }
        .unwrap_or(0),
        // Algolia's `averageRating` absent is "no score" (the source's -1.0 sentinel).
        score: hit.average_rating,
        cover_url: hit
            .poster_image
            .as_ref()
            .and_then(|p| p.original.clone())
            .unwrap_or_default(),
        summary: hit.synopsis.clone().unwrap_or_default(),
        publishing_status: if hit.end_date.is_none() {
            "Publishing"
        } else {
            "Finished"
        }
        .to_owned(),
        publishing_type: hit.subtype.clone().unwrap_or_default(),
        start_date: hit.start_date.map(start_date_text).unwrap_or_default(),
    }
}

/// An anime search result of a hit (`toAnimeTrack` mapping).
pub fn to_anime_search(hit: &AlgoliaHit) -> AnimeTrackSearch {
    let c = common(Kind::Anime, hit);
    let mut out = AnimeTrackSearch::new(
        TrackerId::KITSU,
        hit.id,
        c.title,
        media_url(Kind::Anime, hit.id),
    );
    out.total_episodes = c.total;
    out.score = c.score;
    out.cover_url = c.cover_url;
    out.summary = c.summary;
    out.publishing_status = c.publishing_status;
    out.publishing_type = c.publishing_type;
    out.start_date = c.start_date;
    out
}

/// A manga search result of a hit (`toMangaTrack` mapping).
pub fn to_manga_search(hit: &AlgoliaHit) -> MangaTrackSearch {
    let c = common(Kind::Manga, hit);
    let mut out = MangaTrackSearch::new(
        TrackerId::KITSU,
        hit.id,
        c.title,
        media_url(Kind::Manga, hit.id),
    );
    out.total_chapters = c.total;
    out.score = c.score;
    out.cover_url = c.cover_url;
    out.summary = c.summary;
    out.publishing_status = c.publishing_status;
    out.publishing_type = c.publishing_type;
    out.start_date = c.start_date;
    out
}

impl KitsuApi {
    /// The Algolia key. Fetched with the bearer token once per session and then kept (T-30a-028). Logged out, this
    /// fails with `NotAuthenticated` before any request to Algolia (KT-75).
    async fn search_key(&self) -> Result<String, KitsuError> {
        if let Some(key) = self
            .algolia_key
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            return Ok(key);
        }
        let url = self.endpoint("algolia-keys/media/");
        let body = self.authed(Method::GET, url, None).await?;
        let key = serde_json::from_str::<AlgoliaKeyResponse>(&body)?.media.key;
        *self
            .algolia_key
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(key.clone());
        Ok(key)
    }

    /// One Algolia query with the given key. No bearer token is sent.
    async fn algolia_query(&self, key: &str, body: &Value) -> Result<Vec<Value>, KitsuError> {
        let response = self
            .http
            .post(&self.endpoints.algolia_url)
            .header("X-Algolia-Application-Id", APP_ID)
            .header("X-Algolia-API-Key", key)
            .header(CONTENT_TYPE, "application/json; charset=utf-8")
            .body(body.to_string())
            .send()
            .await?;
        let parsed: AlgoliaResponse = serde_json::from_str(&read_success(response).await?)?;
        Ok(parsed.hits)
    }

    /// Searches media of `kind`. The query is not trimmed and an empty query is sent as `query=` (KT-74). The first
    /// page only. An Algolia 403 drops the cached key, fetches it again and retries once; a second 403 is an error.
    pub async fn search_media(
        &self,
        kind: Kind,
        query: &str,
    ) -> Result<Vec<AlgoliaHit>, KitsuError> {
        let params = format!("query={}{}", encode_query(query), filter(kind));
        let body = json!({ "params": params });
        let key = self.search_key().await?;
        let hits = match self.algolia_query(&key, &body).await {
            Err(KitsuError::Http(403)) => {
                *self
                    .algolia_key
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
                let fresh = self.search_key().await?;
                self.algolia_query(&fresh, &body).await?
            }
            other => other?,
        };
        Ok(decode_hits(hits))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_30a_017_filter_constants_are_the_source_strings() {
        assert_eq!(
            FILTER_MANGA,
            "&facetFilters=%5B%22kind%3Amanga%22%5D&attributesToRetrieve=%5B%22synopsis%22%2C%22averageRating%22%2C%22canonicalTitle%22%2C%22chapterCount%22%2C%22posterImage%22%2C%22startDate%22%2C%22subtype%22%2C%22endDate%22%2C%20%22id%22%5D"
        );
        assert!(FILTER_ANIME.contains("episodeCount"));
        assert!(!FILTER_ANIME.contains("chapterCount"));
    }

    #[test]
    fn t_30a_017_query_encoding_matches_url_encoder() {
        assert_eq!(encode_query("a b&c~d*e"), "a+b%26c%7Ed*e");
        assert_eq!(encode_query(""), "");
        assert_eq!(encode_query("a-b_c.d"), "a-b_c.d");
        // Arabic text is percent-encoded as UTF-8 bytes.
        assert_eq!(encode_query("حلقة"), "%D8%AD%D9%84%D9%82%D8%A9");
    }

    #[test]
    fn t_30a_018_novel_is_dropped_and_null_subtype_is_kept() {
        let hits = vec![
            json!({"id": 1, "canonicalTitle": "A", "subtype": "novel"}),
            json!({"id": 2, "canonicalTitle": "B", "subtype": null}),
            json!({"id": 3, "canonicalTitle": "C", "subtype": "manga"}),
        ];
        let kept: Vec<i64> = decode_hits(hits).iter().map(|h| h.id).collect();
        assert_eq!(kept, vec![2, 3]);
    }

    #[test]
    fn t_30a_018_a_hit_without_its_title_is_skipped_and_the_rest_stay() {
        let hits = vec![
            json!({"id": 1}),
            json!({"id": "2", "canonicalTitle": "Good"}),
            json!({"id": 3, "canonicalTitle": "Also good"}),
        ];
        let kept = decode_hits(hits);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].id, 2);
    }

    #[test]
    fn t_30a_018_manga_hit_maps_to_the_track_search_fields() {
        let hit: AlgoliaHit = serde_json::from_value(json!({
            "id": "77", "canonicalTitle": "Title", "chapterCount": 12,
            "posterImage": {"original": "https://img.test/p.jpg"}, "synopsis": "S",
            "averageRating": 81.5, "startDate": 1_577_880_000, "endDate": null, "subtype": "manga"
        }))
        .unwrap();
        let m = to_manga_search(&hit);
        assert_eq!(m.tracker_id, TrackerId::KITSU);
        assert_eq!(m.remote_id, 77);
        assert_eq!(m.tracking_url, "https://kitsu.app/manga/77");
        assert_eq!(m.total_chapters, 12);
        assert_eq!(m.score, Some(81.5));
        assert_eq!(m.cover_url, "https://img.test/p.jpg");
        assert_eq!(m.summary, "S");
        assert_eq!(m.publishing_status, "Publishing");
        assert_eq!(m.publishing_type, "manga");
        assert_eq!(m.start_date, start_date_text(1_577_880_000));
    }

    #[test]
    fn t_30a_018_anime_hit_uses_episode_count_and_finished_when_end_date_is_set() {
        let hit: AlgoliaHit = serde_json::from_value(json!({
            "id": 5, "canonicalTitle": "Show", "episodeCount": 24, "endDate": 1_600_000_000
        }))
        .unwrap();
        let a = to_anime_search(&hit);
        assert_eq!(a.tracking_url, "https://kitsu.app/anime/5");
        assert_eq!(a.total_episodes, 24);
        assert_eq!(a.score, None, "a missing averageRating is no score");
        assert_eq!(a.publishing_status, "Finished");
        assert_eq!(a.start_date, "");
        assert_eq!(a.cover_url, "");
    }

    #[test]
    fn t_30a_018_a_missing_count_is_zero() {
        let hit: AlgoliaHit =
            serde_json::from_value(json!({"id": 9, "canonicalTitle": "T"})).unwrap();
        assert_eq!(to_manga_search(&hit).total_chapters, 0);
        assert_eq!(to_anime_search(&hit).total_episodes, 0);
    }

    #[test]
    fn t_30a_018_start_date_uses_the_host_zone() {
        let secs = 1_577_880_000;
        let expected = DateTime::from_timestamp(secs, 0)
            .unwrap()
            .with_timezone(&Local)
            .format("%Y-%m-%d")
            .to_string();
        assert_eq!(start_date_text(secs), expected);
        assert_eq!(start_date_text(0).len(), 10);
    }
}
