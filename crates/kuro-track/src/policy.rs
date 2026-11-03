//! Mark-read tracking policy (`AutoTrackState.kt`).

use kuro_platform::kind::Kind;
use kuro_prefs::pref_enum;

use crate::manager::TrackerManager;
use crate::messages::MsgKey;
use crate::model::TrackerId;
use crate::prefs::TrackPrefs;

pref_enum! {
    /// What marking a chapter or episode read does to the trackers. Stored by name.
    pub enum AutoTrackState { ALWAYS, ASK, NEVER }
}

impl AutoTrackState {
    /// Label of the option in the settings list.
    pub fn label(self) -> &'static str {
        match self {
            AutoTrackState::ALWAYS => "Always",
            AutoTrackState::ASK => "Always ask",
            AutoTrackState::NEVER => "Never",
        }
    }
}

/// How long the "Trackers updated" toast stays up.
pub const TOAST_MS: u64 = 3_000;
/// How long the confirm banner stays up. Compose's `SnackbarDuration.Short` is the value used here.
pub const BANNER_MS: u64 = 4_000;

/// What the mark-read flow does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkReadDecision {
    /// Nothing to do.
    Skip,
    /// Ask the user whether to update the trackers.
    Prompt,
    /// Update the trackers now.
    Apply,
}

/// Decision of behaviour 11.2 steps 3, 6 and 7. `read` is the new read state, `tracker_behind` is true
/// when a tracker is below the largest number in the selection. The caller checks for logged-in trackers.
pub fn decide(read: bool, tracker_behind: bool, state: AutoTrackState) -> MarkReadDecision {
    if !read || !tracker_behind || state == AutoTrackState::NEVER {
        MarkReadDecision::Skip
    } else if state == AutoTrackState::ALWAYS {
        MarkReadDecision::Apply
    } else {
        MarkReadDecision::Prompt
    }
}

/// What the mark-read flow does after marking entries read (behaviour 11.2).
#[derive(Debug, Clone, PartialEq)]
pub enum MarkReadPlan {
    /// Nothing to do.
    Nothing,
    /// Update the trackers to `update_to` now, then show `toast` for [`TOAST_MS`].
    Apply { update_to: f64, toast: MsgKey },
    /// Show `banner` for [`BANNER_MS`]. The update to `update_to` runs only when the user confirms.
    Prompt { update_to: f64, banner: MsgKey },
}

/// The mark-read decision for one selection. `max_number` is the largest number marked, `tracker_progress` the
/// progress of each binding of the entry. The caller refreshes the trackers before it asks (step 4).
pub fn mark_read_plan(
    kind: Kind,
    read: bool,
    has_logged_in: bool,
    max_number: f64,
    tracker_progress: &[f64],
    state: AutoTrackState,
) -> MarkReadPlan {
    if !has_logged_in {
        return MarkReadPlan::Nothing;
    }
    let behind = tracker_progress.iter().any(|p| max_number > *p);
    let number = max_number as i32;
    match decide(read, behind, state) {
        MarkReadDecision::Skip => MarkReadPlan::Nothing,
        MarkReadDecision::Apply => MarkReadPlan::Apply {
            update_to: max_number,
            toast: match kind {
                Kind::Anime => MsgKey::TrackersUpdatedAnime(number),
                Kind::Manga => MsgKey::TrackersUpdatedManga(number),
            },
        },
        MarkReadDecision::Prompt => MarkReadPlan::Prompt {
            update_to: max_number,
            banner: match kind {
                Kind::Anime => MsgKey::ConfirmUpdateAnime(number),
                Kind::Manga => MsgKey::ConfirmUpdateManga(number),
            },
        },
    }
}

/// Whether the reader or player may push progress. Incognito never does (behaviour 9.3).
pub fn auto_update_allowed(prefs: &TrackPrefs, incognito: bool) -> bool {
    !incognito && prefs.auto_update_track.get()
}

/// Who of the logged-in trackers counts for an entry (behaviour 11.3). A non-enhanced tracker always counts; an
/// enhanced one counts only when it accepts `source_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackingSupport {
    /// Some supported tracker is logged in.
    pub has_logged_in: bool,
    /// Bindings of the entry whose tracker is supported.
    pub tracking_count: usize,
    /// The supported trackers, in registry order.
    pub supported: Vec<TrackerId>,
}

pub fn tracking_support(
    manager: &TrackerManager,
    kind: Kind,
    bound: &[TrackerId],
    source_id: &str,
) -> TrackingSupport {
    let supported: Vec<TrackerId> = manager
        .logged_in()
        .iter()
        .filter(|t| match kind {
            Kind::Anime => t.as_enhanced_anime().is_none_or(|e| e.accepts(source_id)),
            Kind::Manga => t.as_enhanced_manga().is_none_or(|e| e.accepts(source_id)),
        })
        .map(|t| t.id())
        .collect();
    let tracking_count = bound.iter().filter(|id| supported.contains(id)).count();
    TrackingSupport {
        has_logged_in: !supported.is_empty(),
        tracking_count,
        supported,
    }
}

