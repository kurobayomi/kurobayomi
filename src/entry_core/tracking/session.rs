//! The tracker session of the entry screens (T-38b-029, T-39b-038, T-38a-032; E-TRK-5, T-TRK-12, D-10). The trackers live
//! in the settings host, which belongs to the main thread. The entry runners work on other threads, so the host's
//! trackers are published here once and every question reads the tracker's own login state at the moment it is asked.
//! Nothing is cached: a login or a logout shows on the next read.

use std::sync::{Arc, Mutex};

use kuro_platform::kind::Kind;
use kuro_track::tracker::Tracker;

use super::facts::{facts_of, summarize, Summary};

static TRACKERS: Mutex<Vec<Arc<dyn Tracker>>> = Mutex::new(Vec::new());

/// Publishes the registry. Called with the trackers of the settings host.
pub fn publish(trackers: Vec<Arc<dyn Tracker>>) {
    if let Ok(mut slot) = TRACKERS.lock() {
        *slot = trackers;
    }
}

/// Starts the tracker services when they are not up and publishes them when they are. Main thread only. Safe to call
/// on every screen open and every tracking-button press. Returns whether the services are up: `false` means the
/// registry is not published yet, so "no tracker is logged in" is not an answer and the caller waits for the next ask.
pub fn refresh_from_host() -> bool {
    crate::settings_ui::tracker_host::ensure();
    let Some(host) = crate::settings_ui::tracker_host::host() else {
        return false;
    };
    // only the switched-on trackers get progress writes and refreshes: the others are built, but stay out
    let enabled = crate::settings_ui::tracking::ENABLED;
    host.manager.restrict_to(&enabled);
    publish(enabled.iter().filter_map(|id| host.manager.get(*id)).collect());
    true
}

fn trackers() -> Vec<Arc<dyn Tracker>> {
    TRACKERS.lock().map(|slot| slot.clone()).unwrap_or_default()
}

/// Some tracker is logged in (a synchronous read, E-TRK-5).
pub fn has_logged_in() -> bool {
    trackers().iter().any(|t| t.is_logged_in())
}

/// The ids of the trackers logged in right now.
pub fn logged_in_ids() -> Vec<i64> {
    trackers().iter().filter(|t| t.is_logged_in()).map(|t| t.id().0).collect()
}

/// The count and the logged-in flag of an entry from `source_id`, bound to the trackers `bound`, through the same filter
/// as the sheet.
pub fn summary(kind: Kind, source_id: i64, bound: &[i64]) -> Summary {
    let source = source_id.to_string();
    let facts: Vec<_> = trackers().iter().filter_map(|t| facts_of(t.as_ref(), kind, Some(&source))).collect();
    summarize(&facts, bound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_no_published_trackers_nothing_is_logged_in_and_no_binding_counts() {
        // the registry is process-wide: this test only reads it before anything publishes
        publish(Vec::new());
        assert!(!has_logged_in());
        let s = summary(Kind::Manga, 1, &[2, 3]);
        assert_eq!((s.count, s.has_logged_in), (0, false));
    }
}
