//! Komga response shapes and request bodies (`KomgaModels.kt`). Only the fields the
//! tracker reads are declared, and unknown fields are ignored. Wire names are camelCase, as Komga sends them.

use serde::{Deserialize, Serialize};

/// A series (`SeriesDto`, `KomgaModels.kt:6-20`). Keys without `?` are required (serde rejects a missing one); a `?`
/// key may be absent or null. Wire names are camelCase.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesDto {
    pub id: String,
    pub library_id: String,
    pub name: String,
    pub created: Option<String>,
    pub last_modified: Option<String>,
    pub file_last_modified: String,
    pub books_count: i32,
    pub books_read_count: i32,
    pub books_unread_count: i32,
    pub books_in_progress_count: i32,
    pub metadata: SeriesMetadataDto,
    pub books_metadata: BookMetadataAggregationDto,
}

/// Series metadata (`SeriesMetadataDto`, `KomgaModels.kt:22-43`). The tracker reads `status`, `title` and `summary`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesMetadataDto {
    pub status: String,
    pub created: Option<String>,
    pub last_modified: Option<String>,
    pub title: String,
    pub title_sort: String,
    pub summary: String,
    pub summary_lock: bool,
    pub reading_direction: String,
    pub reading_direction_lock: bool,
    pub publisher: String,
    pub publisher_lock: bool,
    pub age_rating: Option<i32>,
    pub age_rating_lock: bool,
    pub language: String,
    pub language_lock: bool,
    pub genres: Vec<String>,
    pub genres_lock: bool,
    pub tags: Vec<String>,
    pub tags_lock: bool,
}

/// Book metadata of a series (`BookMetadataAggregationDto`, `KomgaModels.kt:45-54`). `authors` may be absent.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookMetadataAggregationDto {
    #[serde(default)]
    pub authors: Vec<AuthorDto>,
    pub release_date: Option<String>,
    pub summary: String,
    pub summary_number: String,
    pub created: String,
    pub last_modified: String,
}

/// An author of a book (`AuthorDto`, `KomgaModels.kt:56-60`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AuthorDto {
    pub name: String,
    pub role: String,
}

/// A readlist (`ReadListDto`, `KomgaModels.kt:72-80`). The tracker reads only `name` (`ReadListDto.toTrack`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadListDto {
    pub id: String,
    pub name: String,
    pub book_ids: Vec<String>,
    pub created_date: String,
    pub last_modified_date: String,
    pub filtered: bool,
}

/// Progress of a readlist (v1 shape). Converted to the v2 shape with [`ReadProgressDto::to_v2`].
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadProgressDto {
    pub books_count: i32,
    pub books_read_count: i32,
    pub books_unread_count: i32,
    pub books_in_progress_count: i32,
    pub last_read_continuous_index: i32,
}

impl ReadProgressDto {
    /// `ReadProgressDto.toV2()`: the book count is also the max number, and the index becomes the continuous number.
    pub fn to_v2(&self) -> ReadProgressV2Dto {
        ReadProgressV2Dto {
            books_count: self.books_count,
            books_read_count: self.books_read_count,
            books_unread_count: self.books_unread_count,
            books_in_progress_count: self.books_in_progress_count,
            last_read_continuous_number_sort: f64::from(self.last_read_continuous_index),
            max_number_sort: self.books_count as f32,
        }
    }
}

/// Progress of a series or a readlist (v2 shape). `maxNumberSort` is required (E-51).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadProgressV2Dto {
    pub books_count: i32,
    pub books_read_count: i32,
    pub books_unread_count: i32,
    pub books_in_progress_count: i32,
    pub last_read_continuous_number_sort: f64,
    pub max_number_sort: f32,
}

/// Body of a readlist progress `PUT`: a count of books (`lastBookRead`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadProgressUpdateDto {
    pub last_book_read: i32,
}

/// Body of a series progress `PUT`: a book number (`lastBookNumberSortRead`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadProgressUpdateV2Dto {
    pub last_book_number_sort_read: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERIES_FULL: &str = include_str!("../../../tests/fixtures/komga/series_full.json");
    const READLIST_FULL: &str = include_str!("../../../tests/fixtures/komga/readlist.json");

    #[test]
    fn t_33a_070_series_fixture_parses_with_the_metadata_the_tracker_reads() {
        let s: SeriesDto = serde_json::from_str(SERIES_FULL).unwrap();
        assert_eq!(s.metadata.title, "Title");
        assert_eq!(s.metadata.status, "ENDED");
        assert_eq!(s.metadata.summary, "Sum");
        assert_eq!(s.metadata.age_rating, None);
        assert_eq!(s.books_metadata.authors[0].role, "writer");
    }

    #[test]
    fn t_33a_070_e51_a_series_without_a_required_key_fails() {
        let mut v: serde_json::Value = serde_json::from_str(SERIES_FULL).unwrap();
        v.as_object_mut().unwrap().remove("fileLastModified");
        assert!(serde_json::from_value::<SeriesDto>(v).is_err());
    }

    #[test]
    fn t_33a_070_a_readlist_fixture_parses_and_gives_its_name() {
        let r: ReadListDto = serde_json::from_str(READLIST_FULL).unwrap();
        assert_eq!(r.name, "Reading list");
        assert_eq!(r.book_ids.len(), 2);
    }

    #[test]
    fn t_33a_071_e51_the_v2_progress_needs_max_number_sort() {
        let r = serde_json::from_str::<ReadProgressV2Dto>(
            r#"{"booksCount":1,"booksReadCount":0,"booksUnreadCount":1,"booksInProgressCount":0,"lastReadContinuousNumberSort":0.0}"#,
        );
        assert!(r.is_err());
    }

    #[test]
    fn t_33a_071_the_v1_progress_converts_to_v2() {
        let v1: ReadProgressDto = serde_json::from_str(
            r#"{"booksCount":5,"booksReadCount":2,"booksUnreadCount":3,"booksInProgressCount":0,"lastReadContinuousIndex":2}"#,
        )
        .unwrap();
        let v2 = v1.to_v2();
        assert_eq!(v2.max_number_sort, 5.0);
        assert_eq!(v2.last_read_continuous_number_sort, 2.0);
    }

    #[test]
    fn t_33a_072_update_bodies_use_the_wire_names() {
        let v2 = serde_json::to_string(&ReadProgressUpdateV2Dto {
            last_book_number_sort_read: 12.0,
        })
        .unwrap();
        assert_eq!(v2, r#"{"lastBookNumberSortRead":12.0}"#);
        let v1 = serde_json::to_string(&ReadProgressUpdateDto { last_book_read: 2 }).unwrap();
        assert_eq!(v1, r#"{"lastBookRead":2}"#);
    }
}
