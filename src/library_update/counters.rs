//! The pass's preference writes (T-26a-025, T-26a-026): manga updates add to `library_unread_updates_count`, anime
//! updates to `library_unseen_updates_count`, and the pass start stores `library_update_last_timestamp`.
//! `PrefsHost` wraps the notification host and does these writes before it forwards each call.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use kuro_platform::kind::Kind;
use kuro_prefs::Prefs;
use kuro_prefs::pref::Pref;

use crate::library_update::pass::{EntryUpdate, Failure, Host};
use crate::library_update::source::SourceItem;

pub struct PrefsHost<H> {
    inner: H,
    last_updated: Pref<i64>,
    manga_count: Pref<i32>,
    anime_count: Pref<i32>,
    /// Serialises the read-modify-write of the two counters.
    counters: Mutex<()>,
}

impl<H> PrefsHost<H> {
    pub fn new(inner: H, prefs: &Prefs) -> Self {
        let library = &prefs.library;
        Self {
            inner,
            last_updated: library.last_updated_timestamp.clone(),
            manga_count: library.new_manga_updates_count.clone(),
            anime_count: library.new_anime_updates_count.clone(),
            counters: Mutex::new(()),
        }
    }

    #[cfg(test)]
    pub fn inner(&self) -> &H {
        &self.inner
    }
}

impl<H: Host> Host for PrefsHost<H> {
    fn set_last_update_ms(&self, now_ms: i64) {
        if let Err(error) = self.last_updated.set(now_ms) {
            tracing::warn!(%error, "could not save the library update timestamp");
        }
        self.inner.set_last_update_ms(now_ms);
    }

    fn add_new_updates(&self, kind: Kind, count: usize) {
        let _guard = self.counters.lock().unwrap_or_else(|p| p.into_inner());
        let add = i32::try_from(count).unwrap_or(i32::MAX);
        let (pref, name) = match kind {
            Kind::Manga => (&self.manga_count, "manga"),
            Kind::Anime => (&self.anime_count, "anime"),
        };
        if let Err(error) = pref.set(pref.get().saturating_add(add)) {
            tracing::warn!(%error, name, "could not save the library update counter");
        }
        self.inner.add_new_updates(kind, count);
    }

    fn show_progress(&self, kind: Kind) {
        self.inner.show_progress(kind);
    }

    fn cancel_progress(&self, kind: Kind) {
        self.inner.cancel_progress(kind);
    }

    fn show_summary(&self, kind: Kind, updates: &[EntryUpdate], hide_content: bool) {
        self.inner.show_summary(kind, updates, hide_content);
    }

    fn enqueue_downloads(&self, kind: Kind, entry_id: i64, items: &[SourceItem]) {
        self.inner.enqueue_downloads(kind, entry_id, items);
    }

    fn start_downloads(&self, kind: Kind) {
        self.inner.start_downloads(kind);
    }

    fn write_error_log(&self, kind: Kind, failures: &[Failure]) -> Option<PathBuf> {
        self.inner.write_error_log(kind, failures)
    }

    fn show_error(&self, kind: Kind, failed: usize, log: Option<&Path>) {
        self.inner.show_error(kind, failed, log);
    }

    fn show_queue_warning(&self, kind: Kind) {
        self.inner.show_queue_warning(kind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library_update::entry::Deps;
    use crate::library_update::fake::{FakeHost, FakeStore, FakeSync, entry, item};
    use crate::library_update::pass::{PassRunner, PassSettings, Trigger};
    use crate::library_update::queue::QueueRestrictions;
    use crate::library_update::source::fake::{FakeRegistry, FakeSource};
    use std::sync::atomic::Ordering;

    type Runner = PassRunner<FakeRegistry, FakeStore, FakeSync, PrefsHost<FakeHost>>;

    fn run(prefs: &Prefs, store: FakeStore, registry: FakeRegistry) -> Runner {
        let host = PrefsHost::new(FakeHost::default(), prefs);
        PassRunner::new(Deps { registry, store, sync: FakeSync::default(), host })
    }

    fn settings(kind: Kind) -> PassSettings {
        PassSettings { restrictions: QueueRestrictions::default(), ..PassSettings::new(kind) }
    }

    #[tokio::test]
    async fn a_manga_pass_adds_to_the_unread_counter_and_stores_the_start_time() {
        let prefs = Prefs::in_memory();
        prefs.library.new_manga_updates_count.set(4).unwrap();
        let mut registry = FakeRegistry::default();
        registry.sources.insert(1, FakeSource::ok(vec![item(1.0, 0), item(2.0, 1)]));
        let store = FakeStore::with(vec![entry(1, 1, "A"), entry(2, 1, "B")]);
        let runner = run(&prefs, store, registry);
        runner.run(settings(Kind::Manga), Trigger::Manual).await;
        assert_eq!(prefs.library.new_manga_updates_count.get(), 4 + 4);
        assert_eq!(prefs.library.new_anime_updates_count.get(), 0);
        assert!(prefs.library.last_updated_timestamp.get() > 0);
    }

    #[tokio::test]
    async fn an_empty_pass_still_stores_the_timestamp_and_leaves_the_counters() {
        let prefs = Prefs::in_memory();
        let runner = run(&prefs, FakeStore::default(), FakeRegistry::default());
        runner.run(settings(Kind::Manga), Trigger::Manual).await;
        assert!(prefs.library.last_updated_timestamp.get() > 0);
        assert_eq!(prefs.library.new_manga_updates_count.get(), 0);
    }

    #[test]
    fn anime_updates_add_to_the_unseen_counter_only() {
        let prefs = Prefs::in_memory();
        let host = PrefsHost::new(FakeHost::default(), &prefs);
        host.add_new_updates(Kind::Anime, 3);
        host.add_new_updates(Kind::Anime, 2);
        assert_eq!(prefs.library.new_anime_updates_count.get(), 5);
        assert_eq!(prefs.library.new_manga_updates_count.get(), 0);
        host.set_last_update_ms(1234);
        assert_eq!(prefs.library.last_updated_timestamp.get(), 1234);
        assert_eq!(host.inner().last_timestamp.load(Ordering::SeqCst), 1234);
    }
}
