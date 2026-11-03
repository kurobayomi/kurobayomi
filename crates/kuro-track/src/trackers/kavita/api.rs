//! Kavita HTTP calls: JWT login, the series read, the volume walk, the latest-chapter read and the chapter mark
//! (`KavitaApi.kt`; T-33a-050 to T-33a-056).

use reqwest::header::CONTENT_TYPE;
use reqwest::{RequestBuilder, Response, StatusCode, Url};
use serde::de::DeserializeOwned;

use super::auth::{KavitaAuth, authorize};
use super::dto::{AuthenticationDto, ChapterDto, SeriesDto, VolumeDto};
use super::{COMPLETED, READING, UNREAD};
use crate::error::redact;
use crate::model::search::MangaTrackSearch;
use crate::model::{MangaTrack, TrackerId};
use crate::trackers::enhanced::EnhancedError;

const JSON_UTF8: &str = "application/json; charset=utf-8";

pub struct KavitaApi {
    http: reqwest::Client,
    auth: KavitaAuth,
}

impl KavitaApi {
    pub fn new(http: reqwest::Client, auth: KavitaAuth) -> Self {
        Self { http, auth }
    }

    /// `url.split("/api/").first() + "/api"` (`KavitaApi.kt:30-32`). Works behind a reverse-proxy prefix (E-37).
    pub fn api_base(url: &str) -> String {
        format!("{}/api", url.split("/api/").next().unwrap_or(url))
    }

    /// The last non-empty path segment parsed as `i32` (D-30). The query and a trailing slash are ignored (E-35, E-36).
    pub fn series_id(url: &str) -> Result<i32, EnhancedError> {
        let parsed = Url::parse(url).map_err(|e| decode(&e.to_string()))?;
        parsed
            .path_segments()
            .and_then(|segments| segments.filter(|p| !p.is_empty()).last())
            .and_then(|p| p.parse::<i32>().ok())
            .ok_or_else(|| decode(&format!("no series id in {}", redact(url))))
    }

    /// The series read (behaviour 2.5): the series, then the total and the latest chapter in parallel.
    pub async fn get_track_search(&self, url: &str) -> Result<MangaTrackSearch, EnhancedError> {
        let api = Self::api_base(url);
        let id = Self::series_id(url)?;
        let series: SeriesDto = self.get_json(&api, url).await?;
        let (total, latest) = tokio::try_join!(
            self.total_chapters(&api, id),
            self.latest_chapter_read(&api, id),
        )?;
        let mut found = MangaTrackSearch::new(TrackerId::KAVITA, 0, series.name.clone(), url);
        // D-01 and T-33a-107: upstream never sends a thumbnail, so the cover stays empty until the cover endpoint is
        // confirmed on a live server. The series thumbnail is not read.
        found.cover_url = String::new();
        found.total_chapters = total;
        found.last_chapter_read = latest;
        found.status = kavita_status(series.pages, series.pages_read);
        Ok(found)
    }

    /// Marks chapters up to `t.last_chapter_read` as read, then reads the series back (behaviour 2.8). The server's
    /// status is the one kept (E-92, E-34).
    pub async fn update_progress(&self, t: &MangaTrack) -> Result<MangaTrack, EnhancedError> {
        let api = Self::api_base(&t.remote_url);
        let id = Self::series_id(&t.remote_url)?;
        let url = Url::parse_with_params(
            &format!("{api}/Tachiyomi/mark-chapter-until-as-read"),
            &[
                ("seriesId", id.to_string()),
                ("chapterNumber", kotlin_double(t.last_chapter_read)),
            ],
        )
        .map_err(|e| decode(&e.to_string()))?;
        self.send(&api, |c| {
            c.post(url.clone())
                .header(CONTENT_TYPE, JSON_UTF8)
                .body("{}")
        })
        .await?
        .error_for_status()?;
        let remote = self.get_track_search(&t.remote_url).await?;
        Ok(apply_remote_progress(t, &remote))
    }

