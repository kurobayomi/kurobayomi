//! Page-list and image disk cache (`ChapterCache.kt`, `DiskUtil.kt`). T-43a-016.
//!
//! Keys are md5 hex: a page list is `md5(mangaId + url)`, an image is `md5(imageUrl)`. The app opens it with the one
//! cache size limit of Settings (`crate::cache_limit`) and it evicts the least recently used file first. Recency is the file mtime, which
//! every read refreshes. A file is written under a temp name and renamed into place, so a crash never leaves a
//! partial image marked as cached.
//!
//! Upstream writes the page list when the chapter closes. This cache writes it as soon as it is fetched
//! (the documented improvement, E-HTTP-5).

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use kuro_core::hash::md5_hex;

use super::loader::PageEntry;

#[cfg(test)]
/// The cache size limit (`100L * 1024 * 1024`).
pub const CACHE_LIMIT_BYTES: u64 = 100 * 1024 * 1024;

/// Page-list key: md5 of the manga id followed by the chapter url, as upstream.
pub fn page_list_key(manga_id: i64, chapter_url: &str) -> String {
    md5_hex(format!("{manga_id}{chapter_url}").as_bytes())
}

/// Image key: md5 of the image url the page list gives (E-HTTP-4).
pub fn image_key(image_url: &str) -> String {
    md5_hex(image_url.as_bytes())
}

/// The disk cache rooted at one directory.
pub struct DiskCache {
    root: PathBuf,
    limit: u64,
}

impl DiskCache {
    /// Opens (creating if needed) the cache at `root` with the given size limit.
    pub fn open(root: impl Into<PathBuf>, limit: u64) -> io::Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root, limit })
    }

    #[cfg(test)]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The file a page list is stored in.
    pub fn page_list_path(&self, manga_id: i64, chapter_url: &str) -> PathBuf {
        self.root.join(format!("{}.pages", page_list_key(manga_id, chapter_url)))
    }

    /// The file an image is stored in (`md5 + ".0"` as upstream).
    pub fn image_path(&self, image_url: &str) -> PathBuf {
        self.root.join(format!("{}.0", image_key(image_url)))
    }

    /// The cached page list, or `None` when there is none or it cannot be read back.
    pub fn get_page_list(&self, manga_id: i64, chapter_url: &str) -> Option<Vec<PageEntry>> {
        let path = self.page_list_path(manga_id, chapter_url);
        let raw = fs::read(&path).ok()?;
        touch(&path);
        serde_json::from_slice(&raw).ok()
    }

    /// Stores a page list atomically.
    pub fn put_page_list(&self, manga_id: i64, chapter_url: &str, pages: &[PageEntry]) -> io::Result<()> {
        let raw = serde_json::to_vec(pages).map_err(io::Error::other)?;
        write_atomic(&self.page_list_path(manga_id, chapter_url), &raw)?;
        self.evict().map(|_| ())
    }

    /// Whether the image bytes for `image_url` are cached.
    pub fn has_image(&self, image_url: &str) -> bool {
        self.image_path(image_url).is_file()
    }

    /// The path of a cached image, with its recency refreshed, or `None` when it is not cached.
    pub fn read_image(&self, image_url: &str) -> Option<PathBuf> {
        let path = self.image_path(image_url);
        path.is_file().then(|| {
            touch(&path);
            path
        })
    }

    /// Stores image bytes atomically, then evicts the least recently used files over the limit.
    pub fn put_image(&self, image_url: &str, bytes: &[u8]) -> io::Result<()> {
        write_atomic(&self.image_path(image_url), bytes)?;
        self.evict().map(|_| ())
    }

    #[cfg(test)]
    /// Total size of the cached files in bytes.
    pub fn size(&self) -> io::Result<u64> {
        Ok(self.entries()?.iter().map(|e| e.len).sum())
    }

    /// Removes least recently used files until the total is within the limit. Returns the bytes removed.
    pub fn evict(&self) -> io::Result<u64> {
        let mut entries = self.entries()?;
        let mut total: u64 = entries.iter().map(|e| e.len).sum();
        if total <= self.limit {
            return Ok(0);
        }
        entries.sort_by_key(|e| e.modified);
        let mut removed = 0;
        for entry in entries {
            if total <= self.limit {
                break;
            }
            fs::remove_file(&entry.path)?;
            total -= entry.len;
            removed += entry.len;
        }
        Ok(removed)
    }

    /// Forgets one chapter (its Reload action): the page list of `chapter_url` and every image it names, so the next
    /// open asks the site again. Returns how many files were removed.
    pub fn forget_chapter(&self, manga_id: i64, chapter_url: &str) -> usize {
        let mut gone = 0;
        for page in self.get_page_list(manga_id, chapter_url).unwrap_or_default() {
            for url in [Some(page.url.as_str()), page.image_url.as_deref()].into_iter().flatten() {
                gone += usize::from(fs::remove_file(self.image_path(url)).is_ok());
            }
        }
        gone + usize::from(fs::remove_file(self.page_list_path(manga_id, chapter_url)).is_ok())
    }

    fn entries(&self) -> io::Result<Vec<Entry>> {
        let mut out = Vec::new();
        for item in fs::read_dir(&self.root)? {
            let item = item?;
            let meta = item.metadata()?;
            if !meta.is_file() {
                continue;
            }
            let modified = meta.modified().unwrap_or(UNIX_EPOCH);
            out.push(Entry { path: item.path(), len: meta.len(), modified });
        }
        Ok(out)
    }
}

