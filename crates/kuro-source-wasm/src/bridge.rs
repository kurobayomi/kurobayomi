//! Host imports for guests: `http`, `pref-get`, `pref-set`, `log` and the allow-list check (area 18), and the `pages`
//! import a guest reads a web page through ([`HostState::page_read`], [`link_pages`]).
//!
//! The guest's `http` import is served by the app's `NetworkClient`, so cookies, user agent and rate limits are the
//! same as for built-in sources. The import is synchronous: it runs on the blocking thread of the guest call and
//! waits on the runtime handle, so it must never run on an async worker.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::prelude::BASE64_STANDARD;
use bytes::Bytes;
use http::header::CONTENT_TYPE;
use http::{HeaderMap, HeaderName, HeaderValue};
use kuro_net::request::{self, Body};
use kuro_net::{NetworkClient, Request};
use kuro_platform::hidden_pages::{NO_READER, PageRead, host_matches};
use kuro_prefs::store::PrefStore;
use kuro_prefs::values::PrefValue;
use kuro_source::prefs::preference_key;
use serde::{Deserialize, Serialize};
use url::Url;
use wasmtime::StoreLimits;
use wasmtime::component::Linker;

use crate::bindings::kuro::source::host::Host;
use crate::limits::CallClock;

/// The state of one guest store: who the source is, what it may reach, and its memory limit.
pub struct HostState {
    pub source_id: i64,
    /// The manifest `hosts` allow-list. Empty means no network.
    pub hosts: Vec<String>,
    pub client: NetworkClient,
    pub prefs: PrefStore,
    pub limits: StoreLimits,
    /// The least gap between two requests to one host, from the source's info (ABI 1.5). `None` spaces nothing.
    pub request_gap: Option<Duration>,
    /// The clock of the current guest call. Host waits are booked here, so they do not count against its deadline.
    pub clock: CallClock,
}

/// The send time booked last for each host, across every source of the process: the host spacing reads it.
static LAST_SENT: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Mutex::default);

/// Books the next send to `host` at `gap` or more after the last one booked for it, and returns how long to wait for
/// that slot. A send with no gap is booked at `now`, so it waits only behind an earlier booking.
fn book_send(sent: &mut HashMap<String, Instant>, host: &str, gap: Duration, now: Instant) -> Duration {
    let slot = sent.get(host).map_or(now, |last| (*last + gap).max(now));
    sent.insert(host.to_owned(), slot);
    slot.duration_since(now)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HttpRequest {
    method: String,
    url: String,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default)]
    body: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    /// The final URL, after any redirects (ABI 1.5).
    url: String,
    body: String,
    /// Present only when `body` is base64 of a non-UTF-8 payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    body_encoding: Option<&'static str>,
}

/// The allow-list rule. An entry is a host name matched exactly, or `*.<suffix>` matching any subdomain of the suffix
/// (not the bare suffix). Matching is case-insensitive. A URL without a host never matches.
pub fn check_host(hosts: &[String], url: &Url) -> Result<(), String> {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host.is_empty() {
        return Err("host not allowed: (none)".to_owned());
    }
    if hosts.iter().any(|entry| host_matches(entry, &host)) {
        Ok(())
    } else {
        Err(format!("host not allowed: {host}"))
    }
}

/// Parses the guest's request JSON, applies the allow-list, and builds the client request.
fn parse_request(json: &str, hosts: &[String]) -> Result<Request, String> {
    let req: HttpRequest = serde_json::from_str(json).map_err(|e| format!("invalid request: {e}"))?;
    let url = Url::parse(&req.url).map_err(|e| format!("invalid url: {e}"))?;
    check_host(hosts, &url)?;

    let mut headers = HeaderMap::new();
    for (name, value) in &req.headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| format!("invalid header name: {name}"))?;
        let value = HeaderValue::from_str(value).map_err(|_| format!("invalid header value for {name}"))?;
        headers.append(name, value);
    }
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let body = match req.body {
        None => Body::Empty,
        Some(text) => Body::Bytes {
            content_type,
            data: Bytes::from(text),
        },
    };
    let raw = url.as_str();
    let request = match req.method.to_ascii_uppercase().as_str() {
        "GET" if matches!(body, Body::Empty) => request::get(raw),
        "GET" => return Err("GET cannot carry a body".to_owned()),
        "POST" => request::post(raw, body),
        "PUT" => request::put(raw, body),
        "DELETE" => request::delete(raw, body),
        other => return Err(format!("unsupported method: {other}")),
    };
    Ok(request.map_err(|e| e.to_string())?.headers(headers))
}

