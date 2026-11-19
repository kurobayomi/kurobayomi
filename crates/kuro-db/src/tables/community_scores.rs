//! `community_scores` statements (schema version 2): the public score of an entry, one row per entry at most. The row
//! is a cache a refresh job fills; `matched_by = 'user'` marks a match the user chose, which a fetched row never replaces.

use kuro_platform::kind::Kind;
use rusqlite::{Connection, Result};

use super::{exec, query_all, query_opt, rows::*};

const COLS: &str = "s.entry_id, s.provider, s.remote_id, s.matched_title, s.matched_by, s.score, s.fetched_at";
/// A fetched row leaves a match the user chose alone: only another choice of the user replaces it.
const UPSERT: &str = "INSERT INTO community_scores(entry_id,provider,remote_id,matched_title,matched_by,score,fetched_at) VALUES (:entryId,:provider,:remoteId,:matchedTitle,:matchedBy,:score,:fetchedAt) ON CONFLICT(entry_id) DO UPDATE SET provider = excluded.provider, remote_id = excluded.remote_id, matched_title = excluded.matched_title, matched_by = excluded.matched_by, score = excluded.score, fetched_at = excluded.fetched_at WHERE community_scores.matched_by <> 'user' OR excluded.matched_by = 'user'";
/// The refresh of a match the user chose: the score and its time only, and only while the row still names that title.
const REFRESH_USER: &str = "UPDATE community_scores SET score = :score, matched_title = :matchedTitle, fetched_at = :fetchedAt WHERE entry_id = :entryId AND matched_by = 'user' AND remote_id IS :remoteId";

pub fn by_entry(c: &Connection, entry_id: i64) -> Result<Option<CommunityScoreRow>> {
    let sql = format!("SELECT {COLS} FROM community_scores s WHERE s.entry_id = :entryId");
    query_opt(c, &sql, &[(":entryId", &entry_id)], CommunityScoreRow::from_row)
}

/// The score of every entry of the kind that has one, as (entry id, score).
pub fn scores(c: &Connection, kind: Kind) -> Result<Vec<(i64, f64)>> {
    query_all(
        c,
        "SELECT s.entry_id, s.score FROM community_scores s JOIN entries e ON e._id = s.entry_id WHERE e.kind = :kind AND s.score IS NOT NULL ORDER BY s.entry_id",
        &[(":kind", &kind.as_str())],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

/// Every library entry with the row it has, if any: what the refresh job chooses from.
pub fn candidates(c: &Connection) -> Result<Vec<ScoreCandidateRow>> {
    let sql = format!(
        "SELECT e._id AS e_id, e.kind, e.title, e.status, {COLS} FROM entries e LEFT JOIN community_scores s ON s.entry_id = e._id WHERE e.favorite = 1 ORDER BY e._id"
    );
    query_all(c, &sql, &[], ScoreCandidateRow::from_row)
}

/// One entry as a candidate, in the library or not (a title opened from Browse). `None` when the entry is gone.
pub fn candidate(c: &Connection, entry_id: i64) -> Result<Option<ScoreCandidateRow>> {
    let sql = format!(
        "SELECT e._id AS e_id, e.kind, e.title, e.status, {COLS} FROM entries e LEFT JOIN community_scores s ON s.entry_id = e._id WHERE e._id = :entryId"
    );
    query_opt(c, &sql, &[(":entryId", &entry_id)], ScoreCandidateRow::from_row)
}

/// Writes the row of its entry. Returns the rows changed: 0 when the stored row is a match the user chose and `row`
/// is not.
pub fn upsert(c: &Connection, row: &CommunityScoreRow) -> Result<usize> {
    exec(
        c,
        UPSERT,
        &[
            (":entryId", &row.entry_id),
            (":provider", &row.provider),
            (":remoteId", &row.remote_id),
            (":matchedTitle", &row.matched_title),
            (":matchedBy", &row.matched_by),
            (":score", &row.score),
            (":fetchedAt", &row.fetched_at),
        ],
    )
}

/// Refreshes the score of a match the user chose. Returns 0 when the user has changed the match meanwhile.
pub fn refresh_user(c: &Connection, row: &CommunityScoreRow) -> Result<usize> {
    exec(
        c,
        REFRESH_USER,
        &[
            (":score", &row.score),
            (":matchedTitle", &row.matched_title),
            (":fetchedAt", &row.fetched_at),
            (":entryId", &row.entry_id),
            (":remoteId", &row.remote_id),
        ],
    )
}

pub fn delete(c: &Connection, entry_id: i64) -> Result<usize> {
    exec(c, "DELETE FROM community_scores WHERE entry_id = :entryId", &[(":entryId", &entry_id)])
}
