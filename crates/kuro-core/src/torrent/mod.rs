//! Torrent helpers: the strict bencode parser and writer and `.torrent` metadata extraction. Playback itself is
//! in `kuro-torrent`.

pub mod bencode;
pub mod meta;

/// Errors of the bencode parser and the `.torrent` metadata reader.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TorrentError {
    #[error("invalid torrent: {0}")]
    InvalidTorrent(String),
    #[error("bencode: {0}")]
    Bencode(String),
}
