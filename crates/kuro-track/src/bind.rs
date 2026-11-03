//! Bind a search result to an entry (`AddAnimeTracks.kt`, `AddMangaTracks.kt`).
//! `register_anime` is the plain bind; `bind_anime` and `bind_manga` add the read history and the progress sync;
//! `bind_enhanced_*` match the entry against the installed sources.

use kuro_platform::kind::Kind;
use std::sync::Arc;

use tokio::task::JoinHandle;

use crate::ctx::TrackCtx;
use crate::detach::detached_locked;
use crate::error::TrackError;
use crate::error::redact;
use crate::events::TrackEvent;
use crate::messages::MsgKey;
use crate::model::search::{AnimeTrackSearch, MangaTrackSearch};
use crate::model::{AnimeTrack, MangaTrack, TrackerId};
use crate::remote::{
    set_anime_start_date, set_last_chapter_read, set_last_episode_seen, set_manga_start_date,
};
use crate::repo::{AnimeTrackRepo, MangaTrackRepo, UnitRef};
use crate::sync::{sync_anime_progress, sync_manga_progress};
use crate::tracker::{AnimeTracker, MangaTracker, Tracker};
use crate::trackers::enhanced::EntryRef;

/// Sets the entry id on `result`, binds it with the tracker, then stores the binding. A bind failure is
/// returned as is, with its raw message, and nothing is stored.
pub async fn register_anime(
    entry_id: i64,
    result: &mut AnimeTrackSearch,
    tracker: &dyn AnimeTracker,
    has_seen: bool,
    repo: &dyn AnimeTrackRepo,
) -> Result<AnimeTrack, TrackError> {
    result.anime_id = entry_id;
    let mut item = result.to_track();
    tracker.bind(&mut item, has_seen).await?;
    let id = repo.upsert(&item).await?;
    Ok(AnimeTrack {
        id: Some(id),
        ..item
    })
}

/// Starts [`register_anime`] detached (T-27b-020): the bind and the stored row survive the dialog closing. The
/// binding lock is taken here, in submission order. [`TrackEvent::TrackChanged`] is sent once the row is stored.
pub fn spawn_register_anime(
    ctx: Arc<TrackCtx>,
    entry_id: i64,
    mut result: AnimeTrackSearch,
    tracker: TrackerId,
    has_seen: bool,
) -> JoinHandle<Result<AnimeTrack, TrackError>> {
    let lock = ctx.locks.for_binding(Kind::Anime, entry_id, tracker);
    detached_locked(vec![lock], async move {
        let api = ctx
            .trackers
            .get(tracker)
            .ok_or(TrackError::UnknownTracker(tracker.0))?;
        let api = api
            .as_anime()
            .ok_or(TrackError::UnknownTracker(tracker.0))?;
        let stored = register_anime(
            entry_id,
            &mut result,
            api,
            has_seen,
            ctx.anime_tracks.as_ref(),
        )
        .await?;
        ctx.events.send(TrackEvent::TrackChanged { entry_id });
        Ok(stored)
    })
}

/// Manga form of [`register_anime`].
pub async fn register_manga(
    entry_id: i64,
    result: &mut MangaTrackSearch,
    tracker: &dyn MangaTracker,
    has_read: bool,
    repo: &dyn MangaTrackRepo,
) -> Result<MangaTrack, TrackError> {
    result.manga_id = entry_id;
    let mut item = result.to_track();
    tracker.bind(&mut item, has_read).await?;
    let id = repo.upsert(&item).await?;
    Ok(MangaTrack {
        id: Some(id),
        ..item
    })
}

