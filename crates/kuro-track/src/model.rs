//! Domain models of a tracker binding.

pub mod search;

use kuro_data::track::Track;
use kuro_platform::kind::Kind;

use crate::error::TrackError;

/// Tracker service id, stored as `sync_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrackerId(pub i64);

impl TrackerId {
    pub const MYANIMELIST: Self = Self(1);
    pub const ANILIST: Self = Self(2);
    pub const KITSU: Self = Self(3);
    pub const SHIKIMORI: Self = Self(4);
    pub const BANGUMI: Self = Self(5);
    pub const KOMGA: Self = Self(6);
    pub const MANGAUPDATES: Self = Self(7);
    pub const KAVITA: Self = Self(8);
    pub const SUWAYOMI: Self = Self(9);
    pub const SIMKL: Self = Self(101);
    pub const JELLYFIN: Self = Self(102);
}

/// One tracker binding of an anime. `id` is `None` before the first insert.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimeTrack {
    pub id: Option<i64>,
    pub anime_id: i64,
    pub tracker_id: TrackerId,
    pub remote_id: i64,
    pub library_id: Option<i64>,
    pub title: String,
    pub last_episode_seen: f64,
    pub total_episodes: i64,
    pub status: i64,
    pub score: f64,
    pub remote_url: String,
    /// Epoch milliseconds, 0 = unset.
    pub start_date: i64,
    /// Epoch milliseconds, 0 = unset.
    pub finish_date: i64,
    pub private: bool,
}

/// One tracker binding of a manga. `id` is `None` before the first insert.
#[derive(Debug, Clone, PartialEq)]
pub struct MangaTrack {
    pub id: Option<i64>,
    pub manga_id: i64,
    pub tracker_id: TrackerId,
    pub remote_id: i64,
    pub library_id: Option<i64>,
    pub title: String,
    pub last_chapter_read: f64,
    pub total_chapters: i64,
    pub status: i64,
    pub score: f64,
    pub remote_url: String,
    /// Epoch milliseconds, 0 = unset.
    pub start_date: i64,
    /// Epoch milliseconds, 0 = unset.
    pub finish_date: i64,
    pub private: bool,
}

/// Copies the personal fields of `src` into `dst`. `private` moves only when `copy_remote_private` is
/// true (the legacy form, TT-27-303).
pub fn copy_personal_from_anime(dst: &mut AnimeTrack, src: &AnimeTrack, copy_remote_private: bool) {
    dst.last_episode_seen = src.last_episode_seen;
    dst.score = src.score;
    dst.status = src.status;
    dst.start_date = src.start_date;
    dst.finish_date = src.finish_date;
    if copy_remote_private {
        dst.private = src.private;
    }
}

/// Manga form of [`copy_personal_from_anime`].
pub fn copy_personal_from_manga(dst: &mut MangaTrack, src: &MangaTrack, copy_remote_private: bool) {
    dst.last_chapter_read = src.last_chapter_read;
    dst.score = src.score;
    dst.status = src.status;
    dst.start_date = src.start_date;
    dst.finish_date = src.finish_date;
    if copy_remote_private {
        dst.private = src.private;
    }
}

impl AnimeTrack {
    /// The shared database row. A `None` id is written as 0 (unsaved, the upsert ignores it) unless `id_required`.
    pub fn to_db(&self, id_required: bool) -> Result<Track, TrackError> {
        Ok(Track {
            id: row_id(self.id, id_required)?,
            entry_id: self.anime_id,
            remote_id: self.remote_id,
            library_id: self.library_id,
            title: self.title.clone(),
            last_item: self.last_episode_seen,
            total_items: self.total_episodes,
            status: self.status,
            score: self.score,
            remote_url: self.remote_url.clone(),
            start_date: self.start_date,
            finish_date: self.finish_date,
            private: self.private,
            ..Track::new(Kind::Anime, self.tracker_id.0)
        })
    }

    /// Maps a database row back. The row id is always present.
    pub fn from_db(row: &Track) -> Self {
        Self {
            id: Some(row.id),
            anime_id: row.entry_id,
            tracker_id: TrackerId(row.tracker_id),
            remote_id: row.remote_id,
            library_id: row.library_id,
            title: row.title.clone(),
            last_episode_seen: row.last_item,
            total_episodes: row.total_items,
            status: row.status,
            score: row.score,
            remote_url: row.remote_url.clone(),
            start_date: row.start_date,
            finish_date: row.finish_date,
            private: row.private,
        }
    }
}

