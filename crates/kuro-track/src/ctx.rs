//! Shared state of the tracking flows: the registry, the repositories, the retry queue and the per-binding
//! locks (race R1). Every flow takes `&TrackCtx`.

use kuro_platform::kind::Kind;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::{Mutex as AsyncMutex, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;

use crate::connectivity::Connectivity;
use crate::error::{TrackError, redact};
use crate::events::{EventBus, TrackEvent};
use crate::manager::TrackerManager;
use crate::model::{AnimeTrack, MangaTrack, TrackerId};
use crate::prefs::TrackPrefs;
use crate::progress::TrackRunner;
use crate::queue::RetryQueue;
use crate::queue::retry::RetryScheduler;
use crate::repo::{
    AnimeEpisodeRepo, AnimeHistoryRepo, AnimeTrackRepo, MangaChapterRepo, MangaHistoryRepo,
    MangaTrackRepo,
};

/// Lock table keyed by `(kind, entry id, tracker id)`.
type LockTable = HashMap<(Kind, i64, i64), Arc<AsyncMutex<()>>>;

/// One async lock per `(kind, entry, tracker)`. Progress updates, refreshes and the retry runner take it, so two
/// writes to one binding never interleave.
#[derive(Default)]
pub struct BindingLocks(Mutex<LockTable>);

impl BindingLocks {
    pub fn for_binding(
        &self,
        kind: Kind,
        entry_id: i64,
        tracker: TrackerId,
    ) -> Arc<AsyncMutex<()>> {
        let mut map = self.0.lock().unwrap_or_else(|p| p.into_inner());
        Arc::clone(
            map.entry((kind, entry_id, tracker.0))
                .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
        )
    }
}

/// Refresh requests one tracker serves at once (T-27a-081). The rest wait for a permit.
pub const REFRESH_PARALLELISM: usize = 4;

/// One semaphore per tracker, so a refresh of many titles never sends more than [`REFRESH_PARALLELISM`] requests to
/// one service at once. Other trackers are not held back.
#[derive(Default)]
pub struct RefreshLimit(Mutex<HashMap<i64, Arc<Semaphore>>>);

impl RefreshLimit {
    /// Takes one permit for a refresh request to `tracker`. The permit is released when it is dropped.
    pub async fn acquire(&self, tracker: TrackerId) -> Option<OwnedSemaphorePermit> {
        let sem = {
            let mut map = self.0.lock().unwrap_or_else(|p| p.into_inner());
            Arc::clone(
                map.entry(tracker.0)
                    .or_insert_with(|| Arc::new(Semaphore::new(REFRESH_PARALLELISM))),
            )
        };
        // The semaphore is never closed, so this only fails if it was closed; then the request runs unlimited.
        sem.acquire_owned().await.ok()
    }
}

/// Epoch milliseconds now.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Everything a tracking flow touches.
pub struct TrackCtx {
    pub trackers: Arc<TrackerManager>,
    pub anime_tracks: Arc<dyn AnimeTrackRepo>,
    pub manga_tracks: Arc<dyn MangaTrackRepo>,
    pub anime_episodes: Arc<dyn AnimeEpisodeRepo>,
    pub manga_chapters: Arc<dyn MangaChapterRepo>,
    pub anime_history: Arc<dyn AnimeHistoryRepo>,
    pub manga_history: Arc<dyn MangaHistoryRepo>,
    pub queue: Arc<RetryQueue>,
    pub scheduler: Arc<RetryScheduler>,
    pub net: Arc<dyn Connectivity>,
    /// The shared tracking switches. Progress from reading or watching is sent only while they allow it.
    pub prefs: Arc<TrackPrefs>,
    pub locks: BindingLocks,
    /// Caps the refresh requests sent to one tracker at once.
    pub refresh_limit: RefreshLimit,
    /// Change notifications, sent after each committed write.
    pub events: EventBus,
}

impl TrackCtx {
    /// Whether Incognito is on now. Every write the app makes by itself (progress, auto-track, enhanced binds) asks it
    /// first and sends nothing while it is on, whichever screen asked for the write. A bind the user makes in the
    /// tracking sheet is their own choice and goes through.
    pub fn incognito(&self) -> bool {
        self.prefs.incognito.get()
    }

    /// Starts the retry loop for this context. `None` when it already runs.
    pub fn start_retry(self: &Arc<Self>) -> Option<JoinHandle<()>> {
        Arc::clone(&self.scheduler).spawn(Arc::new(TrackRunner(Arc::clone(self))))
    }

    /// Queues `progress` for the binding and kicks the scheduler (`DelayedAnimeTrackingStore.addAnime` plus
    /// `setupTask`).
    pub fn enqueue(
        &self,
        kind: Kind,
        entry_id: i64,
        tracker: TrackerId,
        progress: f64,
    ) -> Result<(), TrackError> {
        self.queue
            .add(kind, entry_id, tracker, progress, now_ms())?;
        self.scheduler.kick()
    }

    /// The anime binding of `entry_id` for `tracker`, if any.
    pub async fn anime_binding(
        &self,
        entry_id: i64,
        tracker: TrackerId,
    ) -> Result<Option<AnimeTrack>, TrackError> {
        let rows = self.anime_tracks.list_for_entry(entry_id).await?;
        Ok(rows.into_iter().find(|t| t.tracker_id == tracker))
    }

    /// The manga binding of `manga_id` for `tracker`, if any.
    pub async fn manga_binding(
        &self,
        manga_id: i64,
        tracker: TrackerId,
    ) -> Result<Option<MangaTrack>, TrackError> {
        let rows = self.manga_tracks.list_for_entry(manga_id).await?;
        Ok(rows.into_iter().find(|t| t.tracker_id == tracker))
    }

    /// Writes an anime binding. One retry on failure, because the remote update has already happened and the
    /// local row must not be lost (R4, TT-27-120).
    pub async fn save_anime(&self, t: &AnimeTrack) -> Result<i64, TrackError> {
        let id = match self.anime_tracks.upsert(t).await {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(error = %redact(&e.to_string()), "binding write failed, retrying once");
                self.anime_tracks.upsert(t).await?
            }
        };
        self.events.send(TrackEvent::TrackChanged {
            entry_id: t.anime_id,
        });
        Ok(id)
    }

    /// Manga form of [`TrackCtx::save_anime`].
    pub async fn save_manga(&self, t: &MangaTrack) -> Result<i64, TrackError> {
        let id = match self.manga_tracks.upsert(t).await {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(error = %redact(&e.to_string()), "binding write failed, retrying once");
                self.manga_tracks.upsert(t).await?
            }
        };
        self.events.send(TrackEvent::TrackChanged {
            entry_id: t.manga_id,
        });
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tr_27a_080_two_updates_for_one_binding_share_one_lock() {
        let locks = BindingLocks::default();
        let first = locks.for_binding(Kind::Anime, 1, TrackerId::ANILIST);
        let second = locks.for_binding(Kind::Anime, 1, TrackerId::ANILIST);
        let other_entry = locks.for_binding(Kind::Anime, 2, TrackerId::ANILIST);
        let manga = locks.for_binding(Kind::Manga, 1, TrackerId::ANILIST);
        assert!(Arc::ptr_eq(&first, &second), "R1: one lock per binding");
        assert!(!Arc::ptr_eq(&first, &other_entry));
        assert!(
            !Arc::ptr_eq(&first, &manga),
            "anime and manga keys stay apart"
        );
    }

    #[tokio::test]
    async fn tr_27a_080_the_second_update_waits_for_the_first() {
        let locks = BindingLocks::default();
        let first = locks.for_binding(Kind::Anime, 1, TrackerId::ANILIST);
        let second = locks.for_binding(Kind::Anime, 1, TrackerId::ANILIST);
        let held = first.lock().await;
        assert!(
            second.try_lock().is_err(),
            "R1: the second run waits while the first runs"
        );
        drop(held);
        assert!(second.try_lock().is_ok());
    }

    #[tokio::test]
    async fn tr_27b_040_a_saved_binding_sends_track_changed_once_it_is_stored() {
        let h = crate::fakes::Harness::new();
        let mut rx = h.ctx.events.subscribe();
        h.ctx
            .save_anime(&crate::fakes::anime_binding(7, TrackerId::KITSU, 0.0))
            .await
            .unwrap();
        assert_eq!(
            rx.try_recv().unwrap(),
            TrackEvent::TrackChanged { entry_id: 7 }
        );
        assert_eq!(
            h.anime_repo.list_for_entry(7).await.unwrap().len(),
            1,
            "the row is stored when the event is read"
        );
    }
}
