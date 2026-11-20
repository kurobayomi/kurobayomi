//! The community score of an entry: the public score of a catalogue site (MyAnimeList today), one row per
//! entry at most, on a 0 to 10 scale. This is the one store of it: the library, the details page and the refresh job
//! all go through [`CommunityScoreRepo`], and nothing else asks a site for a score. The rows are a cache, so they are
//! not part of a backup.

use std::collections::HashMap;
use std::sync::Arc;

use kuro_db::handle::Db;
use kuro_db::subscribe::Subscription;
use kuro_db::tables::community_scores as t;
use kuro_db::tables::{CommunityScoreRow, ScoreCandidateRow};
use kuro_platform::kind::Kind;

use crate::{DataResult, ENTRY_DEBOUNCE, LIBRARY_DEBOUNCE};

/// Tables a score listing reads.
const SCORE_TABLE: &[&str] = &["community_scores"];

/// How the entry was matched to the title of the site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchedBy {
    /// By the id of a tracker the entry is bound to: exact.
    Tracker,
    /// By its name: the one result with an equal name.
    Title,
    /// The user chose it (or chose "no score"); a fetched row never replaces it.
    User,
}

impl MatchedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tracker => "tracker",
            Self::Title => "title",
            Self::User => "user",
        }
    }

    /// The stored text; anything unknown reads as a name match, the weakest one.
    pub fn parse(text: &str) -> Self {
        match text {
            "tracker" => Self::Tracker,
            "user" => Self::User,
            _ => Self::Title,
        }
    }
}

/// The stored score of one entry. `remote_id` is `None` when the entry was looked up and not found (or the user said
/// it has none); `score` is `None` then, and also for a title the site has no score for yet.
#[derive(Clone, Debug, PartialEq)]
pub struct CommunityScore {
    pub entry_id: i64,
    /// The provider that answered, `myanimelist` today.
    pub provider: String,
    pub remote_id: Option<i64>,
    /// The title the site gives it, shown so a wrong match is seen.
    pub matched_title: String,
    pub matched_by: MatchedBy,
    /// 0 to 10.
    pub score: Option<f64>,
    /// Epoch milliseconds.
    pub fetched_at: i64,
}

impl From<CommunityScoreRow> for CommunityScore {
    fn from(r: CommunityScoreRow) -> Self {
        Self {
            entry_id: r.entry_id,
            provider: r.provider,
            remote_id: r.remote_id,
            matched_title: r.matched_title,
            matched_by: MatchedBy::parse(&r.matched_by),
            score: r.score,
            fetched_at: r.fetched_at,
        }
    }
}

impl From<&CommunityScore> for CommunityScoreRow {
    fn from(s: &CommunityScore) -> Self {
        Self {
            entry_id: s.entry_id,
            provider: s.provider.clone(),
            remote_id: s.remote_id,
            matched_title: s.matched_title.clone(),
            matched_by: s.matched_by.as_str().to_owned(),
            score: s.score,
            fetched_at: s.fetched_at,
        }
    }
}

/// An entry the refresh job may look up, with the row it has.
#[derive(Clone, Debug, PartialEq)]
pub struct ScoreCandidate {
    pub entry_id: i64,
    pub kind: Kind,
    pub title: String,
    /// The entry's publication status (`entries.status`): a finished title's score is refreshed less often.
    pub status: i64,
    pub current: Option<CommunityScore>,
}

impl From<ScoreCandidateRow> for ScoreCandidate {
    fn from(r: ScoreCandidateRow) -> Self {
        Self { entry_id: r.entry_id, kind: r.kind, title: r.title, status: r.status, current: r.score.map(CommunityScore::from) }
    }
}

#[derive(Clone)]
pub struct CommunityScoreRepo {
    db: Arc<Db>,
}

