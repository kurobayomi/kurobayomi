//! Serde shapes of the AniList responses (`p1-data-and-config.md` section 5). Every field the API may send
//! as `null` is an `Option`, so a missing staff list or studio list is an empty list and never a panic (E-07,
//! E-08, E-09).

use serde::Deserialize;
use serde_json::Value;

use super::error::AniListError;

/// `{ "data": ..., "errors": [...] }`. `errors` is checked before `data` is read.
#[derive(Debug, Deserialize)]
pub struct GraphQlEnvelope {
    #[serde(default)]
    pub data: Option<Value>,
    #[serde(default)]
    pub errors: Vec<GraphQlError>,
}

#[derive(Debug, Deserialize)]
pub struct GraphQlError {
    pub message: String,
}

impl GraphQlEnvelope {
    /// Reads `data` as `T`. Errors win over data (E-22), and a missing `data` is a decode error.
    pub fn into_data<T: serde::de::DeserializeOwned>(self) -> Result<T, AniListError> {
        if !self.errors.is_empty() {
            let text = self
                .errors
                .iter()
                .map(|e| e.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            return Err(AniListError::GraphQl(text));
        }
        let data = self.data.ok_or_else(|| {
            AniListError::Decode(<serde_json::Error as serde::de::Error>::custom(
                "response has no data",
            ))
        })?;
        serde_json::from_value(data).map_err(AniListError::Decode)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FuzzyDate {
    pub year: Option<i32>,
    pub month: Option<i32>,
    pub day: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct MediaTitle {
    #[serde(rename = "userPreferred")]
    pub user_preferred: Option<String>,
    /// Only the search queries ask for these three; the list queries leave them out.
    #[serde(default)]
    pub romaji: Option<String>,
    #[serde(default)]
    pub english: Option<String>,
    #[serde(default)]
    pub native: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CoverImage {
    pub large: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StaffName {
    #[serde(rename = "userPreferred")]
    pub user_preferred: Option<String>,
    pub full: Option<String>,
    pub native: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StaffNode {
    pub name: Option<StaffName>,
}

#[derive(Debug, Deserialize)]
pub struct StaffEdge {
    pub role: Option<String>,
    pub node: Option<StaffNode>,
}

#[derive(Debug, Deserialize)]
pub struct Staff {
    pub edges: Option<Vec<StaffEdge>>,
}

#[derive(Debug, Deserialize)]
pub struct StudioNode {
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioEdge {
    pub is_main: Option<bool>,
    pub node: Option<StudioNode>,
}

#[derive(Debug, Deserialize)]
pub struct Studios {
    pub edges: Option<Vec<StudioEdge>>,
}

/// A media item as returned by search, and as the `media` of a list entry. Manga carry `staff`, anime
/// carry `studios`; the other is absent.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    pub id: i64,
    pub title: Option<MediaTitle>,
    pub cover_image: Option<CoverImage>,
    pub description: Option<String>,
    pub format: Option<String>,
    pub status: Option<String>,
    pub chapters: Option<i64>,
    pub episodes: Option<i64>,
    pub average_score: Option<i64>,
    pub start_date: Option<FuzzyDate>,
    pub staff: Option<Staff>,
    pub studios: Option<Studios>,
    /// Alternative names, asked for by the search queries only.
    #[serde(default)]
    pub synonyms: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct MediaPage {
    #[serde(default)]
    pub media: Vec<Media>,
}

#[derive(Debug, Deserialize)]
pub struct SearchData {
    #[serde(rename = "Page")]
    pub page: MediaPage,
}

/// One entry of the user's list (`mediaList`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaListItem {
    pub id: i64,
    pub status: Option<String>,
    pub score_raw: Option<i64>,
    pub progress: Option<i64>,
    pub private: Option<bool>,
    pub started_at: Option<FuzzyDate>,
    pub completed_at: Option<FuzzyDate>,
    pub media: Media,
}

#[derive(Debug, Deserialize)]
pub struct MediaListPage {
    #[serde(default, rename = "mediaList")]
    pub media_list: Vec<MediaListItem>,
}

#[derive(Debug, Deserialize)]
pub struct FindData {
    #[serde(rename = "Page")]
    pub page: MediaListPage,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaListOptions {
    pub score_format: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewer {
    pub id: i64,
    pub media_list_options: Option<MediaListOptions>,
}

#[derive(Debug, Deserialize)]
pub struct ViewerData {
    #[serde(rename = "Viewer")]
    pub viewer: Viewer,
}

#[derive(Debug, Deserialize)]
pub struct SaveEntry {
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub struct SaveEntryData {
    #[serde(rename = "SaveMediaListEntry")]
    pub entry: SaveEntry,
}

#[derive(Debug, Deserialize)]
pub struct DeleteResult {
    pub deleted: bool,
}

#[derive(Debug, Deserialize)]
pub struct DeleteData {
    #[serde(rename = "DeleteMediaListEntry")]
    pub result: DeleteResult,
}

/// `Media` of the MAL id lookup. `idMal` is null for a media without a MyAnimeList entry.
#[derive(Debug, Deserialize)]
pub struct MalMedia {
    #[serde(rename = "idMal")]
    pub id_mal: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct MalMediaData {
    #[serde(rename = "Media")]
    pub media: Option<MalMedia>,
}

/// `Media` of the next-airing lookup. `nextAiringEpisode` is null when no episode is scheduled.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NextAiringMedia {
    pub next_airing_episode: Option<NextAiringEpisode>,
}

/// The scheduled episode: its number and its airing time in epoch seconds.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NextAiringEpisode {
    pub episode: i64,
    pub airing_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct NextAiringData {
    #[serde(rename = "Media")]
    pub media: Option<NextAiringMedia>,
}

/// `Media { id }` of the MAL-to-AniList lookup. Null when AniList has no media with that MAL id.
#[derive(Debug, Deserialize)]
pub struct AniListIdMedia {
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub struct AniListIdData {
    #[serde(rename = "Media")]
    pub media: Option<AniListIdMedia>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(text: &str) -> Result<GraphQlEnvelope, Box<dyn std::error::Error>> {
        Ok(serde_json::from_str(text)?)
    }

    #[test]
    fn t_28a_015_errors_on_http_200_are_a_graphql_error() -> Result<(), Box<dyn std::error::Error>>
    {
        let env = envelope(include_str!(
            "../../../tests/fixtures/anilist/errors_http200.json"
        ))?;
        let r: Result<SaveEntryData, _> = env.into_data();
        assert!(matches!(r, Err(AniListError::GraphQl(s)) if s == "Invalid id"));
        Ok(())
    }

    #[test]
    fn t_28a_015_partial_data_with_errors_is_still_an_error()
    -> Result<(), Box<dyn std::error::Error>> {
        let env = envelope(include_str!(
            "../../../tests/fixtures/anilist/errors_http200_partial.json"
        ))?;
        let r: Result<SaveEntryData, _> = env.into_data();
        assert!(matches!(r, Err(AniListError::GraphQl(_))));
        Ok(())
    }

    #[test]
    fn t_28a_018_null_staff_and_studios_parse_as_none() -> Result<(), Box<dyn std::error::Error>> {
        let env = envelope(include_str!(
            "../../../tests/fixtures/anilist/search_manga_no_staff.json"
        ))?;
        let data: SearchData = env.into_data()?;
        assert!(data.page.media.iter().all(|m| m.staff.is_none()));
        let env = envelope(include_str!(
            "../../../tests/fixtures/anilist/search_anime_no_studios.json"
        ))?;
        let data: SearchData = env.into_data()?;
        assert!(data.page.media.iter().all(|m| m.studios.is_none()));
        Ok(())
    }
}