/// Runs the request and returns the response JSON. Non-UTF-8 bodies are sent as base64.
///
/// The transport follows redirects, so the allow-list is checked again on the final URL and a response from a host
/// outside it is dropped unread. ponytail: only the final hop is visible here. A per-hop check needs a redirect hook
/// in kuro-net, which this crate cannot add.
async fn send(client: NetworkClient, request: Request, hosts: &[String]) -> Result<String, String> {
    let response = client.execute(request).await.map_err(|e| e.to_string())?;
    check_host(hosts, &response.url)?;
    let status = response.status.as_u16();
    let final_url = response.url.as_str().to_owned();
    let headers = response
        .headers
        .iter()
        .filter_map(|(name, value)| value.to_str().ok().map(|v| (name.as_str().to_owned(), v.to_owned())))
        .collect();
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    let (body, body_encoding) = match std::str::from_utf8(&bytes) {
        Ok(text) => (text.to_owned(), None),
        Err(_) => (BASE64_STANDARD.encode(&bytes), Some("base64")),
    };
    serde_json::to_string(&HttpResponse {
        status,
        headers,
        url: final_url,
        body,
        body_encoding,
    })
    .map_err(|e| e.to_string())
}

impl HostState {
    /// The key a source preference is stored under: the source namespace, then the guest's key.
    fn pref_key(&self, key: &str) -> String {
        format!("{}.{key}", preference_key(self.source_id))
    }

    /// Parses and allow-lists the request, waits for its slot after the last send to its host, then sends it.
    fn send_spaced(&self, request_json: &str) -> Result<String, String> {
        let request = parse_request(request_json, &self.hosts)?;
        let host = request.url.host_str().unwrap_or_default().to_owned();
        let gap = self.request_gap.unwrap_or_default();
        let wait = book_send(&mut LAST_SENT.lock().unwrap_or_else(PoisonError::into_inner), &host, gap, Instant::now());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        let handle = tokio::runtime::Handle::try_current().map_err(|e| format!("no async runtime: {e}"))?;
        // the request runs as a runtime task, not on this blocking thread: when the app quits mid-request the runtime
        // cancels the task, where polling its timers from here would panic (and crash the app on its way out)
        let (client, hosts) = (self.client.clone(), self.hosts.clone());
        let task = handle.spawn(async move { send(client, request, &hosts).await });
        handle.block_on(task).map_err(|e| format!("the request did not finish: {e}"))?
    }

    /// The `read` of the `pages` import (`kuro:source/pages`): has a page of the guest's site read through the app's
    /// hidden pages and returns `{url, title, reads}`. The request is `{url, steps, timeoutMs?}`; the address must be
    /// https and on a host of the manifest's list, which is also all the page itself may ask. The guest names steps
    /// from a fixed set and passes no script (`kuro_platform::hidden_pages::Step`). The wait is host time, and a page
    /// load keeps the source's request gap like any request to that host.
    pub fn page_read(&mut self, request_json: String) -> Result<String, String> {
        let entered = Instant::now();
        let outcome = self.read_page(&request_json);
        self.clock.book_host_wait(entered.elapsed());
        outcome
    }

    fn read_page(&self, request_json: &str) -> Result<String, String> {
        let read = PageRead::parse(request_json, &self.hosts)?;
        let pages = self.client.hidden_pages().ok_or_else(|| NO_READER.to_owned())?;
        let host = Url::parse(&read.url).ok().and_then(|url| url.host_str().map(str::to_owned)).unwrap_or_default();
        let gap = self.request_gap.unwrap_or_default();
        let wait = book_send(&mut LAST_SENT.lock().unwrap_or_else(PoisonError::into_inner), &host, gap, Instant::now());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        // this is the blocking thread of the guest call; the page works on a thread of the web engine
        pages.read(&read)
    }
}

