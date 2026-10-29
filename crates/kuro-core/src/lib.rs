//! Kurobayomi core: UI-free, platform-independent application logic. No GTK, no glib.
//!
//! # Android dependency mapping (`core/common` and `core/archive` Gradle files)
//!
//! | Gradle dependency | Linux replacement |
//! |---|---|
//! | jsoup | `scraper` (html5ever), used by the EPUB reader |
//! | libarchive | `libarchive3-sys` behind one safe wrapper, feature `archive` (spike T-13a-031) |
//! | unifile | dropped: `std::fs` and `Path` replace `UniFile` and SAF documents |
//! | logcat | `tracing`; see [`logging::priority_to_level`] |
//! | rxjava | dropped: sources are `async` traits, watchers are `tokio::sync::watch` |
//! | okhttp (core, logging, brotli, dnsoverhttps) | `reqwest` in `kuro-net` (area 12), not in this crate |
//! | okio | `std::io`, `Vec<u8>` and `bytes::Bytes` |
//! | coil image decoder | `image` 0.25 and `jxl-oxide`; HEIF behind feature `heif`, see [`image`] |
//! | natural comparator | [`text::natural_cmp`] (port of the java-nat-sort comparator, no crate) |
//! | quickjs | `rquickjs`, optional feature `js-sources` (P2) in the source layer, not here |
//! | ffmpeg-kit | dropped: source ffmpeg arguments are not supported, `mpv` in `kuro-player` plays media |
//! | TorrServer | dropped: `librqbit` inside the app, in `kuro-torrent` |
//!
//! # Cargo features
//!
//! `archive` (libarchive wrapper), `heif` (AVIF and HEIF decoding) and `updater`. All are off by default.

pub mod app_info;
pub mod archive;
pub mod crash;
pub mod crash_dump;
pub mod image;
pub mod logging;
pub mod migrate;
pub mod prefs;
pub mod runtime;
pub mod sched;
pub mod serde_conv;
pub mod sources;
pub mod storage;
pub mod torrent;
pub mod util;
pub mod utc;
pub mod xml;

pub use util::{hash, json, text};

#[cfg(test)]
mod build_support;
