//! Refresh (pull remote state) and progress sync (`RefreshAnimeTracks.kt`, `RefreshMangaTracks.kt`,
//! `SyncEpisodeProgressWithTrack.kt`, `SyncChapterProgressWithTrack.kt`). Unlike the source, every tracker pulls (user,
//! 2026-10-25: progress made in another app reaches the tracker, then this app): the units up to the tracker's number
//! are marked seen. Only an enhanced (self-hosted) tracker also gets the local number written back, as upstream.

use kuro_platform::kind::Kind;
use futures::future::join_all;

use crate::ctx::TrackCtx;
use crate::error::TrackError;
use crate::error::redact;
use crate::model::{AnimeTrack, MangaTrack, TrackerId};
use crate::remote::{with_server_anime, with_server_manga};
use crate::repo::UnitRef;
use crate::tracker::{AnimeTracker, MangaTracker, Tracker};

/// The largest number of the leading run of seen units, or `default` when the run is empty. Only the run from
/// the lowest number counts, so a gap ends it (`takeWhile`).
pub(crate) fn continuous_last(sorted: &[UnitRef], default: f64) -> f64 {
    sorted
        .iter()
        .take_while(|u| u.seen)
        .last()
        .map_or(default, |u| u.number)
}

/// The ids of the units at or below `last` that are not seen yet. Nothing when `last` is 0 (nothing watched there), so
/// an episode 0 is not marked by an empty remote.
fn unseen_up_to(units: &[UnitRef], last: f64) -> Vec<i64> {
    if last <= 0.0 {
        return Vec::new();
    }
    units.iter().filter(|u| u.number >= 0.0 && u.number <= last && !u.seen).map(|u| u.id).collect()
}

/// Marks the local episodes up to the remote number seen; an enhanced tracker then also takes the local number
/// (`SyncEpisodeProgressWithTrack`). Returns how many episodes it marked. Failures are logged, not returned, as the
/// source does.
pub async fn sync_anime_progress(
    ctx: &TrackCtx,
    entry_id: i64,
    remote: &AnimeTrack,
    api: &dyn AnimeTracker,
) -> usize {
    let result = if api.as_enhanced_anime().is_some() {
        sync_anime_inner(ctx, entry_id, remote, api).await
    } else {
        async {
            let marked = unseen_up_to(&ctx.anime_episodes.list(entry_id).await?, remote.last_episode_seen);
            ctx.anime_episodes.mark_seen(&marked).await?;
            Ok(marked.len())
        }
        .await
    };
    result.unwrap_or_else(|e: TrackError| {
        tracing::warn!(entry = entry_id, tracker = remote.tracker_id.0, error = %redact(&e.to_string()), "episode progress sync failed");
        0
    })
}

async fn sync_anime_inner(
    ctx: &TrackCtx,
    entry_id: i64,
    remote: &AnimeTrack,
    api: &dyn AnimeTracker,
) -> Result<usize, TrackError> {
    let mut episodes = ctx.anime_episodes.list(entry_id).await?;
    episodes.retain(|e| e.number >= 0.0);
    episodes.sort_by(|a, b| a.number.total_cmp(&b.number));

    let newly_seen: Vec<i64> = episodes
        .iter()
        .filter(|e| e.number <= remote.last_episode_seen && !e.seen)
        .map(|e| e.id)
        .collect();
    let last = remote
        .last_episode_seen
        .max(continuous_last(&episodes, 0.0));
    let updated = AnimeTrack {
        last_episode_seen: last,
        ..remote.clone()
    };

    let server = api.update(&updated, false).await?;
    ctx.anime_episodes.mark_seen(&newly_seen).await?;
    ctx.save_anime(&with_server_anime(&updated, &server))
        .await?;
    Ok(newly_seen.len())
}