    /// `GET` with the bearer token, a success check, and the body parsed as `T` (`awaitSuccess()`, behaviour 2.5).
    async fn get_json<T: DeserializeOwned>(
        &self,
        api: &str,
        url: &str,
    ) -> Result<T, EnhancedError> {
        let resp = self.send(api, |c| c.get(url)).await?.error_for_status()?;
        resp.json::<T>().await.map_err(|e| decode(&e.to_string()))
    }

    async fn total_chapters(&self, api: &str, id: i32) -> Result<i64, EnhancedError> {
        let url = format!("{api}/Series/volumes?seriesId={id}");
        let volumes: Vec<VolumeDto> = self.get_json(api, &url).await?;
        total_from_volumes(&volumes)
    }

    /// The latest read chapter (behaviour 2.7). Only a 200 is read. Any other status is `0.0` (E-29, E-31).
    async fn latest_chapter_read(&self, api: &str, id: i32) -> Result<f64, EnhancedError> {
        let url = format!("{api}/Tachiyomi/latest-chapter?seriesId={id}");
        let resp = self.send(api, |c| c.get(&url)).await?;
        if resp.status() != StatusCode::OK {
            return Ok(0.0);
        }
        let chapter: ChapterDto = resp.json().await.map_err(|e| decode(&e.to_string()))?;
        let raw = chapter
            .number
            .ok_or_else(|| decode("latest chapter number is null"))?;
        let value: f64 = raw
            .replace(',', ".")
            .parse()
            .map_err(|_| decode(&format!("latest chapter number {raw:?}")))?;
        // D-22: a stored "-1" would otherwise become a negative chapter read.
        Ok(value.max(0.0))
    }

    /// Sends with the bearer token of `api`. A 401 drops that token, logs in again and retries once (D-07, E-09). A
    /// second 401 is `Unauthorized`. Every other status is returned for the caller to check.
    async fn send(
        &self,
        api: &str,
        make: impl Fn(&reqwest::Client) -> RequestBuilder,
    ) -> Result<Response, EnhancedError> {
        let jwt = self.auth.token_for(api).await?;
        let resp = authorize(make(&self.http), &jwt).send().await?;
        if resp.status() != StatusCode::UNAUTHORIZED {
            return Ok(resp);
        }
        self.auth.invalidate(api).await;
        let jwt = self.auth.token_for(api).await?;
        let resp = authorize(make(&self.http), &jwt).send().await?;
        if resp.status() == StatusCode::UNAUTHORIZED {
            return Err(EnhancedError::Unauthorized);
        }
        Ok(resp)
    }
}

/// `Plugin/authenticate` (behaviour 2.3, with the D-06 error text). The key is query-encoded (E-11). A timeout is
/// `Ok(None)`. The key is never logged (E-14).
pub async fn authenticate(
    http: &reqwest::Client,
    api_url: &str,
    api_key: &str,
) -> Result<Option<String>, EnhancedError> {
    let url = Url::parse_with_params(
        &format!("{api_url}/Plugin/authenticate"),
        &[("apiKey", api_key), ("pluginName", "Tachiyomi-Kavita")],
    )
    .map_err(|e| decode(&e.to_string()))?;
    let resp = match http
        .post(url)
        .header(CONTENT_TYPE, JSON_UTF8)
        .body("{}")
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) if e.is_timeout() => {
            tracing::warn!(
                "Could not fetch JWT token, the url '{api_url}' is not available, skipping"
            );
            return Ok(None);
        }
        Err(e) => return Err(e.into()),
    };
    match resp.status() {
        StatusCode::OK => {
            let dto: AuthenticationDto = resp.json().await.map_err(|e| decode(&e.to_string()))?;
            Ok(Some(dto.token))
        }
        StatusCode::UNAUTHORIZED => {
            tracing::warn!(
                "Unauthorized / api key not valid: Cleaned api URL: {api_url}, Api key is empty: {}",
                api_key.is_empty()
            );
            Err(EnhancedError::Unauthorized)
        }
        StatusCode::INTERNAL_SERVER_ERROR => {
            tracing::warn!("Error fetching JWT token. Cleaned api URL: {api_url}");
            Err(EnhancedError::ServerError)
        }
        _ => Ok(None),
    }
}

