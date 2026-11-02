//! Shikimori catalogue record to search result (T-31a-009, T-31a-017, `dto/SMEntry.kt`). A manga record without a
//! chapter count is skipped with a warning, so one broken record does not hide the rest (OQ-1, the source fails the
//! whole search). An anime record without an episode count keeps a count of 0.

use super::dto::SmEntry;
use crate::model::TrackerId;
use crate::model::search::{AnimeTrackSearch, MangaTrackSearch};

/// The manga search result of a record. `base` is the site root, e.g. `https://shikimori.io`. `None` when the record
/// has no chapter count.
pub fn to_manga_search(base: &str, e: &SmEntry) -> Option<MangaTrackSearch> {
    let Some(chapters) = e.chapters else {
        tracing::warn!(
            "Shikimori manga {} has no chapter count, skipped from search",
            e.id
        );
        return None;
    };
    let mut out = MangaTrackSearch::new(
        TrackerId::SHIKIMORI,
        e.id,
        e.name.clone(),
        format!("{base}{}", e.url),
    );
    out.total_chapters = chapters;
    out.score = Some(e.score);
    out.cover_url = format!("{base}{}", e.image.preview);
    out.publishing_status = e.status.clone();
    out.publishing_type = e.kind.clone();
    out.start_date = e.aired_on.clone().unwrap_or_default();
    Some(out)
}

/// The anime search result of a record.
pub fn to_anime_search(base: &str, e: &SmEntry) -> AnimeTrackSearch {
    let mut out = AnimeTrackSearch::new(
        TrackerId::SHIKIMORI,
        e.id,
        e.name.clone(),
        format!("{base}{}", e.url),
    );
    out.total_episodes = e.episodes.unwrap_or(0);
    out.score = Some(e.score);
    out.cover_url = format!("{base}{}", e.image.preview);
    out.publishing_status = e.status.clone();
    out.publishing_type = e.kind.clone();
    out.start_date = e.aired_on.clone().unwrap_or_default();
    out
}