/// Manga form of [`sync_anime_progress`] over chapters (`SyncChapterProgressWithTrack`).
pub async fn sync_manga_progress(
    ctx: &TrackCtx,
    manga_id: i64,
    remote: &MangaTrack,
    api: &dyn MangaTracker,
) -> usize {
    let result = if api.as_enhanced_manga().is_some() {
        sync_manga_inner(ctx, manga_id, remote, api).await
    } else {
        async {
            let marked = unseen_up_to(&ctx.manga_chapters.list(manga_id).await?, remote.last_chapter_read);
            ctx.manga_chapters.mark_read(&marked).await?;
            Ok(marked.len())
        }
        .await
    };
    result.unwrap_or_else(|e: TrackError| {
        tracing::warn!(entry = manga_id, tracker = remote.tracker_id.0, error = %redact(&e.to_string()), "chapter progress sync failed");
        0
    })
}

async fn sync_manga_inner(
    ctx: &TrackCtx,
    manga_id: i64,
    remote: &MangaTrack,
    api: &dyn MangaTracker,
) -> Result<usize, TrackError> {
    let mut chapters = ctx.manga_chapters.list(manga_id).await?;
    chapters.retain(|c| c.number >= 0.0);
    chapters.sort_by(|a, b| a.number.total_cmp(&b.number));

    let newly_read: Vec<i64> = chapters
        .iter()
        .filter(|c| c.number <= remote.last_chapter_read && !c.seen)
        .map(|c| c.id)
        .collect();
    let last = remote
        .last_chapter_read
        .max(continuous_last(&chapters, 0.0));
    let updated = MangaTrack {
        last_chapter_read: last,
        ..remote.clone()
    };

    let server = api.update(&updated, false).await?;
    ctx.manga_chapters.mark_read(&newly_read).await?;
    ctx.save_manga(&with_server_manga(&updated, &server))
        .await?;
    Ok(newly_read.len())
}

/// The log line of a failed refresh (T-27b-034, D-5). It names the entry as `animeId=` or `mangaId=`, then the tracker
/// id and name. It never carries the error text, a token or a URL.
pub fn refresh_failure_log(kind: Kind, entry_id: i64, tracker: TrackerId, name: &str) -> String {
    let key = match kind {
        Kind::Anime => "animeId",
        Kind::Manga => "mangaId",
    };
    format!("Failed to refresh track data {key}={entry_id} for service {} ({name})", tracker.0)
}

/// What a refresh of one entry did.
#[derive(Debug, Default)]
pub struct Refreshed {
    /// The trackers that failed, with their error.
    pub failures: Vec<(TrackerId, TrackError)>,
    /// The trackers that marked local units seen, with how many.
    pub marked: Vec<(TrackerId, usize)>,
}

/// Refreshes every logged-in anime binding of the entry, concurrently, and pulls each tracker's progress. A failure
/// does not stop the others. Logged-out bindings are skipped without a failure.
pub async fn refresh_anime(ctx: &TrackCtx, entry_id: i64) -> Result<Refreshed, TrackError> {
    let bindings = ctx.anime_tracks.list_for_entry(entry_id).await?;
    let work: Vec<_> = bindings
        .iter()
        .filter_map(|b| ctx.trackers.get(b.tracker_id).map(|t| (b.tracker_id, t)))
        .filter(|(id, t)| t.is_logged_in() && ctx.trackers.is_enabled(*id))
        .collect();
    let results = join_all(work.iter().map(|(id, t)| async move {
        (refresh_one_anime(ctx, entry_id, *id, &**t).await, *id, t.name())
    }))
    .await;
    let mut out = Refreshed::default();
    for (result, id, name) in results {
        match result {
            Ok(0) => {}
            Ok(n) => out.marked.push((id, n)),
            Err(e) => {
                tracing::error!("{}", refresh_failure_log(Kind::Anime, entry_id, id, name));
                out.failures.push((id, e));
            }
        }
    }
    Ok(out)
}

