//! Watch progress for one episode (area 45 `progress.rs`): the position restored on open, and the saves while the
//! episode plays. The decisions are `kuro_player::progress`; this file reads and writes the anime database.

use std::sync::Arc;

use kuro_data::history::HistoryRepo;
use kuro_data::item::{Item, ItemRepo, ItemUpdate};
use kuro_db::handle::Db;
use kuro_platform::kind::Kind;
use kuro_player::progress::{self, EpisodeChanges, EpisodeProgress, SaveContext, WriteThrottle, HISTORY_INTERVAL_MS};

/// Watched past this fraction of the duration counts as seen. Kotlin's default.
pub const COMPLETION_THRESHOLD: f64 = 0.85;

/// Where playback starts, in milliseconds. An explicit position above zero wins. Otherwise the saved position is
/// used, and a seen episode restarts at zero (`preserve watching position` is off by default).
pub fn start_ms(explicit_ms: i64, episode: &Item) -> i64 {
    progress::resume_position_ms(
        (explicit_ms > 0).then_some(explicit_ms),
        episode.done,
        false,
        episode.progress * 1000,
    )
}

/// The progress fields of an episode row.
fn progress_of(episode: &Item) -> EpisodeProgress {
    EpisodeProgress {
        seen: episode.done,
        bookmark: episode.bookmark,
        fillermark: episode.fillermark,
        last_second_seen: episode.progress,
        total_seconds: episode.total_seconds(),
    }
}

/// Keeps the in-memory row in step with the write that just happened.
fn apply(stored: &mut Item, changes: &EpisodeChanges) {
    if let Some(seen) = changes.seen {
        stored.done = seen;
    }
    if let Some(bookmark) = changes.bookmark {
        stored.bookmark = bookmark;
    }
    if let Some(fillermark) = changes.fillermark {
        stored.fillermark = fillermark;
    }
    if let Some(position) = changes.last_second_seen {
        stored.progress = position;
    }
    if let Some(total) = changes.total_seconds {
        stored.total_seconds = Some(total);
    }
}

/// Progress of one open episode. Holds the row as last read or written, so a save only writes what changed.
pub struct Tracker {
    episodes: ItemRepo,
    history: HistoryRepo,
    episode_id: i64,
    stored: Item,
    /// Read at every save, so the switch takes effect in the middle of an episode.
    incognito: crate::incognito::Check,
    throttle: WriteThrottle,
    /// When the watched time was last added to the history, while the episode plays. A pause, a stop or a switch of
    /// episode (a forced save) clears it, so the time spent paused is never counted.
    played_since: Option<i64>,
    /// Called once with the anime id and the episode number when an episode crosses the threshold.
    on_seen: Box<dyn Fn(i64, f64)>,
}

impl Tracker {
    #[cfg(test)]
    /// Reads the episode row. `None` when no row has `episode_id` or the read fails, and then nothing is saved.
    pub fn open(db: Arc<Db>, episode_id: i64, incognito: bool) -> Option<Self> {
        Self::open_following(db, episode_id, std::rc::Rc::new(move || incognito))
    }

    /// [`open`](Self::open) for the app: the incognito switch is read at every save.
    pub fn open_live(db: Arc<Db>, episode_id: i64) -> Option<Self> {
        Self::open_following(db, episode_id, crate::incognito::live())
    }

    fn open_following(db: Arc<Db>, episode_id: i64, incognito: crate::incognito::Check) -> Option<Self> {
        let episodes = ItemRepo::new(Arc::clone(&db));
        let stored = match episodes.by_id(episode_id) {
            Ok(Some(row)) => row,
            Ok(None) => {
                tracing::warn!(episode_id, "episode is not in the database; progress is not saved");
                return None;
            }
            Err(e) => {
                tracing::error!(episode_id, error = %e, "could not read the episode; progress is not saved");
                return None;
            }
        };
        Some(Self {
            episodes,
            history: HistoryRepo::new(db),
            episode_id,
            stored,
            incognito: std::rc::Rc::clone(&incognito),
            throttle: WriteThrottle::new(HISTORY_INTERVAL_MS),
            played_since: None,
            on_seen: Box::new(move |anime_id, number| {
                crate::entry_core::tracking::progress::item_finished(Kind::Anime, anime_id, number, incognito());
            }),
        })
    }

    pub fn start_ms(&self, explicit_ms: i64) -> i64 {
        start_ms(explicit_ms, &self.stored)
    }

