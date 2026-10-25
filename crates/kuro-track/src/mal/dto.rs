//! Response DTOs (`p1-data-and-config.md` section 3). Optional keys are `Option` or `#[serde(default)]`, and
//! unknown keys are ignored, as the Kotlin `ignoreUnknownKeys` setting does.

use serde::{Deserialize, Deserializer};

/// `null` in place of a string means the empty string (`synopsis` may be null on MAL).
fn nullable_string<'de, D: Deserializer<'de>>(de: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(de)?.unwrap_or_default())
}

/// `mean` absent or `null` is `-1.0`, the Kotlin default (`MALManga.kt`). The caller turns it into "no score".
fn mean_or_absent<'de, D: Deserializer<'de>>(de: D) -> Result<f64, D::Error> {
    Ok(Option::<f64>::deserialize(de)?.unwrap_or(ABSENT_MEAN))
}

const ABSENT_MEAN: f64 = -1.0;

fn absent_mean() -> f64 {
    ABSENT_MEAN
}

/// `GET users/@me` (`MALUser`).
#[derive(Debug, Deserialize)]
pub struct User {
    pub name: String,
}

/// `GET {manga,anime}?q=` (`MALSearchResult`).
#[derive(Debug, Deserialize)]
pub struct SearchPage {
    pub data: Vec<SearchNode>,
}

#[derive(Debug, Deserialize)]
pub struct SearchNode {
    pub node: NodeId,
}

#[derive(Debug, Deserialize)]
pub struct NodeId {
    pub id: i64,
}

#[derive(Debug, Deserialize)]
pub struct Covers {
    #[serde(default)]
    pub large: String,
}

/// `GET manga/{id}` (`MALManga`).
#[derive(Debug, Deserialize)]
pub struct MangaDetails {
    pub id: i64,
    pub title: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub synopsis: String,
    pub num_chapters: i64,
    #[serde(default = "absent_mean", deserialize_with = "mean_or_absent")]
    pub mean: f64,
    #[serde(default)]
    pub main_picture: Option<Covers>,
    pub status: String,
    pub media_type: String,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub alternative_titles: AltTitles,
}

/// `alternative_titles` of a detail response: the other names, used by the auto-track match.
#[derive(Debug, Default, Deserialize)]
pub struct AltTitles {
    #[serde(default)]
    pub synonyms: Vec<String>,
    #[serde(default)]
    pub en: Option<String>,
    #[serde(default)]
    pub ja: Option<String>,
}

impl AltTitles {
    pub fn all(&self) -> Vec<String> {
        self.en.iter().chain(self.ja.iter()).chain(self.synonyms.iter()).filter(|t| !t.is_empty()).cloned().collect()
    }
}

/// `GET anime/{id}` (`MALAnime`).
#[derive(Debug, Deserialize)]
pub struct AnimeDetails {
    pub id: i64,
    pub title: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub synopsis: String,
    pub num_episodes: i64,
    #[serde(default = "absent_mean", deserialize_with = "mean_or_absent")]
    pub mean: f64,
    #[serde(default)]
    pub main_picture: Option<Covers>,
    pub status: String,
    pub media_type: String,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub alternative_titles: AltTitles,
}

/// `GET manga/{id}?fields=num_chapters,my_list_status{...}` (`MALListMangaItem`).
#[derive(Debug, Deserialize)]
pub struct MangaListEntry {
    pub num_chapters: i64,
    #[serde(default)]
    pub my_list_status: Option<MangaListStatus>,
}

/// The list status of a manga, in responses and PUT replies (`MALListMangaItemStatus`).
#[derive(Debug, Deserialize)]
pub struct MangaListStatus {
    pub is_rereading: bool,
    pub status: String,
    pub num_chapters_read: f64,
    pub score: i64,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub finish_date: Option<String>,
}

/// `GET anime/{id}?fields=num_episodes,my_list_status{...}` (`MALListAnimeItem`).
#[derive(Debug, Deserialize)]
pub struct AnimeListEntry {
    pub num_episodes: i64,
    #[serde(default)]
    pub my_list_status: Option<AnimeListStatus>,
}

/// The list status of an anime. The response key is `num_episodes_watched` (`MALListAnimeItemStatus`).
#[derive(Debug, Deserialize)]
pub struct AnimeListStatus {
    pub is_rewatching: bool,
    pub status: String,
    pub num_episodes_watched: f64,
    pub score: i64,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub finish_date: Option<String>,
}

/// `GET users/@me/{mangalist,animelist}` (`MALUserSearchResult`). A missing `paging` ends the list (E-SEARCH-16).
#[derive(Debug, Deserialize)]
pub struct UserListPage {
    pub data: Vec<UserListItem>,
    #[serde(default)]
    pub paging: Paging,
}

#[derive(Debug, Deserialize)]
pub struct UserListItem {
    pub node: UserListNode,
}

#[derive(Debug, Deserialize)]
pub struct UserListNode {
    pub id: i64,
    pub title: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct Paging {
    #[serde(default)]
    pub next: Option<String>,
}
