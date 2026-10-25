//! The app side of the tracking flows (tasks T-38b-031 to T-38b-043 and T-39b-038 to T-39b-047). The flows themselves are
//! `kuro_track`; this file runs them through the tracking context the services built ([`tracker_host::ctx`]), on the app
//! runtime and detached, so a write survives the sheet or the reader closing. Without the services nothing runs and the
//! calls return `None`, which is what a user with no logged-in tracker gets.
//!
//! Callers: the reader calls [`chapter_read`] and the player calls [`episode_seen`] when a chapter or episode completes
//! (the reader's `ReaderExtras::track_update` sink); an entry page calls [`pull_progress`] when it opens; the track sheet
//! and the tracker search call the rest.

use std::sync::Arc;

use kuro_track::ctx::TrackCtx;
use kuro_track::error::TrackError;
use kuro_track::messages::MsgKey;
use kuro_track::model::{AnimeTrack, MangaTrack, TrackerId};
use kuro_track::progress::{TrackOutcome, spawn_track_chapter, spawn_track_episode};
use kuro_track::remove::{spawn_remove_anime, spawn_remove_manga};
use kuro_track::write::{AnimeEdit, MangaEdit, spawn_anime_edit, spawn_manga_edit};
use tokio::task::JoinHandle;

use crate::settings_ui::tracker_host;

/// Runs `launch` with the tracking context and the app runtime entered, so the detached tasks it spawns find the runtime
/// from the GTK thread. `None` when the services are not built.
fn start<T>(launch: impl FnOnce(Arc<TrackCtx>) -> T) -> Option<T> {
    let ctx = tracker_host::ctx()?;
    let handle = crate::shell::host::Shell::get()?.runtime().clone();
    let _entered = handle.enter();
    Some(launch(ctx))
}

/// What each tracker did with a progress update.
pub type Outcomes = JoinHandle<Vec<(TrackerId, TrackOutcome)>>;

/// A chapter was read: its number goes to every logged-in manga tracker behind it. The caller checks the auto-update
/// switch and incognito before it calls this (`progress::item_finished`, `progress::marked`).
pub fn chapter_read(manga_id: i64, chapter: f64) -> Option<Outcomes> {
    start(|ctx| spawn_track_chapter(ctx, manga_id, chapter))
}

/// An episode was seen: its number goes to every logged-in anime tracker behind it, as [`chapter_read`] does for manga.
pub fn episode_seen(anime_id: i64, episode: f64) -> Option<Outcomes> {
    start(|ctx| spawn_track_episode(ctx, anime_id, episode))
}

/// Pulls every tracker's progress for an entry whose page opened (user, 2026-10-25: progress made in another app reaches
/// the tracker, then this one): the episodes or chapters up to each tracker's number are marked, the page follows
/// through its live subscription, and a toast names the tracker and the count. An entry with no binding reads only its
/// own rows.
pub fn pull_progress(kind: kuro_platform::kind::Kind, entry_id: i64) {
    use kuro_platform::kind::Kind;
    let Some(ctx) = tracker_host::ctx() else { return };
    crate::source_bridge::call_on_main(
        async move {
            let refreshed = match kind {
                Kind::Anime => kuro_track::sync::refresh_anime(&ctx, entry_id).await,
                Kind::Manga => kuro_track::sync::refresh_manga(&ctx, entry_id).await,
            };
            let marked = refreshed.map(|r| r.marked).unwrap_or_default();
            marked.into_iter().filter_map(|(id, n)| ctx.trackers.get(id).map(|t| (t.name().to_owned(), n))).collect::<Vec<_>>()
        },
        move |marked| {
            for (tracker, count) in marked {
                let key = if kind.is_anime() { kuro_i18n::MsgKey::track_pulled_episodes } else { kuro_i18n::MsgKey::track_pulled_chapters };
                let args = [("count", kuro_i18n::Arg::Int(count as i64)), ("tracker", kuro_i18n::Arg::Str(&tracker))];
                crate::toast::notify(kuro_i18n::trf(key, &args));
            }
        },
    );
}

/// One remote edit of an anime binding (status, progress, score, dates, private), written detached.
pub fn edit_anime(entry_id: i64, tracker: TrackerId, edit: AnimeEdit) -> Option<JoinHandle<Result<AnimeTrack, TrackError>>> {
    start(|ctx| spawn_anime_edit(ctx, entry_id, tracker, edit))
}

/// One remote edit of a manga binding, written detached.
pub fn edit_manga(entry_id: i64, tracker: TrackerId, edit: MangaEdit) -> Option<JoinHandle<Result<MangaTrack, TrackError>>> {
    start(|ctx| spawn_manga_edit(ctx, entry_id, tracker, edit))
}

/// Removes an anime binding, and the remote entry too when `also_remote` is set. The handle resolves to the message to
/// show when it failed.
pub fn remove_anime(track: AnimeTrack, also_remote: bool) -> Option<JoinHandle<Result<(), MsgKey>>> {
    start(|ctx| spawn_remove_anime(ctx, track, also_remote))
}

/// Removes a manga binding, and the remote entry too when `also_remote` is set.
pub fn remove_manga(track: MangaTrack, also_remote: bool) -> Option<JoinHandle<Result<(), MsgKey>>> {
    start(|ctx| spawn_remove_manga(ctx, track, also_remote))
}
