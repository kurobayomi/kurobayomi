//! Progress updates (`TrackEpisode.kt`, `TrackChapter.kt`): push "episode N seen" or "chapter N read" to every
//! logged-in tracker that is behind it. The trackers run concurrently and one failure never blocks another (EC-44).
//! Offline and failed updates go to the retry queue. Every error is logged at info, and each tracker's outcome is
//! returned as a value, so no error is rethrown to the caller.

use kuro_platform::kind::Kind;
use std::sync::Arc;

use async_trait::async_trait;
use futures::future::join_all;
use tokio::task::JoinHandle;

use crate::ctx::TrackCtx;
use crate::detach::detached_locked;
use crate::error::TrackError;
use crate::error::redact;
use crate::events::TrackEvent;
use crate::model::{AnimeTrack, MangaTrack, TrackerId};
use crate::queue::QueueItem;
use crate::queue::retry::{ItemOutcome, RetryRunner};
use crate::remote::{with_server_anime, with_server_manga};
use crate::tracker::{AnimeTracker, MangaTracker, Tracker};

/// What a progress update did for one tracker.
#[derive(Debug, Clone, PartialEq)]
pub enum TrackOutcome {
    /// Remote and local are updated, and any queued item for the binding is gone.
    Updated,
    /// Offline, or the update failed. The item is in the retry queue.
    Queued,
    /// The binding is already at or past the value, so no request was made.
    Skipped,
    /// The update failed and could not be queued either. Carries the message.
    Failed(String),
}

/// Pushes episode `episode` to every logged-in anime tracker with a binding for `entry_id`.
pub async fn track_episode(
    ctx: &TrackCtx,
    entry_id: i64,
    episode: f64,
) -> Vec<(TrackerId, TrackOutcome)> {
    track_episode_with(ctx, entry_id, episode, false).await
}

/// Starts [`track_episode`] detached (T-27b-020). The locks of every logged-in tracker's binding are taken here, in
/// registry order, so the update follows each write submitted before it and the player closing does not cancel it.
pub fn spawn_track_episode(
    ctx: Arc<TrackCtx>,
    entry_id: i64,
    episode: f64,
) -> JoinHandle<Vec<(TrackerId, TrackOutcome)>> {
    let locks = ctx
        .trackers
        .logged_in()
        .iter()
        .filter(|t| ctx.trackers.is_enabled(t.id()))
        .filter(|t| t.as_anime().is_some())
        .map(|t| ctx.locks.for_binding(Kind::Anime, entry_id, t.id()))
        .collect();
    detached_locked(locks, async move {
        track_episode_with(&ctx, entry_id, episode, true).await
    })
}

/// `already_locked` is set when the caller holds every binding lock, so none is taken here. The eligible trackers run
/// concurrently, so one slow or failing tracker never delays another (EC-44). Results come back in registry order.
async fn track_episode_with(
    ctx: &TrackCtx,
    entry_id: i64,
    episode: f64,
    already_locked: bool,
) -> Vec<(TrackerId, TrackOutcome)> {
    // Incognito sends nothing, whoever asked (`TrackCtx::incognito`). The "update progress after reading" switch is the
    // caller's gate (`auto_update_allowed`): the reader and the player check it, a mark as read does not (Aniyomi checks
    // `autoUpdateTrackOnMarkRead` there instead).
    if ctx.incognito() {
        return Vec::new();
    }
    let runs = ctx
        .trackers
        .logged_in()
        .into_iter()
        .filter(|t| ctx.trackers.is_enabled(t.id()))
        .map(|tracker| episode_for(ctx, tracker, entry_id, episode, already_locked));
    join_all(runs).await.into_iter().flatten().collect()
}

