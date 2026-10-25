//! Watch-progress and history writes (area 45 `ExternalIntents.kt:441-486`, `PlayerActivity`
//! saves; behaviour 16.10 and part 2 behaviour 7.4; T-45a-063, T-45a-064). Pure: the caller runs
//! the returned writes against the database. Times are milliseconds, as in the Kotlin model.

/// The episode fields that a progress write sends (`EpisodeUpdate`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EpisodeProgress {
    pub seen: bool,
    pub bookmark: bool,
    pub fillermark: bool,
    pub last_second_seen: i64,
    pub total_seconds: i64,
}

/// The episode fields that differ from the stored row (E-46). `None` means "leave the column".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EpisodeChanges {
    pub seen: Option<bool>,
    pub bookmark: Option<bool>,
    pub fillermark: Option<bool>,
    pub last_second_seen: Option<i64>,
    pub total_seconds: Option<i64>,
}

impl EpisodeChanges {
    /// The fields of `new` that differ from `stored`.
    pub fn diff(stored: &EpisodeProgress, new: &EpisodeProgress) -> Self {
        fn changed<T: PartialEq + Copy>(old: T, new: T) -> Option<T> {
            (old != new).then_some(new)
        }
        Self {
            seen: changed(stored.seen, new.seen),
            bookmark: changed(stored.bookmark, new.bookmark),
            fillermark: changed(stored.fillermark, new.fillermark),
            last_second_seen: changed(stored.last_second_seen, new.last_second_seen),
            total_seconds: changed(stored.total_seconds, new.total_seconds),
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// The writes of one save. `history_seen_at` is `Some(now)` when a history row is upserted.
/// `episode` is `Some` only when a progress write happens and at least one field changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavePlan {
    pub history_seen_at: Option<i64>,
    pub episode: Option<EpisodeChanges>,
}

/// Fires at most once per `interval_ms`, and always when `forced`. Used to coalesce history and
/// session writes (E-47). A forced write, such as on pause or close, restarts the interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteThrottle {
    interval_ms: i64,
    last_ms: Option<i64>,
}

/// The history and session write interval (E-47 and T-45b-030).
pub const HISTORY_INTERVAL_MS: i64 = 5000;

impl WriteThrottle {
    pub fn new(interval_ms: i64) -> Self {
        Self {
            interval_ms,
            last_ms: None,
        }
    }

    /// Whether a write happens now. The first call always writes.
    pub fn due(&mut self, now_ms: i64, forced: bool) -> bool {
        let due = forced
            || self
                .last_ms
                .is_none_or(|last| now_ms - last >= self.interval_ms);
        if due {
            self.last_ms = Some(now_ms);
        }
        due
    }
}

/// What one whole-second tick of playback asks for (behaviour 7.1 steps 1 to 9). Only the decisions
/// are here. The caller runs the completion and download work, and the save through `plan_save`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecondAction {
    /// The position in milliseconds, written to the episode's last second seen.
    pub position_ms: i64,
    /// The duration in milliseconds.
    pub total_ms: i64,
    /// Past the completion threshold, and tracking is allowed (step 7).
    pub completed: bool,
    /// Past a quarter of the duration, so download-ahead may run (step 9).
    pub download_ahead: bool,
}

/// Behaviour 7.1 `onSecondReached`. Returns `None` while the episode is loading, when there is no
/// episode or id `-1`, or when the duration is zero. Otherwise it returns the decisions for this
/// tick. `progress_allowed` is `progress_allowed(incognito, has_trackers)`.
pub fn on_second(
    loading_episode: bool,
    episode_id: Option<i64>,
    position_s: i64,
    duration_s: i64,
    threshold: f64,
    progress_allowed: bool,
) -> Option<SecondAction> {
    if loading_episode || duration_s == 0 {
        return None;
    }
    episode_id.filter(|id| *id != -1)?;
    let position_ms = position_s * 1000;
    let total_ms = duration_s * 1000;
    Some(SecondAction {
        position_ms,
        total_ms,
        completed: progress_allowed && position_ms as f64 >= total_ms as f64 * threshold,
        download_ahead: position_s as f64 / duration_s as f64 > 0.25,
    })
}

