//! `DownloadError` (thiserror). Each variant has a message key for the UI and an English `Display` for logs.

use kuro_source::SourceError;

use crate::store::StoreError;

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    /// The downloads folder cannot be created or written (FM-01). Holds the root path.
    #[error("Invalid location: {0}")]
    InvalidLocation(String),
    #[error("No pages found")]
    NoPages,
    /// Free space is below the minimum (FM-02).
    #[error("Couldn't download due to low storage space")]
    LowSpace { free: u64, need: u64 },
    /// The folder does not hold one finished file per page.
    #[error("chapter is incomplete: {0}")]
    Incomplete(String),
    /// Some pages failed. Each failed page was already reported on its own.
    #[error("{0} page(s) failed")]
    PagesFailed(usize),
    /// Raw source text is shown as is, as Kotlin does.
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
    /// A chapter task panicked, or the downloader is gone.
    #[error("internal error: {0}")]
    Internal(String),
    /// The hoster list is empty or holds no playable video (anime, `video_list_empty_error`).
    #[error("No video found")]
    NoVideo,
    /// The operation was cancelled by the user. Never shown to the user.
    #[error("Cancelled")]
    Cancelled,
    /// `ffmpeg` or `ffprobe` is not on `PATH` (design 7).
    #[error("ffmpeg is not installed or not on PATH")]
    FfmpegMissing,
    /// ffmpeg exited with a failure status (behaviour 5.7 step 6).
    #[error("Error in ffmpeg!")]
    FfmpegFailed,
    /// ffprobe failed. Holds the first line of its output, as the Kotlin exception does.
    #[error("{0}")]
    Ffprobe(String),
    /// The local HTTP server for `localhost:1` URLs did not start, or gave port 0 (behaviour 5.3, E-X07).
    #[error("Failed to start server")]
    HttpServer(String),
    /// `StartNow` for an episode already in the downloads folder (E-Q06). Nothing is queued.
    #[error("Episode is already downloaded")]
    AlreadyDownloaded,
    /// The finished folder name is taken by a non-empty folder at finalise (E-F02). The staging folder is kept.
    #[error("Episode already exists")]
    EpisodeExists,
    /// The staging folder does not hold exactly one finished video (E-F06, behaviour 5.8).
    #[error("Download incomplete: {0}")]
    DownloadIncomplete(String),
    /// A folder move would cross filesystems. Holds the target path. No copy fallback (T-24a-071, E-F16).
    #[error("Cannot move the download to another disk: {0}")]
    CrossDevice(String),
    /// A torrent link: torrent episodes play in the app but are not downloaded (E-X04).
    #[error("Torrent episodes can be watched but not downloaded")]
    TorrentUnavailable,
}

/// How a chapter failure is handled by the scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The transport failed. The chapter goes back to the queue when the network returns.
    Network,
    /// Stored in the queue as ERROR until the user retries.
    Other,
}

impl DownloadError {
    /// Message key for the UI (`p1-behavior.md` section 15). `None` when the text is raw source or I/O text.
    pub fn key(&self) -> Option<&'static str> {
        match self {
            Self::InvalidLocation(_) => Some("invalid_location"),
            Self::NoPages => Some("page_list_empty_error"),
            Self::LowSpace { .. } => Some("download_insufficient_space"),
            Self::NoVideo => Some("video_list_empty_error"),
            _ => None,
        }
    }

    pub fn failure_kind(&self) -> FailureKind {
        match self {
            Self::Source(SourceError::Transport(_)) => FailureKind::Network,
            _ => FailureKind::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_cover_the_mapped_messages() {
        assert_eq!(DownloadError::NoPages.key(), Some("page_list_empty_error"));
        assert_eq!(DownloadError::LowSpace { free: 1, need: 2 }.key(), Some("download_insufficient_space"));
        assert_eq!(DownloadError::InvalidLocation("/x".into()).key(), Some("invalid_location"));
        assert_eq!(DownloadError::Source(SourceError::Http(500)).key(), None);
    }

    #[test]
    fn only_transport_failures_count_as_network() {
        assert_eq!(
            DownloadError::Source(SourceError::Transport("reset".into())).failure_kind(),
            FailureKind::Network
        );
        assert_eq!(DownloadError::Source(SourceError::Http(404)).failure_kind(), FailureKind::Other);
        assert_eq!(DownloadError::NoPages.failure_kind(), FailureKind::Other);
    }

    #[test]
    fn ffmpeg_and_server_failures_use_the_kotlin_text() {
        assert_eq!(DownloadError::FfmpegFailed.to_string(), "Error in ffmpeg!");
        assert_eq!(DownloadError::HttpServer("port 0".into()).to_string(), "Failed to start server");
        assert_eq!(DownloadError::Ffprobe("Invalid data".into()).failure_kind(), FailureKind::Other);
    }

    #[test]
    fn episode_failures_have_their_english_text() {
        assert_eq!(DownloadError::AlreadyDownloaded.to_string(), "Episode is already downloaded");
        assert_eq!(DownloadError::EpisodeExists.to_string(), "Episode already exists");
        assert_eq!(DownloadError::DownloadIncomplete("2 files".into()).to_string(), "Download incomplete: 2 files");
    }

    #[test]
    fn display_uses_the_english_text() {
        assert_eq!(DownloadError::NoPages.to_string(), "No pages found");
        assert_eq!(DownloadError::Source(SourceError::Http(404)).to_string(), "HTTP error 404");
    }
}

/// The message of an error and of each error it wraps, joined with `: ` (T-23a-041: failures are logged with the full
/// chain, not only the top message).
pub fn error_chain(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(next) = source {
        text.push_str(": ");
        text.push_str(&next.to_string());
        source = next.source();
    }
    text
}

#[cfg(test)]
mod chain_tests {
    use super::*;

    #[test]
    fn the_chain_lists_every_source() {
        let inner = std::io::Error::other("disk full");
        let outer = DownloadError::from(inner);
        assert!(error_chain(&outer).contains("disk full"));
        assert_eq!(error_chain(&std::io::Error::other("alone")), "alone");
    }
}