/// One tracker's share of [`track_episode_with`]. `None` when the tracker is not an anime tracker or has no binding.
async fn episode_for(
    ctx: &TrackCtx,
    tracker: Arc<dyn Tracker>,
    entry_id: i64,
    episode: f64,
    already_locked: bool,
) -> Option<(TrackerId, TrackOutcome)> {
    let api = tracker.as_anime()?;
    let id = tracker.id();
    let lock = ctx.locks.for_binding(Kind::Anime, entry_id, id);
    let _guard = if already_locked {
        None
    } else {
        Some(lock.lock().await)
    };
    let binding = match ctx.anime_binding(entry_id, id).await {
        Ok(Some(b)) => b,
        Ok(None) => return None,
        Err(e) => return Some((id, failed(entry_id, id, "episode", &e.to_string()))),
    };
    let outcome = if episode <= binding.last_episode_seen {
        TrackOutcome::Skipped
    } else if !ctx.net.is_online() {
        queue_progress(ctx, Kind::Anime, entry_id, id, episode)
    } else {
        match apply_anime(ctx, &binding, api, episode).await {
            Ok(()) => {
                settle(ctx, Kind::Anime, entry_id, id, episode);
                TrackOutcome::Updated
            }
            Err(e) if e.is_retry_futile() => failed(entry_id, id, "episode", &e.to_string()),
            Err(e) => {
                tracing::info!(entry = entry_id, tracker = id.0, error = %redact(&e.to_string()), "episode update queued");
                queue_progress(ctx, Kind::Anime, entry_id, id, episode)
            }
        }
    };
    Some((id, outcome))
}

/// Pushes chapter `chapter` to every logged-in manga tracker with a binding for `manga_id`.
pub async fn track_chapter(
    ctx: &TrackCtx,
    manga_id: i64,
    chapter: f64,
) -> Vec<(TrackerId, TrackOutcome)> {
    track_chapter_with(ctx, manga_id, chapter, false).await
}

/// Starts [`track_chapter`] detached (T-27b-020). The locks of every logged-in tracker's binding are taken here, in
/// registry order, so the update follows each write submitted before it and the player closing does not cancel it.
pub fn spawn_track_chapter(
    ctx: Arc<TrackCtx>,
    manga_id: i64,
    chapter: f64,
) -> JoinHandle<Vec<(TrackerId, TrackOutcome)>> {
    let locks = ctx
        .trackers
        .logged_in()
        .iter()
        .filter(|t| ctx.trackers.is_enabled(t.id()))
        .filter(|t| t.as_manga().is_some())
        .map(|t| ctx.locks.for_binding(Kind::Manga, manga_id, t.id()))
        .collect();
    detached_locked(locks, async move {
        track_chapter_with(&ctx, manga_id, chapter, true).await
    })
}

/// `already_locked` is set when the caller holds every binding lock, so none is taken here. The eligible trackers run
/// concurrently (EC-44), as in [`track_episode_with`].
async fn track_chapter_with(
    ctx: &TrackCtx,
    manga_id: i64,
    chapter: f64,
    already_locked: bool,
) -> Vec<(TrackerId, TrackOutcome)> {
    if ctx.incognito() {
        return Vec::new();
    }
    let runs = ctx
        .trackers
        .logged_in()
        .into_iter()
        .filter(|t| ctx.trackers.is_enabled(t.id()))
        .map(|tracker| chapter_for(ctx, tracker, manga_id, chapter, already_locked));
    join_all(runs).await.into_iter().flatten().collect()
}

/// One tracker's share of [`track_chapter_with`]. `None` when the tracker is not a manga tracker or has no binding.
async fn chapter_for(
    ctx: &TrackCtx,
    tracker: Arc<dyn Tracker>,
    manga_id: i64,
    chapter: f64,
    already_locked: bool,
) -> Option<(TrackerId, TrackOutcome)> {
    let api = tracker.as_manga()?;
    let id = tracker.id();
    let lock = ctx.locks.for_binding(Kind::Manga, manga_id, id);
    let _guard = if already_locked {
        None
    } else {
        Some(lock.lock().await)
    };
    let binding = match ctx.manga_binding(manga_id, id).await {
        Ok(Some(b)) => b,
        Ok(None) => return None,
        Err(e) => return Some((id, failed(manga_id, id, "chapter", &e.to_string()))),
    };
    let outcome = if chapter <= binding.last_chapter_read {
        TrackOutcome::Skipped
    } else if !ctx.net.is_online() {
        queue_progress(ctx, Kind::Manga, manga_id, id, chapter)
    } else {
        match apply_manga(ctx, &binding, api, chapter).await {
            Ok(()) => {
                settle(ctx, Kind::Manga, manga_id, id, chapter);
                TrackOutcome::Updated
            }
            Err(e) if e.is_retry_futile() => failed(manga_id, id, "chapter", &e.to_string()),
            Err(e) => {
                tracing::info!(entry = manga_id, tracker = id.0, error = %redact(&e.to_string()), "chapter update queued");
                queue_progress(ctx, Kind::Manga, manga_id, id, chapter)
            }
        }
    };
    Some((id, outcome))
}

