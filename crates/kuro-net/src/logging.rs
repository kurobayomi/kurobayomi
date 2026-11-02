//! Verbose HTTP logging (T-12a-026, `NetworkHelper.kt` `HttpLoggingInterceptor` at `BODY` off). When the client
//! was built with `verbose_logging`, the terminal stage logs each request and response at `debug` on the target
//! `kuro_net::http`. `Cookie`, `Set-Cookie` and `Authorization` values are never written; they show as `<redacted>`.

use http::HeaderMap;
use http::header::{AUTHORIZATION, COOKIE, HeaderName, SET_COOKIE};

use crate::request::Request;
use crate::response::Response;

pub const TARGET: &str = "kuro_net::http";
const REDACTED: &str = "<redacted>";

/// `true` for the headers whose values must not reach a log.
pub fn is_sensitive(name: &HeaderName) -> bool {
    name == COOKIE || name == SET_COOKIE || name == AUTHORIZATION
}

/// The text written for one header: its value, or `<redacted>` for a sensitive header.
fn shown(name: &HeaderName, value: &http::HeaderValue) -> String {
    if is_sensitive(name) {
        REDACTED.to_owned()
    } else {
        String::from_utf8_lossy(value.as_bytes()).into_owned()
    }
}

fn log_headers(headers: &HeaderMap) {
    for (name, value) in headers {
        tracing::debug!(target: TARGET, "    {}: {}", name, shown(name, value));
    }
}

/// Logs the request as the transport is about to send it.
pub fn log_request(req: &Request) {
    tracing::debug!(target: TARGET, "--> {} {}", req.method.as_str(), req.url);
    log_headers(&req.headers);
}

/// Logs the response as the transport received it.
pub fn log_response(resp: &Response) {
    tracing::debug!(target: TARGET, "<-- {} {}", resp.status, resp.url);
    log_headers(&resp.headers);
}

/// Log capture for tests: a subscriber that writes to memory, so a test can read what was logged.
#[cfg(test)]
pub(crate) mod capture {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::fmt::MakeWriter;

    /// Everything the subscriber wrote so far.
    #[derive(Clone, Default)]
    pub(crate) struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Capture {
        pub(crate) fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Capture {
        type Writer = Capture;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// A subscriber that keeps `kuro_net::http` at debug and writes to `capture`, for the current thread.
    pub(crate) fn capturing(capture: &Capture) -> tracing::subscriber::DefaultGuard {
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_ansi(false)
            .with_env_filter(tracing_subscriber::EnvFilter::new("kuro_net::http=debug"))
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        // parallel tests can cache a log statement as unwatched before this thread's subscriber arrives: rebuild the cache
        tracing::callsite::rebuild_interest_cache();
        guard
    }
}

#[cfg(test)]
mod tests {
    use http::HeaderValue;

    use super::capture::{Capture, capturing};
    use super::*;
    use crate::client::NetworkClient;
    use crate::request;
    use crate::response::empty_response;

    #[test]
    fn the_three_sensitive_headers_are_redacted() {
        assert!(is_sensitive(&COOKIE));
        assert!(is_sensitive(&SET_COOKIE));
        assert!(is_sensitive(&AUTHORIZATION));
        assert!(!is_sensitive(&http::header::USER_AGENT));
        let secret = HeaderValue::from_static("token-123");
        assert_eq!(shown(&AUTHORIZATION, &secret), "<redacted>");
        assert_eq!(shown(&http::header::USER_AGENT, &secret), "token-123");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn verbose_logging_writes_method_url_status_and_headers_with_secrets_redacted() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::path("/page"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .insert_header("set-cookie", "session=abc")
                    .insert_header("x-visible", "yes"),
            )
            .mount(&server)
            .await;
        let capture = Capture::default();
        let _guard = capturing(&capture);
        let client = NetworkClient::for_tests_verbose(vec![], true);
        let mut req = request::get(&format!("{}/page", server.uri())).unwrap();
        req.headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer hunter2"));
        req.headers.insert("x-visible", HeaderValue::from_static("req"));
        client.execute(req).await.unwrap();
        let text = capture.text();
        assert!(text.contains("--> GET"), "{text}");
        assert!(text.contains("/page"), "{text}");
        assert!(text.contains("<-- 200"), "{text}");
        assert!(text.contains("authorization: <redacted>"), "{text}");
        assert!(text.contains("set-cookie: <redacted>"), "{text}");
        assert!(text.contains("x-visible: yes"), "{text}");
        assert!(!text.contains("hunter2") && !text.contains("session=abc"), "{text}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn with_verbose_logging_off_nothing_is_logged_at_the_http_target() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let capture = Capture::default();
        let _guard = capturing(&capture);
        let client = NetworkClient::for_tests_verbose(vec![], false);
        client.get(&format!("{}/", server.uri())).await.unwrap();
        assert_eq!(capture.text(), "");
    }

    #[test]
    fn the_helpers_write_one_line_per_header() {
        let capture = Capture::default();
        let _guard = capturing(&capture);
        let mut resp = empty_response(204, url::Url::parse("https://x.test/").unwrap());
        resp.headers.insert(COOKIE, HeaderValue::from_static("a=b"));
        log_response(&resp);
        let text = capture.text();
        assert!(text.contains("<-- 204 "), "{text}");
        assert!(text.contains("https://x.test/"), "{text}");
        assert!(text.contains("cookie: <redacted>"), "{text}");
    }
}
