//! Torrent links of anime episodes (T-24a-023; behaviour 5.5). A link is a torrent when it is a magnet link or a
//! `.torrent` file. Torrent episodes play in the app (`kuro-torrent`) but are not downloaded: such a link fails with
//! `TorrentUnavailable` (E-X04).

use crate::error::DownloadError;

/// `isTorrent`: a magnet link or a `.torrent` file.
pub fn is_torrent(url: &str) -> bool {
    url.starts_with("magnet") || url.ends_with(".torrent")
}

/// Refuses a torrent link, so it is not fetched as a plain URL. A link that is not a torrent passes.
pub fn check_torrent(url: &str) -> Result<(), DownloadError> {
    if is_torrent(url) { Err(DownloadError::TorrentUnavailable) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnet_and_torrent_file_links_are_torrents() {
        assert!(is_torrent("magnet:?xt=urn:btih:abc"));
        assert!(is_torrent("https://h.org/show.torrent"));
        assert!(!is_torrent("https://h.org/video.m3u8"));
    }

    #[test]
    fn a_torrent_link_is_refused() {
        assert!(matches!(check_torrent("magnet:?xt=urn:btih:abc"), Err(DownloadError::TorrentUnavailable)));
        assert!(check_torrent("https://h.org/video.m3u8").is_ok());
    }
}
