//! The one place a finished chapter or episode reaches the trackers (Aniyomi's `TrackChapter` / `TrackEpisode` after a
//! chapter is read or an episode is seen). The reader and the player call [`item_finished`]; the flow behind it is
//! `kuro_track::progress` through [`actions`]: every logged-in tracker with a binding that is behind the number gets
//! `max(number, its progress)`, the tracker's own update moves Plan to Reading and Completed at the known total, and
//! offline or failed writes go to the retry queue.

use crate::ui::DialogOpen as _;
use kuro_i18n::{MsgKey, tr};
use kuro_platform::kind::Kind;
use kuro_track::policy::{AutoTrackState, MarkReadPlan, auto_update_allowed};
use kuro_track::prefs::TrackPrefs;


use super::actions;
use super::auto_track::{self, Marking};

/// Chapter or episode `item_number` of entry `entry_id` was finished. Nothing happens in incognito or with the
/// "update progress after reading" switch off, and without the tracker services. When a tracker takes the update, "Trackers
/// updated to chapter N" or "... episode N" shows, the same notice as after a mark as read (one flow for both kinds).
pub fn item_finished(kind: Kind, entry_id: i64, item_number: f64, incognito: bool) {
    let Some(ctx) = crate::settings_ui::tracker_host::ctx() else { return };
    send_if_allowed(&ctx.prefs, incognito, || send_and_tell(kind, entry_id, item_number));
}

/// The one call into the tracker flow, for a finished item and for a mark-as-read alike.
fn send(kind: Kind, entry_id: i64, number: f64) -> Option<actions::Outcomes> {
    match kind {
        Kind::Manga => actions::chapter_read(entry_id, number),
        Kind::Anime => actions::episode_seen(entry_id, number),
    }
}

/// [`send`] for a mark as read: when a tracker accepted the update, "Trackers updated to chapter N" shows (T-MRK-6).
fn send_and_tell(kind: Kind, entry_id: i64, number: f64) {
    let Some(outcomes) = send(kind, entry_id, number) else { return };
    gtk::glib::spawn_future_local(async move {
        let accepted = outcomes.await.map_or(0, |all| all.iter().filter(|(_, outcome)| *outcome == kuro_track::progress::TrackOutcome::Updated).count());
        if auto_track::success_toast_shown(accepted) {
            let key = match kind {
                Kind::Anime => kuro_track::messages::MsgKey::TrackersUpdatedAnime(number as i32),
                Kind::Manga => kuro_track::messages::MsgKey::TrackersUpdatedManga(number as i32),
            };
            crate::toast::notify(kuro_track::messages::english(&key));
        }
    });
}

/// What marking items read or seen does for the trackers (the title page's mark as read, Aniyomi's
/// `updateTrackChapterMarkedAsRead`). `numbers` are the numbers of the items that changed. Incognito and marking
/// unread send nothing; otherwise `auto_update_on_mark_read` decides: Always sends the largest number, Ask shows a
/// confirm dialog first, Never sends nothing.
pub fn marked(kind: Kind, entry_id: i64, read: bool, numbers: &[f64], incognito: bool) {
    if incognito || !read || numbers.is_empty() {
        return;
    }
    let Some(ctx) = crate::settings_ui::tracker_host::ctx() else { return };
    let progress: Vec<f64> = kuro_data::track::TrackRepo::new(crate::data::app().db.clone())
        .by_entry(entry_id)
        .map(|tracks| tracks.iter().map(|t| t.last_item).collect())
        .unwrap_or_default();
    let plan = mark_plan(kind, read, super::session::has_logged_in(), numbers, &progress, ctx.prefs.auto_update_on_mark_read.get());
    match plan {
        MarkReadPlan::Nothing => {}
        MarkReadPlan::Apply { update_to, .. } => send_and_tell(kind, entry_id, update_to),
        MarkReadPlan::Prompt { update_to, banner } => {
            let body = kuro_track::messages::english(&banner);
            let buttons = vec![
                crate::ui::DialogButton::cancel(&tr(MsgKey::action_cancel)),
                crate::ui::DialogButton::new(&tr(MsgKey::ext_update), crate::ui::Role::Suggested, move || send_and_tell(kind, entry_id, update_to)),
            ];
            crate::ui::alert(&tr(MsgKey::ui_update_tracker), &body, buttons, || {}).open();
        }
    }
}

/// [`marked`] for items of several entries marked read in one write (Library and Updates): one send per entry, from the
/// items that changed. The items are read before the write, so only the ones that were not read yet count.
pub fn marked_items(kind: Kind, changed: &[kuro_data::item::Item]) {
    let incognito = crate::data::is_installed() && crate::data::app().base.incognito_mode.get();
    let mut entries: Vec<i64> = changed.iter().map(|i| i.entry_id).collect();
    entries.sort_unstable();
    entries.dedup();
    for entry_id in entries {
        let numbers: Vec<f64> = changed.iter().filter(|i| i.entry_id == entry_id).map(|i| i.number).collect();
        marked(kind, entry_id, true, &numbers, incognito);
    }
}

/// The decision of [`marked`], without its inputs read from the app.
fn mark_plan(kind: Kind, read: bool, has_logged_in: bool, numbers: &[f64], progress: &[f64], state: AutoTrackState) -> MarkReadPlan {
    let m = Marking { kind, read, has_logged_in, numbers, changed: true, tracker_progress: progress };
    auto_track::plan(&m, state, false)
}

/// Runs `send` when the switch and incognito allow it.
fn send_if_allowed(prefs: &TrackPrefs, incognito: bool, send: impl FnOnce()) {
    if auto_update_allowed(prefs, incognito) {
        send();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use kuro_prefs::store::PrefStore;

    use super::*;

    fn sent(prefs: &TrackPrefs, incognito: bool) -> bool {
        let hit = Cell::new(false);
        send_if_allowed(prefs, incognito, || hit.set(true));
        hit.get()
    }

    #[test]
    fn progress_goes_out_only_with_the_switch_on_and_incognito_off() {
        let prefs = TrackPrefs::new(PrefStore::in_memory());
        assert!(sent(&prefs, false));
        assert!(!sent(&prefs, true), "incognito sends nothing");
        prefs.auto_update_track.set(false).unwrap();
        assert!(!sent(&prefs, false), "the switch off sends nothing");
    }

    #[test]
    fn always_applies_ask_prompts_never_sends_nothing_and_unread_sends_nothing() {
        let plan = |read, state| mark_plan(Kind::Manga, read, true, &[3.0, 8.0], &[2.0], state);
        assert!(matches!(plan(true, AutoTrackState::ALWAYS), MarkReadPlan::Apply { update_to, .. } if update_to == 8.0));
        assert!(matches!(plan(true, AutoTrackState::ASK), MarkReadPlan::Prompt { update_to, .. } if update_to == 8.0));
        assert_eq!(plan(true, AutoTrackState::NEVER), MarkReadPlan::Nothing);
        assert_eq!(plan(false, AutoTrackState::ALWAYS), MarkReadPlan::Nothing, "marking unread sends nothing");
        assert_eq!(mark_plan(Kind::Anime, true, true, &[3.0], &[5.0], AutoTrackState::ALWAYS), MarkReadPlan::Nothing, "tracker already ahead");
    }

    #[test]
    fn without_the_services_it_is_a_no_op() {
        item_finished(Kind::Manga, 1, 2.0, false);
        item_finished(Kind::Anime, 1, 2.0, false);
    }
}
