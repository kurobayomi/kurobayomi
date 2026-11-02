//! Download deletion for a list of items of either kind (`DeleteChapterDownload.kt`, `DeleteEpisodeDownload.kt`;
//! T-10a-015). The interactor resolves the entry's source in the registry and hands the whole list to the download manager
//! in one call. A source that is not installed is not an error: the interactor logs a warning and deletes nothing
//! (EC-27). The deletion runs under [`non_cancellable`], so a closed screen does not stop it (EC-29).
//!
//! The download manager is a port ([`ItemDownloadFacade`]) because `kuro-downloads` sits above this crate. The interactor
//! also implements the synchronous [`ItemDownloads`] port that `SetDoneStatus` calls, so it is the production adapter of
//! that trait. The adapter hands the work to the current tokio runtime and does not wait for it. One interactor serves
//! the registry of one kind (`S` is that kind's source type).

use std::sync::Arc;

use async_trait::async_trait;
use kuro_data::entry::Entry;
use kuro_data::item::Item;

use crate::app::rx::non_cancellable;
use crate::items::status::ItemDownloads;
use crate::source::registry::{InstalledSource, SourceRegistry};

/// What the interactor needs from a download manager (`DownloadManager.deleteChapters`, `deleteEpisodes`).
#[async_trait]
pub trait ItemDownloadFacade<S: ?Sized>: Send + Sync {
    /// Deletes the downloaded files of `items` of `entry`, which `source` serves.
    async fn delete_items(&self, items: &[Item], entry: &Entry, source: Arc<S>);
}

/// `DeleteChapterDownload` and `DeleteEpisodeDownload`.
pub struct DeleteItemDownload<S: ?Sized> {
    sources: Arc<SourceRegistry<S>>,
    downloads: Arc<dyn ItemDownloadFacade<S>>,
}

impl<S: ?Sized> Clone for DeleteItemDownload<S> {
    fn clone(&self) -> Self {
        Self { sources: Arc::clone(&self.sources), downloads: Arc::clone(&self.downloads) }
    }
}

impl<S: InstalledSource + ?Sized + 'static> DeleteItemDownload<S> {
    pub fn new(sources: Arc<SourceRegistry<S>>, downloads: Arc<dyn ItemDownloadFacade<S>>) -> Self {
        Self { sources, downloads }
    }

    /// `awaitAll(entry, items)`: one `delete_items` call with the full slice. An absent source logs a warning and
    /// returns without deleting (EC-27).
    pub async fn await_all(&self, entry: &Entry, items: &[Item]) {
        let Some(source) = self.sources.get(entry.source) else {
            tracing::warn!(entry_id = entry.id, source_id = entry.source, "no source for the entry; downloads are kept");
            return;
        };
        let downloads = Arc::clone(&self.downloads);
        let entry = entry.clone();
        let items = items.to_vec();
        non_cancellable(async move { downloads.delete_items(&items, &entry, source).await }).await;
    }
}

impl<S: InstalledSource + ?Sized + 'static> ItemDownloads for DeleteItemDownload<S> {
    fn delete(&self, e: &Entry, items: &[Item]) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::warn!(entry_id = e.id, "no runtime for the download deletion; downloads are kept");
            return;
        };
        let this = self.clone();
        let entry = e.clone();
        let items = items.to_vec();
        runtime.spawn(async move { this.await_all(&entry, &items).await });
    }
}

#[cfg(test)]
mod tests {
    use kuro_platform::kind::Kind;
    use std::sync::Mutex;

    use super::*;
    use crate::source::fakes::{FakeCatalog, Installed, MemStubStore};
    use crate::source::model::{Id, Source};

