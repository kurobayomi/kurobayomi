//! Cloudflare challenge handling (T-12a-015, T-12a-016, T-12a-021, T-12a-022).
//! A 403 or 503 from a Cloudflare server is solved once per registrable domain in a WebView;
//! the solve is shared by every request that hit the same domain, and each of them then retries once.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use http::header::{HeaderMap, SERVER, USER_AGENT};
use http::StatusCode;
use kuro_platform::hidden_pages::HiddenPages;
use kuro_platform::toast::{ToastLength, Toaster};
use kuro_platform::webview::{SolveFailure, SolveOutcome, SolveRequest, WebViewHost};
use tokio::sync::{Semaphore, watch};
use tokio_util::sync::CancellationToken;
use url::Url;

use super::safe_headers::safe_request_headers;
use super::{Chain, Interceptor};
use crate::cookies::CookieJar;
use crate::error::NetError;
use crate::request::Request;
use crate::response::{Response, body_from_chunks};

/// Statuses that a Cloudflare challenge uses.
pub const CF_STATUS_CODES: [u16; 2] = [403, 503];
/// `Server` values that mark a Cloudflare response. Compared case-insensitively.
pub const CF_SERVER_VALUES: [&str; 2] = ["cloudflare-nginx", "cloudflare"];
/// The cookie that proves the challenge was passed.
pub const CF_COOKIE_NAMES: &[&str] = &["cf_clearance"];
pub const CF_SOLVE_TIMEOUT: Duration = Duration::from_secs(30);
/// Distinct domains that may be solved at once. Later ones wait in FIFO order.
pub const CF_MAX_PARALLEL_SOLVES: usize = 2;

/// Shown when no WebKitGTK is available (`information_webkit_missing`).
pub const WEBKIT_MISSING_TEXT: &str =
    "WebKitGTK is required to solve Cloudflare checks. Install webkitgtk-6.0.";
/// Shown when WebKitGTK is older than the minimum (`information_webview_outdated_linux`).
pub const WEBKIT_OUTDATED_TEXT: &str = "Please update WebKitGTK for better compatibility";

/// `true` when the response is a Cloudflare challenge: a 403 or 503 with a Cloudflare `Server` header.
pub fn is_challenge(status: StatusCode, headers: &HeaderMap) -> bool {
    CF_STATUS_CODES.contains(&status.as_u16())
        && headers
            .get(SERVER)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|server| CF_SERVER_VALUES.iter().any(|s| server.eq_ignore_ascii_case(s)))
}

/// The single-flight key: the registrable domain (public suffix rules), or the host when there is none.
pub fn solve_key(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    psl::domain_str(host).map_or_else(|| host.to_owned(), str::to_owned)
}

/// The work of one solve. It runs on a spawned task, so it outlives the request that started it.
pub(crate) struct SolveJob {
    webview: Arc<dyn WebViewHost>,
    jar: CookieJar,
    url: Url,
    headers: Vec<(String, String)>,
    user_agent: String,
}

impl SolveJob {
    /// Expires the old clearance cookie, then solves. The old value is kept for the "it changed" test.
    async fn run(self, cancel: CancellationToken) -> SolveOutcome {
        let old_clearance = self
            .jar
            .get(&self.url)
            .into_iter()
            .find(|c| c.name() == CF_COOKIE_NAMES[0])
            .map(|c| c.value().to_owned());
        self.jar.remove(&self.url, Some(CF_COOKIE_NAMES));
        let req = SolveRequest {
            url: self.url.clone(),
            headers: self.headers,
            user_agent: self.user_agent,
            cookies: self.jar.export_for(&self.url),
            old_clearance,
            timeout: CF_SOLVE_TIMEOUT,
            cookie_names: CF_COOKIE_NAMES,
            visible: false,
        };
        let hidden = self.webview.solve_challenge(req.clone(), cancel.clone()).await;
        // A hidden solve that ran out of time is tried again where the person can finish it (T-12a-032), when the
        // host can show a window. The deadline there is 120 s.
        let timed_out = matches!(hidden, SolveOutcome::Failed { reason: SolveFailure::Timeout, .. });
        if timed_out && self.webview.supports_visible() {
            return self.webview.solve_challenge(req.into_visible(), cancel).await;
        }
        hidden
    }
}

/// One solve in progress, shared by the requests that joined it.
struct Flight {
    waiters: AtomicUsize,
    cancel: CancellationToken,
    outcome: watch::Sender<Option<SolveOutcome>>,
}

