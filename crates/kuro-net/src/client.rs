//! `NetworkClient` and its builder (T-12a-003, T-12a-008). The client is an `Arc`, so clones are cheap. A builder
//! copies the chain, so `new_builder` leaves the original client unchanged.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use kuro_platform::toast::Toaster;
use kuro_platform::webview::browser::BrowserHost;
use kuro_platform::webview::WebViewHost;
use kuro_prefs::network::NetworkPrefs;

use crate::cache::HttpCache;
use crate::cookies::CookieJar;
use crate::doh::{DohProvider, DohResolver};
use crate::error::NetError;
use crate::interceptors::{
    Chain, CloudflareInterceptor, Interceptor, RateLimit, UncaughtExceptionInterceptor, UserAgentInterceptor,
};
use crate::request::{self, Body, Request};
use crate::response::Response;
use crate::settings::NetworkSettings;
use crate::transport::{Transport, TransportLimits};

/// The period of a `rate_limit` when the caller has no reason to pick one: one second (`rateLimit` default).
pub const DEFAULT_RATE_PERIOD: Duration = Duration::from_secs(1);

/// Gives the default `User-Agent` on each call.
pub type UserAgentSource = Arc<dyn Fn() -> String + Send + Sync>;

/// The HTTP client. Every request goes through the interceptor chain, then the transport.
///
/// Extensions get the `rateLimit` and `rateLimitHost` builders and `add_interceptor`. There is no
/// `cloudflareClient` alias: the Cloudflare step is always part of this client.
#[derive(Clone)]
pub struct NetworkClient {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    chain: Vec<Arc<dyn Interceptor>>,
    /// `true` when the last entry of `chain` is the Cloudflare step, which always stays last.
    cloudflare: bool,
    transport: Arc<Transport>,
    cookies: CookieJar,
    user_agent: UserAgentSource,
}

/// Collects interceptors before a client is built. Extensions add theirs with [`ClientBuilder::add_interceptor`].
pub struct ClientBuilder {
    pub(crate) chain: Vec<Arc<dyn Interceptor>>,
    pub(crate) cloudflare: bool,
    pub(crate) transport: Arc<Transport>,
    pub(crate) cookies: CookieJar,
    pub(crate) user_agent: UserAgentSource,
}

impl NetworkClient {
    /// The app client, built from the `[network]` preferences (T-12a-030). The order is: cookie jar, transport
    /// (dispatcher limits and verbose logging), the application chain (uncaught, user agent, Cloudflare).
    /// `verbose_logging`, `doh_provider` and the default `User-Agent` are the hard-coded `NetworkSettings` defaults
    /// (the settings diet cut their preferences).
    pub fn new(
        prefs: &NetworkPrefs,
        cookie_file: PathBuf,
        webview: Arc<dyn WebViewHost>,
        toaster: Toaster,
    ) -> Result<Self, NetError> {
        Self::with_cookies(prefs, CookieJar::load(cookie_file), webview, toaster)
    }

    /// As [`NetworkClient::new`] over a jar that is already open, so several clients and the settings code share one
    /// jar (T-12a-009). Clones of a jar share one store.
    pub fn with_cookies(
        prefs: &NetworkPrefs,
        cookies: CookieJar,
        webview: Arc<dyn WebViewHost>,
        toaster: Toaster,
    ) -> Result<Self, NetError> {
        Self::with_cookies_and_pages(prefs, cookies, webview, toaster, None)
    }

    /// As [`NetworkClient::with_cookies`], with the hidden pages a Cloudflare check is answered through when the platform
    /// has them (`kuro_platform::hidden_pages`), ahead of the solve through `webview`.
    pub fn with_cookies_and_pages(
        prefs: &NetworkPrefs,
        cookies: CookieJar,
        webview: Arc<dyn WebViewHost>,
        toaster: Toaster,
        pages: Option<Arc<dyn kuro_platform::hidden_pages::HiddenPages>>,
    ) -> Result<Self, NetError> {
        Self::built(prefs, cookies, webview, toaster, pages, false)
    }

