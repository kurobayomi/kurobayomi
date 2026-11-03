//! The local stream server (`HttpServer.kt`, `HttpServerService.kt`; T-16a-033). A player reads a stream from
//! `http://localhost:<port>` and this server answers each request through a [`LocalHandler`]. The server cuts a
//! `Range` request out of the whole resource, so mpv and ffmpeg can seek.
//!
//! ponytail: std threads, one per connection, and a blocking handler. An axum server would add a crate and a
//! runtime for one localhost socket. Upgrade to an async server if a handler needs to await inside a connection.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use bytes::Bytes;
use kuro_source::SourceError;

const ACCEPT_POLL: Duration = Duration::from_millis(5);
const IO_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_HEADER_LINES: usize = 64;

/// What a handler returns for one path. `status` 200 means the whole resource; the server answers a `Range` request for
/// it with 206. A handler that sets 206 itself has its body sent as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

/// Answers the requests of the local server. `path` is the request target with its query, and `range` the raw `Range`
/// header when the request sent one.
pub trait LocalHandler: Send + Sync {
    fn serve(&self, path: &str, range: Option<&str>) -> Result<LocalResponse, SourceError>;
}

/// A running server on `127.0.0.1`. Dropping it stops it.
pub struct LocalStreamServer {
    port: u16,
    stop: Arc<AtomicBool>,
    open: Arc<Mutex<HashMap<u64, TcpStream>>>,
    accept: Option<JoinHandle<()>>,
}

impl LocalStreamServer {
    /// Binds `127.0.0.1` on a free port. A bind failure is `SourceError::LocalServer`.
    pub fn start(handler: Arc<dyn LocalHandler>) -> Result<Self, SourceError> {
        Self::start_on("127.0.0.1:0", handler)
    }

