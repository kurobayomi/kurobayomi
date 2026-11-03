//! Writes of the per-entry dialogs (behaviour 14). Each confirm becomes one detached
//! job: it takes the binding's lock, reads the stored binding, applies one edit through the remote setter, and
//! stores the result. Dropping the handle does not cancel it, because the user already pressed OK.

use kuro_platform::kind::Kind;
use std::sync::Arc;

use tokio::task::JoinHandle;

use crate::ctx::{TrackCtx, now_ms};
use crate::detach::detached_locked;
use crate::error::TrackError;
use crate::events::TrackEvent;
use crate::model::{AnimeTrack, MangaTrack, TrackerId};
use crate::remote::{
    set_anime_finish_date, set_anime_private, set_anime_score, set_anime_start_date,
    set_anime_status, set_last_chapter_read, set_last_episode_seen, set_manga_finish_date,
    set_manga_private, set_manga_score, set_manga_start_date, set_manga_status,
    update_remote_anime, update_remote_manga,
};

/// One edit of an anime binding, from a dialog.
#[derive(Debug, Clone, PartialEq)]
pub enum AnimeEdit {
    Status(i64),
    /// The episode number, already truncated by the dialog.
    Progress(i32),
    /// The score label chosen in the list.
    Score(String),
    StartDate(i64),
    FinishDate(i64),
    Private(bool),
}

/// One edit of a manga binding, from a dialog.
#[derive(Debug, Clone, PartialEq)]
pub enum MangaEdit {
    Status(i64),
    /// The chapter number, already truncated by the dialog.
    Progress(i32),
    Score(String),
    StartDate(i64),
    FinishDate(i64),
    Private(bool),
}

/// Starts the anime edit detached and returns its handle. The caller drops the handle.
pub fn spawn_anime_edit(
    ctx: Arc<TrackCtx>,
    entry_id: i64,
    tracker: TrackerId,
    edit: AnimeEdit,
) -> JoinHandle<Result<AnimeTrack, TrackError>> {
    let lock = ctx.locks.for_binding(Kind::Anime, entry_id, tracker);
    detached_locked(vec![lock], async move {
        let result = run_anime_edit(&ctx, entry_id, tracker, edit).await;
        if result.is_ok() {
            ctx.events.send(TrackEvent::TrackChanged { entry_id });
        }
        result
    })
}

/// Starts the manga edit detached and returns its handle. The caller drops the handle.
pub fn spawn_manga_edit(
    ctx: Arc<TrackCtx>,
    entry_id: i64,
    tracker: TrackerId,
    edit: MangaEdit,
) -> JoinHandle<Result<MangaTrack, TrackError>> {
    let lock = ctx.locks.for_binding(Kind::Manga, entry_id, tracker);
    detached_locked(vec![lock], async move {
        let result = run_manga_edit(&ctx, entry_id, tracker, edit).await;
        if result.is_ok() {
            ctx.events.send(TrackEvent::TrackChanged { entry_id });
        }
        result
    })
}

/// Runs under the binding lock, which [`spawn_anime_edit`] holds.
async fn run_anime_edit(
    ctx: &TrackCtx,
    entry_id: i64,
    tracker: TrackerId,
    edit: AnimeEdit,
) -> Result<AnimeTrack, TrackError> {
    let mut t = ctx
        .anime_binding(entry_id, tracker)
        .await?
        .ok_or(TrackError::MissingRow(tracker.0))?;
    let tr = ctx
        .trackers
        .get(tracker)
        .ok_or(TrackError::UnknownTracker(tracker.0))?;
    let api = tr.as_anime().ok_or(TrackError::UnknownTracker(tracker.0))?;
    let repo = ctx.anime_tracks.as_ref();
    match edit {
        AnimeEdit::Private(private) => set_anime_private(&t, api, repo, private).await,
        AnimeEdit::Status(status) => {
            set_anime_status(&mut t, api, status);
            update_remote_anime(&t, api, repo).await
        }
        AnimeEdit::Progress(episode) => {
            set_last_episode_seen(&mut t, api, episode, now_ms());
            update_remote_anime(&t, api, repo).await
        }
        AnimeEdit::Score(label) => {
            set_anime_score(&mut t, api, &label)?;
            update_remote_anime(&t, api, repo).await
        }
        AnimeEdit::StartDate(ms) => {
            set_anime_start_date(&mut t, ms);
            update_remote_anime(&t, api, repo).await
        }
        AnimeEdit::FinishDate(ms) => {
            set_anime_finish_date(&mut t, ms);
            update_remote_anime(&t, api, repo).await
        }
    }
}

