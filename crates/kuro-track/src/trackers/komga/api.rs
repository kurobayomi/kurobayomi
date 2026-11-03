//! Komga HTTP calls: the series or readlist read, the progress read, and the progress write (`KomgaApi.kt`;
//! T-33a-061, T-33a-063, T-33a-070 to T-33a-073).

use reqwest::header::CONTENT_TYPE;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::auth::KomgaAuth;
use super::dto::{
    ReadListDto, ReadProgressDto, ReadProgressUpdateDto, ReadProgressUpdateV2Dto,
    ReadProgressV2Dto, SeriesDto,
};
use super::{COMPLETED, READING, UNREAD};
use crate::error::redact;
use crate::model::search::MangaTrackSearch;
use crate::model::{MangaTrack, TrackerId};
use crate::trackers::enhanced::EnhancedError;

/// A tracking URL with this part is a readlist, otherwise a series (`KomgaApi.kt:21`).
pub const READLIST_API: &str = "/api/v1/readlists";
const SERIES_V1: &str = "/api/v1/series/";
const SERIES_V2: &str = "/api/v2/series/";
const PROGRESS_SUFFIX: &str = "/read-progress/tachiyomi";
const JSON: &str = "application/json";

pub fn is_readlist(url: &str) -> bool {
    url.contains(READLIST_API)
}

/// The progress endpoint of a tracking URL. The series path moves to v2, because only v2 has the Mihon progress
/// endpoint (D-18 fix: a stored v2 URL is left as it is). A readlist URL has no series path, so it is not rewritten.
pub fn progress_url(url: &str) -> String {
    format!("{}{PROGRESS_SUFFIX}", url.replace(SERIES_V1, SERIES_V2))
}

/// `komga_status` (behaviour 3.3): an empty series is UNREAD (D-20), then all read is COMPLETED, else READING.
pub fn komga_status(books_count: i32, unread: i32, read: i32) -> i64 {
    if books_count == unread {
        UNREAD
    } else if books_count == read {
        COMPLETED
    } else {
        READING
    }
}

pub struct KomgaApi {
    http: reqwest::Client,
}

impl KomgaApi {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    /// The item read and the progress read (behaviour 3.3). The total is the max number, truncated (E-45).
    pub async fn get_track_search(
        &self,
        auth: &KomgaAuth,
        url: &str,
    ) -> Result<MangaTrackSearch, EnhancedError> {
        let (title, summary, publishing_status) = if is_readlist(url) {
            let list: ReadListDto = self.get_json(auth, url).await?;
            (list.name, String::new(), String::new())
        } else {
            let series: SeriesDto = self.get_json(auth, url).await?;
            (
                series.metadata.title,
                series.metadata.summary,
                series.metadata.status,
            )
        };
        let progress = self.progress(auth, url).await?;
        let mut found = MangaTrackSearch::new(TrackerId::KOMGA, 0, title, url);
        found.summary = summary;
        found.publishing_status = publishing_status;
        found.cover_url = format!("{url}/thumbnail");
        found.total_chapters = progress.max_number_sort as i64;
        found.status = komga_status(
            progress.books_count,
            progress.books_unread_count,
            progress.books_read_count,
        );
        found.last_chapter_read = progress.last_read_continuous_number_sort;
        Ok(found)
    }

    /// Writes the progress and reads the item back (behaviour 3.4). The server's progress is kept.
    pub async fn update_progress(
        &self,
        auth: &KomgaAuth,
        t: &MangaTrack,
    ) -> Result<MangaTrack, EnhancedError> {
        let body = if is_readlist(&t.remote_url) {
            // A readlist counts books, so the number is truncated (E-47).
            to_json(&ReadProgressUpdateDto {
                last_book_read: t.last_chapter_read as i32,
            })?
        } else {
            to_json(&ReadProgressUpdateV2Dto {
                last_book_number_sort_read: t.last_chapter_read,
            })?
        };
        auth.apply(self.http.put(progress_url(&t.remote_url)))
            .header(CONTENT_TYPE, JSON)
            .body(body)
            .send()
            .await?
            .error_for_status()?;
        let remote = self.get_track_search(auth, &t.remote_url).await?;
        Ok(apply_remote_progress(t, &remote))
    }

    /// Series use the v2 shape. A readlist uses v1 and converts it (D-18).
    async fn progress(
        &self,
        auth: &KomgaAuth,
        url: &str,
    ) -> Result<ReadProgressV2Dto, EnhancedError> {
        let target = progress_url(url);
        if is_readlist(url) {
            let v1: ReadProgressDto = self.get_json(auth, &target).await?;
            Ok(v1.to_v2())
        } else {
            self.get_json(auth, &target).await
        }
    }

    /// A GET with the credentials. A 401 is `Unauthorized` (E-49). Any other failure status is an error.
    async fn get_json<T: DeserializeOwned>(
        &self,
        auth: &KomgaAuth,
        url: &str,
    ) -> Result<T, EnhancedError> {
        let resp = auth.apply(self.http.get(url)).send().await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(EnhancedError::Unauthorized);
        }
        let resp = resp.error_for_status()?;
        resp.json::<T>()
            .await
            .map_err(|e| EnhancedError::Decode(redact(&e.to_string())))
    }
}

fn to_json<T: Serialize>(value: &T) -> Result<String, EnhancedError> {
    serde_json::to_string(value).map_err(|e| EnhancedError::Decode(e.to_string()))
}

/// The track after a sync: progress and status come from the server, and the personal fields stay (E-91).
pub fn apply_remote_progress(t: &MangaTrack, remote: &MangaTrackSearch) -> MangaTrack {
    let mut out = t.clone();
    out.status = remote.status;
    out.last_chapter_read = remote.last_chapter_read;
    out.total_chapters = remote.total_chapters;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_33a_070_readlist_urls_are_detected_and_not_rewritten() {
        let url = "https://h/api/v1/readlists/5";
        assert!(is_readlist(url));
        assert_eq!(
            progress_url(url),
            "https://h/api/v1/readlists/5/read-progress/tachiyomi"
        );
    }

    #[test]
    fn t_33a_071_e39_a_v1_series_url_moves_to_v2() {
        assert_eq!(
            progress_url("https://h/api/v1/series/9"),
            "https://h/api/v2/series/9/read-progress/tachiyomi"
        );
    }

    #[test]
    fn t_33a_071_e40_a_stored_v2_url_is_left_as_v2() {
        assert_eq!(
            progress_url("https://h/api/v2/series/9"),
            "https://h/api/v2/series/9/read-progress/tachiyomi"
        );
    }

    #[test]
    fn t_33a_073_e41_to_e44_the_status_branches() {
        assert_eq!(komga_status(0, 0, 0), UNREAD);
        assert_eq!(komga_status(10, 10, 0), UNREAD);
        assert_eq!(komga_status(10, 4, 6), READING);
        assert_eq!(komga_status(10, 0, 10), COMPLETED);
    }

    #[test]
    fn t_33a_071_e45_the_total_is_truncated() {
        let progress = ReadProgressV2Dto {
            books_count: 13,
            books_read_count: 0,
            books_unread_count: 13,
            books_in_progress_count: 0,
            last_read_continuous_number_sort: 0.0,
            max_number_sort: 12.9,
        };
        assert_eq!(progress.max_number_sort as i64, 12);
    }
}
