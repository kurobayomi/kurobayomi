//! The HTTP response cache (area 12, T-12a-027). GET responses are stored under
//! `<cache>/network_cache` and capped at 5 MiB, with least-recently-used eviction after each write. Storage rules
//! and freshness follow OkHttp `CacheStrategy` as the topic records. Downloads and progress calls never reach it
//! (`CallOptions::use_cache`), and a request with any other method removes the entry for its URL.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use bytes::Bytes;
use chrono::DateTime;
use futures::StreamExt;
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::error::NetError;
use crate::interceptors::{Chain, Interceptor};
use crate::request::{CacheControl, Method, Request};
use crate::response::{BodyStream, Response, empty_response};

/// The folder under the cache directory (`network_cache`).
pub const CACHE_DIR_NAME: &str = "network_cache";
/// The byte cap of the store (`Cache(maxSize = 5 * 1024 * 1024)`).
pub const CACHE_CAP_BYTES: u64 = 5 * 1024 * 1024;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// What one stored response remembers: the request and response times, the headers and the last access.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub url: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub request_ms: i64,
    pub response_ms: i64,
    pub last_access_ms: i64,
}

impl Meta {
    fn header_map(&self) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in &self.headers {
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                map.append(name, value);
            }
        }
        map
    }

    /// The `Date` header in ms, or the response time when the server sent none.
    fn served_ms(&self, map: &HeaderMap) -> i64 {
        http_date(map, header::DATE).unwrap_or(self.response_ms)
    }
}

fn pairs(map: &HeaderMap) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_owned(), v.to_owned()))
        })
        .collect()
}

/// Lower-cased `Cache-Control` directives with their values, if any.
fn directives(map: &HeaderMap) -> Vec<(String, Option<String>)> {
    map.get_all(header::CACHE_CONTROL)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            let (name, value) = match part.split_once('=') {
                Some((name, value)) => (name, Some(value.trim().trim_matches('"').to_owned())),
                None => (part, None),
            };
            Some((name.trim().to_ascii_lowercase(), value))
        })
        .collect()
}

fn has_directive(dirs: &[(String, Option<String>)], name: &str) -> bool {
    dirs.iter().any(|(n, _)| n == name)
}

fn directive_secs(dirs: &[(String, Option<String>)], name: &str) -> Option<i64> {
    dirs.iter()
        .find(|(n, _)| n == name)
        .and_then(|(_, v)| v.as_deref().and_then(|s| s.parse::<i64>().ok()))
}

fn http_date(map: &HeaderMap, name: HeaderName) -> Option<i64> {
    let text = map.get(name)?.to_str().ok()?;
    DateTime::parse_from_rfc2822(text)
        .ok()
        .map(|d| d.timestamp_millis())
}

/// Whether a response may be stored (`CacheStrategy.isCacheable`).
pub fn is_storable(status: u16, headers: &HeaderMap, request: &CacheControl) -> bool {
    let dirs = directives(headers);
    if has_directive(&dirs, "no-store") || request.no_store {
        return false;
    }
    match status {
        200 | 203 | 204 | 300 | 301 | 308 | 404 | 405 | 410 | 414 | 501 => true,
        302 | 307 => {
            headers.contains_key(header::EXPIRES)
                || has_directive(&dirs, "max-age")
                || has_directive(&dirs, "public")
                || has_directive(&dirs, "private")
        }
        _ => false,
    }
}

/// The freshness lifetime in ms: `max-age`, else `Expires` minus the served date, else
/// the 10 % heuristic for a URL without a query, then capped by the request's `max-age`.
pub fn freshness_lifetime_ms(meta: &Meta, url: &Url, request: &CacheControl) -> i64 {
    let map = meta.header_map();
    let dirs = directives(&map);
    let served = meta.served_ms(&map);
    let mut fresh = if let Some(secs) = directive_secs(&dirs, "max-age") {
        secs * 1000
    } else if let Some(expires) = http_date(&map, header::EXPIRES) {
        (expires - served).max(0)
    } else if let Some(modified) =
        http_date(&map, header::LAST_MODIFIED).filter(|_| url.query().is_none())
    {
        ((served - modified) / 10).max(0)
    } else {
        0
    };
    if let Some(max) = request.max_age {
        fresh = fresh.min(i64::from(max) * 1000);
    }
    fresh
}

/// The age of a stored response in ms (`CacheStrategy.cacheResponseAge`): the apparent age, raised by the `Age`
/// header, plus the time the request took and the time the entry has been resident.
fn current_age_ms(meta: &Meta, now: i64) -> i64 {
    let map = meta.header_map();
    let apparent = (meta.response_ms - meta.served_ms(&map)).max(0);
    let age_header = header_str(&map, header::AGE).and_then(|v| v.trim().parse::<i64>().ok());
    let received = match age_header {
        Some(secs) => apparent.max(secs * 1000),
        None => apparent,
    };
    received + (meta.response_ms - meta.request_ms).max(0) + (now - meta.response_ms).max(0)
}

