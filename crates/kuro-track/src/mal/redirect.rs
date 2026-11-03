//! Redirect strategy (T-29a-039). A loopback listener on `127.0.0.1` takes the
//! code from the browser redirect. The paste field takes the redirect URL or the bare code when the port cannot be
//! bound, or when the redirect never arrives. Nothing here opens a browser; the caller does.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

use super::error::MalError;

/// Port registered with MAL as `http://localhost:<port>/myanimelist-auth` (proposal).
pub const DEFAULT_PORT: u16 = 8765;
pub const REDIRECT_PATH: &str = "/myanimelist-auth";
/// How long the listener waits for the redirect (proposal).
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
/// Shortest bare code the paste field accepts. MAL's code length is not documented; `hello` must be refused.
pub const MIN_CODE_LEN: usize = 16;
/// How long one connection may take to send its request line (ceiling on a stalled connection).
const CONNECTION_READ: Duration = Duration::from_secs(5);
/// Bytes read from one request. The request line and headers of a redirect are far shorter.
const MAX_REQUEST: usize = 8192;

/// The result of waiting for the redirect or of parsing pasted text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeOutcome {
    /// The authorisation code.
    Code(String),
    /// Consent was refused, or the redirect carried no code (E-LOGIN-1, E-LOGIN-10).
    Denied,
    /// No redirect arrived within [`LOGIN_TIMEOUT`] (E-LOGIN-5).
    TimedOut,
}

/// The loopback port: the configured `redirect_port`, else [`DEFAULT_PORT`]. It must match the URI registered with MAL.
pub fn port_from_config(configured: Option<u16>) -> u16 {
    configured.unwrap_or(DEFAULT_PORT)
}

/// The redirect URI to register with MAL.
pub fn redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}{REDIRECT_PATH}")
}

/// Binds `127.0.0.1:<port>`. A failure means the paste field is the only way to finish the login (E-LOGIN-6).
pub async fn bind_loopback(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port)).await
}

/// Waits for one request to [`REDIRECT_PATH`] and answers it with a page that says the tab can be closed. Other
/// paths get a 404 and the wait goes on. The listener is dropped on return.
pub async fn wait_for_code(listener: TcpListener, timeout: Duration) -> CodeOutcome {
    match tokio::time::timeout(timeout, accept_redirect(&listener)).await {
        Ok(outcome) => outcome,
        Err(_) => CodeOutcome::TimedOut,
    }
}

async fn accept_redirect(listener: &TcpListener) -> CodeOutcome {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            // Accept can fail while the process runs out of descriptors. Pause instead of spinning.
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        // A connection that sends nothing (a browser preconnect) must not hold up the next one.
        let read = tokio::time::timeout(CONNECTION_READ, read_request_target(&mut stream)).await;
        let Ok(Some(target)) = read else {
            respond(&mut stream, "400 Bad Request", "Bad request.").await;
            continue;
        };
        let Some(outcome) = outcome_of_target(&target) else {
            respond(&mut stream, "404 Not Found", "Not found.").await;
            continue;
        };
        let page = match outcome {
            CodeOutcome::Code(_) => "You can close this tab. Kurobayomi is finishing the login.",
            _ => "Login was not completed. You can close this tab.",
        };
        respond(&mut stream, "200 OK", page).await;
        return outcome;
    }
}

/// The request target (`/path?query`) of the request line, or `None` when no request line arrives.
async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while buffer.len() < MAX_REQUEST {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(2).any(|w| w == b"\r\n") {
            break;
        }
    }
    let line_end = buffer.windows(2).position(|w| w == b"\r\n")?;
    let line = std::str::from_utf8(&buffer[..line_end]).ok()?;
    let mut parts = line.split(' ');
    let method = parts.next()?;
    let target = parts.next()?;
    (method == "GET" && target.starts_with('/')).then(|| target.to_owned())
}

/// The outcome of a request target. `None` for any path other than [`REDIRECT_PATH`].
fn outcome_of_target(target: &str) -> Option<CodeOutcome> {
    let url = Url::parse("http://127.0.0.1").ok()?.join(target).ok()?;
    if url.path() != REDIRECT_PATH {
        return None;
    }
    Some(outcome_of_query(&url))
}