    /// The client of [`NetworkClient::with_cookies_and_pages`]; `public_only` keeps its requests on the internet
    /// ([`Transport::public_only`]).
    fn built(
        prefs: &NetworkPrefs,
        cookies: CookieJar,
        webview: Arc<dyn WebViewHost>,
        toaster: Toaster,
        pages: Option<Arc<dyn kuro_platform::hidden_pages::HiddenPages>>,
        public_only: bool,
    ) -> Result<Self, NetError> {
        let settings = NetworkSettings::from_prefs(prefs);
        let doh = DohProvider::from_pref(settings.doh_provider)
            .map(|provider| DohResolver::new(provider, &settings.default_user_agent))
            .transpose()?;
        let transport = if public_only {
            Transport::public_only(TransportLimits::default(), cookies.provider(), doh)?
        } else {
            Transport::with_doh(TransportLimits::default(), cookies.provider(), doh)?
        };
        let transport = Arc::new(transport.with_verbose_logging(settings.verbose_logging));
        let default_user_agent = settings.default_user_agent.clone();
        let user_agent: UserAgentSource = Arc::new(move || default_user_agent.clone());
        let ua = user_agent.clone();
        let chain: Vec<Arc<dyn Interceptor>> = vec![
            Arc::new(UncaughtExceptionInterceptor),
            Arc::new(UserAgentInterceptor::new(move || ua()).with_domain_defaults(cookies.clone())),
            Arc::new(CloudflareInterceptor::new(webview, toaster, cookies.clone()).with_hidden_pages(pages)),
        ];
        Ok(ClientBuilder {
            chain,
            cloudflare: true,
            transport,
            cookies,
            user_agent,
        }
        .build())
    }

    /// The client of this desktop. `embedded` is the WebKit view when this build has one; without it (or when it
    /// reports itself unavailable) a Cloudflare challenge goes to [`BrowserHost`]: the system browser opens through
    /// `opener` and the cookies the user exports to the platform's `cookies.txt` clear it.
    pub fn for_desktop(
        prefs: &NetworkPrefs,
        paths: &dyn kuro_platform::paths::Paths,
        embedded: Option<Arc<dyn WebViewHost>>,
        opener: Arc<dyn kuro_platform::opener::Opener>,
        toaster: Toaster,
    ) -> Result<Self, NetError> {
        Self::for_desktop_with_cookies(prefs, paths, CookieJar::load(paths.cookie_jar_file()), embedded, opener, toaster)
    }

    /// As [`NetworkClient::for_desktop`] over a jar that is already open (see [`NetworkClient::with_cookies`]).
    pub fn for_desktop_with_cookies(
        prefs: &NetworkPrefs,
        paths: &dyn kuro_platform::paths::Paths,
        cookies: CookieJar,
        embedded: Option<Arc<dyn WebViewHost>>,
        opener: Arc<dyn kuro_platform::opener::Opener>,
        toaster: Toaster,
    ) -> Result<Self, NetError> {
        let webview = embedded
            .filter(|view| view.available())
            .unwrap_or_else(|| Arc::new(BrowserHost::new(opener, paths.cookies_import_file())));
        // the desktop client carries every source's requests, and anyone may write a source: internet addresses only
        Self::built(prefs, cookies, webview, toaster, kuro_platform::hidden_pages::hidden_pages(paths), true)
    }

    /// A copy of this client's chain, to add interceptors to. The original client is not changed.
    pub fn new_builder(&self) -> ClientBuilder {
        ClientBuilder {
            chain: self.inner.chain.clone(),
            cloudflare: self.inner.cloudflare,
            transport: self.inner.transport.clone(),
            cookies: self.inner.cookies.clone(),
            user_agent: self.inner.user_agent.clone(),
        }
    }

    pub fn cookies(&self) -> &CookieJar {
        &self.inner.cookies
    }

