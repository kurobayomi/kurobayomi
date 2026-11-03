//! Kitsu bind, update and refresh rules (T-30a-019, T-30a-020, T-30a-021), as pure functions over the personal fields of
//! one library entry. The tracker adapts them to anime and manga tracks, so both kinds share one rule set.

use super::api::{EntryFields, EntryMatch};
use super::error::KitsuError;
use super::{COMPLETED, dates, score, status};
use kuro_platform::kind::Kind;

/// The personal fields of one library entry (`copyPersonalFrom` of the source).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Personal {
    /// Episodes or chapters read.
    pub progress: f64,
    pub score: f64,
    /// Local status code.
    pub status: i64,
    /// Epoch milliseconds, 0 = unset.
    pub start_ms: i64,
    /// Epoch milliseconds, 0 = unset.
    pub finish_ms: i64,
    pub private: bool,
}

impl Personal {
    /// The fields a push sends (`EntryFields`).
    pub fn entry_fields(&self) -> EntryFields {
        EntryFields {
            status: self.status,
            progress: self.progress,
            score: self.score,
            start_date_ms: self.start_ms,
            finish_date_ms: self.finish_ms,
            private: self.private,
        }
    }
}

/// The personal fields of a remote entry. Unknown status strings and bad dates are errors, as in the source.
pub fn remote_personal(kind: Kind, found: &EntryMatch) -> Result<Personal, KitsuError> {
    let a = &found.entry.attributes;
    Ok(Personal {
        progress: a.progress as f64,
        score: score::from_rating(a.rating_twenty),
        status: status::from_wire(kind, &a.status)?,
        start_ms: dates::from_wire(a.started_at.as_deref())?,
        finish_ms: dates::from_wire(a.finished_at.as_deref())?,
        private: a.private,
    })
}

/// Bind on a found entry (section 7). The remote personal fields are copied, the local `private` stays, and the
/// status becomes `current` when there is progress, unless the remote entry is completed.
pub fn bind_found(local: &mut Personal, remote: Personal, has_progress: bool, current: i64) {
    local.progress = remote.progress;
    local.score = remote.score;
    local.status = remote.status;
    local.start_ms = remote.start_ms;
    local.finish_ms = remote.finish_ms;
    if local.status != COMPLETED && has_progress {
        local.status = current;
    }
}

/// The status of a new entry (section 7): `current` with progress, otherwise `planned`.
pub fn bind_new_status(has_progress: bool, current: i64, planned: i64) -> i64 {
    if has_progress { current } else { planned }
}