async fn refresh_one_anime(
    ctx: &TrackCtx,
    entry_id: i64,
    id: TrackerId,
    tracker: &dyn Tracker,
) -> Result<usize, TrackError> {
    let Some(api) = tracker.as_anime() else {
        return Ok(0);
    };
    let lock = ctx.locks.for_binding(Kind::Anime, entry_id, id);
    let _guard = lock.lock().await;
    let Some(binding) = ctx.anime_binding(entry_id, id).await? else {
        return Ok(0);
    };
    let fresh = {
        let _permit = ctx.refresh_limit.acquire(id).await;
        api.refresh(&binding).await?
    };
    let updated = AnimeTrack {
        id: binding.id,
        anime_id: binding.anime_id,
        tracker_id: binding.tracker_id,
        ..fresh
    };
    ctx.save_anime(&updated).await?;
    Ok(sync_anime_progress(ctx, entry_id, &updated, api).await)
}

/// Manga form of [`refresh_anime`].
pub async fn refresh_manga(ctx: &TrackCtx, manga_id: i64) -> Result<Refreshed, TrackError> {
    let bindings = ctx.manga_tracks.list_for_entry(manga_id).await?;
    let work: Vec<_> = bindings
        .iter()
        .filter_map(|b| ctx.trackers.get(b.tracker_id).map(|t| (b.tracker_id, t)))
        .filter(|(id, t)| t.is_logged_in() && ctx.trackers.is_enabled(*id))
        .collect();
    let results = join_all(work.iter().map(|(id, t)| async move {
        (refresh_one_manga(ctx, manga_id, *id, &**t).await, *id, t.name())
    }))
    .await;
    let mut out = Refreshed::default();
    for (result, id, name) in results {
        match result {
            Ok(0) => {}
            Ok(n) => out.marked.push((id, n)),
            Err(e) => {
                tracing::error!("{}", refresh_failure_log(Kind::Manga, manga_id, id, name));
                out.failures.push((id, e));
            }
        }
    }
    Ok(out)
}

