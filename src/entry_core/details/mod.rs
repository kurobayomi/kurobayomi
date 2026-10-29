//! Anime entry details (plan areas 39a and 39b). Pure rules and state: `airing`, `entry_state`, `episode_row`, `format`,
//! `header`, `info`, `layout`, `model` (the load sequence), `remote` (the source refresh), `runner` (the store
//! runner) and `store`, plus `dispatch` (what the bar and the menu send), `downloads` (the Download menu rules)
//! and `download_backend` (the downloads folder and queue). The thin widgets are `season_grid` and
//! `route`; they read those values. The list options (episodes and seasons) are the shared `entry_core::list_options`. The episode list and its row menu are
//! the shared `entry_core::list_view` and `entry_core::list_model::row_menu`.
//!
//! Reused from the domain layer, not copied here: episode and season flag masks and codecs
//! (`kuro_domain::entries::flags`), the flag setters (`kuro_domain::entries::flag_setters`), the episode sorter
//! (`kuro_domain::items::sorter`), the filter engine (`kuro_domain::app::items`), the missing-episode gaps
//! (`kuro_domain::items::missing`), the anime preference keys (`kuro_prefs`), and the shared manga text rules
//! (`entry_core::text`) for title, author, artist and description.

pub mod airing;
pub mod entry_state;
pub mod episode_row;
pub mod format;
pub mod header;
pub mod info;
pub mod layout;
pub mod intro_dialog;
#[cfg(test)]
mod acceptance;
pub mod actions;
pub mod bottom;
pub mod dialog_state;
pub mod dispatch;
pub mod download_backend;
pub mod downloads;
pub mod refresh;
pub mod remote;
pub mod model;
pub mod route;
pub mod runner;
pub mod related_pack;
pub mod related_view;
pub mod details_error;
pub mod summary;
pub mod set_default;
pub mod season_grid;
pub mod season_fetch;
pub mod season_pipeline;
pub mod season_tile;
pub mod seen;
pub mod store;
pub mod viewer;