fn header_str(map: &HeaderMap, name: HeaderName) -> Option<String> {
    map.get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// Whether a stored response may be served with no network call.
pub fn is_fresh(meta: &Meta, url: &Url, request: &CacheControl, now: i64) -> bool {
    if request.no_cache {
        return false;
    }
    let map = meta.header_map();
    let must_revalidate = has_directive(&directives(&map), "must-revalidate");
    let fresh = freshness_lifetime_ms(meta, url, request);
    let min_fresh = i64::from(request.min_fresh.unwrap_or(0)) * 1000;
    let max_stale = if must_revalidate {
        0
    } else {
        i64::from(request.max_stale.unwrap_or(0)) * 1000
    };
    current_age_ms(meta, now) + min_fresh < fresh + max_stale
}

/// The validator headers of a stale entry: `If-None-Match` for an ETag, else `If-Modified-Since`.
fn validators(meta: &Meta) -> Vec<(HeaderName, HeaderValue)> {
    let map = meta.header_map();
    if let Some(etag) = map.get(header::ETAG) {
        return vec![(header::IF_NONE_MATCH, etag.clone())];
    }
    match map.get(header::LAST_MODIFIED) {
        Some(modified) => vec![(header::IF_MODIFIED_SINCE, modified.clone())],
        None => Vec::new(),
    }
}

/// The on-disk store: one `.meta` and one `.body` file per URL, named by the SHA-256 of the URL.
pub struct DiskCache {
    dir: PathBuf,
    cap: u64,
}

impl DiskCache {
    pub fn open(dir: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            cap: CACHE_CAP_BYTES,
        })
    }

    fn key(url: &str) -> String {
        Sha256::digest(url.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn meta_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.meta"))
    }

    fn body_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.body"))
    }

    /// The stored response for `url`. A missing or unreadable entry is a miss. A hit updates its last access.
    pub fn get(&self, url: &str) -> Option<(Meta, Bytes)> {
        let key = Self::key(url);
        let mut meta: Meta = serde_json::from_slice(&fs::read(self.meta_path(&key)).ok()?).ok()?;
        let body = Bytes::from(fs::read(self.body_path(&key)).ok()?);
        meta.last_access_ms = now_ms();
        let _ = write_atomic(&self.meta_path(&key), &serde_json::to_vec(&meta).ok()?);
        Some((meta, body))
    }

    /// Stores a response and then evicts least recently used entries until the store is within its cap.
    pub fn put(&self, meta: &Meta, body: &[u8]) -> io::Result<()> {
        let key = Self::key(&meta.url);
        write_atomic(&self.body_path(&key), body)?;
        write_atomic(&self.meta_path(&key), &serde_json::to_vec(meta)?)?;
        self.evict()
    }

    /// Removes the entry for `url`, if any.
    pub fn remove(&self, url: &str) {
        let key = Self::key(url);
        let _ = fs::remove_file(self.meta_path(&key));
        let _ = fs::remove_file(self.body_path(&key));
    }

    /// Bytes in use: the `.meta` and `.body` files together.
    pub fn size(&self) -> io::Result<u64> {
        Ok(self.entries()?.iter().map(|e| e.size).sum())
    }

    fn entries(&self) -> io::Result<Vec<Entry>> {
        let mut entries = Vec::new();
        for file in fs::read_dir(&self.dir)? {
            let path = file?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("meta") {
                continue;
            }
            let Some(key) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
                continue;
            };
            let last_access = fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Meta>(&bytes).ok())
                .map_or(0, |m| m.last_access_ms);
            let size = file_len(&path) + file_len(&self.body_path(&key));
            entries.push(Entry {
                key,
                last_access,
                size,
            });
        }
        Ok(entries)
    }

    fn evict(&self) -> io::Result<()> {
        let mut entries = self.entries()?;
        let mut total: u64 = entries.iter().map(|e| e.size).sum();
        entries.sort_by_key(|e| e.last_access);
        for entry in entries {
            if total <= self.cap {
                break;
            }
            let _ = fs::remove_file(self.meta_path(&entry.key));
            let _ = fs::remove_file(self.body_path(&entry.key));
            total = total.saturating_sub(entry.size);
        }
        Ok(())
    }
}

struct Entry {
    key: String,
    last_access: i64,
    size: u64,
}