/// A failure that was not queued: logged at info (`TrackChapter.kt` logs and swallows) and kept as an outcome.
fn failed(entry: i64, tracker: TrackerId, what: &str, message: &str) -> TrackOutcome {
    tracing::info!(entry, tracker = tracker.0, error = %redact(message), "{what} update failed");
    TrackOutcome::Failed(message.to_owned())
}

/// Refresh, copy the progress in, update the remote, then store locally (`TrackEpisode.kt:31-44`). Nothing is
/// written locally when the remote update fails.
pub(crate) async fn apply_anime(
    ctx: &TrackCtx,
    binding: &AnimeTrack,
    api: &dyn AnimeTracker,
    episode: f64,
) -> Result<(), TrackError> {
    // Race 4 (T-29a-056): the local row may have been deleted while this update waited. Nothing is sent for it.
    if ctx
        .anime_binding(binding.anime_id, binding.tracker_id)
        .await?
        .is_none()
    {
        return Ok(());
    }
    let fresh = api.refresh(binding).await?;
    let updated = AnimeTrack {
        id: binding.id,
        anime_id: binding.anime_id,
        tracker_id: binding.tracker_id,
        last_episode_seen: episode,
        ..fresh
    };
    let server = api.update(&updated, true).await?;
    ctx.save_anime(&with_server_anime(&updated, &server))
        .await?;
    Ok(())
}

/// Manga form of [`apply_anime`].
pub(crate) async fn apply_manga(
    ctx: &TrackCtx,
    binding: &MangaTrack,
    api: &dyn MangaTracker,
    chapter: f64,
) -> Result<(), TrackError> {
    // Race 4 (T-29a-056): the local row may have been deleted while this update waited. Nothing is sent for it.
    if ctx
        .manga_binding(binding.manga_id, binding.tracker_id)
        .await?
        .is_none()
    {
        return Ok(());
    }
    let fresh = api.refresh(binding).await?;
    let updated = MangaTrack {
        id: binding.id,
        manga_id: binding.manga_id,
        tracker_id: binding.tracker_id,
        last_chapter_read: chapter,
        ..fresh
    };
    let server = api.update(&updated, true).await?;
    ctx.save_manga(&with_server_manga(&updated, &server))
        .await?;
    Ok(())
}

/// Queues the progress for the binding. A failure to queue is returned as an outcome, not raised.
fn queue_progress(
    ctx: &TrackCtx,
    kind: Kind,
    entry_id: i64,
    tracker: TrackerId,
    progress: f64,
) -> TrackOutcome {
    match ctx.enqueue(kind, entry_id, tracker, progress) {
        Ok(()) => TrackOutcome::Queued,
        Err(e) => failed(entry_id, tracker, "queue", &e.to_string()),
    }
}

/// Removes the queued item after a successful update. Conditional on `progress`, so a newer queued value stays.
fn settle(ctx: &TrackCtx, kind: Kind, entry_id: i64, tracker: TrackerId, progress: f64) {
    if let Err(e) = ctx
        .queue
        .remove_processed(kind, entry_id, tracker, progress)
    {
        tracing::warn!(error = %redact(&e.to_string()), "queued item not removed after a successful update");
    }
    ctx.events.send(TrackEvent::TrackChanged { entry_id });
}

/// Runs queued items for the retry scheduler (`TrackEpisode` with `setupJobOnFailure = false`, plus defect D5).
pub struct TrackRunner(pub Arc<TrackCtx>);

#[async_trait]
impl RetryRunner for TrackRunner {
    async fn run_item(&self, item: &QueueItem) -> Result<ItemOutcome, TrackError> {
        match item.kind {
            Kind::Anime => run_anime_item(&self.0, item).await,
            Kind::Manga => run_manga_item(&self.0, item).await,
        }
    }
}

