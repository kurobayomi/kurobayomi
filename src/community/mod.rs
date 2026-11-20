//! The refresh of the community scores (`kuro-scores`): one task for the whole app, started once after the window is
//! up. It looks up the library entries whose score is missing or old (`kuro_scores::policy`), a batch at a time, and
//! stores the answers in the one score store (`kuro_data::community_score`). The library and the details page read
//! that store and follow its changes; nothing in the UI asks a site.
//!
//! It runs a few seconds after the start, when an entry joins the library, when a details page opens on an entry with
//! no fresh score ([`lookup_now`], also for a title that is not in the library; it goes before the library's and does
//! not wait for the start), and every hour. Each answer is stored as it arrives, so the stars appear one by one. The provider keeps the
//! pace (`kuro_scores::mal::PACE`), so a large library fills in over a while. A site that asks to slow down, or does
//! not answer, ends the pass: the scores shown stay, and the next pass comes after a pause. Incognito does not stop it:
//! Incognito keeps what the user watches and reads out of the history and off the trackers, and a lookup records
//! neither (it needs no login, and Browse sends titles to sites in Incognito too). It says nothing to the user: it
//! logs.
//!
//! Beside the lookups it keeps the score site's bulk lists ([`lists`]): a few requests a day for the titles on air and
//! the most popular ones, with their scores. A title Browse shows ([`browsed`]) is found in them by name and gets its
//! score at once, without a request of its own.

mod lists;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use kuro_data::community_score::{CommunityScore, CommunityScoreRepo, MatchedBy, ScoreCandidate};
use kuro_data::track::TrackRepo;
use kuro_db::handle::Db;
use kuro_platform::kind::Kind;
use kuro_platform::paths::Paths;
use kuro_scores::provider::{Hit, Query, ScoreProvider, Stop, TrackRef};
use kuro_scores::{policy, refresh};
use tokio::sync::Notify;

/// Entries read from the store at a time. Each one's answer is stored as it arrives.
const BATCH: usize = 20;
/// The wait after the start, so the window is up and drawn first. It was 30 seconds, and a first score took 42 on a
/// library of four titles (user, 2026-11-20). A title's page that opens, or a title that joins the library, ends it.
const START_DELAY: Duration = Duration::from_secs(5);
/// How often the library is checked for scores that came due while the app stayed open.
const TICK: Duration = Duration::from_secs(60 * 60);
/// The shortest pause after a site asked to slow down or did not answer.
const BACKOFF: Duration = Duration::from_secs(10 * 60);
/// A Browse page stores its entries one by one: they are matched together after this long, with one write.
const BROWSED_GATHER: Duration = Duration::from_millis(200);
/// The browsed titles kept while the lists of their kind are not there yet (the first start, before they are fetched).
const BROWSED_KEPT: usize = 2000;

/// A title Browse showed: its entry row and its name in the source.
struct Browsed {
    kind: Kind,
    entry_id: i64,
    title: String,
}

struct Job {
    scores: CommunityScoreRepo,
    tracks: TrackRepo,
    provider: Arc<dyn ScoreProvider>,
    wake: Notify,
    /// Entries a details page asked for, looked up before the library's.
    urgent: Mutex<VecDeque<i64>>,
    lists: lists::Lists,
    /// The titles Browse showed that are not matched against the lists yet.
    browsed: Mutex<Vec<Browsed>>,
    browsed_wake: Notify,
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
    let paths = match kuro_platform::system_paths::SystemPaths::system() {
        Ok(paths) => paths,
        Err(error) => return tracing::warn!(%error, "community scores: the app folders do not resolve; the scores are not refreshed"),
    };
    // the client id of the MyAnimeList tracker: the scores come from the same API, without a login
    let client_id = match kuro_track::mal::store::configured_client_id(&paths.config_dir()) {
        Ok(client_id) => client_id,
        Err(error) => return tracing::warn!(%error, "community scores: no MyAnimeList client id; the scores are not refreshed"),
    };
    let provider = match kuro_scores::mal::Mal::new(&client_id, transport) {
        Ok(provider) => Arc::new(provider),
        Err(error) => return tracing::warn!(%error, "community scores: no HTTP client; the scores are not refreshed"),
    };
    let files = [paths.score_lists_file(Kind::Manga), paths.score_lists_file(Kind::Anime)];
    let job = Arc::new(Job::new(crate::data::app().db.clone(), provider, lists::Lists::new(Some(files))));
    if JOB.set(Arc::clone(&job)).is_ok() {
        runtime.spawn(Arc::clone(&job).keep_lists());
        runtime.spawn(Arc::clone(&job).match_as_browsed());
        runtime.spawn(job.run());
    }
}

/// A title Browse showed, with its entry row and its name in the source: it gets its score from the bulk lists when
/// they hold a title of that name, without a request. Cheap, and safe from any thread.
pub fn browsed(kind: Kind, entry_id: i64, title: &str) {
    let Some(job) = JOB.get() else { return };
    if entry_id <= 0 {
        return;
    }
    {
        let mut queue = job.browsed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if queue.len() >= BROWSED_KEPT {
            queue.remove(0);
        }
        queue.push(Browsed { kind, entry_id, title: title.to_owned() });
    }
    job.browsed_wake.notify_one();
}

