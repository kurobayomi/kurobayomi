//! Domain logic for entries and items: flag words, number recognition, sorting, fetch intervals and the
//! read-side facades over `kuro-data` repositories. No GTK, no OS code.
//!
//! The models are defined in `kuro-data`;
//! their behaviour is added here as extension traits, because inherent impls must live in the crate
//! that defines the type.

pub mod app;
pub mod backup;
pub mod category;
pub mod dates;
pub mod entries;
pub mod error;
pub mod extension;
pub mod extension_repo;
pub mod download;
mod dropped;
pub mod flags;
pub mod history;
pub mod items;
pub mod library;
pub mod release;
pub mod source;
pub mod storage;
pub mod track;
pub mod updates;
pub mod upcoming;

/// The Default category: id 0, never listed, never editable, never deleted (`UNCATEGORIZED_ID`).
pub const SYSTEM_CATEGORY_ID: i64 = 0;

/// Rows the updates feed loads at most (`GetMangaUpdates` limit).
pub const UPDATES_LIMIT: i64 = 500;

/// How far back the updates feed looks, in months.
pub const UPDATES_WINDOW_MONTHS: i64 = 3;

/// Quiet time of the library search box before the query runs.
pub const SEARCH_DEBOUNCE_MS: u64 = 250;

#[cfg(test)]
mod testdb;