async fn run_anime_item(ctx: &TrackCtx, item: &QueueItem) -> Result<ItemOutcome, TrackError> {
    let lock = ctx
        .locks
        .for_binding(Kind::Anime, item.entry_id, item.tracker_id);
    let _guard = lock.lock().await;
    let Some(binding) = ctx.anime_binding(item.entry_id, item.tracker_id).await? else {
        return Ok(ItemOutcome::Done);
    };
    let Some(tracker) = ctx.trackers.get(item.tracker_id) else {
        return Ok(ItemOutcome::Kept);
    };
    if !tracker.is_logged_in() {
        return Ok(ItemOutcome::Kept);
    }
    let Some(api) = tracker.as_anime() else {
        return Ok(ItemOutcome::Kept);
    };
    if item.progress <= binding.last_episode_seen {
        return Ok(ItemOutcome::Done);
    }
    apply_anime(ctx, &binding, api, item.progress).await?;
    Ok(ItemOutcome::Done)
}

async fn run_manga_item(ctx: &TrackCtx, item: &QueueItem) -> Result<ItemOutcome, TrackError> {
    let lock = ctx
        .locks
        .for_binding(Kind::Manga, item.entry_id, item.tracker_id);
    let _guard = lock.lock().await;
    let Some(binding) = ctx.manga_binding(item.entry_id, item.tracker_id).await? else {
        return Ok(ItemOutcome::Done);
    };
    let Some(tracker) = ctx.trackers.get(item.tracker_id) else {
        return Ok(ItemOutcome::Kept);
    };
    if !tracker.is_logged_in() {
        return Ok(ItemOutcome::Kept);
    }
    let Some(api) = tracker.as_manga() else {
        return Ok(ItemOutcome::Kept);
    };
    if item.progress <= binding.last_chapter_read {
        return Ok(ItemOutcome::Done);
    }
    apply_manga(ctx, &binding, api, item.progress).await?;
    Ok(ItemOutcome::Done)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::fakes::{Harness, anime_binding, manga_binding};
    use crate::queue::retry::RetryRunner;

    fn logs(h: &Harness) -> Vec<String> {
        h.log.lock().unwrap().clone()
    }

    #[tokio::test]
    async fn tr_27_027_online_update_refreshes_updates_stores_and_clears_the_queue() {
        let h = Harness::new();
        h.login(TrackerId::KITSU);
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(1, TrackerId::KITSU, 5.0))
            .await
            .unwrap();
        h.ctx
            .queue
            .add(Kind::Anime, 1, TrackerId::KITSU, 6.0, 0)
            .unwrap();

        let out = track_episode(&h.ctx, 1, 6.0).await;
        assert_eq!(out, vec![(TrackerId::KITSU, TrackOutcome::Updated)]);
        assert!(logs(&h).contains(&"update:true".to_string()));
        let row = h.ctx.anime_tracks.list_for_entry(1).await.unwrap();
        assert_eq!(row[0].last_episode_seen, 6.0);
        assert!(h.ctx.queue.is_empty().unwrap());
    }

    #[tokio::test]
    async fn incognito_sends_no_progress_whoever_asks() {
        let h = Harness::new();
        h.login(TrackerId::KITSU);
        h.ctx.anime_tracks.upsert(&anime_binding(1, TrackerId::KITSU, 5.0)).await.unwrap();
        h.ctx.prefs.incognito.set(true).unwrap();
        assert!(track_episode(&h.ctx, 1, 6.0).await.is_empty(), "no tracker is asked");
        assert!(!logs(&h).iter().any(|l| l.starts_with("update")), "no request left the app");
        assert_eq!(h.ctx.anime_tracks.list_for_entry(1).await.unwrap()[0].last_episode_seen, 5.0);
        assert!(crate::auto_bind::auto_bind_anime(&h.ctx, 2, "A title").await.0.is_empty(), "and nothing is auto-bound");
    }

    #[tokio::test]
    async fn tr_27b_020_dropping_the_caller_does_not_cancel_a_spawned_progress_update() {
        let h = Harness::new();
        h.login(TrackerId::KITSU);
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(1, TrackerId::KITSU, 5.0))
            .await
            .unwrap();
        let ctx = Arc::clone(&h.ctx);
        let caller = tokio::spawn(async move {
            drop(spawn_track_episode(ctx, 1, 6.0));
            std::future::pending::<()>().await;
        });
        tokio::task::yield_now().await;
        caller.abort();
        let _ = caller.await;
        for _ in 0..500 {
            let row = h.ctx.anime_tracks.list_for_entry(1).await.unwrap();
            if row[0].last_episode_seen == 6.0 {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the update still ran after its caller was dropped");
    }

    #[tokio::test]
    async fn tr_27_027_logged_out_and_at_or_past_trackers_skip_without_a_request() {
        let h = Harness::new();
        h.login(TrackerId::KITSU);
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(1, TrackerId::KITSU, 5.0))
            .await
            .unwrap();
        // Jellyfin has a binding but is logged out, so it is never asked.
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(1, TrackerId::JELLYFIN, 0.0))
            .await
            .unwrap();

        let out = track_episode(&h.ctx, 1, 5.0).await;
        assert_eq!(out, vec![(TrackerId::KITSU, TrackOutcome::Skipped)]);
        assert!(!logs(&h).iter().any(|l| l.starts_with("update")));
    }

    #[tokio::test]
    async fn tr_27_111_offline_anime_queues_without_a_network_call() {
        let h = Harness::new();
        h.login(TrackerId::KITSU);
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(3, TrackerId::KITSU, 1.0))
            .await
            .unwrap();
        h.net.set_online(false);

        let out = track_episode(&h.ctx, 3, 7.0).await;
        assert_eq!(out, vec![(TrackerId::KITSU, TrackOutcome::Queued)]);
        assert!(
            !logs(&h).iter().any(|l| l.starts_with("update")),
            "TT-27-111"
        );
        let items = h.ctx.queue.list(Kind::Anime).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            (items[0].entry_id, items[0].tracker_id, items[0].progress),
            (3, TrackerId::KITSU, 7.0)
        );
    }

    #[tokio::test]
    async fn tr_27_027_online_error_is_queued_too() {
        let h = Harness::new();
        h.login(TrackerId::KITSU);
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(4, TrackerId::KITSU, 1.0))
            .await
            .unwrap();
        *h.anime_tracker(TrackerId::KITSU).fail.lock().unwrap() = Some("HTTP 500".into());

        let out = track_episode(&h.ctx, 4, 2.0).await;
        assert_eq!(out, vec![(TrackerId::KITSU, TrackOutcome::Queued)]);
        assert_eq!(h.ctx.queue.list(Kind::Anime).unwrap()[0].progress, 2.0);
        let row = h.ctx.anime_tracks.list_for_entry(4).await.unwrap();
        assert_eq!(
            row[0].last_episode_seen, 1.0,
            "nothing local is written on failure"
        );
    }

    #[tokio::test]
    async fn tr_27_d02_external_path_queues_by_entry_and_tracker_not_binding_id() {
        let h = Harness::new();
        h.login(TrackerId::KITSU);
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(3, TrackerId::KITSU, 0.0))
            .await
            .unwrap();
        h.net.set_online(false);
        track_episode(&h.ctx, 3, 1.0).await;
        let item = h.ctx.queue.list(Kind::Anime).unwrap()[0];
        assert_eq!(item.entry_id, 3, "the entry id, not the binding row id");
        assert_eq!(item.tracker_id, TrackerId::KITSU);
    }

    #[tokio::test]
    async fn ec_44_eligible_anime_trackers_run_together() {
        let h = Harness::new();
        h.login(TrackerId::KITSU);
        h.login(TrackerId::SIMKL);
        for id in [TrackerId::KITSU, TrackerId::SIMKL] {
            h.ctx
                .anime_tracks
                .upsert(&anime_binding(1, id, 1.0))
                .await
                .unwrap();
        }
        // Each update waits for the other one. A sequential loop would never get past the first.
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        for id in [TrackerId::KITSU, TrackerId::SIMKL] {
            *h.anime_tracker(id).gate.lock().unwrap() = Some(barrier.clone());
        }
        let out = tokio::time::timeout(Duration::from_secs(5), track_episode(&h.ctx, 1, 2.0))
            .await
            .expect("the trackers ran concurrently");
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|(_, o)| *o == TrackOutcome::Updated));
    }

    #[tokio::test]
    async fn ec_44_one_failing_manga_tracker_does_not_block_the_other() {
        let h = Harness::new();
        h.login(TrackerId::MANGAUPDATES);
        h.login(TrackerId::KAVITA);
        for id in [TrackerId::MANGAUPDATES, TrackerId::KAVITA] {
            h.ctx
                .manga_tracks
                .upsert(&manga_binding(9, id, 1.0))
                .await
                .unwrap();
        }
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        for id in [TrackerId::MANGAUPDATES, TrackerId::KAVITA] {
            *h.manga_tracker(id).gate.lock().unwrap() = Some(barrier.clone());
        }
        *h.manga_tracker(TrackerId::MANGAUPDATES)
            .fail
            .lock()
            .unwrap() = Some("HTTP 500".into());

        let out = tokio::time::timeout(Duration::from_secs(5), track_chapter(&h.ctx, 9, 2.0))
            .await
            .expect("the trackers ran concurrently");
        let outcome = |id| out.iter().find(|(t, _)| *t == id).map(|(_, o)| o.clone());
        assert_eq!(outcome(TrackerId::MANGAUPDATES), Some(TrackOutcome::Queued));
        assert_eq!(outcome(TrackerId::KAVITA), Some(TrackOutcome::Updated));
        let rows = h.ctx.manga_tracks.list_for_entry(9).await.unwrap();
        assert!(
            rows.iter()
                .any(|r| r.tracker_id == TrackerId::KAVITA && r.last_chapter_read == 2.0)
        );
    }

    #[tokio::test]
    async fn tr_27_112_chapter_success_and_failure_queue_rules() {
        let h = Harness::new();
        h.login(TrackerId::MANGAUPDATES);
        h.ctx
            .manga_tracks
            .upsert(&manga_binding(9, TrackerId::MANGAUPDATES, 2.0))
            .await
            .unwrap();

        let out = track_chapter(&h.ctx, 9, 3.0).await;
        assert_eq!(out, vec![(TrackerId::MANGAUPDATES, TrackOutcome::Updated)]);
        assert!(logs(&h).contains(&"update:true".to_string()));

        *h.manga_tracker(TrackerId::MANGAUPDATES)
            .fail
            .lock()
            .unwrap() = Some("timeout".into());
        let out = track_chapter(&h.ctx, 9, 4.0).await;
        assert_eq!(out, vec![(TrackerId::MANGAUPDATES, TrackOutcome::Queued)]);
        let items = h.ctx.queue.list(Kind::Manga).unwrap();
        assert_eq!((items[0].entry_id, items[0].progress), (9, 4.0));
        assert!(
            h.ctx.queue.list(Kind::Anime).unwrap().is_empty(),
            "D3: kinds stay apart"
        );
    }

    #[tokio::test]
    async fn t_29a_057_the_core_sends_whatever_the_reader_switch_says_because_the_caller_gates_it() {
        let h = Harness::new();
        h.login(TrackerId::KITSU);
        h.ctx.anime_tracks.upsert(&anime_binding(5, TrackerId::KITSU, 1.0)).await.unwrap();
        h.ctx.prefs.auto_update_track.set(false).unwrap();

        // a mark as read with "Always" reaches the tracker although the reader's switch is off
        let out = track_episode(&h.ctx, 5, 2.0).await;
        assert_eq!(out, vec![(TrackerId::KITSU, TrackOutcome::Updated)]);
        assert!(logs(&h).iter().any(|l| l.starts_with("update")));
    }

    #[tokio::test]
    async fn t_29a_056_a_deleted_row_sends_no_remote_update() {
        let h = Harness::new();
        h.login(TrackerId::KITSU);
        let binding = anime_binding(12, TrackerId::KITSU, 3.0);
        h.ctx.anime_tracks.upsert(&binding).await.unwrap();
        h.ctx
            .anime_tracks
            .delete(12, TrackerId::KITSU)
            .await
            .unwrap();
        let api = h.anime_tracker(TrackerId::KITSU);
        apply_anime(&h.ctx, &binding, &*api, 5.0).await.unwrap();
        assert!(
            !logs(&h).iter().any(|l| l.starts_with("update") || l.starts_with("refresh")),
            "no refresh and no update for a deleted row: {:?}",
            logs(&h)
        );
    }

    #[tokio::test]
    async fn tr_27_d05_runner_removes_items_whose_binding_is_gone_or_already_past() {
        let h = Harness::new();
        h.login(TrackerId::KITSU);
        let runner = TrackRunner(h.ctx.clone());

        // No binding for entry 11 any more.
        let gone = QueueItem {
            kind: Kind::Anime,
            entry_id: 11,
            tracker_id: TrackerId::KITSU,
            progress: 3.0,
            queued_at_ms: 0,
            attempts: 0,
        };
        assert_eq!(runner.run_item(&gone).await.unwrap(), ItemOutcome::Done);

        // TT-27-D05: the binding is at 12 and the queued value is 12.
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(12, TrackerId::KITSU, 12.0))
            .await
            .unwrap();
        let stale = QueueItem {
            entry_id: 12,
            progress: 12.0,
            ..gone
        };
        assert_eq!(runner.run_item(&stale).await.unwrap(), ItemOutcome::Done);
        assert!(
            !logs(&h).iter().any(|l| l.starts_with("update")),
            "no request for a stale item"
        );
    }

    #[tokio::test]
    async fn tr_27_026_logged_out_item_is_kept_and_not_an_error() {
        let h = Harness::new();
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(5, TrackerId::KITSU, 0.0))
            .await
            .unwrap();
        let runner = TrackRunner(h.ctx.clone());
        let item = QueueItem {
            kind: Kind::Anime,
            entry_id: 5,
            tracker_id: TrackerId::KITSU,
            progress: 1.0,
            queued_at_ms: 0,
            attempts: 0,
        };
        assert_eq!(runner.run_item(&item).await.unwrap(), ItemOutcome::Kept);
    }

    #[tokio::test(start_paused = true)]
    async fn tr_27_029_started_loop_drains_a_queued_update_when_back_online() {
        let h = Harness::new();
        h.login(TrackerId::KITSU);
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(6, TrackerId::KITSU, 0.0))
            .await
            .unwrap();
        h.net.set_online(false);
        track_episode(&h.ctx, 6, 4.0).await;
        assert_eq!(h.ctx.queue.list(Kind::Anime).unwrap().len(), 1);

        let handle = h.ctx.start_retry().expect("first start");
        assert!(h.ctx.start_retry().is_none(), "TT-27-127: one loop only");
        tokio::time::sleep(Duration::from_secs(30 * 60)).await;
        assert!(
            h.ctx.queue.list(Kind::Anime).unwrap().len() == 1,
            "still offline"
        );

        h.net.set_online(true);
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(h.ctx.queue.is_empty().unwrap());
        assert_eq!(
            h.ctx.anime_tracks.list_for_entry(6).await.unwrap()[0].last_episode_seen,
            4.0
        );
        handle.abort();
    }

    #[tokio::test]
    async fn tr_27_120_a_failed_local_write_is_retried_once_without_a_second_remote_update() {
        use crate::fakes::FlakyAnimeRepo;
        use std::sync::atomic::AtomicBool;
        let mut h = Harness::new();
        h.login(TrackerId::KITSU);
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(30, TrackerId::KITSU, 1.0))
            .await
            .unwrap();
        let inner = Arc::clone(&h.anime_repo);
        Arc::get_mut(&mut h.ctx).expect("sole owner").anime_tracks = Arc::new(FlakyAnimeRepo {
            inner,
            fail_next: AtomicBool::new(true),
            fail_delete: AtomicBool::new(false),
        });

        let out = track_episode(&h.ctx, 30, 2.0).await;
        assert_eq!(out, vec![(TrackerId::KITSU, TrackOutcome::Updated)]);
        let updates = logs(&h).iter().filter(|l| l.starts_with("update")).count();
        assert_eq!(updates, 1, "one remote update");
        assert_eq!(
            h.ctx.anime_tracks.list_for_entry(30).await.unwrap()[0].last_episode_seen,
            2.0
        );
    }

    #[tokio::test]
    async fn tr_27_030_the_manga_path_never_runs_an_anime_item() {
        // The entry has an anime binding for Kitsu and no manga binding. A manga queue item for the same
        // numbers must not reach the anime tracker.
        let h = Harness::new();
        h.login(TrackerId::KITSU);
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(40, TrackerId::KITSU, 0.0))
            .await
            .unwrap();
        let runner = TrackRunner(h.ctx.clone());
        let manga_item = QueueItem {
            kind: Kind::Manga,
            entry_id: 40,
            tracker_id: TrackerId::KITSU,
            progress: 9.0,
            queued_at_ms: 0,
            attempts: 0,
        };
        assert_eq!(
            runner.run_item(&manga_item).await.unwrap(),
            ItemOutcome::Done
        );
        assert!(!logs(&h).iter().any(|l| l.starts_with("update")));
        assert_eq!(
            h.ctx.anime_tracks.list_for_entry(40).await.unwrap()[0].last_episode_seen,
            0.0
        );
    }
}
