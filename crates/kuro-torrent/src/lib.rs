//! Torrent playback: an episode a source gives as a magnet link plays while it downloads. The engine is librqbit
//! (Apache-2.0), running inside the app on its tokio runtime; a small local server on 127.0.0.1 hands the chosen file to
//! the player with range requests, the pieces it asks for downloaded first.
//!
//! A source marks a video as a torrent the way Aniyomi's extensions do: its address is a `magnet:` link, and a
//! non-standard `index=N` parameter names the file inside the torrent. Without it the largest video file plays.
//!
//! One torrent plays at a time. Opening another, or closing the player, removes the previous one and its files; the
//! cache folder is emptied when the engine starts. Nothing is seeded after playback and there are no settings.
//!
//! Source traffic rule: the engine makes its own tracker and peer connections, outside the app's internet-only client.
//! Trackers on this computer or a private network are dropped from the magnet before it is added, and local peer
//! discovery is off. Peers learned from the DHT or from trackers are not filtered.
//!
//! Incoming peers: the engine listens on a port and asks the router to forward it (UPnP) while the app runs, as
//! common torrent clients do by default. Without it only peers with an open port could be reached, and most home peers
//! have none, so the first pieces took over a minute.

use std::collections::HashSet;
use std::io::SeekFrom;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use librqbit::dht::DhtPersistenceConfig;
use librqbit::{AddTorrent, AddTorrentOptions, DhtSessionConfig, ListenerOptions, ManagedTorrent, Session, SessionOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::OnceCell;

/// How long a magnet may take to find peers and its file list before playback gives up.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(90);

/// File extensions played as video, as Aniyomi's torrent extensions list them.
const VIDEO_EXTENSIONS: &[&str] = &["mkv", "mp4", "webm", "avi", "mov", "m4v", "ts", "m2ts", "wmv", "flv", "mpg", "mpeg", "ogv"];

/// Why a torrent did not open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No peers or no file list within [`OPEN_TIMEOUT`].
    NoPeers,
    /// The link is not a usable magnet.
    BadLink(String),
    /// The torrent holds no video file (or the `index` is not one of its files).
    NoVideo,
    /// The engine or its local server could not start, or the torrent failed.
    Engine(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoPeers => write!(f, "no peers answered within {} s", OPEN_TIMEOUT.as_secs()),
            Error::BadLink(e) => write!(f, "not a usable magnet link: {e}"),
            Error::NoVideo => write!(f, "the torrent holds no video file"),
            Error::Engine(e) => write!(f, "torrent engine: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// Whether `url` is a torrent the engine plays: a magnet link.
pub fn is_torrent(url: &str) -> bool {
    url.get(..7).is_some_and(|scheme| scheme.eq_ignore_ascii_case("magnet:"))
}

/// A magnet as the engine adds it: the link without the `index` parameter and without trackers on this computer or a
/// private network, and the file index it named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Magnet {
    pub link: String,
    pub index: Option<usize>,
}

/// Splits `index=N` off a magnet link and drops trackers that are not on the internet (by literal address, or
/// `localhost`; tracker host names are not resolved).
pub fn parse_magnet(url: &str) -> Result<Magnet, Error> {
    let (scheme, query) = url.split_once('?').ok_or_else(|| Error::BadLink("no parameters".into()))?;
    if !is_torrent(scheme) {
        return Err(Error::BadLink("not a magnet link".into()));
    }
    let mut index = None;
    let mut kept = Vec::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "index" => index = value.parse::<usize>().ok(),
            "tr" if !tracker_is_public(&percent_decode(value)) => {}
            _ => kept.push(pair),
        }
    }
    if !kept.iter().any(|p| p.starts_with("xt=urn:btih:")) {
        return Err(Error::BadLink("no info hash".into()));
    }
    Ok(Magnet { link: format!("magnet:?{}", kept.join("&")), index })
}