/// Whether the watched position counts as finished: `lastSecondSeen >= totalSeconds * threshold`.
/// The Kotlin default threshold is 0.85.
pub fn is_completed(last_second_seen: i64, total_seconds: i64, threshold: f64) -> bool {
    last_second_seen as f64 >= total_seconds as f64 * threshold
}

/// Whether progress is written in a save: `!incognito_source || has_trackers` (T56, T57). A source
/// in incognito keeps progress only for a tracked entry, so the tracker can still read it.
pub fn progress_allowed(incognito_source: bool, has_trackers: bool) -> bool {
    !incognito_source || has_trackers
}

/// Whether history is written: never for a source in incognito (T56, T57).
pub fn history_allowed(incognito_source: bool) -> bool {
    !incognito_source
}

/// Whether tracker sync runs. It reads the app-wide incognito flag, not the per-source one (E-17).
pub fn tracker_sync_allowed(incognito_global: bool, has_trackers: bool, auto_update: bool) -> bool {
    !incognito_global && has_trackers && auto_update
}

/// The inputs of one save that are not episode data. `forced` is set on pause and close.
#[derive(Clone, Copy, Debug)]
pub struct SaveContext {
    pub incognito_source: bool,
    pub has_trackers: bool,
    pub now_ms: i64,
    pub forced: bool,
    pub threshold: f64,
}

/// Plans one save of the current episode (T-45a-063, T-45a-064, T-45b-056, T-45b-026, T-45b-028).
/// `incognito_source` is the per-source state read for the current anime. History gets `now_ms`
/// unless incognito, and only when `history_throttle` says a write is due (every 5 s, or `forced`
/// on pause and close; E-47). Progress is written when `progress_allowed` and `total_seconds > 0`,
/// and only the fields that differ from `stored` (E-46). An unseen episode becomes seen when it is
/// completed, and a seen episode stays seen.
pub fn plan_save(
    ctx: SaveContext,
    history_throttle: &mut WriteThrottle,
    stored: &EpisodeProgress,
    episode: &EpisodeProgress,
) -> SavePlan {
    let SaveContext {
        incognito_source,
        has_trackers,
        now_ms,
        forced,
        threshold,
    } = ctx;
    let progress = (progress_allowed(incognito_source, has_trackers) && episode.total_seconds > 0)
        .then(|| {
            EpisodeChanges::diff(
                stored,
                &EpisodeProgress {
                    seen: episode.seen
                        || is_completed(episode.last_second_seen, episode.total_seconds, threshold),
                    ..episode.clone()
                },
            )
        })
        .filter(|changes| !changes.is_empty());
    SavePlan {
        history_seen_at: (history_allowed(incognito_source)
            && history_throttle.due(now_ms, forced))
        .then_some(now_ms),
        episode: progress,
    }
}

/// Where playback starts, in ms (`PlayerActivity.kt:1074`). An explicit `position` wins. Otherwise
/// a seen episode restarts at 0 unless `pref_preserve_watching_position` is on, and any other
/// episode resumes at its last second seen.
///
/// DECISION: this is the Aniyomi rule. An unseen episode resumes whatever the preference says,
/// because the preference only decides what a finished episode does.
pub fn resume_position_ms(
    position: Option<i64>,
    seen: bool,
    preserve_position: bool,
    last_second_seen_ms: i64,
) -> i64 {
    position.unwrap_or(if seen && !preserve_position {
        0
    } else {
        last_second_seen_ms
    })
}

/// One tracker of the anime, as far as the seen update needs it (T-45a-066).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackerEntry {
    pub track_id: i64,
    pub logged_in: bool,
    pub last_episode_seen: f64,
}

/// What an episode seen does for one tracker (T-45a-066, T-45a-067).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrackUpdate {
    /// Send the episode to the tracker now.
    Send { track_id: i64, episode: f64 },
    /// Offline: store it in the delayed-tracking store and schedule its replay job.
    Delay { track_id: i64, episode: f64 },
}