/// The track sheet opens after adding to the library only when the preference is on and the entry already has a
/// supported binding (behaviour 11.4, defect D13).
pub fn auto_open_track_sheet(pref_on: bool, tracking_count: usize) -> bool {
    pref_on && tracking_count > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tr_27_043_mark_read_list_has_three_options_in_order() {
        let labels: Vec<&str> = AutoTrackState::ALL.iter().map(|s| s.label()).collect();
        assert_eq!(labels, vec!["Always", "Always ask", "Never"]);
    }

    #[test]
    fn tr_27_039_no_prompt_when_no_logged_in_tracker_or_policy_never_or_unread() {
        let behind = [1.0];
        assert_eq!(
            mark_read_plan(
                Kind::Manga,
                true,
                false,
                5.0,
                &behind,
                AutoTrackState::ALWAYS
            ),
            MarkReadPlan::Nothing
        );
        assert_eq!(
            mark_read_plan(Kind::Manga, true, true, 5.0, &behind, AutoTrackState::NEVER),
            MarkReadPlan::Nothing
        );
        assert_eq!(
            mark_read_plan(Kind::Manga, false, true, 5.0, &behind, AutoTrackState::ASK),
            MarkReadPlan::Nothing
        );
    }

    #[test]
    fn tr_27_039_always_applies_with_the_toast_text() {
        let plan = mark_read_plan(Kind::Anime, true, true, 3.0, &[1.0], AutoTrackState::ALWAYS);
        assert_eq!(
            plan,
            MarkReadPlan::Apply {
                update_to: 3.0,
                toast: MsgKey::TrackersUpdatedAnime(3),
            }
        );
        assert_eq!(TOAST_MS, 3_000);
    }

    #[test]
    fn tr_27_039_ask_prompts_with_the_selection_maximum_and_four_seconds() {
        // TT-27-113: marking 5 while the tracker is at 4.
        let plan = mark_read_plan(Kind::Manga, true, true, 5.0, &[4.0], AutoTrackState::ASK);
        assert_eq!(
            plan,
            MarkReadPlan::Prompt {
                update_to: 5.0,
                banner: MsgKey::ConfirmUpdateManga(5),
            }
        );
        assert_eq!(BANNER_MS, 4_000);
    }

    #[test]
    fn tr_27_039_no_prompt_when_every_tracker_is_already_at_the_maximum() {
        let plan = mark_read_plan(
            Kind::Manga,
            true,
            true,
            5.0,
            &[5.0, 7.0],
            AutoTrackState::ASK,
        );
        assert_eq!(plan, MarkReadPlan::Nothing);
    }

    #[test]
    fn tr_27_039_truncates_fractional_numbers_in_messages_only() {
        let plan = mark_read_plan(
            Kind::Manga,
            true,
            true,
            12.5,
            &[1.0],
            AutoTrackState::ALWAYS,
        );
        assert_eq!(
            plan,
            MarkReadPlan::Apply {
                update_to: 12.5,
                toast: MsgKey::TrackersUpdatedManga(12),
            }
        );
    }

    #[test]
    fn tr_27_040_gate_blocks_incognito_and_the_preference() {
        let prefs = crate::fakes::prefs();
        assert!(auto_update_allowed(&prefs, false));
        assert!(!auto_update_allowed(&prefs, true));
        prefs.auto_update_track.set(false).unwrap();
        assert!(!auto_update_allowed(&prefs, false));
    }

    #[test]
    fn tr_27_d13_tr_27_115_auto_open_needs_the_pref_and_a_supported_binding() {
        assert!(!auto_open_track_sheet(true, 0));
        assert!(auto_open_track_sheet(true, 1));
        assert!(!auto_open_track_sheet(false, 1));
    }

    #[test]
    fn stored_names_are_the_kotlin_names() {
        assert_eq!(AutoTrackState::ALWAYS.name(), "ALWAYS");
        assert_eq!(AutoTrackState::ASK.name(), "ASK");
        assert_eq!(AutoTrackState::NEVER.name(), "NEVER");
    }

    #[test]
    fn tr_27_113_ask_prompts_when_tracker_is_behind() {
        assert_eq!(
            decide(true, true, AutoTrackState::ASK),
            MarkReadDecision::Prompt
        );
        assert_eq!(
            decide(true, false, AutoTrackState::ASK),
            MarkReadDecision::Skip
        );
    }

    #[test]
    fn always_applies_and_never_skips() {
        assert_eq!(
            decide(true, true, AutoTrackState::ALWAYS),
            MarkReadDecision::Apply
        );
        assert_eq!(
            decide(true, true, AutoTrackState::NEVER),
            MarkReadDecision::Skip
        );
        assert_eq!(
            decide(false, true, AutoTrackState::ALWAYS),
            MarkReadDecision::Skip
        );
    }

    #[tokio::test]
    async fn tr_27_114_an_enhanced_tracker_that_rejects_the_source_does_not_count() {
        use crate::fakes::Harness;
        use crate::model::TrackerId;
        let h = Harness::new();
        h.login(TrackerId::KITSU);
        h.login(TrackerId::JELLYFIN);
        let bound = [TrackerId::KITSU, TrackerId::JELLYFIN];
        let s = tracking_support(&h.ctx.trackers, Kind::Anime, &bound, "src.other");
        assert_eq!(s.tracking_count, 1);
        assert_eq!(s.supported, vec![TrackerId::KITSU]);
        assert!(s.has_logged_in);
        let s = tracking_support(&h.ctx.trackers, Kind::Anime, &bound, "src.jf");
        assert_eq!(s.tracking_count, 2);
    }

    #[tokio::test]
    async fn tr_27_114_no_logged_in_tracker_means_no_support() {
        use crate::fakes::Harness;
        let h = Harness::new();
        let s = tracking_support(&h.ctx.trackers, Kind::Manga, &[], "src.kv");
        assert!(!s.has_logged_in);
        assert_eq!(s.tracking_count, 0);
    }
}