    /// The default `User-Agent` as it is now, trimmed. Extensions read it for headers they build themselves.
    pub fn default_user_agent(&self) -> String {
        (self.inner.user_agent)().trim().to_owned()
    }

    /// Runs the request through the chain. Cancelling the request's token returns `Canceled`; dropping the future
    /// drops the request in flight.
    pub async fn execute(&self, req: Request) -> Result<Response, NetError> {
        let cancel = req.opts.cancel.clone();
        let chain = Chain::new(&self.inner.chain, &self.inner.transport);
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(NetError::Canceled),
            resp = chain.proceed(req) => resp,
        }
    }

    /// A copy of this client with the HTTP response cache in `dir` (T-12a-027). The cache sits before the
    /// Cloudflare step. The app passes `<cache>/network_cache`.
    pub fn with_disk_cache(&self, dir: PathBuf) -> Result<NetworkClient, NetError> {
        let cache = HttpCache::open(dir)?;
        Ok(self.new_builder().add_interceptor(Arc::new(cache)).build())
    }

    /// `GET` with `awaitSuccess`: a non-2xx status is `NetError::Http`.
    pub async fn get(&self, url: &str) -> Result<Response, NetError> {
        self.execute(request::get(url)?).await?.await_success()
    }

    /// `POST` with `awaitSuccess`.
    pub async fn post(&self, url: &str, body: Body) -> Result<Response, NetError> {
        self.execute(request::post(url, body)?).await?.await_success()
    }
}

impl ClientBuilder {
    /// Adds an interceptor after the ones already present and before the Cloudflare step.
    pub fn add_interceptor(mut self, interceptor: Arc<dyn Interceptor>) -> Self {
        let at = if self.cloudflare { self.chain.len() - 1 } else { self.chain.len() };
        self.chain.insert(at, interceptor);
        self
    }

    /// Allows `permits` requests per `period` across all hosts (`rateLimit`). Each call makes its own limiter.
    /// `permits` must be at least 1. `period` is not a default: use [`DEFAULT_RATE_PERIOD`] for one second.
    pub fn rate_limit(self, permits: usize, period: Duration) -> Result<Self, NetError> {
        Ok(self.add_interceptor(Arc::new(RateLimit::new(None, permits, period)?)))
    }

    /// Allows `permits` requests per `period` to the host of `url` (`rateLimitHost`). Other hosts are not limited.
    pub fn rate_limit_host(self, url: &str, permits: usize, period: Duration) -> Result<Self, NetError> {
        Ok(self.add_interceptor(Arc::new(RateLimit::for_host_url(url, permits, period)?)))
    }

    pub fn build(self) -> NetworkClient {
        NetworkClient {
            inner: Arc::new(ClientInner {
                chain: self.chain,
                cloudflare: self.cloudflare,
                transport: self.transport,
                cookies: self.cookies,
                user_agent: self.user_agent,
            }),
        }
    }
}

/// Turns a request into a progress call: no cache, a 30 hour call timeout, and the listener on the body.
pub fn progress_call(
    mut req: Request,
    listener: Arc<dyn crate::progress::ProgressListener>,
) -> Request {
    req.opts.use_cache = false;
    req.opts.timeout = Some(std::time::Duration::from_secs(30 * 60 * 60));
    req.opts.progress = Some(listener);
    req
}

/// The user agent of a test client: a fixed string, or none.
#[cfg(test)]
pub(crate) fn fixed_user_agent(ua: &str) -> UserAgentSource {
    let ua = ua.to_owned();
    Arc::new(move || ua.clone())
}

#[cfg(test)]
impl NetworkClient {
    /// A client with a given chain and default transport, and a jar in a fresh temp directory.
    pub(crate) fn for_tests(chain: Vec<Arc<dyn Interceptor>>) -> Self {
        Self::for_tests_verbose(chain, false)
    }