    /// Records `(entry id, item ids, source id)` of every deletion call.
    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<(i64, Vec<i64>, Id)>>,
    }

    #[async_trait]
    impl ItemDownloadFacade<Installed> for Recorder {
        async fn delete_items(&self, items: &[Item], entry: &Entry, source: Arc<Installed>) {
            // A yield point, so a dropped caller would cancel an interrupted deletion.
            tokio::task::yield_now().await;
            self.calls.lock().unwrap().push((entry.id, items.iter().map(|i| i.id).collect(), source.id));
        }
    }

    fn registry(kind: Kind, installed: &[Id]) -> Arc<SourceRegistry<Installed>> {
        let registry =
            SourceRegistry::new(kind, Arc::new(MemStubStore::default()), Arc::new(FakeCatalog::default()));
        registry.finish_loading(
            installed
                .iter()
                .map(|&id| {
                    Arc::new(Installed {
                        id,
                        http: true,
                        catalogue: true,
                        source: Source::new(id, &format!("S{id}"), "en"),
                    })
                })
                .collect(),
        );
        Arc::new(registry)
    }

    fn entry(kind: Kind, id: i64, source: i64) -> Entry {
        Entry { id, source, ..Entry::create(kind) }
    }

    fn items(kind: Kind, ids: &[i64]) -> Vec<Item> {
        ids.iter().map(|&id| Item { id, ..Item::create(kind) }).collect()
    }

    /// Captures what the subscriber logs on this thread.
    #[derive(Clone, Default)]
    struct Logs(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Logs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logs {
        type Writer = Logs;
        fn make_writer(&'a self) -> Logs {
            self.clone()
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime")
    }

    #[test]
    fn the_whole_slice_goes_to_one_call_with_the_resolved_source() {
        let downloads = Arc::new(Recorder::default());
        let interactor = DeleteItemDownload::new(registry(Kind::Manga, &[7]), downloads.clone());
        runtime().block_on(interactor.await_all(&entry(Kind::Manga, 3, 7), &items(Kind::Manga, &[1, 2, 3])));
        assert_eq!(*downloads.calls.lock().unwrap(), vec![(3, vec![1, 2, 3], 7)]);
    }

    #[test]
    fn an_absent_source_logs_a_warning_and_deletes_nothing() {
        let downloads = Arc::new(Recorder::default());
        let interactor = DeleteItemDownload::new(registry(Kind::Manga, &[7]), downloads.clone());
        let logs = Logs::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::WARN)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            runtime().block_on(interactor.await_all(&entry(Kind::Manga, 3, 99), &items(Kind::Manga, &[1])));
        });
        assert!(downloads.calls.lock().unwrap().is_empty(), "EC-27");
        let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        assert!(text.contains("WARN") && text.contains("no source for the entry"), "{text}");
    }

    #[test]
    fn the_deletion_finishes_when_the_caller_stops_waiting() {
        let downloads = Arc::new(Recorder::default());
        let interactor = DeleteItemDownload::new(registry(Kind::Manga, &[7]), downloads.clone());
        runtime().block_on(async {
            let (entry, rows) = (entry(Kind::Manga, 3, 7), items(Kind::Manga, &[1]));
            let work = interactor.await_all(&entry, &rows);
            // The caller gives up after one poll; the spawned deletion still lands.
            let _ = tokio::time::timeout(std::time::Duration::ZERO, work).await;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        });
        assert_eq!(downloads.calls.lock().unwrap().len(), 1, "EC-29");
    }

    #[test]
    fn the_status_port_adapter_hands_the_items_to_the_interactor() {
        let downloads = Arc::new(Recorder::default());
        let interactor = DeleteItemDownload::new(registry(Kind::Manga, &[7]), downloads.clone());
        runtime().block_on(async {
            ItemDownloads::delete(&interactor, &entry(Kind::Manga, 4, 7), &items(Kind::Manga, &[5, 6]));
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        });
        assert_eq!(*downloads.calls.lock().unwrap(), vec![(4, vec![5, 6], 7)]);
    }

    #[test]
    fn the_anime_registry_has_the_same_contract() {
        let downloads = Arc::new(Recorder::default());
        let interactor = DeleteItemDownload::new(registry(Kind::Anime, &[9]), downloads.clone());
        let episodes = items(Kind::Anime, &[10, 11]);
        runtime().block_on(interactor.await_all(&entry(Kind::Anime, 2, 9), &episodes));
        assert_eq!(*downloads.calls.lock().unwrap(), vec![(2, vec![10, 11], 9)]);

        runtime().block_on(interactor.await_all(&entry(Kind::Anime, 3, 1), &episodes));
        assert_eq!(downloads.calls.lock().unwrap().len(), 1, "an absent source deletes nothing");
    }
}