/// Runs at most one solve per key, and at most `CF_MAX_PARALLEL_SOLVES` at once.
pub(crate) struct SolveCoordinator {
    flights: Mutex<HashMap<String, Arc<Flight>>>,
    slots: Arc<Semaphore>,
}

impl Default for SolveCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl SolveCoordinator {
    fn new() -> Self {
        Self {
            flights: Mutex::new(HashMap::new()),
            slots: Arc::new(Semaphore::new(CF_MAX_PARALLEL_SOLVES)),
        }
    }

    fn flights(&self) -> MutexGuard<'_, HashMap<String, Arc<Flight>>> {
        self.flights.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Joins the solve for `key`, starting it when none is running. The caller's cancellation ends only its own
    /// wait; the solve itself stops when its last waiter is gone.
    pub(crate) async fn solve(
        self: &Arc<Self>,
        key: String,
        job: SolveJob,
        caller: &CancellationToken,
    ) -> Result<SolveOutcome, NetError> {
        let (flight, mut outcome) = self.join(key.clone(), job);
        let _waiter = Waiter {
            coordinator: self.clone(),
            key,
            flight,
        };
        tokio::select! {
            biased;
            _ = caller.cancelled() => Err(NetError::Canceled),
            done = wait_for_outcome(&mut outcome) => Ok(done),
        }
    }

    fn join(
        self: &Arc<Self>,
        key: String,
        job: SolveJob,
    ) -> (Arc<Flight>, watch::Receiver<Option<SolveOutcome>>) {
        let mut flights = self.flights();
        let flight = match flights.get(&key) {
            Some(existing) => existing.clone(),
            None => {
                let (tx, _) = watch::channel(None);
                let flight = Arc::new(Flight {
                    waiters: AtomicUsize::new(0),
                    cancel: CancellationToken::new(),
                    outcome: tx,
                });
                flights.insert(key.clone(), flight.clone());
                tokio::spawn(run_flight(self.clone(), key, flight.clone(), job));
                flight
            }
        };
        flight.waiters.fetch_add(1, Ordering::SeqCst);
        let rx = flight.outcome.subscribe();
        (flight, rx)
    }
}

async fn run_flight(coordinator: Arc<SolveCoordinator>, key: String, flight: Arc<Flight>, job: SolveJob) {
    let cancel = flight.cancel.clone();
    let outcome = tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        permit = coordinator.slots.clone().acquire_owned() => permit.ok(),
    };
    let outcome = match outcome {
        Some(_permit) => job.run(cancel).await,
        None => SolveOutcome::Failed {
            reason: SolveFailure::Cancelled,
            outdated: false,
        },
    };
    let mut flights = coordinator.flights();
    flight.outcome.send_replace(Some(outcome));
    if flights.get(&key).is_some_and(|f| Arc::ptr_eq(f, &flight)) {
        flights.remove(&key);
    }
}

async fn wait_for_outcome(rx: &mut watch::Receiver<Option<SolveOutcome>>) -> SolveOutcome {
    loop {
        if let Some(outcome) = rx.borrow_and_update().clone() {
            return outcome;
        }
        if rx.changed().await.is_err() {
            return SolveOutcome::Failed {
                reason: SolveFailure::Cancelled,
                outdated: false,
            };
        }
    }
}

/// Counts one waiter. When the last waiter leaves an unfinished solve, the solve is cancelled and forgotten.
struct Waiter {
    coordinator: Arc<SolveCoordinator>,
    key: String,
    flight: Arc<Flight>,
}

impl Drop for Waiter {
    fn drop(&mut self) {
        let mut flights = self.coordinator.flights();
        let left = self.flight.waiters.fetch_sub(1, Ordering::SeqCst) - 1;
        if left == 0 && self.flight.outcome.borrow().is_none() {
            self.flight.cancel.cancel();
            if flights.get(&self.key).is_some_and(|f| Arc::ptr_eq(f, &self.flight)) {
                flights.remove(&self.key);
            }
        }
    }
}

pub struct CloudflareInterceptor {
    webview: Arc<dyn WebViewHost>,
    toaster: Toaster,
    jar: CookieJar,
    solves: Arc<SolveCoordinator>,
    pages: Option<Arc<dyn HiddenPages>>,
}

impl CloudflareInterceptor {
    pub fn new(webview: Arc<dyn WebViewHost>, toaster: Toaster, jar: CookieJar) -> Self {
        Self {
            webview,
            toaster,
            jar,
            solves: Arc::new(SolveCoordinator::new()),
            pages: None,
        }
    }