    /// As [`NetworkClient::for_tests`], with verbose logging on or off.
    pub(crate) fn for_tests_verbose(chain: Vec<Arc<dyn Interceptor>>, verbose: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let cookies = CookieJar::load(dir.path().join("cookies.json"));
        let transport = Arc::new(
            Transport::new(TransportLimits::default(), cookies.provider())
                .unwrap()
                .with_verbose_logging(verbose),
        );
        ClientBuilder {
            chain,
            cloudflare: false,
            transport,
            cookies,
            user_agent: fixed_user_agent(""),
        }
        .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use crate::interceptors::RateLimit;
    use std::time::Duration;

    #[test]
    fn a_builder_copy_leaves_the_original_chain_alone() {
        let base = NetworkClient::for_tests(vec![Arc::new(UncaughtExceptionInterceptor)]);
        let extended = base.new_builder().add_interceptor(Arc::new(
            RateLimit::new(None, 10, Duration::from_secs(1)).unwrap(),
        ));
        let _ = extended.build();
        assert_eq!(base.inner.chain.len(), 1);
    }

    #[test]
    fn added_interceptors_stay_before_the_cloudflare_step() {
        let dir = tempfile::tempdir().unwrap();
        let cookies = CookieJar::load(dir.path().join("cookies.json"));
        let transport = Arc::new(Transport::new(TransportLimits::default(), cookies.provider()).unwrap());
        let webview: Arc<dyn WebViewHost> = Arc::new(kuro_platform::webview::NoWebView);
        let (toaster, _feed) = kuro_platform::toast::toast_channel();
        let chain: Vec<Arc<dyn Interceptor>> = vec![
            Arc::new(UncaughtExceptionInterceptor),
            Arc::new(CloudflareInterceptor::new(webview, toaster, cookies.clone())),
        ];
        let client = ClientBuilder {
            chain,
            cloudflare: true,
            transport,
            cookies,
            user_agent: fixed_user_agent(""),
        }
        .add_interceptor(Arc::new(UncaughtExceptionInterceptor))
        .build();
        assert_eq!(client.inner.chain.len(), 3);
        assert!(client.inner.cloudflare);
    }

    /// Records its name when the request passes, to check the order of the chain.
    struct Recorder(&'static str, Arc<std::sync::Mutex<Vec<&'static str>>>);

    #[async_trait::async_trait]
    impl Interceptor for Recorder {
        async fn intercept(&self, chain: Chain<'_>, req: Request) -> Result<Response, NetError> {
            self.1.lock().unwrap().push(self.0);
            chain.proceed(req).await
        }
    }

    #[tokio::test]
    async fn interceptors_run_in_insertion_order_once_per_call() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let client = NetworkClient::for_tests(vec![
            Arc::new(Recorder("first", order.clone())),
            Arc::new(Recorder("second", order.clone())),
        ]);
        client.get(&format!("{}/", server.uri())).await.unwrap();
        assert_eq!(*order.lock().unwrap(), vec!["first", "second"]);
    }