async fn refresh_one_manga(
    ctx: &TrackCtx,
    manga_id: i64,
    id: TrackerId,
    tracker: &dyn Tracker,
) -> Result<usize, TrackError> {
    let Some(api) = tracker.as_manga() else {
        return Ok(0);
    };
    let lock = ctx.locks.for_binding(Kind::Manga, manga_id, id);
    let _guard = lock.lock().await;
    let Some(binding) = ctx.manga_binding(manga_id, id).await? else {
        return Ok(0);
    };
    let fresh = {
        let _permit = ctx.refresh_limit.acquire(id).await;
        api.refresh(&binding).await?
    };
    let updated = MangaTrack {
        id: binding.id,
        manga_id: binding.manga_id,
        tracker_id: binding.tracker_id,
        ..fresh
    };
    ctx.save_manga(&updated).await?;
    Ok(sync_manga_progress(ctx, manga_id, &updated, api).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::REFRESH_PARALLELISM;
    use crate::fakes::{Harness, anime_binding, manga_binding};

    #[test]
    fn continuous_last_stops_at_the_first_gap() {
        let u = |n: f64, seen: bool| UnitRef {
            id: 0,
            number: n,
            seen,
        };
        let units = [u(1.0, true), u(2.0, true), u(3.0, false), u(4.0, true)];
        assert_eq!(continuous_last(&units, 0.0), 2.0);
        assert_eq!(continuous_last(&[u(1.0, false)], 0.0), 0.0);
    }

    #[tokio::test]
    async fn a_non_enhanced_tracker_marks_episodes_up_to_its_number_and_writes_nothing_back() {
        let h = Harness::new();
        h.login(TrackerId::KITSU);
        h.episodes.add(1, 1.0, true);
        let ep2 = h.episodes.add(1, 2.0, false);
        let ep6 = h.episodes.add(1, 6.0, false);
        let remote = anime_binding(1, TrackerId::KITSU, 5.0);
        let marked = sync_anime_progress(&h.ctx, 1, &remote, &*h.anime_tracker(TrackerId::KITSU)).await;
        assert_eq!(marked, 1, "only episode 2: 1 was seen already and 6 is past the tracker's 5");
        assert!(!h.log.lock().unwrap().iter().any(|l| l.starts_with("update")), "the tracker is not written");
        let seen = h.episodes.seen_ids();
        assert!(seen.contains(&ep2) && !seen.contains(&ep6));
    }

    #[tokio::test]
    async fn an_empty_remote_marks_nothing_not_even_an_episode_zero() {
        let h = Harness::new();
        h.login(TrackerId::KITSU);
        let ep0 = h.episodes.add(1, 0.0, false);
        let remote = anime_binding(1, TrackerId::KITSU, 0.0);
        assert_eq!(sync_anime_progress(&h.ctx, 1, &remote, &*h.anime_tracker(TrackerId::KITSU)).await, 0);
        assert!(!h.episodes.seen_ids().contains(&ep0));
    }

    #[tokio::test]
    async fn tr_27_108_enhanced_sync_marks_episodes_up_to_the_remote_value() {
        let h = Harness::new();
        h.login(TrackerId::JELLYFIN);
        // Episodes 1..5 with 1, 2 and 4 seen: the continuous run is 2.
        h.episodes.add(1, 1.0, true);
        h.episodes.add(1, 2.0, true);
        let e3 = h.episodes.add(1, 3.0, false);
        h.episodes.add(1, 4.0, true);
        h.episodes.add(1, 5.0, false);
        let remote = anime_binding(1, TrackerId::JELLYFIN, 3.0);

        sync_anime_progress(&h.ctx, 1, &remote, &*h.anime_tracker(TrackerId::JELLYFIN)).await;

        assert!(h.log.lock().unwrap().contains(&"update:false".to_string()));
        assert!(
            h.episodes.seen_ids().contains(&e3),
            "episode 3 is at or below the remote 3"
        );
        let row = h.ctx.anime_tracks.list_for_entry(1).await.unwrap();
        assert_eq!(row[0].last_episode_seen, 3.0, "max(remote 3, local run 2)");
    }

    #[tokio::test]
    async fn tr_27_109_remote_value_never_decreases() {
        let h = Harness::new();
        h.login(TrackerId::JELLYFIN);
        for n in 1..=5 {
            h.episodes.add(2, n as f64, true);
        }
        let remote = anime_binding(2, TrackerId::JELLYFIN, 8.0);
        sync_anime_progress(&h.ctx, 2, &remote, &*h.anime_tracker(TrackerId::JELLYFIN)).await;
        assert_eq!(
            h.ctx.anime_tracks.list_for_entry(2).await.unwrap()[0].last_episode_seen,
            8.0
        );
    }

    #[tokio::test]
    async fn tr_27_109_a_failed_remote_update_writes_nothing_local() {
        let h = Harness::new();
        h.login(TrackerId::JELLYFIN);
        let e1 = h.episodes.add(7, 1.0, false);
        *h.anime_tracker(TrackerId::JELLYFIN).fail.lock().unwrap() = Some("down".into());
        let remote = anime_binding(7, TrackerId::JELLYFIN, 1.0);
        sync_anime_progress(&h.ctx, 7, &remote, &*h.anime_tracker(TrackerId::JELLYFIN)).await;
        assert!(
            !h.episodes.seen_ids().contains(&e1),
            "remote first, so nothing local"
        );
    }

    #[tokio::test]
    async fn tr_27_038_manga_sync_marks_chapters_read_for_an_enhanced_tracker() {
        let h = Harness::new();
        h.login(TrackerId::KAVITA);
        let c1 = h.chapters.add(3, 1.0, false);
        let remote = manga_binding(3, TrackerId::KAVITA, 1.0);
        sync_manga_progress(&h.ctx, 3, &remote, &*h.manga_tracker(TrackerId::KAVITA)).await;
        assert!(h.chapters.seen_ids().contains(&c1));
        assert_eq!(
            h.ctx.manga_tracks.list_for_entry(3).await.unwrap()[0].last_chapter_read,
            1.0
        );
    }

    #[tokio::test]
    async fn tr_27_035_refresh_skips_logged_out_bindings_and_collects_failures() {
        let h = Harness::new();
        h.login(TrackerId::KITSU);
        h.login(TrackerId::ANILIST);
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(4, TrackerId::KITSU, 2.0))
            .await
            .unwrap();
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(4, TrackerId::ANILIST, 2.0))
            .await
            .unwrap();
        // Simkl is not logged in. Its failure switch must not produce an entry.
        h.ctx
            .anime_tracks
            .upsert(&anime_binding(4, TrackerId::SIMKL, 2.0))
            .await
            .unwrap();
        *h.anime_tracker(TrackerId::SIMKL).fail.lock().unwrap() = Some("never".into());
        *h.anime_tracker(TrackerId::KITSU).fail.lock().unwrap() = Some("kitsu down".into());

        let failures = refresh_anime(&h.ctx, 4).await.unwrap().failures;
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, TrackerId::KITSU);
        assert_eq!(failures[0].1.to_string(), "kitsu down");
        // AniList still refreshed and stored its row.
        assert_eq!(h.ctx.anime_tracks.list_for_entry(4).await.unwrap().len(), 3);
    }

    #[test]
    fn t_27b_034_anime_and_manga_log_lines_use_their_own_key_and_no_secret() {
        assert_eq!(
            refresh_failure_log(Kind::Anime, 7, TrackerId::KITSU, "Kitsu"),
            "Failed to refresh track data animeId=7 for service 3 (Kitsu)"
        );
        assert_eq!(
            refresh_failure_log(Kind::Manga, 9, TrackerId::KAVITA, "Kavita"),
            "Failed to refresh track data mangaId=9 for service 8 (Kavita)"
        );
    }

    #[tokio::test]
    async fn t_27a_081_one_tracker_serves_at_most_four_refresh_requests_at_once() {
        let h = Harness::new();
        let mut held = Vec::new();
        for _ in 0..REFRESH_PARALLELISM {
            held.push(h.ctx.refresh_limit.acquire(TrackerId::ANILIST).await.unwrap());
        }
        let fifth = tokio::time::timeout(
            std::time::Duration::from_millis(30),
            h.ctx.refresh_limit.acquire(TrackerId::ANILIST),
        )
        .await;
        assert!(fifth.is_err(), "the fifth request waits for a permit");
        let other = tokio::time::timeout(
            std::time::Duration::from_millis(30),
            h.ctx.refresh_limit.acquire(TrackerId::KITSU),
        )
        .await;
        assert!(other.is_ok(), "another tracker is not held back");
        held.pop();
        let again = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            h.ctx.refresh_limit.acquire(TrackerId::ANILIST),
        )
        .await;
        assert!(again.is_ok(), "a released permit lets the next request run");
    }

    #[tokio::test]
    async fn tr_27_036_manga_refresh_returns_the_failure_of_one_tracker() {
        let h = Harness::new();
        h.login(TrackerId::MANGAUPDATES);
        h.login(TrackerId::KAVITA);
        h.ctx
            .manga_tracks
            .upsert(&manga_binding(5, TrackerId::MANGAUPDATES, 1.0))
            .await
            .unwrap();
        h.ctx
            .manga_tracks
            .upsert(&manga_binding(5, TrackerId::KAVITA, 1.0))
            .await
            .unwrap();
        *h.manga_tracker(TrackerId::KAVITA).fail.lock().unwrap() = Some("kavita".into());
        let failures = refresh_manga(&h.ctx, 5).await.unwrap().failures;
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, TrackerId::KAVITA);
    }
}