/// Starts [`register_manga`] detached (T-27b-020): the bind and the stored row survive the dialog closing. The
/// binding lock is taken here, in submission order. [`TrackEvent::TrackChanged`] is sent once the row is stored.
pub fn spawn_register_manga(
    ctx: Arc<TrackCtx>,
    entry_id: i64,
    mut result: MangaTrackSearch,
    tracker: TrackerId,
    has_read: bool,
) -> JoinHandle<Result<MangaTrack, TrackError>> {
    let lock = ctx.locks.for_binding(Kind::Manga, entry_id, tracker);
    detached_locked(vec![lock], async move {
        let api = ctx
            .trackers
            .get(tracker)
            .ok_or(TrackError::UnknownTracker(tracker.0))?;
        let api = api
            .as_manga()
            .ok_or(TrackError::UnknownTracker(tracker.0))?;
        let stored = register_manga(
            entry_id,
            &mut result,
            api,
            has_read,
            ctx.manga_tracks.as_ref(),
        )
        .await?;
        ctx.events.send(TrackEvent::TrackChanged { entry_id });
        Ok(stored)
    })
}

/// Latest read number of the entry: the largest number of the leading run of seen units, `-1.0` when there is
/// none. Unrecognised numbers take part (behaviour 6.1 step 6a).
pub(crate) fn latest_read(units: &[UnitRef]) -> f64 {
    let mut sorted = units.to_vec();
    sorted.sort_by(|a, b| a.number.total_cmp(&b.number));
    sorted
        .iter()
        .take_while(|u| u.seen)
        .last()
        .map_or(-1.0, |u| u.number)
}

/// The earliest history time that is set. Reset rows (`None`, or zero) are ignored (defect D20).
pub(crate) fn earliest(times: &[Option<i64>]) -> Option<i64> {
    times.iter().flatten().copied().filter(|&t| t > 0).min()
}

/// Epoch milliseconds moved by the local UTC offset. The wall clock is kept and read as UTC
/// (`convertEpochMillisZone(systemDefault, UTC)`).
pub fn convert_epoch_millis_zone(ms: i64, offset_secs: i64) -> i64 {
    ms + offset_secs * 1000
}

/// The local UTC offset in seconds at instant `ms`.
pub fn local_offset_secs(ms: i64) -> i64 {
    use chrono::{Local, Offset, TimeZone};
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| {
            Local
                .offset_from_utc_datetime(&dt.naive_utc())
                .fix()
                .local_minus_utc() as i64
        })
        .unwrap_or(0)
}

/// Binds an anime (`AddAnimeTracks.bind`, behaviour 6.1). The local write happens once, after the setters, so it
/// carries the latest values (D19). The progress sync runs last.
pub async fn bind_anime(
    ctx: &TrackCtx,
    entry_id: i64,
    api: &dyn AnimeTracker,
    mut item: AnimeTrack,
    now: i64,
) -> Result<AnimeTrack, TrackError> {
    item.anime_id = entry_id;
    let episodes = ctx.anime_episodes.list(entry_id).await?;
    let has_seen = episodes.iter().any(|e| e.seen);
    api.bind(&mut item, has_seen).await?;

    if has_seen {
        let latest = latest_read(&episodes);
        if latest > item.last_episode_seen {
            set_last_episode_seen(&mut item, api, latest as i32, now);
        }
        if item.start_date <= 0 {
            let times = ctx.anime_history.seen_times(entry_id).await?;
            if let Some(first) = earliest(&times) {
                set_anime_start_date(
                    &mut item,
                    convert_epoch_millis_zone(first, local_offset_secs(first)),
                );
            }
        }
    }

    item.id = Some(ctx.save_anime(&item).await?);
    sync_anime_progress(ctx, entry_id, &item, api).await;
    Ok(item)
}

/// Binds a manga (`AddMangaTracks.bind`). Same flow as [`bind_anime`] over chapters and read history.
pub async fn bind_manga(
    ctx: &TrackCtx,
    manga_id: i64,
    api: &dyn MangaTracker,
    mut item: MangaTrack,
    now: i64,
) -> Result<MangaTrack, TrackError> {
    item.manga_id = manga_id;
    let chapters = ctx.manga_chapters.list(manga_id).await?;
    let has_read = chapters.iter().any(|c| c.seen);
    api.bind(&mut item, has_read).await?;

    if has_read {
        let latest = latest_read(&chapters);
        if latest > item.last_chapter_read {
            set_last_chapter_read(&mut item, api, latest as i32, now);
        }
        if item.start_date <= 0 {
            let times = ctx.manga_history.read_times(manga_id).await?;
            if let Some(first) = earliest(&times) {
                set_manga_start_date(
                    &mut item,
                    convert_epoch_millis_zone(first, local_offset_secs(first)),
                );
            }
        }
    }

    item.id = Some(ctx.save_manga(&item).await?);
    sync_manga_progress(ctx, manga_id, &item, api).await;
    Ok(item)
}