fn tracker_is_public(tracker: &str) -> bool {
    let Ok(url) = url::Url::parse(tracker) else { return false };
    match url.host() {
        Some(url::Host::Ipv4(ip)) => kuro_net::public_only::is_public(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => kuro_net::public_only::is_public(IpAddr::V6(ip)),
        Some(url::Host::Domain(name)) => !(name.eq_ignore_ascii_case("localhost") || name.to_ascii_lowercase().ends_with(".localhost")),
        None => false,
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (b'+', _) => {
                out.push(b' ');
                i += 1;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The file to play: `index` when it names a video file, else the largest video file. `files` is (path, length) in the
/// torrent's order.
pub fn choose_file(files: &[(String, u64)], index: Option<usize>) -> Option<usize> {
    let is_video = |path: &str| path.rsplit_once('.').is_some_and(|(_, ext)| VIDEO_EXTENSIONS.iter().any(|v| v.eq_ignore_ascii_case(ext)));
    match index {
        Some(i) => files.get(i).filter(|(path, _)| is_video(path)).map(|_| i),
        None => files.iter().enumerate().filter(|(_, (path, _))| is_video(path)).max_by_key(|(_, (_, len))| *len).map(|(i, _)| i),
    }
}

/// The torrent being played.
struct Playing {
    id: usize,
    handle: Arc<ManagedTorrent>,
    file: usize,
    /// In the local address, so a stale address from an earlier torrent never serves the new one.
    token: u64,
    name: String,
}

struct Engine {
    session: Arc<Session>,
    port: u16,
    playing: Mutex<Option<Playing>>,
    next_token: Mutex<u64>,
}

static ENGINE: OnceCell<Result<Arc<Engine>, String>> = OnceCell::const_new();

async fn engine(cache_dir: &Path) -> Result<Arc<Engine>, Error> {
    ENGINE.get_or_init(|| start(cache_dir.to_owned())).await.clone().map_err(Error::Engine)
}

async fn start(cache_dir: PathBuf) -> Result<Arc<Engine>, String> {
    // the files of earlier runs are never needed again: playback is not resumed from them
    let _ = tokio::fs::remove_dir_all(&cache_dir).await;
    let files_dir = cache_dir.join("files");
    tokio::fs::create_dir_all(&files_dir).await.map_err(|e| format!("cache folder: {e}"))?;
    let options = SessionOptions {
        dht: Some(DhtSessionConfig {
            persistence: Some(DhtPersistenceConfig { config_filename: Some(cache_dir.join("dht.json")), ..Default::default() }),
            ..Default::default()
        }),
        persistence: None,
        disable_local_service_discovery: true,
        // incoming peers on a port the router forwards (see the module docs)
        listen: Some(ListenerOptions { enable_upnp_port_forwarding: true, ..Default::default() }),
        ..Default::default()
    };
    let session = Session::new_with_opts(files_dir, options).await.map_err(|e| format!("{e:#}"))?;
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| format!("local server: {e}"))?;
    let port = listener.local_addr().map_err(|e| format!("local server: {e}"))?.port();
    let engine = Arc::new(Engine { session, port, playing: Mutex::default(), next_token: Mutex::new(0) });
    let serving = Arc::clone(&engine);
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((conn, _)) => {
                    let engine = Arc::clone(&serving);
                    tokio::spawn(async move {
                        if let Err(e) = engine.serve(conn).await {
                            tracing::debug!(error = %e, "torrent stream connection ended");
                        }
                    });
                }
                Err(e) => tracing::warn!(error = %e, "torrent stream server: accept failed"),
            }
        }
    });
    tracing::info!(port, "torrent engine started");
    Ok(engine)
}

