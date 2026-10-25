//! Search routing and result mapping (`p1-behavior.md` sections 5.3 to 5.5 and 7, `p1-rust-design.md` section 4).
//! Query routing: `id:<n>` is a direct lookup, `my:<title>` searches the user's own list, anything else is a text
//! search. Detail requests run with bounded concurrency, and results keep MAL's order.

use futures::{StreamExt, TryStreamExt};

use super::api::{ListKind, MalApi};
use super::consts::{
    DETAIL_CONCURRENCY, PAGE_SIZE, QUERY_MAX, SEARCH_ID_PREFIX, SEARCH_LIST_PREFIX,
};
use super::dates::parse_mal_date;
use super::dto::{AnimeDetails, MangaDetails};
use super::error::MalError;
use crate::model::TrackerId;
use crate::model::search::{AnimeTrackSearch, MangaTrackSearch};

/// The `q` value: at most [`QUERY_MAX`] UTF-16 units. The cut falls on a character boundary, so an emoji that
/// straddles the limit is dropped.
pub fn truncate_query(query: &str) -> &str {
    let mut units = 0;
    for (index, ch) in query.char_indices() {
        units += ch.len_utf16();
        if units > QUERY_MAX {
            return &query[..index];
        }
    }
    query
}

/// The id after `id:`, when it parses as an integer. `id:` alone and `id:abc` are text searches.
pub fn id_query(query: &str) -> Option<i64> {
    query.strip_prefix(SEARCH_ID_PREFIX)?.parse().ok()
}

/// The title filter after `my:`.
pub fn list_query(query: &str) -> Option<&str> {
    query.strip_prefix(SEARCH_LIST_PREFIX)
}

/// The `publishing_type` of a MAL media type, with `_` replaced by a space (`MyAnimeListApi.kt:141-142`).
fn publishing_text(value: &str) -> String {
    value.replace('_', " ")
}

/// The MAL mean as a score. The DTO keeps the Kotlin `-1.0` for "no mean"; the result model uses `None`.
fn score_of(mean: f64) -> Option<f64> {
    (mean >= 0.0).then_some(mean)
}

pub fn manga_hit(id: i64, d: &MangaDetails) -> MangaTrackSearch {
    let mut hit = MangaTrackSearch::new(
        TrackerId::MYANIMELIST,
        id,
        d.title.clone(),
        format!("https://myanimelist.net/manga/{id}"),
    );
    hit.summary = d.synopsis.clone();
    hit.total_chapters = d.num_chapters;
    hit.score = score_of(d.mean);
    hit.cover_url = d
        .main_picture
        .as_ref()
        .map(|c| c.large.clone())
        .unwrap_or_default();
    hit.publishing_status = publishing_text(&d.status);
    hit.publishing_type = publishing_text(&d.media_type);
    hit.start_date = d.start_date.clone().unwrap_or_default();
    hit.alt_titles = d.alternative_titles.all();
    hit
}

pub fn anime_hit(id: i64, d: &AnimeDetails) -> AnimeTrackSearch {
    let mut hit = AnimeTrackSearch::new(
        TrackerId::MYANIMELIST,
        id,
        d.title.clone(),
        format!("https://myanimelist.net/anime/{id}"),
    );
    hit.summary = d.synopsis.clone();
    hit.total_episodes = d.num_episodes;
    hit.score = score_of(d.mean);
    hit.cover_url = d
        .main_picture
        .as_ref()
        .map(|c| c.large.clone())
        .unwrap_or_default();
    hit.publishing_status = publishing_text(&d.status);
    hit.publishing_type = publishing_text(&d.media_type);
    hit.start_date = d.start_date.clone().unwrap_or_default();
    hit.alt_titles = d.alternative_titles.all();
    hit
}

/// Runs `fetch` for each id, at most [`DETAIL_CONCURRENCY`] at once (T-29a-037). Results keep the input order. One
/// failure fails the whole call, as upstream does.
pub(crate) async fn fetch_in_order<T, F, Fut>(ids: Vec<i64>, fetch: F) -> Result<Vec<T>, MalError>
where
    F: Fn(i64) -> Fut,
    Fut: std::future::Future<Output = Result<T, MalError>>,
{
    futures::stream::iter(ids)
        .map(fetch)
        .buffered(DETAIL_CONCURRENCY)
        .try_collect()
        .await
}

