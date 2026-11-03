//! `ImageService::fetch`: resolve, cache layers, network.
//!
//! The service returns the image as a file to read or as the response bytes. Decoding belongs to the
//! decode lane, which holds one of the `decode_permit` slots while it decodes.

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use ::http::HeaderMap;
use bytes::Bytes;
use kuro_net::NetworkClient;
use kuro_platform::paths::Paths;
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio_util::sync::CancellationToken;

use crate::art::ArtRef;
use crate::cover_file;
use crate::cover_paths;
use crate::decode::{DecodeOptions, Decoded, decode};
use crate::disk_image_cache::DiskImageCache;
use crate::error::ImageError;
use crate::http;
use crate::icc::{self, DisplayProfile};
use crate::keys;
use crate::memory_cache::{DEFAULT_BUDGET_BYTES, MemoryCache};
use crate::options::{LoadOptions, Priority};
use crate::platform::image_cache_dir;
use crate::recheck;
use crate::resource::{self, Resource};

/// At most this many fetches run at once.
pub const FETCH_CONCURRENCY: usize = 8;
/// At most this many decodes run at once.
pub const DECODE_CONCURRENCY: usize = 3;
/// The extra fetch and decode slots only `Priority::High` requests take, so the open page's picture starts at once even
/// while every normal slot is busy.
pub const HIGH_PRIORITY_SLOTS: usize = 1;

/// Where the image came from. `fetch` returns `Disk` or `Network`; `Memory` is
/// the reader's own buffer (`buffered`) or a decoded image served by the memory cache (`ImageService::load`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataSource {
    Memory,
    Disk,
    Network,
}

/// The fetched image: a file to read from, or the response bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    File(PathBuf),
    Bytes(Bytes),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
    pub source: DataSource,
    pub payload: Payload,
}

impl Fetched {
    fn file(source: DataSource, path: PathBuf) -> Self {
        Self {
            source,
            payload: Payload::File(path),
        }
    }
}

/// The HTTP side of a source: its client and its headers.
pub struct SourceClient {
    pub client: NetworkClient,
    pub headers: HeaderMap,
}

/// Answers which HTTP client and headers a source uses. `None` for local and unknown sources, which
/// fall back to the shared client without extra headers (edge E-IMG-31).
pub trait SourceHttp: Send + Sync {
    fn http_source(&self, source_id: i64) -> Option<SourceClient>;

    /// The real picture URL of a listing's [`COVER_MARKER`] thumbnail (`anime_url` is the rest of it). `None`: no picture.
    fn cover_url<'a>(&'a self, _source_id: i64, _anime_url: &'a str) -> Pin<Box<dyn Future<Output = Option<String>> + Send + 'a>> {
        Box::pin(async { None })
    }
}

/// The start of a thumbnail URL that is no picture but a request for the source's poster: a listing gives
/// `kb-cover:<anime url>` for an anime whose poster it does not know yet, and the image service asks the source when the
/// card needs it (so the list itself never waits on lookups).
pub const COVER_MARKER: &str = "kb-cover:"; // the same text as `kuro_source::anime::COVER_MARKER`

pub struct ImageService {
    shared: NetworkClient,
    sources: Arc<dyn SourceHttp>,
    paths: Arc<dyn Paths + Send + Sync>,
    disk: DiskImageCache,
    fetch_permits: Semaphore,
    decode_permits: Semaphore,
    high_fetch_permits: Semaphore,
    high_decode_permits: Semaphore,
    /// One gate per cache key being loaded: a second request for the same picture waits for the first and then finds it
    /// in the memory cache, instead of fetching and decoding it again.
    in_flight: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    memory: Arc<MemoryCache>,
    /// The display ICC profile every decode applies, `None` for no colour management (T-34a-031).
    display_profile: RwLock<Option<Arc<DisplayProfile>>>,
}

/// One image request for `ImageService::load`.
pub struct LoadRequest {
    pub art: ArtRef,
    pub options: LoadOptions,
    pub cancel: CancellationToken,
}

/// A decoded image, shared with the memory cache.
#[derive(Clone, Debug)]
pub struct LoadedImage {
    pub image: Arc<Decoded>,
    /// `Memory` for a decoded-image cache hit, otherwise where the bytes came from.
    pub source: DataSource,
    /// The cache key, what a widget compares to drop a stale result.
    pub key: String,
}

impl ImageService {
    pub async fn new(
        shared: NetworkClient,
        sources: Arc<dyn SourceHttp>,
        paths: Arc<dyn Paths + Send + Sync>,
    ) -> io::Result<Self> {
        let disk = DiskImageCache::open(image_cache_dir(&*paths)).await?;
        Ok(Self {
            shared,
            sources,
            paths,
            disk,
            fetch_permits: Semaphore::new(FETCH_CONCURRENCY),
            decode_permits: Semaphore::new(DECODE_CONCURRENCY),
            high_fetch_permits: Semaphore::new(HIGH_PRIORITY_SLOTS),
            high_decode_permits: Semaphore::new(HIGH_PRIORITY_SLOTS),
            in_flight: Mutex::default(),
            memory: Arc::new(MemoryCache::new(DEFAULT_BUDGET_BYTES)),
            display_profile: RwLock::new(None),
        })
    }

    /// Replaces the decoded-image cache with one of `bytes` (the default is 64 MiB). Call it before sharing the
    /// service.
    pub fn with_memory_budget(mut self, bytes: u64) -> Self {
        self.memory = Arc::new(MemoryCache::new(bytes));
        self
    }

