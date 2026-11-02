//! GUI logic of area 42 (History, Updates, the tab shell and the storage helpers).
//!
//! Pure modules come first and carry their own tests: `cargo test -p kurobayomi updates`. The widgets (`shell`) stay
//! thin. The domain queries and writes live in `kuro-domain`; these modules decide what the screens do with them.
//! Android items with no desktop port (T-42a-055 to T-42a-061, T-42b-042):
//! - The tablet check and the fast-scroll animation: the 720 px width breakpoint replaces them (`storage`, `upcoming::layout`).
//! - Voyager navigation and animated tab icons: the `gtk::Stack` navigation and static symbolic icons.
//! - Compose previews: not ported; deterministic test fixtures replace their data (T-42a-060).
//! - The splash hand-off: the window shows when the GTK application activates.
//! - `TAB_ANIME`/`TAB_MANGA`, the anime `onInvertSelection` and the search query state: no port. Invert stays in `selection`.
//! - Refresh: the Refresh button and F5 (`mount`). The back gesture: Escape. `animateItem`: dropped.
//!   `FastScrollLazyColumn`: the GTK scrollbar.

#[cfg(test)]
pub mod area42_tests;
pub mod bottom_bar;
pub mod bulk;
pub mod coalesce;
pub mod copy;
pub mod dates;
pub mod download_actions;
pub mod download_exec;
pub mod download_host;
pub mod download_state;
pub mod format;
pub mod feed;
pub mod history;
pub mod history_add;
pub mod history_data;
pub mod history_dialog;
pub mod history_screen;
pub mod history_row;
pub mod library_trigger;
pub mod list;
pub mod mount;
pub mod open_route;
pub mod prefs;
pub mod qualities;
pub mod selection;
pub mod signals;
pub mod state_page;
pub mod store;
pub mod storage;
pub mod storage_data;
pub mod storage_row;
pub mod storage_screen;
pub mod track_binding;
pub mod updates_dialog;
pub mod updates_page;
pub mod updates_screen;
pub mod upcoming;