/// Walks the user's list page by page and returns the ids whose title contains `query` (case-insensitive).
async fn list_ids(api: &MalApi, kind: ListKind, query: &str) -> Result<Vec<i64>, MalError> {
    let needle = query.to_lowercase();
    let mut ids = Vec::new();
    let mut offset = 0u32;
    loop {
        let page = api.user_list_page(kind, offset).await?;
        ids.extend(
            page.data
                .iter()
                .filter(|item| item.node.title.to_lowercase().contains(&needle))
                .map(|item| item.node.id),
        );
        match page.paging.next {
            Some(next) if !next.is_empty() => offset += PAGE_SIZE,
            _ => return Ok(ids),
        }
    }
}

impl MalApi {
    /// Manga search (`MyAnimeList.kt:209-239`, `MyAnimeListApi.kt:81-98`). Novels are dropped from text results.
    pub async fn search_manga(&self, query: &str) -> Result<Vec<MangaTrackSearch>, MalError> {
        if let Some(id) = id_query(query) {
            let details = self.manga_details(id).await?;
            return Ok(vec![manga_hit(id, &details)]);
        }
        if let Some(title) = list_query(query) {
            let ids = list_ids(self, ListKind::Manga, title).await?;
            let details = fetch_in_order(ids.clone(), |id| self.manga_details(id)).await?;
            return Ok(ids
                .into_iter()
                .zip(details)
                .map(|(id, d)| {
                    let mut hit = manga_hit(id, &d);
                    hit.from_my_list = true;
                    hit
                })
                .collect());
        }
        let ids = self.search_ids("manga", truncate_query(query)).await?;
        let details = fetch_in_order(ids.clone(), |id| self.manga_details(id)).await?;
        Ok(ids
            .into_iter()
            .zip(details)
            .map(|(id, d)| manga_hit(id, &d))
            .filter(|hit| !hit.publishing_type.contains("novel"))
            .collect())
    }

    /// Anime search (`MyAnimeListApi.kt:100-117`). Single `q` (D-09). No novel filter.
    pub async fn search_anime(&self, query: &str) -> Result<Vec<AnimeTrackSearch>, MalError> {
        if let Some(id) = id_query(query) {
            let details = self.anime_details(id).await?;
            return Ok(vec![anime_hit(id, &details)]);
        }
        if let Some(title) = list_query(query) {
            // D-01: the anime list, not the manga list. Its ids are anime ids.
            let ids = list_ids(self, ListKind::Anime, title).await?;
            let details = fetch_in_order(ids.clone(), |id| self.anime_details(id)).await?;
            return Ok(ids
                .into_iter()
                .zip(details)
                .map(|(id, d)| {
                    let mut hit = anime_hit(id, &d);
                    hit.from_my_list = true;
                    hit
                })
                .collect());
        }
        let ids = self.search_ids("anime", truncate_query(query)).await?;
        let details = fetch_in_order(ids.clone(), |id| self.anime_details(id)).await?;
        Ok(ids
            .into_iter()
            .zip(details)
            .map(|(id, d)| anime_hit(id, &d))
            .collect())
    }
}

