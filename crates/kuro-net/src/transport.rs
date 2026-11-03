//! The terminal stage: reqwest with the Aniyomi transport limits (T-12a-004) and the dispatcher emulation.
//! A request waits for its host slot and then for a global slot, both before its call timeout starts, so a
//! queued request never times out while it waits.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use reqwest::redirect::Policy;
use tokio::sync::Semaphore;

use crate::doh::DohResolver;
use crate::error::NetError;
use crate::logging;
use crate::progress;
use crate::public_only::{self, PublicResolver};
use crate::request::{Body, Method, Request};
use crate::response::{BodyStream, Response};

/// The Aniyomi `NetworkHelper` values. Every field is checked by a unit test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportLimits {
    pub connect_timeout: Duration,
    /// reqwest's read timeout is an inactivity timeout, the same as OkHttp `readTimeout`.
    pub read_timeout: Duration,
    /// The whole call, from dispatch to the end of the body.
    pub call_timeout: Duration,
    pub max_redirects: usize,
    pub pool_idle_timeout: Duration,
    pub pool_max_idle_per_host: usize,
    /// Dispatcher: calls in flight across all hosts.
    pub max_requests: usize,
    /// Dispatcher: calls in flight to one host.
    pub max_requests_per_host: usize,
}

impl Default for TransportLimits {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(30),
            read_timeout: Duration::from_secs(30),
            call_timeout: Duration::from_secs(120),
            max_redirects: 20,
            pool_idle_timeout: Duration::from_secs(300),
            pool_max_idle_per_host: 5,
            max_requests: 64,
            max_requests_per_host: 5,
        }
    }
}

/// The reqwest client builder for `limits` and `cookies`. Cookies are the jar's store.
pub fn client_builder<C: reqwest::cookie::CookieStore + 'static>(
    limits: &TransportLimits,
    cookies: Arc<C>,
) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .connect_timeout(limits.connect_timeout)
        .read_timeout(limits.read_timeout)
        .timeout(limits.call_timeout)
        .redirect(Policy::limited(limits.max_redirects))
        .pool_idle_timeout(limits.pool_idle_timeout)
        .pool_max_idle_per_host(limits.pool_max_idle_per_host)
        .cookie_provider(cookies)
        .use_rustls_tls()
}

/// Sends one request. Holds the dispatcher slots until the body is dropped.
pub struct Transport {
    client: reqwest::Client,
    limits: TransportLimits,
    all: Arc<Semaphore>,
    per_host: Mutex<HashMap<String, Arc<Semaphore>>>,
    /// Logs every request and response at `kuro_net::http` (T-12a-026). Set once, at construction.
    verbose: bool,
    /// Only internet addresses are reached ([`Transport::public_only`]).
    public_only: bool,
}

