//! Row and patch structs of the unified tables. A row holds the STORED layout: the same fields as the columns,
//! no translation and no old-name aliases (`item_flags` is the anime superset layout for both kinds). A patch
//! field of `None` keeps the column (`coalesce`), so a column cannot be cleared to NULL through a patch.

use kuro_platform::kind::Kind;
use rusqlite::{Result, Row};

use super::kind_col;
use crate::codec::{GenreList, UpdateStrategy};
use crate::codec::{FetchType, Memo};

/// Every `entries` column. The anime-only ones are `None` for manga (the table's two `CHECK`s).
#[derive(Clone, Debug, PartialEq)]
pub struct EntryRow {
    pub id: i64,
    pub kind: Kind,
    pub source: i64,
    pub url: String,
    pub title: String,
    pub artist: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub genre: Option<GenreList>,
    pub status: i64,
    pub thumbnail_url: Option<String>,
    pub favorite: bool,
    pub last_update: Option<i64>,
    pub next_update: Option<i64>,
    pub initialized: bool,
    pub viewer: i64,
    pub item_flags: i64,
    pub cover_last_modified: i64,
    pub date_added: i64,
    pub update_strategy: UpdateStrategy,
    pub calculate_interval: i64,
    pub last_modified_at: i64,
    pub favorite_modified_at: Option<i64>,
    pub version: i64,
    pub fetch_type: Option<FetchType>,
    pub parent_id: Option<i64>,
    pub season_flags: Option<i64>,
    pub season_number: Option<f64>,
    pub season_source_order: Option<i64>,
    pub background_url: Option<String>,
    pub background_last_modified: Option<i64>,
    pub memo: Option<Memo>,
}

impl EntryRow {
    /// Reads by column name, so a view that selects the entry columns plus aggregates works too.
    pub fn from_row(r: &Row<'_>) -> Result<Self> {
        Ok(Self {
            id: r.get("_id")?,
            kind: kind_col(r, "kind")?,
            source: r.get("source")?,
            url: r.get("url")?,
            title: r.get("title")?,
            artist: r.get("artist")?,
            author: r.get("author")?,
            description: r.get("description")?,
            genre: r.get("genre")?,
            status: r.get("status")?,
            thumbnail_url: r.get("thumbnail_url")?,
            favorite: r.get("favorite")?,
            last_update: r.get("last_update")?,
            next_update: r.get("next_update")?,
            initialized: r.get("initialized")?,
            viewer: r.get("viewer")?,
            item_flags: r.get("item_flags")?,
            cover_last_modified: r.get("cover_last_modified")?,
            date_added: r.get("date_added")?,
            update_strategy: r.get("update_strategy")?,
            calculate_interval: r.get("calculate_interval")?,
            last_modified_at: r.get("last_modified_at")?,
            favorite_modified_at: r.get("favorite_modified_at")?,
            version: r.get("version")?,
            fetch_type: r.get("fetch_type")?,
            parent_id: r.get("parent_id")?,
            season_flags: r.get("season_flags")?,
            season_number: r.get("season_number")?,
            season_source_order: r.get("season_source_order")?,
            background_url: r.get("background_url")?,
            background_last_modified: r.get("background_last_modified")?,
            memo: r.get("memo")?,
        })
    }
}

/// Coalesce update of one entry. Setting an anime-only field on a manga row fails the table's `CHECK`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntryPatch {
    pub id: i64,
    pub source: Option<i64>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub genre: Option<GenreList>,
    pub status: Option<i64>,
    pub thumbnail_url: Option<String>,
    pub favorite: Option<bool>,
    pub last_update: Option<i64>,
    pub next_update: Option<i64>,
    pub initialized: Option<bool>,
    pub viewer: Option<i64>,
    pub item_flags: Option<i64>,
    pub cover_last_modified: Option<i64>,
    pub date_added: Option<i64>,
    pub update_strategy: Option<UpdateStrategy>,
    pub calculate_interval: Option<i64>,
    pub version: Option<i64>,
    pub fetch_type: Option<FetchType>,
    pub parent_id: Option<i64>,
    pub season_flags: Option<i64>,
    pub season_number: Option<f64>,
    pub season_source_order: Option<i64>,
    pub background_url: Option<String>,
    pub background_last_modified: Option<i64>,
    pub memo: Option<Memo>,
}