/// A progress or watch event (section 8). Only runs when `did_progress` is set, and never changes a completed entry.
/// A positive total reached by the truncated progress completes the entry. Any other event moves the entry to
/// `current`, which also turns dropped or on-hold entries back into reading or watching. Progress of exactly 1.0
/// starts the entry now. A total of 0 never completes it.
pub fn apply_progress_event(
    p: &mut Personal,
    total: i64,
    did_progress: bool,
    current: i64,
    now_ms: i64,
) {
    if !did_progress || p.status == COMPLETED {
        return;
    }
    if total > 0 && p.progress.trunc() as i64 == total {
        p.status = COMPLETED;
        p.finish_ms = now_ms;
    } else {
        p.status = current;
        if p.progress == 1.0 {
            p.start_ms = now_ms;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kitsu::{DROPPED, ON_HOLD, PLAN_TO_READ, READING, WATCHING};

    fn p(progress: f64, status: i64) -> Personal {
        Personal {
            progress,
            score: 0.0,
            status,
            start_ms: 0,
            finish_ms: 0,
            private: false,
        }
    }

    #[test]
    fn t_30a_020_reaching_the_total_completes_and_sets_the_finish_date() {
        let mut x = p(12.5, READING);
        apply_progress_event(&mut x, 12, true, READING, 99);
        assert_eq!(x.status, COMPLETED);
        assert_eq!(x.finish_ms, 99);
    }

    #[test]
    fn t_30a_020_a_zero_total_never_completes() {
        let mut x = p(12.0, READING);
        apply_progress_event(&mut x, 0, true, READING, 99);
        assert_eq!(x.status, READING);
        assert_eq!(x.finish_ms, 0);
    }

    #[test]
    fn t_30a_020_the_first_unit_starts_the_entry() {
        let mut x = p(1.0, PLAN_TO_READ);
        apply_progress_event(&mut x, 10, true, READING, 77);
        assert_eq!(x.status, READING);
        assert_eq!(x.start_ms, 77);
    }

    #[test]
    fn t_30a_020_dropped_and_on_hold_become_current_on_a_read_event() {
        for from in [DROPPED, ON_HOLD] {
            let mut x = p(4.0, from);
            apply_progress_event(&mut x, 10, true, READING, 1);
            assert_eq!(x.status, READING);
        }
        let mut x = p(4.0, DROPPED);
        apply_progress_event(&mut x, 10, true, WATCHING, 1);
        assert_eq!(x.status, WATCHING);
    }

    #[test]
    fn t_30a_020_completed_is_never_changed_by_an_event() {
        let mut x = p(3.0, COMPLETED);
        apply_progress_event(&mut x, 10, true, READING, 1);
        assert_eq!(x.status, COMPLETED);
    }

    #[test]
    fn t_30a_020_no_flag_leaves_the_status_alone() {
        let mut x = p(12.0, DROPPED);
        apply_progress_event(&mut x, 12, false, READING, 1);
        assert_eq!(x, p(12.0, DROPPED));
    }

    #[test]
    fn t_30a_019_found_copies_personal_but_not_private_and_forces_reading() {
        let mut local = Personal {
            private: true,
            ..p(0.0, PLAN_TO_READ)
        };
        let remote = Personal {
            progress: 7.0,
            score: 8.0,
            status: DROPPED,
            start_ms: 5,
            finish_ms: 6,
            private: false,
        };
        bind_found(&mut local, remote, true, READING);
        assert_eq!(local.progress, 7.0);
        assert_eq!(local.score, 8.0);
        assert_eq!(local.status, READING);
        assert_eq!((local.start_ms, local.finish_ms), (5, 6));
        assert!(local.private, "the local private flag is kept");
    }

    #[test]
    fn t_30a_019_found_completed_is_kept_and_dropped_without_progress_stays() {
        let mut local = p(0.0, PLAN_TO_READ);
        bind_found(&mut local, p(10.0, COMPLETED), true, READING);
        assert_eq!(local.status, COMPLETED);
        let mut local = p(0.0, PLAN_TO_READ);
        bind_found(&mut local, p(2.0, DROPPED), false, READING);
        assert_eq!(local.status, DROPPED);
    }

    #[test]
    fn t_30a_019_new_entry_status_follows_progress() {
        assert_eq!(bind_new_status(true, READING, PLAN_TO_READ), READING);
        assert_eq!(bind_new_status(false, READING, PLAN_TO_READ), PLAN_TO_READ);
    }

    #[test]
    fn t_30a_021_remote_personal_reads_every_field_of_the_entry() {
        let found = found_entry(
            r#"{"id":"77","attributes":{"status":"current","progress":7,"ratingTwenty":16,
            "startedAt":"2024-05-01T00:00:00.000Z","finishedAt":null,"private":true}}"#,
        );
        let got = remote_personal(Kind::Manga, &found).unwrap();
        assert_eq!(got.progress, 7.0);
        assert_eq!(got.score, 8.0);
        assert_eq!(got.status, READING);
        assert_eq!(got.start_ms, 1_714_521_600_000);
        assert_eq!(got.finish_ms, 0);
        assert!(got.private);
    }

    #[test]
    fn t_30a_021_remote_personal_rejects_an_unknown_status() {
        let found = found_entry(
            r#"{"id":"77","attributes":{"status":"weird","progress":0,"ratingTwenty":null,
            "startedAt":null,"finishedAt":null,"private":false}}"#,
        );
        assert_eq!(
            remote_personal(Kind::Anime, &found),
            Err(KitsuError::UnknownStatus("weird".into()))
        );
    }

    fn found_entry(entry: &str) -> EntryMatch {
        let entry: super::super::dto::LibraryEntry = serde_json::from_str(entry).unwrap();
        let media = serde_json::from_str(
            r#"{"id":9,"attributes":{"canonicalTitle":"T","status":"finished"}}"#,
        )
        .unwrap();
        EntryMatch { entry, media }
    }
}
