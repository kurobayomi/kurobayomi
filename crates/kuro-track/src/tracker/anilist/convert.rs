//! DTO to domain conversion (`p1-behavior.md` sections 7.1 to 7.5). Search results keep `score` as `None`
//! for the source's `-1` placeholder. Anime search results never carry artists (E-66).

use crate::model::search::{AnimeTrackSearch, MangaTrackSearch};
use crate::model::{AnimeTrack, MangaTrack, TrackerId};

use super::date::{format_ymd, fuzzy_to_ms};
use super::dto::{FuzzyDate, Media, MediaListItem, StaffNode, StudioEdge};
use super::error::AniListError;
use super::html;
use super::status::{AnimeStatus, ApiStatus, MangaStatus};

pub const MANGA_URL: &str = "https://anilist.co/manga/";
pub const ANIME_URL: &str = "https://anilist.co/anime/";

/// Browser link of a manga on AniList.
pub fn manga_url(id: i64) -> String {
    format!("{MANGA_URL}{id}")
}

/// Browser link of an anime on AniList.
pub fn anime_url(id: i64) -> String {
    format!("{ANIME_URL}{id}")
}

fn fuzzy_ms(d: Option<&FuzzyDate>) -> i64 {
    d.map_or(0, |d| fuzzy_to_ms(d.year, d.month, d.day))
}

fn title_of(m: &Media) -> String {
    m.title
        .as_ref()
        .and_then(|t| t.user_preferred.clone())
        .unwrap_or_default()
}

/// Every name of the media besides the display title, for the auto-track match.
fn alt_titles_of(m: &Media) -> Vec<String> {
    let t = m.title.as_ref();
    [t.and_then(|t| t.romaji.clone()), t.and_then(|t| t.english.clone()), t.and_then(|t| t.native.clone())]
        .into_iter()
        .flatten()
        .chain(m.synonyms.iter().flatten().cloned())
        .collect()
}

fn cover_of(m: &Media) -> String {
    m.cover_image
        .as_ref()
        .and_then(|c| c.large.clone())
        .unwrap_or_default()
}

fn publishing_type_of(m: &Media) -> String {
    m.format
        .as_deref()
        .map(|f| f.replace('_', "-"))
        .unwrap_or_default()
}

/// `userPreferred`, then `full`, then `native` (E-10). `None` when all three are missing.
fn staff_name(node: &StaffNode) -> Option<String> {
    let name = node.name.as_ref()?;
    name.user_preferred
        .clone()
        .or_else(|| name.full.clone())
        .or_else(|| name.native.clone())
}

/// Authors are staff whose role contains "Story"; artists are staff whose role contains "Art". A role of
/// "Story & Art" lands in both lists.
fn manga_people(m: &Media) -> (Vec<String>, Vec<String>) {
    let mut authors = Vec::new();
    let mut artists = Vec::new();
    let edges = m
        .staff
        .as_ref()
        .and_then(|s| s.edges.as_deref())
        .unwrap_or(&[]);
    for edge in edges {
        let Some(name) = edge.node.as_ref().and_then(staff_name) else {
            continue;
        };
        let role = edge.role.as_deref().unwrap_or_default();
        if role.contains("Story") {
            authors.push(name.clone());
        }
        if role.contains("Art") {
            artists.push(name);
        }
    }
    (authors, artists)
}

fn studio_name(edge: &StudioEdge) -> Option<String> {
    edge.node.as_ref().and_then(|n| n.name.clone())
}

/// The main studios, or every studio when none is main, capped at three.
fn anime_authors(m: &Media) -> Vec<String> {
    let edges = m
        .studios
        .as_ref()
        .and_then(|s| s.edges.as_deref())
        .unwrap_or(&[]);
    let mains: Vec<String> = edges
        .iter()
        .filter(|e| e.is_main == Some(true))
        .filter_map(studio_name)
        .collect();
    let chosen = if mains.is_empty() {
        edges.iter().filter_map(studio_name).collect()
    } else {
        mains
    };
    chosen.into_iter().take(3).collect()
}

/// Search row of a manga.
pub fn manga_search(m: &Media) -> MangaTrackSearch {
    let (authors, artists) = manga_people(m);
    let start_date_millis = fuzzy_ms(m.start_date.as_ref());
    let mut out = MangaTrackSearch::new(TrackerId::ANILIST, m.id, title_of(m), manga_url(m.id));
    out.total_chapters = m.chapters.unwrap_or(0);
    out.score = m.average_score.map(|v| v as f64);
    out.start_date_millis = start_date_millis;
    out.start_date = format_ymd(start_date_millis);
    out.authors = authors;
    out.artists = artists;
    out.cover_url = cover_of(m);
    out.summary = m
        .description
        .as_deref()
        .map(html::plain_text)
        .unwrap_or_default();
    out.publishing_type = publishing_type_of(m);
    out.publishing_status = m.status.clone().unwrap_or_default();
    out.alt_titles = alt_titles_of(m);
    out
}

