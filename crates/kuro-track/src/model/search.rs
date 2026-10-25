//! Search results of a tracker (`AnimeTrackSearch.kt`, `MangaTrackSearch.kt`).

use super::{AnimeTrack, MangaTrack, TrackerId};

/// One anime found by a tracker search. `score` is `None` when the tracker has no score (replaces the
/// source sentinel -1.0). Equality uses `(anime_id, tracker_id, remote_id)`.
#[derive(Debug, Clone)]
pub struct AnimeTrackSearch {
    /// 0 until the result is registered for an entry.
    pub anime_id: i64,
    pub tracker_id: TrackerId,
    pub remote_id: i64,
    pub library_id: Option<i64>,
    pub title: String,
    pub last_episode_seen: f64,
    pub total_episodes: i64,
    pub score: Option<f64>,
    pub status: i64,
    pub start_date_millis: i64,
    pub finish_date_millis: i64,
    pub private: bool,
    pub tracking_url: String,
    /// The absolute page URL of the result when `tracking_url` is a relative form (Simkl stores `/anime/<id>`). Empty
    /// otherwise. "Open in browser" uses it before `tracking_url` (R-5.2, R-5.7).
    pub source_url: String,
    pub authors: Vec<String>,
    pub artists: Vec<String>,
    pub cover_url: String,
    pub summary: String,
    pub publishing_status: String,
    pub publishing_type: String,
    pub start_date: String,
    /// True for a row of the `my:` search: it comes from the user's own list, so the card shows no status and
    /// offers Refresh (T-29a-049).
    pub from_my_list: bool,
    /// Other names of the result (AniList: romaji, English, native, synonyms), for the auto-track match. Empty for
    /// trackers that do not send them.
    pub alt_titles: Vec<String>,
}

impl AnimeTrackSearch {
    /// A result with the two fields the UI needs to open it. Everything else starts empty.
    pub fn new(
        tracker_id: TrackerId,
        remote_id: i64,
        title: impl Into<String>,
        tracking_url: impl Into<String>,
    ) -> Self {
        Self {
            anime_id: 0,
            tracker_id,
            remote_id,
            library_id: None,
            title: title.into(),
            last_episode_seen: 0.0,
            total_episodes: 0,
            score: None,
            status: 0,
            start_date_millis: 0,
            finish_date_millis: 0,
            private: false,
            tracking_url: tracking_url.into(),
            source_url: String::new(),
            authors: Vec::new(),
            artists: Vec::new(),
            cover_url: String::new(),
            summary: String::new(),
            publishing_status: String::new(),
            publishing_type: String::new(),
            start_date: String::new(),
            from_my_list: false,
            alt_titles: Vec::new(),
        }
    }

    /// The list key used by the search screen: `(tracker_id, remote_id)`.
    pub fn list_key(&self) -> (TrackerId, i64) {
        (self.tracker_id, self.remote_id)
    }

    /// The URL "open in browser" opens: `source_url` when the tracker set one, else `tracking_url`.
    pub fn open_url(&self) -> &str {
        if self.source_url.is_empty() {
            &self.tracking_url
        } else {
            &self.source_url
        }
    }

    /// The binding this result would create. A missing score becomes 0 (no score).
    pub fn to_track(&self) -> AnimeTrack {
        AnimeTrack {
            id: None,
            anime_id: self.anime_id,
            tracker_id: self.tracker_id,
            remote_id: self.remote_id,
            library_id: self.library_id,
            title: self.title.clone(),
            last_episode_seen: self.last_episode_seen,
            total_episodes: self.total_episodes,
            status: self.status,
            score: self.score.unwrap_or(0.0),
            remote_url: self.tracking_url.clone(),
            start_date: self.start_date_millis,
            finish_date: self.finish_date_millis,
            private: self.private,
        }
    }
}

impl MangaTrackSearch {
    /// The binding this result would create. A missing score becomes 0 (no score).
    pub fn to_track(&self) -> MangaTrack {
        MangaTrack {
            id: None,
            manga_id: self.manga_id,
            tracker_id: self.tracker_id,
            remote_id: self.remote_id,
            library_id: self.library_id,
            title: self.title.clone(),
            last_chapter_read: self.last_chapter_read,
            total_chapters: self.total_chapters,
            status: self.status,
            score: self.score.unwrap_or(0.0),
            remote_url: self.tracking_url.clone(),
            start_date: self.start_date_millis,
            finish_date: self.finish_date_millis,
            private: self.private,
        }
    }
}

