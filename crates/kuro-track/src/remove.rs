//! Remove-tracking flow (`AnimeTrackInfoDialog.kt`). The remote delete runs only when the
//! checkbox is ticked and the tracker can delete. By default it runs first: the local row is unregistered after the
//! remote delete worked or the entry was already absent, and a remote failure keeps the local row and is shown as a
//! toast (E-19, E-47). A tracker with `removes_local_first` (Kitsu, T-30a-022) deletes the local row first; a failed
//! remote delete is then shown as a toast and the local row stays gone.
//!
//! Decision (T-29a-031): the shared order stays, because other trackers rely on it (tr_27_d16). It meets the MAL
//! acceptance anyway: `MalTracker`'s delete reports success after logging its own failure (ERROR, or WARN for a 404
//! that means the title was already gone, E-DEL-1), so no toast is raised and the local row is always removed.

use kuro_platform::kind::Kind;
use std::sync::Arc;

use tokio::task::JoinHandle;

use crate::ctx::TrackCtx;
use crate::detach::detached_locked;
use crate::error::{TrackError, redact};
use crate::events::TrackEvent;
use crate::messages::MsgKey;
use crate::model::{AnimeTrack, MangaTrack};
use crate::repo::{AnimeTrackRepo, MangaTrackRepo};
use crate::tracker::Tracker;

/// The confirmation dialog. The checkbox is shown only for trackers that can delete remotely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveDialog {
    pub tracker: String,
    pub deletable: bool,
}

impl RemoveDialog {
    /// Built from the tracker. Deletable means the tracker exposes a remote delete for this kind.
    pub fn for_anime(tracker: &dyn Tracker) -> Self {
        Self {
            tracker: tracker.name().to_owned(),
            deletable: tracker.as_deletable_anime().is_some(),
        }
    }

    /// Manga form of [`RemoveDialog::for_anime`].
    pub fn for_manga(tracker: &dyn Tracker) -> Self {
        Self {
            tracker: tracker.name().to_owned(),
            deletable: tracker.as_deletable_manga().is_some(),
        }
    }

    pub fn title(&self) -> String {
        crate::messages::english(&MsgKey::TrackDeleteTitle(self.tracker.clone()))
    }

    pub fn text(&self) -> String {
        crate::messages::english(&MsgKey::TrackDeleteText)
    }

    pub fn checkbox_shown(&self) -> bool {
        self.deletable
    }

    pub fn checkbox_label(&self) -> String {
        crate::messages::english(&MsgKey::TrackDeleteRemoteText(self.tracker.clone()))
    }

    /// The checkbox starts off, so OK removes locally only.
    pub fn checkbox_default(&self) -> bool {
        false
    }

    /// The buttons, in the order of the Kotlin dialog.
    pub fn buttons(&self) -> [String; 2] {
        [
            crate::messages::english(&MsgKey::ActionCancel),
            crate::messages::english(&MsgKey::ActionOk),
        ]
    }
}

/// Removes the anime binding of `track`. With `also_remote` set and a deletable tracker, the remote delete runs
/// first (after the local delete for a `removes_local_first` tracker). Its failure is returned as the tracker's toast;
/// the local row stays only in the remote-first order. An entry already absent on the
/// remote counts as success. The local row is deleted next; its failure returns [`MsgKey::LocalRemovalFailed`].
pub async fn remove_anime(
    track: &AnimeTrack,
    tracker: &dyn Tracker,
    also_remote: bool,
    repo: &dyn AnimeTrackRepo,
) -> Result<(), MsgKey> {
    remove_anime_inner(track, tracker, also_remote, repo)
        .await
        .1
}

