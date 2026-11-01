//! Where each chapter and episode stands in the one download queue: the adapter every download button reads (the chapter
//! and episode lists, the Updates rows and the queue page). It reads the service's snapshot and reports each change as a
//! [`Live`] value: the queue state and percent while queued, then `Downloaded` or `NotDownloaded` once the unit has left
//! the queue.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use kuro_downloads::DownloadState;
use kuro_downloads::queue::{CancelTarget, Snapshot};
use kuro_downloads::state::RunState;
use kuro_downloads::service::Downloads;
use kuro_downloads::unit::QueuedUnit;
use kuro_platform::kind::Kind;

// ponytail: each follower polls the snapshot 4 times a second (progress moves inside the units, not in the snapshot);
// move to the queue's progress events if many pages follow.
const TICK: Duration = Duration::from_millis(250);

/// One change of a unit, as a download button draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Live {
    pub kind: Kind,
    pub id: i64,
    pub state: DownloadState,
    pub percent: u8,
}

/// A unit's queue state and percent now.
fn now(unit: &QueuedUnit) -> (DownloadState, u8) {
    (unit.state, unit.progress.percent())
}

/// The download service, or `None` when the app runs none. Call it on the GTK thread.
pub fn downloads() -> Option<Downloads> {
    crate::shell::host::Shell::get()?.download_service()
}

/// The queue state and percent of one unit in `snapshot`, or `None` when it is not queued.
pub fn queued(snapshot: &Snapshot, kind: Kind, id: i64) -> Option<(DownloadState, u8)> {
    snapshot.find(kind, id).map(|unit| now(unit))
}

/// Takes one unit out of its queue (a cancel, with no question). Files already on disk are not touched.
pub fn cancel(kind: Kind, id: i64) {
    let Some(shell) = crate::shell::host::Shell::get() else { return };
    let Some(downloads) = shell.download_service() else { return };
    shell.runtime().spawn(async move {
        if let Err(error) = downloads.cancel(kind, CancelTarget::Units(vec![id])).await {
            tracing::warn!(%error, id, "the download was not cancelled");
        }
    });
}

/// Whether `kind`'s queue is running (not paused). Without a download service nothing is paused.
pub fn lane_running(kind: Kind) -> bool {
    downloads().is_none_or(|downloads| downloads.snapshot().run(kind) == RunState::Running)
}

/// Pauses `kind`'s queue: the running unit goes back to the queue. The download manager pauses a whole queue, not one unit.
pub fn pause(kind: Kind) {
    if let Some(downloads) = downloads() {
        downloads.pause(kind);
    }
}

/// Starts `kind`'s queue again (the queue page's Resume), which also retries the units that failed.
pub fn resume(kind: Kind) {
    if let Some(downloads) = downloads() {
        downloads.start(kind);
    }
}

/// Calls `on_change` on the GTK thread for every unit whose queue state or percent changed, and once more when it left
/// the queue, for as long as `owner` lives. The first call reports every unit queued at that time. Without a download
/// service nothing is ever reported.
pub fn follow(owner: &impl IsA<gtk::Widget>, on_change: impl Fn(Live) + 'static) {
    let Some(downloads) = downloads() else { return };
    let owner = owner.as_ref().downgrade();
    let mut seen: HashMap<(Kind, i64), (Arc<QueuedUnit>, (DownloadState, u8))> = HashMap::new();
    glib::timeout_add_local(TICK, move || {
        if owner.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        let snapshot = downloads.snapshot();
        for unit in &snapshot.items {
            let (kind, id, state) = (unit.kind, unit.unit.id, now(unit));
            if seen.get(&(kind, id)).is_none_or(|(_, last)| *last != state) {
                on_change(Live { kind, id, state: state.0, percent: state.1 });
            }
            seen.insert((kind, id), (Arc::clone(unit), state));
        }
        seen.retain(|&(kind, id), (unit, _)| {
            if snapshot.find(kind, id).is_some() {
                return true;
            }
            // its files on disk make it `Downloaded`, anything else (a cancel) is `NotDownloaded`
            let on_disk = downloads.is_downloaded(&unit.entry, &unit.source, &unit.unit);
            on_change(Live { kind, id, state: if on_disk { DownloadState::Downloaded } else { DownloadState::NotDownloaded }, percent: 0 });
            false
        });
        glib::ControlFlow::Continue
    });
}