impl MangaTrack {
    /// The shared database row. Same id rule as [`AnimeTrack::to_db`].
    pub fn to_db(&self, id_required: bool) -> Result<Track, TrackError> {
        Ok(Track {
            id: row_id(self.id, id_required)?,
            entry_id: self.manga_id,
            remote_id: self.remote_id,
            library_id: self.library_id,
            title: self.title.clone(),
            last_item: self.last_chapter_read,
            total_items: self.total_chapters,
            status: self.status,
            score: self.score,
            remote_url: self.remote_url.clone(),
            start_date: self.start_date,
            finish_date: self.finish_date,
            private: self.private,
            ..Track::new(Kind::Manga, self.tracker_id.0)
        })
    }

    /// Maps a database row back. The row id is always present.
    pub fn from_db(row: &Track) -> Self {
        Self {
            id: Some(row.id),
            manga_id: row.entry_id,
            tracker_id: TrackerId(row.tracker_id),
            remote_id: row.remote_id,
            library_id: row.library_id,
            title: row.title.clone(),
            last_chapter_read: row.last_item,
            total_chapters: row.total_items,
            status: row.status,
            score: row.score,
            remote_url: row.remote_url.clone(),
            start_date: row.start_date,
            finish_date: row.finish_date,
            private: row.private,
        }
    }
}

fn row_id(id: Option<i64>, required: bool) -> Result<i64, TrackError> {
    match (id, required) {
        (Some(id), _) => Ok(id),
        (None, true) => Err(TrackError::MissingId),
        (None, false) => Ok(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anime() -> AnimeTrack {
        AnimeTrack {
            id: Some(7),
            anime_id: 42,
            tracker_id: TrackerId::ANILIST,
            remote_id: 900,
            library_id: Some(3),
            title: "Show".into(),
            last_episode_seen: 4.0,
            total_episodes: 12,
            status: 1,
            score: 8.0,
            remote_url: "https://example.test/900".into(),
            start_date: 10,
            finish_date: 20,
            private: true,
        }
    }

    #[test]
    fn tr_27_304_db_round_trip_is_identity_with_id() {
        let t = anime();
        assert_eq!(AnimeTrack::from_db(&t.to_db(true).unwrap()), t);
        let m = MangaTrack {
            id: Some(5),
            manga_id: 1,
            tracker_id: TrackerId::KITSU,
            remote_id: 2,
            library_id: None,
            title: "M".into(),
            last_chapter_read: 12.5,
            total_chapters: 0,
            status: 2,
            score: 0.0,
            remote_url: String::new(),
            start_date: 0,
            finish_date: 0,
            private: false,
        };
        assert_eq!(MangaTrack::from_db(&m.to_db(true).unwrap()), m);
    }

    #[test]
    fn tr_27_d12_missing_id_is_an_error_only_when_required() {
        let t = AnimeTrack {
            id: None,
            ..anime()
        };
        assert!(matches!(t.to_db(true), Err(TrackError::MissingId)));
        assert_eq!(t.to_db(false).unwrap().id, 0);
    }

    #[test]
    fn tr_27_303_copy_personal_keeps_private_only_when_asked() {
        let src = AnimeTrack {
            private: true,
            last_episode_seen: 9.0,
            ..anime()
        };
        let mut dst = AnimeTrack {
            private: false,
            last_episode_seen: 1.0,
            ..anime()
        };
        copy_personal_from_anime(&mut dst, &src, false);
        assert_eq!(dst.last_episode_seen, 9.0);
        assert!(!dst.private, "legacy form with false never changes private");
        copy_personal_from_anime(&mut dst, &src, true);
        assert!(dst.private);
    }

    #[test]
    fn copy_personal_copies_dates_and_score() {
        let src = MangaTrack {
            id: None,
            manga_id: 1,
            tracker_id: TrackerId::ANILIST,
            remote_id: 1,
            library_id: None,
            title: String::new(),
            last_chapter_read: 3.0,
            total_chapters: 0,
            status: 4,
            score: 7.0,
            remote_url: String::new(),
            start_date: 5,
            finish_date: 6,
            private: true,
        };
        let mut dst = MangaTrack {
            status: 0,
            score: 0.0,
            start_date: 0,
            finish_date: 0,
            private: false,
            ..src.clone()
        };
        copy_personal_from_manga(&mut dst, &src, true);
        assert_eq!(
            (dst.last_chapter_read, dst.status, dst.score),
            (3.0, 4, 7.0)
        );
        assert_eq!((dst.start_date, dst.finish_date, dst.private), (5, 6, true));
    }

    #[test]
    fn kind_text_round_trips() {
        assert_eq!(Kind::parse(Kind::Manga.as_str()), Some(Kind::Manga));
        assert_eq!(Kind::parse("novel"), None);
    }
}