    /// Records the position. A write happens when one is due (every five seconds), or at once when `forced` (on
    /// pause and on close). Nothing is written before the duration is known. A failed write is logged.
    pub fn save(&mut self, now_ms: i64, position_ms: i64, duration_ms: i64, forced: bool) {
        if duration_ms <= 0 {
            return;
        }
        let due = forced || {
            let mut probe = self.throttle;
            probe.due(now_ms, false)
        };
        if !due {
            return;
        }
        let stored = progress_of(&self.stored);
        let episode = EpisodeProgress {
            last_second_seen: position_ms / 1000,
            total_seconds: duration_ms / 1000,
            ..stored.clone()
        };
        let plan = progress::plan_save(
            SaveContext {
                incognito_source: (self.incognito)(),
                has_trackers: false,
                now_ms,
                forced,
                threshold: COMPLETION_THRESHOLD,
            },
            &mut self.throttle,
            &stored,
            &episode,
        );
        if let Some(changes) = plan.episode {
            let update = ItemUpdate {
                done: changes.seen,
                progress: changes.last_second_seen,
                total_seconds: changes.total_seconds,
                ..ItemUpdate::new(self.episode_id)
            };
            match self.episodes.update(&update) {
                Ok(()) => {
                    apply(&mut self.stored, &changes);
                    if changes.seen == Some(true) {
                        (self.on_seen)(self.stored.entry_id, self.stored.number);
                        super::after_seen::run(self.episode_id);
                    }
                }
                Err(e) => tracing::error!(episode_id = self.episode_id, error = %e, "progress write failed"),
            }
        }
        if let Some(seen_at) = plan.history_seen_at {
            // the time watched since the last write goes onto the history row (at most two intervals at once)
            let watched = self.played_since.map_or(0, |since| (seen_at - since).clamp(0, 2 * HISTORY_INTERVAL_MS));
            if let Err(e) = self.history.upsert(self.episode_id, seen_at, watched) {
                tracing::error!(episode_id = self.episode_id, error = %e, "history write failed");
            }
            self.played_since = (!forced).then_some(seen_at);
        }
    }

