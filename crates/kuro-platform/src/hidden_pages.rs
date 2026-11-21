//! Hidden web pages for sites that answer plain clients with a browser check: Cloudflare's ("Just a moment...") or Anubis's
//! proof of work ("Making sure you're not a bot!"). A site's
//! page is opened once, unshown, in the system's web engine, which passes the check by itself, and the plug-ins'
//! requests to that site are then sent with `fetch()` from inside the page: the pass only holds for the browser that
//! earned it, so its cookie cannot be replayed from the app's own client. One page per site, closed after
//! [`IDLE_CLOSE`] unused; its cookies stay in [`Paths::web_pages_data_dir`].
//!
//! The engine is per platform: WebKitGTK on Linux (`linux::hidden_pages`), the system's Edge WebView2 on Windows
//! (`windows::hidden_pages`). Elsewhere there is none yet, so [`hidden_pages`] gives `None` and such sites answer with their
//! check. The request and answer shapes, the page script and the rules below are shared by every engine.

//!
//! A plug-in may also have a page of its site **read** ([`HiddenPages::read`], [`PageRead`]): the page is opened in the
//! same engine, the app's own script ([`READ_JS`]) waits, scrolls and hands back the HTML it finds. That is for sites that
//! only fill their pages from a script of their own. The plug-in names the address and picks steps from a fixed set
//! ([`Step`]); it never passes a script. Such a page asks only the hosts of the plug-in's manifest and loads no image, media
//! or font ([`page_may_ask`], [`content_rules`]), and a check that does not clear by itself fails the read: nothing here
//! answers one.
//!
//! Anyone may write a plug-in, so the pages reach the internet only: every connection of every page goes through a gate
//! (`kuro_net::page_gate`, the address [`hidden_pages`] is given) that looks the name up itself and refuses this computer
//! and the local network.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::paths::Paths;

/// Sends plug-in requests from a site's hidden page, and reads a page for a plug-in.
pub trait HiddenPages: Send + Sync {
    /// Sends `request_json` (`{method, url, headers, body}`, already allow-listed) and returns
    /// `{status, headers, url, body, bodyEncoding?}`. Called on a worker thread; blocks until the page answers.
    fn fetch(&self, request_json: &str) -> Result<String, String>;

    /// Opens `read.url` in a page kept for reading, runs its steps there and returns `{url, title, reads}`: the address
    /// and title the page ended on and the HTML each [`Step::Read`] found. Called on a worker thread; blocks until the
    /// page answers or the read's time is up. An engine without it refuses.
    fn read(&self, read: &PageRead) -> Result<String, String> {
        let _ = read;
        Err(NO_READER.to_owned())
    }
}

/// The error of a read where no engine can open pages.
pub const NO_READER: &str = "this build of the app cannot read web pages for a source";

/// The hidden pages of this platform, or `None` where it has no web engine for them. `gate` is the proxy every page's
/// connections go through (`kuro_net::page_gate`); without one the pages connect by themselves, which only tests do.
#[allow(unused_variables, unreachable_code)]
pub fn hidden_pages(paths: &dyn Paths, gate: Option<SocketAddr>) -> Option<Arc<dyn HiddenPages>> {
    #[cfg(all(target_os = "linux", feature = "platform-linux"))]
    return Some(Arc::new(crate::linux::hidden_pages::WebKitPages::new(paths.web_pages_data_dir(), paths.web_pages_cache_dir()).through(gate)));
    // WebView2 keeps its cache inside its data folder
    #[cfg(all(target_os = "windows", feature = "platform-windows"))]
    return Some(Arc::new(crate::windows::hidden_pages::WebView2Pages::new(paths.web_pages_data_dir()).through(gate)));
    None
}