impl PartialEq for AnimeTrackSearch {
    fn eq(&self, other: &Self) -> bool {
        self.anime_id == other.anime_id
            && self.tracker_id == other.tracker_id
            && self.remote_id == other.remote_id
    }
}

impl Eq for AnimeTrackSearch {}

/// One manga found by a tracker search. Same rules as [`AnimeTrackSearch`].
#[derive(Debug, Clone)]
pub struct MangaTrackSearch {
    /// 0 until the result is registered for an entry.
    pub manga_id: i64,
    pub tracker_id: TrackerId,
    pub remote_id: i64,
    pub library_id: Option<i64>,
    pub title: String,
    pub last_chapter_read: f64,
    pub total_chapters: i64,
    pub score: Option<f64>,
    pub status: i64,
    pub start_date_millis: i64,
    pub finish_date_millis: i64,
    pub private: bool,
    pub tracking_url: String,
    pub authors: Vec<String>,
    pub artists: Vec<String>,
    pub cover_url: String,
    pub summary: String,
    pub publishing_status: String,
    pub publishing_type: String,
    pub start_date: String,
    /// True for a row of the `my:` search: it comes from the user's own list, so the card shows no status and
    /// offers Refresh (T-29a-049).
    pub from_my_list: bool,
    /// Other names of the result (AniList: romaji, English, native, synonyms), for the auto-track match. Empty for
    /// trackers that do not send them.
    pub alt_titles: Vec<String>,
}

impl MangaTrackSearch {
    /// A result with the two fields the UI needs to open it. Everything else starts empty.
    pub fn new(
        tracker_id: TrackerId,
        remote_id: i64,
        title: impl Into<String>,
        tracking_url: impl Into<String>,
    ) -> Self {
        Self {
            manga_id: 0,
            tracker_id,
            remote_id,
            library_id: None,
            title: title.into(),
            last_chapter_read: 0.0,
            total_chapters: 0,
            score: None,
            status: 0,
            start_date_millis: 0,
            finish_date_millis: 0,
            private: false,
            tracking_url: tracking_url.into(),
            authors: Vec::new(),
            artists: Vec::new(),
            cover_url: String::new(),
            summary: String::new(),
            publishing_status: String::new(),
            publishing_type: String::new(),
            start_date: String::new(),
            from_my_list: false,
            alt_titles: Vec::new(),
        }
    }

    /// The list key used by the search screen: `(tracker_id, remote_id)`.
    pub fn list_key(&self) -> (TrackerId, i64) {
        (self.tracker_id, self.remote_id)
    }
}

impl PartialEq for MangaTrackSearch {
    fn eq(&self, other: &Self) -> bool {
        self.manga_id == other.manga_id
            && self.tracker_id == other.tracker_id
            && self.remote_id == other.remote_id
    }
}

impl Eq for MangaTrackSearch {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tr_27_d10_equality_uses_entry_tracker_and_remote_id() {
        let a = AnimeTrackSearch::new(TrackerId::ANILIST, 5, "A", "u");
        let b = AnimeTrackSearch::new(TrackerId::KITSU, 5, "A", "u");
        assert_ne!(a, b, "same remote id, different tracker");
        assert_eq!(a.list_key(), (TrackerId::ANILIST, 5));
        assert_ne!(a.list_key(), b.list_key());
        let mut c = a.clone();
        c.title = "other title".into();
        assert_eq!(a, c, "title does not take part in equality");
    }

    #[test]
    fn tr_27_d09_required_fields_are_set_by_the_constructor() {
        let r = MangaTrackSearch::new(TrackerId::SUWAYOMI, 1, "Title", "https://x");
        assert_eq!(
            (r.title.as_str(), r.tracking_url.as_str()),
            ("Title", "https://x")
        );
        assert_eq!(r.score, None, "no score is None, not -1.0");
    }

    #[test]
    fn to_track_maps_missing_score_to_zero() {
        let mut r = AnimeTrackSearch::new(TrackerId::SIMKL, 3, "T", "https://t");
        r.anime_id = 11;
        let t = r.to_track();
        assert_eq!(
            (t.id, t.anime_id, t.score, t.remote_url.as_str()),
            (None, 11, 0.0, "https://t")
        );
        r.score = Some(6.0);
        assert_eq!(r.to_track().score, 6.0);
    }
}
