//! One tracking row as the flows see it, anime or manga. The two stored shapes stay in `model.rs`; this enum is the
//! one type the Home list and the dialogs pass around.
//!
//! Decision (T-27b-001): the enum over `AnimeTrack` and `MangaTrack` replaces a flat struct.
//! The stored shapes already carry the fields with their own types, and the id is an `Option<i64>`, where `None` is a row that is not saved. The `-1` sentinel of the flat struct is
//! not used, because `None` cannot be mistaken for a real id. The accessors below are the one way the flows read a
//! field.

use kuro_platform::kind::Kind;
use crate::model::{AnimeTrack, MangaTrack, TrackerId};

#[derive(Debug, Clone, PartialEq)]
pub enum TrackRecord {
    Anime(AnimeTrack),
    Manga(MangaTrack),
}

impl TrackRecord {
    pub fn kind(&self) -> Kind {
        match self {
            TrackRecord::Anime(_) => Kind::Anime,
            TrackRecord::Manga(_) => Kind::Manga,
        }
    }

    pub fn tracker_id(&self) -> TrackerId {
        match self {
            TrackRecord::Anime(t) => t.tracker_id,
            TrackRecord::Manga(t) => t.tracker_id,
        }
    }

    /// The entry the row belongs to (`anime_id` or `manga_id`).
    pub fn entry_id(&self) -> i64 {
        match self {
            TrackRecord::Anime(t) => t.anime_id,
            TrackRecord::Manga(t) => t.manga_id,
        }
    }

    /// Last episode seen or last chapter read.
    pub fn progress(&self) -> f64 {
        match self {
            TrackRecord::Anime(t) => t.last_episode_seen,
            TrackRecord::Manga(t) => t.last_chapter_read,
        }
    }

    /// Episode or chapter count of the remote entry, `0` when unknown.
    pub fn total(&self) -> i64 {
        match self {
            TrackRecord::Anime(t) => t.total_episodes,
            TrackRecord::Manga(t) => t.total_chapters,
        }
    }

    pub fn status(&self) -> i64 {
        match self {
            TrackRecord::Anime(t) => t.status,
            TrackRecord::Manga(t) => t.status,
        }
    }

    pub fn score(&self) -> f64 {
        match self {
            TrackRecord::Anime(t) => t.score,
            TrackRecord::Manga(t) => t.score,
        }
    }

    pub fn remote_url(&self) -> &str {
        match self {
            TrackRecord::Anime(t) => &t.remote_url,
            TrackRecord::Manga(t) => &t.remote_url,
        }
    }

    pub fn private(&self) -> bool {
        match self {
            TrackRecord::Anime(t) => t.private,
            TrackRecord::Manga(t) => t.private,
        }
    }

    /// Epoch millis of the start date, `0` when unset.
    pub fn start_date(&self) -> i64 {
        match self {
            TrackRecord::Anime(t) => t.start_date,
            TrackRecord::Manga(t) => t.start_date,
        }
    }

    /// Epoch millis of the finish date, `0` when unset.
    pub fn finish_date(&self) -> i64 {
        match self {
            TrackRecord::Anime(t) => t.finish_date,
            TrackRecord::Manga(t) => t.finish_date,
        }
    }
}

impl From<AnimeTrack> for TrackRecord {
    fn from(t: AnimeTrack) -> Self {
        TrackRecord::Anime(t)
    }
}

impl From<MangaTrack> for TrackRecord {
    fn from(t: MangaTrack) -> Self {
        TrackRecord::Manga(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_27b_001_stored_rows_round_trip_through_the_record_with_every_field_set() {
        let anime = AnimeTrack {
            id: Some(11),
            anime_id: 7,
            tracker_id: TrackerId::KITSU,
            remote_id: 99,
            library_id: Some(55),
            title: "Show".into(),
            last_episode_seen: 4.0,
            total_episodes: 12,
            status: 2,
            score: 7.5,
            remote_url: "https://kitsu.app/anime/show".into(),
            start_date: 1_791_621_000_000,
            finish_date: 1_791_707_400_000,
            private: true,
        };
        let row = anime.to_db(true).unwrap();
        // The tracking URL goes to the URL field and progress to last_item.
        assert_eq!(row.remote_url, "https://kitsu.app/anime/show");
        assert_eq!(row.last_item, 4.0);
        let record = TrackRecord::from(AnimeTrack::from_db(&row));
        match &record {
            TrackRecord::Anime(back) => assert_eq!(back.to_db(true).unwrap(), row),
            TrackRecord::Manga(_) => panic!("an anime row became a manga record"),
        }
        assert_eq!(record.progress(), 4.0);

        let manga = MangaTrack {
            id: Some(12),
            manga_id: 9,
            tracker_id: TrackerId::ANILIST,
            remote_id: 3,
            library_id: Some(8),
            title: "Book".into(),
            last_chapter_read: 3.5,
            total_chapters: 40,
            status: 1,
            score: 6.0,
            remote_url: "https://anilist.co/manga/3".into(),
            start_date: 1,
            finish_date: 2,
            private: false,
        };
        let row = manga.to_db(true).unwrap();
        assert_eq!(row.remote_url, "https://anilist.co/manga/3");
        assert_eq!(row.last_item, 3.5);
        let record = TrackRecord::from(MangaTrack::from_db(&row));
        match &record {
            TrackRecord::Manga(back) => assert_eq!(back.to_db(true).unwrap(), row),
            TrackRecord::Anime(_) => panic!("a manga row became an anime record"),
        }
        assert_eq!(record.progress(), 3.5);
        assert_eq!((record.start_date(), record.finish_date()), (1, 2));
    }

    #[test]
    fn tr_27b_001_accessors_read_the_fields_of_each_shape() {
        let anime = TrackRecord::from(AnimeTrack {
            id: Some(3),
            anime_id: 7,
            tracker_id: TrackerId::KITSU,
            remote_id: 1,
            library_id: None,
            title: "A".into(),
            last_episode_seen: 4.0,
            total_episodes: 12,
            status: 2,
            score: 8.5,
            remote_url: "https://x.test/a".into(),
            start_date: 10,
            finish_date: 20,
            private: true,
        });
        assert_eq!(anime.kind(), Kind::Anime);
        assert_eq!(anime.entry_id(), 7);
        assert_eq!(anime.progress(), 4.0);
        assert_eq!(anime.total(), 12);
        assert_eq!((anime.start_date(), anime.finish_date()), (10, 20));
        assert!(anime.private());
        assert_eq!(anime.remote_url(), "https://x.test/a");

        let manga = TrackRecord::from(MangaTrack {
            id: None,
            manga_id: 9,
            tracker_id: TrackerId::KAVITA,
            remote_id: 1,
            library_id: None,
            title: "M".into(),
            last_chapter_read: 3.5,
            total_chapters: 0,
            status: 1,
            score: 0.0,
            remote_url: String::new(),
            start_date: 0,
            finish_date: 0,
            private: false,
        });
        assert_eq!(manga.kind(), Kind::Manga);
        assert_eq!(manga.entry_id(), 9);
        assert_eq!(manga.progress(), 3.5);
        assert_eq!(manga.tracker_id(), TrackerId::KAVITA);
    }
}
