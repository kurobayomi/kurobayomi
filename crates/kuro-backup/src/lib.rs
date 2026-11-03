//! Kurobayomi backup: `.tachibk` create and restore, restore planner, dialog model.
//!
//! Modules are added by the backup areas (21 and 22).
//! GTK-free: notices and dialogs are data here, shown by the binary crate.

pub mod auto;
pub mod creator;
pub mod error;
pub mod file;
pub mod flags;
pub mod format;
pub mod job;
pub mod notify;
pub mod options;
pub mod reveal;
pub mod restore;
pub mod schema;
pub mod schema_text;
pub mod screen;
pub mod settings;
pub mod storage_info;
pub mod text;
pub mod validate;
pub mod wire;
