//! Suwayomi HTTP calls: the manga read, the chapter list and the chapter write (`SuwayomiApi.kt`;
//! T-33a-090 to T-33a-093, T-33a-096).

use std::sync::Arc;

use reqwest::StatusCode;
use reqwest::header::USER_AGENT;
use serde::de::DeserializeOwned;

use super::dto::{ChapterDataClass, MangaDataClass};
use super::{COMPLETED, READING, UNREAD};
use crate::error::redact;
use crate::model::search::MangaTrackSearch;
use crate::model::{MangaTrack, TrackerId};
use crate::trackers::enhanced::EnhancedError;
use crate::trackers::http::current_user_agent;
use crate::trackers::sources::{SourceDirectory, generate_source_id};

/// The source preference keys. They hold spaces and parentheses, as the extension writes them (`SuwayomiApi.kt:121-126`).
pub const KEY_SERVER_URL: &str = "Server URL Address";
pub const KEY_LOGIN: &str = "Login (Basic Auth)";
pub const KEY_PASSWORD: &str = "Password (Basic Auth)";

/// The settings as they are read for one call (T-33a-090).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SuwayomiSettings {
    pub base_url: String,
    pub login: String,
    pub password: String,
}

impl SuwayomiSettings {
    /// Basic auth is sent only when both values are set (`SuwayomiApi.kt:37-40`, E-62).
    pub fn has_basic_auth(&self) -> bool {
        !self.login.is_empty() && !self.password.is_empty()
    }
}

/// `suwayomi_status` (behaviour 4.4, E-54 to E-57). `None` (a null unread count) matches no branch and is READING.
pub fn suwayomi_status(unread: Option<i64>, chapters: i64) -> i64 {
    match unread {
        Some(u) if u == chapters => UNREAD,
        Some(0) => COMPLETED,
        _ => READING,
    }
}

pub struct SuwayomiApi {
    http: reqwest::Client,
    dir: Arc<dyn SourceDirectory>,
    source_id: i64,
}

impl SuwayomiApi {
    pub fn new(http: reqwest::Client, dir: Arc<dyn SourceDirectory>) -> Self {
        Self {
            http,
            dir,
            source_id: generate_source_id("tachidesk", "en", 1),
        }
    }

    /// Reads the three preferences now (D-08 fix, E-63). An extension that is not installed reads as empty settings.
    pub fn settings(&self) -> SuwayomiSettings {
        let Some(prefs) = self.dir.source_prefs(self.source_id) else {
            return SuwayomiSettings::default();
        };
        SuwayomiSettings {
            base_url: prefs.get_string(KEY_SERVER_URL, ""),
            login: prefs.get_string(KEY_LOGIN, ""),
            password: prefs.get_string(KEY_PASSWORD, ""),
        }
    }

    /// The read of a tracking URL (behaviour 4.3 and 4.4). A numeric URL is joined to the server address; any other
    /// URL is used as it is. An empty address with a numeric URL is `SourceNotConfigured` (D-26, E-52).
    pub async fn get_track_search(
        &self,
        track_url: &str,
    ) -> Result<MangaTrackSearch, EnhancedError> {
        let settings = self.settings();
        let url = match track_url.parse::<i64>() {
            Ok(id) => {
                if settings.base_url.is_empty() {
                    return Err(EnhancedError::SourceNotConfigured);
                }
                format!("{}/api/v1/manga/{id}", settings.base_url)
            }
            Err(_) => track_url.to_owned(),
        };
        let manga: MangaDataClass = self.get_json(&settings, &format!("{url}/full")).await?;
        let mut found = MangaTrackSearch::new(TrackerId::SUWAYOMI, 0, manga.title.clone(), url.clone());
        found.cover_url = format!("{url}/thumbnail");
        found.summary = manga.description.clone().unwrap_or_default();
        found.total_chapters = manga.chapter_count;
        found.publishing_status = manga.status.clone();
        found.last_chapter_read = manga
            .last_chapter_read
            .as_ref()
            .map_or(0.0, |c| c.chapter_number);
        found.status = suwayomi_status(manga.unread_count, manga.chapter_count);
        Ok(found)
    }