    /// Replaces the disk cache budget (the default follows the free space) with the app's one cache size limit.
    pub async fn with_disk_budget(mut self, bytes: u64) -> io::Result<Self> {
        self.disk = DiskImageCache::open_with_budget(self.disk.root().to_path_buf(), bytes).await?;
        Ok(self)
    }

    /// The settings-change handler of `pref_display_profile_key`: loads the profile file once and applies it to every
    /// later decode. An empty value, or a file that cannot be used, gives no profile and no error. Decoded images
    /// cached under the old profile are dropped.
    pub fn set_display_profile(&self, setting: &str) {
        let profile = icc::load_setting(setting);
        *self.display_profile.write().unwrap_or_else(|e| e.into_inner()) = profile;
        self.memory.clear();
    }

    pub fn memory(&self) -> &MemoryCache {
        &self.memory
    }

    /// One of the decode slots. The decode lane holds it for the whole decode.
    pub async fn decode_permit(&self) -> SemaphorePermit<'_> {
        self.decode_permits
            .acquire()
            .await
            .expect("decode semaphore is never closed")
    }

    /// A slot from `normal`, or for a `High` request the first free one of `normal` and `high`.
    async fn slot<'a>(normal: &'a Semaphore, high: &'a Semaphore, priority: Priority) -> SemaphorePermit<'a> {
        let permit = match priority {
            Priority::Normal => normal.acquire().await,
            Priority::High => tokio::select! {
                permit = normal.acquire() => permit,
                permit = high.acquire() => permit,
            },
        };
        permit.expect("image semaphores are never closed")
    }

    /// The image `load` would return from the decoded-image cache, without waiting: a cover drawn again (a list
    /// rebuilt when its screen comes back) shows at once instead of passing through its placeholder. Costs one
    /// `stat` for the custom cover file.
    pub fn cached(&self, art: &ArtRef, options: &LoadOptions) -> Option<LoadedImage> {
        let key = keys::cache_key(art, cover_paths::has_custom(&*self.paths, art));
        let image = self.memory.get(&key, options.memory_cache)?;
        Some(LoadedImage { image, source: DataSource::Memory, key })
    }

    /// Loads one image: the decoded-image cache, then `fetch`, then decode on a blocking thread under one of the
    /// three decode slots. Cancelling ends the call with `ImageError::Cancelled`
    /// and nothing is cached; a result that finishes after the cancel is dropped (edge E-IMG-30).
    pub async fn load(&self, req: &LoadRequest) -> Result<LoadedImage, ImageError> {
        let paths: &dyn Paths = &*self.paths;
        let custom_exists = tokio::fs::try_exists(cover_paths::custom_file_for(paths, &req.art))
            .await
            .unwrap_or(false);
        let key = keys::cache_key(&req.art, custom_exists);
        let policy = req.options.memory_cache;
        if let Some(image) = self.memory.get(&key, policy) {
            return Ok(LoadedImage {
                image,
                source: DataSource::Memory,
                key,
            });
        }
        // one load per picture: a request for a key already loading waits for it, then takes it from the memory cache.
        // The gate is per priority, so the open page never waits behind a thumbnail still queued for a slot.
        let lane = format!("{:?};{key}", req.options.priority);
        let gate = self.gate(&lane);
        let held = tokio::select! {
            biased;
            _ = req.cancel.cancelled() => {
                self.release(&lane, gate);
                return Err(ImageError::Cancelled);
            }
            held = gate.clone().lock_owned() => held,
        };
        if let Some(image) = self.memory.get(&key, policy) {
            drop(held);
            self.release(&lane, gate);
            return Ok(LoadedImage {
                image,
                source: DataSource::Memory,
                key,
            });
        }
        let result = self.load_once(req, &key).await;
        drop(held);
        self.release(&lane, gate);
        result
    }

    /// The gate of `lane` (a priority and a cache key), shared by every request for it while one is loading.
    fn gate(&self, lane: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut map = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(map.entry(lane.to_owned()).or_default())
    }

    /// Lets go of `gate`; the last request of a lane removes its entry.
    fn release(&self, lane: &str, gate: Arc<tokio::sync::Mutex<()>>) {
        let mut map = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        // the map holds one reference and `gate` the other: nobody else is waiting
        if Arc::strong_count(&gate) <= 2 {
            map.remove(lane);
        }
    }

    /// Fetches and decodes one picture, and keeps the result in the memory cache.
    async fn load_once(&self, req: &LoadRequest, key: &str) -> Result<LoadedImage, ImageError> {
        let (key, policy) = (key.to_owned(), req.options.memory_cache);
        let fetched = self.fetch(&req.art, &req.options, &req.cancel).await?;
        let bytes = match fetched.payload {
            Payload::Bytes(bytes) => bytes,
            Payload::File(path) => Bytes::from(tokio::fs::read(path).await?),
        };
        let image = self.decode_bytes(bytes, &req.options, &req.cancel).await?;
        self.memory.insert(key.clone(), image.clone(), policy);
        Ok(LoadedImage {
            image,
            source: fetched.source,
            key,
        })
    }

    /// Decodes `bytes` off the caller's thread, holding a decode slot for the whole decode. Reader pages call
    /// this with the bytes of the page cache.
    pub async fn decode_bytes(
        &self,
        bytes: Bytes,
        opts: &LoadOptions,
        cancel: &CancellationToken,
    ) -> Result<Arc<Decoded>, ImageError> {
        let decode_opts = DecodeOptions {
            display_profile: self.display_profile.read().unwrap_or_else(|e| e.into_inner()).clone(),
            ..DecodeOptions::from(opts)
        };
        let permit = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(ImageError::Cancelled),
            permit = Self::slot(&self.decode_permits, &self.high_decode_permits, opts.priority) => permit,
        };
        let joined = tokio::task::spawn_blocking(move || decode(&bytes, &decode_opts)).await;
        drop(permit);
        if cancel.is_cancelled() {
            return Err(ImageError::Cancelled);
        }
        match joined {
            Ok(result) => result.map(Arc::new),
            Err(_) => Err(ImageError::DecodeFailed),
        }
    }

    /// Fetches one image. Cancelling `cancel` ends the call with `ImageError::Cancelled`. The network call
    /// is dropped, and so is any partial cache write.
    pub async fn fetch(
        &self,
        art: &ArtRef,
        opts: &LoadOptions,
        cancel: &CancellationToken,
    ) -> Result<Fetched, ImageError> {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(ImageError::Cancelled),
            result = self.fetch_limited(art, opts, cancel) => result,
        }
    }

    async fn fetch_limited(
        &self,
        art: &ArtRef,
        opts: &LoadOptions,
        cancel: &CancellationToken,
    ) -> Result<Fetched, ImageError> {
        let _permit = Self::slot(&self.fetch_permits, &self.high_fetch_permits, opts.priority).await;
        self.resolve(art, opts, cancel).await
    }

    /// The custom cover first, then the URL by its kind.
    async fn resolve(
        &self,
        art: &ArtRef,
        opts: &LoadOptions,
        cancel: &CancellationToken,
    ) -> Result<Fetched, ImageError> {
        let paths: &dyn Paths = &*self.paths;
        if opts.use_custom_cover {
            let custom = cover_paths::custom_file_for(paths, art);
            if tokio::fs::try_exists(&custom).await.unwrap_or(false) {
                return Ok(Fetched::file(DataSource::Disk, custom));
            }
        }
        let mut url = art.url.as_deref().ok_or(ImageError::NoCoverSpecified)?;
        let found;
        let resolved;
        let mut art = art;
        if let Some(anime_url) = url.strip_prefix(COVER_MARKER) {
            found = self.sources.cover_url(art.source_id, anime_url).await.ok_or(ImageError::NoCoverSpecified)?;
            url = &found;
            // the poster is cached under its own address, so a row that stores the real address later finds it
            resolved = ArtRef { url: Some(found.clone()), ..art.clone() };
            art = &resolved;
        }
        match resource::classify(Some(url)) {
            Some(Resource::Http) => self.fetch_http(art, opts, cancel, url).await,
            Some(Resource::File) => Ok(Fetched::file(DataSource::Disk, file_path(url))),
            // Content URIs are an Android picker format, not supported on Linux (E-IMG-11).
            Some(Resource::Uri) | None => Err(ImageError::InvalidImage),
        }
    }

    /// Library cover file, disk snapshot (promoted for library entries), then network,
    /// written through to the cover file or the disk cache when the policy allows.
    async fn fetch_http(
        &self,
        art: &ArtRef,
        opts: &LoadOptions,
        cancel: &CancellationToken,
        url: &str,
    ) -> Result<Fetched, ImageError> {
        let paths: &dyn Paths = &*self.paths;
        let custom_exists = tokio::fs::try_exists(cover_paths::custom_file_for(paths, art))
            .await
            .unwrap_or(false);
        let key = keys::cache_key(art, custom_exists);
        let library_cover = if art.favorite {
            cover_paths::cover_file_for(paths, art)
        } else {
            None
        };

        if let Some(cover) = &library_cover {
            if opts.disk_cache.read_enabled() && tokio::fs::try_exists(cover).await.unwrap_or(false)
            {
                self.recheck_if_due(art, url, cover);
                return Ok(Fetched::file(DataSource::Disk, cover.clone()));
            }
        }

        if opts.disk_cache.read_enabled() {
            if let Some(snapshot) = self.disk.get(&key).await {
                if let Some(cover) = &library_cover {
                    match cover_file::promote_snapshot(&self.disk, &key, &snapshot, cover).await {
                        Ok(()) => {
                            self.recheck_if_due(art, url, cover);
                            return Ok(Fetched::file(DataSource::Disk, cover.clone()));
                        }
                        Err(e) => tracing::error!(
                            error = %e,
                            "Failed to write snapshot data to cover cache {}",
                            file_name(cover)
                        ),
                    }
                }
                self.recheck_if_due(art, url, &snapshot);
                return Ok(Fetched::file(DataSource::Disk, snapshot));
            }
        }

        let (client, headers) = self.client_of(art);
        let request = http::image_request(url, &headers, opts.network_cache, cancel)?;
        let response = client.execute(request).await?;
        http::check_status(response.status)?;
        let check = recheck::Check::from_headers(&response.headers, recheck::now_ms());
        let body = response.bytes().await?;
        recheck::write(self.disk.root(), url, &check);

        if let Some(cover) = &library_cover {
            if opts.disk_cache.write_enabled() {
                match cover_file::write_atomic(cover, &body).await {
                    Ok(()) => return Ok(Fetched::file(DataSource::Network, cover.clone())),
                    Err(e) => tracing::error!(
                        error = %e,
                        "Failed to write response data to cover cache {}",
                        file_name(cover)
                    ),
                }
            }
        }

        // The write policy is checked here, before every disk write (E-IMG-9). A failed write is returned.
        if opts.disk_cache.write_enabled() {
            let path = self.disk.put(&key, &body).await?;
            return Ok(Fetched::file(DataSource::Network, path));
        }
        Ok(Fetched {
            source: DataSource::Network,
            payload: Payload::Bytes(body),
        })
    }
}