/// Date from a response field. Absent keeps the local value (`None` here). Empty or invalid text is 0 (D-08).
pub(crate) fn remote_date(text: Option<&str>) -> Option<i64> {
    let text = text?;
    if text.is_empty() {
        return Some(0);
    }
    Some(parse_mal_date(text).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_routing_id() {
        assert_eq!(id_query("id:42"), Some(42));
        assert_eq!(id_query("id:-7"), Some(-7));
        assert_eq!(id_query("id:abc"), None);
        assert_eq!(id_query("id:"), None);
        assert_eq!(list_query("my:"), Some(""));
        assert_eq!(list_query("my:Title"), Some("Title"));
    }

    #[test]
    fn truncate_counts_utf16_units() {
        let long = "a".repeat(80);
        assert_eq!(truncate_query(&long).len(), 64);
        // 63 ASCII then an emoji (two UTF-16 units): the emoji would reach 65, so it is dropped.
        let edge = format!("{}😀tail", "a".repeat(63));
        assert_eq!(truncate_query(&edge), "a".repeat(63));
        assert_eq!(truncate_query("short"), "short");
    }

    #[test]
    fn remote_date_rules() {
        assert_eq!(remote_date(None), None);
        assert_eq!(remote_date(Some("")), Some(0));
        assert_eq!(remote_date(Some("2020-02-30")), Some(0));
        assert!(remote_date(Some("2020-05")).unwrap() > 0);
    }

    #[test]
    fn manga_hit_maps_fixture_without_score() {
        let d: MangaDetails = serde_json::from_str(
            r#"{"id":2,"title":"Fixture Manga","synopsis":"","num_chapters":10,"status":"currently_publishing","media_type":"manga"}"#,
        )
        .unwrap();
        let hit = manga_hit(2, &d);
        assert_eq!(hit.score, None);
        assert_eq!(hit.cover_url, "");
        assert_eq!(hit.start_date, "");
        assert_eq!(hit.tracking_url, "https://myanimelist.net/manga/2");
        assert_eq!(hit.publishing_status, "currently publishing");
        assert_eq!(hit.total_chapters, 10);
    }

    #[test]
    fn search_routing_my_reads_the_list_by_title() {
        assert_eq!(list_query("my:Naruto"), Some("Naruto"));
        assert_eq!(list_query("naruto"), None);
    }

    #[tokio::test]
    async fn detail_fan_out_keeps_order_and_runs_four_at_a_time() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let ids: Vec<i64> = (1..=12).collect();
        let fetch = |id: i64| {
            let running = running.clone();
            let peak = peak.clone();
            async move {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                // Later ids finish first, so only the ordering code keeps the output in input order.
                tokio::time::sleep(Duration::from_millis(u64::try_from(20 - id).unwrap_or(0)))
                    .await;
                running.fetch_sub(1, Ordering::SeqCst);
                Ok::<i64, MalError>(id * 10)
            }
        };
        let out = fetch_in_order(ids.clone(), fetch)
            .await
            .expect("all succeed");
        assert_eq!(out, ids.iter().map(|id| id * 10).collect::<Vec<_>>());
        let observed = peak.load(Ordering::SeqCst);
        assert!(observed <= 4, "at most four in flight, saw {observed}");
        assert!(
            observed > 1,
            "the fan-out really runs in parallel, saw {observed}"
        );
    }

    #[tokio::test]
    async fn one_failed_detail_fails_the_whole_search_call() {
        let fetch = |id: i64| async move {
            if id == 2 {
                Err(MalError::Http(404))
            } else {
                Ok(id)
            }
        };
        let err = fetch_in_order(vec![1, 2, 3], fetch)
            .await
            .expect_err("one 404 fails the call");
        assert!(matches!(err, MalError::Http(404)));
    }

    #[test]
    fn absent_mean_is_no_score_and_a_mean_is_kept() {
        assert_eq!(score_of(-1.0), None);
        assert_eq!(score_of(0.0), Some(0.0));
        assert_eq!(score_of(8.27), Some(8.27));
    }

    #[test]
    fn novel_filter_is_case_sensitive_contains() {
        let novel: MangaDetails = serde_json::from_str(
            r#"{"id":1,"title":"Fixture Title","synopsis":"Text","num_chapters":0,"mean":8.27,"main_picture":{"large":"https://example.invalid/c.jpg"},"status":"finished","media_type":"light_novel","start_date":"2000-01-01"}"#,
        )
        .unwrap();
        let hit = manga_hit(1, &novel);
        assert!(hit.publishing_type.contains("novel"));
        assert_eq!(hit.score, Some(8.27));
        assert_eq!(hit.start_date, "2000-01-01");
    }
}
