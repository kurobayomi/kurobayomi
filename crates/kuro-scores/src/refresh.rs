//! One pass of the refresh: the entries that are due become queries, a provider answers them one at a time, and the
//! answers become rows of the store. A provider that stops (the site asks to slow down, or does not answer) ends the
//! pass with what it has: the scores already stored stay as they are.

use kuro_data::community_score::{CommunityScore, MatchedBy, ScoreCandidate};
use kuro_data::track::Track;
use kuro_track::model::TrackerId;

use crate::provider::{Outcome, Query, ScoreProvider, Stop, TrackRef};

/// The bindings of an entry as a provider reads them.
pub fn track_refs(tracks: &[Track]) -> Vec<TrackRef> {
    tracks.iter().map(|t| TrackRef { tracker: TrackerId(t.tracker_id), remote_id: t.remote_id, title: t.title.clone() }).collect()
}

/// The query of one entry.
pub fn query_of(candidate: &ScoreCandidate, tracks: Vec<TrackRef>) -> Query {
    let known = candidate.current.as_ref().and_then(|current| current.remote_id.map(|id| (id, current.matched_by)));
    Query { entry_id: candidate.entry_id, kind: candidate.kind, title: candidate.title.clone(), known, tracks }
}

/// The row to store for `candidate` after `outcome`. A title that is not found now but had a match keeps that match
/// and its score (the site may have dropped it for a while) and only counts as asked; one that never had a match gets
/// the row that records the miss.
pub fn row_of(candidate: &ScoreCandidate, provider: &str, outcome: Outcome, now_ms: i64) -> CommunityScore {
    match (outcome, &candidate.current) {
        (Outcome::Found { hit, by }, _) => CommunityScore {
            entry_id: candidate.entry_id,
            provider: provider.to_owned(),
            remote_id: Some(hit.remote_id),
            matched_title: hit.title,
            matched_by: by,
            score: hit.score,
            fetched_at: now_ms,
        },
        (Outcome::NotFound, Some(current)) if current.remote_id.is_some() => CommunityScore { fetched_at: now_ms, ..current.clone() },
        (Outcome::NotFound, _) => CommunityScore {
            entry_id: candidate.entry_id,
            provider: provider.to_owned(),
            remote_id: None,
            matched_title: String::new(),
            matched_by: MatchedBy::Title,
            score: None,
            fetched_at: now_ms,
        },
    }
}

/// Asks `provider` for each of `batch` in order. Returns the rows of the entries it answered, and why it stopped when
/// it did not reach the end.
pub async fn run_batch(provider: &dyn ScoreProvider, batch: &[(ScoreCandidate, Query)], now_ms: impl Fn() -> i64) -> (Vec<CommunityScore>, Option<Stop>) {
    let mut rows = Vec::with_capacity(batch.len());
    for (candidate, query) in batch {
        if !provider.supports(candidate.kind) {
            continue;
        }
        match provider.lookup(query).await {
            Ok(outcome) => rows.push(row_of(candidate, provider.id(), outcome, now_ms())),
            Err(stop) => return (rows, Some(stop)),
        }
    }
    (rows, None)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use kuro_platform::kind::Kind;

    use super::*;
    use crate::provider::Hit;

    /// Answers each lookup with the next of a script.
    struct Scripted(Mutex<Vec<Result<Outcome, Stop>>>);

    #[async_trait]
    impl ScoreProvider for Scripted {
        fn id(&self) -> &'static str {
            "scripted"
        }
        fn supports(&self, kind: Kind) -> bool {
            kind == Kind::Anime
        }
        async fn lookup(&self, _query: &Query) -> Result<Outcome, Stop> {
            self.0.lock().expect("script").remove(0)
        }
        async fn search(&self, _kind: Kind, _text: &str) -> Result<Vec<Hit>, Stop> {
            Ok(Vec::new())
        }
    }

    fn candidate(entry_id: i64, kind: Kind, current: Option<CommunityScore>) -> ScoreCandidate {
        ScoreCandidate { entry_id, kind, title: format!("Title {entry_id}"), status: 1, current }
    }

    fn found(id: i64, score: f64) -> Outcome {
        Outcome::Found { hit: Hit { remote_id: id, title: format!("Site {id}"), alt_titles: Vec::new(), score: Some(score) }, by: MatchedBy::Title }
    }

    fn stored(entry_id: i64, remote_id: Option<i64>, by: MatchedBy) -> CommunityScore {
        CommunityScore { entry_id, provider: "scripted".into(), remote_id, matched_title: "Old".into(), matched_by: by, score: remote_id.map(|_| 7.0), fetched_at: 1 }
    }

    #[test]
    fn the_stored_match_is_what_a_query_asks_by() {
        let tracks = vec![Track { entry_id: 3, remote_id: 44, title: "On tracker".into(), ..Track::new(Kind::Anime, 1) }];
        let query = query_of(&candidate(3, Kind::Anime, Some(stored(3, Some(9), MatchedBy::User))), track_refs(&tracks));
        assert_eq!(query.known, Some((9, MatchedBy::User)));
        assert_eq!(query.tracks, [TrackRef { tracker: TrackerId::MYANIMELIST, remote_id: 44, title: "On tracker".into() }]);
        assert_eq!(query_of(&candidate(3, Kind::Anime, Some(stored(3, None, MatchedBy::Title))), Vec::new()).known, None, "a miss has no id");
    }

    #[test]
    fn a_miss_keeps_an_earlier_match_and_records_a_first_one() {
        let had = candidate(1, Kind::Anime, Some(stored(1, Some(9), MatchedBy::Tracker)));
        let kept = row_of(&had, "scripted", Outcome::NotFound, 50);
        assert_eq!((kept.remote_id, kept.score, kept.matched_by, kept.fetched_at), (Some(9), Some(7.0), MatchedBy::Tracker, 50), "asked, nothing lost");
        let never = row_of(&candidate(2, Kind::Anime, None), "scripted", Outcome::NotFound, 50);
        assert_eq!((never.remote_id, never.score, never.matched_by, never.fetched_at), (None, None, MatchedBy::Title, 50));
        let again = row_of(&candidate(2, Kind::Anime, Some(never)), "scripted", found(4, 8.5), 60);
        assert_eq!((again.remote_id, again.matched_title.as_str(), again.score), (Some(4), "Site 4", Some(8.5)));
    }

    #[tokio::test]
    async fn a_batch_stops_where_the_provider_stops_and_keeps_what_it_has() {
        let provider = Scripted(Mutex::new(vec![Ok(found(4, 8.5)), Err(Stop::RateLimited { retry_after: None }), Ok(found(5, 9.0))]));
        let batch: Vec<(ScoreCandidate, Query)> = [(1, Kind::Anime), (2, Kind::Manga), (3, Kind::Anime), (4, Kind::Anime)]
            .into_iter()
            .map(|(id, kind)| {
                let candidate = candidate(id, kind, None);
                let query = query_of(&candidate, Vec::new());
                (candidate, query)
            })
            .collect();
        let (rows, stopped) = run_batch(&provider, &batch, || 77).await;
        assert_eq!(rows.iter().map(|r| (r.entry_id, r.score, r.fetched_at)).collect::<Vec<_>>(), [(1, Some(8.5), 77)], "the manga is not this provider's, the third was not answered");
        assert_eq!(stopped, Some(Stop::RateLimited { retry_after: None }));
        assert_eq!(provider.0.lock().expect("script").len(), 1, "nothing was asked after the stop");
    }
}
