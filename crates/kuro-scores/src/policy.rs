//! When a stored score is old enough to be fetched again. A score moves slowly, and slower once a title has ended.

use kuro_data::community_score::{MatchedBy, ScoreCandidate};
use kuro_track::model::TrackerId;

use crate::provider::TrackRef;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// A title still coming out.
pub const ONGOING_MS: i64 = 7 * DAY_MS;
/// A title that has ended (`entries.status` 2 completed, 4 publishing finished, 5 cancelled).
pub const FINISHED_MS: i64 = 30 * DAY_MS;
/// A title that was not found: it may have been added to the site since.
pub const NOT_FOUND_MS: i64 = 30 * DAY_MS;
/// A new MyAnimeList binding replaces a match by name, but is not asked more than once a day.
pub const REBIND_MS: i64 = DAY_MS;
/// When the name match learned to look past a plural `s` (`kuro_track::auto_bind::loose`, 2026-11-21 19:00 UTC): a
/// title found missing before it is asked once more at once, instead of a month after its miss.
pub const MATCH_RULE_CHANGED_AT: i64 = 1_795_287_600_000;

fn finished(status: i64) -> bool {
    matches!(status, 2 | 4 | 5)
}

fn ttl(status: i64) -> i64 {
    if finished(status) { FINISHED_MS } else { ONGOING_MS }
}

/// Whether `candidate` is looked up now.
pub fn due(candidate: &ScoreCandidate, tracks: &[TrackRef], now_ms: i64) -> bool {
    let Some(current) = &candidate.current else { return true };
    // a clock set back must not make everything due at once
    let age = (now_ms - current.fetched_at).max(0);
    if current.matched_by == MatchedBy::User {
        // "no score for this title" stays; a title the user chose is refreshed like any other
        return current.remote_id.is_some() && age >= ttl(candidate.status);
    }
    let Some(remote_id) = current.remote_id else { return age >= NOT_FOUND_MS || current.fetched_at < MATCH_RULE_CHANGED_AT };
    // the entry was bound on MyAnimeList since: that id is exact, a match by name is not
    let rebound = tracks.iter().any(|t| t.tracker == TrackerId::MYANIMELIST && t.remote_id != remote_id);
    (rebound && age >= REBIND_MS) || age >= ttl(candidate.status)
}

#[cfg(test)]
mod tests {
    use kuro_data::community_score::CommunityScore;
    use kuro_platform::kind::Kind;

    use super::*;

    fn candidate(status: i64, current: Option<CommunityScore>) -> ScoreCandidate {
        ScoreCandidate { entry_id: 1, kind: Kind::Anime, title: "T".into(), status, current }
    }

    fn row(remote_id: Option<i64>, by: MatchedBy, fetched_at: i64) -> CommunityScore {
        CommunityScore { entry_id: 1, provider: "myanimelist".into(), remote_id, matched_title: String::new(), matched_by: by, score: remote_id.map(|_| 8.0), fetched_at }
    }

    #[test]
    fn a_score_expires_after_a_week_while_the_title_runs_and_a_month_once_it_ended() {
        let now = 100 * DAY_MS;
        assert!(due(&candidate(1, None), &[], now), "never looked up");
        let fresh = row(Some(5), MatchedBy::Title, now - 6 * DAY_MS);
        let week_old = row(Some(5), MatchedBy::Title, now - 7 * DAY_MS);
        assert!(!due(&candidate(1, Some(fresh)), &[], now));
        assert!(due(&candidate(1, Some(week_old.clone())), &[], now), "ongoing: a week");
        for ended in [2, 4, 5] {
            assert!(!due(&candidate(ended, Some(week_old.clone())), &[], now), "status {ended}: a month");
            assert!(due(&candidate(ended, Some(row(Some(5), MatchedBy::Title, now - 30 * DAY_MS))), &[], now));
        }
        assert!(!due(&candidate(1, Some(row(Some(5), MatchedBy::Title, now + DAY_MS))), &[], now), "a clock set back ages nothing");
    }

    #[test]
    fn a_miss_is_retried_after_a_month_and_a_no_score_choice_never() {
        let now = MATCH_RULE_CHANGED_AT + 100 * DAY_MS;
        assert!(!due(&candidate(1, Some(row(None, MatchedBy::Title, now - 29 * DAY_MS))), &[], now));
        assert!(due(&candidate(1, Some(row(None, MatchedBy::Title, now - 30 * DAY_MS))), &[], now));
        assert!(!due(&candidate(1, Some(row(None, MatchedBy::User, 0))), &[], now), "the user said it has none");
        assert!(due(&candidate(1, Some(row(Some(9), MatchedBy::User, now - 8 * DAY_MS))), &[], now), "a chosen title is refreshed");
    }

    #[test]
    fn a_miss_from_before_the_plural_rule_is_asked_again_once() {
        let now = MATCH_RULE_CHANGED_AT + DAY_MS;
        assert!(due(&candidate(1, Some(row(None, MatchedBy::Title, MATCH_RULE_CHANGED_AT - DAY_MS))), &[], now), "missed under the old rule");
        assert!(!due(&candidate(1, Some(row(None, MatchedBy::Title, MATCH_RULE_CHANGED_AT + 1))), &[], now), "missed under the new rule: a month");
        assert!(!due(&candidate(1, Some(row(None, MatchedBy::User, 0))), &[], now), "the user's no-score choice stays");
        assert!(!due(&candidate(1, Some(row(Some(5), MatchedBy::Title, MATCH_RULE_CHANGED_AT - DAY_MS))), &[], now), "a found title waits its week");
    }

    #[test]
    fn a_new_myanimelist_binding_replaces_a_match_by_name_within_a_day() {
        let now = 100 * DAY_MS;
        let mal = |id| TrackRef { tracker: TrackerId::MYANIMELIST, remote_id: id, title: "T".into() };
        let by_name = row(Some(5), MatchedBy::Title, now - 2 * DAY_MS);
        assert!(due(&candidate(1, Some(by_name.clone())), &[mal(77)], now), "another id on MyAnimeList");
        assert!(!due(&candidate(1, Some(by_name.clone())), &[mal(5)], now), "the same id: nothing to change");
        assert!(!due(&candidate(1, Some(row(Some(5), MatchedBy::Title, now - DAY_MS / 2))), &[mal(77)], now), "asked today already");
        let anilist = TrackRef { tracker: TrackerId::ANILIST, remote_id: 77, title: "T".into() };
        assert!(!due(&candidate(1, Some(by_name)), &[anilist], now), "another tracker's id is not a MyAnimeList id");
    }
}
