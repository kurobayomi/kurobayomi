//! The gate of the hidden web pages (`kuro_platform::hidden_pages`): a small proxy on this computer that every page's
//! connections go through. A page runs a site's own script, and anyone may write the plug-in that names the site, so the
//! pages keep the rule of the app's client ([`public_only`](crate::public_only)): the internet only, never this computer
//! or the local network. The web engine cannot be told that, but it can be told to use a proxy. The gate looks each name
//! up itself, keeps the public addresses and connects to one of those, so a name that points at 192.168.1.1 reaches
//! nothing, whatever the page was told it resolves to.
//!
//! It speaks the two things a browser asks of a proxy: `CONNECT host:port` (https and web sockets), answered with a
//! tunnel, and a plain `GET http://host/...`, sent on with the connection closed after one answer. It reads no page and
//! keeps nothing.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::OnceLock;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::doh::DohResolver;
use crate::public_only::{REFUSED, is_public};

/// The longest request head the gate reads: a browser's is a few hundred bytes, a cookie-heavy one a few thousand.
const MAX_HEAD: usize = 64 * 1024;
/// How long a browser has to send its request head, and the gate to reach the site.
const SETUP_TIMEOUT: Duration = Duration::from_secs(20);

/// How the gate looks names up and which addresses it connects to.
#[derive(Clone)]
struct Policy {
    doh: Option<DohResolver>,
    allow: fn(IpAddr) -> bool,
}

static ADDRESS: OnceLock<Option<SocketAddr>> = OnceLock::new();

/// The address of the app's gate, started at the first call on a thread of its own; every later call gives the same one.
/// Names are looked up through `doh()` when the app uses DNS over HTTPS (asked once, at the start), else by the system.
/// `None` when it could not start: the hidden pages then cannot be used.
pub fn address(doh: impl FnOnce() -> Option<DohResolver>) -> Option<SocketAddr> {
    *ADDRESS.get_or_init(|| {
        start(Policy { doh: doh(), allow: is_public })
            .map_err(|error| tracing::warn!(%error, "the hidden pages' gate did not start"))
            .ok()
    })
}

/// Starts a gate on a free port of this computer and returns its address. It runs until the app quits.
fn start(policy: Policy) -> io::Result<SocketAddr> {
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    std::thread::Builder::new().name("page-gate".to_owned()).spawn(move || {
        runtime.block_on(async move {
            let listener = match TcpListener::from_std(listener) {
                Ok(listener) => listener,
                Err(error) => return tracing::warn!(%error, "the hidden pages' gate stopped"),
            };
            loop {
                match listener.accept().await {
                    Ok((browser, _)) => {
                        let policy = policy.clone();
                        tokio::spawn(async move {
                            if let Err(error) = serve(browser, &policy).await {
                                tracing::debug!(%error, "a hidden page's connection ended");
                            }
                        });
                    }
                    // out of sockets for a moment: the browser retries
                    Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                }
            }
        });
    })?;
    Ok(address)
}

/// One connection of a browser: reads its request, reaches the site if it may, and carries the bytes both ways.
async fn serve(mut browser: TcpStream, policy: &Policy) -> io::Result<()> {
    let (head, rest) = tokio::time::timeout(SETUP_TIMEOUT, read_head(&mut browser)).await.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "no request"))??;
    let asked = match parse(&head) {
        Ok(asked) => asked,
        Err(why) => return refuse(&mut browser, "400 Bad Request", why).await,
    };
    let site = match tokio::time::timeout(SETUP_TIMEOUT, reach(&asked.host, asked.port, policy)).await {
        Ok(Ok(site)) => site,
        Ok(Err(error)) if error.kind() == io::ErrorKind::PermissionDenied => return refuse(&mut browser, "403 Forbidden", REFUSED).await,
        Ok(Err(error)) => return refuse(&mut browser, "502 Bad Gateway", &error.to_string()).await,
        Err(_) => return refuse(&mut browser, "504 Gateway Timeout", "the site did not answer").await,
    };
    let mut site = site;
    match asked.forward {
        // a tunnel: the browser speaks to the site itself from here on (TLS, a web socket)
        None => browser.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await?,
        Some(head) => site.write_all(head.as_bytes()).await?,
    }
    site.write_all(&rest).await?;
    tokio::io::copy_bidirectional(&mut browser, &mut site).await?;
    Ok(())
}

async fn refuse(browser: &mut TcpStream, status: &str, why: &str) -> io::Result<()> {
    let body = format!("{why}\n");
    browser.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await?;
    browser.shutdown().await
}

/// Reads up to the blank line that ends a request head. Returns the head as text and any bytes that came after it.
async fn read_head(browser: &mut TcpStream) -> io::Result<(String, Vec<u8>)> {
    let mut bytes = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = bytes.split_off(end + 4);
            return String::from_utf8(bytes).map(|head| (head, rest)).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "the request head is not text"));
        }
        if bytes.len() > MAX_HEAD {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "the request head is too long"));
        }
        let n = browser.read(&mut chunk).await?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the connection closed before a request"));
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
}

