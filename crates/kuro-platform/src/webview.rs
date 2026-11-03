//! The WebView port (area 12 section 3.3, area 36). This module holds the trait, the value types and the
//! no-WebView fallback. Nothing here touches GTK or WebKit, so `kuro-net` compiles and tests without a display.
//! The WebKitGTK implementation is a separate module behind a feature (not built yet, see T-12a-017). Without a view the
//! browser fallback in [`browser`] takes over: system browser, loopback redirect, cookie import.

pub mod browser;
pub mod data;
pub mod solve_state;
pub mod view;
pub mod visible;

use std::io;
use std::time::Duration;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use url::Url;

pub use solve_state::{MachineStep, SolveEvent, SolveMachine};

/// Lowest WebKitGTK `(major, minor)` that is not reported as outdated (the Chrome 118 era).
pub const MIN_WEBKITGTK: (u32, u32) = (2, 42);

/// `true` when a WebKitGTK `(major, minor)` is older than [`MIN_WEBKITGTK`].
pub fn is_outdated_version((major, minor): (u32, u32)) -> bool {
    (major, minor) < MIN_WEBKITGTK
}

/// The version line shown in the UI (`WebViewUtil.getVersion`), or the fallback when no version is known.
pub fn version_text(version: Option<(u32, u32, u32)>) -> String {
    match version {
        Some((major, minor, micro)) => format!("WebKitGTK {major}.{minor}.{micro}"),
        None => "WebKitGTK not available".to_owned(),
    }
}

/// `SameSite` of a cookie, as WebKit reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SameSite {
    None,
    Lax,
    Strict,
}

/// A cookie as the WebView sees it. `domain` keeps its leading dot when the cookie is not host-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    /// Unix seconds; `None` for a session cookie.
    pub expires: Option<i64>,
    pub same_site: Option<SameSite>,
}

/// Why a solve did not clear the challenge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SolveFailure {
    Timeout,
    LoadError(String),
    Unavailable,
    Cancelled,
}

/// The end of one solve. `outdated` is read before the view is torn down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SolveOutcome {
    Cleared { cookies: Vec<WebCookie>, outdated: bool },
    NoChallenge { outdated: bool },
    Failed { reason: SolveFailure, outdated: bool },
}

/// One challenge to solve. `headers` are already filtered for WebKit; `cookie_names` lists the clearance cookies.
#[derive(Clone, Debug)]
pub struct SolveRequest {
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub user_agent: String,
    pub cookies: Vec<WebCookie>,
    pub old_clearance: Option<String>,
    pub timeout: Duration,
    pub cookie_names: &'static [&'static str],
    /// Solve in the visible challenge window (T-12a-032, see [`visible`]); the hidden solve is `false`.
    pub visible: bool,
}

/// The WebView capability the network stack needs. Implementations run WebKit on the GTK main thread.
#[async_trait]
pub trait WebViewHost: Send + Sync {
    /// Whether a view can be created now (feature compiled, WebKit loadable, a display exists).
    fn available(&self) -> bool;

    /// The WebKitGTK version, or `None` when it cannot be read.
    fn version(&self) -> Option<(u32, u32, u32)> {
        None
    }

    /// Whether the host can show the visible challenge window. The Cloudflare step asks for it after a hidden solve
    /// times out (T-12a-032).
    fn supports_visible(&self) -> bool {
        false
    }

    fn version_string(&self) -> String {
        version_text(self.version())
    }

    fn is_outdated(&self) -> bool {
        self.version()
            .is_some_and(|(major, minor, _)| is_outdated_version((major, minor)))
    }

    /// Runs one solve to its end. Cancelling `cancel` finishes it with `Failed(Cancelled)`.
    async fn solve_challenge(&self, req: SolveRequest, cancel: CancellationToken) -> SolveOutcome;

    /// The page's `document.documentElement.outerHTML` as plain text, or `None` when the view has no page. The host
    /// reads the JavaScript string value, so the text is not JSON-quoted (Android's `getHtml` returns it quoted).
    async fn get_html(&self) -> Option<String>;

    /// Runs an OAuth login and returns the query pairs of the redirect. `build_url` gets the redirect URI to put in the
    /// login URL. The default is for hosts with no login path, so the caller falls back to its own error.
    async fn oauth_login(
        &self,
        build_url: Box<dyn for<'r> FnOnce(&'r str) -> Url + Send>,
        limit: Duration,
    ) -> io::Result<Vec<(String, String)>> {
        let _ = (build_url, limit);
        Err(io::Error::new(io::ErrorKind::Unsupported, "this host has no login path"))
    }

    /// The cookies the user imported for `url`'s host. The default has none.
    fn imported_cookies(&self, _url: &Url) -> Vec<WebCookie> {
        Vec::new()
    }
}