/// The remove with the flag the callers need: whether the local row is gone, even when the result is an error.
async fn remove_anime_inner(
    track: &AnimeTrack,
    tracker: &dyn Tracker,
    also_remote: bool,
    repo: &dyn AnimeTrackRepo,
) -> (bool, Result<(), MsgKey>) {
    let deletable = if also_remote {
        tracker.as_deletable_anime()
    } else {
        None
    };
    if tracker.removes_local_first() {
        if let Err(e) = repo.delete(track.anime_id, track.tracker_id).await {
            return (false, Err(local_failure(&e)));
        }
        let remote = match deletable {
            Some(d) => d
                .delete(track)
                .await
                .map_err(|e| remote_failure(tracker, &e)),
            None => Ok(()),
        };
        return (true, remote);
    }
    if let Some(d) = deletable {
        if let Err(e) = d.delete(track).await {
            return (false, Err(remote_failure(tracker, &e)));
        }
    }
    match repo.delete(track.anime_id, track.tracker_id).await {
        Ok(()) => (true, Ok(())),
        Err(e) => (false, Err(local_failure(&e))),
    }
}

/// Manga form of [`remove_anime`].
pub async fn remove_manga(
    track: &MangaTrack,
    tracker: &dyn Tracker,
    also_remote: bool,
    repo: &dyn MangaTrackRepo,
) -> Result<(), MsgKey> {
    remove_manga_inner(track, tracker, also_remote, repo)
        .await
        .1
}

/// The remove with the flag the callers need: whether the local row is gone, even when the result is an error.
async fn remove_manga_inner(
    track: &MangaTrack,
    tracker: &dyn Tracker,
    also_remote: bool,
    repo: &dyn MangaTrackRepo,
) -> (bool, Result<(), MsgKey>) {
    let deletable = if also_remote {
        tracker.as_deletable_manga()
    } else {
        None
    };
    if tracker.removes_local_first() {
        if let Err(e) = repo.delete(track.manga_id, track.tracker_id).await {
            return (false, Err(local_failure(&e)));
        }
        let remote = match deletable {
            Some(d) => d
                .delete(track)
                .await
                .map_err(|e| remote_failure(tracker, &e)),
            None => Ok(()),
        };
        return (true, remote);
    }
    if let Some(d) = deletable {
        if let Err(e) = d.delete(track).await {
            return (false, Err(remote_failure(tracker, &e)));
        }
    }
    match repo.delete(track.manga_id, track.tracker_id).await {
        Ok(()) => (true, Ok(())),
        Err(e) => (false, Err(local_failure(&e))),
    }
}

fn local_failure(e: &TrackError) -> MsgKey {
    tracing::error!(error = %redact(&e.to_string()), "local tracking removal failed");
    MsgKey::LocalRemovalFailed
}

fn remote_failure(tracker: &dyn Tracker, e: &TrackError) -> MsgKey {
    tracing::error!(error = %redact(&e.to_string()), "remote delete failed");
    e.toast(tracker.name())
}

/// Removes the anime binding under the binding's lock (T-27b-041). A queued write for the same row finishes first, so
/// the remove runs after it (E-42). [`TrackEvent::TrackChanged`] is sent once the local row is gone.
pub async fn remove_anime_serialized(
    ctx: &TrackCtx,
    track: &AnimeTrack,
    also_remote: bool,
) -> Result<(), MsgKey> {
    let lock = ctx
        .locks
        .for_binding(Kind::Anime, track.anime_id, track.tracker_id);
    let _guard = lock.lock().await;
    remove_anime_locked(ctx, track, also_remote).await
}

/// Starts [`remove_anime_serialized`] detached (T-27b-020). Closing the dialog drops the handle, and the delete still
/// runs. The binding lock is taken here, so the remove follows every write submitted before it.
pub fn spawn_remove_anime(
    ctx: Arc<TrackCtx>,
    track: AnimeTrack,
    also_remote: bool,
) -> JoinHandle<Result<(), MsgKey>> {
    let lock = ctx
        .locks
        .for_binding(Kind::Anime, track.anime_id, track.tracker_id);
    detached_locked(vec![lock], async move {
        remove_anime_locked(&ctx, &track, also_remote).await
    })
}