/// `kavita_status` with the D-24 fix: an empty series (`0` of `0` read) is UNREAD. `pagesRead > pages` stays READING.
pub fn kavita_status(pages: i32, pages_read: i32) -> i64 {
    if pages_read == 0 {
        UNREAD
    } else if pages_read == pages {
        COMPLETED
    } else {
        READING
    }
}

/// The total chapter count from the volume list (behaviour 2.6). A volume with only `0` chapters counts once (single
/// file volumes). Otherwise the largest chapter number, truncated, competes with that count (E-21 to E-25). A volume
/// with no chapters is skipped (D-23). A non-numeric number is a decode error (E-26).
pub fn total_from_volumes(volumes: &[VolumeDto]) -> Result<i64, EnhancedError> {
    let mut volume_count: i64 = 0;
    let mut max_chapter: i64 = 0;
    for volume in volumes {
        if volume.chapters.is_empty() {
            continue;
        }
        let mut top = f32::MIN;
        for chapter in &volume.chapters {
            // A null number reads as -1 (E-27, documented difference from the Kotlin NullPointerException).
            let raw = chapter.number.as_deref().unwrap_or("-1");
            let n: f32 = raw
                .parse()
                .map_err(|_| decode(&format!("chapter number {raw:?}")))?;
            top = top.max(n);
        }
        if top == 0.0 {
            volume_count += 1;
        } else if (max_chapter as f32) < top {
            max_chapter = top as i64;
        }
    }
    Ok(if max_chapter > volume_count {
        max_chapter
    } else {
        volume_count
    })
}

/// Kotlin `Double.toString()` for a chapter number (pitfall P-6, E-32, E-33): `12.0`, `0.5`. Values below `1e-3` or at
/// least `1e7` use the `E` form, as Kotlin does (`1.0E-4`).
pub fn kotlin_double(value: f64) -> String {
    let magnitude = value.abs();
    if magnitude != 0.0 && (magnitude < 1e-3 || magnitude >= 1e7) {
        let text = format!("{value:e}");
        let (mantissa, exponent) = text.split_once('e').unwrap_or((text.as_str(), "0"));
        let mantissa = if mantissa.contains('.') {
            mantissa.to_owned()
        } else {
            format!("{mantissa}.0")
        };
        format!("{mantissa}E{exponent}")
    } else {
        format!("{value:?}")
    }
}

/// The track after a sync: progress and status come from the server, and the personal fields stay (E-91, E-92).
pub fn apply_remote_progress(t: &MangaTrack, remote: &MangaTrackSearch) -> MangaTrack {
    let mut out = t.clone();
    out.status = remote.status;
    out.last_chapter_read = remote.last_chapter_read;
    out.total_chapters = remote.total_chapters;
    out
}

