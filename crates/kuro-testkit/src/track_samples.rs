//! Sample tracks and search results for logic tests and GTK snapshot tests (T-27a-061). The values follow the Kotlin preview data
//! (`AnimeTrackInfoDialogHomePreviewProvider.kt:14-29`). The Compose preview
//! annotations are not ported.

use kuro_track::model::search::{AnimeTrackSearch, MangaTrackSearch};
use kuro_track::model::{AnimeTrack, MangaTrack, TrackerId};

/// The anime track of the preview: id 1, entry 2, tracker 1, remote 4, 2 of 12 episodes, status 1, score 2.0.
pub fn anime_track() -> AnimeTrack {
    AnimeTrack {
        id: Some(1),
        anime_id: 2,
        tracker_id: TrackerId::MYANIMELIST,
        remote_id: 4,
        library_id: None,
        title: "Manage Name On Tracker Site".to_owned(),
        last_episode_seen: 2.0,
        total_episodes: 12,
        status: 1,
        score: 2.0,
        remote_url: "https://example.com".to_owned(),
        start_date: 0,
        finish_date: 0,
        private: false,
    }
}

/// The same track marked private.
pub fn private_anime_track() -> AnimeTrack {
    AnimeTrack {
        private: true,
        ..anime_track()
    }
}

/// The same track with no total episode count.
pub fn anime_track_without_total() -> AnimeTrack {
    AnimeTrack {
        total_episodes: 0,
        ..anime_track()
    }
}

/// The manga counterpart of [`anime_track`]: the same remote id, two chapters of twelve read.
pub fn manga_track() -> MangaTrack {
    MangaTrack {
        id: Some(1),
        manga_id: 2,
        tracker_id: TrackerId::MYANIMELIST,
        remote_id: 4,
        library_id: None,
        title: "Manage Name On Tracker Site".to_owned(),
        last_chapter_read: 2.0,
        total_chapters: 12,
        status: 1,
        score: 2.0,
        remote_url: "https://example.com".to_owned(),
        start_date: 0,
        finish_date: 0,
        private: false,
    }
}

/// Three anime search results with distinct remote ids and tracker ids. The last one has no tracking URL (D9).
pub fn anime_search_results() -> Vec<AnimeTrackSearch> {
    vec![
        AnimeTrackSearch::new(TrackerId::ANILIST, 11, "First result", "https://anilist.example/11"),
        AnimeTrackSearch::new(TrackerId::KITSU, 12, "Second result", "https://kitsu.example/12"),
        AnimeTrackSearch::new(TrackerId::SHIKIMORI, 13, "Third result", ""),
    ]
}

/// Three manga search results with the same shape as [`anime_search_results`].
pub fn manga_search_results() -> Vec<MangaTrackSearch> {
    vec![
        MangaTrackSearch::new(TrackerId::ANILIST, 11, "First result", "https://anilist.example/11"),
        MangaTrackSearch::new(TrackerId::KITSU, 12, "Second result", "https://kitsu.example/12"),
        MangaTrackSearch::new(TrackerId::SHIKIMORI, 13, "Third result", ""),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t27a061_the_anime_sample_matches_the_preview_values() {
        let t = anime_track();
        assert_eq!((t.id, t.anime_id, t.tracker_id, t.remote_id), (Some(1), 2, TrackerId::MYANIMELIST, 4));
        assert_eq!((t.total_episodes, t.last_episode_seen, t.status, t.score), (12, 2.0, 1, 2.0));
        assert_eq!(t.remote_url, "https://example.com");
    }

    #[test]
    fn t27a061_the_variants_differ_only_where_they_should() {
        assert!(private_anime_track().private);
        assert_eq!(anime_track_without_total().total_episodes, 0);
        let m = manga_track();
        assert_eq!((m.manga_id, m.remote_id, m.total_chapters), (2, 4, 12));
    }

    #[test]
    fn t27a061_search_results_have_distinct_ids_and_one_without_a_url() {
        let anime = anime_search_results();
        let mut remote: Vec<i64> = anime.iter().map(|r| r.remote_id).collect();
        remote.sort_unstable();
        remote.dedup();
        assert_eq!(remote.len(), 3);
        assert_eq!(anime.iter().filter(|r| r.tracking_url.is_empty()).count(), 1);
        let manga = manga_search_results();
        assert_eq!(manga.len(), 3);
        assert_eq!(manga.iter().filter(|r| r.tracking_url.is_empty()).count(), 1);
    }
}