/// [`bind_enhanced_anime_entry`] for a caller that only has the id, the title and the source id. The `EntryRef` it
/// builds has no `url`, so a tracker that matches by URL fails for it; callers that know the entry URL use
/// [`bind_enhanced_anime_entry`].
pub async fn bind_enhanced_anime(
    ctx: &TrackCtx,
    entry_id: i64,
    entry_title: &str,
    source_id: &str,
) -> Vec<(TrackerId, TrackError)> {
    let entry = EntryRef {
        id: entry_id,
        source_id: source_id.parse().unwrap_or_default(),
        url: String::new(),
        title: entry_title.to_owned(),
    };
    bind_enhanced_anime_entry(ctx, &entry, source_id).await
}

/// Binds the entry to every logged-in enhanced anime tracker that accepts `source_identity`, in registry order
/// (`bindEnhancedTrackers`, behaviour 6.2). A tracker that already has a binding is skipped. A failure is logged
/// with the entry title and tracker, and the next tracker still runs. The failures are returned for the caller.
pub async fn bind_enhanced_anime_entry(
    ctx: &TrackCtx,
    entry: &EntryRef,
    source_identity: &str,
) -> Vec<(TrackerId, TrackError)> {
    let mut failures = Vec::new();
    // an automatic bind never runs while Incognito is on (`TrackCtx::incognito`)
    if ctx.incognito() {
        return failures;
    }
    for tracker in ctx.trackers.logged_in() {
        let (Some(api), Some(enhanced)) = (tracker.as_anime(), tracker.as_enhanced_anime()) else {
            continue;
        };
        if !enhanced.accepts(source_identity) {
            continue;
        }
        let id = tracker.id();
        if let Err(e) = bind_enhanced_one_anime(ctx, entry, api, enhanced).await {
            tracing::warn!(entry = entry.title, tracker = id.0, error = %redact(&e.to_string()), "could not match anime with tracker");
            failures.push((id, e));
        }
    }
    failures
}

async fn bind_enhanced_one_anime(
    ctx: &TrackCtx,
    entry: &EntryRef,
    api: &dyn AnimeTracker,
    enhanced: &dyn crate::tracker::EnhancedAnimeTracker,
) -> Result<(), TrackError> {
    let entry_id = entry.id;
    if ctx.anime_binding(entry_id, api.id()).await?.is_some() {
        return Ok(());
    }
    let Some(mut search) = enhanced.match_entry(entry).await? else {
        return Ok(());
    };
    search.anime_id = entry_id;
    let mut item = search.to_track();
    api.bind(&mut item, false).await?;
    item.id = Some(ctx.save_anime(&item).await?);
    sync_anime_progress(ctx, entry_id, &item, api).await;
    Ok(())
}

/// Manga form of [`bind_enhanced_anime`].
pub async fn bind_enhanced_manga(
    ctx: &TrackCtx,
    manga_id: i64,
    manga_title: &str,
    source_id: &str,
) -> Vec<(TrackerId, TrackError)> {
    let entry = EntryRef {
        id: manga_id,
        source_id: source_id.parse().unwrap_or_default(),
        url: String::new(),
        title: manga_title.to_owned(),
    };
    bind_enhanced_manga_entry(ctx, &entry, source_id).await
}