fn decode(text: &str) -> EnhancedError {
    EnhancedError::Decode(redact(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volumes(json: &str) -> Vec<VolumeDto> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn t_33a_050_api_base_keeps_a_proxy_prefix() {
        assert_eq!(
            KavitaApi::api_base("https://h/kavita/api/series/12"),
            "https://h/kavita/api"
        );
        assert_eq!(
            KavitaApi::api_base("https://h/api/series/12"),
            "https://h/api"
        );
    }

    #[test]
    fn t_33a_050_e35_the_query_string_is_ignored_for_the_id() {
        assert_eq!(
            KavitaApi::series_id("https://h/api/series/12?x=1").unwrap(),
            12
        );
    }

    #[test]
    fn t_33a_050_e36_a_trailing_slash_is_skipped_for_the_id() {
        assert_eq!(
            KavitaApi::series_id("https://h/api/series/12/").unwrap(),
            12
        );
    }

    #[test]
    fn t_33a_050_a_non_numeric_id_is_a_decode_error() {
        assert!(matches!(
            KavitaApi::series_id("https://h/api/series/abc"),
            Err(EnhancedError::Decode(_))
        ));
    }

    #[test]
    fn t_33a_050_e37_the_proxy_prefix_gives_the_prefixed_api_base() {
        let url = "https://h/kavita/api/series/12";
        assert_eq!(KavitaApi::api_base(url), "https://h/kavita/api");
        assert_eq!(KavitaApi::series_id(url).unwrap(), 12);
    }

    #[test]
    fn t_33a_017_status_empty_series_is_unread_and_full_is_completed() {
        assert_eq!(kavita_status(0, 0), UNREAD);
        assert_eq!(kavita_status(10, 10), COMPLETED);
        assert_eq!(kavita_status(10, 12), READING);
        assert_eq!(kavita_status(10, 4), READING);
    }

    #[test]
    fn t_33a_053_e21_one_zero_volume_counts_one() {
        let v = volumes(r#"[{"chapters":[{"number":"0"}]}]"#);
        assert_eq!(total_from_volumes(&v).unwrap(), 1);
    }

    #[test]
    fn t_33a_053_e22_a_fractional_max_is_truncated() {
        let v = volumes(r#"[{"chapters":[{"number":"1"},{"number":"2"},{"number":"10.5"}]}]"#);
        assert_eq!(total_from_volumes(&v).unwrap(), 10);
    }

    #[test]
    fn t_33a_053_e23_the_chapter_max_beats_a_smaller_volume_count() {
        let v = volumes(
            r#"[{"chapters":[{"number":"0"}]},{"chapters":[{"number":"0"}]},{"chapters":[{"number":"5"}]}]"#,
        );
        assert_eq!(total_from_volumes(&v).unwrap(), 5);
    }

    #[test]
    fn t_33a_053_e24_the_volume_count_beats_a_smaller_max() {
        let v = volumes(
            r#"[{"chapters":[{"number":"0"}]},{"chapters":[{"number":"0"}]},{"chapters":[{"number":"0"}]},{"chapters":[{"number":"0"}]},{"chapters":[{"number":"1"},{"number":"2"}]}]"#,
        );
        assert_eq!(total_from_volumes(&v).unwrap(), 4);
    }

    #[test]
    fn t_33a_053_e25_an_empty_volume_is_skipped() {
        let v = volumes(r#"[{"chapters":[]},{"chapters":[{"number":"3"}]}]"#);
        assert_eq!(total_from_volumes(&v).unwrap(), 3);
    }

    #[test]
    fn t_33a_053_e26_a_non_numeric_number_is_a_decode_error() {
        let v = volumes(r#"[{"chapters":[{"number":"abc"}]}]"#);
        assert!(matches!(
            total_from_volumes(&v),
            Err(EnhancedError::Decode(_))
        ));
    }

    #[test]
    fn t_33a_053_e27_a_null_number_counts_as_minus_one() {
        let v = volumes(r#"[{"chapters":[{"number":null}]},{"chapters":[{"number":"2"}]}]"#);
        assert_eq!(total_from_volumes(&v).unwrap(), 2);
    }

    #[test]
    fn t_33a_056_e32_e33_a_whole_and_a_fraction_keep_the_kotlin_text() {
        assert_eq!(kotlin_double(12.0), "12.0");
        assert_eq!(kotlin_double(0.5), "0.5");
        assert_eq!(kotlin_double(12.5), "12.5");
    }

    #[test]
    fn t_33a_056_the_very_small_and_large_values_use_the_e_form() {
        assert_eq!(kotlin_double(1e-4), "1.0E-4");
        assert_eq!(kotlin_double(1e7), "1.0E7");
    }
}
