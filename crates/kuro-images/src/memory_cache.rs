//! Decoded image LRU cache, sized by bytes (area:34 section 6.1, T-34a-071).
//!
//! It holds shared RGBA buffers only, never a GTK object, so it is `Send + Sync`
//! and the textures are built on the main thread from the cached bytes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::decode::Decoded;
use crate::options::CachePolicy;

/// The decoded-image budget: a fixed 64 MiB instead of Coil's share of the Java heap.
pub const DEFAULT_BUDGET_BYTES: u64 = 64 * 1024 * 1024;

struct Entry {
    image: Arc<Decoded>,
    /// Counter value of the last use. The entry with the lowest value is the least recently used.
    used: u64,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<String, Entry>,
    bytes: u64,
    clock: u64,
}

pub struct MemoryCache {
    budget: u64,
    inner: Mutex<Inner>,
}

fn weight(image: &Decoded) -> u64 {
    image.rgba.len() as u64
}

impl MemoryCache {
    pub fn new(budget_bytes: u64) -> Self {
        Self {
            budget: budget_bytes,
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The image cached under `key`, marked as most recently used. A policy that does not read gives `None`
    /// without looking.
    pub fn get(&self, key: &str, policy: CachePolicy) -> Option<Arc<Decoded>> {
        if !policy.read_enabled() {
            return None;
        }
        let mut inner = self.lock();
        inner.clock += 1;
        let clock = inner.clock;
        let entry = inner.entries.get_mut(key)?;
        entry.used = clock;
        Some(entry.image.clone())
    }

    /// Stores `image` under `key`, then evicts least recently used entries until the total fits the budget. A
    /// policy that does not write stores nothing, and an image bigger than the whole budget is not kept.
    pub fn insert(&self, key: String, image: Arc<Decoded>, policy: CachePolicy) {
        let size = weight(&image);
        if !policy.write_enabled() || size > self.budget {
            return;
        }
        let mut inner = self.lock();
        inner.clock += 1;
        let used = inner.clock;
        if let Some(old) = inner.entries.insert(key, Entry { image, used }) {
            inner.bytes -= weight(&old.image);
        }
        inner.bytes += size;
        // ponytail: scanning for the oldest entry is O(n) per eviction; fine for the few hundred covers a
        // 64 MiB budget holds, an index by `used` is the upgrade if that ever shows up in a profile.
        while inner.bytes > self.budget {
            let Some(oldest) = inner
                .entries
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(gone) = inner.entries.remove(&oldest) {
                inner.bytes -= weight(&gone.image);
            }
        }
    }

    /// Drops every cached image, for a change that alters how pictures decode (the display profile).
    pub fn clear(&self) {
        let mut inner = self.lock();
        inner.entries.clear();
        inner.bytes = 0;
    }

    /// Drops the images decoded from `url` (a key is `<owner>;<url>;<stamp>`), for a cover the site replaced.
    pub fn forget_url(&self, url: &str) {
        let needle = format!(";{url};");
        let mut inner = self.lock();
        let gone: Vec<String> = inner.entries.keys().filter(|key| key.contains(&needle)).cloned().collect();
        for key in gone {
            if let Some(entry) = inner.entries.remove(&key) {
                inner.bytes -= weight(&entry.image);
            }
        }
    }

    /// Bytes held by the cached images.
    pub fn size(&self) -> u64 {
        self.lock().bytes
    }

    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    /// An image of `bytes` RGBA bytes (a 1 x n strip, so `bytes` must be a multiple of 4).
    fn image(bytes: usize) -> Arc<Decoded> {
        Arc::new(Decoded {
            width: (bytes / 4) as u32,
            height: 1,
            rgba: Bytes::from(vec![9u8; bytes]),
            is_sampled: false,
        })
    }

    const ON: CachePolicy = CachePolicy::Enabled;

    #[test]
    fn hit_returns_the_shared_buffer() {
        let cache = MemoryCache::new(1000);
        let img = image(100);
        cache.insert("a".into(), img.clone(), ON);
        let got = cache.get("a", ON).unwrap();
        assert!(Arc::ptr_eq(&got, &img));
        assert!(cache.get("b", ON).is_none());
        assert_eq!(cache.size(), 100);
    }

    #[test]
    fn forgetting_a_url_drops_every_image_decoded_from_it_only() {
        let cache = MemoryCache::new(1000);
        cache.insert("anime;https://c/a.jpg;0".into(), image(100), ON);
        cache.insert("manga;https://c/a.jpg;5".into(), image(100), ON);
        cache.insert("anime;https://c/a.jpg.th;0".into(), image(100), ON);
        cache.forget_url("https://c/a.jpg");
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.size(), 100);
    }

    #[test]
    fn least_recently_used_entry_goes_first() {
        let cache = MemoryCache::new(250);
        cache.insert("a".into(), image(100), ON);
        cache.insert("b".into(), image(100), ON);
        // Touching "a" makes "b" the oldest.
        assert!(cache.get("a", ON).is_some());
        cache.insert("c".into(), image(100), ON);
        assert!(cache.get("b", ON).is_none());
        assert!(cache.get("a", ON).is_some());
        assert!(cache.get("c", ON).is_some());
        assert!(cache.size() <= 250);
    }

    #[test]
    fn eviction_continues_until_the_new_entry_fits() {
        let cache = MemoryCache::new(300);
        for key in ["a", "b", "c"] {
            cache.insert(key.into(), image(100), ON);
        }
        cache.insert("big".into(), image(280), ON);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.size(), 280);
    }

    #[test]
    fn replacing_a_key_does_not_double_count() {
        let cache = MemoryCache::new(1000);
        cache.insert("a".into(), image(100), ON);
        cache.insert("a".into(), image(40), ON);
        assert_eq!((cache.len(), cache.size()), (1, 40));
    }

    #[test]
    fn an_image_over_the_whole_budget_is_not_kept() {
        let cache = MemoryCache::new(100);
        cache.insert("keep".into(), image(60), ON);
        cache.insert("huge".into(), image(104), ON);
        assert!(cache.get("huge", ON).is_none());
        assert!(cache.get("keep", ON).is_some());
    }

    #[test]
    fn a_disabled_policy_neither_reads_nor_writes() {
        let cache = MemoryCache::new(1000);
        cache.insert("a".into(), image(100), CachePolicy::Disabled);
        assert!(cache.is_empty());
        cache.insert("a".into(), image(100), ON);
        assert!(cache.get("a", CachePolicy::Disabled).is_none());
        assert!(cache.get("a", CachePolicy::WriteOnly).is_none());
        assert!(cache.get("a", CachePolicy::ReadOnly).is_some());
        cache.insert("b".into(), image(100), CachePolicy::ReadOnly);
        assert!(cache.get("b", ON).is_none());
        cache.insert("c".into(), image(100), CachePolicy::WriteOnly);
        assert!(cache.get("c", ON).is_some());
    }
}