impl CommunityScoreRepo {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }

    /// The stored row of one entry, found or not.
    pub fn by_entry(&self, entry_id: i64) -> DataResult<Option<CommunityScore>> {
        Ok(self.db.read(|c| Ok(t::by_entry(c, entry_id)?))?.map(CommunityScore::from))
    }

    /// The score of every entry of the kind that has one.
    pub fn scores(&self, kind: Kind) -> DataResult<HashMap<i64, f64>> {
        Ok(self.db.read(|c| Ok(t::scores(c, kind)?))?.into_iter().collect())
    }

    /// Every library entry with the row it has: what the refresh job chooses from.
    pub fn candidates(&self) -> DataResult<Vec<ScoreCandidate>> {
        Ok(self.db.read(|c| Ok(t::candidates(c)?))?.into_iter().map(ScoreCandidate::from).collect())
    }

    /// One entry as a candidate, in the library or not. `None` when the entry is gone.
    pub fn candidate(&self, entry_id: i64) -> DataResult<Option<ScoreCandidate>> {
        Ok(self.db.read(|c| Ok(t::candidate(c, entry_id)?))?.map(ScoreCandidate::from))
    }

    /// Stores fetched rows in one transaction. A row of an entry whose stored match the user chose only refreshes that
    /// match's score (and nothing when the user has changed the match meanwhile); a row of an entry that was deleted
    /// meanwhile is skipped.
    pub fn store_fetched(&self, rows: &[CommunityScore]) -> DataResult<()> {
        self.db.write(|tx| {
            for row in rows {
                if t::candidate(tx, row.entry_id)?.is_none() {
                    continue;
                }
                let row = CommunityScoreRow::from(row);
                if row.matched_by == MatchedBy::User.as_str() {
                    t::refresh_user(tx, &row)?;
                } else {
                    t::upsert(tx, &row)?;
                }
            }
            Ok(())
        })
    }

    /// The user's own choice for an entry: `chosen` is the title they picked with its score, or `None` for "this title
    /// has no score", which is kept and never looked up again.
    pub fn set_user_match(&self, entry_id: i64, provider: &str, chosen: Option<(i64, String, Option<f64>)>, now_ms: i64) -> DataResult<()> {
        let (remote_id, matched_title, score) = match chosen {
            Some((id, title, score)) => (Some(id), title, score),
            None => (None, String::new(), None),
        };
        let row = CommunityScoreRow { entry_id, provider: provider.to_owned(), remote_id, matched_title, matched_by: MatchedBy::User.as_str().to_owned(), score, fetched_at: now_ms };
        self.db.write(|tx| Ok(t::upsert(tx, &row).map(drop)?))
    }

    /// Forgets the row of an entry, so it is looked up again as if never seen.
    pub fn delete(&self, entry_id: i64) -> DataResult<()> {
        self.db.write(|tx| Ok(t::delete(tx, entry_id).map(drop)?))
    }

    /// The row of one entry, published again after each write to the table.
    pub fn watch_by_entry(&self, entry_id: i64) -> Subscription<Option<CommunityScore>> {
        self.db.subscribe(SCORE_TABLE, ENTRY_DEBOUNCE, move |c| Ok(t::by_entry(c, entry_id)?.map(CommunityScore::from)))
    }

    /// The scores of the kind as (entry id, score) in id order, published again after each write to the table. The
    /// refresh job stores a batch at a time, so the library reloads once per batch.
    pub fn watch_scores(&self, kind: Kind) -> Subscription<Vec<(i64, f64)>> {
        self.db.subscribe(SCORE_TABLE, LIBRARY_DEBOUNCE, move |c| Ok(t::scores(c, kind)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::{insert_anime, insert_manga, test_db};

    fn fetched(entry_id: i64, remote_id: Option<i64>, by: MatchedBy, score: Option<f64>, at: i64) -> CommunityScore {
        CommunityScore { entry_id, provider: "myanimelist".into(), remote_id, matched_title: remote_id.map_or(String::new(), |id| format!("Title {id}")), matched_by: by, score, fetched_at: at }
    }

    #[test]
    fn a_fetched_score_round_trips_and_is_listed_for_its_kind_only() {
        let (_d, db) = test_db();
        let manga = insert_manga(&db, "/m", true);
        let anime = insert_anime(&db, "/a", true);
        let repo = CommunityScoreRepo::new(db);
        assert_eq!(repo.by_entry(manga).expect("read"), None);
        let stored = [fetched(manga, Some(11), MatchedBy::Tracker, Some(8.25), 100), fetched(anime, Some(22), MatchedBy::Title, Some(7.5), 100)];
        repo.store_fetched(&stored).expect("store");
        assert_eq!(repo.by_entry(manga).expect("read"), Some(stored[0].clone()));
        assert_eq!(repo.scores(Kind::Manga).expect("scores"), HashMap::from([(manga, 8.25)]));
        assert_eq!(repo.scores(Kind::Anime).expect("scores"), HashMap::from([(anime, 7.5)]));
        // fetched again later: the row is replaced, not doubled
        repo.store_fetched(&[fetched(manga, Some(11), MatchedBy::Tracker, Some(8.3), 200)]).expect("store");
        assert_eq!(repo.by_entry(manga).expect("read").map(|s| (s.score, s.fetched_at)), Some((Some(8.3), 200)));
    }

    #[test]
    fn a_title_that_was_not_found_or_has_no_score_is_kept_but_not_listed() {
        let (_d, db) = test_db();
        let missing = insert_manga(&db, "/missing", true);
        let unrated = insert_manga(&db, "/unrated", true);
        let repo = CommunityScoreRepo::new(db);
        repo.store_fetched(&[fetched(missing, None, MatchedBy::Title, None, 5), fetched(unrated, Some(3), MatchedBy::Title, None, 5)]).expect("store");
        assert!(repo.scores(Kind::Manga).expect("scores").is_empty(), "neither has a score to show");
        assert_eq!(repo.by_entry(missing).expect("read").map(|s| s.remote_id), Some(None), "the miss is remembered");
        assert_eq!(repo.by_entry(unrated).expect("read").map(|s| s.remote_id), Some(Some(3)));
    }

    #[test]
    fn a_match_the_user_chose_is_only_refreshed_never_replaced() {
        let (_d, db) = test_db();
        let id = insert_manga(&db, "/m", true);
        let repo = CommunityScoreRepo::new(db);
        repo.store_fetched(&[fetched(id, Some(1), MatchedBy::Title, Some(6.0), 10)]).expect("store");
        repo.set_user_match(id, "myanimelist", Some((2, "The right one".into(), Some(9.0))), 20).expect("choose");
        // a lookup that started before the choice comes back with the old match
        repo.store_fetched(&[fetched(id, Some(1), MatchedBy::Title, Some(6.1), 30)]).expect("store");
        let kept = repo.by_entry(id).expect("read").expect("row");
        assert_eq!((kept.remote_id, kept.matched_by, kept.score), (Some(2), MatchedBy::User, Some(9.0)));
        // the refresh of the chosen title updates its score
        repo.store_fetched(&[CommunityScore { matched_title: "The right one".into(), ..fetched(id, Some(2), MatchedBy::User, Some(9.1), 40) }]).expect("store");
        assert_eq!(repo.by_entry(id).expect("read").map(|s| (s.score, s.fetched_at)), Some((Some(9.1), 40)));
        // a refresh of a title the user has since replaced changes nothing
        repo.set_user_match(id, "myanimelist", Some((3, "Another".into(), None)), 50).expect("choose again");
        repo.store_fetched(&[fetched(id, Some(2), MatchedBy::User, Some(9.9), 60)]).expect("store");
        assert_eq!(repo.by_entry(id).expect("read").map(|s| (s.remote_id, s.score)), Some((Some(3), None)));
        // "no score for this title" is a choice too
        repo.set_user_match(id, "myanimelist", None, 70).expect("no score");
        let none = repo.by_entry(id).expect("read").expect("row");
        assert_eq!((none.remote_id, none.matched_by, none.score), (None, MatchedBy::User, None));
    }

    #[test]
    fn candidates_are_the_library_entries_with_the_row_they_have() {
        let (_d, db) = test_db();
        let in_library = insert_manga(&db, "/in", true);
        let browsed = insert_manga(&db, "/browsed", false);
        let repo = CommunityScoreRepo::new(db);
        repo.store_fetched(&[fetched(in_library, Some(4), MatchedBy::Title, Some(7.0), 9)]).expect("store");
        let all = repo.candidates().expect("candidates");
        assert_eq!(all.iter().map(|c| c.entry_id).collect::<Vec<_>>(), [in_library], "a title that was only browsed is not refreshed");
        assert_eq!(all[0].current.as_ref().and_then(|s| s.score), Some(7.0));
        assert_eq!(all[0].kind, Kind::Manga);
        let one = repo.candidate(browsed).expect("candidate").expect("the entry exists");
        assert_eq!((one.entry_id, one.current), (browsed, None), "but it can be looked up when its page opens");
        assert_eq!(repo.candidate(i64::MAX).expect("candidate"), None);
        // a row for an entry that is gone is skipped, not an error
        repo.store_fetched(&[fetched(i64::MAX, Some(1), MatchedBy::Title, Some(1.0), 1)]).expect("store");
        repo.delete(in_library).expect("forget");
        assert_eq!(repo.by_entry(in_library).expect("read"), None);
    }
}