fn file_len(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Writes through a `.part` file and renames it over the target.
fn write_atomic(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let part = target.with_extension("part");
    fs::write(&part, bytes)?;
    fs::rename(&part, target)
}

fn body_stream(body: Bytes) -> BodyStream {
    futures::stream::once(async move { Ok::<Bytes, NetError>(body) }).boxed()
}

fn replay(meta: &Meta, body: Bytes, url: &Url) -> Response {
    Response::new(
        StatusCode::from_u16(meta.status).unwrap_or(StatusCode::OK),
        meta.header_map(),
        url.clone(),
        body_stream(body),
    )
}

/// The cache as an interceptor. It sits before the Cloudflare step, so a fresh entry skips the challenge too.
pub struct HttpCache {
    store: DiskCache,
}

impl HttpCache {
    pub fn open(dir: PathBuf) -> io::Result<Self> {
        Ok(Self {
            store: DiskCache::open(dir)?,
        })
    }

    pub fn store(&self) -> &DiskCache {
        &self.store
    }
}

#[async_trait]
impl Interceptor for HttpCache {
    async fn intercept(&self, chain: Chain<'_>, mut req: Request) -> Result<Response, NetError> {
        if req.method != Method::Get {
            self.store.remove(req.url.as_str());
            return chain.proceed(req).await;
        }
        if !req.opts.use_cache {
            return chain.proceed(req).await;
        }
        let url = req.url.clone();
        let request_cache = req.cache.clone();
        let now = now_ms();
        let cached = self.store.get(url.as_str());
        if let Some((meta, body)) = &cached {
            if is_fresh(meta, &url, &request_cache, now) {
                return Ok(replay(meta, body.clone(), &url).from_cache());
            }
            for (name, value) in validators(meta) {
                req.headers.insert(name, value);
            }
        }
        if request_cache.only_if_cached {
            return Ok(empty_response(504, url));
        }
        let sent = now_ms();
        let resp = chain.proceed(req).await?;
        let received = now_ms();

        if resp.status == StatusCode::NOT_MODIFIED
            && let Some((mut meta, body)) = cached
        {
            let mut map = meta.header_map();
            for (name, value) in resp.headers.iter() {
                map.insert(name.clone(), value.clone());
            }
            meta.headers = pairs(&map);
            meta.request_ms = sent;
            meta.response_ms = received;
            meta.last_access_ms = received;
            let _ = self.store.put(&meta, &body);
            return Ok(replay(&meta, body, &url));
        }

        let declared = resp
            .headers
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let storable = is_storable(resp.status.as_u16(), &resp.headers, &request_cache);
        if !storable || declared.is_none_or(|len| len > CACHE_CAP_BYTES) {
            return Ok(resp);
        }
        let status = resp.status;
        let headers = resp.headers.clone();
        let final_url = resp.url.clone();
        let body = resp.bytes().await?;
        let meta = Meta {
            url: url.to_string(),
            status: status.as_u16(),
            headers: pairs(&headers),
            request_ms: sent,
            response_ms: received,
            last_access_ms: received,
        };
        if let Err(err) = self.store.put(&meta, &body) {
            tracing::warn!(error = %err, "HTTP cache write failed");
        }
        Ok(Response::new(status, headers, final_url, body_stream(body)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    fn meta_with(headers: &[(&str, &str)], served: i64, now_response: i64) -> Meta {
        Meta {
            url: "https://example.org/a".into(),
            status: 200,
            headers: headers
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
                .collect(),
            request_ms: now_response,
            response_ms: now_response,
            last_access_ms: now_response,
        }
        .with_date(served)
    }

    impl Meta {
        fn with_date(mut self, served: i64) -> Self {
            let date = DateTime::from_timestamp_millis(served)
                .unwrap()
                .to_rfc2822()
                .replace("+0000", "GMT");
            self.headers.push(("date".into(), date));
            self
        }
    }

    const T0: i64 = 1_700_000_000_000;

    #[test]
    fn storable_codes_follow_the_topic_table() {
        let none = HeaderMap::new();
        let cc = CacheControl::none();
        assert!(is_storable(200, &none, &cc));
        assert!(is_storable(404, &none, &cc));
        assert!(!is_storable(500, &none, &cc));
        assert!(
            !is_storable(302, &none, &cc),
            "302 needs an explicit lifetime"
        );
        let mut with_max_age = HeaderMap::new();
        with_max_age.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=60"),
        );
        assert!(is_storable(302, &with_max_age, &cc));
        let mut no_store = HeaderMap::new();
        no_store.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        assert!(!is_storable(200, &no_store, &cc));
        let request_no_store = CacheControl {
            no_store: true,
            ..CacheControl::none()
        };
        assert!(!is_storable(200, &none, &request_no_store));
    }

    #[test]
    fn max_age_gives_the_lifetime_and_the_request_cap_limits_it() {
        let meta = meta_with(&[("cache-control", "max-age=3600")], T0, T0);
        let u = url("https://example.org/a");
        assert_eq!(
            freshness_lifetime_ms(&meta, &u, &CacheControl::none()),
            3_600_000
        );
        assert_eq!(
            freshness_lifetime_ms(&meta, &u, &CacheControl::default_request()),
            600_000
        );
    }

    #[test]
    fn expires_minus_the_served_date_when_there_is_no_max_age() {
        let expires = DateTime::from_timestamp_millis(T0 + 120_000)
            .unwrap()
            .to_rfc2822()
            .replace("+0000", "GMT");
        let meta = meta_with(&[("expires", &expires)], T0, T0);
        assert_eq!(
            freshness_lifetime_ms(&meta, &url("https://example.org/a"), &CacheControl::none()),
            120_000
        );
    }

    #[test]
    fn the_heuristic_is_a_tenth_of_the_age_and_skips_urls_with_a_query() {
        let modified = DateTime::from_timestamp_millis(T0 - 100_000)
            .unwrap()
            .to_rfc2822()
            .replace("+0000", "GMT");
        let meta = meta_with(&[("last-modified", &modified)], T0, T0);
        let plain = url("https://example.org/a");
        let query = url("https://example.org/a?page=2");
        assert_eq!(
            freshness_lifetime_ms(&meta, &plain, &CacheControl::none()),
            10_000
        );
        assert_eq!(
            freshness_lifetime_ms(&meta, &query, &CacheControl::none()),
            0
        );
    }

    #[test]
    fn a_fresh_entry_expires_after_its_lifetime() {
        let meta = meta_with(&[("cache-control", "max-age=60")], T0, T0);
        let u = url("https://example.org/a");
        let cc = CacheControl::none();
        assert!(is_fresh(&meta, &u, &cc, T0 + 59_000));
        assert!(!is_fresh(&meta, &u, &cc, T0 + 61_000));
    }

    #[test]
    fn request_directives_change_the_answer() {
        let meta = meta_with(&[("cache-control", "max-age=60")], T0, T0);
        let u = url("https://example.org/a");
        let no_cache = CacheControl {
            no_cache: true,
            ..CacheControl::none()
        };
        assert!(!is_fresh(&meta, &u, &no_cache, T0));
        let min_fresh = CacheControl {
            min_fresh: Some(30),
            ..CacheControl::none()
        };
        assert!(
            !is_fresh(&meta, &u, &min_fresh, T0 + 40_000),
            "age plus min-fresh passes the lifetime"
        );
        let stale_ok = CacheControl {
            max_stale: Some(600),
            ..CacheControl::none()
        };
        assert!(is_fresh(&meta, &u, &stale_ok, T0 + 120_000));
    }

    #[test]
    fn the_age_header_raises_the_apparent_age() {
        let meta = meta_with(&[("cache-control", "max-age=60"), ("age", "100")], T0, T0);
        let u = url("https://example.org/a");
        assert!(!is_fresh(&meta, &u, &CacheControl::none(), T0));
    }

    #[test]
    fn a_must_revalidate_response_gets_no_stale_allowance() {
        let meta = meta_with(&[("cache-control", "max-age=60, must-revalidate")], T0, T0);
        let stale_ok = CacheControl {
            max_stale: Some(600),
            ..CacheControl::none()
        };
        assert!(!is_fresh(
            &meta,
            &url("https://example.org/a"),
            &stale_ok,
            T0 + 120_000
        ));
    }

    #[test]
    fn the_store_stays_within_its_five_mib_cap() {
        let dir = tempfile::tempdir().unwrap();
        let store = DiskCache::open(dir.path().join(CACHE_DIR_NAME)).unwrap();
        let body = vec![7u8; 2 * 1024 * 1024];
        for i in 0..4 {
            let meta = Meta {
                url: format!("https://example.org/{i}"),
                status: 200,
                headers: Vec::new(),
                request_ms: T0,
                response_ms: T0,
                last_access_ms: T0 + i,
            };
            store.put(&meta, &body).unwrap();
            assert!(store.size().unwrap() <= CACHE_CAP_BYTES, "after write {i}");
        }
        assert!(
            store.get("https://example.org/0").is_none(),
            "the least recently used entry is gone"
        );
        assert!(store.get("https://example.org/3").is_some());
    }

    #[test]
    fn a_stored_entry_round_trips_and_a_removed_one_is_a_miss() {
        let dir = tempfile::tempdir().unwrap();
        let store = DiskCache::open(dir.path().to_path_buf()).unwrap();
        let meta = Meta {
            url: "https://example.org/x".into(),
            status: 200,
            headers: vec![("etag".into(), "\"v1\"".into())],
            request_ms: T0,
            response_ms: T0,
            last_access_ms: T0,
        };
        store.put(&meta, b"hello").unwrap();
        let (got, body) = store.get("https://example.org/x").unwrap();
        assert_eq!(body.as_ref(), b"hello");
        assert_eq!(got.headers, meta.headers);
        store.remove("https://example.org/x");
        assert!(store.get("https://example.org/x").is_none());
    }
}
