//! The library screen (area 37). The logic is pure and tested here; it sits on kuro-domain's library model, category
//! service and preferences. GTK widgets are in `view`; `mount` puts the view in the window.
//!
//! Tests: the package is a binary, so there is no `--lib` target. Run them without a display with
//! `cargo test -p kurobayomi --bin kurobayomi library::`. The widget checks in `view::options` need a display and skip
//! themselves without one (they run on the shared GTK thread of `crate::testing`). `t2` holds the T2-* tests.
//!
//! Fixtures: the T-3 data sets live with the code that reads them, in `crates/kuro-domain` (its `testdb` and the
//! row builders in `fixtures`), not in `tests/fixtures/library`, because every pure rule they feed is tested there
//! and in `t2` without a JSON round trip.
//!
//! Decision records (Android-only parts dropped or replaced):
//! - T-37a-036: F5 and the header refresh button update the active category, and the toast shows the
//!   outcome (`actions::update_toast`).
//! - T-37a-050: the global update says "Updating library"; a category update says "Updating category".
//! - T-37a-052: dropped: animated vector tab icons and the splash ready flag.
//!   Nothing functional is lost; only motion feedback.
//! - T-37a-053: an empty library with no repository shows the welcome view (`view::welcome`: the steps, the add button,
//!   the storage folder, AniList, restore and the guide). Once a repository exists it offers Browse
//!   (`actions::empty_library_action`).
//! - T-37a-057: the interval-custom row is not drawn on the desktop. The filter still runs when the restriction is on
//!   (`t2::filter`, T2-FILTER-06).
//! - T-37a-058: an entry with only logged-out tracks sorts as -1.0 (E-14). A NaN mean counts as unscored
//!   (`kuro_domain::library::order::tracker_mean`).

/// How long a Library toast stays on the shared toast layer (U-1.8).
pub const TOAST_TIME: std::time::Duration = std::time::Duration::from_secs(4);

pub mod actions;
pub mod cells;
pub mod download_facts;
pub mod frame_cover;
#[cfg(test)]
mod fixtures;
pub mod keys;
pub mod mount;
pub mod category_dialog;
pub mod category_screen;
pub mod delete_dialog;
pub mod layout;
pub mod load;
pub mod query;
pub mod trackers;
#[cfg(test)]
mod t2;
pub mod select;
pub mod view;
pub mod worker;