    /// Marks the chapter whose number equals the stored progress, then reads the item back (behaviour 4.5). The first
    /// chapter in list order wins (E-61). A number the server does not have is `ChapterNotOnServer` (D-25, E-60).
    pub async fn update_progress(&self, t: &MangaTrack) -> Result<MangaTrack, EnhancedError> {
        let settings = self.settings();
        let chapters: Vec<ChapterDataClass> = self
            .get_json(&settings, &format!("{}/chapters", t.remote_url))
            .await?;
        let target = t.last_chapter_read;
        let index = chapters
            .iter()
            .find(|c| c.chapter_number == target)
            .ok_or_else(|| EnhancedError::ChapterNotOnServer(format!("{target:?}")))?
            .index;
        let req = self
            .http
            .put(format!("{}/chapter/{index}", t.remote_url))
            .form(&[("markPrevRead", "true"), ("read", "true")]);
        apply_headers(req, &settings)
            .send()
            .await?
            .error_for_status()?;
        let remote = self.get_track_search(&t.remote_url).await?;
        Ok(apply_remote_progress(t, &remote))
    }

    /// A GET with the headers. A 401 is `Unauthorized`, any other failure status is an HTTP error.
    async fn get_json<T: DeserializeOwned>(
        &self,
        settings: &SuwayomiSettings,
        url: &str,
    ) -> Result<T, EnhancedError> {
        let resp = apply_headers(self.http.get(url), settings).send().await?;
        if resp.status() == StatusCode::UNAUTHORIZED {
            return Err(EnhancedError::Unauthorized);
        }
        resp.error_for_status()?
            .json::<T>()
            .await
            .map_err(|e| EnhancedError::Decode(redact(&e.to_string())))
    }
}

/// The user's default User-Agent (T-33a-091) and the Basic credential when both values are set (`SuwayomiApi.kt:35-43`, E-62).
fn apply_headers(
    req: reqwest::RequestBuilder,
    settings: &SuwayomiSettings,
) -> reqwest::RequestBuilder {
    let req = req.header(USER_AGENT, current_user_agent());
    if settings.has_basic_auth() {
        req.basic_auth(&settings.login, Some(&settings.password))
    } else {
        req
    }
}

/// The track after a sync: progress and status come from the server; the personal fields stay (D-03, E-91).
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
    fn t_33a_085_e54_null_unread_is_reading() {
        assert_eq!(suwayomi_status(None, 10), READING);
    }

    #[test]
    fn t_33a_085_e55_an_empty_manga_is_unread() {
        assert_eq!(suwayomi_status(Some(0), 0), UNREAD);
    }

    #[test]
    fn t_33a_085_e56_e57_all_read_and_all_unread() {
        assert_eq!(suwayomi_status(Some(0), 10), COMPLETED);
        assert_eq!(suwayomi_status(Some(10), 10), UNREAD);
    }

    #[test]
    fn t_33a_090_the_setting_keys_keep_their_spaces_and_parentheses() {
        assert_eq!(KEY_SERVER_URL, "Server URL Address");
        assert_eq!(KEY_LOGIN, "Login (Basic Auth)");
        assert_eq!(KEY_PASSWORD, "Password (Basic Auth)");
    }

    #[test]
    fn t_33a_091_basic_auth_needs_both_values() {
        let both = SuwayomiSettings {
            base_url: String::new(),
            login: "a".into(),
            password: "b".into(),
        };
        assert!(both.has_basic_auth());
        let one = SuwayomiSettings {
            password: String::new(),
            ..both
        };
        assert!(!one.has_basic_auth());
    }

    #[test]
    fn t_33a_091_the_request_carries_the_default_user_agent_preference() {
        use crate::trackers::http::{
            USER_AGENT as APP_USER_AGENT, USER_AGENT_TEST_LOCK, clear_user_agent_source, set_user_agent_source,
            tracker_client,
        };
        let _guard = USER_AGENT_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let build = || {
            apply_headers(tracker_client().unwrap().get("http://127.0.0.1:1/x"), &SuwayomiSettings::default())
                .build()
                .unwrap()
        };
        assert_eq!(build().headers()[USER_AGENT], APP_USER_AGENT);
        set_user_agent_source(Arc::new(|| "Mine/1.0".to_owned()));
        let req = build();
        clear_user_agent_source();
        assert_eq!(req.headers()[USER_AGENT], "Mine/1.0");
    }
}
