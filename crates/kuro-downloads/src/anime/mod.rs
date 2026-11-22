//! Anime download engine (area 24) on top of the shared queue pieces (area 25): `DownloadState`, the per-source
//! scheduler, the retry policy and the reader-facing error type. Folder names follow `AnimeDownloadProvider`.

pub mod downloader;
pub mod ffmpeg;
pub mod lifecycle;
pub mod local_server;
pub mod model;
pub mod offline;
pub mod progress;
pub mod resolve;
pub mod runner;
#[cfg(test)]
mod test_scripts;
pub mod torrent;