/// How long a site's check may take to clear in the hidden page: less than the 30 s the app waits for a source, so a stuck
/// check is reported (and the page dropped) before the person gives up.
pub const READY_TIMEOUT: Duration = Duration::from_secs(25);
/// How long a plug-in waits for one answer, the check included.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(90);
/// A site's hidden page is closed after this long unused: each holds a web process. Its cookies stay, so reopening it
/// rarely meets the check again.
pub const IDLE_CLOSE: Duration = Duration::from_secs(10 * 60);

/// Runs in the page: sends the plug-in's request (`req`, its JSON) with the page's own `fetch` and returns the plug-in's
/// response shape. A Referer cannot be set as a header from a page, so it becomes the request's referrer.
pub const FETCH_JS: &str = r#"
const q = JSON.parse(req);
const headers = {};
let referrer;
for (const [name, value] of (q.headers || [])) {
  if (name.toLowerCase() === 'referer') referrer = value; else headers[name] = value;
}
const init = { method: q.method || 'GET', headers, credentials: 'include' };
if (referrer) init.referrer = referrer;
if (q.body != null) init.body = q.body;
const r = await fetch(q.url, init);
const bytes = new Uint8Array(await r.arrayBuffer());
let body, bodyEncoding;
try {
  body = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
} catch (e) {
  let s = '';
  for (let i = 0; i < bytes.length; i += 32768) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 32768));
  body = btoa(s);
  bodyEncoding = 'base64';
}
return JSON.stringify({ status: r.status, headers: [...r.headers.entries()], url: r.url, body, bodyEncoding });
"#;

#[derive(Deserialize)]
struct Target {
    url: String,
}

/// The address a request goes to.
pub fn url_of(request_json: &str) -> Result<String, String> {
    let target: Target = serde_json::from_str(request_json).map_err(|e| format!("invalid request: {e}"))?;
    url::Url::parse(&target.url).map(|u| u.to_string()).map_err(|e| format!("invalid url: {e}"))
}

/// The site a request goes to: its scheme, host and port.
pub fn origin_of(request_json: &str) -> Result<String, String> {
    let url = url::Url::parse(&url_of(request_json)?).map_err(|e| format!("invalid url: {e}"))?;
    Ok(url.origin().ascii_serialization())
}

/// The start of the cookie name Anubis (a proof-of-work bot check) sets while it shows its check page.
pub const ANUBIS_COOKIE_PREFIX: &str = "techaro.lol-anubis-cookie-verification";
/// A mark of Anubis's check page in its HTML: the element that carries the proof-of-work task.
pub const ANUBIS_PAGE_MARK: &str = r#"id="anubis_challenge""#;

/// Whether the page's answer is still the browser check (the pass ran out): the page is reloaded once and asked again.
/// Cloudflare's check is a 403 or 503 with `cf-mitigated: challenge`. Anubis's is its check page, at any status: a page's
/// `fetch` never shows `set-cookie` to a script, so the page's HTML tells it, and the cookie only where an engine passes it.
pub fn still_challenged(answer: &str) -> bool {
    #[derive(Deserialize)]
    struct Answer {
        status: u16,
        headers: Vec<(String, String)>,
        #[serde(default)]
        body: String,
    }
    serde_json::from_str::<Answer>(answer).is_ok_and(|a| {
        let header = |name: &str, test: &dyn Fn(&str) -> bool| a.headers.iter().any(|(n, v)| n.eq_ignore_ascii_case(name) && test(v));
        let cloudflare = matches!(a.status, 403 | 503) && header("cf-mitigated", &|v| v.eq_ignore_ascii_case("challenge"));
        let anubis = header("set-cookie", &|v| v.trim_start().starts_with(ANUBIS_COOKIE_PREFIX)) || (header("content-type", &|v| v.to_ascii_lowercase().starts_with("text/html")) && a.body.contains(ANUBIS_PAGE_MARK));
        cloudflare || anubis
    })
}

/// The language the hidden pages browse in, whatever the system's. Cloudflare's check retitles its page in the browser's
/// language ("Un instant…" on a French system), and [`is_check_page`] knows the English titles only: a page in another
/// language passed for cleared while the check still ran, and the site answered its requests with 403.
pub const PAGE_LANGUAGE: &str = "en-US";