/// Manga form of [`bind_enhanced_anime_entry`].
pub async fn bind_enhanced_manga_entry(
    ctx: &TrackCtx,
    entry: &EntryRef,
    source_identity: &str,
) -> Vec<(TrackerId, TrackError)> {
    let mut failures = Vec::new();
    if ctx.incognito() {
        return failures;
    }
    for tracker in ctx.trackers.logged_in() {
        let (Some(api), Some(enhanced)) = (tracker.as_manga(), tracker.as_enhanced_manga()) else {
            continue;
        };
        if !enhanced.accepts(source_identity) {
            continue;
        }
        let id = tracker.id();
        if let Err(e) = bind_enhanced_one_manga(ctx, entry, api, enhanced).await {
            tracing::warn!(entry = entry.title, tracker = id.0, error = %redact(&e.to_string()), "could not match manga with tracker");
            failures.push((id, e));
        }
    }
    failures
}

async fn bind_enhanced_one_manga(
    ctx: &TrackCtx,
    entry: &EntryRef,
    api: &dyn MangaTracker,
    enhanced: &dyn crate::tracker::EnhancedMangaTracker,
) -> Result<(), TrackError> {
    let manga_id = entry.id;
    if ctx.manga_binding(manga_id, api.id()).await?.is_some() {
        return Ok(());
    }
    let Some(mut search) = enhanced.match_entry(entry).await? else {
        return Ok(());
    };
    search.manga_id = manga_id;
    let mut item = search.to_track();
    api.bind(&mut item, false).await?;
    item.id = Some(ctx.save_manga(&item).await?);
    sync_manga_progress(ctx, manga_id, &item, api).await;
    Ok(())
}

/// What "Add tracking" does on an unbound row (T-27a-075).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddTrackingAction {
    /// An enhanced tracker matches the entry and registers the match, with no search page.
    MatchAndRegister,
    /// Any other tracker opens its search screen.
    OpenSearch,
}

/// The "Add tracking" action of `tracker` for an entry of `kind`.
pub fn add_tracking_action(tracker: &dyn Tracker, kind: Kind) -> AddTrackingAction {
    let enhanced = match kind {
        Kind::Anime => tracker.as_enhanced_anime().is_some(),
        Kind::Manga => tracker.as_enhanced_manga().is_some(),
    };
    if enhanced {
        AddTrackingAction::MatchAndRegister
    } else {
        AddTrackingAction::OpenSearch
    }
}

/// "Add tracking" on an enhanced anime tracker (B§3.9): match the entry, then register the match, with no search
/// page. `entry` is `None` when the entry is no longer in the library: nothing is shown then (E-09). No match gives "No match found" (E-07). A failure gives the tracker's toast (E-08).
pub async fn add_enhanced_anime(
    ctx: &TrackCtx,
    tracker: TrackerId,
    entry: Option<&EntryRef>,
) -> Result<Option<AnimeTrack>, MsgKey> {
    let Some(entry) = entry else {
        return Ok(None);
    };
    let entry_id = entry.id;
    let tr = ctx
        .trackers
        .get(tracker)
        .ok_or_else(|| TrackError::UnknownTracker(tracker.0).toast(&tracker.0.to_string()))?;
    let name = tr.name().to_owned();
    let (Some(api), Some(enhanced)) = (tr.as_anime(), tr.as_enhanced_anime()) else {
        return Err(TrackError::UnknownTracker(tracker.0).toast(&name));
    };
    let mut found = enhanced
        .match_entry(entry)
        .await
        .map_err(|e| e.toast(&name))?
        .ok_or(MsgKey::NoMatchFound)?;
    let stored = register_anime(entry_id, &mut found, api, false, ctx.anime_tracks.as_ref())
        .await
        .map_err(|e| e.toast(&name))?;
    ctx.events.send(TrackEvent::TrackChanged { entry_id });
    Ok(Some(stored))
}

