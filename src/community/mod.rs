//! The refresh of the community scores (`kuro-scores`): one task for the whole app, started once after the window is
//! up. It looks up the library entries whose score is missing or old (`kuro_scores::policy`), a batch at a time, and
//! stores the answers in the one score store (`kuro_data::community_score`). The library and the details page read
//! that store and follow its changes; nothing in the UI asks a site.
//!
//! It runs a little after the start, when an entry joins the library, when a details page opens on an entry with no
//! fresh score ([`lookup_now`], also for a title that is not in the library), and every hour. The provider keeps the
//! pace (`kuro_scores::mal::PACE`), so a large library fills in over a while. A site that asks to slow down, or does
//! not answer, ends the pass: the scores shown stay, and the next pass comes after a pause. Incognito does not stop it:
//! Incognito keeps what the user watches and reads out of the history and off the trackers, and a lookup records
//! neither (it needs no login, and Browse sends titles to sites in Incognito too). It says nothing to the user: it
//! logs.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use kuro_data::community_score::{CommunityScore, CommunityScoreRepo, ScoreCandidate};
use kuro_data::track::TrackRepo;
use kuro_db::handle::Db;
use kuro_platform::kind::Kind;
use kuro_platform::paths::Paths;
use kuro_scores::provider::{Hit, Query, ScoreProvider, Stop, TrackRef};
use kuro_scores::{policy, refresh};
use tokio::sync::Notify;

/// Entries asked between two writes to the store: the library reloads once per batch.
const BATCH: usize = 20;
/// The wait after the start, so the first minutes of the app are not spent on this.
const START_DELAY: Duration = Duration::from_secs(30);
/// How often the library is checked for scores that came due while the app stayed open.
const TICK: Duration = Duration::from_secs(60 * 60);
/// The shortest pause after a site asked to slow down or did not answer.
const BACKOFF: Duration = Duration::from_secs(10 * 60);

struct Job {
    scores: CommunityScoreRepo,
    tracks: TrackRepo,
    provider: Arc<dyn ScoreProvider>,
    wake: Notify,
    /// Entries a details page asked for, looked up before the library's.
    urgent: Mutex<VecDeque<i64>>,
}

static JOB: OnceLock<Arc<Job>> = OnceLock::new();

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// A score as the app writes it, with one decimal ("8.2"): the cover badge and the details page share it.
pub fn score_text(score: f64) -> String {
    format!("{score:.1}")
}

/// Starts the refresh task. Runs once; call from the GTK main thread inside the app's runtime.
pub fn start() {
    if JOB.get().is_some() || !crate::data::is_installed() {
        return;
    }
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return tracing::warn!("community scores: no runtime on this thread; the scores are not refreshed");
    };
    let transport = match kuro_track::tracker::anilist::api::HttpTransport::new() {
        Ok(transport) => Arc::new(transport),
        Err(error) => return tracing::warn!(%error, "community scores: no HTTP client; the scores are not refreshed"),
    };
    // the client id of the MyAnimeList tracker: the scores come from the same API, without a login
    let client_id = kuro_platform::system_paths::SystemPaths::system()
        .map_err(|error| error.to_string())
        .and_then(|paths| kuro_track::mal::store::configured_client_id(&paths.config_dir()).map_err(|error| error.to_string()));
    let client_id = match client_id {
        Ok(client_id) => client_id,
        Err(error) => return tracing::warn!(%error, "community scores: no MyAnimeList client id; the scores are not refreshed"),
    };
    let provider = match kuro_scores::mal::Mal::new(&client_id, transport) {
        Ok(provider) => Arc::new(provider),
        Err(error) => return tracing::warn!(%error, "community scores: no HTTP client; the scores are not refreshed"),
    };
    let job = Arc::new(Job::new(crate::data::app().db.clone(), provider));
    if JOB.set(Arc::clone(&job)).is_ok() {
        runtime.spawn(job.run());
    }
}

/// Starts a pass now when none runs: an entry joined the library.
pub fn wake() {
    if let Some(job) = JOB.get() {
        job.wake.notify_one();
    }
}

/// Looks `entry_id` up ahead of the library when its score is missing or old: its details page opened. Works for a
/// title that is not in the library too.
pub fn lookup_now(entry_id: i64) {
    if let Some(job) = JOB.get() {
        job.urgent.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push_back(entry_id);
        job.wake.notify_one();
    }
}

/// The titles `text` finds on the score site, for the user to pick the right one. `Err` is the text of a toast.
pub async fn search(kind: Kind, text: String) -> Result<Vec<Hit>, String> {
    let Some(job) = JOB.get() else { return Err(kuro_i18n::tr(kuro_i18n::MsgKey::unknown_error)) };
    job.provider.search(kind, &text).await.map_err(|_| kuro_i18n::tr(kuro_i18n::MsgKey::unknown_error))
}