/// The scores of a kind by entry row, published again after each write to the store: the Browse covers follow it.
/// `None` before the app data exists. Needs a tokio runtime: the task that republishes the subscription runs on it.
pub fn follow_scores(kind: Kind) -> Option<tokio::sync::watch::Receiver<HashMap<i64, f64>>> {
    if !crate::data::is_installed() {
        return None;
    }
    let mut subscription = CommunityScoreRepo::new(crate::data::app().db.clone()).watch_scores(kind);
    let initial: HashMap<i64, f64> = subscription.current().and_then(|published| published.ok()).map(|rows| rows.into_iter().collect()).unwrap_or_default();
    let (tx, rx) = tokio::sync::watch::channel(initial);
    tokio::spawn(async move {
        while let Some(published) = subscription.changed().await {
            let Ok(rows) = published else { continue };
            if tx.send(rows.into_iter().collect()).is_err() {
                break;
            }
        }
    });
    Some(rx)
}

/// The year and the month of today (UTC), for a provider that lists by season.
fn today() -> (i32, u32) {
    use chrono::Datelike;
    let now = chrono::Utc::now();
    (now.year(), now.month())
}

/// How long nothing is asked after a stop: as long as the site said, and never less than [`BACKOFF`].
fn pause_after(stop: Stop) -> Duration {
    match stop {
        Stop::RateLimited { retry_after } => retry_after.unwrap_or_default().max(BACKOFF),
        Stop::Unreachable => BACKOFF,
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
    fn new(db: Arc<Db>, provider: Arc<dyn ScoreProvider>, lists: lists::Lists) -> Self {
        Self {
            scores: CommunityScoreRepo::new(Arc::clone(&db)),
            tracks: TrackRepo::new(db),
            provider,
            wake: Notify::new(),
            urgent: Mutex::default(),
            lists,
            browsed: Mutex::default(),
            browsed_wake: Notify::new(),
        }
    }

    /// Keeps the bulk lists: the cached ones at once, fresh ones when there are none or they are a day old. It runs
    /// beside the lookups and shares their pace, so neither waits for the other to end.
    async fn keep_lists(self: Arc<Self>) {
        self.lists.load_files();
        // what Browse showed before the files were read
        self.browsed_wake.notify_one();
        tokio::time::sleep(START_DELAY).await;
        loop {
            let mut stopped = None;
            for kind in [Kind::Anime, Kind::Manga] {
                if !self.provider.supports(kind) || !self.lists.due(kind, now_ms()) {
                    continue;
                }
                match self.provider.catalogue(kind, today()).await {
                    // a provider with no lists, or a site that answered with none: asked again at the next round
                    Ok(hits) if hits.is_empty() => {}
                    Ok(hits) => {
                        tracing::debug!(?kind, titles = hits.len(), "community scores: the lists were fetched");
                        self.lists.set(kind, hits, now_ms());
                        self.browsed_wake.notify_one();
                    }
                    Err(stop) => {
                        stopped = Some(stop);
                        break;
                    }
                }
            }
            // the lookups warn when the site stops answering; the lists, which fail with them, only note it
            let pause = match stopped {
                Some(stop) => {
                    tracing::debug!(?stop, "community scores: the lists were not fetched; trying again after the pause");
                    pause_after(stop)
                }
                None => TICK,
            };
            tokio::time::sleep(pause).await;
        }
    }

    /// Matches the titles Browse showed against the lists, a page at a time.
    async fn match_as_browsed(self: Arc<Self>) {
        loop {
            self.browsed_wake.notified().await;
            tokio::time::sleep(BROWSED_GATHER).await;
            let entries = std::mem::take(&mut *self.browsed.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
            let waiting = self.match_browsed(entries);
            if !waiting.is_empty() {
                // their kind has no lists yet: they are matched when the lists arrive
                let mut queue = self.browsed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let room = BROWSED_KEPT.saturating_sub(queue.len());
                queue.extend(waiting.into_iter().take(room));
            }
        }
    }

    /// Stores the score of each browsed entry the lists hold a title for, in one write. Returns the entries whose kind
    /// has no lists yet. An entry the lists do not hold is left alone: its own page looks it up.
    fn match_browsed(&self, entries: Vec<Browsed>) -> Vec<Browsed> {
        let mut waiting: Vec<Browsed> = Vec::new();
        let mut rows: Vec<CommunityScore> = Vec::new();
        let mut seen: HashSet<i64> = HashSet::new();
        for browsed in entries {
            let Some(held) = self.lists.get(browsed.kind) else {
                waiting.push(browsed);
                continue;
            };
            if !seen.insert(browsed.entry_id) {
                continue;
            }
            let Some(hit) = held.catalogue.find(&browsed.title) else { continue };
            let stored = match self.scores.by_entry(browsed.entry_id) {
                Ok(stored) => stored,
                Err(error) => {
                    tracing::warn!(%error, "community scores: a browsed entry could not be read");
                    continue;
                }
            };
            if lists::takes(stored.as_ref(), hit.remote_id, held.fetched_at) {
                rows.push(CommunityScore {
                    entry_id: browsed.entry_id,
                    provider: self.provider.id().to_owned(),
                    remote_id: Some(hit.remote_id),
                    matched_title: hit.title.clone(),
                    matched_by: MatchedBy::Title,
                    score: hit.score,
                    fetched_at: held.fetched_at,
                });
            }
        }
        if !rows.is_empty() {
            if let Err(error) = self.scores.store_fetched(&rows) {
                tracing::warn!(%error, "community scores: the scores of browsed titles could not be stored");
            }
        }
        waiting
    }

    async fn run(self: Arc<Self>) {
        tokio::select! {
            () = tokio::time::sleep(START_DELAY) => {}
            () = self.wake.notified() => {}
        }
        // whether the pass before this one stopped too: the first stop of a run of them is a warning, the rest are not
        let mut stopped_before = false;
        loop {
            match self.pass().await {
                // the site said when, or said nothing: no pass before the pause is over, whatever wakes it
                Some(stop) => {
                    let pause = pause_after(stop);
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
            for (index, pair) in batch.iter().enumerate() {
                // a title's page asked meanwhile: it goes first, so the rest of this batch waits for the next one
                if index > 0 && !self.urgent.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_empty() {
                    break;
                }
                asked.insert(pair.0.entry_id);
                let (rows, stopped) = refresh::run_batch(self.provider.as_ref(), std::slice::from_ref(pair), now_ms).await;
                // a kind the provider does not score answers with no row: nothing to write, nobody to wake
                if !rows.is_empty() {
                    if let Err(error) = self.scores.store_fetched(&rows) {
                        tracing::warn!(%error, "community scores: the answer could not be stored");
                        return None;
                    }
                }
                if stopped.is_some() {
                    return stopped;
                }
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
        (Job::new(Arc::clone(db), fake.clone(), lists::Lists::new(None)), fake)
    }

    #[tokio::test]
    async fn a_browsed_title_gets_its_score_from_the_lists_without_a_request() {
        let dir = kuro_testkit::TempDir::new().unwrap();
        let db = kuro_testkit::TestDb::handle(&dir);
        let listed = entry(&db, Kind::Anime, "/listed", false);
        let unlisted = entry(&db, Kind::Anime, "/unlisted", false);
        let manga = entry(&db, Kind::Manga, "/m", false);
        let (job, fake) = job(&db, usize::MAX);
        let hit = |id: i64, title: &str, score: f64| Hit { remote_id: id, title: title.into(), alt_titles: Vec::new(), score: Some(score) };
        let shown = |kind: Kind, entry_id: i64, title: &str| Browsed { kind, entry_id, title: title.into() };
        job.lists.set(Kind::Anime, vec![hit(7, "Title /listed", 8.1)], 500);
        let page = vec![shown(Kind::Anime, listed, "title: /LISTED"), shown(Kind::Anime, unlisted, "Title /unlisted"), shown(Kind::Manga, manga, "Title /m")];
        let waiting = job.match_browsed(page);
        assert_eq!(waiting.iter().map(|browsed| browsed.entry_id).collect::<Vec<_>>(), [manga], "no manga lists yet: it waits for them");
        let stored = job.scores.by_entry(listed).expect("read").expect("a row");
        assert_eq!((stored.remote_id, stored.score, stored.matched_by, stored.fetched_at), (Some(7), Some(8.1), MatchedBy::Title, 500));
        assert_eq!(stored.matched_title, "Title /listed");
        assert_eq!(job.scores.by_entry(unlisted).expect("read"), None, "not in the lists: left to its own page");
        assert!(fake.asked.lock().expect("asked").is_empty(), "nothing was asked of the site");
        // newer lists refresh a score that came from the lists
        job.lists.set(Kind::Anime, vec![hit(7, "Title /listed", 8.3)], 900);
        assert!(job.match_browsed(vec![shown(Kind::Anime, listed, "Title /listed")]).is_empty());
        assert_eq!(job.scores.by_entry(listed).expect("read").map(|s| (s.score, s.fetched_at)), Some((Some(8.3), 900)));
        // the title the user chose stays, whatever the lists say
        job.scores.set_user_match(listed, "fake", Some((9, "Chosen".into(), Some(5.0))), 950).expect("choose");
        job.lists.set(Kind::Anime, vec![hit(7, "Title /listed", 8.6)], 1_200);
        job.match_browsed(vec![shown(Kind::Anime, listed, "Title /listed")]);
        assert_eq!(job.scores.by_entry(listed).expect("read").map(|s| (s.remote_id, s.score)), Some((Some(9), Some(5.0))));
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
