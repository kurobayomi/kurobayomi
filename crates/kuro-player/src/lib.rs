//! Player core (areas 45, 46, 48): libmpv2 wrapper, startup option table, event observer,
//! URL and load helpers, key maps and the UI state model. No GTK here. The GL area, windows and
//! MPRIS sit in the GTK and platform layers. Pure modules are tested without libmpv; `mpv` tests
//! need a system libmpv.

pub mod aniskip;
pub mod assets;
pub mod chapters;
pub mod config;
pub mod controls;
pub mod enums;
pub mod errors;
pub mod keys;
pub mod load;
pub mod mpv;
pub mod observer;
pub mod options;
pub mod progress;
pub mod remember;
pub mod seek;
pub mod seekbar_model;
pub mod session;
pub mod shortcuts;
pub mod skip;
pub mod source_url;
pub mod speed;
pub mod strings;
pub mod time;
pub mod torrent;
pub mod track_select;
pub mod tracks;
pub mod types;