/// The user's own choice for an entry: one of the titles [`search`] found, or `None` for "no score for this title".
pub fn set_match(entry_id: i64, chosen: Option<Hit>) -> Result<(), String> {
    let Some(job) = JOB.get() else { return Ok(()) };
    let chosen = chosen.map(|hit| (hit.remote_id, hit.title, hit.score));
    job.scores.set_user_match(entry_id, job.provider.id(), chosen, now_ms()).map_err(|e| e.to_string())
}

impl Job {
    fn new(db: Arc<Db>, provider: Arc<dyn ScoreProvider>) -> Self {
        Self { scores: CommunityScoreRepo::new(Arc::clone(&db)), tracks: TrackRepo::new(db), provider, wake: Notify::new(), urgent: Mutex::default() }
    }

    async fn run(self: Arc<Self>) {
        tokio::time::sleep(START_DELAY).await;
        // whether the pass before this one stopped too: the first stop of a run of them is a warning, the rest are not
        let mut stopped_before = false;
        loop {
            match self.pass().await {
                // the site said when, or said nothing: no pass before the pause is over, whatever wakes it
                Some(stop) => {
                    let pause = match stop {
                        Stop::RateLimited { retry_after } => retry_after.unwrap_or_default().max(BACKOFF),
                        Stop::Unreachable => BACKOFF,
                    };
                    // the log file keeps warnings only, and it is where "why is there no score" gets answered (a site
                    // that was gone for good once left no trace there)
                    if stopped_before {
                        tracing::debug!(?stop, pause_s = pause.as_secs(), "community scores: the pass stopped again; pausing");
                    } else {
                        tracing::warn!(?stop, pause_s = pause.as_secs(), "community scores: the score site did not answer or asked to slow down; no score arrives until it does, the next try is after the pause");
                    }
                    stopped_before = true;
                    tokio::time::sleep(pause).await;
                }
                None => {
                    stopped_before = false;
                    tokio::select! {
                        () = tokio::time::sleep(TICK) => {}
                        () = self.wake.notified() => {}
                    }
                }
            }
        }
    }

    /// Looks up everything that is due, a batch at a time. Returns why it stopped early, if it did.
    async fn pass(&self) -> Option<Stop> {
        // an entry is asked once per pass, whatever its answer leaves in the store
        let mut asked: HashSet<i64> = HashSet::new();
        loop {
            let batch = match self.next_batch(&asked) {
                Ok(batch) if batch.is_empty() => return None,
                Ok(batch) => batch,
                Err(error) => {
                    tracing::warn!(%error, "community scores: the entries could not be read");
                    return None;
                }
            };
            asked.extend(batch.iter().map(|(candidate, _)| candidate.entry_id));
            let (rows, stopped) = refresh::run_batch(self.provider.as_ref(), &batch, now_ms).await;
            tracing::debug!(asked = batch.len(), answered = rows.len(), "community scores: a batch");
            if let Err(error) = self.scores.store_fetched(&rows) {
                tracing::warn!(%error, "community scores: the answers could not be stored");
                return None;
            }
            if stopped.is_some() {
                return stopped;
            }
        }
    }

    /// The next entries to ask: the ones a details page asked for, then the library's, each only when due.
    fn next_batch(&self, asked: &HashSet<i64>) -> Result<Vec<(ScoreCandidate, Query)>, String> {
        let text = |e: kuro_data::DataError| e.to_string();
        let mut tracks: HashMap<i64, Vec<TrackRef>> = HashMap::new();
        for kind in [Kind::Manga, Kind::Anime] {
            for track in self.tracks.all(kind).map_err(text)? {
                tracks.entry(track.entry_id).or_default().extend(refresh::track_refs(std::slice::from_ref(&track)));
            }
        }
        let urgent: Vec<i64> = self.urgent.lock().unwrap_or_else(std::sync::PoisonError::into_inner).drain(..).collect();
        let mut candidates: Vec<ScoreCandidate> = Vec::new();
        for id in urgent {
            candidates.extend(self.scores.candidate(id).map_err(text)?);
        }
        candidates.extend(self.scores.candidates().map_err(text)?);
        let now = now_ms();
        let mut batch: Vec<(ScoreCandidate, Query)> = Vec::new();
        for candidate in candidates {
            if batch.len() == BATCH {
                break;
            }
            let id = candidate.entry_id;
            let of_entry = tracks.get(&id).cloned().unwrap_or_default();
            if asked.contains(&id) || batch.iter().any(|(c, _)| c.entry_id == id) || !policy::due(&candidate, &of_entry, now) {
                continue;
            }
            let query = refresh::query_of(&candidate, of_entry);
            batch.push((candidate, query));
        }
        Ok(batch)
    }
}