impl Transport {
    pub fn new<C: reqwest::cookie::CookieStore + 'static>(
        limits: TransportLimits,
        cookies: Arc<C>,
    ) -> Result<Self, NetError> {
        Self::with_doh(limits, cookies, None)
    }

    /// Like [`Transport::new`], with DNS over HTTPS answering every host name when `doh` is set (T-12a-025).
    pub fn with_doh<C: reqwest::cookie::CookieStore + 'static>(
        limits: TransportLimits,
        cookies: Arc<C>,
        doh: Option<DohResolver>,
    ) -> Result<Self, NetError> {
        let mut builder = client_builder(&limits, cookies);
        if let Some(resolver) = doh {
            builder = builder.dns_resolver(resolver);
        }
        let client = builder
            .build()
            .map_err(|e| NetError::Other(format!("cannot build HTTP client: {e}")))?;
        Ok(Self {
            client,
            all: Arc::new(Semaphore::new(limits.max_requests)),
            limits,
            per_host: Mutex::new(HashMap::new()),
            verbose: false,
            public_only: false,
        })
    }

    /// Like [`Transport::with_doh`], reaching only the internet: an address that is this computer or the local network
    /// is refused, whether it is written in the URL, a name resolves to it or a redirect leads to it
    /// ([`public_only`](crate::public_only)). The app's client is built this way, since sources are written by anyone.
    pub fn public_only<C: reqwest::cookie::CookieStore + 'static>(
        limits: TransportLimits,
        cookies: Arc<C>,
        doh: Option<DohResolver>,
    ) -> Result<Self, NetError> {
        let client = client_builder(&limits, cookies)
            .dns_resolver(Arc::new(PublicResolver { doh }))
            .redirect(public_only::redirect_policy(limits.max_redirects))
            .build()
            .map_err(|e| NetError::Other(format!("cannot build HTTP client: {e}")))?;
        Ok(Self {
            client,
            all: Arc::new(Semaphore::new(limits.max_requests)),
            limits,
            per_host: Mutex::new(HashMap::new()),
            verbose: false,
            public_only: true,
        })
    }

    /// Turns verbose logging on or off for this transport.
    pub fn with_verbose_logging(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    fn host_semaphore(&self, host: &str) -> Arc<Semaphore> {
        let mut map = self.per_host.lock().unwrap_or_else(|p| p.into_inner());
        map.entry(host.to_owned())
            .or_insert_with(|| Arc::new(Semaphore::new(self.limits.max_requests_per_host)))
            .clone()
    }

    /// Runs one request through reqwest. The per-host slot is taken before the global one.
    pub async fn send(&self, req: Request) -> Result<Response, NetError> {
        if self.public_only && !public_only::url_allowed(&req.url) {
            return Err(NetError::Other(public_only::REFUSED.to_owned()));
        }
        if self.verbose {
            logging::log_request(&req);
        }
        let host = req.url.host_str().unwrap_or_default().to_owned();
        let host_permit = self.host_semaphore(&host).acquire_owned().await.map_err(closed)?;
        let all_permit = self.all.clone().acquire_owned().await.map_err(closed)?;

        let mut builder = self.client.request(to_reqwest_method(req.method), req.url.clone());
        builder = builder.headers(req.headers.clone());
        builder = match &req.body {
            Body::Empty => builder,
            Body::Form(pairs) => builder.form(pairs),
            // a Content-Type the caller already set wins: a second one (a plug-in sends its own) is refused as a bad
            // request by some servers, Senkuro's GraphQL API among them
            Body::Bytes { content_type, data } => {
                let builder = if req.headers.contains_key(http::header::CONTENT_TYPE) {
                    builder
                } else {
                    builder.header(http::header::CONTENT_TYPE, content_type.as_str())
                };
                builder.body(data.clone())
            }
        };
        if let Some(timeout) = req.opts.timeout {
            builder = builder.timeout(timeout);
        }
        let resp = builder.send().await?;

        let status = resp.status();
        let headers = resp.headers().clone();
        let url = resp.url().clone();
        let mut body: BodyStream = resp.bytes_stream().map(|c| c.map_err(NetError::from)).boxed();
        if let Some(listener) = req.opts.progress.clone() {
            let length = progress::content_length_of(&headers);
            body = progress::wrap_body(body, length, listener);
        }
        // The permits live as long as the body stream, so a slow download keeps its slot.
        let held = (host_permit, all_permit);
        let body = body.map(move |item| {
            let _ = &held;
            item
        });
        let response = Response::new(status, headers, url, body.boxed());
        if self.verbose {
            logging::log_response(&response);
        }
        Ok(response)
    }
}

fn closed(_: tokio::sync::AcquireError) -> NetError {
    NetError::Other("dispatcher closed".to_owned())
}