impl ImageService {
    fn client_of(&self, art: &ArtRef) -> (NetworkClient, HeaderMap) {
        match self.sources.http_source(art.source_id) {
            Some(source) => (source.client, source.headers),
            None => (self.shared.clone(), HeaderMap::new()),
        }
    }

    /// A cached cover is shown at once; when its last check is [`recheck::RECHECK_AFTER_MS`] old, the site is asked in
    /// the background whether it changed (a conditional request), and a changed picture replaces `file` for the next
    /// look. A cover cached before checks existed starts its clock now.
    fn recheck_if_due(&self, art: &ArtRef, url: &str, file: &Path) {
        let (root, now) = (self.disk.root().to_path_buf(), recheck::now_ms());
        let Some(check) = recheck::read(&root, url) else {
            return recheck::write(&root, url, &recheck::Check { checked_ms: now, ..recheck::Check::default() });
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
        if !check.due(now) {
            return;
        }
        // marked first, so the covers on screen do not each start a check of the same address
        recheck::write(&root, url, &recheck::Check { checked_ms: now, ..check.clone() });
        let (client, headers) = self.client_of(art);
        let (url, file, driver, memory) = (url.to_owned(), file.to_path_buf(), runtime.clone(), Arc::clone(&self.memory));
        // the client's future is not `Send`, so it runs to the end on a blocking-pool thread
        runtime.spawn_blocking(move || {
            driver.block_on(async move {
                let request = http::image_request(&url, &check.conditional(&headers), crate::options::CachePolicy::Enabled, &CancellationToken::new());
                let Ok(response) = client.execute(request?).await else { return Ok(()) };
                if response.status == ::http::StatusCode::NOT_MODIFIED || !response.status.is_success() {
                    return Ok(());
                }
                let changed = recheck::Check::from_headers(&response.headers, now);
                let body = response.bytes().await?;
                cover_file::write_atomic(&file, &body).await?;
                recheck::write(&root, &url, &changed);
                // the next look decodes the new picture
                memory.forget_url(&url);
                tracing::debug!(url, "a cover changed behind its address and was downloaded again");
                Ok::<(), ImageError>(())
            })
        });
    }
}

/// The path of a `file://` link with its percent escapes decoded (a local cover's link escapes spaces), or the path
/// itself.
fn file_path(url: &str) -> PathBuf {
    // the url crate decodes it for this system: `file:///C:/x` is `C:\x` on Windows, and non-UTF-8 names survive
    let decoded = url.starts_with("file://").then(|| url::Url::parse(url).ok()?.to_file_path().ok()).flatten();
    decoded.unwrap_or_else(|| PathBuf::from(url))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use kuro_platform::kind::Kind;
    use super::*;
    use crate::art::ArtKind;
    use crate::options::CachePolicy;
    use crate::testing::{FixedSource, JPEG, NoSources, TestPaths, test_client};
    use ::http::HeaderValue;
    use ::http::header::HeaderName;
    use std::time::Duration;
    use wiremock::matchers::{header, method, path as path_matcher};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn cover(url: Option<&str>, favorite: bool, source_id: i64) -> ArtRef {
        ArtRef {
            owner: Kind::Manga,
            entry_id: 7,
            source_id,
            kind: ArtKind::Cover,
            url: url.map(str::to_owned),
            favorite,
            last_modified: 100,
        }
    }

    async fn service(root: &Path, sources: Arc<dyn SourceHttp>) -> ImageService {
        let paths: Arc<dyn Paths + Send + Sync> = Arc::new(TestPaths::new(root));
        ImageService::new(test_client(root), sources, paths)
            .await
            .unwrap()
    }

    /// A source that answers every cover request with one URL, and keeps the anime url it was asked for.
    struct CoverSource(String, std::sync::Mutex<Vec<String>>);

    impl SourceHttp for CoverSource {
        fn http_source(&self, _source_id: i64) -> Option<SourceClient> {
            None
        }

        fn cover_url<'a>(&'a self, _source_id: i64, anime_url: &'a str) -> Pin<Box<dyn Future<Output = Option<String>> + Send + 'a>> {
            self.1.lock().unwrap().push(anime_url.to_owned());
            let url = (!self.0.is_empty()).then(|| self.0.clone());
            Box::pin(async move { url })
        }
    }

    #[tokio::test]
    async fn a_cover_marker_is_asked_of_the_source_and_the_poster_is_fetched() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path_matcher("/poster.jpg")).respond_with(ResponseTemplate::new(200).set_body_bytes(JPEG)).mount(&server).await;
        let tmp = tempfile::tempdir().unwrap();
        let source = Arc::new(CoverSource(format!("{}/poster.jpg", server.uri()), Default::default()));
        let svc = service(tmp.path(), source.clone()).await;
        let art = cover(Some("kb-cover:/a/42"), false, 1);
        svc.fetch(&art, &LoadOptions::default(), &CancellationToken::new()).await.unwrap();
        assert_eq!(*source.1.lock().unwrap(), ["/a/42"]);
        // the second look asks again (the marker is the cache key); the poster itself comes from the cache
        svc.fetch(&art, &LoadOptions::default(), &CancellationToken::new()).await.unwrap();
        assert_eq!(source.1.lock().unwrap().len(), 2);
        let none = Arc::new(CoverSource(String::new(), Default::default()));
        let svc = service(&tmp.path().join("other"), none).await;
        assert!(svc.fetch(&art, &LoadOptions::default(), &CancellationToken::new()).await.is_err(), "no poster: no picture");
    }

    #[tokio::test]
    async fn a_cached_cover_is_asked_again_after_three_days_and_replaced_only_when_it_changed() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path_matcher("/c.jpg")).respond_with(ResponseTemplate::new(200).set_body_bytes(JPEG).insert_header("etag", "\"v1\"")).mount(&server).await;
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let url = format!("{}/c.jpg", server.uri());
        let art = cover(Some(&url), false, 1);
        let opts = LoadOptions::default();
        let Payload::File(file) = svc.fetch(&art, &opts, &CancellationToken::new()).await.unwrap().payload else { panic!("a file") };
        let root = svc.disk.root().to_path_buf();
        assert_eq!(recheck::read(&root, &url).unwrap().etag.as_deref(), Some("\"v1\""));
        svc.fetch(&art, &opts, &CancellationToken::new()).await.unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "a fresh check: the cache answers");

        server.reset().await;
        Mock::given(method("GET")).and(header("if-none-match", "\"v1\"")).respond_with(ResponseTemplate::new(304)).mount(&server).await;
        recheck::write(&root, &url, &recheck::Check { checked_ms: 0, etag: Some("\"v1\"".into()), last_modified: None });
        assert_eq!(svc.fetch(&art, &opts, &CancellationToken::new()).await.unwrap().source, DataSource::Disk);
        for _ in 0..100 {
            if !server.received_requests().await.unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 1, "a due check asks the site");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(std::fs::read(&file).unwrap(), JPEG, "not modified: the cached cover stays");

        server.reset().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_bytes(b"NEW".to_vec()).insert_header("etag", "\"v2\"")).mount(&server).await;
        recheck::write(&root, &url, &recheck::Check { checked_ms: 0, etag: Some("\"v1\"".into()), last_modified: None });
        svc.fetch(&art, &opts, &CancellationToken::new()).await.unwrap();
        // the new file is written first and its check record right after: wait for both, or a fast runner reads the
        // record before it is there
        let v2 = || recheck::read(&root, &url).and_then(|check| check.etag).as_deref() == Some("\"v2\"");
        for _ in 0..100 {
            if std::fs::read(&file).unwrap() == b"NEW" && v2() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(std::fs::read(&file).unwrap(), b"NEW", "changed: downloaded again");
        assert!(v2(), "the record keeps the new ETag");
    }

    #[test]
    fn a_file_link_is_decoded_to_its_path() {
        let page = std::env::temp_dir().join("a").join("Ch 1").join("001.png");
        let link = url::Url::from_file_path(&page).unwrap().to_string();
        assert!(link.ends_with("/a/Ch%201/001.png"), "{link}");
        assert_eq!(file_path(&link), page);
        assert_eq!(file_path(page.to_str().unwrap()), page, "a plain path is itself");
        let odd = std::env::temp_dir().join("100%");
        assert_eq!(file_path(url::Url::from_file_path(&odd).unwrap().as_str()), odd);
    }

    #[tokio::test]
    async fn custom_cover_wins_without_touching_the_network() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let art = cover(Some("https://unreachable.invalid/a.jpg"), true, 1);
        let custom = cover_paths::custom_file_for(&*svc.paths, &art);
        std::fs::write(&custom, JPEG).unwrap();
        let got = svc
            .fetch(&art, &LoadOptions::default(), &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(got, Fetched::file(DataSource::Disk, custom));
    }

    #[tokio::test]
    async fn custom_cover_is_skipped_when_the_option_is_off() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let file = tmp.path().join("own.jpg");
        std::fs::write(&file, JPEG).unwrap();
        let art = cover(Some(file.to_str().unwrap()), false, 1);
        let custom = cover_paths::custom_file_for(&*svc.paths, &art);
        std::fs::write(&custom, b"custom").unwrap();
        let opts = LoadOptions {
            use_custom_cover: false,
            ..LoadOptions::default()
        };
        let got = svc
            .fetch(&art, &opts, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(got, Fetched::file(DataSource::Disk, file));
    }

    #[tokio::test]
    async fn missing_url_without_custom_is_no_cover_specified() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let err = svc
            .fetch(
                &cover(None, true, 1),
                &LoadOptions::default(),
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ImageError::NoCoverSpecified));
    }

    #[tokio::test]
    async fn empty_unknown_and_content_urls_are_invalid_image() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        for url in ["", "ftp://host/x.jpg", "content://media/1"] {
            let err = svc
                .fetch(
                    &cover(Some(url), false, 1),
                    &LoadOptions::default(),
                    &CancellationToken::new(),
                )
                .await
                .unwrap_err();
            assert!(matches!(err, ImageError::InvalidImage), "{url}");
        }
    }

    #[tokio::test]
    async fn file_scheme_is_stripped_and_absolute_path_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let opts = LoadOptions::default();
        let cancel = CancellationToken::new();
        // absolute paths of this system (`/srv` is not absolute on Windows)
        let (a, b) = (std::env::temp_dir().join("srv").join("a.jpg"), std::env::temp_dir().join("srv").join("b.jpg"));
        let link = url::Url::from_file_path(&a).unwrap().to_string();
        let stripped = svc
            .fetch(&cover(Some(&link), false, 1), &opts, &cancel)
            .await
            .unwrap();
        assert_eq!(stripped, Fetched::file(DataSource::Disk, a));
        let absolute = svc
            .fetch(&cover(Some(b.to_str().unwrap()), false, 1), &opts, &cancel)
            .await
            .unwrap();
        assert_eq!(absolute, Fetched::file(DataSource::Disk, b));
    }

    #[tokio::test]
    async fn library_entry_writes_the_cover_file_then_reads_it() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/a.jpg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(JPEG))
            .expect(1)
            .mount(&server)
            .await;
        let art = cover(Some(&format!("{}/a.jpg", server.uri())), true, 1);
        let opts = LoadOptions::default();
        let cancel = CancellationToken::new();
        let cover_path = cover_paths::cover_file_for(&*svc.paths, &art).unwrap();

        let first = svc.fetch(&art, &opts, &cancel).await.unwrap();
        assert_eq!(
            first,
            Fetched::file(DataSource::Network, cover_path.clone())
        );
        assert_eq!(std::fs::read(&cover_path).unwrap(), JPEG);

        let second = svc.fetch(&art, &opts, &cancel).await.unwrap();
        assert_eq!(second, Fetched::file(DataSource::Disk, cover_path));
    }

    #[tokio::test]
    async fn non_library_entry_uses_the_disk_cache_only() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/b.jpg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(JPEG))
            .expect(1)
            .mount(&server)
            .await;
        let url = format!("{}/b.jpg", server.uri());
        let art = cover(Some(&url), false, 1);
        let opts = LoadOptions::default();
        let cancel = CancellationToken::new();
        let key = keys::cache_key(&art, false);
        let expected = tmp
            .path()
            .join("cache/image_cache")
            .join(keys::hash_key(&key));

        let first = svc.fetch(&art, &opts, &cancel).await.unwrap();
        assert_eq!(first, Fetched::file(DataSource::Network, expected.clone()));
        assert_eq!(std::fs::read(&expected).unwrap(), JPEG);
        let second = svc.fetch(&art, &opts, &cancel).await.unwrap();
        assert_eq!(second, Fetched::file(DataSource::Disk, expected));
        let cover_copy = cover_paths::cover_file_for(&*svc.paths, &art).unwrap();
        assert!(!cover_copy.exists());
    }

    #[tokio::test]
    async fn disabled_disk_cache_returns_the_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/c.jpg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(JPEG))
            .mount(&server)
            .await;
        let art = cover(Some(&format!("{}/c.jpg", server.uri())), true, 1);
        let opts = LoadOptions {
            disk_cache: CachePolicy::Disabled,
            ..LoadOptions::default()
        };
        let got = svc
            .fetch(&art, &opts, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            got,
            Fetched {
                source: DataSource::Network,
                payload: Payload::Bytes(Bytes::from_static(JPEG)),
            }
        );
        assert!(
            !cover_paths::cover_file_for(&*svc.paths, &art)
                .unwrap()
                .exists()
        );
        assert_eq!(svc.disk.size().await, 0);
    }

    #[tokio::test]
    async fn read_only_policy_reads_but_does_not_write() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/d.jpg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(JPEG))
            .mount(&server)
            .await;
        let art = cover(Some(&format!("{}/d.jpg", server.uri())), true, 1);
        let opts = LoadOptions {
            disk_cache: CachePolicy::ReadOnly,
            ..LoadOptions::default()
        };
        let got = svc
            .fetch(&art, &opts, &CancellationToken::new())
            .await
            .unwrap();
        assert!(matches!(got.payload, Payload::Bytes(_)));
        assert!(
            !cover_paths::cover_file_for(&*svc.paths, &art)
                .unwrap()
                .exists()
        );
    }

    #[tokio::test]
    async fn snapshot_is_promoted_into_the_cover_file() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/e.jpg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(JPEG))
            .expect(0)
            .mount(&server)
            .await;
        let art = cover(Some(&format!("{}/e.jpg", server.uri())), true, 1);
        let key = keys::cache_key(&art, false);
        svc.disk.put(&key, b"snapshot bytes").await.unwrap();

        let got = svc
            .fetch(&art, &LoadOptions::default(), &CancellationToken::new())
            .await
            .unwrap();
        let cover_path = cover_paths::cover_file_for(&*svc.paths, &art).unwrap();
        assert_eq!(got, Fetched::file(DataSource::Disk, cover_path.clone()));
        assert_eq!(std::fs::read(cover_path).unwrap(), b"snapshot bytes");
        assert!(svc.disk.get(&key).await.is_none());
    }

    #[tokio::test]
    async fn http_error_carries_the_reason_phrase() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/f.jpg"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let art = cover(Some(&format!("{}/f.jpg", server.uri())), false, 1);
        match svc
            .fetch(&art, &LoadOptions::default(), &CancellationToken::new())
            .await
        {
            Err(ImageError::Http { code, reason }) => {
                assert_eq!(code, 404);
                assert_eq!(reason, "Not Found");
            }
            other => panic!("expected HTTP 404, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn source_client_and_headers_are_used() {
        let tmp = tempfile::tempdir().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-source"),
            HeaderValue::from_static("yes"),
        );
        let sources = FixedSource {
            id: 9,
            client: test_client(tmp.path()),
            headers,
        };
        let svc = service(tmp.path(), Arc::new(sources)).await;
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/g.jpg"))
            .and(header("x-source", "yes"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(JPEG))
            .expect(1)
            .mount(&server)
            .await;
        let art = cover(Some(&format!("{}/g.jpg", server.uri())), false, 9);
        let got = svc
            .fetch(&art, &LoadOptions::default(), &CancellationToken::new())
            .await
            .unwrap();
        assert!(matches!(got.source, DataSource::Network));
    }

    #[tokio::test]
    async fn cancelling_a_slow_request_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/slow.jpg"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
            .mount(&server)
            .await;
        let art = cover(Some(&format!("{}/slow.jpg", server.uri())), true, 1);
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            trigger.cancel();
        });
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            svc.fetch(&art, &LoadOptions::default(), &cancel),
        )
        .await
        .expect("cancel must end the call promptly");
        assert!(matches!(result, Err(ImageError::Cancelled)));
        assert!(
            !cover_paths::cover_file_for(&*svc.paths, &art)
                .unwrap()
                .exists()
        );
        assert_eq!(svc.disk.size().await, 0);
    }

    #[tokio::test]
    async fn at_most_eight_fetches_run_at_once() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let file = tmp.path().join("local.jpg");
        std::fs::write(&file, JPEG).unwrap();
        let art = cover(Some(file.to_str().unwrap()), false, 1);
        let mut held = Vec::new();
        for _ in 0..FETCH_CONCURRENCY {
            held.push(svc.fetch_permits.acquire().await.unwrap());
        }
        let blocked = tokio::time::timeout(
            Duration::from_millis(100),
            svc.fetch(&art, &LoadOptions::default(), &CancellationToken::new()),
        )
        .await;
        assert!(blocked.is_err());
        drop(held);
        assert!(
            svc.fetch(&art, &LoadOptions::default(), &CancellationToken::new())
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn at_most_three_decodes_run_at_once() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let mut held = Vec::new();
        for _ in 0..DECODE_CONCURRENCY {
            held.push(svc.decode_permit().await);
        }
        let fourth = tokio::time::timeout(Duration::from_millis(100), svc.decode_permit()).await;
        assert!(fourth.is_err());
        drop(held);
        let _ = svc.decode_permit().await;
    }

    #[tokio::test]
    async fn background_request_fetches_the_background_url_into_animebackgrounds() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_matcher("/bg.jpg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(JPEG))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_matcher("/thumb.jpg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(JPEG))
            .expect(0)
            .mount(&server)
            .await;
        let art = ArtRef::anime_entry(
            true,
            7,
            1,
            true,
            (Some(format!("{}/thumb.jpg", server.uri())), 100),
            (Some(format!("{}/bg.jpg", server.uri())), 300),
        );
        let opts = LoadOptions {
            use_background: true,
            ..LoadOptions::default()
        };
        let cancel = CancellationToken::new();
        let cover_path = cover_paths::cover_file_for(&*svc.paths, &art).unwrap();
        assert!(cover_path.starts_with(tmp.path().join("data/animebackgrounds")));

        let first = svc.fetch(&art, &opts, &cancel).await.unwrap();
        assert_eq!(
            first,
            Fetched::file(DataSource::Network, cover_path.clone())
        );
        assert_eq!(std::fs::read(&cover_path).unwrap(), JPEG);
        let second = svc.fetch(&art, &opts, &cancel).await.unwrap();
        assert_eq!(second, Fetched::file(DataSource::Disk, cover_path));
    }

    #[tokio::test]
    async fn custom_background_serves_background_requests_only() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let thumb = tmp.path().join("thumb.jpg");
        std::fs::write(&thumb, JPEG).unwrap();
        let thumb_url = thumb.to_str().unwrap().to_owned();
        let background_url = "https://unreachable.invalid/bg.jpg".to_owned();
        let bg = ArtRef::anime_entry(
            true,
            7,
            1,
            true,
            (Some(thumb_url.clone()), 100),
            (Some(background_url.clone()), 300),
        );
        let custom_bg = cover_paths::custom_file_for(&*svc.paths, &bg);
        assert!(custom_bg.starts_with(tmp.path().join("data/animebackgrounds/custom")));
        std::fs::write(&custom_bg, b"custom background").unwrap();
        let cancel = CancellationToken::new();

        let bg_opts = LoadOptions {
            use_background: true,
            ..LoadOptions::default()
        };
        let got = svc.fetch(&bg, &bg_opts, &cancel).await.unwrap();
        assert_eq!(got, Fetched::file(DataSource::Disk, custom_bg));

        let cover = ArtRef::anime_entry(
            false,
            7,
            1,
            true,
            (Some(thumb_url), 100),
            (Some(background_url), 300),
        );
        let got = svc
            .fetch(&cover, &LoadOptions::default(), &cancel)
            .await
            .unwrap();
        assert_eq!(got, Fetched::file(DataSource::Disk, thumb));
    }

    fn local_png(dir: &Path, w: u32, h: u32) -> String {
        let file = dir.join("local.png");
        std::fs::write(
            &file,
            crate::testing::encode(&crate::testing::gradient(w, h), image::ImageFormat::Png),
        )
        .unwrap();
        file.to_str().unwrap().to_owned()
    }

    fn request(url: &str, options: LoadOptions) -> LoadRequest {
        LoadRequest {
            art: cover(Some(url), false, 1),
            options,
            cancel: CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn load_decodes_then_serves_from_the_memory_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let req = request(&local_png(tmp.path(), 40, 30), LoadOptions::default());
        let first = svc.load(&req).await.unwrap();
        assert_eq!(first.source, DataSource::Disk);
        assert_eq!((first.image.width, first.image.height), (40, 30));
        assert_eq!(first.key, keys::cache_key(&req.art, false));
        let second = svc.load(&req).await.unwrap();
        assert_eq!(second.source, DataSource::Memory);
        assert!(Arc::ptr_eq(&first.image, &second.image));
    }

    #[tokio::test]
    async fn cached_answers_only_once_the_picture_is_in_memory() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let req = request(&local_png(tmp.path(), 40, 30), LoadOptions::default());
        assert!(svc.cached(&req.art, &req.options).is_none(), "nothing loaded yet");
        let loaded = svc.load(&req).await.unwrap();
        let hit = svc.cached(&req.art, &req.options).expect("in memory after a load");
        assert_eq!(hit.source, DataSource::Memory);
        assert!(Arc::ptr_eq(&loaded.image, &hit.image));
        let no_memory = LoadOptions { memory_cache: CachePolicy::Disabled, ..LoadOptions::default() };
        assert!(svc.cached(&req.art, &no_memory).is_none(), "a load that skips the memory cache never reads it");
    }

    #[tokio::test]
    async fn two_loads_of_one_picture_at_once_decode_it_once() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let req = request(&local_png(tmp.path(), 40, 30), LoadOptions::default());
        let (a, b) = tokio::join!(svc.load(&req), svc.load(&req));
        let (a, b) = (a.unwrap(), b.unwrap());
        assert!(Arc::ptr_eq(&a.image, &b.image), "the second waited for the first and shares its picture");
        assert!(svc.in_flight.lock().unwrap().is_empty(), "nothing is left waiting");
    }

    #[tokio::test]
    async fn a_high_priority_load_does_not_wait_for_busy_normal_slots() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let file = local_png(tmp.path(), 40, 30);
        let mut held = Vec::new();
        for _ in 0..DECODE_CONCURRENCY {
            held.push(svc.decode_permit().await);
        }
        let normal = request(&file, LoadOptions::default());
        assert!(tokio::time::timeout(Duration::from_millis(100), svc.load(&normal)).await.is_err(), "a normal load waits");
        let high = request(&file, LoadOptions { priority: Priority::High, ..LoadOptions::default() });
        let loaded = tokio::time::timeout(Duration::from_secs(5), svc.load(&high)).await;
        assert!(loaded.is_ok_and(|r| r.is_ok()), "the open page's picture takes its own slot");
        drop(held);
    }

    #[tokio::test]
    async fn changing_the_display_profile_setting_drops_cached_images_and_a_bad_file_is_no_error() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let req = request(&local_png(tmp.path(), 8, 8), LoadOptions::default());
        svc.load(&req).await.unwrap();
        assert!(!svc.memory().is_empty());
        svc.set_display_profile("/no/such/profile.icc");
        assert!(svc.memory().is_empty());
        assert!(svc.display_profile.read().unwrap().is_none());
        let again = svc.load(&req).await.unwrap();
        assert_eq!(again.source, DataSource::Disk, "decoded afresh, without colour management");
    }

    #[cfg(feature = "icc")]
    #[tokio::test]
    async fn a_loaded_display_profile_is_applied_to_service_decodes() {
        use lcms2::Profile;
        let tmp = tempfile::tempdir().unwrap();
        let profile_file = tmp.path().join("srgb.icc");
        std::fs::write(&profile_file, Profile::new_srgb().icc().unwrap()).unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        svc.set_display_profile(profile_file.to_str().unwrap());
        assert!(svc.display_profile.read().unwrap().is_some());
        let req = request(&local_png(tmp.path(), 8, 8), LoadOptions::default());
        assert!(svc.load(&req).await.is_ok());
        svc.set_display_profile("");
        assert!(svc.display_profile.read().unwrap().is_none());
    }

    #[tokio::test]
    async fn load_with_the_memory_cache_disabled_never_touches_it() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let opts = LoadOptions {
            memory_cache: CachePolicy::Disabled,
            ..LoadOptions::default()
        };
        let req = request(&local_png(tmp.path(), 8, 8), opts);
        assert_eq!(svc.load(&req).await.unwrap().source, DataSource::Disk);
        assert!(svc.memory().is_empty());
        assert_eq!(svc.load(&req).await.unwrap().source, DataSource::Disk);
        // A cached copy is not read either.
        let enabled = request(&local_png(tmp.path(), 8, 8), LoadOptions::default());
        svc.load(&enabled).await.unwrap();
        assert_eq!(svc.load(&req).await.unwrap().source, DataSource::Disk);
    }

    #[tokio::test]
    async fn load_downsamples_to_the_requested_size() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let opts = LoadOptions {
            size: crate::options::RequestSize::Pixels {
                width: 50,
                height: 75,
            },
            ..LoadOptions::default()
        };
        let got = svc
            .load(&request(&local_png(tmp.path(), 400, 600), opts))
            .await
            .unwrap();
        assert_eq!((got.image.width, got.image.height), (50, 75));
        assert!(got.image.is_sampled);
    }

    #[tokio::test]
    async fn a_cancelled_load_returns_cancelled_and_caches_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let req = request(&local_png(tmp.path(), 8, 8), LoadOptions::default());
        req.cancel.cancel();
        assert!(matches!(svc.load(&req).await, Err(ImageError::Cancelled)));
        assert!(svc.memory().is_empty());
    }

    #[tokio::test]
    async fn decode_bytes_runs_off_the_caller_thread_and_reports_decode_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(tmp.path(), Arc::new(NoSources)).await;
        let cancel = CancellationToken::new();
        let png = Bytes::from(crate::testing::encode(
            &crate::testing::gradient(6, 4),
            image::ImageFormat::Png,
        ));
        let got = svc
            .decode_bytes(png, &LoadOptions::webtoon_page(false), &cancel)
            .await
            .unwrap();
        assert_eq!((got.width, got.height), (6, 4));
        let bad = svc
            .decode_bytes(
                Bytes::from_static(b"not an image"),
                &LoadOptions::default(),
                &cancel,
            )
            .await;
        assert!(matches!(bad, Err(ImageError::UnsupportedFormat)));
    }
}
