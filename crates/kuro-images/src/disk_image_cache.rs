//! Disk cache for non-library images, equivalent to Coil's disk cache (area:34 section 6.2).
//!
//! Entries live at `<root>/<md5(key)>`. A write goes to `<name>.part` and is renamed into place, and a
//! second writer of the same key waits for the first. Eviction removes the oldest files (by mtime) until the
//! total is under the budget.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::SystemTime;

use tokio::sync::Mutex;

use crate::cover_file::write_atomic;
use crate::keys::hash_key;

const MIB: u64 = 1024 * 1024;
/// Coil 3.1.0 defaults: 2 percent of free space, clamped to [10 MiB, 250 MiB].
const FREE_FRACTION: f64 = 0.02;
const MIN_BYTES: u64 = 10 * MIB;
const MAX_BYTES: u64 = 250 * MIB;

/// The budget for a disk with `free` bytes available. Unknown free space gives the minimum.
pub fn budget_for(free: Option<u64>) -> u64 {
    match free {
        None => MIN_BYTES,
        Some(free) => ((free as f64 * FREE_FRACTION) as u64).clamp(MIN_BYTES, MAX_BYTES),
    }
}

pub struct DiskImageCache {
    root: PathBuf,
    /// Appended to every entry name: empty for the image cache, `.0` for the chapter cache (DiskLruCache value 0).
    suffix: &'static str,
    max_bytes: u64,
    /// Total size of the finished entries. Writes and removals update it under this lock.
    total: Mutex<u64>,
    /// One lock per entry name that is being written (edge E-IMG-29).
    writers: StdMutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl DiskImageCache {
    /// Opens the cache with the budget of the disk that holds `root`.
    pub async fn open(root: PathBuf) -> io::Result<Self> {
        tokio::fs::create_dir_all(&root).await?;
        let max = budget_for(kuro_platform::storage::free_bytes(&root));
        Self::open_with_budget(root, max).await
    }

    /// Opens the cache with an explicit budget. Stale `.part` files from an interrupted write are removed,
    /// and the total is counted from the files present.
    pub async fn open_with_budget(root: PathBuf, max_bytes: u64) -> io::Result<Self> {
        Self::open_named(root, max_bytes, "").await
    }

    /// Like `open_with_budget`, with `suffix` added to every `<md5(key)>` entry name.
    pub async fn open_named(
        root: PathBuf,
        max_bytes: u64,
        suffix: &'static str,
    ) -> io::Result<Self> {
        tokio::fs::create_dir_all(&root).await?;
        let mut total = 0u64;
        let mut dir = tokio::fs::read_dir(&root).await?;
        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "part") {
                let _ = tokio::fs::remove_file(&path).await;
            } else {
                total += entry.metadata().await?.len();
            }
        }
        Ok(Self {
            root,
            suffix,
            max_bytes,
            total: Mutex::new(total),
            writers: StdMutex::new(HashMap::new()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn name_of(&self, key: &str) -> String {
        format!("{}{}", hash_key(key), self.suffix)
    }

    /// Where the entry for `key` lives, whether or not it exists.
    pub fn path_for(&self, key: &str) -> PathBuf {
        self.root.join(self.name_of(key))
    }

    /// The file of a cached entry, or `None` when the key is not cached. A hit refreshes the file's mtime,
    /// which is what eviction orders by.
    pub async fn get(&self, key: &str) -> Option<PathBuf> {
        let path = self.path_for(key);
        let file = tokio::fs::File::open(&path).await.ok()?;
        let _ = file.into_std().await.set_modified(SystemTime::now());
        Some(path)
    }

    /// Writes the bytes as the entry for `key`, then evicts old entries if the total is over the budget.
    /// A failed write is returned, and no partial entry is left behind.
    pub async fn put(&self, key: &str, bytes: &[u8]) -> io::Result<PathBuf> {
        let name = self.name_of(key);
        let slot = self.writer_slot(&name);
        let result = {
            let _writing = slot.lock().await;
            self.write_entry(&name, bytes).await
        };
        self.release_slot(&name, slot);
        result
    }

    /// Removes the entry for `key`. `true` when it existed.
    pub async fn remove(&self, key: &str) -> io::Result<bool> {
        let path = self.path_for(key);
        let mut total = self.total.lock().await;
        let len = match tokio::fs::metadata(&path).await {
            Ok(meta) => meta.len(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        match tokio::fs::remove_file(&path).await {
            Ok(()) => {
                *total = total.saturating_sub(len);
                Ok(true)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Removes every finished entry and returns how many were removed. A `.part` file is
    /// left alone, because a write in progress may own it.
    pub async fn clear(&self) -> io::Result<usize> {
        let mut total = self.total.lock().await;
        let mut removed = 0;
        let mut dir = tokio::fs::read_dir(&self.root).await?;
        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "part") {
                continue;
            }
            let len = entry.metadata().await?.len();
            match tokio::fs::remove_file(&path).await {
                Ok(()) => {
                    removed += 1;
                    *total = total.saturating_sub(len);
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(removed)
    }

    /// Bytes held by finished entries.
    pub async fn size(&self) -> u64 {
        *self.total.lock().await
    }

    async fn write_entry(&self, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
        let target = self.root.join(name);
        let old = tokio::fs::metadata(&target)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        write_atomic(&target, bytes).await?;
        let mut total = self.total.lock().await;
        *total = total.saturating_sub(old) + bytes.len() as u64;
        self.evict(&mut total, &target).await;
        Ok(target)
    }

    /// Removes the oldest entries, never `keep`, until the total is within the budget.
    async fn evict(&self, total: &mut u64, keep: &Path) {
        if *total <= self.max_bytes {
            return;
        }
        let Ok(mut dir) = tokio::fs::read_dir(&self.root).await else {
            return;
        };
        let mut files: Vec<(SystemTime, PathBuf, u64)> = Vec::new();
        while let Ok(Some(entry)) = dir.next_entry().await {
            let path = entry.path();
            if path == keep || path.extension().is_some_and(|ext| ext == "part") {
                continue;
            }
            if let Ok(meta) = entry.metadata().await {
                let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                files.push((modified, path, meta.len()));
            }
        }
        files.sort_by_key(|(modified, _, _)| *modified);
        for (_, path, len) in files {
            if *total <= self.max_bytes {
                break;
            }
            if tokio::fs::remove_file(&path).await.is_ok() {
                *total = total.saturating_sub(len);
            }
        }
    }

    fn writer_slot(&self, name: &str) -> Arc<Mutex<()>> {
        let mut writers = self.writers.lock().unwrap_or_else(|e| e.into_inner());
        writers.entry(name.to_owned()).or_default().clone()
    }

    /// Drops the slot once nobody else holds it, so the map does not grow with every key ever written.
    fn release_slot(&self, name: &str, slot: Arc<Mutex<()>>) {
        let mut writers = self.writers.lock().unwrap_or_else(|e| e.into_inner());
        // The map holds one clone and `slot` is the other, so 2 means no waiter.
        if Arc::strong_count(&slot) == 2 {
            writers.remove(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn budget_follows_coil_defaults() {
        assert_eq!(budget_for(None), 10 * MIB);
        assert_eq!(budget_for(Some(100 * MIB)), 10 * MIB);
        assert_eq!(budget_for(Some(1024 * MIB)), (1024 * MIB) / 50);
        assert_eq!(budget_for(Some(1024 * 1024 * MIB)), 250 * MIB);
    }

    #[tokio::test]
    async fn put_then_get_uses_the_md5_name() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = DiskImageCache::open_with_budget(tmp.path().join("images"), 100 * MIB)
            .await
            .unwrap();
        assert_eq!(cache.get("manga;U;1").await, None);
        let path = cache.put("manga;U;1", b"image").await.unwrap();
        assert_eq!(path, tmp.path().join("images").join(hash_key("manga;U;1")));
        assert_eq!(cache.get("manga;U;1").await, Some(path.clone()));
        assert_eq!(std::fs::read(path).unwrap(), b"image");
        assert_eq!(cache.size().await, 5);
    }

    #[tokio::test]
    async fn remove_reports_whether_the_entry_existed() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = DiskImageCache::open_with_budget(tmp.path().join("images"), 100 * MIB)
            .await
            .unwrap();
        cache.put("k", b"abc").await.unwrap();
        assert!(cache.remove("k").await.unwrap());
        assert!(!cache.remove("k").await.unwrap());
        assert_eq!(cache.size().await, 0);
    }

    #[tokio::test]
    async fn oldest_entries_are_evicted_to_stay_under_budget() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = DiskImageCache::open_with_budget(tmp.path().join("images"), 250)
            .await
            .unwrap();
        let blob = vec![7u8; 100];
        cache.put("a", &blob).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        cache.put("b", &blob).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        cache.put("c", &blob).await.unwrap();
        assert!(cache.size().await <= 250);
        assert_eq!(cache.get("a").await, None);
        assert!(cache.get("b").await.is_some());
        assert!(cache.get("c").await.is_some());
    }

    #[tokio::test]
    async fn open_removes_stale_part_files_and_counts_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("images");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("x.part"), vec![0u8; 50]).unwrap();
        std::fs::write(root.join("y"), vec![0u8; 30]).unwrap();
        let cache = DiskImageCache::open_with_budget(root.clone(), 100 * MIB)
            .await
            .unwrap();
        assert!(!root.join("x.part").exists());
        assert_eq!(cache.size().await, 30);
    }

    #[tokio::test]
    async fn two_writers_of_one_key_both_finish_with_a_whole_file() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Arc::new(
            DiskImageCache::open_with_budget(tmp.path().join("images"), 100 * MIB)
                .await
                .unwrap(),
        );
        let (a, b) = {
            let (c1, c2) = (cache.clone(), cache.clone());
            tokio::join!(async move { c1.put("same", b"aaaa").await }, async move {
                c2.put("same", b"bbbb").await
            },)
        };
        assert!(a.is_ok() && b.is_ok());
        let body = std::fs::read(cache.get("same").await.unwrap()).unwrap();
        assert!(body == b"aaaa" || body == b"bbbb");
        assert!(cache.writers.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_write_is_returned_and_leaves_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("images");
        std::fs::write(&root, b"a file where the folder should be").unwrap();
        assert!(
            DiskImageCache::open_with_budget(root.clone(), 100 * MIB)
                .await
                .is_err()
        );
        let blocked = tmp.path().join("blocked");
        std::fs::create_dir_all(&blocked).unwrap();
        let cache = DiskImageCache::open_with_budget(blocked.clone(), 100 * MIB)
            .await
            .unwrap();
        std::fs::remove_dir_all(&blocked).unwrap();
        std::fs::write(&blocked, b"now a file").unwrap();
        assert!(cache.put("k", b"x").await.is_err());
        assert_eq!(cache.size().await, 0);
    }
}