/// Search row of an anime.
pub fn anime_search(m: &Media) -> AnimeTrackSearch {
    let start_date_millis = fuzzy_ms(m.start_date.as_ref());
    let mut out = AnimeTrackSearch::new(TrackerId::ANILIST, m.id, title_of(m), anime_url(m.id));
    out.total_episodes = m.episodes.unwrap_or(0);
    out.score = m.average_score.map(|v| v as f64);
    out.start_date_millis = start_date_millis;
    out.start_date = format_ymd(start_date_millis);
    out.authors = anime_authors(m);
    out.cover_url = cover_of(m);
    out.summary = m
        .description
        .as_deref()
        .map(html::plain_text)
        .unwrap_or_default();
    out.publishing_type = publishing_type_of(m);
    out.publishing_status = m.status.clone().unwrap_or_default();
    out.alt_titles = alt_titles_of(m);
    out
}

/// The local manga binding of a list entry. `library_id` is the MediaList id; `remote_url` is set from the
/// media id on every read (E-39).
pub fn manga_from_entry(e: &MediaListItem) -> Result<MangaTrack, AniListError> {
    let api = ApiStatus::from_api_str(e.status.as_deref().unwrap_or_default())?;
    Ok(MangaTrack {
        id: None,
        manga_id: 0,
        tracker_id: TrackerId::ANILIST,
        remote_id: e.media.id,
        library_id: Some(e.id),
        title: title_of(&e.media),
        last_chapter_read: e.progress.unwrap_or(0) as f64,
        total_chapters: e.media.chapters.unwrap_or(0),
        status: MangaStatus::from_api(api).to_db(),
        score: e.score_raw.unwrap_or(0) as f64,
        remote_url: manga_url(e.media.id),
        start_date: fuzzy_ms(e.started_at.as_ref()),
        finish_date: fuzzy_ms(e.completed_at.as_ref()),
        private: e.private.unwrap_or(false),
    })
}