/// Where a request goes, and for a plain request the head to send on.
#[derive(Debug, PartialEq)]
struct Asked {
    host: String,
    port: u16,
    /// `None` for `CONNECT` (a tunnel). For a plain http request, its head as the site takes it: the path in place of the
    /// full address, and the connection closed after the answer, so the next request is judged again.
    forward: Option<String>,
}

/// Reads a request head (it ends with its blank line) as a proxy gets it.
fn parse(head: &str) -> Result<Asked, &'static str> {
    let (line, headers) = head.split_once("\r\n").ok_or("no request line")?;
    let mut parts = line.split(' ');
    let (Some(method), Some(target), Some(version), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else { return Err("a malformed request line") };
    if method.eq_ignore_ascii_case("CONNECT") {
        // `host:port`, the host in brackets when it is an IPv6 address
        let (host, port) = target.rsplit_once(':').ok_or("CONNECT needs host:port")?;
        let port = port.parse().map_err(|_| "CONNECT needs a port number")?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if host.is_empty() {
            return Err("CONNECT needs a host");
        }
        return Ok(Asked { host: host.to_owned(), port, forward: None });
    }
    let url = url::Url::parse(target).map_err(|_| "a proxy request needs a full address")?;
    if url.scheme() != "http" {
        return Err("only http addresses are sent on; https goes through CONNECT");
    }
    let host = match url.host() {
        Some(url::Host::Domain(name)) => name.to_owned(),
        Some(url::Host::Ipv4(ip)) => ip.to_string(),
        Some(url::Host::Ipv6(ip)) => ip.to_string(),
        None => return Err("the address has no host"),
    };
    let path = &url[url::Position::BeforePath..url::Position::AfterQuery];
    let mut forward = format!("{method} {path} {version}\r\n");
    for header in headers.split("\r\n").filter(|header| !header.is_empty()) {
        let name = header.split(':').next().unwrap_or_default().trim();
        if !name.eq_ignore_ascii_case("connection") && !name.eq_ignore_ascii_case("proxy-connection") && !name.eq_ignore_ascii_case("proxy-authorization") {
            forward.push_str(header);
            forward.push_str("\r\n");
        }
    }
    forward.push_str("Connection: close\r\n\r\n");
    Ok(Asked { host, port: url.port_or_known_default().unwrap_or(80), forward: Some(forward) })
}