/// The stored score of an entry, for a page that shows it. `None` before the app data exists.
pub fn stored(entry_id: i64) -> Option<CommunityScore> {
    if !crate::data::is_installed() {
        return None;
    }
    CommunityScoreRepo::new(crate::data::app().db.clone()).by_entry(entry_id).ok().flatten()
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use kuro_data::community_score::MatchedBy;
    use kuro_data::entry::{Entry, EntryRepo};
    use kuro_scores::provider::Outcome;

    use super::*;

    #[test]
    fn a_score_is_written_with_one_decimal() {
        assert_eq!(score_text(8.24), "8.2");
        assert_eq!(score_text(7.96), "8.0");
        assert_eq!(score_text(9.0), "9.0");
    }

    /// Scores every anime 8.5 and stops when told to; counts what it was asked.
    struct Fake {
        asked: Mutex<Vec<i64>>,
        stop_after: usize,
    }

    #[async_trait]
    impl ScoreProvider for Fake {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn supports(&self, kind: Kind) -> bool {
            kind == Kind::Anime
        }
        async fn lookup(&self, query: &Query) -> Result<Outcome, Stop> {
            let mut asked = self.asked.lock().expect("asked");
            if asked.len() == self.stop_after {
                return Err(Stop::Unreachable);
            }
            asked.push(query.entry_id);
            Ok(Outcome::Found { hit: Hit { remote_id: query.entry_id * 10, title: query.title.clone(), alt_titles: Vec::new(), score: Some(8.5) }, by: MatchedBy::Title })
        }
        async fn search(&self, _kind: Kind, _text: &str) -> Result<Vec<Hit>, Stop> {
            Ok(Vec::new())
        }
    }

    fn entry(db: &Arc<Db>, kind: Kind, url: &str, favorite: bool) -> i64 {
        EntryRepo::new(Arc::clone(db)).insert(&Entry { favorite, source: 7, url: url.into(), title: format!("Title {url}"), ..Entry::create(kind) }).expect("insert")
    }

    fn job(db: &Arc<Db>, stop_after: usize) -> (Job, Arc<Fake>) {
        let fake = Arc::new(Fake { asked: Mutex::default(), stop_after });
        (Job::new(Arc::clone(db), fake.clone()), fake)
    }

    #[tokio::test]
    async fn a_pass_scores_every_library_entry_once_and_the_next_pass_asks_nothing() {
        let dir = kuro_testkit::TempDir::new().unwrap();
        let db = kuro_testkit::TestDb::handle(&dir);
        let ids: Vec<i64> = (0..BATCH + 5).map(|i| entry(&db, Kind::Anime, &format!("/a{i}"), true)).collect();
        // a manga: the fake does not score manga, and that must not keep the pass going round
        let manga = entry(&db, Kind::Manga, "/m", true);
        let browsed = entry(&db, Kind::Anime, "/browsed", false);
        let (job, fake) = job(&db, usize::MAX);
        assert_eq!(job.pass().await, None);
        let mut asked = fake.asked.lock().expect("asked").clone();
        asked.sort_unstable();
        assert_eq!(asked, ids, "each library anime once, over two batches; not the manga, not the browsed title");
        let scores = job.scores.scores(Kind::Anime).expect("scores");
        assert_eq!(scores.len(), ids.len());
        assert_eq!(job.scores.by_entry(manga).expect("read"), None);
        assert_eq!(job.pass().await, None);
        assert_eq!(fake.asked.lock().expect("asked").len(), ids.len(), "fresh scores are not asked again");
        // a details page asks for a title outside the library
        job.urgent.lock().expect("urgent").push_back(browsed);
        assert_eq!(job.pass().await, None);
        assert_eq!(job.scores.by_entry(browsed).expect("read").and_then(|s| s.score), Some(8.5));
    }

    #[tokio::test]
    async fn a_stop_ends_the_pass_and_keeps_what_was_answered() {
        let dir = kuro_testkit::TempDir::new().unwrap();
        let db = kuro_testkit::TestDb::handle(&dir);
        for i in 0..5 {
            entry(&db, Kind::Anime, &format!("/a{i}"), true);
        }
        let (job, fake) = job(&db, 2);
        assert_eq!(job.pass().await, Some(Stop::Unreachable));
        assert_eq!(fake.asked.lock().expect("asked").len(), 2);
        assert_eq!(job.scores.scores(Kind::Anime).expect("scores").len(), 2, "the two answers are stored");
    }
}
