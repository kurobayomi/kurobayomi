//! Per-source preferences (`ConfigurableSource.kt`, T-15a-019). Decision: values live in the app `PrefStore` under
//! `source_<id>.<key>`, the namespace `kuro-source-wasm` and `kuro-backup` already use, not in a `source_prefs` table
//! (Option A): the table would need a `kuro-db` migration, and the store already gives one
//! typed value per key plus change events. Backup reads [`SourcePrefs::entries`]; restore writes
//! [`SourcePrefs::restore`], which needs no installed source (EC-PR-07).

use std::collections::BTreeSet;

use kuro_prefs::store::PrefStore;
use kuro_prefs::values::PrefValue;
use kuro_source::preference_key;

/// The preferences of one source, `Some` only for a key that is stored with the asked kind (EC-PR-02).
#[derive(Clone)]
pub struct SourcePrefs {
    store: PrefStore,
    source_id: i64,
}

impl SourcePrefs {
    pub fn new(store: PrefStore, source_id: i64) -> Self {
        Self { store, source_id }
    }

    fn full_key(&self, key: &str) -> String {
        format!("{}.{key}", preference_key(self.source_id))
    }

    pub fn get(&self, key: &str) -> Option<PrefValue> {
        self.store.get_raw(&self.full_key(key))
    }

    pub fn set(&self, key: &str, value: PrefValue) {
        self.store.set_raw(&self.full_key(key), value);
    }

    pub fn remove(&self, key: &str) {
        self.store.remove(&self.full_key(key));
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            PrefValue::Bool(v) => Some(v),
            _ => None,
        }
    }

    pub fn set_bool(&self, key: &str, value: bool) {
        self.set(key, PrefValue::Bool(value));
    }

    pub fn get_string(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            PrefValue::Str(v) => Some(v),
            _ => None,
        }
    }

    pub fn set_string(&self, key: &str, value: impl Into<String>) {
        self.set(key, PrefValue::Str(value.into()));
    }

    pub fn get_string_set(&self, key: &str) -> Option<BTreeSet<String>> {
        match self.get(key)? {
            PrefValue::Set(v) => Some(v),
            _ => None,
        }
    }

    pub fn set_string_set(&self, key: &str, value: BTreeSet<String>) {
        self.set(key, PrefValue::Set(value));
    }

    /// Every stored `(key, value)` of this source, with the `source_<id>.` prefix cut off, in key order. Empty when the
    /// source stores nothing, which is what keeps it out of a backup.
    pub fn entries(&self) -> Vec<(String, PrefValue)> {
        let prefix = format!("{}.", preference_key(self.source_id));
        self.store
            .all()
            .into_iter()
            .filter_map(|(key, value)| key.strip_prefix(&prefix).map(|short| (short.to_owned(), value)))
            .collect()
    }

    /// Writes backed-up entries into `source_<id>`, whether or not the source is installed (EC-PR-07). One change
    /// notification for the whole batch.
    pub fn restore(&self, entries: Vec<(String, PrefValue)>) {
        let full = entries.into_iter().map(|(key, value)| (self.full_key(&key), value)).collect();
        self.store.replace_many(full);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips_and_an_absent_key_is_none() {
        let prefs = SourcePrefs::new(PrefStore::in_memory(), 7);
        assert_eq!((prefs.get_bool("a"), prefs.get_string("a"), prefs.get_string_set("a")), (None, None, None), "EC-PR-02");
        prefs.set_bool("hd", true);
        prefs.set_string("quality", "720p");
        prefs.set_string_set("langs", BTreeSet::from(["en".to_owned(), "ja".to_owned()]));
        assert_eq!(prefs.get_bool("hd"), Some(true));
        assert_eq!(prefs.get_string("quality").as_deref(), Some("720p"));
        assert_eq!(prefs.get_string_set("langs").map(|s| s.len()), Some(2));
        assert_eq!(prefs.get_string("hd"), None, "a key stored as another kind is absent for this kind");
    }

    #[test]
    fn remove_deletes_only_that_key() {
        let prefs = SourcePrefs::new(PrefStore::in_memory(), 7);
        prefs.set_bool("a", true);
        prefs.set_bool("b", true);
        prefs.remove("a");
        assert_eq!((prefs.get_bool("a"), prefs.get_bool("b")), (None, Some(true)));
    }

    #[test]
    fn sources_do_not_see_each_others_keys_and_ids_that_share_a_prefix_stay_apart() {
        let store = PrefStore::in_memory();
        let (one, ten) = (SourcePrefs::new(store.clone(), 1), SourcePrefs::new(store.clone(), 10));
        one.set_bool("k", true);
        ten.set_bool("k", false);
        assert_eq!(one.entries(), vec![("k".to_owned(), PrefValue::Bool(true))]);
        assert_eq!(ten.entries(), vec![("k".to_owned(), PrefValue::Bool(false))]);
        assert!(store.get_raw("source_1.k").is_some(), "the namespace is source_<id>");
    }

    #[test]
    fn restore_writes_all_kinds_for_a_source_nothing_has_loaded() {
        let prefs = SourcePrefs::new(PrefStore::in_memory(), -42);
        let entries = vec![
            ("i".to_owned(), PrefValue::Int(3)),
            ("l".to_owned(), PrefValue::Long(1 << 40)),
            ("f".to_owned(), PrefValue::Float(1.5)),
            ("s".to_owned(), PrefValue::Str("x".into())),
            ("b".to_owned(), PrefValue::Bool(true)),
            ("set".to_owned(), PrefValue::Set(BTreeSet::from(["a".to_owned()]))),
        ];
        prefs.restore(entries.clone());
        let mut back = prefs.entries();
        back.sort_by(|a, b| a.0.cmp(&b.0));
        let mut expected = entries;
        expected.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(back, expected, "EC-PR-04..07");
    }
}