/// The body of the remove, run while the binding lock is held.
async fn remove_anime_locked(
    ctx: &TrackCtx,
    track: &AnimeTrack,
    also_remote: bool,
) -> Result<(), MsgKey> {
    let tracker = ctx.trackers.get(track.tracker_id).ok_or_else(|| {
        TrackError::UnknownTracker(track.tracker_id.0).toast(&track.tracker_id.0.to_string())
    })?;
    let (removed, result) = remove_anime_inner(
        track,
        tracker.as_ref(),
        also_remote,
        ctx.anime_tracks.as_ref(),
    )
    .await;
    if removed {
        ctx.events.send(TrackEvent::TrackChanged {
            entry_id: track.anime_id,
        });
    }
    result
}

/// Manga form of [`remove_anime_serialized`].
pub async fn remove_manga_serialized(
    ctx: &TrackCtx,
    track: &MangaTrack,
    also_remote: bool,
) -> Result<(), MsgKey> {
    let lock = ctx
        .locks
        .for_binding(Kind::Manga, track.manga_id, track.tracker_id);
    let _guard = lock.lock().await;
    remove_manga_locked(ctx, track, also_remote).await
}

/// Starts [`remove_manga_serialized`] detached (T-27b-020). Closing the dialog drops the handle, and the delete still
/// runs. The binding lock is taken here, so the remove follows every write submitted before it.
pub fn spawn_remove_manga(
    ctx: Arc<TrackCtx>,
    track: MangaTrack,
    also_remote: bool,
) -> JoinHandle<Result<(), MsgKey>> {
    let lock = ctx
        .locks
        .for_binding(Kind::Manga, track.manga_id, track.tracker_id);
    detached_locked(vec![lock], async move {
        remove_manga_locked(&ctx, &track, also_remote).await
    })
}