/// Manga form of [`add_enhanced_anime`].
pub async fn add_enhanced_manga(
    ctx: &TrackCtx,
    tracker: TrackerId,
    entry: Option<&EntryRef>,
) -> Result<Option<MangaTrack>, MsgKey> {
    let Some(entry) = entry else {
        return Ok(None);
    };
    let manga_id = entry.id;
    let tr = ctx
        .trackers
        .get(tracker)
        .ok_or_else(|| TrackError::UnknownTracker(tracker.0).toast(&tracker.0.to_string()))?;
    let name = tr.name().to_owned();
    let (Some(api), Some(enhanced)) = (tr.as_manga(), tr.as_enhanced_manga()) else {
        return Err(TrackError::UnknownTracker(tracker.0).toast(&name));
    };
    let mut found = enhanced
        .match_entry(entry)
        .await
        .map_err(|e| e.toast(&name))?
        .ok_or(MsgKey::NoMatchFound)?;
    let stored = register_manga(manga_id, &mut found, api, false, ctx.manga_tracks.as_ref())
        .await
        .map_err(|e| e.toast(&name))?;
    ctx.events
        .send(TrackEvent::TrackChanged { entry_id: manga_id });
    Ok(Some(stored))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{
        FakeAnime, Harness, MemAnimeRepo, MemSecrets, anime_binding, manga_binding, new_log,
    };
    use crate::model::TrackerId;

    fn entry(id: i64, title: &str) -> EntryRef {
        EntryRef {
            id,
            source_id: 7,
            url: format!("/series/{id}"),
            title: title.into(),
        }
    }

    #[test]
    fn t_27a_075_enhanced_trackers_match_and_others_open_the_search_screen() {
        let secrets = MemSecrets::new();
        let log = new_log();
        let mut enhanced = FakeAnime::new(
            TrackerId::JELLYFIN,
            "Jellyfin",
            secrets.clone(),
            log.clone(),
        );
        enhanced.enhanced = true;
        let mut plain = FakeAnime::new(TrackerId::KITSU, "Kitsu", secrets, log);
        plain.enhanced = false;
        assert_eq!(
            add_tracking_action(&enhanced, Kind::Anime),
            AddTrackingAction::MatchAndRegister
        );
        assert_eq!(
            add_tracking_action(&plain, Kind::Anime),
            AddTrackingAction::OpenSearch
        );
    }

    #[tokio::test]
    async fn tr_27b_013_add_tracking_on_an_enhanced_tracker_binds_the_match_without_a_search() {
        let h = Harness::new();
        h.login(TrackerId::JELLYFIN);
        let got = add_enhanced_anime(&h.ctx, TrackerId::JELLYFIN, Some(&entry(4, "Show")))
            .await
            .unwrap()
            .unwrap();
        assert_eq!((got.anime_id, got.tracker_id), (4, TrackerId::JELLYFIN));
        assert_eq!(h.anime_repo.list_for_entry(4).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn tr_27b_013_no_match_gives_the_no_match_toast_and_stores_nothing() {
        let h = Harness::new();
        h.login(TrackerId::JELLYFIN);
        let err = add_enhanced_anime(&h.ctx, TrackerId::JELLYFIN, Some(&entry(4, "")))
            .await
            .unwrap_err();
        assert_eq!(err, MsgKey::NoMatchFound);
        assert!(h.anime_repo.list_for_entry(4).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn tr_27b_013_a_missing_entry_returns_silently() {
        let h = Harness::new();
        h.login(TrackerId::JELLYFIN);
        assert_eq!(
            add_enhanced_anime(&h.ctx, TrackerId::JELLYFIN, None).await,
            Ok(None)
        );
    }

    #[tokio::test]
    async fn tr_27b_013_a_register_failure_is_prefixed_with_the_tracker_name() {
        let h = Harness::new();
        h.login(TrackerId::JELLYFIN);
        *h.anime_tracker(TrackerId::JELLYFIN).fail.lock().unwrap() = Some("server said no".into());
        let err = add_enhanced_anime(&h.ctx, TrackerId::JELLYFIN, Some(&entry(4, "Show")))
            .await
            .unwrap_err();
        assert_eq!(
            err,
            MsgKey::TrackerError {
                tracker: "Jellyfin".into(),
                detail: "server said no".into()
            }
        );
    }

    #[tokio::test]
    async fn tr_27_011_register_sets_the_entry_id_before_binding() {
        let log = new_log();
        let tr = FakeAnime::new(
            TrackerId::ANILIST,
            "AniList",
            MemSecrets::new(),
            log.clone(),
        );
        let repo = MemAnimeRepo::default();
        let mut result = AnimeTrackSearch::new(TrackerId::ANILIST, 3, "Show", "https://x");
        let stored = register_anime(42, &mut result, &tr, false, &repo)
            .await
            .unwrap();
        assert_eq!(result.anime_id, 42);
        assert_eq!(stored.anime_id, 42);
        assert_eq!(stored.id, Some(1));
        assert_eq!(*log.lock().unwrap(), vec!["bind".to_string()]);
    }

    #[tokio::test]
    async fn tr_27b_020_dropping_the_caller_does_not_cancel_a_spawned_register() {
        let h = Harness::new();
        h.login(TrackerId::ANILIST);
        let ctx = Arc::clone(&h.ctx);
        let caller = tokio::spawn(async move {
            let result = AnimeTrackSearch::new(TrackerId::ANILIST, 3, "Show", "https://x");
            drop(spawn_register_anime(
                ctx,
                42,
                result,
                TrackerId::ANILIST,
                false,
            ));
            std::future::pending::<()>().await;
        });
        tokio::task::yield_now().await;
        caller.abort();
        let _ = caller.await;
        for _ in 0..500 {
            if !h.anime_repo.list_for_entry(42).await.unwrap().is_empty() {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the register still ran after its caller was dropped");
    }

    #[tokio::test]
    async fn tr_27_r3_concurrent_binds_of_the_same_media_for_the_same_id_give_one_row() {
        let h = Harness::new();
        h.login(TrackerId::ANILIST);
        let first = AnimeTrackSearch::new(TrackerId::ANILIST, 3, "Show", "https://x");
        let second = first.clone();
        let a = spawn_register_anime(Arc::clone(&h.ctx), 42, first, TrackerId::ANILIST, false);
        let b = spawn_register_anime(Arc::clone(&h.ctx), 42, second, TrackerId::ANILIST, false);
        let (a, b) = (a.await.unwrap().unwrap(), b.await.unwrap().unwrap());
        assert_eq!(a.id, b.id, "both binds resolve to the same row");
        let rows = h.anime_repo.list_for_entry(42).await.unwrap();
        assert_eq!(
            rows.len(),
            1,
            "R-3: one row for the same media and local id"
        );
        assert_eq!(rows[0].remote_id, 3);

        h.login(TrackerId::KAVITA);
        let m1 = MangaTrackSearch::new(TrackerId::KAVITA, 8, "Book", "https://x");
        let m2 = m1.clone();
        let a = spawn_register_manga(Arc::clone(&h.ctx), 7, m1, TrackerId::KAVITA, false);
        let b = spawn_register_manga(Arc::clone(&h.ctx), 7, m2, TrackerId::KAVITA, false);
        let (a, b) = (a.await.unwrap().unwrap(), b.await.unwrap().unwrap());
        assert_eq!(a.id, b.id, "both binds resolve to the same row");
        assert_eq!(h.ctx.manga_tracks.list_for_entry(7).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn tr_27_011_bind_failure_returns_the_raw_message_and_stores_nothing() {
        let tr = FakeAnime::new(TrackerId::ANILIST, "AniList", MemSecrets::new(), new_log());
        *tr.fail.lock().unwrap() = Some("Not found".into());
        let repo = MemAnimeRepo::default();
        let mut result = AnimeTrackSearch::new(TrackerId::ANILIST, 3, "Show", "https://x");
        let err = register_anime(42, &mut result, &tr, false, &repo)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "Not found");
        assert!(repo.rows.lock().unwrap().is_empty());
    }

    #[test]
    fn tr_27_110_utc_conversion_keeps_the_wall_clock() {
        // 2026-03-01T22:30:00Z is 23:30 at UTC+1. The stored value is 23:30 read as UTC.
        let ms = chrono::DateTime::parse_from_rfc3339("2026-03-01T22:30:00Z")
            .unwrap()
            .timestamp_millis();
        let want = chrono::DateTime::parse_from_rfc3339("2026-03-01T23:30:00Z")
            .unwrap()
            .timestamp_millis();
        assert_eq!(convert_epoch_millis_zone(ms, 3600), want);
        assert_eq!(convert_epoch_millis_zone(ms, 0), ms);
    }

    #[test]
    fn tr_27_d20_a_null_history_row_is_ignored_for_the_first_date() {
        assert_eq!(
            earliest(&[None, Some(1_700_000_000_000)]),
            Some(1_700_000_000_000)
        );
        assert_eq!(
            earliest(&[None, Some(0)]),
            None,
            "a reset row is not a date"
        );
        assert_eq!(earliest(&[]), None);
    }

    fn unit(n: f64, seen: bool) -> UnitRef {
        UnitRef {
            id: 0,
            number: n,
            seen,
        }
    }

    #[test]
    fn latest_read_is_the_continuous_run_or_minus_one() {
        assert_eq!(
            latest_read(&[
                unit(1.0, true),
                unit(2.0, true),
                unit(3.0, false),
                unit(4.0, true)
            ]),
            2.0
        );
        assert_eq!(latest_read(&[unit(1.0, false)]), -1.0);
        assert_eq!(
            latest_read(&[unit(-1.0, true), unit(0.0, true)]),
            0.0,
            "unrecognised numbers count"
        );
    }

    #[tokio::test]
    async fn tr_27_d19_bind_writes_the_latest_values_after_the_setters() {
        let h = Harness::new();
        h.episodes.add(8, 1.0, true);
        h.episodes.add(8, 2.0, true);
        h.episodes.add(8, 3.0, false);
        let api = h.anime_tracker(TrackerId::KITSU);
        let item = anime_binding(0, TrackerId::KITSU, 0.0);
        let stored = bind_anime(&h.ctx, 8, &*api, item, 1_000).await.unwrap();

        assert!(stored.id.is_some());
        let row = h.ctx.anime_tracks.list_for_entry(8).await.unwrap();
        assert_eq!(row.len(), 1);
        assert_eq!(row[0].last_episode_seen, 2.0, "the latest seen episode");
        assert_eq!(
            row[0].status,
            api.watching_status(),
            "D19: the setter's status is stored"
        );
    }

    #[tokio::test]
    async fn tr_27_031_bind_with_no_seen_episodes_skips_the_history_lookup() {
        let h = Harness::new();
        h.episodes.add(9, 1.0, false);
        *h.anime_history.times.lock().unwrap() = vec![Some(1_700_000_000_000)];
        let api = h.anime_tracker(TrackerId::KITSU);
        let stored = bind_anime(&h.ctx, 9, &*api, anime_binding(9, TrackerId::KITSU, 0.0), 0)
            .await
            .unwrap();
        assert_eq!(stored.start_date, 0);
        assert_eq!(stored.last_episode_seen, 0.0);
    }

    #[tokio::test]
    async fn tr_27_031_bind_sets_the_start_date_from_the_first_history_row() {
        let h = Harness::new();
        h.episodes.add(10, 1.0, true);
        let first = 1_769_900_000_000_i64;
        *h.anime_history.times.lock().unwrap() = vec![None, Some(first + 60_000), Some(first)];
        let api = h.anime_tracker(TrackerId::KITSU);
        let stored = bind_anime(
            &h.ctx,
            10,
            &*api,
            anime_binding(10, TrackerId::KITSU, 0.0),
            0,
        )
        .await
        .unwrap();
        assert_eq!(
            stored.start_date,
            convert_epoch_millis_zone(first, local_offset_secs(first))
        );
    }

    #[tokio::test]
    async fn tr_27_031_bind_failure_propagates_and_stores_nothing() {
        let h = Harness::new();
        let api = h.anime_tracker(TrackerId::KITSU);
        *api.fail.lock().unwrap() = Some("Not found".into());
        let err = bind_anime(
            &h.ctx,
            11,
            &*api,
            anime_binding(11, TrackerId::KITSU, 0.0),
            0,
        )
        .await
        .unwrap_err();
        assert_eq!(err.to_string(), "Not found");
        assert!(
            h.ctx
                .anime_tracks
                .list_for_entry(11)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn tr_27_032_manga_bind_with_chapters_one_to_three_read_sets_progress_to_three() {
        let h = Harness::new();
        for n in 1..=3 {
            h.chapters.add(12, n as f64, true);
        }
        h.chapters.add(12, 4.0, false);
        let api = h.manga_tracker(TrackerId::MANGAUPDATES);
        let item = manga_binding(0, TrackerId::MANGAUPDATES, 0.0);
        bind_manga(&h.ctx, 12, &*api, item, 0).await.unwrap();
        assert_eq!(
            h.ctx.manga_tracks.list_for_entry(12).await.unwrap()[0].last_chapter_read,
            3.0
        );
    }

    #[tokio::test]
    async fn tr_27_033_enhanced_bind_matches_only_logged_in_accepting_trackers() {
        let h = Harness::new();
        h.login(TrackerId::JELLYFIN);
        let failures = bind_enhanced_anime(&h.ctx, 20, "Show", "src.jf").await;
        assert!(failures.is_empty());
        let rows = h.ctx.anime_tracks.list_for_entry(20).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tracker_id, TrackerId::JELLYFIN);
        assert_eq!(rows[0].anime_id, 20);
    }

    #[tokio::test]
    async fn tr_27_033_enhanced_bind_ignores_a_source_the_tracker_does_not_accept() {
        let h = Harness::new();
        h.login(TrackerId::JELLYFIN);
        bind_enhanced_anime(&h.ctx, 21, "Show", "src.other").await;
        assert!(
            h.ctx
                .anime_tracks
                .list_for_entry(21)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn tr_27_033_enhanced_bind_does_not_bind_a_tracker_twice() {
        let h = Harness::new();
        h.login(TrackerId::JELLYFIN);
        bind_enhanced_anime(&h.ctx, 22, "Show", "src.jf").await;
        bind_enhanced_anime(&h.ctx, 22, "Show", "src.jf").await;
        assert_eq!(
            h.ctx.anime_tracks.list_for_entry(22).await.unwrap().len(),
            1
        );
        let binds = h
            .log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| *l == "bind")
            .count();
        assert_eq!(binds, 1, "the second call does not bind again");
    }

    #[tokio::test]
    async fn tr_27_033_enhanced_failure_is_returned_with_its_tracker() {
        let h = Harness::new();
        h.login(TrackerId::JELLYFIN);
        *h.anime_tracker(TrackerId::JELLYFIN).fail.lock().unwrap() = Some("bind refused".into());
        let failures = bind_enhanced_anime(&h.ctx, 23, "Show", "src.jf").await;
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, TrackerId::JELLYFIN);
        assert!(
            h.ctx
                .anime_tracks
                .list_for_entry(23)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn tr_27_034_enhanced_manga_bind_uses_the_manga_kind() {
        let h = Harness::new();
        h.login(TrackerId::KAVITA);
        let failures = bind_enhanced_manga(&h.ctx, 24, "Manga", "src.kv").await;
        assert!(failures.is_empty());
        let rows = h.ctx.manga_tracks.list_for_entry(24).await.unwrap();
        assert_eq!((rows.len(), rows[0].tracker_id), (1, TrackerId::KAVITA));
    }

    #[tokio::test]
    async fn t_33a_006_the_bind_path_passes_the_entry_ref_to_match_and_stores_its_url() {
        let h = Harness::new();
        h.login(TrackerId::JELLYFIN);
        let failures = bind_enhanced_anime_entry(&h.ctx, &entry(30, "Show"), "src.jf").await;
        assert!(failures.is_empty());
        let rows = h.ctx.anime_tracks.list_for_entry(30).await.unwrap();
        assert_eq!(rows[0].remote_url, "/series/30", "match used entry.url");
    }

    #[tokio::test]
    async fn t_33a_006_the_manga_bind_path_passes_the_entry_ref_to_match() {
        let h = Harness::new();
        h.login(TrackerId::KAVITA);
        let failures = bind_enhanced_manga_entry(&h.ctx, &entry(31, "Manga"), "src.kv").await;
        assert!(failures.is_empty());
        let rows = h.ctx.manga_tracks.list_for_entry(31).await.unwrap();
        assert_eq!(rows[0].remote_url, "/series/31");
    }
}