/// Adds the magnet `url` and returns the local address that plays its file. The previous torrent is removed first.
/// `cache_dir` is where torrents download (`Paths::torrent_cache_dir`).
pub async fn open(url: &str, cache_dir: &Path) -> Result<String, Error> {
    let magnet = parse_magnet(url)?;
    let engine = engine(cache_dir).await?;
    engine.close().await;
    let options = AddTorrentOptions { only_files: magnet.index.map(|i| vec![i]), overwrite: true, ..Default::default() };
    let added = tokio::time::timeout(OPEN_TIMEOUT, engine.session.add_torrent(AddTorrent::from_url(magnet.link.as_str()), Some(options)))
        .await
        .map_err(|_| Error::NoPeers)?
        .map_err(|e| Error::Engine(format!("{e:#}")))?;
    let handle = added.into_handle().ok_or_else(|| Error::Engine("the torrent was not added".into()))?;
    // a file streams only once the torrent has checked what it already holds and gone live
    tokio::time::timeout(OPEN_TIMEOUT, handle.wait_until_initialized())
        .await
        .map_err(|_| Error::NoPeers)?
        .map_err(|e| Error::Engine(format!("{e:#}")))?;
    let files: Vec<(String, u64)> = handle
        .with_metadata(|meta| meta.file_infos.iter().map(|f| (f.relative_filename.to_string_lossy().into_owned(), f.len)).collect())
        .map_err(|e| Error::Engine(format!("{e:#}")))?;
    let id = handle.id();
    let Some(file) = choose_file(&files, magnet.index) else {
        let _ = engine.session.delete(id.into(), true).await;
        return Err(Error::NoVideo);
    };
    if magnet.index.is_none() {
        // only the file that plays is downloaded
        if let Err(e) = engine.session.update_only_files(&handle, &HashSet::from([file])).await {
            tracing::warn!(error = %e, "torrent: could not limit the download to the played file");
        }
    }
    let token = {
        let mut next = engine.next_token.lock().unwrap_or_else(PoisonError::into_inner);
        *next += 1;
        *next
    };
    let name = files[file].0.rsplit(['/', '\\']).next().unwrap_or("video").to_owned();
    let address = format!("http://127.0.0.1:{}/{token}/{}", engine.port, url_safe(&name));
    *engine.playing.lock().unwrap_or_else(PoisonError::into_inner) = Some(Playing { id, handle, file, token, name });
    Ok(address)
}

/// Removes the playing torrent and its files, when there is one. Called when the player closes.
pub async fn close() {
    if let Some(Ok(engine)) = ENGINE.get() {
        engine.close().await;
    }
}

impl Engine {
    async fn close(&self) {
        let playing = self.playing.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(playing) = playing {
            if let Err(e) = self.session.delete(playing.id.into(), true).await {
                tracing::warn!(error = %e, "torrent: could not remove the played torrent");
            }
        }
    }

    /// One request of the player: `GET` or `HEAD /<token>/<name>`, with or without `Range: bytes=N-[M]`.
    async fn serve(&self, mut conn: TcpStream) -> std::io::Result<()> {
        let head = read_head(&mut conn).await?;
        let mut lines = head.lines();
        let request = lines.next().unwrap_or_default();
        let mut parts = request.split_whitespace();
        let (method, path) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
        let range = lines.find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("range")).map(|(_, v)| v.trim().to_owned()));
        let token = path.trim_start_matches('/').split('/').next().and_then(|t| t.parse::<u64>().ok());
        let found = {
            let playing = self.playing.lock().unwrap_or_else(PoisonError::into_inner);
            playing.as_ref().filter(|p| Some(p.token) == token).map(|p| (Arc::clone(&p.handle), p.file, p.name.clone()))
        };
        let Some((handle, file, name)) = found else {
            return conn.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        };
        let mut stream = handle.stream(file).await.map_err(|e| std::io::Error::other(format!("{e:#}")))?;
        let len = stream.len();
        let Some((start, end)) = byte_range(range.as_deref(), len) else {
            let answer = format!("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{len}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            return conn.write_all(answer.as_bytes()).await;
        };
        let partial = range.is_some();
        let mut answer = format!(
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nConnection: close\r\n",
            if partial { "206 Partial Content" } else { "200 OK" },
            mime_of(&name),
            end + 1 - start,
        );
        if partial {
            answer.push_str(&format!("Content-Range: bytes {start}-{end}/{len}\r\n"));
        }
        answer.push_str("\r\n");
        conn.write_all(answer.as_bytes()).await?;
        if method.eq_ignore_ascii_case("HEAD") || len == 0 {
            return Ok(());
        }
        stream.seek(SeekFrom::Start(start)).await?;
        tokio::io::copy(&mut stream.take(end + 1 - start), &mut conn).await?;
        conn.flush().await
    }
}