/// Runs under the binding lock, which [`spawn_manga_edit`] holds.
async fn run_manga_edit(
    ctx: &TrackCtx,
    entry_id: i64,
    tracker: TrackerId,
    edit: MangaEdit,
) -> Result<MangaTrack, TrackError> {
    let mut t = ctx
        .manga_binding(entry_id, tracker)
        .await?
        .ok_or(TrackError::MissingRow(tracker.0))?;
    let tr = ctx
        .trackers
        .get(tracker)
        .ok_or(TrackError::UnknownTracker(tracker.0))?;
    let api = tr.as_manga().ok_or(TrackError::UnknownTracker(tracker.0))?;
    let repo = ctx.manga_tracks.as_ref();
    match edit {
        MangaEdit::Private(private) => set_manga_private(&t, api, repo, private).await,
        MangaEdit::Status(status) => {
            set_manga_status(&mut t, api, status);
            update_remote_manga(&t, api, repo).await
        }
        MangaEdit::Progress(chapter) => {
            set_last_chapter_read(&mut t, api, chapter, now_ms());
            update_remote_manga(&t, api, repo).await
        }
        MangaEdit::Score(label) => {
            set_manga_score(&mut t, api, &label)?;
            update_remote_manga(&t, api, repo).await
        }
        MangaEdit::StartDate(ms) => {
            set_manga_start_date(&mut t, ms);
            update_remote_manga(&t, api, repo).await
        }
        MangaEdit::FinishDate(ms) => {
            set_manga_finish_date(&mut t, ms);
            update_remote_manga(&t, api, repo).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{Harness, anime_binding};

    #[tokio::test]
    async fn tr_27b_007_a_status_edit_is_stored_after_the_remote_write() {
        let h = Harness::new();
        h.login(TrackerId::ANILIST);
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(1, TrackerId::ANILIST, 2.0))
            .await
            .unwrap();
        let done = spawn_anime_edit(h.ctx.clone(), 1, TrackerId::ANILIST, AnimeEdit::Status(1))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(done.status, 1);
        let stored = h
            .ctx
            .anime_binding(1, TrackerId::ANILIST)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, 1);
    }

    #[tokio::test]
    async fn tr_27b_008_a_failed_remote_write_leaves_the_stored_binding_alone() {
        let h = Harness::new();
        h.login(TrackerId::ANILIST);
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(1, TrackerId::ANILIST, 2.0))
            .await
            .unwrap();
        *h.anime_tracker(TrackerId::ANILIST).fail.lock().unwrap() = Some("remote is down".into());
        let err = spawn_anime_edit(h.ctx.clone(), 1, TrackerId::ANILIST, AnimeEdit::Progress(5))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(err.to_string(), "remote is down");
        let stored = h
            .ctx
            .anime_binding(1, TrackerId::ANILIST)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.last_episode_seen, 2.0,
            "remote first: nothing stored locally"
        );
    }

    #[tokio::test]
    async fn tr_27b_009_an_unknown_score_label_is_an_error_and_writes_nothing() {
        let h = Harness::new();
        h.login(TrackerId::ANILIST);
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(1, TrackerId::ANILIST, 2.0))
            .await
            .unwrap();
        let err = spawn_anime_edit(
            h.ctx.clone(),
            1,
            TrackerId::ANILIST,
            AnimeEdit::Score("zz".into()),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(matches!(err, TrackError::UnknownScore(ref s) if s == "zz"));
        let stored = h
            .ctx
            .anime_binding(1, TrackerId::ANILIST)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.score, 0.0);
    }

    #[tokio::test]
    async fn tr_27b_007_a_missing_row_is_a_typed_error_not_a_panic() {
        let h = Harness::new();
        h.login(TrackerId::ANILIST);
        let err = spawn_anime_edit(h.ctx.clone(), 9, TrackerId::ANILIST, AnimeEdit::Status(1))
            .await
            .unwrap()
            .unwrap_err();
        assert!(matches!(err, TrackError::MissingRow(2)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn tr_27b_041_two_edits_of_one_binding_land_in_submission_order() {
        for _ in 0..30 {
            let h = Harness::new();
            h.login(TrackerId::ANILIST);
            h.ctx
                .anime_tracks
                .upsert(&anime_binding(1, TrackerId::ANILIST, 0.0))
                .await
                .unwrap();
            let a = spawn_anime_edit(h.ctx.clone(), 1, TrackerId::ANILIST, AnimeEdit::Progress(5));
            let b = spawn_anime_edit(h.ctx.clone(), 1, TrackerId::ANILIST, AnimeEdit::Progress(9));
            let (a, b) = (a.await.unwrap().unwrap(), b.await.unwrap().unwrap());
            assert_eq!((a.last_episode_seen, b.last_episode_seen), (5.0, 9.0));
            let stored = h
                .ctx
                .anime_binding(1, TrackerId::ANILIST)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                stored.last_episode_seen, 9.0,
                "B, submitted last, lands last"
            );
        }
    }

    #[tokio::test]
    async fn tr_27b_007_dropping_the_handle_does_not_cancel_the_edit() {
        let h = Harness::new();
        h.login(TrackerId::ANILIST);
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(1, TrackerId::ANILIST, 2.0))
            .await
            .unwrap();
        drop(spawn_anime_edit(
            h.ctx.clone(),
            1,
            TrackerId::ANILIST,
            AnimeEdit::Status(3),
        ));
        for _ in 0..500 {
            if h.ctx
                .anime_binding(1, TrackerId::ANILIST)
                .await
                .unwrap()
                .unwrap()
                .status
                == 3
            {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the dropped edit still ran");
    }
}