    #[tokio::test]
    async fn a_client_with_no_interceptors_reaches_the_transport() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::path("/ok"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("hi"))
            .mount(&server)
            .await;
        let client = NetworkClient::for_tests(vec![]);
        let resp = client.get(&format!("{}/ok", server.uri())).await.unwrap();
        assert_eq!(resp.bytes().await.unwrap(), Bytes::from_static(b"hi"));
    }

    #[tokio::test]
    async fn a_redirect_reaches_the_chain_once() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::path("/start"))
            .respond_with(wiremock::ResponseTemplate::new(302).insert_header("location", "/end"))
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::path("/end"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let client = NetworkClient::for_tests(vec![Arc::new(Recorder("chain", order.clone()))]);
        let resp = client.get(&format!("{}/start", server.uri())).await.unwrap();
        assert!(resp.url.path().ends_with("/end"));
        assert_eq!(*order.lock().unwrap(), vec!["chain"]);
    }

    #[tokio::test]
    async fn a_progress_call_reports_every_chunk_and_done() {
        #[derive(Default)]
        struct Calls(std::sync::Mutex<Vec<(u64, i64, bool)>>);
        impl crate::progress::ProgressListener for Calls {
            fn update(&self, bytes_read: u64, content_length: i64, done: bool) {
                self.0.lock().unwrap().push((bytes_read, content_length, done));
            }
        }
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_bytes(vec![7u8; 1000]))
            .mount(&server)
            .await;
        let client = NetworkClient::for_tests(vec![]);
        let calls = Arc::new(Calls::default());
        let req = progress_call(request::get(&format!("{}/big", server.uri())).unwrap(), calls.clone());
        let resp = client.execute(req).await.unwrap();
        assert_eq!(resp.bytes().await.unwrap().len(), 1000);
        let seen = calls.0.lock().unwrap().clone();
        assert_eq!(seen.last(), Some(&(1000, 1000, true)));
        assert!(seen.iter().filter(|c| !c.2).count() >= 1);
    }

    #[test]
    fn a_progress_call_has_no_cache_and_a_thirty_hour_timeout() {
        struct Nothing;
        impl crate::progress::ProgressListener for Nothing {
            fn update(&self, _: u64, _: i64, _: bool) {}
        }
        let req = progress_call(request::get("https://x.test/").unwrap(), Arc::new(Nothing));
        assert!(!req.opts.use_cache);
        assert_eq!(req.opts.timeout, Some(Duration::from_secs(30 * 60 * 60)));
        assert!(req.opts.progress.is_some());
    }

    #[tokio::test]
    async fn a_304_is_an_http_error_in_get() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(304))
            .mount(&server)
            .await;
        let client = NetworkClient::for_tests(vec![]);
        let err = client.get(&format!("{}/", server.uri())).await.err().unwrap();
        assert!(matches!(err, NetError::Http { code: 304 }));
    }

    #[tokio::test]
    async fn dropping_the_future_closes_the_connection_on_the_server_side() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // The server reads the request, then waits for the client to hang up.
        let server = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => return true,
                        Ok(_) => {}
                    }
                }
            })
            .await
            .unwrap_or(false)
        });
        let client = NetworkClient::for_tests(vec![]);
        let url = format!("http://{addr}/");
        let timed = tokio::time::timeout(Duration::from_millis(300), client.get(&url)).await;
        assert!(timed.is_err());
        assert!(server.await.unwrap(), "the server never saw the connection close");
    }

    #[tokio::test]
    async fn a_cancelled_token_returns_canceled() {
        let client = NetworkClient::for_tests(vec![]);
        let req = request::get("http://127.0.0.1:9/").unwrap();
        req.opts.cancel.cancel();
        let err = client.execute(req).await.err().unwrap();
        assert!(matches!(err, NetError::Canceled));
    }

    #[tokio::test]
    async fn a_silent_server_times_out_after_the_read_timeout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((sock, _)) = listener.accept().await {
                held.push(sock);
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let cookies = CookieJar::load(dir.path().join("cookies.json"));
        let limits = TransportLimits {
            read_timeout: Duration::from_millis(200),
            ..TransportLimits::default()
        };
        let transport = Arc::new(Transport::new(limits, cookies.provider()).unwrap());
        let client = ClientBuilder {
            chain: vec![],
            cloudflare: false,
            transport,
            cookies,
            user_agent: fixed_user_agent(""),
        }
        .build();
        let err = client.get(&format!("http://{addr}/")).await.err().unwrap();
        assert!(matches!(err, NetError::Timeout), "{err:?}");
    }
}

#[cfg(test)]
mod preference_and_extension_tests {
    use std::time::Duration;

    use kuro_platform::toast::toast_channel;
    use kuro_platform::webview::NoWebView;
    use kuro_prefs::store::PrefStore;

    use super::*;