/// The body of the remove, run while the binding lock is held.
async fn remove_manga_locked(
    ctx: &TrackCtx,
    track: &MangaTrack,
    also_remote: bool,
) -> Result<(), MsgKey> {
    let tracker = ctx.trackers.get(track.tracker_id).ok_or_else(|| {
        TrackError::UnknownTracker(track.tracker_id.0).toast(&track.tracker_id.0.to_string())
    })?;
    let (removed, result) = remove_manga_inner(
        track,
        tracker.as_ref(),
        also_remote,
        ctx.manga_tracks.as_ref(),
    )
    .await;
    if removed {
        ctx.events.send(TrackEvent::TrackChanged {
            entry_id: track.manga_id,
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::fakes::{FakeAnime, MemAnimeRepo, MemSecrets, new_log};
    use crate::model::TrackerId;
    use crate::repo::AnimeTrackRepo;

    fn binding(anime_id: i64, tracker: TrackerId) -> AnimeTrack {
        AnimeTrack {
            id: None,
            anime_id,
            tracker_id: tracker,
            remote_id: 1,
            library_id: None,
            title: "Show".into(),
            last_episode_seen: 0.0,
            total_episodes: 0,
            status: 0,
            score: 0.0,
            remote_url: String::new(),
            start_date: 0,
            finish_date: 0,
            private: false,
        }
    }

    fn deletable(log: crate::fakes::Log) -> FakeAnime {
        let mut t = FakeAnime::new(TrackerId::JELLYFIN, "Jellyfin", MemSecrets::new(), log);
        t.deletable = true;
        t
    }

    #[test]
    fn tr_27_059_dialog_title_text_and_checkbox_follow_the_tracker() {
        let log = new_log();
        let yes = RemoveDialog::for_anime(&deletable(log.clone()));
        assert_eq!(yes.title(), "Remove Jellyfin tracking?");
        assert_eq!(yes.text(), "This will remove the tracking locally.");
        assert!(yes.checkbox_shown());
        assert_eq!(yes.checkbox_label(), "Also remove from Jellyfin");
        let no = RemoveDialog::for_anime(&FakeAnime::new(
            TrackerId::ANILIST,
            "AniList",
            MemSecrets::new(),
            log,
        ));
        assert!(
            !no.checkbox_shown(),
            "only deletable trackers show the checkbox"
        );
    }

    #[tokio::test]
    async fn tr_27_059_checked_remove_deletes_remotely_then_locally() {
        let log = new_log();
        let tracker = deletable(log.clone());
        let repo = MemAnimeRepo {
            rows: Default::default(),
            log: Some(log.clone()),
            watchers: Default::default(),
        };
        repo.upsert(&binding(9, TrackerId::JELLYFIN)).await.unwrap();
        remove_anime(&binding(9, TrackerId::JELLYFIN), &tracker, true, &repo)
            .await
            .unwrap();
        let calls = log.lock().unwrap().clone();
        assert_eq!(calls, ["upsert", "delete"], "the remote delete ran once");
        assert!(repo.list_for_entry(9).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn tr_27_d16_a_failed_remote_delete_is_a_toast_and_the_local_row_stays() {
        let log = new_log();
        let tracker = deletable(log.clone());
        *tracker.fail.lock().unwrap() = Some("delete refused".into());
        let repo = MemAnimeRepo::default();
        repo.upsert(&binding(9, TrackerId::JELLYFIN)).await.unwrap();
        let err = remove_anime(&binding(9, TrackerId::JELLYFIN), &tracker, true, &repo)
            .await
            .unwrap_err();
        assert_eq!(
            err,
            MsgKey::TrackerError {
                tracker: "Jellyfin".into(),
                detail: "delete refused".into()
            }
        );
        assert_eq!(
            crate::messages::english(&err),
            "Jellyfin error: delete refused"
        );
        assert_eq!(
            repo.list_for_entry(9).await.unwrap().len(),
            1,
            "E-47: remote first, so the local row is kept"
        );
    }

    #[tokio::test]
    async fn t_30a_022_a_local_first_tracker_loses_the_local_row_even_when_the_remote_delete_fails()
    {
        let log = new_log();
        let mut tracker = deletable(log.clone());
        tracker.local_first = true;
        *tracker.fail.lock().unwrap() = Some("delete refused".into());
        let repo = MemAnimeRepo::default();
        repo.upsert(&binding(9, TrackerId::JELLYFIN)).await.unwrap();
        let err = remove_anime(&binding(9, TrackerId::JELLYFIN), &tracker, true, &repo)
            .await
            .unwrap_err();
        assert_eq!(
            crate::messages::english(&err),
            "Jellyfin error: delete refused"
        );
        assert!(repo.list_for_entry(9).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn tr_27_t19_a_failed_local_delete_after_the_remote_delete_is_the_local_failure() {
        let log = new_log();
        let tracker = deletable(log.clone());
        let repo = crate::fakes::FlakyAnimeRepo {
            inner: Arc::new(MemAnimeRepo::default()),
            fail_next: std::sync::atomic::AtomicBool::new(false),
            fail_delete: std::sync::atomic::AtomicBool::new(true),
        };
        repo.inner
            .upsert(&binding(9, TrackerId::JELLYFIN))
            .await
            .unwrap();
        let err = remove_anime(&binding(9, TrackerId::JELLYFIN), &tracker, true, &repo)
            .await
            .unwrap_err();
        assert_eq!(err, MsgKey::LocalRemovalFailed);
        assert_eq!(
            log.lock()
                .unwrap()
                .iter()
                .filter(|e| *e == "delete")
                .count(),
            1,
            "the remote delete ran first"
        );
        assert_eq!(
            repo.inner.list_for_entry(9).await.unwrap().len(),
            1,
            "local row kept"
        );
    }

    #[tokio::test]
    async fn tr_27_059_without_the_checkbox_only_the_local_row_is_deleted() {
        let log = new_log();
        let tracker = deletable(log.clone());
        let repo = MemAnimeRepo::default();
        repo.upsert(&binding(9, TrackerId::JELLYFIN)).await.unwrap();
        repo.upsert(&binding(9, TrackerId::ANILIST)).await.unwrap();
        remove_anime(&binding(9, TrackerId::JELLYFIN), &tracker, false, &repo)
            .await
            .unwrap();
        assert!(
            !log.lock().unwrap().iter().any(|e| e == "delete"),
            "no remote call"
        );
        let left = repo.list_for_entry(9).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(
            left[0].tracker_id,
            TrackerId::ANILIST,
            "other trackers stay"
        );
    }

    #[tokio::test]
    async fn tr_27b_020_dropping_the_caller_does_not_cancel_a_spawned_remove() {
        let h = crate::fakes::Harness::new();
        let track = crate::fakes::anime_binding(1, TrackerId::KITSU, 0.0);
        h.anime_repo.upsert(&track).await.unwrap();
        let ctx = Arc::clone(&h.ctx);
        let caller = tokio::spawn(async move {
            drop(spawn_remove_anime(ctx, track, false));
            std::future::pending::<()>().await;
        });
        tokio::task::yield_now().await;
        caller.abort();
        let _ = caller.await;
        for _ in 0..500 {
            if h.anime_repo.list_for_entry(1).await.unwrap().is_empty() {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the remove still ran after its caller was dropped");
    }

    #[tokio::test]
    async fn tr_27b_041_a_remove_waits_for_the_queued_write_on_the_same_row() {
        let h = crate::fakes::Harness::new();
        let track = crate::fakes::anime_binding(1, TrackerId::KITSU, 0.0);
        h.anime_repo.upsert(&track).await.unwrap();
        let lock = h.ctx.locks.for_binding(Kind::Anime, 1, TrackerId::KITSU);
        let held = lock.lock().await;
        let ctx = Arc::clone(&h.ctx);
        let job = tokio::spawn(async move { remove_anime_serialized(&ctx, &track, false).await });
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            h.anime_repo.list_for_entry(1).await.unwrap().len(),
            1,
            "the remove waits for the write that holds the row"
        );
        drop(held);
        job.await.unwrap().unwrap();
        assert!(h.anime_repo.list_for_entry(1).await.unwrap().is_empty());
    }
}

#[cfg(test)]
mod mal_tests {
    use std::sync::Arc;

    use super::*;
    use crate::fakes::MemSecrets;
    use crate::mal::store::{SecretTokenStore, TokenStore};
    use crate::mal::tracker::MalTracker;

    fn mal() -> (tempfile::TempDir, MalTracker) {
        let dir = tempfile::tempdir().expect("tempdir");
        let tokens: Arc<dyn TokenStore> = Arc::new(SecretTokenStore::new(MemSecrets::new()));
        let tracker = MalTracker::new(dir.path(), tokens, Some("cid".into())).expect("tracker");
        (dir, tracker)
    }

    #[test]
    fn t_29a_031_the_mal_delete_dialog_has_the_source_texts_and_a_remote_option() {
        let (_dir, tracker) = mal();
        let dialog = RemoveDialog::for_manga(&tracker);
        assert_eq!(dialog.title(), "Remove MyAnimeList tracking?");
        assert_eq!(dialog.text(), "This will remove the tracking locally.");
        assert!(dialog.checkbox_shown(), "MAL can delete remotely");
        assert_eq!(dialog.checkbox_label(), "Also remove from MyAnimeList");
        assert!(RemoveDialog::for_anime(&tracker).checkbox_shown());
    }

    #[test]
    fn t_29a_031_buttons_are_cancel_then_ok_and_the_checkbox_starts_off() {
        let (_dir, tracker) = mal();
        let dialog = RemoveDialog::for_manga(&tracker);
        assert_eq!(dialog.buttons(), ["Cancel", "OK"]);
        assert!(!dialog.checkbox_default());
    }
}
