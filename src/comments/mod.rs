//! Comments and reviews that AniList already has (docs/design/comments.md): the comments of the playing episode in a
//! panel of the player ([`panel`]), and a title's reviews on a page of their own ([`reviews`]). Read-only, without a
//! login, nothing stored: what was found lives in memory for the app's run.
//!
//! What is remembered: the AniList id of an entry, and the thread of an episode, once found or found missing, so a
//! second look costs one request. A rate limit stops every ask until AniList's wait (or a minute) is over.

pub mod panel;
pub mod reviews;
pub mod text;
mod view;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use kuro_data::track::TrackRepo;
use kuro_i18n::{MsgKey, tr};
use kuro_platform::kind::Kind;
use kuro_track::model::TrackerId;
use kuro_track::tracker::anilist::api::Transport;
use kuro_track::tracker::anilist::comments::{self as api, Thread};
use kuro_track::tracker::anilist::error::AniListError;

/// How long the asks stop after a rate limit that says no wait of its own.
const RATE_LIMIT_WAIT: Duration = Duration::from_secs(60);

/// What is known of an entry to find it on AniList.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ids {
    /// The entry's AniList binding.
    pub anilist: Option<i64>,
    /// Its MyAnimeList binding, else the community score's match.
    pub mal: Option<i64>,
}

thread_local! {
    /// Entry id to AniList id; `None` for a title AniList does not know.
    static MEDIA: RefCell<HashMap<i64, Option<i64>>> = RefCell::default();
    /// (AniList id, episode) to its thread; `None` for an episode without one.
    static THREADS: RefCell<HashMap<(i64, i64), Option<Thread>>> = RefCell::default();
    /// No ask before this moment (a rate limit).
    static BLOCKED_UNTIL: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// The ids of `entry_id` from its tracker bindings and the community score store. The bindings are exact; the score's
/// match is the app's own, made the same way for every title.
pub fn ids_of(entry_id: i64) -> Ids {
    if !crate::data::is_installed() {
        return Ids::default();
    }
    let tracks = TrackRepo::new(crate::data::app().db.clone()).by_entry(entry_id).unwrap_or_default();
    let bound = |tracker: i64| tracks.iter().find(|t| t.tracker_id == tracker && t.remote_id > 0).map(|t| t.remote_id);
    Ids {
        anilist: bound(TrackerId::ANILIST.0),
        mal: bound(TrackerId::MYANIMELIST.0).or_else(|| crate::community::stored(entry_id).and_then(|score| score.remote_id)),
    }
}

/// The AniList id remembered for `entry_id`: `Some(None)` when AniList is known not to have it.
pub fn remembered_media(entry_id: i64) -> Option<Option<i64>> {
    MEDIA.with(|m| m.borrow().get(&entry_id).copied())
}

pub fn remember_media(entry_id: i64, media: Option<i64>) {
    MEDIA.with(|m| m.borrow_mut().insert(entry_id, media));
}

pub fn remembered_thread(media: i64, episode: i64) -> Option<Option<Thread>> {
    THREADS.with(|t| t.borrow().get(&(media, episode)).cloned())
}

pub fn remember_thread(media: i64, episode: i64, thread: Option<Thread>) {
    THREADS.with(|t| t.borrow_mut().insert((media, episode), thread));
}

/// The AniList id from the binding, else asked by the MyAnimeList id; `None` when there is neither, or AniList does not
/// know the title. No search by name: a wrong match would show another title's comments.
pub async fn resolve_media(transport: &dyn Transport, ids: Ids, kind: Kind) -> Result<Option<i64>, AniListError> {
    if let Some(id) = ids.anilist {
        return Ok(Some(id));
    }
    match ids.mal {
        Some(mal) => api::media_id(transport, mal, kind).await,
        None => Ok(None),
    }
}

/// Whether a rate limit still stops the asks.
pub fn blocked() -> bool {
    BLOCKED_UNTIL.with(|b| b.get().is_some_and(|until| Instant::now() < until))
}

/// Notes what an answer says about the rate limit.
pub fn note_error(error: &AniListError) {
    if let AniListError::RateLimited { retry_after } = error {
        let wait = retry_after.unwrap_or(RATE_LIMIT_WAIT).max(Duration::from_secs(1));
        BLOCKED_UNTIL.with(|b| b.set(Some(Instant::now() + wait)));
    }
}

/// The text of a failed ask: AniList's rate limit says so; anything else is `failed`.
pub fn error_text(error: Option<&AniListError>, failed: MsgKey) -> String {
    match error {
        Some(AniListError::RateLimited { .. }) => tr(MsgKey::comments_rate_limited),
        _ => tr(failed),
    }
}

/// The app's tokio runtime, which the asks run on.
fn runtime() -> Option<tokio::runtime::Handle> {
    tokio::runtime::Handle::try_current().ok()
}

/// A transport for one ask, without a token.
fn transport() -> Result<kuro_track::tracker::anilist::api::HttpTransport, AniListError> {
    kuro_track::tracker::anilist::api::HttpTransport::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use kuro_track::tracker::anilist::api::RawResponse;
    use serde_json::Value;

    /// Answers every request with `body` and counts them.
    struct Answer {
        body: &'static str,
        calls: Mutex<usize>,
    }

    #[async_trait]
    impl Transport for Answer {
        async fn post(&self, _body: &Value, _bearer: Option<&str>) -> Result<RawResponse, AniListError> {
            *self.calls.lock().unwrap() += 1;
            Ok(RawResponse { status: 200, body: self.body.as_bytes().to_vec().into(), retry_after: None })
        }
    }

    #[tokio::test]
    async fn the_anilist_binding_wins_then_the_mal_id_is_asked_and_nothing_else() {
        let answer = Answer { body: r#"{"data":{"Media":{"id":154587}}}"#, calls: Mutex::new(0) };
        assert_eq!(resolve_media(&answer, Ids { anilist: Some(7), mal: Some(52991) }, Kind::Anime).await.unwrap(), Some(7));
        assert_eq!(*answer.calls.lock().unwrap(), 0, "a binding needs no ask");
        assert_eq!(resolve_media(&answer, Ids { anilist: None, mal: Some(52991) }, Kind::Anime).await.unwrap(), Some(154587));
        assert_eq!(resolve_media(&answer, Ids::default(), Kind::Anime).await.unwrap(), None, "no search by name");
        assert_eq!(*answer.calls.lock().unwrap(), 1);
    }

    #[test]
    fn a_rate_limit_stops_the_asks_until_its_wait_is_over() {
        BLOCKED_UNTIL.with(|b| b.set(None));
        assert!(!blocked());
        note_error(&AniListError::Http(500));
        assert!(!blocked(), "only a rate limit stops the asks");
        note_error(&AniListError::RateLimited { retry_after: Some(Duration::from_secs(30)) });
        assert!(blocked());
        BLOCKED_UNTIL.with(|b| b.set(Some(Instant::now() - Duration::from_secs(1))));
        assert!(!blocked(), "the wait is over");
    }

    #[test]
    fn what_was_found_is_remembered() {
        assert_eq!(remembered_media(-42), None);
        remember_media(-42, None);
        assert_eq!(remembered_media(-42), Some(None), "known missing");
        remember_thread(-1, 3, None);
        assert_eq!(remembered_thread(-1, 3), Some(None));
        assert_eq!(remembered_thread(-1, 4), None);
    }
}
