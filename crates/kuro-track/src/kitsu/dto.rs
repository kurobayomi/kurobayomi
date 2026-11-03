//! Kitsu JSON:API wire shapes (T-30a-014). Unknown keys are
//! ignored. Ids accept a JSON number or a numeric string.

use serde::{Deserialize, Deserializer};

/// A numeric id sent as a number or a numeric string.
fn de_id<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Number(i64),
        Text(String),
    }
    match Raw::deserialize(d)? {
        Raw::Number(n) => Ok(n),
        Raw::Text(s) => s.parse().map_err(serde::de::Error::custom),
    }
}

/// A string id sent as a string or a number.
fn de_text_id<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Text(String),
        Number(i64),
    }
    Ok(match Raw::deserialize(d)? {
        Raw::Text(s) => s,
        Raw::Number(n) => n.to_string(),
    })
}

/// `GET library-entries` (find and get). `included` is empty when absent.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ListResponse {
    pub data: Vec<LibraryEntry>,
    #[serde(default)]
    pub included: Vec<Included>,
}

/// One library entry (`libraryEntries` resource).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LibraryEntry {
    #[serde(deserialize_with = "de_id")]
    pub id: i64,
    pub attributes: EntryAttributes,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EntryAttributes {
    pub status: String,
    pub progress: i64,
    #[serde(default, rename = "ratingTwenty")]
    pub rating_twenty: Option<i64>,
    #[serde(default, rename = "startedAt")]
    pub started_at: Option<String>,
    #[serde(default, rename = "finishedAt")]
    pub finished_at: Option<String>,
    pub private: bool,
}

/// The media an entry points to, from `included`. `status` is required, so an item without it fails decoding.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Included {
    #[serde(deserialize_with = "de_id")]
    pub id: i64,
    pub attributes: IncludedAttributes,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct IncludedAttributes {
    #[serde(rename = "canonicalTitle")]
    pub canonical_title: String,
    #[serde(default, rename = "chapterCount")]
    pub chapter_count: Option<i64>,
    #[serde(default, rename = "episodeCount")]
    pub episode_count: Option<i64>,
    #[serde(default, rename = "mangaType")]
    pub manga_type: Option<String>,
    #[serde(default, rename = "showType")]
    pub show_type: Option<String>,
    #[serde(default, rename = "posterImage")]
    pub poster_image: Option<PosterImage>,
    #[serde(default)]
    pub synopsis: Option<String>,
    #[serde(default, rename = "startDate")]
    pub start_date: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PosterImage {
    #[serde(default)]
    pub original: Option<String>,
}

/// `POST library-entries` response. Only `data.id` is read.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AddResponse {
    pub data: AddedEntry,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AddedEntry {
    #[serde(deserialize_with = "de_id")]
    pub id: i64,
}

/// `GET users?filter[self]=true` response.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct UserResponse {
    pub data: Vec<User>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct User {
    #[serde(deserialize_with = "de_text_id")]
    pub id: String,
}

/// `GET algolia-keys/media/`. Only the key is read.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AlgoliaKeyResponse {
    pub media: AlgoliaKey,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AlgoliaKey {
    pub key: String,
}

/// The Algolia `query` answer. Hits stay raw, so one bad hit can be skipped on its own (KT-79).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AlgoliaResponse {
    pub hits: Vec<serde_json::Value>,
}

/// One media hit (the `attributesToRetrieve` fields of the search filter). `id` and `canonicalTitle` are required.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AlgoliaHit {
    #[serde(deserialize_with = "de_id")]
    pub id: i64,
    #[serde(rename = "canonicalTitle")]
    pub canonical_title: String,
    #[serde(default, rename = "chapterCount")]
    pub chapter_count: Option<i64>,
    #[serde(default, rename = "episodeCount")]
    pub episode_count: Option<i64>,
    #[serde(default, rename = "posterImage")]
    pub poster_image: Option<PosterImage>,
    #[serde(default)]
    pub synopsis: Option<String>,
    #[serde(default, rename = "averageRating")]
    pub average_rating: Option<f64>,
    /// Epoch seconds in Algolia.
    #[serde(default, rename = "startDate")]
    pub start_date: Option<i64>,
    /// Epoch seconds in Algolia.
    #[serde(default, rename = "endDate")]
    pub end_date: Option<i64>,
    #[serde(default)]
    pub subtype: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"{
        "data": [{
            "id": "77",
            "type": "libraryEntries",
            "attributes": {"status": "current", "progress": 4, "ratingTwenty": 15,
                "startedAt": null, "finishedAt": null, "private": true, "updatedAt": "x"}
        }],
        "included": [{
            "id": 9,
            "type": "manga",
            "attributes": {"canonicalTitle": "T", "chapterCount": null, "status": "finished", "scope": 1}
        }],
        "links": {"first": "x"}
    }"#;

    #[test]
    fn t_30a_014_list_response_decodes_string_and_number_ids() {
        let parsed: ListResponse = serde_json::from_str(LIST).unwrap();
        assert_eq!(parsed.data[0].id, 77);
        assert_eq!(parsed.data[0].attributes.progress, 4);
        assert_eq!(parsed.data[0].attributes.rating_twenty, Some(15));
        assert!(parsed.data[0].attributes.private);
        assert_eq!(parsed.included[0].id, 9);
        assert_eq!(parsed.included[0].attributes.chapter_count, None);
    }

    #[test]
    fn t_30a_014_included_defaults_to_empty_when_absent() {
        let parsed: ListResponse = serde_json::from_str(r#"{"data": []}"#).unwrap();
        assert!(parsed.data.is_empty());
        assert!(parsed.included.is_empty());
    }

    #[test]
    fn t_30a_014_included_item_without_status_fails_decoding() {
        let text =
            r#"{"data": [], "included": [{"id": 1, "attributes": {"canonicalTitle": "T"}}]}"#;
        assert!(serde_json::from_str::<ListResponse>(text).is_err());
    }

    #[test]
    fn t_30a_014_add_and_user_ids_accept_numbers_and_strings() {
        let add: AddResponse = serde_json::from_str(r#"{"data": {"id": 5}, "x": 1}"#).unwrap();
        assert_eq!(add.data.id, 5);
        let add: AddResponse = serde_json::from_str(r#"{"data": {"id": "6"}}"#).unwrap();
        assert_eq!(add.data.id, 6);
        let user: UserResponse = serde_json::from_str(r#"{"data": [{"id": "42"}]}"#).unwrap();
        assert_eq!(user.data[0].id, "42");
        let user: UserResponse = serde_json::from_str(r#"{"data": [{"id": 42}]}"#).unwrap();
        assert_eq!(user.data[0].id, "42");
    }
}
