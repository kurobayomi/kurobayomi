//! Hidden web pages for sites that answer plain clients with Cloudflare's browser check ("Just a moment..."). A site's
//! page is opened once, unshown, in the system's web engine, which passes the check by itself, and the plug-ins'
//! requests to that site are then sent with `fetch()` from inside the page: the pass only holds for the browser that
//! earned it, so its cookie cannot be replayed from the app's own client. One page per site, closed after
//! [`IDLE_CLOSE`] unused; its cookies stay in [`Paths::web_pages_data_dir`].
//!
//! The engine is per platform: WebKitGTK on Linux (`linux::hidden_pages`), the system's Edge WebView2 on Windows
//! (`windows::hidden_pages`). Elsewhere there is none yet, so [`hidden_pages`] gives `None` and such sites answer with their
//! check. The request and answer shapes, the page script and the rules below are shared by every engine.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::paths::Paths;

/// Sends plug-in requests from a site's hidden page.
pub trait HiddenPages: Send + Sync {
    /// Sends `request_json` (`{method, url, headers, body}`, already allow-listed) and returns
    /// `{status, headers, url, body, bodyEncoding?}`. Called on a worker thread; blocks until the page answers.
    fn fetch(&self, request_json: &str) -> Result<String, String>;
}

/// The hidden pages of this platform, or `None` where it has no web engine for them.
#[allow(unused_variables, unreachable_code)]
pub fn hidden_pages(paths: &dyn Paths) -> Option<Arc<dyn HiddenPages>> {
    #[cfg(all(target_os = "linux", feature = "platform-linux"))]
    return Some(Arc::new(crate::linux::hidden_pages::WebKitPages::new(paths.web_pages_data_dir(), paths.web_pages_cache_dir())));
    // WebView2 keeps its cache inside its data folder
    #[cfg(all(target_os = "windows", feature = "platform-windows"))]
    return Some(Arc::new(crate::windows::hidden_pages::WebView2Pages::new(paths.web_pages_data_dir())));
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

/// Whether the page's answer is still the browser check (the pass ran out): the page is reloaded once and asked again.
pub fn still_challenged(answer: &str) -> bool {
    #[derive(Deserialize)]
    struct Answer {
        status: u16,
        headers: Vec<(String, String)>,
    }
    serde_json::from_str::<Answer>(answer).is_ok_and(|a| {
        matches!(a.status, 403 | 503) && a.headers.iter().any(|(n, v)| n.eq_ignore_ascii_case("cf-mitigated") && v.eq_ignore_ascii_case("challenge"))
    })
}

/// Whether a page titled `title` is the check rather than the site.
pub fn is_check_page(title: &str) -> bool {
    title.to_lowercase().contains("just a moment")
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
    fn only_pages_unused_long_enough_are_closed() {
        let now = Instant::now();
        let used = HashMap::from([("https://old.test".to_owned(), now - Duration::from_secs(700)), ("https://new.test".to_owned(), now)]);
        assert_eq!(idle_sites(&used, now, IDLE_CLOSE), ["https://old.test"]);
    }
}