/// Whether a page titled `title` is a browser check rather than the site: Cloudflare's ("Just a moment...") or Anubis's
/// ("Making sure you're not a bot!"). The pages browse in [`PAGE_LANGUAGE`].
pub fn is_check_page(title: &str) -> bool {
    let title = title.to_lowercase();
    title.contains("just a moment") || (title.contains("making sure you") && title.contains("not a bot"))
}

/// How long a read's steps may take when the plug-in names no time.
pub const READ_TIMEOUT: Duration = Duration::from_secs(20);
/// The longest a plug-in may ask for.
pub const MAX_READ_TIMEOUT: Duration = Duration::from_secs(60);
/// The most steps one read may carry.
pub const MAX_STEPS: usize = 32;
/// The longest selector of a step, in bytes.
pub const MAX_SELECTOR: usize = 512;
/// The most HTML one read hands back, in UTF-16 units (a page's script counts those).
pub const MAX_READ_CHARS: usize = 4 * 1024 * 1024;
/// How long [`Step::Scroll`] rests on each element, so the site's script sees it in view.
pub const SCROLL_PAUSE: Duration = Duration::from_millis(150);

/// One thing the app's script does in a page that is read. Each takes a CSS selector. The JSON form is an object with one
/// key: `{"wait": "ul.list > li"}`, `{"waitGone": ".is-loading"}`, `{"scroll": "div.page"}`, `{"read": "html"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Step {
    /// Waits until an element matches.
    Wait(String),
    /// Waits until no element matches.
    WaitGone(String),
    /// Brings each matching element into view in turn, as a reader who scrolls down would.
    Scroll(String),
    /// Hands back the HTML of every matching element (`html` matches the whole page).
    Read(String),
}

impl Step {
    fn selector(&self) -> &str {
        match self {
            Step::Wait(selector) | Step::WaitGone(selector) | Step::Scroll(selector) | Step::Read(selector) => selector,
        }
    }
}

/// A page to read for a plug-in: its address, the hosts the page may ask (the plug-in's manifest list), the steps and
/// the time they may take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRead {
    pub url: String,
    pub hosts: Vec<String>,
    pub steps: Vec<Step>,
    pub timeout: Duration,
}

/// The read as a plug-in asks for it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadRequest {
    url: String,
    steps: Vec<Step>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

impl PageRead {
    /// The read a plug-in asks for with `request_json` (`{url, steps, timeoutMs?}`), or why it is refused: the address
    /// must be https and on one of `hosts`, with 1 to [`MAX_STEPS`] steps. The time is kept between one second and
    /// [`MAX_READ_TIMEOUT`].
    pub fn parse(request_json: &str, hosts: &[String]) -> Result<Self, String> {
        let request: ReadRequest = serde_json::from_str(request_json).map_err(|e| format!("invalid page read: {e}"))?;
        let url = url::Url::parse(&request.url).map_err(|e| format!("invalid url: {e}"))?;
        if url.scheme() != "https" {
            return Err("only https pages can be read".to_owned());
        }
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        if !host_allowed(hosts, &host) {
            return Err(format!("host not allowed: {}", if host.is_empty() { "(none)" } else { &host }));
        }
        if request.steps.is_empty() || request.steps.len() > MAX_STEPS {
            return Err(format!("a page read takes 1 to {MAX_STEPS} steps"));
        }
        if let Some(step) = request.steps.iter().find(|step| step.selector().trim().is_empty() || step.selector().len() > MAX_SELECTOR) {
            return Err(format!("a step's selector must be 1 to {MAX_SELECTOR} bytes: {:?}", step.selector().chars().take(40).collect::<String>()));
        }
        let timeout = request.timeout_ms.map_or(READ_TIMEOUT, Duration::from_millis).clamp(Duration::from_secs(1), MAX_READ_TIMEOUT);
        Ok(Self { url: url.to_string(), hosts: hosts.to_vec(), steps: request.steps, timeout })
    }

