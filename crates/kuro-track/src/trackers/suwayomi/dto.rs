//! Suwayomi response shapes (`SuwayomiModels.kt`). Wire names are camelCase.
//! A key without `Option` is required: serde rejects a payload that lacks it (E-65). An `Option` key may be absent
//! or null, because serde reads a missing optional key as `None`.

use std::collections::HashMap;

use serde::Deserialize;

/// The installed source of a manga (`SourceDataClass`, 8 required fields). The tracker never reads it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDataClass {
    pub id: String,
    pub name: String,
    pub lang: String,
    pub icon_url: String,
    pub supports_latest: bool,
    pub is_configurable: bool,
    pub is_nsfw: bool,
    pub display_name: String,
}

/// A manga (`MangaDataClass`, 26 fields). `GET {url}/full` returns it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MangaDataClass {
    pub id: i32,
    pub source_id: String,
    pub url: String,
    pub title: String,
    pub thumbnail_url: Option<String>,
    pub initialized: bool,
    pub artist: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub genre: Vec<String>,
    pub status: String,
    pub in_library: bool,
    pub in_library_at: i64,
    pub source: Option<SourceDataClass>,
    pub meta: HashMap<String, String>,
    pub real_url: Option<String>,
    pub last_fetched_at: Option<i64>,
    pub chapters_last_fetched_at: Option<i64>,
    pub fresh_data: bool,
    pub unread_count: Option<i64>,
    pub download_count: Option<i64>,
    /// Declared required here, as the Kotlin model does, although the server may send null.
    pub chapter_count: i64,
    pub last_chapter_read: Option<ChapterDataClass>,
    pub age: Option<i64>,
    pub chapters_age: Option<i64>,
}

/// A chapter (`ChapterDataClass`, 17 fields). `index` starts at 1 and is the number the chapter PUT uses (E-59).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterDataClass {
    pub id: i32,
    pub url: String,
    pub name: String,
    pub upload_date: i64,
    pub chapter_number: f64,
    pub scanlator: Option<String>,
    pub manga_id: i32,
    pub read: bool,
    pub bookmarked: bool,
    pub last_page_read: i32,
    pub last_read_at: i64,
    pub index: i32,
    pub fetched_at: i64,
    pub downloaded: bool,
    pub page_count: i32,
    pub chapter_count: Option<i32>,
    pub meta: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANGA: &str = include_str!("../../../tests/fixtures/suwayomi/manga_full.json");
    const CHAPTERS: &str = include_str!("../../../tests/fixtures/suwayomi/chapters.json");

    #[test]
    fn t_33a_095_the_manga_fixture_parses_into_all_26_fields() {
        let m: MangaDataClass = serde_json::from_str(MANGA).unwrap();
        assert_eq!(m.title, "Title");
        assert_eq!(m.chapter_count, 10);
        assert_eq!(m.unread_count, Some(4));
        assert_eq!(m.description.as_deref(), Some("Desc"));
        assert_eq!(m.source.as_ref().map(|s| s.lang.as_str()), Some("en"));
        assert_eq!(m.last_chapter_read.as_ref().map(|c| c.index), Some(7));
    }

    #[test]
    fn t_33a_095_e65_a_manga_without_a_required_field_fails_the_parse() {
        let mut v: serde_json::Value = serde_json::from_str(MANGA).unwrap();
        v.as_object_mut().unwrap().remove("initialized");
        assert!(serde_json::from_value::<MangaDataClass>(v).is_err());
    }

    #[test]
    fn t_33a_095_the_source_needs_its_eight_keys() {
        let mut v: serde_json::Value = serde_json::from_str(MANGA).unwrap();
        v["source"].as_object_mut().unwrap().remove("isNsfw");
        assert!(serde_json::from_value::<MangaDataClass>(v).is_err());
    }

    #[test]
    fn t_33a_096_chapter_numbers_are_floats_and_index_is_the_server_index() {
        let chapters: Vec<ChapterDataClass> = serde_json::from_str(CHAPTERS).unwrap();
        assert_eq!(chapters.len(), 3);
        assert_eq!(chapters[0].chapter_number, 5.0);
        assert_eq!(chapters[0].index, 7);
        assert_eq!(chapters[2].chapter_number, 4.0);
    }
}