    fn start_on(addr: &str, handler: Arc<dyn LocalHandler>) -> Result<Self, SourceError> {
        let listener = TcpListener::bind(addr).map_err(|err| SourceError::LocalServer(err.to_string()))?;
        let port = listener.local_addr().map_err(|err| SourceError::LocalServer(err.to_string()))?.port();
        if port == 0 {
            return Err(SourceError::LocalServer("the server has no port".to_owned()));
        }
        listener.set_nonblocking(true).map_err(|err| SourceError::LocalServer(err.to_string()))?;
        let stop = Arc::new(AtomicBool::new(false));
        let open = Arc::new(Mutex::new(HashMap::new()));
        let accept = {
            let (stop, open) = (stop.clone(), open.clone());
            thread::spawn(move || accept_loop(&listener, &handler, &stop, &open))
        };
        Ok(Self { port, stop, open, accept: Some(accept) })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The address a player uses: `http://localhost:<port>`.
    pub fn url(&self) -> String {
        format!("http://localhost:{}", self.port)
    }

    /// Stops accepting, closes every open connection and waits for the accept loop. A handler that is still running
    /// finishes on its own thread, so this does not wait for it.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for (_, conn) in self.open.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).drain() {
            let _ = conn.shutdown(Shutdown::Both);
        }
        if let Some(handle) = self.accept.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for LocalStreamServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn accept_loop(listener: &TcpListener, handler: &Arc<dyn LocalHandler>, stop: &AtomicBool, open: &Arc<Mutex<HashMap<u64, TcpStream>>>) {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                if stream.set_nonblocking(false).is_err() || stream.set_read_timeout(Some(IO_TIMEOUT)).is_err() {
                    continue;
                }
                let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
                let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                if let Ok(clone) = stream.try_clone() {
                    open.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(id, clone);
                }
                let (handler, open) = (handler.clone(), open.clone());
                thread::spawn(move || {
                    serve_connection(stream, handler.as_ref());
                    open.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(&id);
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(ACCEPT_POLL),
            Err(err) => {
                tracing::warn!(%err, "local stream server stopped accepting");
                return;
            }
        }
    }
}

fn serve_connection(stream: TcpStream, handler: &dyn LocalHandler) {
    let Ok(writer) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    let Some(request) = read_request(&mut reader) else { return };
    let mut writer = writer;
    let response = match request.method.as_str() {
        "GET" | "HEAD" => answer(handler, &request),
        _ => plain(405, Bytes::new()),
    };
    let head_only = request.method == "HEAD";
    let _ = write_response(&mut writer, &response, head_only);
}

struct Request {
    method: String,
    path: String,
    range: Option<String>,
}

fn read_request(reader: &mut BufReader<TcpStream>) -> Option<Request> {
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let mut range = None;
    for _ in 0..MAX_HEADER_LINES {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 {
            break;
        }
        let header = header.trim_end();
        if header.is_empty() {
            return Some(Request { method, path, range });
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.trim().eq_ignore_ascii_case("range") {
                range = Some(value.trim().to_owned());
            }
        }
    }
    Some(Request { method, path, range })
}

fn answer(handler: &dyn LocalHandler, request: &Request) -> Out {
    match handler.serve(&request.path, request.range.as_deref()) {
        Err(err) => {
            tracing::warn!(%err, path = %request.path, "local stream handler failed");
            plain(500, Bytes::new())
        }
        Ok(found) if found.status == 200 => {
            let cut_out = request.range.as_deref().map(|range| cut(range, &found.body));
            match cut_out {
                Some(Cut::Part(part)) => {
                    Out { status: 206, headers: found.headers, body: part.body, content_range: Some(part.content_range) }
                }
                Some(Cut::Unsatisfiable) => Out {
                    status: 416,
                    headers: Vec::new(),
                    body: Bytes::new(),
                    content_range: Some(format!("bytes */{}", found.body.len())),
                },
                Some(Cut::Ignored) | None => Out { status: 200, headers: found.headers, body: found.body, content_range: None },
            }
        }
        Ok(other) => Out { status: other.status, headers: other.headers, body: other.body, content_range: None },
    }
}

/// One answer to write. `content_range` is the `Content-Range` header when there is one.
struct Out {
    status: u16,
    headers: Vec<(String, String)>,
    body: Bytes,
    content_range: Option<String>,
}

fn plain(status: u16, body: Bytes) -> Out {
    Out { status, headers: Vec::new(), body, content_range: None }
}

struct Part {
    body: Bytes,
    content_range: String,
}

/// The result of reading a `Range` header against a resource.
enum Cut {
    Part(Part),
    /// The range starts past the end, or ends before it starts: 416.
    Unsatisfiable,
    /// A header this server does not read (not `bytes=`, or several ranges). The whole resource is sent, as RFC 9110
    /// allows.
    Ignored,
}

/// Reads `bytes=start-end`, `bytes=start-` or `bytes=-suffix` against `whole`.
fn cut(range: &str, whole: &Bytes) -> Cut {
    let total = whole.len() as u64;
    let Some(spec) = range.strip_prefix("bytes=").map(str::trim) else { return Cut::Ignored };
    let Some((first, last)) = spec.split_once('-') else { return Cut::Ignored };
    let parsed = if first.is_empty() {
        last.parse::<u64>().ok().map(|suffix| (total.saturating_sub(suffix), total.wrapping_sub(1)))
    } else {
        let end = if last.is_empty() { Some(u64::MAX) } else { last.parse::<u64>().ok() };
        first.parse::<u64>().ok().zip(end)
    };
    let Some((start, end)) = parsed else { return Cut::Ignored };
    if total == 0 || start >= total || start > end {
        return Cut::Unsatisfiable;
    }
    let end = end.min(total - 1);
    let body = whole.slice(start as usize..=end as usize);
    Cut::Part(Part { body, content_range: format!("bytes {start}-{end}/{total}") })
}

fn write_response(writer: &mut TcpStream, out: &Out, head_only: bool) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {} {}\r\n", out.status, reason(out.status));
    let has_type = out.headers.iter().any(|(name, _)| name.eq_ignore_ascii_case("content-type"));
    if !has_type {
        head.push_str("Content-Type: application/octet-stream\r\n");
    }
    for (name, value) in &out.headers {
        if name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("Accept-Ranges: bytes\r\n");
    if let Some(range) = &out.content_range {
        head.push_str(&format!("Content-Range: {range}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\nConnection: close\r\n\r\n", out.body.len()));
    writer.write_all(head.as_bytes())?;
    if !head_only {
        writer.write_all(&out.body)?;
    }
    writer.flush()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        206 => "Partial Content",
        404 => "Not Found",
        405 => "Method Not Allowed",
        416 => "Range Not Satisfiable",
        500 => "Internal Server Error",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Instant;

    use super::*;

    /// Serves one resource of `size` bytes, where byte `i` is `i % 251`, for any path that is `/video`.
    struct Resource {
        size: usize,
    }

    impl LocalHandler for Resource {
        fn serve(&self, path: &str, _range: Option<&str>) -> Result<LocalResponse, SourceError> {
            if path != "/video" {
                return Ok(LocalResponse { status: 404, headers: Vec::new(), body: Bytes::from_static(b"no") });
            }
            let body: Vec<u8> = (0..self.size).map(|i| (i % 251) as u8).collect();
            Ok(LocalResponse { status: 200, headers: vec![("Content-Type".into(), "video/mp4".into())], body: body.into() })
        }
    }

    /// Sends one raw request and returns the whole response text (the server closes the connection).
    fn request(port: u16, raw: &str) -> Vec<u8> {
        let mut conn = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        conn.write_all(raw.as_bytes()).expect("write");
        let mut out = Vec::new();
        conn.read_to_end(&mut out).expect("read");
        out
    }

    fn split(response: &[u8]) -> (String, Vec<u8>) {
        let at = response.windows(4).position(|w| w == b"\r\n\r\n").expect("header end");
        (String::from_utf8_lossy(&response[..at]).into_owned(), response[at + 4..].to_vec())
    }

    #[test]
    fn it_binds_a_non_zero_port_and_its_url_is_localhost_with_that_port() {
        let server = LocalStreamServer::start(Arc::new(Resource { size: 10 })).expect("start");
        assert_ne!(server.port(), 0);
        assert_eq!(server.url(), format!("http://localhost:{}", server.port()));
    }

    #[test]
    fn a_range_request_is_answered_with_206_and_the_requested_bytes() {
        let server = LocalStreamServer::start(Arc::new(Resource { size: 1000 })).expect("start");
        let raw = format!("GET /video HTTP/1.1\r\nHost: localhost\r\nRange: bytes=100-199\r\n\r\n");
        let (head, body) = split(&request(server.port(), &raw));
        assert!(head.starts_with("HTTP/1.1 206 "), "{head}");
        assert!(head.contains("Content-Range: bytes 100-199/1000"), "{head}");
        assert!(head.contains("Content-Length: 100"), "{head}");
        let expected: Vec<u8> = (100..200).map(|i| (i % 251) as u8).collect();
        assert_eq!(body, expected);
    }

    #[test]
    fn an_open_ended_and_a_suffix_range_are_cut_from_the_whole_resource() {
        let server = LocalStreamServer::start(Arc::new(Resource { size: 1000 })).expect("start");
        let (head, body) = split(&request(server.port(), "GET /video HTTP/1.1\r\nRange: bytes=990-\r\n\r\n"));
        assert!(head.contains("Content-Range: bytes 990-999/1000"), "{head}");
        assert_eq!(body.len(), 10);
        let (head, body) = split(&request(server.port(), "GET /video HTTP/1.1\r\nRange: bytes=-5\r\n\r\n"));
        assert!(head.contains("Content-Range: bytes 995-999/1000"), "{head}");
        assert_eq!(body.len(), 5);
    }

    #[test]
    fn an_unsatisfiable_range_is_416_with_the_total() {
        let server = LocalStreamServer::start(Arc::new(Resource { size: 100 })).expect("start");
        let (head, _) = split(&request(server.port(), "GET /video HTTP/1.1\r\nRange: bytes=500-600\r\n\r\n"));
        assert!(head.starts_with("HTTP/1.1 416 "), "{head}");
        assert!(head.contains("Content-Range: bytes */100"), "{head}");
    }

    #[test]
    fn a_plain_request_gets_the_whole_resource_and_the_handler_decides_the_missing_paths() {
        let server = LocalStreamServer::start(Arc::new(Resource { size: 4 })).expect("start");
        let (head, body) = split(&request(server.port(), "GET /video HTTP/1.1\r\n\r\n"));
        assert!(head.starts_with("HTTP/1.1 200 "), "{head}");
        assert_eq!(body, vec![0, 1, 2, 3]);
        let (head, _) = split(&request(server.port(), "GET /other HTTP/1.1\r\n\r\n"));
        assert!(head.starts_with("HTTP/1.1 404 "), "{head}");
    }

    #[test]
    fn a_bind_failure_is_a_local_server_error() {
        let taken = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = taken.local_addr().expect("addr").port();
        let err = LocalStreamServer::start_on(&format!("127.0.0.1:{port}"), Arc::new(Resource { size: 1 })).err();
        assert!(matches!(err, Some(SourceError::LocalServer(_))), "{err:?}");
    }

    #[test]
    fn stop_completes_within_two_seconds_while_a_download_is_still_reading() {
        struct Stalled {
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl LocalHandler for Stalled {
            fn serve(&self, _path: &str, _range: Option<&str>) -> Result<LocalResponse, SourceError> {
                // The handler is stuck until the test drops its sender at the end.
                let _ = self.release.lock().unwrap().recv_timeout(Duration::from_secs(10));
                Ok(LocalResponse { status: 200, headers: Vec::new(), body: Bytes::from_static(b"late") })
            }
        }
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let server = LocalStreamServer::start(Arc::new(Stalled { release: Mutex::new(release_rx) })).expect("start");
        let mut conn = TcpStream::connect(("127.0.0.1", server.port())).expect("connect");
        conn.write_all(b"GET /video HTTP/1.1\r\n\r\n").expect("write");
        thread::sleep(Duration::from_millis(50));
        let started = Instant::now();
        server.stop();
        assert!(started.elapsed() < Duration::from_secs(2), "stop took {:?}", started.elapsed());
        drop(release_tx);
    }

    #[test]
    fn a_range_header_this_server_does_not_read_gets_the_whole_resource() {
        let server = LocalStreamServer::start(Arc::new(Resource { size: 8 })).expect("start");
        let (head, body) = split(&request(server.port(), "GET /video HTTP/1.1\r\nRange: bytes=0-1,4-5\r\n\r\n"));
        assert!(head.starts_with("HTTP/1.1 200 "), "{head}");
        assert_eq!(body.len(), 8);
    }

    #[test]
    fn a_handler_error_is_a_500_and_the_server_keeps_answering() {
        struct Broken;
        impl LocalHandler for Broken {
            fn serve(&self, _path: &str, _range: Option<&str>) -> Result<LocalResponse, SourceError> {
                Err(SourceError::Transport("reset".into()))
            }
        }
        let server = LocalStreamServer::start(Arc::new(Broken)).expect("start");
        let (head, _) = split(&request(server.port(), "GET /x HTTP/1.1\r\n\r\n"));
        assert!(head.starts_with("HTTP/1.1 500 "), "{head}");
        let (head, _) = split(&request(server.port(), "GET /y HTTP/1.1\r\n\r\n"));
        assert!(head.starts_with("HTTP/1.1 500 "), "{head}");
    }
}