    /// Sends a challenged request from a hidden page of its site instead of solving the check for the app's own client:
    /// the pass is tied to the browser that earned it (its TLS fingerprint), so the app's client cannot reuse it. With hidden
    /// pages the solve through [`WebViewHost`] (the system browser) is not used at all, even when a page fails.
    pub fn with_hidden_pages(mut self, pages: Option<Arc<dyn HiddenPages>>) -> Self {
        self.pages = pages;
        self
    }
}

/// The answer of a hidden page (`kuro_platform::hidden_pages`): the status, headers and final address of the page's
/// own `fetch`, with the body as text or base64.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageAnswer {
    status: u16,
    #[serde(default)]
    headers: Vec<(String, String)>,
    url: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    body_encoding: Option<String>,
}

/// The request as the hidden page takes it. `None` when its body is not text (a page sends text bodies only).
fn page_request(req: &Request) -> Option<String> {
    let body = match &req.body {
        crate::request::Body::Empty => None,
        crate::request::Body::Bytes { data, .. } => Some(std::str::from_utf8(data).ok()?.to_owned()),
        _ => return None,
    };
    // the headers a page can set: its own browser sends the user agent, the encodings and the connection, and the cache
    // directive of the app's client only means something to that client
    let skipped = ["user-agent", "cache-control", "pragma", "accept-encoding", "connection", "host", "content-length", "cookie", "origin"];
    let headers: Vec<(&str, &str)> = req.headers.iter().filter(|(n, _)| !skipped.contains(&n.as_str())).filter_map(|(n, v)| Some((n.as_str(), v.to_str().ok()?))).collect();
    serde_json::to_string(&serde_json::json!({ "method": req.method.as_str(), "url": req.url.as_str(), "headers": headers, "body": body })).ok()
}

/// The response of a page's answer. The page's `fetch` has already decoded the body, so the headers that describe its
/// encoding and length are left out.
fn response_of(answer: &str) -> Option<Response> {
    use base64::Engine as _;
    let answer: PageAnswer = serde_json::from_str(answer).ok()?;
    let bytes = match answer.body_encoding.as_deref() {
        Some("base64") => base64::engine::general_purpose::STANDARD.decode(&answer.body).ok()?,
        _ => answer.body.into_bytes(),
    };
    let mut headers = HeaderMap::new();
    for (name, value) in &answer.headers {
        if ["content-encoding", "content-length", "transfer-encoding"].contains(&name.to_ascii_lowercase().as_str()) {
            continue;
        }
        if let (Ok(name), Ok(value)) = (http::header::HeaderName::from_bytes(name.as_bytes()), http::HeaderValue::from_str(value)) {
            headers.append(name, value);
        }
    }
    Some(Response::new(StatusCode::from_u16(answer.status).ok()?, headers, Url::parse(&answer.url).ok()?, body_from_chunks(&[&bytes[..]])))
}