/// Connects to `host`: an address written out, or a name looked up here. Only the addresses the policy allows are
/// tried; with none left the error is `PermissionDenied`.
async fn reach(host: &str, port: u16, policy: &Policy) -> io::Result<TcpStream> {
    let found: Vec<IpAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![ip],
        Err(_) => match &policy.doh {
            Some(doh) => doh.lookup(host).await?,
            None => tokio::net::lookup_host((host, port)).await?.map(|address| address.ip()).collect(),
        },
    };
    let allowed: Vec<IpAddr> = found.into_iter().filter(|ip| (policy.allow)(*ip)).collect();
    if allowed.is_empty() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, REFUSED));
    }
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no address");
    for ip in allowed {
        match TcpStream::connect(SocketAddr::new(ip, port)).await {
            Ok(site) => return Ok(site),
            Err(error) => last = error,
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A gate that lets every address through (the sites of these tests are on this computer), or the app's own rule.
    fn gate(allow: fn(IpAddr) -> bool) -> SocketAddr {
        start(Policy { doh: None, allow }).expect("the gate starts")
    }

    /// A site on this computer that answers every connection with `answer` after reading what was sent.
    async fn site(answer: &'static str) -> (SocketAddr, tokio::sync::mpsc::UnboundedReceiver<String>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (seen, requests) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let seen = seen.clone();
                tokio::spawn(async move {
                    let mut buffer = [0u8; 4096];
                    let n = stream.read(&mut buffer).await.unwrap_or(0);
                    let _ = seen.send(String::from_utf8_lossy(&buffer[..n]).into_owned());
                    let _ = stream.write_all(answer.as_bytes()).await;
                });
            }
        });
        (address, requests)
    }

    /// Sends `request` to the gate and returns everything it answers until the connection closes.
    async fn ask(gate: SocketAddr, request: &str) -> String {
        let mut browser = TcpStream::connect(gate).await.unwrap();
        browser.write_all(request.as_bytes()).await.unwrap();
        let mut answer = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(10), browser.read_to_end(&mut answer)).await;
        String::from_utf8_lossy(&answer).into_owned()
    }

    #[test]
    fn a_connect_names_its_host_and_port() {
        assert_eq!(parse("CONNECT comix.test:443 HTTP/1.1\r\nHost: comix.test:443\r\n\r\n"), Ok(Asked { host: "comix.test".into(), port: 443, forward: None }));
        assert_eq!(parse("CONNECT [2606:4700::1111]:8443 HTTP/1.1\r\n\r\n"), Ok(Asked { host: "2606:4700::1111".into(), port: 8443, forward: None }));
        assert!(parse("CONNECT comix.test HTTP/1.1\r\n\r\n").is_err());
        assert!(parse("CONNECT :443 HTTP/1.1\r\n\r\n").is_err());
        assert!(parse("GARBAGE\r\n\r\n").is_err());
    }

    #[test]
    fn a_plain_request_is_sent_on_with_its_path_and_one_answer_per_connection() {
        let asked = parse("GET http://site.test:8080/a/b?c=1 HTTP/1.1\r\nHost: site.test:8080\r\nProxy-Connection: keep-alive\r\nConnection: keep-alive\r\nAccept: */*\r\n\r\n").unwrap();
        assert_eq!((asked.host.as_str(), asked.port), ("site.test", 8080));
        assert_eq!(asked.forward.as_deref(), Some("GET /a/b?c=1 HTTP/1.1\r\nHost: site.test:8080\r\nAccept: */*\r\nConnection: close\r\n\r\n"));
        assert_eq!(parse("GET http://site.test HTTP/1.1\r\n\r\n").unwrap().port, 80);
        assert!(parse("GET /a HTTP/1.1\r\nHost: site.test\r\n\r\n").is_err(), "a request that names no site is not a proxy request");
        assert!(parse("GET https://site.test/ HTTP/1.1\r\n\r\n").is_err());
        assert!(parse("GET file:///etc/passwd HTTP/1.1\r\n\r\n").is_err());
    }

    #[tokio::test]
    async fn this_computer_is_refused_by_address_and_by_name() {
        let (local, mut requests) = site("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi").await;
        let gate = gate(is_public);
        for request in [
            format!("CONNECT 127.0.0.1:{} HTTP/1.1\r\n\r\n", local.port()),
            format!("CONNECT localhost:{} HTTP/1.1\r\n\r\n", local.port()),
            format!("CONNECT [::1]:{} HTTP/1.1\r\n\r\n", local.port()),
            "CONNECT 192.168.1.1:80 HTTP/1.1\r\n\r\n".to_owned(),
            "CONNECT 169.254.169.254:80 HTTP/1.1\r\n\r\n".to_owned(),
            format!("GET http://127.0.0.1:{}/admin HTTP/1.1\r\nHost: x\r\n\r\n", local.port()),
            format!("GET http://localhost:{}/admin HTTP/1.1\r\nHost: x\r\n\r\n", local.port()),
        ] {
            let answer = ask(gate, &request).await;
            assert!(answer.starts_with("HTTP/1.1 403 Forbidden"), "{request:?} gave {answer:?}");
            assert!(answer.contains("address not allowed"), "{answer:?}");
        }
        assert!(requests.try_recv().is_err(), "nothing reached the site on this computer");
    }

    #[tokio::test]
    async fn a_tunnel_carries_the_bytes_both_ways() {
        let (local, mut requests) = site("pong").await;
        let gate = gate(|_| true);
        let mut browser = TcpStream::connect(gate).await.unwrap();
        browser.write_all(format!("CONNECT 127.0.0.1:{} HTTP/1.1\r\nHost: x\r\n\r\n", local.port()).as_bytes()).await.unwrap();
        let mut opened = [0u8; 39];
        browser.read_exact(&mut opened).await.unwrap();
        assert_eq!(&opened[..], b"HTTP/1.1 200 Connection Established\r\n\r\n");
        browser.write_all(b"ping").await.unwrap();
        let mut answer = [0u8; 4];
        browser.read_exact(&mut answer).await.unwrap();
        assert_eq!(&answer, b"pong");
        assert_eq!(requests.recv().await.as_deref(), Some("ping"), "the site got the browser's bytes, not the CONNECT");
    }

    #[tokio::test]
    async fn a_plain_request_reaches_the_site_with_its_path() {
        let (local, mut requests) = site("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await;
        let gate = gate(|_| true);
        let answer = ask(gate, &format!("GET http://127.0.0.1:{}/page?x=1 HTTP/1.1\r\nHost: 127.0.0.1\r\nProxy-Connection: keep-alive\r\n\r\n", local.port())).await;
        assert!(answer.ends_with("\r\n\r\nok"), "{answer:?}");
        let sent = requests.recv().await.unwrap();
        assert!(sent.starts_with("GET /page?x=1 HTTP/1.1\r\n"), "{sent:?}");
        assert!(sent.contains("Connection: close") && !sent.contains("Proxy-Connection"), "{sent:?}");
    }

    #[tokio::test]
    async fn a_request_the_gate_cannot_read_is_answered_not_dropped() {
        let gate = gate(|_| true);
        assert!(ask(gate, "GET /no-site HTTP/1.1\r\nHost: x\r\n\r\n").await.starts_with("HTTP/1.1 400 Bad Request"));
        // a port nothing listens on
        let free = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        assert!(ask(gate, &format!("CONNECT 127.0.0.1:{free} HTTP/1.1\r\n\r\n")).await.starts_with("HTTP/1.1 502 Bad Gateway"));
    }
}