fn to_reqwest_method(method: Method) -> reqwest::Method {
    match method {
        Method::Get => reqwest::Method::GET,
        Method::Post => reqwest::Method::POST,
        Method::Put => reqwest::Method::PUT,
        Method::Delete => reqwest::Method::DELETE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::UserMessage;

    #[test]
    fn the_limits_are_the_aniyomi_values() {
        let l = TransportLimits::default();
        assert_eq!(l.connect_timeout, Duration::from_secs(30));
        assert_eq!(l.read_timeout, Duration::from_secs(30));
        assert_eq!(l.call_timeout, Duration::from_secs(120));
        assert_eq!(l.max_redirects, 20);
        assert_eq!(l.pool_idle_timeout, Duration::from_secs(300));
        assert_eq!(l.pool_max_idle_per_host, 5);
        assert_eq!(l.max_requests, 64);
        assert_eq!(l.max_requests_per_host, 5);
    }

    #[test]
    fn the_global_slot_count_is_the_configured_maximum() {
        let store = Arc::new(reqwest_cookie_store::CookieStoreMutex::new(
            cookie_store::CookieStore::new(),
        ));
        let limits = TransportLimits::default();
        let transport = Transport::new(limits.clone(), store).unwrap();
        assert_eq!(transport.all.available_permits(), limits.max_requests);
        assert_eq!(transport.all.available_permits(), 64);
    }

    #[test]
    fn the_host_slot_is_shared_per_host() {
        let store = Arc::new(reqwest_cookie_store::CookieStoreMutex::new(
            cookie_store::CookieStore::new(),
        ));
        let transport = Transport::new(TransportLimits::default(), store).unwrap();
        let a = transport.host_semaphore("a.test");
        let again = transport.host_semaphore("a.test");
        let other = transport.host_semaphore("b.test");
        assert!(Arc::ptr_eq(&a, &again));
        assert!(!Arc::ptr_eq(&a, &other));
    }

    /// The app's transport reaches only the internet; the plain one (tests' local servers) still reaches this computer.
    #[tokio::test]
    async fn the_public_only_transport_refuses_this_computer_by_address_and_by_name() {
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = server.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = server.accept().await {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                // read the request first: closing a socket with unread data resets the connection on Windows
                let _ = stream.read(&mut [0; 1024]).await;
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await;
            }
        });
        let store = || Arc::new(reqwest_cookie_store::CookieStoreMutex::new(cookie_store::CookieStore::new()));
        let guarded = Transport::public_only(TransportLimits::default(), store(), None).unwrap();
        for url in [format!("http://127.0.0.1:{port}/"), format!("http://localhost:{port}/")] {
            let err = guarded.send(crate::request::get(&url).unwrap()).await.err().unwrap_or_else(|| panic!("{url} was reached"));
            assert!(format!("{err:?}").contains("address not allowed"), "{url}: {err:?}");
        }
        let plain = Transport::new(TransportLimits::default(), store()).unwrap();
        let ok = plain.send(crate::request::get(&format!("http://127.0.0.1:{port}/")).unwrap()).await.expect("the plain transport reaches it");
        assert_eq!(ok.status.as_u16(), 200);
    }

    /// A name the resolver rejects (a label over 63 bytes, so no lookup is sent) is an unknown host, online or not.
    /// The sandbox resolves real `.invalid` names, so this uses the length rule, which fails the same way.
    #[tokio::test]
    async fn an_unresolvable_host_is_an_unknown_host() {
        let store = Arc::new(reqwest_cookie_store::CookieStoreMutex::new(
            cookie_store::CookieStore::new(),
        ));
        let transport = Transport::new(TransportLimits::default(), store).unwrap();
        let host = format!("{}.invalid", "a".repeat(64));
        let req = crate::request::get(&format!("http://{host}/")).unwrap();
        let err = transport.send(req).await.err().expect("an unresolvable host fails");
        assert!(
            matches!(&err, NetError::UnknownHost { host: named } if *named == host),
            "{err:?}"
        );
        assert_eq!(err.user_message(true), UserMessage::KeyArg("exception_unknown_host", host.clone()));
        assert_eq!(err.user_message(false), UserMessage::Key("exception_offline"));
    }

    /// A request queued behind the per-host limit does not start its call timeout while it waits.
    #[tokio::test]
    async fn a_body_sends_one_content_type_even_when_the_caller_set_one() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let store = Arc::new(reqwest_cookie_store::CookieStoreMutex::new(cookie_store::CookieStore::new()));
        let transport = Transport::new(TransportLimits::default(), store).unwrap();
        let url = format!("{}/graphql", server.uri());
        for with_header in [true, false] {
            let mut headers = http::HeaderMap::new();
            if with_header {
                headers.insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static("application/json"));
            }
            let body = Body::Bytes { content_type: "application/json".into(), data: bytes::Bytes::from_static(b"{}") };
            let req = crate::request::post(&url, body).unwrap().headers(headers);
            transport.send(req).await.unwrap();
        }
        let received = server.received_requests().await.unwrap();
        for request in &received {
            let types: Vec<_> = request.headers.get_all(http::header::CONTENT_TYPE).iter().collect();
            assert_eq!(types, ["application/json"], "exactly one Content-Type");
        }
        assert_eq!(received.len(), 2);
    }

    #[tokio::test]
    async fn a_queued_request_is_not_timed_out_while_it_waits_for_a_host_slot() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_delay(Duration::from_millis(300)),
            )
            .mount(&server)
            .await;
        let limits = TransportLimits {
            max_requests_per_host: 1,
            call_timeout: Duration::from_millis(500),
            ..TransportLimits::default()
        };
        let store = Arc::new(reqwest_cookie_store::CookieStoreMutex::new(
            cookie_store::CookieStore::new(),
        ));
        let transport = Arc::new(Transport::new(limits, store).unwrap());
        let url = format!("{}/", server.uri());
        let mut calls = Vec::new();
        for _ in 0..2 {
            let transport = transport.clone();
            let url = url.clone();
            calls.push(tokio::spawn(async move {
                let req = crate::request::get(&url).unwrap();
                transport.send(req).await.map(|r| r.status)
            }));
        }
        for call in calls {
            assert_eq!(call.await.unwrap().unwrap(), http::StatusCode::OK);
        }
    }
}