impl Host for HostState {
    fn http(&mut self, request_json: String) -> Result<String, String> {
        let entered = Instant::now();
        let outcome = self.send_spaced(&request_json);
        self.clock.book_host_wait(entered.elapsed());
        outcome
    }

    fn pref_get(&mut self, key: String) -> String {
        match self.prefs.get_raw(&self.pref_key(&key)) {
            Some(PrefValue::Str(value)) => value,
            _ => String::new(),
        }
    }

    fn pref_set(&mut self, key: String, value_json: String) -> Result<(), String> {
        serde_json::from_str::<serde_json::Value>(&value_json).map_err(|e| format!("value is not JSON: {e}"))?;
        self.prefs.set_raw(&self.pref_key(&key), PrefValue::Str(value_json));
        Ok(())
    }

    fn log(&mut self, level: u8, message: String) {
        let source = self.source_id;
        match level {
            0 => tracing::debug!(source, "{message}"),
            2 => tracing::warn!(source, "{message}"),
            3 => tracing::error!(source, "{message}"),
            _ => tracing::info!(source, "{message}"),
        }
    }

    fn unix_seconds(&mut self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |now| now.as_secs())
    }
}

/// Links the ABI 1.4 host instance, so a guest built against 1.4 (it imports `kuro:source/host@1.4.0`) still loads. The
/// four functions are the 1.4 ones of [`Host`]; the clock is 1.5 only.
pub fn link_v14(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    let mut host = linker.instance("kuro:source/host@1.4.0")?;
    host.func_wrap("http", |mut store, (request_json,): (String,)| {
        Ok((store.data_mut().http(request_json),))
    })?;
    host.func_wrap("pref-get", |mut store, (key,): (String,)| {
        Ok((store.data_mut().pref_get(key),))
    })?;
    host.func_wrap("pref-set", |mut store, (key, value_json): (String, String)| {
        Ok((store.data_mut().pref_set(key, value_json),))
    })?;
    host.func_wrap("log", |mut store, (level, message): (u8, String)| {
        store.data_mut().log(level, message);
        Ok(())
    })?;
    Ok(())
}