/// The request head, up to the blank line (at most 16 KiB).
async fn read_head(conn: &mut TcpStream) -> std::io::Result<String> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && head.len() < 16 * 1024 {
        if conn.read(&mut byte).await? == 0 {
            break;
        }
        head.push(byte[0]);
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

/// The inclusive byte range `(start, end)` a `Range` header asks of a file of `len` bytes; the whole file without one.
/// `None` when it cannot be served. Only `bytes=N-` and `bytes=N-M` are read (what mpv sends); others serve the whole
/// file.
pub fn byte_range(range: Option<&str>, len: u64) -> Option<(u64, u64)> {
    let whole = (len > 0).then(|| (0, len - 1));
    let Some(spec) = range.and_then(|r| r.trim().strip_prefix("bytes=")) else { return whole.or(Some((0, 0))) };
    let Some((from, to)) = spec.split_once('-').filter(|_| !spec.contains(',')) else { return whole };
    let Ok(start) = from.trim().parse::<u64>() else { return whole };
    let end = if to.trim().is_empty() { len.checked_sub(1)? } else { to.trim().parse::<u64>().ok()?.min(len.checked_sub(1)?) };
    (start <= end).then_some((start, end))
}

fn mime_of(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).as_deref() {
        Some("mkv") => "video/x-matroska",
        Some("mp4" | "m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("avi") => "video/x-msvideo",
        Some("ts" | "m2ts") => "video/mp2t",
        _ => "application/octet-stream",
    }
}

/// The file name for the local address: letters, digits and `.-_` kept, everything else `_`, so the address needs no
/// escaping (mpv picks the demuxer partly by its extension).
fn url_safe(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnets_are_torrents_and_lose_their_index_and_private_trackers() {
        assert!(is_torrent("magnet:?xt=urn:btih:abc") && is_torrent("MAGNET:?xt=urn:btih:abc"));
        assert!(!is_torrent("https://example.org/a.mp4") && !is_torrent("magnet"));
        let link = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Show&tr=udp%3A%2F%2Ftracker.example.org%3A1337&tr=http%3A%2F%2F192.168.1.5%3A80%2Fannounce&tr=http://localhost/a&index=3";
        let magnet = parse_magnet(link).unwrap();
        assert_eq!(magnet.index, Some(3));
        assert_eq!(magnet.link, "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Show&tr=udp%3A%2F%2Ftracker.example.org%3A1337");
        assert_eq!(parse_magnet("magnet:?xt=urn:btih:abc").unwrap().index, None);
        assert!(matches!(parse_magnet("magnet:?dn=no-hash"), Err(Error::BadLink(_))));
        assert!(matches!(parse_magnet("https://example.org/?xt=urn:btih:abc"), Err(Error::BadLink(_))));
    }

    #[test]
    fn the_named_video_file_plays_else_the_largest_video() {
        let files = vec![("Show/Show 01.mkv".to_owned(), 700), ("Show/Show 02.mkv".to_owned(), 900), ("Show/cover.jpg".to_owned(), 5_000), ("Show/Show.nfo".to_owned(), 1)];
        assert_eq!(choose_file(&files, Some(0)), Some(0));
        assert_eq!(choose_file(&files, None), Some(1));
        assert_eq!(choose_file(&files, Some(2)), None, "an index on a picture is not played");
        assert_eq!(choose_file(&files, Some(9)), None);
        assert_eq!(choose_file(&[("a.txt".to_owned(), 1)], None), None);
    }

    #[test]
    fn ranges_are_the_ones_mpv_sends() {
        assert_eq!(byte_range(None, 100), Some((0, 99)));
        assert_eq!(byte_range(Some("bytes=10-"), 100), Some((10, 99)));
        assert_eq!(byte_range(Some("bytes=10-19"), 100), Some((10, 19)));
        assert_eq!(byte_range(Some("bytes=90-500"), 100), Some((90, 99)));
        assert_eq!(byte_range(Some("bytes=100-"), 100), None, "past the end");
        assert_eq!(byte_range(Some("bytes=-10"), 100), Some((0, 99)), "a suffix range serves the whole file");
        assert_eq!(byte_range(Some("bytes=0-1,5-6"), 100), Some((0, 99)));
    }

    #[test]
    fn local_names_need_no_escaping() {
        assert_eq!(url_safe("[Sub] Show - 01 (1080p).mkv"), "_Sub__Show_-_01__1080p_.mkv");
        assert_eq!(mime_of("a.MKV"), "video/x-matroska");
        assert_eq!(mime_of("a.bin"), "application/octet-stream");
        assert_eq!(percent_decode("udp%3A%2F%2Fa%3A1"), "udp://a:1");
    }
}