#[async_trait]
impl Interceptor for CloudflareInterceptor {
    async fn intercept(&self, chain: Chain<'_>, req: Request) -> Result<Response, NetError> {
        let retry = req.clone();
        let response = chain.proceed(req).await?;
        if !is_challenge(response.status, &response.headers) {
            return Ok(response);
        }
        if let (Some(pages), Some(json)) = (&self.pages, page_request(&retry)) {
            let pages = Arc::clone(pages);
            let sent = json.chars().take(400).collect::<String>();
            // the page works on the window's thread and the call waits for it, so it runs off the async workers
            let answer = tokio::select! {
                biased;
                _ = retry.opts.cancel.cancelled() => return Err(NetError::Canceled),
                answer = tokio::task::spawn_blocking(move || pages.fetch(&json)) => answer.map_err(|e| e.to_string()).and_then(|r| r),
            };
            match answer.as_deref().map(|a| response_of(a).ok_or("the page's answer did not read")) {
                Ok(Ok(page_response)) => return Ok(page_response),
                Ok(Err(why)) => tracing::warn!(url = %retry.url, why, "the hidden page could not answer"),
                Err(why) => tracing::warn!(url = %retry.url, why = %why, request = %sent, "the hidden page could not answer"),
            }
            // a platform with hidden pages does not open the system browser: the person would have to export cookies by hand,
            // and a cookie cannot clear a check that is tied to the browser that solved it. The challenge comes back as the
            // answer, and the source shows that it is not answering.
            return Ok(response);
        }
        if !self.webview.available() {
            self.toaster.show(WEBKIT_MISSING_TEXT, ToastLength::Long);
            return Ok(response);
        }
        // The page is read now rather than held through the solve: a held body keeps its dispatcher slot, and the
        // requests that would join this solve would queue behind it. The copy is what comes back on Unavailable.
        // ponytail: the whole page is read with no size cap; a cap is the upgrade if a server streams a huge 503.
        let (status, resp_headers, resp_url, from_cache) =
            (response.status, response.headers.clone(), response.url.clone(), response.from_cache);
        let page = tokio::select! {
            biased;
            _ = retry.opts.cancel.cancelled() => return Err(NetError::Canceled),
            page = response.bytes() => page?,
        };

        let user_agent = retry
            .headers
            .get(USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let headers = safe_request_headers(&retry.headers)
            .into_iter()
            .filter_map(|(n, v)| Some((n.as_str().to_owned(), v.to_str().ok()?.to_owned())))
            .collect();
        let job = SolveJob {
            webview: self.webview.clone(),
            jar: self.jar.clone(),
            url: retry.url.clone(),
            headers,
            user_agent,
        };
        let outcome = self
            .solves
            .solve(solve_key(&retry.url), job, &retry.opts.cancel)
            .await?;
        // Section 2.2: an unavailable WebKit is the same as the `!available` branch, so the original response
        // comes back with its body.
        if matches!(outcome, SolveOutcome::Failed { reason: SolveFailure::Unavailable, .. }) {
            self.toaster.show(WEBKIT_MISSING_TEXT, ToastLength::Long);
            let mut back = Response::new(status, resp_headers, resp_url, body_from_chunks(&[&page[..]]));
            back.from_cache = from_cache;
            return Ok(back);
        }
        match outcome {
            SolveOutcome::Cleared { cookies, .. } => {
                self.jar.import(&retry.url, &cookies);
                chain.proceed(retry).await
            }
            SolveOutcome::Failed {
                reason: SolveFailure::Cancelled,
                ..
            } => Err(NetError::Canceled),
            SolveOutcome::NoChallenge { outdated }
            | SolveOutcome::Failed { outdated, .. } => {
                if outdated {
                    self.toaster.show(WEBKIT_OUTDATED_TEXT, ToastLength::Long);
                }
                Err(NetError::CloudflareBypassFailed)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::Duration;

    use http::HeaderValue;
    use kuro_platform::toast::{ToastFeed, toast_channel};
    use kuro_platform::webview::{NoWebView, SolveRequest, WebCookie};
    use wiremock::matchers::any;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::client::ClientBuilder;
    use crate::request;
    use crate::transport::{Transport, TransportLimits};

    /// A WebView double. It counts solves and answers `outcome` after `delay`, or `Cancelled` when cancelled.
    struct FakeWebView {
        solves: AtomicUsize,
        saw_cancel: AtomicBool,
        delay: Duration,
        outcome: SolveOutcome,
        available: bool,
    }

    impl FakeWebView {
        fn new(outcome: SolveOutcome, delay: Duration) -> Arc<Self> {
            Arc::new(Self {
                solves: AtomicUsize::new(0),
                saw_cancel: AtomicBool::new(false),
                delay,
                outcome,
                available: true,
            })
        }
    }

    #[async_trait]
    impl WebViewHost for FakeWebView {
        fn available(&self) -> bool {
            self.available
        }

        async fn get_html(&self) -> Option<String> {
            None
        }

        async fn solve_challenge(&self, _req: SolveRequest, cancel: CancellationToken) -> SolveOutcome {
            self.solves.fetch_add(1, Ordering::SeqCst);
            tokio::select! {
                _ = tokio::time::sleep(self.delay) => self.outcome.clone(),
                _ = cancel.cancelled() => {
                    self.saw_cancel.store(true, Ordering::SeqCst);
                    SolveOutcome::Failed { reason: SolveFailure::Cancelled, outdated: false }
                }
            }
        }
    }

    fn cleared() -> SolveOutcome {
        SolveOutcome::Cleared {
            cookies: vec![WebCookie {
                name: "cf_clearance".into(),
                value: "fresh".into(),
                domain: String::new(),
                path: "/".into(),
                secure: false,
                http_only: false,
                expires: None,
                same_site: None,
            }],
            outdated: false,
        }
    }

    /// A client whose jar is the one the Cloudflare step writes into.
    fn client(webview: Arc<dyn WebViewHost>, toaster: Toaster, dir: &std::path::Path) -> (crate::NetworkClient, CookieJar) {
        let jar = CookieJar::load(dir.join("cookies.json"));
        let transport = Arc::new(Transport::new(TransportLimits::default(), jar.provider()).unwrap());
        let chain: Vec<Arc<dyn Interceptor>> = vec![
            Arc::new(crate::interceptors::UncaughtExceptionInterceptor),
            Arc::new(CloudflareInterceptor::new(webview, toaster, jar.clone())),
        ];
        let client = ClientBuilder {
            chain,
            cloudflare: true,
            transport,
            cookies: jar.clone(),
            user_agent: crate::client::fixed_user_agent(""),
        }
        .build();
        (client, jar)
    }

    /// A hidden page that answers with a fixed JSON, or fails; it counts its calls and keeps the last request.
    struct FakePages {
        answer: Result<String, String>,
        calls: AtomicUsize,
        last: Mutex<String>,
    }

    impl HiddenPages for FakePages {
        fn fetch(&self, request_json: &str) -> Result<String, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last.lock().unwrap() = request_json.to_owned();
            self.answer.clone()
        }
    }

    fn client_with_pages(webview: Arc<dyn WebViewHost>, pages: Arc<FakePages>, dir: &std::path::Path) -> crate::NetworkClient {
        let jar = CookieJar::load(dir.join("cookies.json"));
        let transport = Arc::new(Transport::new(TransportLimits::default(), jar.provider()).unwrap());
        let (toaster, _feed) = toast_channel();
        let chain: Vec<Arc<dyn Interceptor>> = vec![Arc::new(CloudflareInterceptor::new(webview, toaster, jar.clone()).with_hidden_pages(Some(pages)))];
        ClientBuilder { chain, cloudflare: true, transport, cookies: jar, user_agent: crate::client::fixed_user_agent("") }.build()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_hidden_page_answers_the_challenge_and_the_solve_is_not_asked() {
        let server = challenge_server_for(None).await;
        let dir = tempfile::tempdir().unwrap();
        let webview = FakeWebView::new(cleared(), Duration::from_millis(10));
        use base64::Engine as _;
        let body = base64::engine::general_purpose::STANDARD.encode(b"\x00\xffimage");
        let pages = Arc::new(FakePages {
            answer: Ok(format!(r#"{{"status":200,"headers":[["content-type","image/webp"],["content-encoding","gzip"]],"url":"{}/x","body":"{body}","bodyEncoding":"base64"}}"#, server.uri())),
            calls: AtomicUsize::new(0),
            last: Mutex::new(String::new()),
        });
        let client = client_with_pages(webview.clone(), pages.clone(), dir.path());
        let mut request = request::get(&format!("{}/x", server.uri())).unwrap();
        request.headers.insert("referer", "https://site.test/".parse().unwrap());
        let resp = client.execute(request).await.unwrap();
        assert_eq!(resp.status, StatusCode::OK);
        assert_eq!(resp.headers.get("content-type").unwrap(), "image/webp");
        assert!(resp.headers.get("content-encoding").is_none(), "the page already decoded the body");
        assert_eq!(&resp.bytes().await.unwrap()[..], b"\x00\xffimage");
        assert_eq!(pages.calls.load(Ordering::SeqCst), 1);
        let sent = pages.last.lock().unwrap().clone();
        assert!(sent.contains(r#"["referer","https://site.test/"]"#), "the headers a page can set travel with it");
        assert!(!sent.contains("user-agent") && !sent.contains("cache-control"), "{sent}");
        assert_eq!(webview.solves.load(Ordering::SeqCst), 0, "the system browser was not asked");
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "no retry from the app's own client");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_page_that_cannot_answer_leaves_the_challenge_and_does_not_open_the_browser() {
        let server = challenge_server_for(None).await;
        let dir = tempfile::tempdir().unwrap();
        let webview = FakeWebView::new(cleared(), Duration::from_millis(10));
        let pages = Arc::new(FakePages { answer: Err("no engine".into()), calls: AtomicUsize::new(0), last: Mutex::new(String::new()) });
        let client = client_with_pages(webview.clone(), pages.clone(), dir.path());
        let resp = client.execute(request::get(&format!("{}/", server.uri())).unwrap()).await.unwrap();
        assert_eq!(resp.status, StatusCode::SERVICE_UNAVAILABLE, "the challenge comes back");
        assert_eq!(pages.calls.load(Ordering::SeqCst), 1);
        assert_eq!(webview.solves.load(Ordering::SeqCst), 0, "the system browser was not asked");
    }

    /// A Cloudflare challenge for the first `challenges` requests, then a plain 200. `None` means always a challenge.
    async fn challenge_server_for(challenges: Option<u64>) -> MockServer {
        let server = MockServer::start().await;
        let mut challenge = Mock::given(any())
            .respond_with(ResponseTemplate::new(503).insert_header("server", "cloudflare"));
        challenge = match challenges {
            Some(n) => challenge.up_to_n_times(n),
            None => challenge,
        };
        challenge.mount(&server).await;
        if challenges.is_some() {
            Mock::given(any())
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
        }
        server
    }


    async fn next_toast(feed: &ToastFeed) -> Option<String> {
        tokio::time::timeout(Duration::from_millis(200), feed.next())
            .await
            .ok()
            .flatten()
            .map(|m| m.text)
    }

    #[test]
    fn a_403_or_503_from_cloudflare_is_a_challenge_in_any_case() {
        let mut h = HeaderMap::new();
        h.insert(SERVER, HeaderValue::from_static("cloudflare-nginx"));
        assert!(is_challenge(StatusCode::SERVICE_UNAVAILABLE, &h));
        h.insert(SERVER, HeaderValue::from_static("Cloudflare"));
        assert!(is_challenge(StatusCode::FORBIDDEN, &h));
    }

    #[test]
    fn a_403_without_a_cloudflare_server_is_not_a_challenge() {
        let mut h = HeaderMap::new();
        assert!(!is_challenge(StatusCode::FORBIDDEN, &h));
        h.insert(SERVER, HeaderValue::from_static("nginx"));
        assert!(!is_challenge(StatusCode::SERVICE_UNAVAILABLE, &h));
        h.insert(SERVER, HeaderValue::from_static("cloudflare"));
        assert!(!is_challenge(StatusCode::OK, &h));
    }

    #[test]
    fn the_key_is_the_registrable_domain_or_the_host() {
        let u = |s: &str| Url::parse(s).unwrap();
        assert_eq!(solve_key(&u("https://a.b.example.co.uk/x")), "example.co.uk");
        assert_eq!(solve_key(&u("https://localhost:8080/")), "localhost");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn twenty_parallel_requests_to_one_domain_make_one_solve() {
        let server = challenge_server_for(None).await;
        let dir = tempfile::tempdir().unwrap();
        let webview = FakeWebView::new(cleared(), Duration::from_millis(500));
        let (toaster, _feed) = toast_channel();
        let (client, _jar) = client(webview.clone(), toaster, dir.path());
        let url = format!("{}/page", server.uri());
        let mut tasks = Vec::new();
        for _ in 0..20 {
            let client = client.clone();
            let url = url.clone();
            // The response is dropped inside the task: an unread body holds a dispatcher slot.
            tasks.push(tokio::spawn(async move {
                client.execute(request::get(&url).unwrap()).await.map(|r| r.status)
            }));
        }
        for task in tasks {
            let status = task.await.unwrap().unwrap();
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        }
        assert_eq!(webview.solves.load(Ordering::SeqCst), 1);
        assert_eq!(server.received_requests().await.unwrap().len(), 40);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cleared_solve_lets_the_single_retry_succeed_and_keeps_the_cookie() {
        let server = challenge_server_for(Some(1)).await;
        let dir = tempfile::tempdir().unwrap();
        let webview = FakeWebView::new(cleared(), Duration::from_millis(20));
        let (toaster, _feed) = toast_channel();
        let (client, jar) = client(webview.clone(), toaster, dir.path());
        let resp = client.get(&format!("{}/page", server.uri())).await.unwrap();
        assert_eq!(resp.status, StatusCode::OK);
        assert_eq!(webview.solves.load(Ordering::SeqCst), 1);
        let url = Url::parse(&server.uri()).unwrap();
        assert!(jar.get(&url).iter().any(|c| c.name() == "cf_clearance" && c.value() == "fresh"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_retry_that_is_still_blocked_returns_the_response() {
        let server = challenge_server_for(None).await;
        let dir = tempfile::tempdir().unwrap();
        let webview = FakeWebView::new(cleared(), Duration::from_millis(10));
        let (toaster, _feed) = toast_channel();
        let (client, _jar) = client(webview.clone(), toaster, dir.path());
        let resp = client.execute(request::get(&format!("{}/", server.uri())).unwrap()).await.unwrap();
        assert_eq!(resp.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_second_503_after_a_cleared_solve_is_an_http_error_and_only_two_requests_are_made() {
        let server = challenge_server_for(None).await;
        let dir = tempfile::tempdir().unwrap();
        let webview = FakeWebView::new(cleared(), Duration::from_millis(10));
        let (toaster, _feed) = toast_channel();
        let (client, _jar) = client(webview.clone(), toaster, dir.path());
        let err = client.get(&format!("{}/", server.uri())).await.err().unwrap();
        assert!(matches!(err, NetError::Http { code: 503 }), "{err:?}");
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        assert_eq!(webview.solves.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_challenge_or_a_timeout_maps_to_the_bypass_failure() {
        for outcome in [
            SolveOutcome::NoChallenge { outdated: false },
            SolveOutcome::Failed { reason: SolveFailure::Timeout, outdated: true },
        ] {
            let server = challenge_server_for(None).await;
            let dir = tempfile::tempdir().unwrap();
            let webview = FakeWebView::new(outcome, Duration::from_millis(10));
            let (toaster, feed) = toast_channel();
            let (client, _jar) = client(webview, toaster, dir.path());
            let err = client.get(&format!("{}/", server.uri())).await.err().unwrap();
            assert!(matches!(err, NetError::CloudflareBypassFailed), "{err:?}");
            let _ = next_toast(&feed).await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_outdated_webkit_after_a_failure_shows_the_update_toast() {
        let server = challenge_server_for(None).await;
        let dir = tempfile::tempdir().unwrap();
        let webview = FakeWebView::new(
            SolveOutcome::Failed { reason: SolveFailure::Timeout, outdated: true },
            Duration::from_millis(10),
        );
        let (toaster, feed) = toast_channel();
        let (client, _jar) = client(webview, toaster, dir.path());
        let _ = client.get(&format!("{}/", server.uri())).await;
        assert_eq!(next_toast(&feed).await.as_deref(), Some(WEBKIT_OUTDATED_TEXT));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn without_webkit_the_original_response_comes_back_with_a_toast() {
        let server = challenge_server_for(None).await;
        let dir = tempfile::tempdir().unwrap();
        let webview = Arc::new(NoWebView);
        let (toaster, feed) = toast_channel();
        let (client, _jar) = client(webview, toaster, dir.path());
        let resp = client.execute(request::get(&format!("{}/", server.uri())).unwrap()).await.unwrap();
        assert_eq!(resp.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(next_toast(&feed).await.as_deref(), Some(WEBKIT_MISSING_TEXT));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unavailable_solve_returns_the_original_response_with_its_body() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(
                ResponseTemplate::new(503)
                    .insert_header("server", "cloudflare")
                    .set_body_string("challenge page"),
            )
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let webview = FakeWebView::new(
            SolveOutcome::Failed { reason: SolveFailure::Unavailable, outdated: false },
            Duration::from_millis(10),
        );
        let (toaster, feed) = toast_channel();
        let (client, _jar) = client(webview.clone(), toaster, dir.path());
        let resp = client.execute(request::get(&format!("{}/", server.uri())).unwrap()).await.unwrap();
        assert_eq!(resp.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(resp.bytes().await.unwrap().as_ref(), &b"challenge page"[..]);
        assert_eq!(next_toast(&feed).await.as_deref(), Some(WEBKIT_MISSING_TEXT));
        assert_eq!(webview.solves.load(Ordering::SeqCst), 1);
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "no retry after Unavailable");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelling_the_only_waiter_aborts_the_solve() {
        let server = challenge_server_for(None).await;
        let dir = tempfile::tempdir().unwrap();
        let webview = FakeWebView::new(cleared(), Duration::from_secs(30));
        let (toaster, _feed) = toast_channel();
        let (client, _jar) = client(webview.clone(), toaster, dir.path());
        let req = request::get(&format!("{}/", server.uri())).unwrap();
        let cancel = req.opts.cancel.clone();
        let call = tokio::spawn(async move { client.execute(req).await });
        tokio::time::sleep(Duration::from_millis(300)).await;
        cancel.cancel();
        let err = call.await.unwrap().err().unwrap();
        assert!(matches!(err, NetError::Canceled));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(webview.saw_cancel.load(Ordering::SeqCst));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_waiter_that_cancels_does_not_cancel_the_shared_solve() {
        let server = challenge_server_for(Some(2)).await;
        let dir = tempfile::tempdir().unwrap();
        let webview = FakeWebView::new(cleared(), Duration::from_millis(400));
        let (toaster, _feed) = toast_channel();
        let (client, _jar) = client(webview.clone(), toaster, dir.path());
        let url = format!("{}/", server.uri());
        let stays = {
            let client = client.clone();
            let url = url.clone();
            tokio::spawn(async move { client.get(&url).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        let leaving = request::get(&url).unwrap();
        let cancel = leaving.opts.cancel.clone();
        let quitter = tokio::spawn(async move { client.execute(leaving).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();
        assert!(matches!(quitter.await.unwrap().err(), Some(NetError::Canceled)));
        let resp = stays.await.unwrap().unwrap();
        assert_eq!(resp.status, StatusCode::OK);
        assert!(!webview.saw_cancel.load(Ordering::SeqCst));
        assert_eq!(webview.solves.load(Ordering::SeqCst), 1);
    }

    /// Counts the solves in flight and the most that ran at once.
    struct Gauge {
        now: AtomicUsize,
        max: AtomicUsize,
        solves: AtomicUsize,
    }

    #[async_trait]
    impl WebViewHost for Gauge {
        fn available(&self) -> bool {
            true
        }

        async fn get_html(&self) -> Option<String> {
            None
        }

        async fn solve_challenge(&self, _req: SolveRequest, _cancel: CancellationToken) -> SolveOutcome {
            self.solves.fetch_add(1, Ordering::SeqCst);
            let now = self.now.fetch_add(1, Ordering::SeqCst) + 1;
            self.max.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(300)).await;
            self.now.fetch_sub(1, Ordering::SeqCst);
            SolveOutcome::NoChallenge { outdated: false }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn at_most_two_domains_solve_at_once_and_a_third_waits_its_turn() {
        let gauge = Arc::new(Gauge {
            now: AtomicUsize::new(0),
            max: AtomicUsize::new(0),
            solves: AtomicUsize::new(0),
        });
        let dir = tempfile::tempdir().unwrap();
        let jar = CookieJar::load(dir.path().join("cookies.json"));
        let coordinator = Arc::new(SolveCoordinator::new());
        let caller = CancellationToken::new();
        let mut tasks = Vec::new();
        for host in ["a.test", "b.test", "c.test"] {
            let url = Url::parse(&format!("https://{host}/")).unwrap();
            let job = SolveJob {
                webview: gauge.clone(),
                jar: jar.clone(),
                url: url.clone(),
                headers: Vec::new(),
                user_agent: String::new(),
            };
            let coordinator = coordinator.clone();
            let caller = caller.clone();
            tasks.push(tokio::spawn(async move {
                coordinator.solve(solve_key(&url), job, &caller).await
            }));
        }
        for task in tasks {
            assert!(matches!(task.await.unwrap(), Ok(SolveOutcome::NoChallenge { .. })));
        }
        assert_eq!(gauge.solves.load(Ordering::SeqCst), 3);
        assert_eq!(gauge.max.load(Ordering::SeqCst), CF_MAX_PARALLEL_SOLVES);
    }

    /// A host that times out the hidden solve and clears the visible one. It records every request it is given.
    struct TwoStage {
        visible_supported: bool,
        seen: std::sync::Mutex<Vec<(bool, Duration)>>,
    }

    #[async_trait]
    impl WebViewHost for TwoStage {
        fn available(&self) -> bool {
            true
        }

        fn supports_visible(&self) -> bool {
            self.visible_supported
        }

        async fn get_html(&self) -> Option<String> {
            None
        }

        async fn solve_challenge(&self, req: SolveRequest, _cancel: CancellationToken) -> SolveOutcome {
            self.seen.lock().unwrap().push((req.visible, req.timeout));
            if req.visible {
                cleared()
            } else {
                SolveOutcome::Failed { reason: SolveFailure::Timeout, outdated: false }
            }
        }
    }

    async fn run_job(host: Arc<TwoStage>) -> SolveOutcome {
        let dir = tempfile::tempdir().unwrap();
        let job = SolveJob {
            webview: host,
            jar: CookieJar::load(dir.path().join("cookies.json")),
            url: Url::parse("https://site.test/").unwrap(),
            headers: Vec::new(),
            user_agent: String::new(),
        };
        job.run(CancellationToken::new()).await
    }

    #[tokio::test]
    async fn a_hidden_timeout_is_tried_again_in_the_visible_window_with_a_120_second_deadline() {
        let host = Arc::new(TwoStage { visible_supported: true, seen: Default::default() });
        assert!(matches!(run_job(host.clone()).await, SolveOutcome::Cleared { .. }));
        assert_eq!(*host.seen.lock().unwrap(), [(false, CF_SOLVE_TIMEOUT), (true, Duration::from_secs(120))]);
    }

    #[tokio::test]
    async fn a_host_without_a_visible_window_keeps_the_hidden_timeout() {
        let host = Arc::new(TwoStage { visible_supported: false, seen: Default::default() });
        assert_eq!(
            run_job(host.clone()).await,
            SolveOutcome::Failed { reason: SolveFailure::Timeout, outdated: false }
        );
        assert_eq!(*host.seen.lock().unwrap(), [(false, CF_SOLVE_TIMEOUT)], "no second solve");
    }
}
