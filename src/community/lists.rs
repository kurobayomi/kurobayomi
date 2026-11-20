//! The bulk lists of the score site (`ScoreProvider::catalogue`), kept for a day: in memory, ready to find a title by
//! name, and in one cache file a kind, so that starting the app again asks the site nothing. A Browse cover gets its
//! score from them without a request of its own.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use kuro_data::community_score::{CommunityScore, MatchedBy};
use kuro_platform::kind::Kind;
use kuro_scores::catalogue::Catalogue;
use kuro_scores::provider::Hit;
use serde::{Deserialize, Serialize};

/// How long fetched lists are used before they are fetched again. A score moves slowly.
pub(super) const KEPT_MS: i64 = 24 * 60 * 60 * 1000;

/// The cache file of one kind, as it is read.
#[derive(Deserialize)]
struct Stored {
    fetched_at: i64,
    hits: Vec<Hit>,
}

/// The cache file of one kind, as it is written.
#[derive(Serialize)]
struct ToStore<'a> {
    fetched_at: i64,
    hits: &'a [Hit],
}

/// The lists of one kind and when they were fetched (epoch milliseconds).
#[derive(Clone)]
pub(super) struct Held {
    pub catalogue: Arc<Catalogue>,
    pub fetched_at: i64,
}

pub(super) struct Lists {
    /// Manga, then anime.
    held: Mutex<[Option<Held>; 2]>,
    /// The cache files, manga then anime; `None` keeps the lists in memory only (the tests).
    files: Option<[PathBuf; 2]>,
}

fn slot(kind: Kind) -> usize {
    usize::from(kind == Kind::Anime)
}

impl Lists {
    pub(super) fn new(files: Option<[PathBuf; 2]>) -> Self {
        Self { held: Mutex::new([None, None]), files }
    }

    fn lock(&self) -> MutexGuard<'_, [Option<Held>; 2]> {
        self.held.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn get(&self, kind: Kind) -> Option<Held> {
        self.lock()[slot(kind)].clone()
    }

    /// Whether the lists of `kind` are to be fetched: there are none, or they are a day old. Lists dated after now (a
    /// clock that was set back) count as old.
    pub(super) fn due(&self, kind: Kind, now_ms: i64) -> bool {
        match self.get(kind) {
            Some(held) => !(0..KEPT_MS).contains(&(now_ms - held.fetched_at)),
            None => true,
        }
    }

    /// Reads the cache files. A missing or unreadable file leaves its kind without lists; old ones are still read, and
    /// used until fresh ones replace them.
    pub(super) fn load_files(&self) {
        let Some(files) = &self.files else { return };
        for kind in [Kind::Manga, Kind::Anime] {
            let stored = std::fs::read(&files[slot(kind)]).ok().and_then(|bytes| serde_json::from_slice::<Stored>(&bytes).ok());
            if let Some(stored) = stored {
                self.lock()[slot(kind)] = Some(Held { catalogue: Arc::new(Catalogue::new(stored.hits)), fetched_at: stored.fetched_at });
            }
        }
    }

    /// Takes the lists just fetched and writes their cache file. A file that cannot be written is logged: the lists
    /// are used all the same, and fetched again at the next start.
    pub(super) fn set(&self, kind: Kind, hits: Vec<Hit>, now_ms: i64) {
        if let Some(files) = &self.files {
            let path = &files[slot(kind)];
            let written = serde_json::to_vec(&ToStore { fetched_at: now_ms, hits: &hits }).map_err(|error| error.to_string()).and_then(|bytes| {
                if let Some(folder) = path.parent() {
                    std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
                }
                std::fs::write(path, bytes).map_err(|error| error.to_string())
            });
            if let Err(error) = written {
                tracing::warn!(%error, "community scores: the cache file of the lists was not written");
            }
        }
        self.lock()[slot(kind)] = Some(Held { catalogue: Arc::new(Catalogue::new(hits)), fetched_at: now_ms });
    }
}