    /// The site the address is on: its scheme, host and port.
    pub fn origin(&self) -> Result<String, String> {
        url::Url::parse(&self.url).map(|u| u.origin().ascii_serialization()).map_err(|e| format!("invalid url: {e}"))
    }

    /// What names the page this read runs in: one page per site and host list, so two plug-ins never share one.
    pub fn page_key(&self) -> Result<String, String> {
        Ok(format!("read {} {}", self.origin()?, self.hosts.join(",").to_ascii_lowercase()))
    }

    /// What [`READ_JS`] takes as `req`.
    pub fn script_request(&self) -> String {
        serde_json::json!({
            "steps": self.steps,
            "timeoutMs": self.timeout.as_millis() as u64,
            "pauseMs": SCROLL_PAUSE.as_millis() as u64,
            "maxChars": MAX_READ_CHARS,
        })
        .to_string()
    }

    /// How long the caller waits for the answer: the page's making, the site's check and the steps.
    pub fn answer_timeout(&self) -> Duration {
        ANSWER_TIMEOUT + self.timeout
    }
}

/// Runs in a page that is read: does the steps of `req` (`{steps, timeoutMs, pauseMs, maxChars}`) in order and returns
/// `{url, title, reads}`. A wait that is not met in time, or a page too large, throws with the page's title and address,
/// so the plug-in can tell a page that changed from a check page. It reads and scrolls the page's own document only: it
/// calls nothing of the site, clicks nothing and never looks inside a frame.
pub const READ_JS: &str = r#"
const q = JSON.parse(req);
const deadline = Date.now() + q.timeoutMs;
const sleep = (ms) => new Promise((done) => setTimeout(done, ms));
const where = () => ' (the page is "' + document.title + '" at ' + location.href + ')';
const until = async (met, what) => {
  while (!met()) {
    if (Date.now() >= deadline) throw new Error(what + where());
    await sleep(100);
  }
};
const reads = [];
let chars = 0;
for (const step of q.steps) {
  if (step.wait !== undefined) {
    await until(() => document.querySelector(step.wait) !== null, 'nothing matched ' + step.wait + ' in time');
  } else if (step.waitGone !== undefined) {
    await until(() => document.querySelector(step.waitGone) === null, step.waitGone + ' still matched when the time ran out');
  } else if (step.scroll !== undefined) {
    for (const element of document.querySelectorAll(step.scroll)) {
      if (Date.now() >= deadline) throw new Error('the page was not scrolled through in time' + where());
      element.scrollIntoView({ block: 'center' });
      await sleep(q.pauseMs);
    }
  } else if (step.read !== undefined) {
    const html = Array.from(document.querySelectorAll(step.read), (element) => element.outerHTML).join('\n');
    chars += html.length;
    if (chars > q.maxChars) throw new Error('the page is too large to read' + where());
    reads.push(html);
  }
}
return JSON.stringify({ url: location.href, title: document.title, reads });
"#;

/// Whether `host` is `entry`: a host name matched exactly, or `*.<suffix>` matching any subdomain of the suffix (not the
/// bare suffix). The rule of a plug-in's manifest `hosts`; case does not matter.
pub fn host_matches(entry: &str, host: &str) -> bool {
    let (entry, host) = (entry.to_ascii_lowercase(), host.to_ascii_lowercase());
    match entry.strip_prefix("*.") {
        Some(suffix) => host.ends_with(&format!(".{suffix}")),
        None => host == entry,
    }
}

/// Whether `host` is on the list `hosts`. An empty host never is.
pub fn host_allowed(hosts: &[String], host: &str) -> bool {
    !host.is_empty() && hosts.iter().any(|entry| host_matches(entry, host))
}

