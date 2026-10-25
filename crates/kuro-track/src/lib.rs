//! Kurobayomi tracking core: tracker trait, OAuth/PKCE, remote tracker clients and retry queue.
//!
//! Layout follows `plans/27-tracking-core/p1-rust-design.md` section 1. No GTK, `gio` or `keyring` here.

pub mod auto_bind;
pub mod bangumi;
pub mod base;
pub mod bind;
pub mod connectivity;
pub mod ctx;
#[cfg(test)]
mod dropped;
pub mod detach;
pub mod dialog;
pub mod entry_kind;
pub mod error;
pub mod events;
pub mod home;
pub mod item;
pub mod keyed_lock;
pub mod kitsu;
pub mod logo;
pub mod login_rows;
pub mod mal;
pub mod manager;
pub mod mangaupdates;
pub mod messages;
pub mod model;
pub mod nav;
pub mod oauth;
pub mod picker;
pub mod policy;
pub mod prefs;
pub mod progress;
pub mod queue;
pub mod record;
pub mod remote;
pub mod remove;
pub mod repo;
pub mod search_screen;
pub mod sheet;
pub mod shikimori;
pub mod simkl;
pub mod sync;
pub mod toast;
pub mod tracker;
pub mod trackers;
pub mod write;

/// The User-Agent of the Shikimori and Bangumi calls: `Kurobayomi/<version> (<os>) (<app id>)` (T-31a-040). The app id
/// names the project, since no repository URL is decided yet (see the DECISIONS of the tracker lane).
pub fn user_agent() -> String {
    format!(
        "Kurobayomi/{} ({}) (io.github.anas1412.Kurobayomi)",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS
    )
}

#[cfg(test)]
mod fakes;