/// The tracker updates of a seen episode. Nothing happens unless the auto-update preference is on
/// and the episode is seen. Then every logged-in tracker whose last seen episode is lower than
/// `episode_number` gets one update: a send when online, a delayed entry when offline.
pub fn seen_tracker_updates(
    auto_update: bool,
    seen: bool,
    episode_number: f64,
    online: bool,
    trackers: &[TrackerEntry],
) -> Vec<TrackUpdate> {
    if !auto_update || !seen {
        return Vec::new();
    }
    trackers
        .iter()
        .filter(|t| t.logged_in && t.last_episode_seen < episode_number)
        .map(|t| {
            if online {
                TrackUpdate::Send {
                    track_id: t.track_id,
                    episode: episode_number,
                }
            } else {
                TrackUpdate::Delay {
                    track_id: t.track_id,
                    episode: episode_number,
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_throttle_fires_first_then_every_five_seconds_or_when_forced() {
        let mut t = WriteThrottle::new(HISTORY_INTERVAL_MS);
        assert!(t.due(0, false));
        assert!(!t.due(4999, false));
        assert!(t.due(5000, false));
        assert!(t.due(5001, true));
        assert!(!t.due(5002, false));
    }

    #[test]
    fn second_tick_is_skipped_while_loading_or_without_an_episode_or_duration() {
        assert_eq!(on_second(true, Some(1), 10, 100, 0.85, true), None);
        assert_eq!(on_second(false, None, 10, 100, 0.85, true), None);
        assert_eq!(on_second(false, Some(-1), 10, 100, 0.85, true), None);
        assert_eq!(on_second(false, Some(1), 10, 0, 0.85, true), None);
    }

    #[test]
    fn second_tick_reports_completion_and_download_ahead() {
        let mid = on_second(false, Some(1), 40, 100, 0.85, true).unwrap();
        assert_eq!((mid.position_ms, mid.total_ms), (40_000, 100_000));
        assert!(!mid.completed);
        assert!(mid.download_ahead);
        let end = on_second(false, Some(1), 85, 100, 0.85, true).unwrap();
        assert!(end.completed);
        assert!(
            !on_second(false, Some(1), 85, 100, 0.85, false)
                .unwrap()
                .completed
        );
        assert!(
            !on_second(false, Some(1), 25, 100, 0.85, true)
                .unwrap()
                .download_ahead
        );
    }

    fn episode(seen: bool, last: i64, total: i64) -> EpisodeProgress {
        EpisodeProgress {
            seen,
            bookmark: true,
            fillermark: false,
            last_second_seen: last,
            total_seconds: total,
        }
    }

    fn save(
        incognito: bool,
        trackers: bool,
        now: i64,
        stored: &EpisodeProgress,
        new: &EpisodeProgress,
    ) -> SavePlan {
        let mut throttle = WriteThrottle::new(HISTORY_INTERVAL_MS);
        plan_save(
            ctx(incognito, trackers, now, false),
            &mut throttle,
            stored,
            new,
        )
    }

    fn ctx(incognito_source: bool, has_trackers: bool, now_ms: i64, forced: bool) -> SaveContext {
        SaveContext {
            incognito_source,
            has_trackers,
            now_ms,
            forced,
            threshold: 0.85,
        }
    }

    fn blank() -> EpisodeProgress {
        episode(false, 0, 0)
    }

    #[test]
    fn t56_incognito_without_trackers_writes_neither_history_nor_progress() {
        let plan = save(true, false, 42, &blank(), &episode(false, 500, 1000));
        assert_eq!(plan, SavePlan::default());
    }

    #[test]
    fn t57_incognito_with_a_tracker_writes_progress_but_not_history() {
        let plan = save(true, true, 42, &blank(), &episode(false, 500, 1000));
        assert_eq!(plan.history_seen_at, None);
        assert_eq!(plan.episode.unwrap().last_second_seen, Some(500));
    }

    #[test]
    fn tracker_sync_follows_the_global_flag_only() {
        assert!(tracker_sync_allowed(false, true, true));
        assert!(!tracker_sync_allowed(true, true, true));
        assert!(!tracker_sync_allowed(false, false, true));
        assert!(!tracker_sync_allowed(false, true, false));
    }

    #[test]
    fn t58_normal_save_upserts_history_at_now_and_writes_only_changed_fields() {
        let stored = episode(false, 0, 1000);
        let plan = save(false, false, 42, &stored, &episode(false, 100, 1000));
        assert_eq!(plan.history_seen_at, Some(42));
        assert_eq!(
            plan.episode.unwrap(),
            EpisodeChanges {
                last_second_seen: Some(100),
                ..EpisodeChanges::default()
            }
        );
    }

    #[test]
    fn unchanged_episode_writes_no_progress() {
        let stored = episode(false, 100, 1000);
        let plan = save(false, false, 42, &stored, &stored.clone());
        assert_eq!(plan.episode, None);
        assert_eq!(plan.history_seen_at, Some(42));
    }

    #[test]
    fn zero_total_skips_progress_but_keeps_history() {
        let plan = save(false, false, 7, &blank(), &episode(false, 0, 0));
        assert_eq!(plan.history_seen_at, Some(7));
        assert_eq!(plan.episode, None);
    }

    #[test]
    fn t55_unseen_becomes_seen_at_threshold() {
        let stored = episode(false, 0, 1000);
        let at = save(false, false, 1, &stored, &episode(false, 850, 1000));
        assert_eq!(at.episode.unwrap().seen, Some(true));
        let before = save(false, false, 1, &stored, &episode(false, 849, 1000));
        assert_eq!(before.episode.unwrap().seen, None);
    }

    #[test]
    fn seen_stays_seen_even_below_threshold() {
        let stored = episode(true, 0, 1000);
        let plan = save(false, false, 1, &stored, &episode(true, 10, 1000));
        assert_eq!(plan.episode.unwrap().seen, None);
    }

    #[test]
    fn history_is_coalesced_to_five_seconds_but_forced_on_pause_and_close() {
        let mut throttle = WriteThrottle::new(HISTORY_INTERVAL_MS);
        let stored = episode(false, 0, 1000);
        let run = |t: &mut WriteThrottle, now: i64, forced: bool, last: i64| {
            plan_save(
                ctx(false, false, now, forced),
                t,
                &stored,
                &episode(false, last, 1000),
            )
        };
        assert_eq!(run(&mut throttle, 0, false, 1).history_seen_at, Some(0));
        let skipped = run(&mut throttle, 1000, false, 2);
        assert_eq!(skipped.history_seen_at, None);
        assert!(skipped.episode.is_some(), "progress is not throttled");
        assert_eq!(
            run(&mut throttle, 1500, true, 3).history_seen_at,
            Some(1500)
        );
        assert_eq!(
            run(&mut throttle, 6500, false, 4).history_seen_at,
            Some(6500)
        );
    }

    #[test]
    fn incognito_does_not_use_up_the_history_interval() {
        let mut throttle = WriteThrottle::new(HISTORY_INTERVAL_MS);
        let stored = blank();
        let new = episode(false, 5, 100);
        plan_save(ctx(true, false, 0, false), &mut throttle, &stored, &new);
        assert!(throttle.due(1, false));
    }

    #[test]
    fn completion_uses_the_given_threshold() {
        assert!(is_completed(500, 1000, 0.5));
        assert!(!is_completed(499, 1000, 0.5));
    }

    #[test]
    fn a_seen_episode_restarts_and_an_unseen_one_resumes() {
        // an explicit position always wins
        assert_eq!(resume_position_ms(Some(9000), true, false, 5000), 9000);
        // seen: restart at 0 (the preference is cut and off)
        assert_eq!(resume_position_ms(None, true, false, 5000), 0);
        // unseen: resume
        assert_eq!(resume_position_ms(None, false, false, 5000), 5000);
    }

    fn tracker(id: i64, logged_in: bool, seen: f64) -> TrackerEntry {
        TrackerEntry {
            track_id: id,
            logged_in,
            last_episode_seen: seen,
        }
    }

    #[test]
    fn seen_update_goes_to_each_logged_in_tracker_behind_the_episode() {
        let list = [
            tracker(1, true, 4.0),
            tracker(2, true, 6.0),
            tracker(3, false, 0.0),
        ];
        assert_eq!(
            seen_tracker_updates(true, true, 5.0, true, &list),
            vec![TrackUpdate::Send {
                track_id: 1,
                episode: 5.0
            }]
        );
    }

    #[test]
    fn nothing_without_the_preference_or_when_not_seen() {
        let list = [tracker(1, true, 0.0)];
        assert!(seen_tracker_updates(false, true, 5.0, true, &list).is_empty());
        assert!(seen_tracker_updates(true, false, 5.0, true, &list).is_empty());
    }

    #[test]
    fn offline_updates_are_delayed_not_sent() {
        let list = [tracker(7, true, 1.0)];
        assert_eq!(
            seen_tracker_updates(true, true, 2.0, false, &list),
            vec![TrackUpdate::Delay {
                track_id: 7,
                episode: 2.0
            }]
        );
    }
}