/// Whether a page that is read may ask `url`: a web address (http, https or a web socket) on one of `hosts`, with no
/// user name in it, or something the page made for itself that never leaves it (`about:`, `data:`, `blob:`). An engine
/// that sees each request in Rust asks this.
pub fn page_may_ask(hosts: &[String], url: &str) -> bool {
    let Ok(url) = url::Url::parse(url) else { return false };
    match url.scheme() {
        "http" | "https" | "ws" | "wss" => url.username().is_empty() && url.password().is_none() && host_allowed(hosts, url.host_str().unwrap_or_default()),
        "about" | "data" | "blob" => true,
        _ => false,
    }
}

/// The same rule as WebKit content-filter rules (JSON), for the engine that filters by rule list: every web address is
/// blocked, the ones on `hosts` are let through again, then an address with a user name, and every image, media file and
/// font, is blocked whatever its host. An entry that is not a plain host name lets nothing through.
pub fn content_rules(hosts: &[String]) -> String {
    let mut rules = vec![serde_json::json!({ "trigger": { "url-filter": "^[htpsw]+://" }, "action": { "type": "block" } })];
    for entry in hosts {
        let entry = entry.to_ascii_lowercase();
        let (any_subdomain, name) = match entry.strip_prefix("*.") {
            Some(suffix) => (true, suffix),
            None => (false, entry.as_str()),
        };
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
            continue;
        }
        let name = name.replace('.', "\\.");
        let filter = if any_subdomain { format!("^[htpsw]+://[^:/@]+\\.{name}[:/]") } else { format!("^[htpsw]+://{name}[:/]") };
        rules.push(serde_json::json!({ "trigger": { "url-filter": filter }, "action": { "type": "ignore-previous-rules" } }));
    }
    rules.push(serde_json::json!({ "trigger": { "url-filter": "^[^:]+://[^/]*@" }, "action": { "type": "block" } }));
    rules.push(serde_json::json!({ "trigger": { "url-filter": ".*", "resource-type": ["image", "media", "font"] }, "action": { "type": "block" } }));
    serde_json::Value::Array(rules).to_string()
}

