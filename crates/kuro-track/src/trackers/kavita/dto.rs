//! Kavita response shapes (`KavitaModels.kt`). Only the fields the tracker reads
//! are declared. Unknown fields are ignored, as Kotlin's `ignoreUnknownKeys` does (T-33a-051).

use serde::Deserialize;

/// `SeriesDto`. `thumbnail_url` is absent from upstream responses (D-01), so a missing key and an explicit `null` both
/// read as `None` and the cover becomes `""` (E-15, E-16).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SeriesDto {
    pub id: i32,
    pub name: String,
    pub pages: i32,
    /// Required: a missing `pagesRead` is a parse error (E-66).
    #[serde(rename = "pagesRead")]
    pub pages_read: i32,
    pub format: i32,
    #[serde(rename = "libraryId")]
    pub library_id: i32,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
}

/// `VolumeDto`. Only the chapter list is read for the total (`KavitaApi.kt:101-124`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct VolumeDto {
    #[serde(default)]
    pub chapters: Vec<ChapterDto>,
}

/// `ChapterDto`. The Kotlin defaults are kept: an absent `number` is `"-1"`, and an explicit `null` is `None`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChapterDto {
    #[serde(default = "default_number")]
    pub number: Option<String>,
}

fn default_number() -> Option<String> {
    Some("-1".to_owned())
}

/// `AuthenticationDto`: the `token` of a successful `Plugin/authenticate` call.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AuthenticationDto {
    pub token: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERIES: &str =
        r#"{"id":12,"name":"Berserk","pages":10,"pagesRead":4,"format":3,"libraryId":1}"#;

    #[test]
    fn t_33a_051_e15_a_series_without_thumbnail_parses_with_no_cover() {
        let s: SeriesDto = serde_json::from_str(SERIES).unwrap();
        assert_eq!(s.thumbnail_url, None);
        assert_eq!((s.pages, s.pages_read), (10, 4));
    }

    #[test]
    fn t_33a_051_e16_a_null_thumbnail_is_none_not_the_word_null() {
        let s: SeriesDto = serde_json::from_str(
            r#"{"id":12,"name":"B","pages":1,"pagesRead":0,"format":3,"libraryId":1,"thumbnail_url":null}"#,
        )
        .unwrap();
        assert_eq!(s.thumbnail_url, None);
    }

    #[test]
    fn t_33a_051_e66_pages_read_is_required() {
        let r = serde_json::from_str::<SeriesDto>(
            r#"{"id":12,"name":"B","pages":1,"format":3,"libraryId":1}"#,
        );
        assert!(r.is_err());
    }

    #[test]
    fn t_33a_051_a_chapter_without_number_is_minus_one_and_null_is_none() {
        let absent: ChapterDto = serde_json::from_str("{}").unwrap();
        assert_eq!(absent.number.as_deref(), Some("-1"));
        let null: ChapterDto = serde_json::from_str(r#"{"number":null}"#).unwrap();
        assert_eq!(null.number, None);
    }

    #[test]
    fn t_33a_051_a_volume_without_chapters_has_an_empty_list() {
        let v: VolumeDto = serde_json::from_str(r#"{"id":1}"#).unwrap();
        assert!(v.chapters.is_empty());
    }

    #[test]
    fn t_33a_051_authentication_needs_a_token() {
        assert!(serde_json::from_str::<AuthenticationDto>("{}").is_err());
        let ok: AuthenticationDto = serde_json::from_str(r#"{"token":"tok"}"#).unwrap();
        assert_eq!(ok.token, "tok");
    }
}