/// Whether an entry takes the score the lists hold for it. An entry with no stored score does. One with a score takes a
/// newer one only for the same title, matched by name before: a title the user chose, a tracker's match and a recorded
/// miss (a search already found no single title of that name) all stay as they are.
pub(super) fn takes(stored: Option<&CommunityScore>, remote_id: i64, lists_fetched_at: i64) -> bool {
    match stored {
        None => true,
        Some(row) => row.matched_by == MatchedBy::Title && row.remote_id == Some(remote_id) && row.fetched_at < lists_fetched_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: i64, title: &str, score: Option<f64>) -> Hit {
        Hit { remote_id: id, title: title.into(), alt_titles: vec!["Another name".into()], score }
    }

    fn row(remote_id: Option<i64>, by: MatchedBy, fetched_at: i64) -> CommunityScore {
        CommunityScore { entry_id: 1, provider: "myanimelist".into(), remote_id, matched_title: "T".into(), matched_by: by, score: remote_id.map(|_| 7.0), fetched_at }
    }

    #[test]
    fn lists_are_due_when_there_are_none_or_they_are_a_day_old() {
        let lists = Lists::new(None);
        assert!(lists.due(Kind::Anime, 1_000));
        lists.set(Kind::Anime, vec![hit(1, "Naruto", Some(8.0))], 1_000);
        assert!(!lists.due(Kind::Anime, 1_000));
        assert!(!lists.due(Kind::Anime, 1_000 + KEPT_MS - 1));
        assert!(lists.due(Kind::Anime, 1_000 + KEPT_MS), "a day old");
        assert!(lists.due(Kind::Anime, 999), "dated after now: the clock was set back");
        assert!(lists.due(Kind::Manga, 1_000), "each kind has its own");
        assert_eq!(lists.get(Kind::Anime).map(|held| (held.catalogue.len(), held.fetched_at)), Some((1, 1_000)));
    }

    #[test]
    fn the_lists_come_back_from_their_cache_files_with_their_date() {
        let dir = kuro_testkit::TempDir::new().unwrap();
        // the folder does not exist yet: it is made when the first lists are written
        let files = || Some([dir.path().join("scores").join("manga.json"), dir.path().join("scores").join("anime.json")]);
        let lists = Lists::new(files());
        lists.load_files();
        assert!(lists.get(Kind::Anime).is_none(), "no file yet");
        lists.set(Kind::Anime, vec![hit(1, "Naruto", Some(8.0)), hit(2, "Unrated", None)], 5_000);
        let again = Lists::new(files());
        again.load_files();
        let held = again.get(Kind::Anime).expect("read from the file");
        assert_eq!(held.fetched_at, 5_000);
        assert_eq!(held.catalogue.hits(), [hit(1, "Naruto", Some(8.0)), hit(2, "Unrated", None)]);
        assert_eq!(held.catalogue.find("another name").map(|hit| hit.remote_id), None, "two titles answer to that name");
        assert_eq!(held.catalogue.find("naruto").and_then(|hit| hit.score), Some(8.0));
        assert!(again.get(Kind::Manga).is_none(), "the other kind has no file");
        // a file that is not the lists is no lists
        std::fs::write(dir.path().join("scores").join("manga.json"), b"not json").unwrap();
        let broken = Lists::new(files());
        broken.load_files();
        assert!(broken.get(Kind::Manga).is_none());
    }

    #[test]
    fn an_entry_takes_the_lists_score_when_it_has_none_or_an_older_one_of_the_same_title_by_name() {
        assert!(takes(None, 7, 100));
        assert!(takes(Some(&row(Some(7), MatchedBy::Title, 50)), 7, 100), "the same title, an older score");
        assert!(!takes(Some(&row(Some(7), MatchedBy::Title, 100)), 7, 100), "as new as the lists");
        assert!(!takes(Some(&row(Some(8), MatchedBy::Title, 50)), 7, 100), "another title was matched");
        assert!(!takes(Some(&row(Some(7), MatchedBy::User, 50)), 7, 100), "the user's choice stays");
        assert!(!takes(Some(&row(Some(7), MatchedBy::Tracker, 50)), 7, 100), "a tracker's match is refreshed by its id");
        assert!(!takes(Some(&row(None, MatchedBy::Title, 50)), 7, 100), "a recorded miss stays");
    }
}