/// The sites whose pages have been unused for `idle` or more at `now`.
pub fn idle_sites(used: &HashMap<String, Instant>, now: Instant, idle: Duration) -> Vec<String> {
    used.iter().filter(|(_, at)| now.duration_since(**at) >= idle).map(|(site, _)| site.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_goes_to_the_page_of_its_site() {
        assert_eq!(origin_of(r#"{"url":"https://animepahe.pw/api?m=airing"}"#).unwrap(), "https://animepahe.pw");
        assert_eq!(origin_of(r#"{"url":"http://a.test:8080/x"}"#).unwrap(), "http://a.test:8080");
        assert!(origin_of("nope").is_err());
    }

    #[test]
    fn only_a_check_answer_reloads_the_page() {
        assert!(still_challenged(r#"{"status":403,"headers":[["cf-mitigated","challenge"]],"url":"u","body":""}"#));
        assert!(!still_challenged(r#"{"status":200,"headers":[],"url":"u","body":"{}"}"#));
        assert!(!still_challenged(r#"{"status":403,"headers":[],"url":"u","body":""}"#));
        assert!(is_check_page("Just a moment...") && !is_check_page("animepahe"));
    }

    #[test]
    fn an_anubis_check_answer_reloads_the_page_at_any_status() {
        // the page as inkstory.net served it on 2026-11-21 (Anubis v1.28.2), cut down to the mark
        let page = r#"<!doctype html><html><head><title>Making sure you&#39;re not a bot!</title></head><body><script id="anubis_challenge" type="application/json">{}</script></body></html>"#;
        let answer = |status: u16, headers: &str| serde_json::json!({ "status": status, "headers": serde_json::from_str::<serde_json::Value>(headers).unwrap(), "url": "u", "body": page }).to_string();
        assert!(still_challenged(&answer(200, r#"[["content-type","text/html; charset=utf-8"]]"#)), "a page's fetch never shows set-cookie: the HTML tells");
        assert!(still_challenged(&answer(401, r#"[["Content-Type","TEXT/HTML"]]"#)));
        assert!(still_challenged(r#"{"status":200,"headers":[["set-cookie","techaro.lol-anubis-cookie-verification-347ddb4a=01a1; Path=/"]],"url":"u","body":""}"#));
        assert!(!still_challenged(&answer(200, r#"[["content-type","application/json"]]"#)), "a JSON answer that quotes the mark is not the check");
        assert!(!still_challenged(r#"{"status":200,"headers":[["content-type","text/html"],["set-cookie","session=1"]],"url":"u","body":"<title>InkStory</title>"}"#));
        assert!(is_check_page("Making sure you're not a bot!") && is_check_page("Making sure you’re not a bot!"));
        assert!(!is_check_page("InkStory") && !is_check_page("Not a bot? Read manga"));
    }

    fn hosts(list: &[&str]) -> Vec<String> {
        list.iter().map(|h| (*h).to_owned()).collect()
    }

    #[test]
    fn a_read_is_an_https_page_of_a_listed_host_with_steps_from_the_fixed_set() {
        let allowed = hosts(&["comix.test", "*.cdn.test"]);
        let read = PageRead::parse(
            r#"{"url":"https://comix.test/title/1?page=2","steps":[{"wait":"ul > li"},{"scroll":"div.page"},{"waitGone":"div.page:not(:has(img))"},{"read":"html"}],"timeoutMs":30000}"#,
            &allowed,
        )
        .unwrap();
        assert_eq!(read.url, "https://comix.test/title/1?page=2");
        assert_eq!(read.steps, [Step::Wait("ul > li".into()), Step::Scroll("div.page".into()), Step::WaitGone("div.page:not(:has(img))".into()), Step::Read("html".into())]);
        assert_eq!(read.timeout, Duration::from_secs(30));
        assert_eq!(read.hosts, allowed);
        assert_eq!(read.origin().unwrap(), "https://comix.test");
        assert_eq!(read.page_key().unwrap(), "read https://comix.test comix.test,*.cdn.test");

        let refusal = |json: &str| PageRead::parse(json, &allowed).unwrap_err();
        assert_eq!(refusal(r#"{"url":"https://other.test/","steps":[{"read":"html"}]}"#), "host not allowed: other.test");
        assert_eq!(refusal(r#"{"url":"http://comix.test/","steps":[{"read":"html"}]}"#), "only https pages can be read");
        assert_eq!(refusal(r#"{"url":"file:///etc/passwd","steps":[{"read":"html"}]}"#), "only https pages can be read");
        assert!(refusal(r#"{"url":"https://comix.test/","steps":[]}"#).contains("1 to 32 steps"));
        assert!(refusal(r#"{"url":"https://comix.test/","steps":[{"read":" "}]}"#).contains("selector"));
        assert!(refusal(r#"{"url":"https://comix.test/","steps":[{"click":"a"}]}"#).starts_with("invalid page read"), "a step outside the set is refused");
        assert!(refusal(r#"{"url":"https://comix.test/","steps":[{"script":"fetch('/x')"}]}"#).starts_with("invalid page read"), "a plug-in passes no script");
        assert!(refusal("nope").starts_with("invalid page read"));
    }

    #[test]
    fn a_reads_time_is_kept_within_its_limits() {
        let allowed = hosts(&["a.test"]);
        let timeout = |ms: &str| PageRead::parse(&format!(r#"{{"url":"https://a.test/","steps":[{{"read":"html"}}]{ms}}}"#), &allowed).unwrap().timeout;
        assert_eq!(timeout(""), READ_TIMEOUT);
        assert_eq!(timeout(r#","timeoutMs":5"#), Duration::from_secs(1));
        assert_eq!(timeout(r#","timeoutMs":3600000"#), MAX_READ_TIMEOUT);
    }

    #[test]
    fn the_page_script_gets_the_steps_as_the_plug_in_wrote_them() {
        let read = PageRead { url: "https://a.test/".into(), hosts: hosts(&["a.test"]), steps: vec![Step::WaitGone(".busy".into()), Step::Read("main".into())], timeout: Duration::from_secs(7) };
        let request: serde_json::Value = serde_json::from_str(&read.script_request()).unwrap();
        assert_eq!(request["steps"], serde_json::json!([{ "waitGone": ".busy" }, { "read": "main" }]));
        assert_eq!(request["timeoutMs"], 7000);
        assert_eq!(request["pauseMs"], 150);
        assert_eq!(request["maxChars"], MAX_READ_CHARS);
        // the script knows every step of the set by the name it has in JSON
        for name in ["step.wait", "step.waitGone", "step.scroll", "step.read"] {
            assert!(READ_JS.contains(name), "{name}");
        }
        assert!(read.answer_timeout() > ANSWER_TIMEOUT);
    }

    #[test]
    fn a_read_page_asks_only_the_listed_hosts() {
        let allowed = hosts(&["comix.test", "*.cdn.test"]);
        for ok in ["https://comix.test/api/v1/x?y", "https://COMIX.test:8443/", "wss://comix.test/live", "https://a.b.cdn.test/s.js", "about:blank", "blob:https://comix.test/1b2c", "data:text/css,a{}"] {
            assert!(page_may_ask(&allowed, ok), "{ok}");
        }
        for refused in [
            "https://evil.test/",
            "https://cdn.test/",
            "https://comix.test.evil.test/",
            "https://comix.test:pw@evil.test/",
            "https://user@comix.test/",
            "http://192.168.1.1/",
            "file:///C:/Users/me/secret.txt",
            "ftp://comix.test/",
            "not an address",
        ] {
            assert!(!page_may_ask(&allowed, refused), "{refused}");
        }
        assert!(!page_may_ask(&[], "https://comix.test/"), "an empty list lets no site through");
        assert!(host_matches("*.cdn.test", "A.CDN.test") && !host_matches("*.cdn.test", "cdn.test") && !host_matches("cdn.test", "evilcdn.test"));
    }

    #[test]
    fn the_rule_list_blocks_everything_then_lets_the_listed_hosts_through() {
        let rules: serde_json::Value = serde_json::from_str(&content_rules(&hosts(&["Comix.test", "*.cdn.test", "bad host/", "*."]))).unwrap();
        let rules = rules.as_array().unwrap();
        let filter = |i: usize| rules[i]["trigger"]["url-filter"].as_str().unwrap().to_owned();
        let action = |i: usize| rules[i]["action"]["type"].as_str().unwrap().to_owned();
        assert_eq!(rules.len(), 5, "the two entries that are not host names let nothing through");
        assert_eq!((filter(0), action(0)), ("^[htpsw]+://".to_owned(), "block".to_owned()));
        assert_eq!((filter(1), action(1)), (r"^[htpsw]+://comix\.test[:/]".to_owned(), "ignore-previous-rules".to_owned()));
        assert_eq!((filter(2), action(2)), (r"^[htpsw]+://[^:/@]+\.cdn\.test[:/]".to_owned(), "ignore-previous-rules".to_owned()));
        assert_eq!((filter(3), action(3)), ("^[^:]+://[^/]*@".to_owned(), "block".to_owned()), "a user name in the address is blocked after the hosts are let through");
        assert_eq!(rules[4]["trigger"]["resource-type"], serde_json::json!(["image", "media", "font"]));
        assert_eq!(action(4), "block");
    }

    #[test]
    fn only_pages_unused_long_enough_are_closed() {
        let now = Instant::now();
        let used = HashMap::from([("https://old.test".to_owned(), now - Duration::from_secs(700)), ("https://new.test".to_owned(), now)]);
        assert_eq!(idle_sites(&used, now, IDLE_CLOSE), ["https://old.test"]);
    }
}