struct Entry {
    path: PathBuf,
    len: u64,
    modified: SystemTime,
}

/// Writes `bytes` to a sibling temp file and renames it over `path`.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let tmp = path.with_extension(format!("tmp-{nanos}"));
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Marks a file as just used, so eviction keeps it longer. Failure is harmless: only the order changes.
fn touch(path: &Path) {
    if let Ok(file) = fs::OpenOptions::new().write(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

/// Sets a file's mtime. Used by the eviction tests to order files without sleeping.
#[cfg(test)]
pub(crate) fn set_mtime(path: &Path, at: SystemTime) {
    fs::OpenOptions::new().write(true).open(path).unwrap().set_modified(at).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("reader-cache-tests").join(name);
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn keys_are_md5_of_the_documented_inputs() {
        assert_eq!(image_key("https://a/1.jpg"), md5_hex(b"https://a/1.jpg"));
        assert_eq!(page_list_key(7, "/ch/1"), md5_hex(b"7/ch/1"));
        assert_eq!(page_list_key(7, "/ch/1").len(), 32);
    }

    #[test]
    fn page_list_round_trips_and_is_written_when_fetched() {
        let cache = DiskCache::open(scratch("pages"), CACHE_LIMIT_BYTES).unwrap();
        assert!(cache.get_page_list(1, "/c").is_none());
        let pages = vec![
            PageEntry { url: "p0".into(), image_url: None },
            PageEntry { url: "p1".into(), image_url: Some("http://i/1.jpg".into()) },
        ];
        cache.put_page_list(1, "/c", &pages).unwrap();
        assert_eq!(cache.get_page_list(1, "/c"), Some(pages));
    }

    #[test]
    fn image_write_is_atomic_and_leaves_no_temp_file() {
        let cache = DiskCache::open(scratch("atomic"), CACHE_LIMIT_BYTES).unwrap();
        cache.put_image("http://i/a.png", b"bytes").unwrap();
        assert!(cache.has_image("http://i/a.png"));
        assert_eq!(fs::read(cache.image_path("http://i/a.png")).unwrap(), b"bytes");
        let leftovers = fs::read_dir(cache.root()).unwrap().filter(|e| {
            e.as_ref().unwrap().file_name().to_string_lossy().contains("tmp")
        });
        assert_eq!(leftovers.count(), 0);
    }

    #[test]
    fn forgetting_a_chapter_removes_its_page_list_and_its_images_only() {
        let cache = DiskCache::open(scratch("forget"), CACHE_LIMIT_BYTES).unwrap();
        let pages = vec![PageEntry { url: "p0".into(), image_url: Some("http://i/0.jpg".into()) }];
        cache.put_page_list(1, "/c", &pages).unwrap();
        cache.put_image("http://i/0.jpg", b"a").unwrap();
        cache.put_image("http://i/other.jpg", b"b").unwrap();
        assert_eq!(cache.forget_chapter(1, "/c"), 2);
        assert!(cache.get_page_list(1, "/c").is_none() && !cache.has_image("http://i/0.jpg"));
        assert!(cache.has_image("http://i/other.jpg"));
    }

    #[test]
    fn eviction_removes_least_recently_used_first_until_within_limit() {
        let cache = DiskCache::open(scratch("lru"), 100).unwrap();
        let base = SystemTime::now() - Duration::from_secs(1000);
        for (i, url) in ["old", "mid", "new"].iter().enumerate() {
            cache.put_image(url, &[b'x'; 40]).unwrap();
            set_mtime(&cache.image_path(url), base + Duration::from_secs(i as u64 * 10));
        }
        // 120 bytes against a 100 byte limit: the oldest goes
        cache.evict().unwrap();
        assert!(!cache.has_image("old"));
        assert!(cache.has_image("mid"));
        assert!(cache.has_image("new"));
        assert!(cache.size().unwrap() <= 100);
    }

    #[test]
    fn reading_refreshes_recency_so_a_read_page_survives_eviction() {
        let cache = DiskCache::open(scratch("touch"), 100).unwrap();
        let base = SystemTime::now() - Duration::from_secs(1000);
        cache.put_image("a", &[1; 40]).unwrap();
        set_mtime(&cache.image_path("a"), base);
        cache.put_image("b", &[2; 40]).unwrap();
        set_mtime(&cache.image_path("b"), base + Duration::from_secs(5));
        touch(&cache.image_path("a"));
        cache.put_image("c", &[3; 40]).unwrap();
        assert!(cache.has_image("a"), "a was used last, so it must not be the eviction victim");
        assert!(!cache.has_image("b"));
    }
}