/// Links the `pages` import (`kuro:source/pages@1.5.0`), which only a guest that reads web pages declares: a guest
/// without it loads as before.
pub fn link_pages(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    let mut pages = linker.instance("kuro:source/pages@1.5.0")?;
    pages.func_wrap("read", |mut store, (request_json,): (String,)| {
        Ok((store.data_mut().page_read(request_json),))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kuro_platform::hidden_pages::{HiddenPages, Step};
    use serde_json::json;

    use super::*;

    /// Hidden pages that answer every read with a fixed text and keep what was asked.
    #[derive(Default)]
    struct FakePages {
        reads: Mutex<Vec<PageRead>>,
    }

    impl HiddenPages for FakePages {
        fn fetch(&self, _request_json: &str) -> Result<String, String> {
            Err("not asked in these tests".to_owned())
        }

        fn read(&self, read: &PageRead) -> Result<String, String> {
            self.reads.lock().unwrap_or_else(PoisonError::into_inner).push(read.clone());
            Ok(r#"{"url":"https://ok.test/final","title":"A page","reads":["<li>1</li>"]}"#.to_owned())
        }
    }

    /// A host state whose client has `pages` as its hidden pages. The directory must outlive the state.
    fn state_with_pages(allowed: &[&str], pages: Arc<FakePages>) -> (HostState, tempfile::TempDir) {
        let (mut host, dir) = state(1, allowed);
        let prefs = kuro_prefs::network::NetworkPrefs::new(kuro_prefs::store::PrefStore::in_memory());
        let (toaster, _feed) = kuro_platform::toast::toast_channel();
        host.client = NetworkClient::with_cookies_and_pages(
            &prefs,
            kuro_net::CookieJar::load(dir.path().join("pages-cookies.json")),
            Arc::new(kuro_platform::webview::NoWebView),
            toaster,
            Some(pages),
        )
        .expect("client");
        (host, dir)
    }

    #[test]
    fn a_page_read_goes_to_the_hidden_pages_with_the_manifest_hosts_as_its_limit() {
        let pages = Arc::new(FakePages::default());
        let (mut host, _dir) = state_with_pages(&["ok.test", "*.cdn.test"], Arc::clone(&pages));
        let request = json!({ "url": "https://ok.test/title/1?page=2", "steps": [{ "wait": "li" }, { "read": "li" }], "timeoutMs": 9000 }).to_string();
        let answer = host.page_read(request).expect("the page was read");
        let answer: serde_json::Value = serde_json::from_str(&answer).expect("answer JSON");
        assert_eq!(answer["reads"][0], "<li>1</li>");
        assert_eq!(answer["url"], "https://ok.test/final");
        let asked = pages.reads.lock().unwrap().clone();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].url, "https://ok.test/title/1?page=2");
        assert_eq!(asked[0].hosts, hosts(&["ok.test", "*.cdn.test"]), "the page may ask what the manifest lists, nothing else");
        assert_eq!(asked[0].steps, [Step::Wait("li".into()), Step::Read("li".into())]);
        assert_eq!(asked[0].timeout, Duration::from_secs(9));
    }

    #[test]
    fn a_page_read_outside_the_manifest_or_with_a_script_never_reaches_a_page() {
        let pages = Arc::new(FakePages::default());
        let (mut host, _dir) = state_with_pages(&["ok.test"], Arc::clone(&pages));
        let refusal = |host: &mut HostState, request: serde_json::Value| host.page_read(request.to_string()).expect_err("refused");
        assert_eq!(refusal(&mut host, json!({ "url": "https://evil.test/", "steps": [{ "read": "html" }] })), "host not allowed: evil.test");
        assert_eq!(refusal(&mut host, json!({ "url": "http://ok.test/", "steps": [{ "read": "html" }] })), "only https pages can be read");
        assert_eq!(refusal(&mut host, json!({ "url": "https://192.168.1.1/", "steps": [{ "read": "html" }] })), "host not allowed: 192.168.1.1");
        assert!(refusal(&mut host, json!({ "url": "https://ok.test/", "steps": [{ "script": "fetch('/api')" }] })).starts_with("invalid page read"));
        assert!(refusal(&mut host, json!({ "url": "https://ok.test/", "steps": [{ "click": "button" }] })).starts_with("invalid page read"));
        assert!(pages.reads.lock().unwrap().is_empty(), "nothing was opened");
    }

    #[test]
    fn a_page_read_without_hidden_pages_says_so_and_its_wait_is_host_time() {
        let (mut host, _dir) = state(1, &["ok.test"]);
        let request = json!({ "url": "https://ok.test/", "steps": [{ "read": "html" }] }).to_string();
        assert_eq!(host.page_read(request).expect_err("no engine"), NO_READER);

        let pages = Arc::new(FakePages::default());
        let (mut spaced, _dir) = state_with_pages(&["gap.test"], pages);
        spaced.request_gap = Some(Duration::from_millis(200));
        let request = json!({ "url": "https://gap.test/", "steps": [{ "read": "html" }] }).to_string();
        spaced.page_read(request.clone()).expect("first read");
        spaced.page_read(request).expect("second read");
        assert!(spaced.clock.host_wait >= Duration::from_millis(200), "a page load keeps the source's gap, and the wait is not guest run time");
    }

    fn hosts(list: &[&str]) -> Vec<String> {
        list.iter().map(|h| (*h).to_owned()).collect()
    }

    #[test]
    fn an_exact_entry_matches_only_that_host() {
        let allowed = hosts(&["api.example.com"]);
        assert!(check_host(&allowed, &Url::parse("https://api.example.com/x").unwrap()).is_ok());
        assert!(check_host(&allowed, &Url::parse("https://API.example.com/x").unwrap()).is_ok());
        assert!(check_host(&allowed, &Url::parse("https://cdn.example.com/x").unwrap()).is_err());
    }

    #[test]
    fn a_wildcard_entry_matches_subdomains_but_not_the_bare_suffix() {
        let allowed = hosts(&["*.example.com"]);
        assert!(check_host(&allowed, &Url::parse("https://a.b.example.com/").unwrap()).is_ok());
        assert!(check_host(&allowed, &Url::parse("https://example.com/").unwrap()).is_err());
        assert!(check_host(&allowed, &Url::parse("https://evilexample.com/").unwrap()).is_err());
    }

    #[test]
    fn the_allow_list_does_not_restrict_the_scheme() {
        let allowed = hosts(&["ok.test"]);
        assert!(check_host(&allowed, &Url::parse("http://ok.test/x").unwrap()).is_ok());
        assert!(check_host(&allowed, &Url::parse("https://ok.test/x").unwrap()).is_ok());
    }

    #[test]
    fn a_disallowed_host_is_named_in_the_error() {
        let err = check_host(&hosts(&["ok.test"]), &Url::parse("https://evil.test/x").unwrap()).unwrap_err();
        assert_eq!(err, "host not allowed: evil.test");
    }

    #[test]
    fn an_empty_allow_list_refuses_every_host() {
        assert!(check_host(&[], &Url::parse("https://ok.test/").unwrap()).is_err());
    }

    /// The refusal text of a request that must not be built.
    fn refusal(json: &str, allowed: &[&str]) -> String {
        match parse_request(json, &hosts(allowed)) {
            Err(text) => text,
            Ok(_) => panic!("the request was built, but it should be refused: {json}"),
        }
    }

    #[test]
    fn a_request_to_a_disallowed_host_is_refused_before_any_network_io() {
        let json = r#"{"method":"GET","url":"https://evil.test/x","headers":[],"body":null}"#;
        assert_eq!(refusal(json, &["ok.test"]), "host not allowed: evil.test");
    }

    #[test]
    fn a_request_with_a_body_on_get_is_refused() {
        let json = r#"{"method":"GET","url":"https://ok.test/x","headers":[],"body":"x"}"#;
        assert_eq!(refusal(json, &["ok.test"]), "GET cannot carry a body");
    }

    #[test]
    fn a_malformed_request_is_an_error_not_a_panic() {
        assert!(refusal("not json", &["ok.test"]).starts_with("invalid request"));
        let json = r#"{"method":"GET","url":"nope","headers":[],"body":null}"#;
        assert!(refusal(json, &["ok.test"]).starts_with("invalid url"));
    }

    #[test]
    fn an_allowed_request_builds_the_method_url_and_headers() {
        let json = r#"{"method":"post","url":"https://ok.test/login","headers":[["X-Token","t"]],"body":"a=1"}"#;
        let request = parse_request(json, &hosts(&["ok.test"])).expect("allowed");
        assert_eq!(request.method.as_str(), "POST");
        assert_eq!(request.url.as_str(), "https://ok.test/login");
        assert_eq!(request.headers.get("x-token").and_then(|v| v.to_str().ok()), Some("t"));
        assert!(
            matches!(request.body, Body::Bytes { ref content_type, .. } if content_type == "application/octet-stream")
        );
    }

    /// A host state for one source, with the shared test environment. The directory must outlive the state.
    fn state(source_id: i64, allowed: &[&str]) -> (HostState, tempfile::TempDir) {
        let (env, dir) = crate::test_guest::env();
        let host = HostState {
            source_id,
            hosts: hosts(allowed),
            client: env.client,
            prefs: env.prefs,
            limits: crate::limits::store_limits(),
            request_gap: None,
            clock: CallClock::start(crate::limits::CALL_DEADLINE_TICKS),
        };
        (host, dir)
    }

    #[test]
    fn a_preference_is_stored_under_its_own_source_only() {
        let (mut first, _dir) = state(5, &[]);
        assert_eq!(first.pref_get("sort".into()), "");
        first
            .pref_set("sort".into(), r#"{"by":"name"}"#.into())
            .expect("valid JSON");
        assert_eq!(first.pref_get("sort".into()), r#"{"by":"name"}"#);

        let mut other = HostState {
            source_id: 6,
            hosts: Vec::new(),
            client: first.client.clone(),
            prefs: first.prefs.clone(),
            limits: crate::limits::store_limits(),
            request_gap: None,
            clock: CallClock::start(crate::limits::CALL_DEADLINE_TICKS),
        };
        assert_eq!(
            other.pref_get("sort".into()),
            "",
            "another source does not see the value"
        );
    }

    #[test]
    fn a_preference_value_must_be_json() {
        let (mut host, _dir) = state(5, &[]);
        assert!(host.pref_set("k".into(), "not json".into()).is_err());
        assert_eq!(host.pref_get("k".into()), "", "a refused write stores nothing");
    }

    /// The app quitting while a source waits on a request: the runtime shuts down, and the call fails instead of
    /// panicking on the blocking thread (which crashed the app on its way out).
    #[test]
    fn a_request_in_flight_when_the_runtime_shuts_down_fails_without_a_panic() {
        // a server that takes the connection and never answers
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
        let url = format!("http://{}/slow", listener.local_addr().expect("address"));
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                std::thread::sleep(Duration::from_secs(10));
                drop(stream);
            }
        });
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("runtime");
        let (sent, outcome) = std::sync::mpsc::channel();
        {
            let _entered = rt.enter();
            let (mut host, dir) = state(1, &["127.0.0.1"]);
            let request = json!({ "method": "GET", "url": url, "headers": [], "body": null }).to_string();
            rt.spawn_blocking(move || {
                let _dir = dir;
                let _ = sent.send(host.http(request));
            });
        }
        std::thread::sleep(Duration::from_millis(300));
        rt.shutdown_timeout(Duration::from_secs(2));
        let result = outcome.recv_timeout(Duration::from_secs(5)).expect("the call returned, it did not panic");
        assert!(result.is_err(), "{result:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_allowed_request_is_sent_and_its_response_comes_back_as_json() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/hello"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .insert_header("x-reply", "yes")
                    .set_body_string("hi"),
            )
            .mount(&server)
            .await;
        let (mut host, _dir) = state(1, &["127.0.0.1"]);
        let request = json!({ "method": "GET", "url": format!("{}/hello", server.uri()), "headers": [["Accept", "text/plain"]], "body": null })
            .to_string();
        let response = tokio::task::spawn_blocking(move || host.http(request))
            .await
            .expect("the call finished")
            .expect("the request was sent");
        let value: serde_json::Value = serde_json::from_str(&response).expect("response JSON");
        assert_eq!(value["status"], 200);
        assert_eq!(value["body"], "hi");
        assert_eq!(value["url"], format!("{}/hello", server.uri()), "the reply names the final url");
        assert!(value.get("bodyEncoding").is_none(), "UTF-8 bodies are sent as text");
        assert!(value["headers"].to_string().contains("x-reply"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_non_utf8_body_is_sent_as_base64_and_an_error_status_is_not_a_host_error() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/bin"))
            .respond_with(wiremock::ResponseTemplate::new(404).set_body_bytes(vec![0xff_u8, 0x00, 0x41]))
            .mount(&server)
            .await;
        let (mut host, _dir) = state(1, &["127.0.0.1"]);
        let request =
            json!({ "method": "GET", "url": format!("{}/bin", server.uri()), "headers": [], "body": null }).to_string();
        let response = tokio::task::spawn_blocking(move || host.http(request))
            .await
            .expect("the call finished")
            .expect("the guest sees the status, the host does not fail");
        let value: serde_json::Value = serde_json::from_str(&response).expect("response JSON");
        assert_eq!(value["status"], 404);
        assert_eq!(value["bodyEncoding"], "base64");
        assert_eq!(value["body"], "/wBB");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_request_to_a_host_outside_the_allow_list_never_reaches_the_network() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let (mut host, _dir) = state(1, &["api.elsewhere.test"]);
        let request = json!({ "method": "GET", "url": server.uri(), "headers": [], "body": null }).to_string();
        let refusal = tokio::task::spawn_blocking(move || host.http(request))
            .await
            .expect("the call finished")
            .expect_err("refused");
        assert_eq!(refusal, "host not allowed: 127.0.0.1");
        server.verify().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_redirect_to_a_host_outside_the_allow_list_is_refused_and_its_body_not_returned() {
        let server = wiremock::MockServer::start().await;
        let port = server.address().port();
        wiremock::Mock::given(wiremock::matchers::path("/go"))
            .respond_with(
                wiremock::ResponseTemplate::new(302).insert_header("location", format!("http://127.0.0.1:{port}/final")),
            )
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::path("/final"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("leaked"))
            .mount(&server)
            .await;
        let (mut host, _dir) = state(1, &["localhost"]);
        let request =
            json!({ "method": "GET", "url": format!("http://localhost:{port}/go"), "headers": [], "body": null })
                .to_string();
        let refusal = tokio::task::spawn_blocking(move || host.http(request))
            .await
            .expect("the call finished")
            .expect_err("the cross-host redirect is refused");
        assert_eq!(refusal, "host not allowed: 127.0.0.1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_redirect_within_the_allow_list_is_followed() {
        let server = wiremock::MockServer::start().await;
        let port = server.address().port();
        wiremock::Mock::given(wiremock::matchers::path("/go"))
            .respond_with(wiremock::ResponseTemplate::new(302).insert_header("location", "/final"))
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::path("/final"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;
        let (mut host, _dir) = state(1, &["localhost"]);
        let request =
            json!({ "method": "GET", "url": format!("http://localhost:{port}/go"), "headers": [], "body": null })
                .to_string();
        let response = tokio::task::spawn_blocking(move || host.http(request))
            .await
            .expect("the call finished")
            .expect("same-host redirect allowed");
        let value: serde_json::Value = serde_json::from_str(&response).expect("response JSON");
        assert_eq!(value["body"], "ok");
        assert_eq!(value["url"], format!("http://localhost:{port}/final"), "the url is the one after the redirect");
    }

    #[test]
    fn a_booked_send_waits_for_the_gap_after_the_last_send_to_the_same_host() {
        let gap = Duration::from_millis(100);
        let start = Instant::now();
        let mut sent = HashMap::new();
        assert_eq!(book_send(&mut sent, "a.test", gap, start), Duration::ZERO, "the first send is not held");
        let ms = Duration::from_millis;
        assert_eq!(book_send(&mut sent, "a.test", gap, start + ms(30)), ms(70), "the second waits out the gap");
        assert_eq!(book_send(&mut sent, "b.test", gap, start + ms(30)), Duration::ZERO, "another host has its own slot");
        assert_eq!(book_send(&mut sent, "a.test", gap, start + ms(1000)), Duration::ZERO, "a late send waits for nothing");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn two_requests_to_one_host_are_spaced_by_the_source_gap() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;
        let (mut host, _dir) = state(1, &["127.0.0.1"]);
        host.request_gap = Some(Duration::from_millis(300));
        let request = json!({ "method": "GET", "url": format!("{}/a", server.uri()), "headers": [], "body": null }).to_string();
        let started = Instant::now();
        tokio::task::spawn_blocking(move || {
            host.http(request.clone()).expect("first request sent");
            host.http(request).expect("second request sent");
            assert!(started.elapsed() >= Duration::from_millis(300), "the second send waits for the gap");
            assert!(host.clock.host_wait >= Duration::from_millis(300), "the spacing wait is host time, not guest run time");
        })
        .await
        .expect("the calls finished");
    }

    #[test]
    fn the_clock_reads_unix_seconds_now() {
        let (mut host, _dir) = state(1, &[]);
        let before = SystemTime::now().duration_since(UNIX_EPOCH).expect("after 1970").as_secs();
        let now = host.unix_seconds();
        let after = SystemTime::now().duration_since(UNIX_EPOCH).expect("after 1970").as_secs();
        assert!(before <= now && now <= after, "{before} <= {now} <= {after}");
    }
}