/// The local anime binding of a list entry. Same rules as [`manga_from_entry`].
pub fn anime_from_entry(e: &MediaListItem) -> Result<AnimeTrack, AniListError> {
    let api = ApiStatus::from_api_str(e.status.as_deref().unwrap_or_default())?;
    Ok(AnimeTrack {
        id: None,
        anime_id: 0,
        tracker_id: TrackerId::ANILIST,
        remote_id: e.media.id,
        library_id: Some(e.id),
        title: title_of(&e.media),
        last_episode_seen: e.progress.unwrap_or(0) as f64,
        total_episodes: e.media.episodes.unwrap_or(0),
        status: AnimeStatus::from_api(api).to_db(),
        score: e.score_raw.unwrap_or(0) as f64,
        remote_url: anime_url(e.media.id),
        start_date: fuzzy_ms(e.started_at.as_ref()),
        finish_date: fuzzy_ms(e.completed_at.as_ref()),
        private: e.private.unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::anilist::dto::{GraphQlEnvelope, MediaListPage, SearchData};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn manga_results() -> Result<Vec<Media>, Box<dyn std::error::Error>> {
        let env: GraphQlEnvelope = serde_json::from_str(include_str!(
            "../../../tests/fixtures/anilist/search_manga_ok.json"
        ))?;
        let data: SearchData = env.into_data()?;
        Ok(data.page.media)
    }

    #[test]
    fn t_28a_016_manga_search_returns_three_results_and_urls() -> TestResult {
        let media = manga_results()?;
        let rows: Vec<MangaTrackSearch> = media.iter().map(manga_search).collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].tracking_url, "https://anilist.co/manga/30001");
        assert_eq!(rows[0].remote_id, 30001);
        Ok(())
    }

    #[test]
    fn t_28a_018_manga_search_conversion_rules() -> TestResult {
        let media = manga_results()?;
        let first = manga_search(&media[0]);
        assert_eq!(first.publishing_type, "ONE-SHOT");
        assert_eq!(first.publishing_status, "NOT_YET_RELEASED");
        assert_eq!(first.start_date, "", "year-only date is blank (E-28)");
        assert_eq!(first.score, None, "null averageScore is no score");
        assert_eq!(first.authors, vec!["Ann Author", "Bo Writer"]);
        assert_eq!(
            first.artists,
            vec!["Ann Author"],
            "Story & Art feeds artists"
        );
        assert_eq!(first.summary, "A line\nwith & and 'quotes' here");
        assert_eq!(first.cover_url, "https://img.test/a.jpg");
        let second = manga_search(&media[1]);
        assert_eq!(second.score, Some(72.0));
        assert_eq!(second.total_chapters, 0);
        assert_eq!(
            second.artists,
            vec!["Cy Artist"],
            "name falls back to native, null names skipped"
        );
        assert_eq!(second.start_date, "2010-05-07");
        let third = manga_search(&media[2]);
        assert!(third.authors.is_empty(), "null staff does not panic (E-07)");
        assert_eq!(third.publishing_type, "");
        Ok(())
    }

    #[test]
    fn t_28a_017_anime_search_authors_are_main_studios_capped_at_three() -> TestResult {
        let env: GraphQlEnvelope = serde_json::from_str(include_str!(
            "../../../tests/fixtures/anilist/search_anime_ok.json"
        ))?;
        let data: SearchData = env.into_data()?;
        let rows: Vec<AnimeTrackSearch> = data.page.media.iter().map(anime_search).collect();
        assert_eq!(
            rows[0].authors,
            vec!["Main A", "Main B", "Main C"],
            "E-08 cap of three"
        );
        assert_eq!(
            rows[1].authors,
            vec!["Side Studio"],
            "no main studio falls back to all"
        );
        assert!(
            rows[0].artists.is_empty(),
            "anime never carries artists (E-66)"
        );
        assert_eq!(rows[0].total_episodes, 0);
        assert_eq!(rows[1].total_episodes, 2);
        Ok(())
    }

    #[test]
    fn t_28a_018_anime_without_studios_gives_empty_authors() -> TestResult {
        let env: GraphQlEnvelope = serde_json::from_str(include_str!(
            "../../../tests/fixtures/anilist/search_anime_no_studios.json"
        ))?;
        let data: SearchData = env.into_data()?;
        let row = anime_search(&data.page.media[0]);
        assert!(row.authors.is_empty());
        assert_eq!(row.score, None);
        Ok(())
    }

    #[test]
    fn t_28a_019_list_entry_maps_to_a_manga_track() -> TestResult {
        let env: GraphQlEnvelope = serde_json::from_str(include_str!(
            "../../../tests/fixtures/anilist/find_manga_found.json"
        ))?;
        let page: MediaListPage = env
            .into_data::<crate::tracker::anilist::dto::FindData>()?
            .page;
        let entry = page.media_list.first().ok_or("no entry")?;
        let track = manga_from_entry(entry)?;
        assert_eq!(
            track.status,
            MangaStatus::Rereading.to_db(),
            "REPEATING maps to REREADING"
        );
        assert_eq!(track.score, 85.0);
        assert_eq!(track.last_chapter_read, 12.0);
        assert_eq!(track.library_id, Some(9100));
        assert_eq!(track.remote_id, 30001);
        assert!(track.private);
        assert_eq!(track.start_date, 0, "year-only start date is 0");
        assert!(track.finish_date > 0);
        assert_eq!(track.remote_url, "https://anilist.co/manga/30001");
        Ok(())
    }

    #[test]
    fn t_28a_019_empty_list_has_no_entry() -> TestResult {
        let env: GraphQlEnvelope = serde_json::from_str(include_str!(
            "../../../tests/fixtures/anilist/find_manga_empty.json"
        ))?;
        let data: crate::tracker::anilist::dto::FindData = env.into_data()?;
        assert!(data.page.media_list.is_empty());
        Ok(())
    }

    #[test]
    fn t_28a_019_anime_entry_paused_maps_to_on_hold() -> TestResult {
        let env: GraphQlEnvelope = serde_json::from_str(include_str!(
            "../../../tests/fixtures/anilist/find_anime_found.json"
        ))?;
        let data: crate::tracker::anilist::dto::FindData = env.into_data()?;
        let track = anime_from_entry(data.page.media_list.first().ok_or("no entry")?)?;
        assert_eq!(track.status, AnimeStatus::OnHold.to_db());
        assert_eq!(track.score, 0.0);
        assert_eq!(track.last_episode_seen, 4.0);
        assert_eq!(
            track.start_date, 0,
            "entry startedAt is null; the media startDate is not used"
        );
        Ok(())
    }
}
