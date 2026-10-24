//! The download service, started by the shell once the data has opened (docs/design/unified-downloads.md): one queue for
//! both kinds, saved in the `download_queue` table. Closing the app while a lane runs asks first (`confirm_close`); there
//! is no background process, so a run stops when the app exits and the saved queue continues at the next start.

pub mod anime_io;
pub mod directory;
pub mod live;
pub mod manga_io;
pub mod platform;
pub mod queue_page;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use kuro_db::handle::Db;
use kuro_domain::download::prefs::DownloadPrefs;
use kuro_downloads::anime::downloader::EpisodeEngine;
use kuro_downloads::manga::runner::ChapterPipeline;
use kuro_downloads::gate::Connectivity;
use kuro_downloads::index::{DownloadIndex, IndexRoots};
use kuro_downloads::layout::Layout;
use kuro_downloads::notify::spawn_notifier;
use kuro_downloads::pending::PendingStore;
use kuro_downloads::queue::{Layouts, Queue, QueueParams};
use kuro_downloads::runner::UnitRunner;
use kuro_downloads::service::Downloads;
use kuro_downloads::state::RunState;
use kuro_downloads::store::QueueStore;
use kuro_downloads::watch::RootWatch;
use kuro_downloads::notice::NotifierSink;
use kuro_platform::kind::Kind;
use kuro_platform::notifier::Notifier;
use kuro_platform::paths::Paths;
use kuro_prefs::network::NetworkPrefs;
use kuro_prefs::store::PrefStore;
use tokio::sync::watch;

use self::anime_io::{HttpVideoFetch, LiveHosters};
use self::directory::AppDirectory;
use self::platform::NetworkFeed;

/// The platform free-space query the queue uses for the low-storage check.
pub fn free_space(path: &Path) -> Option<u64> {
    kuro_platform::storage::free_bytes(path)
}

/// What [`start_service`] keeps alive: the service, the folder watch and the network feed.
pub struct Service {
    pub downloads: Downloads,
    _watch: Option<RootWatch>,
    _network: NetworkFeed,
}

/// Makes the Library selection bar's Download menu queue the units (both kinds): see `library::actions::download_selection`.
/// Call it on the GTK thread; the work runs on the blocking pool.
pub fn wire_library_download(db: Arc<Db>) {
    crate::library::mount::on_download(move |kind, entry_ids, amount| {
        let db = db.clone();
        crate::updates::download_host::spawn_detached(move || {
            let backend = crate::updates::download_host::current();
            crate::library::actions::download_selection(&db, &*backend, kind, &entry_ids, amount);
        });
    });
}

/// Whether any lane is downloading now: closing the app then stops it.
pub fn any_lane_running(downloads: &Downloads) -> bool {
    let snapshot = downloads.snapshot();
    Kind::ALL.into_iter().any(|kind| snapshot.run(kind) == RunState::Running)
}

/// How often the index looks at whether a folder change made it stale.
const INDEX_CHECK: std::time::Duration = std::time::Duration::from_secs(10);

/// Starts the one `Downloads` on the current runtime. The queue is saved in `download_queue` of `db`, the app database.
pub fn start_service(paths: &dyn Paths, db: Arc<Db>, prefs: PrefStore, notifier: Arc<dyn Notifier>) -> Result<Service, String> {
    let storage = crate::settings_ui::storage::manager(&prefs).ok_or("the storage folder could not be resolved")?;
    let root = |kind| storage.downloads_dir(kind).map_err(|e| format!("the {} downloads folder is not available: {e}", kind.as_str()));
    let (manga_root, anime_root) = (root(Kind::Manga)?, root(Kind::Anime)?);
    let directory = Arc::new(AppDirectory::new(db.clone()));
    let client = crate::net_client::build(&NetworkPrefs::new(prefs.clone()), paths).map_err(|e| e.to_string())?;
    let runners: Vec<Arc<dyn UnitRunner>> = vec![
        Arc::new(ChapterPipeline::new(directory.clone())),
        Arc::new(EpisodeEngine::new(Arc::new(LiveHosters(kuro_prefs::player::PlayerPrefs::new(prefs.clone())))).with_fetch(Arc::new(HttpVideoFetch::new(client)))),
    ];
    let index = DownloadIndex::open(
        IndexRoots { manga: storage.downloads_path(Kind::Manga), anime: storage.downloads_path(Kind::Anime) },
        &paths.download_index_cache_dir(),
    );
    // Read at every scheduling pass, so a change in Settings > Downloads applies to the running queue.
    let numbers = DownloadPrefs::new(&prefs);
    let (connectivity_tx, connectivity) = watch::channel(Connectivity { online: true, metered: false });
    let layouts = || Layouts { manga: Layout::new(Kind::Manga, manga_root.clone()), anime: Layout::new(Kind::Anime, anime_root.clone()) };
    let queue = Queue::spawn(QueueParams {
        db: db.clone(),
        store: Arc::new(QueueStore::new(db.clone())),
        runners,
        directory: directory.clone(),
        layouts: layouts(),
        index: index.clone(),
        free_space,
        max_active: Arc::new(move || numbers.number_of_downloads().max(1) as usize),
        connectivity,
        only_over_wifi: false,
    });
    let downloads = Downloads::new(queue, index.clone(), layouts(), Arc::new(PendingStore::new(db.clone())), directory, db);
    spawn_notifier(downloads.events(), Arc::new(NotifierSink::new(notifier)), Arc::new(AtomicBool::new(false)));
    follow_sources(&downloads);
    Ok(Service { _watch: index.watch(), downloads, _network: NetworkFeed::start(connectivity_tx) })
}

/// Keeps the index and the waiting units in step with the sources: after each load of the sources both kinds are scanned
/// again and the waiting units are asked about their source, and a folder change that made the index stale is rescanned.
fn follow_sources(downloads: &Downloads) {
    let (index, queue) = (downloads.index().clone(), downloads.clone());
    let first_load = Arc::new(AtomicBool::new(true));
    crate::recipe_sources::follow_loads(move || {
        let (index, downloads, first_load) = (index.clone(), queue.clone(), first_load.clone());
        async move {
            // a source that changed its name takes its download folder with it, before the folders are read again
            for (kind, old, new) in crate::browse::directory::take_renames() {
                if let Err(error) = downloads.rename_source(kind, &old, &new).await {
                    tracing::warn!(source = old.id, %error, "the download folder of a renamed source was not renamed");
                }
            }
            for kind in Kind::ALL {
                index.rescan(kind, crate::recipe_sources::ready(), || crate::recipe_sources::folder_names(kind)).await;
            }
            downloads.sources_changed();
            // the reader's delete-after-read waits in the pending table: once at startup, when the sources can be named
            if first_load.swap(false, std::sync::atomic::Ordering::Relaxed)
                && let Err(error) = downloads.flush_pending_deletes().await
            {
                tracing::warn!(%error, "the deletes that waited from the last run were not done");
            }
        }
    });
    let index = downloads.index().clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(INDEX_CHECK);
        loop {
            tick.tick().await;
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
            for kind in Kind::ALL {
                index.renew_if_stale(kind, now, crate::recipe_sources::ready(), || crate::recipe_sources::folder_names(kind)).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_space_reports_the_filesystem_of_an_existing_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert!(free_space(dir.path()).is_some_and(|bytes| bytes > 0));
    }
}
