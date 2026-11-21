//! Storage domain (area 09 section 1, T-09a-047 to T-09a-051): the base directory preference, the folder tree under
//! it, and the change notification. This module is the one place that names the folders inside the storage folder;
//! everything else asks [`StorageManager`] for them.
//!
//! The base directory is a local path. Android content URIs and remote locations are refused with
//! `StorageUnavailable`; the directory picker returns local paths only.

pub mod cache;
mod manager;

use kuro_platform::kind::Kind;

pub use manager::{STORAGE_DIR_KEY, StorageManager};

/// Folder of the backups, under the base.
pub const BACKUPS_PATH: &str = "Backups";
/// Folder of downloaded anime episodes.
pub const ANIME_DOWNLOADS_PATH: &str = "Anime";
/// Folder of downloaded manga chapters.
pub const MANGA_DOWNLOADS_PATH: &str = "Manga";
/// Folder of the local source folders; it holds one folder per kind, named like the downloads folders.
pub const LOCAL_PATH: &str = "Local";
/// Folder of the player configuration.
pub const MPV_CONFIG_PATH: &str = "mpv-config";
/// Player fonts, under `mpv-config`.
pub const FONTS_PATH: &str = "fonts";
/// Player scripts, under `mpv-config`.
pub const SCRIPTS_PATH: &str = "scripts";
/// Player script options, under `mpv-config`.
pub const SCRIPT_OPTS_PATH: &str = "script-opts";
/// Player shaders, under `mpv-config`.
pub const SHADERS_PATH: &str = "shaders";

/// The folders `ensure_tree` and `set_base` create under the base, as path segments (TC-STO-09).
pub const TREE: &[&[&str]] = &[
    &[ANIME_DOWNLOADS_PATH],
    &[MANGA_DOWNLOADS_PATH],
    &[BACKUPS_PATH],
    &[LOCAL_PATH, ANIME_DOWNLOADS_PATH],
    &[LOCAL_PATH, MANGA_DOWNLOADS_PATH],
    &[MPV_CONFIG_PATH],
    &[MPV_CONFIG_PATH, FONTS_PATH],
    &[MPV_CONFIG_PATH, SCRIPTS_PATH],
    &[MPV_CONFIG_PATH, SCRIPT_OPTS_PATH],
    &[MPV_CONFIG_PATH, SHADERS_PATH],
];

/// The folder of one kind (its downloads folder, and its folder under `Local`).
pub fn kind_folder(kind: Kind) -> &'static str {
    match kind {
        Kind::Anime => ANIME_DOWNLOADS_PATH,
        Kind::Manga => MANGA_DOWNLOADS_PATH,
    }
}