/// Every `items` column plus the `kind` of the owning entry (joined, never stored on the item).
#[derive(Clone, Debug, PartialEq)]
pub struct ItemRow {
    pub id: i64,
    pub entry_id: i64,
    pub kind: Kind,
    pub url: String,
    pub name: String,
    pub scanlator: Option<String>,
    pub done: bool,
    pub bookmark: bool,
    pub fillermark: bool,
    pub progress: i64,
    pub number: f64,
    pub source_order: i64,
    pub date_fetch: i64,
    pub date_upload: i64,
    pub last_modified_at: i64,
    pub version: i64,
    pub total_seconds: Option<i64>,
    pub summary: Option<String>,
    pub preview_url: Option<String>,
    pub memo: Option<Memo>,
    /// How many pages a chapter has, as the reader last saw it; `None` for an episode and a chapter not read since
    /// the column came (version 3).
    pub page_count: Option<i64>,
}

impl ItemRow {
    pub fn from_row(r: &Row<'_>) -> Result<Self> {
        Ok(Self {
            id: r.get("_id")?,
            entry_id: r.get("entry_id")?,
            kind: kind_col(r, "kind")?,
            url: r.get("url")?,
            name: r.get("name")?,
            scanlator: r.get("scanlator")?,
            done: r.get("done")?,
            bookmark: r.get("bookmark")?,
            fillermark: r.get("fillermark")?,
            progress: r.get("progress")?,
            number: r.get("number")?,
            source_order: r.get("source_order")?,
            date_fetch: r.get("date_fetch")?,
            date_upload: r.get("date_upload")?,
            last_modified_at: r.get("last_modified_at")?,
            version: r.get("version")?,
            total_seconds: r.get("total_seconds")?,
            summary: r.get("summary")?,
            preview_url: r.get("preview_url")?,
            memo: r.get("memo")?,
            page_count: r.get("page_count")?,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemPatch {
    pub id: i64,
    pub entry_id: Option<i64>,
    pub url: Option<String>,
    pub name: Option<String>,
    pub scanlator: Option<String>,
    pub done: Option<bool>,
    pub bookmark: Option<bool>,
    pub fillermark: Option<bool>,
    pub progress: Option<i64>,
    pub number: Option<f64>,
    pub source_order: Option<i64>,
    pub date_fetch: Option<i64>,
    pub date_upload: Option<i64>,
    pub version: Option<i64>,
    pub total_seconds: Option<i64>,
    pub summary: Option<String>,
    pub preview_url: Option<String>,
    pub memo: Option<Memo>,
    pub page_count: Option<i64>,
}

/// A `history` row. `last_at` 0 means reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryRow {
    pub id: i64,
    pub item_id: i64,
    pub last_at: i64,
    pub duration_ms: i64,
}

impl HistoryRow {
    pub fn from_row(r: &Row<'_>) -> Result<Self> {
        Ok(Self { id: r.get("_id")?, item_id: r.get("item_id")?, last_at: r.get("last_at")?, duration_ms: r.get("duration_ms")? })
    }
}

/// A `historyView` row (the entry's newest history row).
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryViewRow {
    pub id: i64,
    pub kind: Kind,
    pub entry_id: i64,
    pub item_id: i64,
    pub title: String,
    pub thumbnail_url: Option<String>,
    pub source: i64,
    pub favorite: bool,
    pub cover_last_modified: i64,
    pub item_number: f64,
    pub last_at: i64,
    pub duration_ms: i64,
}

impl HistoryViewRow {
    pub fn from_row(r: &Row<'_>) -> Result<Self> {
        Ok(Self {
            id: r.get("id")?,
            kind: kind_col(r, "kind")?,
            entry_id: r.get("entryId")?,
            item_id: r.get("itemId")?,
            title: r.get("title")?,
            thumbnail_url: r.get("thumbnailUrl")?,
            source: r.get("source")?,
            favorite: r.get("favorite")?,
            cover_last_modified: r.get("cover_last_modified")?,
            item_number: r.get("itemNumber")?,
            last_at: r.get("lastAt")?,
            duration_ms: r.get("durationMs")?,
        })
    }
}

/// A `tracks` row plus the `kind` of its entry (joined).
#[derive(Clone, Debug, PartialEq)]
pub struct TrackRow {
    pub id: i64,
    pub entry_id: i64,
    pub kind: Kind,
    pub tracker_id: i64,
    pub remote_id: i64,
    pub library_id: Option<i64>,
    pub title: String,
    pub last_item: f64,
    pub total_items: i64,
    pub status: i64,
    pub score: f64,
    pub remote_url: String,
    pub start_date: i64,
    pub finish_date: i64,
    pub private: bool,
}

impl TrackRow {
    pub fn from_row(r: &Row<'_>) -> Result<Self> {
        Ok(Self {
            id: r.get("_id")?,
            entry_id: r.get("entry_id")?,
            kind: kind_col(r, "kind")?,
            tracker_id: r.get("tracker_id")?,
            remote_id: r.get("remote_id")?,
            library_id: r.get("library_id")?,
            title: r.get("title")?,
            last_item: r.get("last_item")?,
            total_items: r.get("total_items")?,
            status: r.get("status")?,
            score: r.get("score")?,
            remote_url: r.get("remote_url")?,
            start_date: r.get("start_date")?,
            finish_date: r.get("finish_date")?,
            private: r.get("private")?,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackPatch {
    pub id: i64,
    pub entry_id: Option<i64>,
    pub tracker_id: Option<i64>,
    pub remote_id: Option<i64>,
    pub library_id: Option<i64>,
    pub title: Option<String>,
    pub last_item: Option<f64>,
    pub total_items: Option<i64>,
    pub status: Option<i64>,
    pub score: Option<f64>,
    pub remote_url: Option<String>,
    pub start_date: Option<i64>,
    pub finish_date: Option<i64>,
    pub private: Option<bool>,
}

/// A `categories` row. `order` is the `sort` column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategoryRow {
    pub kind: Kind,
    pub id: i64,
    pub name: String,
    pub order: i64,
    pub flags: i64,
    pub hidden: bool,
}

impl CategoryRow {
    pub fn from_row(r: &Row<'_>) -> Result<Self> {
        Ok(Self {
            kind: kind_col(r, "kind")?,
            id: r.get("id")?,
            name: r.get("name")?,
            order: r.get("order")?,
            flags: r.get("flags")?,
            hidden: r.get("hidden")?,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CategoryPatch {
    pub id: i64,
    pub name: Option<String>,
    pub order: Option<i64>,
    pub flags: Option<i64>,
    pub hidden: Option<bool>,
}

/// A `stub_sources` row (its kind is the query's).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StubSourceRow {
    pub id: i64,
    pub lang: String,
    pub name: String,
}

/// An aggregate column as the old mappers read it: possibly REAL, truncated to an integer, NULL is 0.
pub(crate) fn count_col(r: &Row<'_>, name: &str) -> Result<i64> {
    Ok(r.get::<_, Option<f64>>(name)?.unwrap_or(0.0) as i64)
}

/// One `libraryView` row: the entry, its item aggregates and one category it is in (0 when none).
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryRow {
    pub entry: EntryRow,
    pub category: i64,
    pub total: i64,
    pub done: i64,
    pub bookmarks: i64,
    pub fillermarks: i64,
    pub latest_upload: i64,
    pub item_fetched_at: i64,
    pub last_activity: i64,
}

impl LibraryRow {
    pub fn from_row(r: &Row<'_>) -> Result<Self> {
        Ok(Self {
            entry: EntryRow::from_row(r)?,
            category: r.get("category")?,
            total: r.get("totalCount")?,
            done: count_col(r, "doneCount")?,
            bookmarks: count_col(r, "bookmarkCount")?,
            fillermarks: count_col(r, "fillermarkCount")?,
            latest_upload: r.get("latestUpload")?,
            item_fetched_at: r.get("itemFetchedAt")?,
            last_activity: r.get("lastActivity")?,
        })
    }
}

/// One season (child entry) with its item aggregates.
#[derive(Clone, Debug, PartialEq)]
pub struct SeasonRow {
    pub entry: EntryRow,
    pub total: i64,
    pub done: i64,
    pub bookmarks: i64,
    pub fillermarks: i64,
    pub latest_upload: i64,
    pub fetched_at: i64,
    pub last_activity: i64,
}

/// A non-favourite top-level entry: a candidate of the clear screen. `fetch_type` is Episodes for manga.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeletableRow {
    pub entry_id: i64,
    pub source_id: i64,
    pub kind: Kind,
    pub fetch_type: FetchType,
}

/// An `updatesView` row.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateRow {
    pub kind: Kind,
    pub entry_id: i64,
    pub entry_title: String,
    pub item_id: i64,
    pub item_name: String,
    pub scanlator: Option<String>,
    pub done: bool,
    pub bookmark: bool,
    pub fillermark: bool,
    pub progress: i64,
    pub total_seconds: Option<i64>,
    pub source: i64,
    pub favorite: bool,
    pub thumbnail_url: Option<String>,
    pub cover_last_modified: i64,
    pub date_upload: i64,
    pub date_fetch: i64,
}

impl UpdateRow {
    pub fn from_row(r: &Row<'_>) -> Result<Self> {
        Ok(Self {
            kind: kind_col(r, "kind")?,
            entry_id: r.get("entryId")?,
            entry_title: r.get("entryTitle")?,
            item_id: r.get("itemId")?,
            item_name: r.get("itemName")?,
            scanlator: r.get("scanlator")?,
            done: r.get("done")?,
            bookmark: r.get("bookmark")?,
            fillermark: r.get("fillermark")?,
            progress: r.get("progress")?,
            total_seconds: r.get("totalSeconds")?,
            source: r.get("source")?,
            favorite: r.get("favorite")?,
            thumbnail_url: r.get("thumbnailUrl")?,
            cover_last_modified: r.get("coverLastModified")?,
            date_upload: r.get("dateUpload")?,
            date_fetch: r.get("datefetch")?,
        })
    }
}

/// A `community_scores` row. `remote_id` and `score` are `None` for an entry that was looked up and not found;
/// `score` alone is `None` for a title the site has no score for yet. `matched_by` is `tracker`, `title` or `user`.
#[derive(Clone, Debug, PartialEq)]
pub struct CommunityScoreRow {
    pub entry_id: i64,
    pub provider: String,
    pub remote_id: Option<i64>,
    pub matched_title: String,
    pub matched_by: String,
    pub score: Option<f64>,
    pub fetched_at: i64,
}

impl CommunityScoreRow {
    pub fn from_row(r: &Row<'_>) -> Result<Self> {
        Ok(Self {
            entry_id: r.get("entry_id")?,
            provider: r.get("provider")?,
            remote_id: r.get("remote_id")?,
            matched_title: r.get("matched_title")?,
            matched_by: r.get("matched_by")?,
            score: r.get("score")?,
            fetched_at: r.get("fetched_at")?,
        })
    }
}

/// An entry with the `community_scores` row it has (left join): what the refresh job reads.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreCandidateRow {
    pub entry_id: i64,
    pub kind: Kind,
    pub title: String,
    pub status: i64,
    pub score: Option<CommunityScoreRow>,
}

impl ScoreCandidateRow {
    pub fn from_row(r: &Row<'_>) -> Result<Self> {
        // every column of the joined row is NULL when the entry has none
        let joined: Option<i64> = r.get("entry_id")?;
        Ok(Self {
            entry_id: r.get("e_id")?,
            kind: kind_col(r, "kind")?,
            title: r.get("title")?,
            status: r.get("status")?,
            score: joined.map(|_| CommunityScoreRow::from_row(r)).transpose()?,
        })
    }
}