    fn prefs() -> NetworkPrefs {
        NetworkPrefs::new(PrefStore::in_memory())
    }

    fn app_client(prefs: &NetworkPrefs, dir: &std::path::Path) -> NetworkClient {
        let (toaster, _feed) = toast_channel();
        NetworkClient::new(prefs, dir.join("cookies.json"), Arc::new(NoWebView), toaster).unwrap()
    }

    /// T-35a-031: the app client (rustls with the system roots) does not refuse a plain `http://` server.
    #[tokio::test]
    async fn the_app_client_does_not_refuse_cleartext_http() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any()).respond_with(wiremock::ResponseTemplate::new(200)).mount(&server).await;
        assert!(server.uri().starts_with("http://"));
        let dir = tempfile::tempdir().unwrap();
        let resp = app_client(&prefs(), dir.path()).get(&server.uri()).await.unwrap();
        assert_eq!(resp.status, http::StatusCode::OK);
    }

    /// T-12a-009: clients built over one jar see each other's changes, and the jar remembers its file.
    #[test]
    fn clients_over_one_jar_share_its_cookies() {
        let dir = tempfile::tempdir().unwrap();
        let jar = CookieJar::load(dir.path().join("cookies.json"));
        let (toaster, _feed) = toast_channel();
        let a = NetworkClient::with_cookies(&prefs(), jar.clone(), Arc::new(NoWebView), toaster.clone()).unwrap();
        let b = NetworkClient::with_cookies(&prefs(), jar.clone(), Arc::new(NoWebView), toaster).unwrap();
        let site = reqwest::Url::parse("https://site.test/").unwrap();
        a.cookies().import_clearance(&site, "abc", "Mozilla/5.0 Firefox/136.0").unwrap();
        assert!(b.cookies().get(&site).iter().any(|c| c.name() == "cf_clearance"));
        jar.remove_all().unwrap();
        assert!(a.cookies().get(&site).is_empty() && b.cookies().get(&site).is_empty());
        assert_eq!(jar.path(), dir.path().join("cookies.json"));
    }

    #[tokio::test]
    async fn the_app_client_sends_the_hard_coded_default_user_agent() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let client = app_client(&prefs(), dir.path());
        assert_eq!(client.default_user_agent(), crate::settings::DEFAULT_USER_AGENT);
        client.get(&format!("{}/", server.uri())).await.unwrap();
        let sent: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| r.headers.get("user-agent").unwrap().to_str().unwrap().to_owned())
            .collect();
        assert_eq!(sent, vec![crate::settings::DEFAULT_USER_AGENT]);
    }

    #[test]
    fn the_default_rate_period_is_one_second_and_a_zero_permit_limit_is_refused() {
        assert_eq!(DEFAULT_RATE_PERIOD, Duration::from_secs(1));
        let base = NetworkClient::for_tests(vec![]);
        assert!(base.new_builder().rate_limit(0, DEFAULT_RATE_PERIOD).is_err());
        assert!(base.new_builder().rate_limit_host("not a url", 1, DEFAULT_RATE_PERIOD).is_err());
    }

    #[tokio::test]
    async fn two_builders_make_two_independent_limiters() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let base = NetworkClient::for_tests(vec![]);
        let period = Duration::from_secs(60);
        let a = base.new_builder().rate_limit(1, period).unwrap().build();
        let b = base.new_builder().rate_limit(1, period).unwrap().build();
        let url = format!("{}/", server.uri());
        a.get(&url).await.unwrap();
        // `a` has used its one permit for the next minute, but `b` has its own.
        let second_on_a = tokio::time::timeout(Duration::from_millis(200), a.get(&url)).await;
        assert!(second_on_a.is_err(), "a second request on the first client must wait");
        tokio::time::timeout(Duration::from_secs(2), b.get(&url))
            .await
            .expect("the second client has its own permit")
            .unwrap();
    }

    #[tokio::test]
    async fn a_host_limit_does_not_limit_other_hosts() {
        let limited = wiremock::MockServer::start().await;
        let other = wiremock::MockServer::start().await;
        for server in [&limited, &other] {
            wiremock::Mock::given(wiremock::matchers::any())
                .respond_with(wiremock::ResponseTemplate::new(200))
                .mount(server)
                .await;
        }
        let client = NetworkClient::for_tests(vec![])
            .new_builder()
            .rate_limit_host(&limited.uri(), 1, Duration::from_secs(60))
            .unwrap()
            .build();
        client.get(&format!("{}/", limited.uri())).await.unwrap();
        let blocked = tokio::time::timeout(Duration::from_millis(200), client.get(&format!("{}/", limited.uri()))).await;
        assert!(blocked.is_err(), "the limited host waits");
        // The limit is by host name and ignores the port, so the other server is reached by another name.
        let other_url = format!("http://localhost:{}/", other.address().port());
        for _ in 0..3 {
            tokio::time::timeout(Duration::from_secs(2), client.get(&other_url))
                .await
                .expect("other hosts are not limited")
                .unwrap();
        }
    }

    struct Root(std::path::PathBuf);

    impl kuro_platform::paths::Paths for Root {
        fn data_dir(&self) -> std::path::PathBuf { self.0.join("data") }
        fn config_dir(&self) -> std::path::PathBuf { self.0.join("config") }
        fn state_dir(&self) -> std::path::PathBuf { self.0.join("state") }
        fn cache_dir(&self) -> std::path::PathBuf { self.0.join("cache") }
        fn log_dir(&self) -> std::path::PathBuf { self.0.join("log") }
        fn temp_dir(&self) -> std::path::PathBuf { self.0.join("temp") }
        fn crash_dir(&self) -> std::path::PathBuf { self.0.join("crash") }
        fn covers_dir(&self) -> std::path::PathBuf { self.0.join("covers") }
        fn plugin_dir(&self) -> std::path::PathBuf { self.0.join("plugins") }
    }

    #[derive(Default)]
    struct Opened(std::sync::Mutex<Vec<String>>);

    struct Shared(Arc<Opened>);

    impl kuro_platform::opener::Opener for Shared {
        fn open_target(&self, target: &str) -> std::io::Result<()> {
            self.0.0.lock().unwrap().push(target.to_owned());
            Ok(())
        }
    }

    /// Without an embedded view the desktop client sends a Cloudflare challenge to `BrowserHost::solve_challenge`: the
    /// page opens in the system browser, and the exported clearance cookie lets the request through.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn without_a_web_view_a_challenge_reaches_the_browser_host() {
        use kuro_platform::paths::Paths;
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(503).insert_header("server", "cloudflare"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::any()).respond_with(wiremock::ResponseTemplate::new(200)).mount(&server).await;
        let dir = tempfile::tempdir().unwrap();
        let paths = Root(dir.path().to_path_buf());
        std::fs::create_dir_all(paths.config_dir()).unwrap();
        std::fs::create_dir_all(paths.data_dir()).unwrap();
        std::fs::write(paths.cookies_import_file(), "127.0.0.1\tFALSE\t/\tFALSE\t0\tcf_clearance\texported\n").unwrap();
        let opened = Arc::new(Opened::default());
        let (toaster, _feed) = toast_channel();
        // no hidden pages either: a machine with a web engine (CI) would answer the check through them instead
        let host = Arc::new(BrowserHost::new(Arc::new(Shared(Arc::clone(&opened))), paths.cookies_import_file()));
        let client = NetworkClient::with_cookies_and_pages(&prefs(), CookieJar::load(paths.cookie_jar_file()), host, toaster, None).unwrap();
        let url = format!("{}/", server.uri());
        let resp = client.get(&url).await.unwrap();
        assert_eq!(resp.status, http::StatusCode::OK);
        assert_eq!(*opened.0.lock().unwrap(), vec![url], "the browser host opened the challenged page");
    }
}