/// A `code` value wins. Anything else on the redirect (an `error`, or nothing) is a denial.
fn outcome_of_query(url: &Url) -> CodeOutcome {
    url.query_pairs()
        .find(|(key, value)| key == "code" && !value.is_empty())
        .map_or(CodeOutcome::Denied, |(_, value)| {
            CodeOutcome::Code(value.into_owned())
        })
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let text = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Kurobayomi</title></head>\
         <body><p>{body}</p></body></html>"
    );
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{text}",
        text.len()
    );
    // The browser may already be gone. Nothing depends on the reply reaching it.
    let _ = stream.write_all(reply.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Reads what the user pasted: a full redirect URL, a relative redirect path, or the bare code (E-LOGIN-9). A URL
/// with `error` is a denial (E-LOGIN-10). A URL with neither a code nor an error is refused.
pub fn parse_pasted(input: &str) -> Result<CodeOutcome, MalError> {
    let text = input.trim();
    if text.is_empty() {
        return Err(MalError::PasteInvalid);
    }
    if text.contains("://") || text.contains('?') || text.starts_with('/') {
        let base = Url::parse("http://127.0.0.1").map_err(|_| MalError::PasteInvalid)?;
        let url = Url::parse(text)
            .or_else(|_| base.join(text))
            .map_err(|_| MalError::PasteInvalid)?;
        let has_code = url
            .query_pairs()
            .any(|(key, value)| key == "code" && !value.is_empty());
        let has_error = url.query_pairs().any(|(key, _)| key == "error");
        return match (has_code, has_error) {
            (true, _) => Ok(outcome_of_query(&url)),
            (false, true) => Ok(CodeOutcome::Denied),
            (false, false) => Err(MalError::PasteInvalid),
        };
    }
    let is_code = text.len() >= MIN_CODE_LEN
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'));
    if is_code {
        Ok(CodeOutcome::Code(text.to_owned()))
    } else {
        Err(MalError::PasteInvalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pasted_url() {
        let out = parse_pasted("http://localhost:8765/myanimelist-auth?code=abcDEF1234567890xyz")
            .expect("a redirect URL with a code");
        assert_eq!(out, CodeOutcome::Code("abcDEF1234567890xyz".into()));
    }

    #[test]
    fn parse_pasted_bare() {
        let out = parse_pasted("  abcDEF1234567890xyz_-.~ ").expect("a bare code");
        assert_eq!(out, CodeOutcome::Code("abcDEF1234567890xyz_-.~".into()));
    }

    #[test]
    fn parse_pasted_bad() {
        for input in [
            "hello",
            "",
            "   ",
            "https://example.com/",
            "https://example.com/?x=1",
            "a b c d e f g h i j k",
        ] {
            assert!(
                matches!(parse_pasted(input), Err(MalError::PasteInvalid)),
                "{input:?}"
            );
        }
    }

    #[test]
    fn parse_pasted_denied_redirect_is_denial() {
        let out = parse_pasted("http://localhost:8765/myanimelist-auth?error=access_denied")
            .expect("a redirect with an error is a denial, not bad input");
        assert_eq!(out, CodeOutcome::Denied);
    }

    #[test]
    fn redirect_targets_are_matched_by_path_and_query() {
        assert_eq!(
            outcome_of_target("/myanimelist-auth?code=XYZ&state=1"),
            Some(CodeOutcome::Code("XYZ".into()))
        );
        assert_eq!(
            outcome_of_target("/myanimelist-auth?error=access_denied"),
            Some(CodeOutcome::Denied)
        );
        assert_eq!(outcome_of_target("/favicon.ico"), None);
    }

    #[test]
    fn t_29a_039_port_defaults_to_8765() {
        assert_eq!(port_from_config(None), 8765);
        assert_eq!(port_from_config(Some(9001)), 9001);
    }

    #[test]
    fn redirect_uri_uses_the_port_and_path() {
        assert_eq!(
            redirect_uri(DEFAULT_PORT),
            "http://localhost:8765/myanimelist-auth"
        );
    }

    #[tokio::test]
    async fn loopback_takes_the_code_and_answers_the_browser() {
        let listener = bind_loopback(0).await.expect("ephemeral port binds");
        let port = listener.local_addr().expect("address").port();
        let waiting = tokio::spawn(wait_for_code(listener, Duration::from_secs(5)));

        let mut client = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connects");
        client
            .write_all(b"GET /favicon.ico HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .expect("writes");
        let mut sink = Vec::new();
        client.read_to_end(&mut sink).await.expect("reads 404");
        assert!(String::from_utf8_lossy(&sink).starts_with("HTTP/1.1 404"));

        let mut client = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connects");
        client
            .write_all(b"GET /myanimelist-auth?code=CODE123456789012 HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .expect("writes");
        let mut page = Vec::new();
        client.read_to_end(&mut page).await.expect("reads page");
        assert!(String::from_utf8_lossy(&page).contains("You can close this tab"));

        assert_eq!(
            waiting.await.expect("task"),
            CodeOutcome::Code("CODE123456789012".into())
        );
    }

    #[tokio::test]
    async fn loopback_times_out_without_a_redirect() {
        let listener = bind_loopback(0).await.expect("ephemeral port binds");
        let outcome = wait_for_code(listener, Duration::from_millis(50)).await;
        assert_eq!(outcome, CodeOutcome::TimedOut);
    }
}