/// The host used when WebKitGTK is not compiled in or cannot start.
pub struct NoWebView;

#[async_trait]
impl WebViewHost for NoWebView {
    fn available(&self) -> bool {
        false
    }

    async fn get_html(&self) -> Option<String> {
        None
    }

    async fn solve_challenge(&self, _req: SolveRequest, _cancel: CancellationToken) -> SolveOutcome {
        SolveOutcome::Failed {
            reason: SolveFailure::Unavailable,
            outdated: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_text_names_the_version_or_the_fallback() {
        assert_eq!(version_text(Some((2, 54, 1))), "WebKitGTK 2.54.1");
        assert_eq!(version_text(None), "WebKitGTK not available");
    }

    #[test]
    fn two_forty_is_outdated_and_two_forty_two_is_not() {
        assert!(is_outdated_version((2, 40)));
        assert!(!is_outdated_version((2, 42)));
        assert!(!is_outdated_version((2, 54)));
    }

    /// A host that reports only a version, to exercise the trait's default `is_outdated`.
    struct Versioned((u32, u32, u32));

    #[async_trait]
    impl WebViewHost for Versioned {
        fn available(&self) -> bool {
            true
        }

        fn version(&self) -> Option<(u32, u32, u32)> {
            Some(self.0)
        }

        async fn solve_challenge(&self, _req: SolveRequest, _cancel: CancellationToken) -> SolveOutcome {
            SolveOutcome::NoChallenge { outdated: false }
        }

        async fn get_html(&self) -> Option<String> {
            None
        }
    }

    #[test]
    fn the_trait_method_says_two_forty_is_outdated_and_two_forty_two_is_not() {
        assert!(Versioned((2, 40, 5)).is_outdated());
        assert!(!Versioned((2, 42, 0)).is_outdated());
        assert_eq!(Versioned((2, 40, 5)).version_string(), "WebKitGTK 2.40.5");
    }

    #[test]
    fn no_webview_is_unavailable_and_solves_as_unavailable() {
        let host = NoWebView;
        assert!(!host.available());
        assert_eq!(host.version_string(), "WebKitGTK not available");
        assert!(!host.is_outdated());
        let req = SolveRequest {
            url: Url::parse("https://example.test/").unwrap(),
            headers: Vec::new(),
            user_agent: String::new(),
            cookies: Vec::new(),
            old_clearance: None,
            timeout: Duration::from_secs(30),
            cookie_names: &["cf_clearance"],
            visible: false,
        };
        let outcome = futures_block_on(host.solve_challenge(req, CancellationToken::new()));
        assert_eq!(
            outcome,
            SolveOutcome::Failed {
                reason: SolveFailure::Unavailable,
                outdated: false
            }
        );
    }

    #[test]
    fn no_webview_has_no_login_path_and_no_imported_cookies() {
        let login = futures_block_on(NoWebView.oauth_login(Box::new(|_: &str| Url::parse("https://auth.test/").unwrap()), Duration::from_secs(1)));
        assert_eq!(login.unwrap_err().kind(), io::ErrorKind::Unsupported);
        assert!(NoWebView.imported_cookies(&Url::parse("https://site.test/").unwrap()).is_empty());
    }

    /// A host whose page holds quotes and newlines. Android's `getHtml` returns such text JSON-quoted and escaped.
    struct Page(&'static str);

    #[async_trait]
    impl WebViewHost for Page {
        fn available(&self) -> bool {
            true
        }

        async fn solve_challenge(&self, _req: SolveRequest, _cancel: CancellationToken) -> SolveOutcome {
            SolveOutcome::NoChallenge { outdated: false }
        }

        async fn get_html(&self) -> Option<String> {
            Some(self.0.to_owned())
        }
    }

    #[test]
    fn get_html_is_the_page_text_with_no_json_quoting() {
        let page = "<html lang=\"en\">\n<p>say \"hi\"</p>\n</html>";
        let html = futures_block_on(Page(page).get_html()).unwrap();
        assert_eq!(html, page);
        assert!(html.starts_with("<html"), "the text is not wrapped in quotes");
        assert_eq!(futures_block_on(NoWebView.get_html()), None);
    }

    fn futures_block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(fut)
    }
}
