//! Tracking on the entry screens (plan areas 38b and 39b, tasks T-38a-031 to T-38b-043 and T-39b-038 to T-39b-047): pure
//! rules and state for the track sheet, the pickers, the detached writes, the search screen, the auto-track prompt and
//! tracker removal. The tracker logic is `kuro_track`, and the rules here take plain values read off it (`facts`), so
//! they are tested without a live tracker. The widgets read these values and draw them.
//!
//! Reused, not copied: the tracker status and score rules (`kuro_track::remote`, `kuro_track::picker`), the mark-read
//! policy (`kuro_track::policy`), the row actions and link copy (`kuro_track::sheet`), the remove dialog text and the
//! registration (`kuro_track::remove`, `kuro_track::bind`), and the tracking button text (`entry_core::action_row`).

pub mod actions;
pub mod auto_track;
pub mod community_page;
pub mod dates;
pub mod facts;
pub mod page;
pub mod pickers;
pub mod progress;
pub mod remove;
pub mod session;
pub mod summary;
pub mod writes;
