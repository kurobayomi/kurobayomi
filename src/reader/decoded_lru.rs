//! The byte-budget LRU of decoded pages (T-44a-069, RD §5.6). Pure: it holds values and their sizes and knows nothing
//! about GTK. A page that leaves the realised window drops its widget, but its decoded pyramid stays here, so coming
//! back to it costs no decode. When the budget is passed, the least recently used entries go first, and an entry that is
//! still in use (`pinned`: a visible page holds a share of it) is never evicted.

use std::collections::HashMap;
use std::hash::Hash;

/// The most a reader keeps decoded: 512 MiB of pixels (RD §5.6).
pub const MAX_BUDGET_BYTES: usize = 512 * 1024 * 1024;
/// The least it keeps, enough for a few full spreads on a small machine.
pub const MIN_BUDGET_BYTES: usize = 128 * 1024 * 1024;

/// The decoded-page budget for a machine with `total` bytes of memory: a sixteenth of it, between
/// [`MIN_BUDGET_BYTES`] and [`MAX_BUDGET_BYTES`]; the maximum when the total is unknown.
pub fn budget_for(total: Option<u64>) -> usize {
    total.map_or(MAX_BUDGET_BYTES, |bytes| usize::try_from(bytes / 16).unwrap_or(usize::MAX).clamp(MIN_BUDGET_BYTES, MAX_BUDGET_BYTES))
}

struct Entry<V> {
    value: V,
    bytes: usize,
    used: u64,
}

pub struct ByteLru<K, V> {
    budget: usize,
    total: usize,
    clock: u64,
    entries: HashMap<K, Entry<V>>,
}

impl<K: Hash + Eq + Clone, V: Clone> ByteLru<K, V> {
    pub fn new(budget: usize) -> Self {
        Self { budget, total: 0, clock: 0, entries: HashMap::new() }
    }

    /// The entry for `key`, marked as the most recently used.
    pub fn get(&mut self, key: &K) -> Option<V> {
        self.clock += 1;
        let clock = self.clock;
        self.entries.get_mut(key).map(|e| {
            e.used = clock;
            e.value.clone()
        })
    }

    /// Stores `value` (`bytes` large), replacing an older entry of the key, then evicts least recently used entries
    /// while the total is over budget. The new entry and every entry `pinned` answers true for stay. Returns the keys
    /// that were evicted.
    pub fn insert(&mut self, key: K, value: V, bytes: usize, pinned: impl Fn(&V) -> bool) -> Vec<K> {
        self.clock += 1;
        if let Some(old) = self.entries.insert(key.clone(), Entry { value, bytes, used: self.clock }) {
            self.total -= old.bytes;
        }
        self.total += bytes;
        let mut evicted = Vec::new();
        while self.total > self.budget {
            let oldest = self.entries.iter().filter(|(k, e)| **k != key && !pinned(&e.value)).min_by_key(|(_, e)| e.used).map(|(k, _)| k.clone());
            let Some(oldest) = oldest else { break };
            if let Some(gone) = self.entries.remove(&oldest) {
                self.total -= gone.bytes;
            }
            evicted.push(oldest);
        }
        evicted
    }

    /// Drops every entry (a new reader session).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.total = 0;
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg(test)]
    /// The bytes held.
    pub fn bytes(&self) -> usize {
        self.total
    }

    #[cfg(test)]
    pub fn contains(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_budget_is_a_sixteenth_of_the_memory_within_its_bounds() {
        const GIB: u64 = 1024 * 1024 * 1024;
        assert_eq!(budget_for(Some(16 * GIB)), MAX_BUDGET_BYTES, "a big machine keeps the most");
        assert_eq!(budget_for(Some(4 * GIB)), 256 * 1024 * 1024);
        assert_eq!(budget_for(Some(GIB)), MIN_BUDGET_BYTES, "a small machine keeps the least");
        assert_eq!(budget_for(None), MAX_BUDGET_BYTES);
    }

    use super::*;
    use std::rc::Rc;

    fn unpinned(_: &Rc<u8>) -> bool {
        false
    }

    fn in_use(v: &Rc<u8>) -> bool {
        Rc::strong_count(v) > 1
    }

    #[test]
    fn entries_within_the_budget_all_stay() {
        let mut lru = ByteLru::new(100);
        lru.insert("a", Rc::new(1), 40, unpinned);
        lru.insert("b", Rc::new(2), 40, unpinned);
        assert_eq!((lru.len(), lru.bytes()), (2, 80));
    }

    #[test]
    fn passing_the_budget_evicts_the_least_recently_used_first() {
        let mut lru = ByteLru::new(100);
        lru.insert("a", Rc::new(1), 40, unpinned);
        lru.insert("b", Rc::new(2), 40, unpinned);
        assert!(lru.get(&"a").is_some(), "a is now newer than b");
        let evicted = lru.insert("c", Rc::new(3), 40, unpinned);
        assert_eq!(evicted, ["b"]);
        assert!(lru.contains(&"a") && lru.contains(&"c") && !lru.contains(&"b"));
        assert_eq!(lru.bytes(), 80);
    }

    #[test]
    fn a_pinned_entry_is_never_evicted_even_over_budget() {
        let mut lru = ByteLru::new(50);
        let visible = Rc::new(1u8);
        lru.insert("visible", Rc::clone(&visible), 40, in_use);
        let evicted = lru.insert("next", Rc::new(2), 40, in_use);
        assert!(evicted.is_empty(), "the visible page is in use and the new one is kept");
        assert_eq!(lru.bytes(), 80);
        drop(visible);
        let evicted = lru.insert("later", Rc::new(3), 10, in_use);
        assert_eq!(evicted, ["visible"], "once released, the old entry is the first to go");
    }

    #[test]
    fn the_new_entry_is_kept_even_when_it_alone_is_over_budget() {
        let mut lru = ByteLru::new(10);
        let evicted = lru.insert("big", Rc::new(1), 500, unpinned);
        assert!(evicted.is_empty());
        assert!(lru.contains(&"big"));
    }

    #[test]
    fn replacing_a_key_swaps_its_size() {
        let mut lru = ByteLru::new(100);
        lru.insert("a", Rc::new(1), 60, unpinned);
        lru.insert("a", Rc::new(2), 30, unpinned);
        assert_eq!((lru.len(), lru.bytes()), (1, 30));
        assert_eq!(*lru.get(&"a").unwrap(), 2);
    }

    #[test]
    fn clear_empties_it() {
        let mut lru = ByteLru::new(100);
        lru.insert("a", Rc::new(1), 60, unpinned);
        lru.clear();
        assert!(lru.is_empty());
        assert_eq!(lru.bytes(), 0);
    }
}