    #[cfg(test)]
    /// Whether the stored row is seen, after the saves so far.
    pub fn seen(&self) -> bool {
        self.stored.done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_data::entry::{Entry, EntryRepo};
    use kuro_testkit::{TempDir, TestDb};

    /// One anime with one episode in a fresh anime database. Returns the database and the episode id.
    fn seeded(dir: &TempDir) -> (Arc<Db>, i64) {
        let db = TestDb::handle(dir);
        let anime_id = EntryRepo::new(Arc::clone(&db)).insert(&Entry::create(Kind::Anime)).expect("anime row");
        let episode = Item {
            entry_id: anime_id,
            url: "/episode-1".to_owned(),
            name: "Episode 1".to_owned(),
            ..Item::create(Kind::Anime)
        };
        let id = ItemRepo::new(Arc::clone(&db)).add_all(&[episode]).expect("episode row")[0].id;
        (db, id)
    }

    fn row(db: &Arc<Db>, id: i64) -> Item {
        ItemRepo::new(Arc::clone(db)).by_id(id).expect("read").expect("row")
    }

    fn fresh(seen: bool, last_second_seen: i64) -> Item {
        Item { done: seen, progress: last_second_seen, total_seconds: Some(0), ..Item::create(Kind::Anime) }
    }

    #[test]
    fn an_explicit_position_wins_and_an_unseen_episode_resumes_its_saved_one() {
        assert_eq!(start_ms(7_000, &fresh(false, 300)), 7_000);
        assert_eq!(start_ms(0, &fresh(false, 300)), 300_000);
        assert_eq!(start_ms(-1, &fresh(false, 42)), 42_000);
    }

    #[test]
    fn a_seen_episode_restarts_at_zero_unless_an_explicit_position_is_given() {
        assert_eq!(start_ms(0, &fresh(true, 1_300)), 0);
        assert_eq!(start_ms(5_000, &fresh(true, 1_300)), 5_000);
    }

    #[test]
    fn saves_wait_for_the_interval_then_restore_on_reopen() {
        let dir = TempDir::new().unwrap();
        let (db, id) = seeded(&dir);
        let mut tracker = Tracker::open(Arc::clone(&db), id, false).expect("tracker");
        tracker.save(0, 2_000, 600_000, false);
        assert_eq!(row(&db, id).progress, 2);
        tracker.save(1_000, 3_000, 600_000, false);
        assert_eq!(row(&db, id).progress, 2, "a save inside the interval is skipped");
        tracker.save(1_000, 3_500, 600_000, true);
        assert_eq!(row(&db, id).progress, 3, "a forced save is written at once");
        assert_eq!(row(&db, id).total_seconds(), 600);

        let reopened = Tracker::open(db, id, false).expect("reopen");
        assert_eq!(reopened.start_ms(0), 3_000);
    }

    #[test]
    fn watching_past_the_threshold_marks_the_episode_seen() {
        let dir = TempDir::new().unwrap();
        let (db, id) = seeded(&dir);
        let mut tracker = Tracker::open(Arc::clone(&db), id, false).expect("tracker");
        tracker.save(0, 50_000, 100_000, true);
        assert!(!row(&db, id).done, "50 of 100 seconds is below 85 percent");
        tracker.save(10_000, 90_000, 100_000, true);
        assert!(row(&db, id).done);
        assert!(tracker.seen());
        assert!(Tracker::open(db, id, false).unwrap().seen());
    }

    #[test]
    fn crossing_the_threshold_reports_the_episode_to_the_trackers_once() {
        let dir = TempDir::new().unwrap();
        let (db, id) = seeded(&dir);
        let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut tracker = Tracker::open(Arc::clone(&db), id, false).expect("tracker");
        let sink = std::rc::Rc::clone(&calls);
        tracker.on_seen = Box::new(move |anime, number| sink.borrow_mut().push((anime, number)));
        tracker.save(0, 50_000, 100_000, true);
        assert!(calls.borrow().is_empty(), "below the threshold");
        tracker.save(10_000, 90_000, 100_000, true);
        tracker.save(20_000, 95_000, 100_000, true);
        let anime_id = row(&db, id).entry_id;
        assert_eq!(*calls.borrow(), vec![(anime_id, row(&db, id).number)]);
    }

    #[test]
    fn a_save_writes_history_unless_incognito_and_then_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let (db, id) = seeded(&dir);
        let mut private = Tracker::open(Arc::clone(&db), id, true).expect("tracker");
        private.save(0, 2_000, 4_000, true);
        assert_eq!(row(&db, id).progress, 0, "incognito keeps no progress for an untracked source");
        assert!(HistoryRepo::new(Arc::clone(&db)).by_entry_id(row(&db, id).entry_id).unwrap().is_empty());

        let mut open = Tracker::open(Arc::clone(&db), id, false).expect("tracker");
        open.save(0, 2_000, 4_000, true);
        let history = HistoryRepo::new(Arc::clone(&db)).by_entry_id(row(&db, id).entry_id).unwrap();
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn the_watched_time_is_added_to_the_history_while_playing_and_never_for_a_pause() {
        let dir = TempDir::new().unwrap();
        let (db, id) = seeded(&dir);
        let anime_id = row(&db, id).entry_id;
        let watched = || HistoryRepo::new(Arc::clone(&db)).by_entry_id(anime_id).unwrap().first().map_or(0, |h| h.duration_ms);
        let mut tracker = Tracker::open(Arc::clone(&db), id, false).expect("tracker");
        tracker.save(1_000, 2_000, 100_000, false);
        assert_eq!(watched(), 0, "the first write starts the clock");
        tracker.save(6_000, 7_000, 100_000, false);
        tracker.save(11_000, 12_000, 100_000, false);
        assert_eq!(watched(), 10_000, "two intervals of playing");
        tracker.save(14_000, 15_000, 100_000, true);
        assert_eq!(watched(), 13_000, "the pause adds the time up to it");
        // a long pause, then playing again: the pause is not counted
        tracker.save(500_000, 15_000, 100_000, false);
        tracker.save(505_000, 20_000, 100_000, false);
        assert_eq!(watched(), 18_000);
    }

    #[test]
    fn an_unknown_episode_opens_no_tracker() {
        let dir = TempDir::new().unwrap();
        let (db, _) = seeded(&dir);
        assert!(Tracker::open(db, 9_999, false).is_none());
    }

    #[test]
    fn nothing_is_written_before_the_duration_is_known() {
        let dir = TempDir::new().unwrap();
        let (db, id) = seeded(&dir);
        let mut tracker = Tracker::open(Arc::clone(&db), id, false).expect("tracker");
        tracker.save(0, 2_000, 0, true);
        assert_eq!(row(&db, id).progress, 0);
    }
}
