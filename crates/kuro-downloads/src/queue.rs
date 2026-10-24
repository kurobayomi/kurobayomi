//! The `Queue` handle and its value types (docs/design/unified-downloads.md 2.5). The handle is a cheap clone: commands go
//! to the actor on an unbounded channel, so none is ever dropped, and reads come from a `watch` snapshot of both kinds.
//! Also the enqueue plan and the bulk warning, which are pure.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use kuro_db::Db;
use kuro_platform::kind::Kind;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::actor::{self, Command};
use crate::events::{DownloadEvent, EVENT_CAPACITY};
use crate::gate::Connectivity;
use crate::index::DownloadIndex;
use crate::layout::{FreeSpaceFn, Layout};
use crate::runner::{SourceDirectory, UnitRunner};
use crate::state::RunState;
use crate::store::QueueStore;
use crate::unit::{BULK_PER_SOURCE_THRESHOLD, BULK_TOTAL_THRESHOLD, EntryRef, QueuedUnit, SourceKey, UnitRef};
use crate::error::DownloadError;

/// How long the bulk-download warning stays.
pub const WARNING_TIMEOUT_SECS: u32 = 30;
pub const BULK_WARNING_TEXT: &str =
    "Warning: large bulk downloads may lead to sources becoming slower and/or blocking Kurobayomi. Click to learn more.";
pub const BULK_WARNING_HELP_URL: &str = kuro_platform::links::URL_LARGE_UPDATES;

pub struct Layouts {
    pub manga: Layout,
    pub anime: Layout,
}

impl Layouts {
    pub fn of(&self, kind: Kind) -> &Layout {
        match kind {
            Kind::Manga => &self.manga,
            Kind::Anime => &self.anime,
        }
    }
}

pub struct QueueParams {
    /// The one database: restore reads the entry and item rows through it.
    pub db: Arc<Db>,
    pub store: Arc<QueueStore>,
    /// One runner per kind.
    pub runners: Vec<Arc<dyn UnitRunner>>,
    pub directory: Arc<dyn SourceDirectory>,
    pub layouts: Layouts,
    pub index: DownloadIndex,
    pub free_space: FreeSpaceFn,
    /// The "Maximum downloads" pref, read at every scheduling pass.
    pub max_active: Arc<dyn Fn() -> usize + Send + Sync>,
    pub connectivity: watch::Receiver<Connectivity>,
    /// `false`: the diet cut the switch, a metered link does not stop the downloads.
    pub only_over_wifi: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Back,
    Front,
}

#[derive(Clone, Debug)]
pub struct EnqueueRequest {
    pub kind: Kind,
    pub entry: EntryRef,
    pub source: SourceKey,
    pub units: Vec<UnitRef>,
    pub place: Place,
    pub auto_start: bool,
    /// Skip fillermarked units. Only the automatic download after a refresh sets it; a hand pick never does (D6).
    pub skip_fillermarked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    LocalSource,
    NothingToDo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BulkWarning {
    pub text: &'static str,
    pub timeout_secs: u32,
    pub help_url: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enqueued {
    /// The unit ids added, in queue order.
    pub added: Vec<i64>,
    /// A run began with this call.
    pub started: bool,
    pub warning: Option<BulkWarning>,
    /// Units were added to a lane that is not running: the app offers a "Continue" action.
    pub continue_toast: bool,
    pub refused: Option<Refusal>,
}

impl Enqueued {
    pub(crate) fn refused(why: Refusal) -> Self {
        Self { added: Vec::new(), started: false, warning: None, continue_toast: false, refused: Some(why) }
    }
}

#[derive(Clone, Debug)]
pub enum CancelTarget {
    Units(Vec<i64>),
    Entry(i64),
}

/// Both lanes at one moment. Units are immutable behind their `Arc`; a change swaps the `Arc`.
pub struct Snapshot {
    pub items: Vec<Arc<QueuedUnit>>,
    pub manga: RunState,
    pub anime: RunState,
}

impl Snapshot {
    pub fn empty() -> Self {
        Self { items: Vec::new(), manga: RunState::Stopped, anime: RunState::Stopped }
    }

    /// The units of one lane, in queue order.
    pub fn of(&self, kind: Kind) -> impl Iterator<Item = &Arc<QueuedUnit>> {
        self.items.iter().filter(move |d| d.kind == kind)
    }

    pub fn run(&self, kind: Kind) -> RunState {
        match kind {
            Kind::Manga => self.manga,
            Kind::Anime => self.anime,
        }
    }

    pub fn find(&self, kind: Kind, unit_id: i64) -> Option<&Arc<QueuedUnit>> {
        self.items.iter().find(|d| d.kind == kind && d.unit.id == unit_id)
    }

    /// `true` when `other` shows the same units in the same order and the same run states.
    pub(crate) fn same_as(&self, other: &Snapshot) -> bool {
        self.manga == other.manga
            && self.anime == other.anime
            && self.items.len() == other.items.len()
            && self.items.iter().zip(&other.items).all(|(a, b)| Arc::ptr_eq(a, b))
    }
}

/// Holds a lane still while the disk changes under it. Dropping it lets the lane run again.
pub struct EntryHold {
    tx: mpsc::UnboundedSender<Command>,
    kind: Kind,
    /// `false` when no run held a unit of the entry, so there was nothing to stop and there is nothing to release.
    registered: bool,
}

impl Drop for EntryHold {
    fn drop(&mut self) {
        if self.registered {
            let _ = self.tx.send(Command::Release { kind: self.kind });
        }
    }
}

#[derive(Clone)]
pub struct Queue {
    tx: mpsc::UnboundedSender<Command>,
    snapshot: watch::Receiver<Arc<Snapshot>>,
    events: broadcast::Sender<DownloadEvent>,
}

fn gone() -> DownloadError {
    DownloadError::Internal("the download queue is not running".to_owned())
}

impl Queue {
    /// Starts the actor on the current Tokio runtime. It restores the saved queue and sweeps stray staging folders
    /// before it serves the first command.
    pub fn spawn(params: QueueParams) -> Queue {
        let (tx, rx) = mpsc::unbounded_channel();
        let (snapshot_tx, snapshot) = watch::channel(Arc::new(Snapshot::empty()));
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        tokio::spawn(actor::run(params, rx, snapshot_tx, events.clone()));
        Queue { tx, snapshot, events }
    }

    pub async fn enqueue(&self, req: EnqueueRequest) -> Result<Enqueued, DownloadError> {
        let (reply, answer) = oneshot::channel();
        self.tx.send(Command::Enqueue { req, reply }).map_err(|_| gone())?;
        answer.await.map_err(|_| gone())?
    }

    /// Puts the units at the front and starts the lane. `AlreadyDownloaded` when one is on disk.
    pub async fn start_now(&self, req: EnqueueRequest) -> Result<(), DownloadError> {
        let (reply, answer) = oneshot::channel();
        self.tx.send(Command::StartNow { req, reply }).map_err(|_| gone())?;
        answer.await.map_err(|_| gone())?
    }

    /// Explicit start: retries every ERROR unit of the lane.
    pub fn start(&self, kind: Kind) {
        let _ = self.tx.send(Command::Start { kind });
    }

    pub fn pause(&self, kind: Kind) {
        let _ = self.tx.send(Command::Pause { kind });
    }

    pub fn clear(&self, kind: Kind) {
        let _ = self.tx.send(Command::Clear { kind });
    }

    /// Puts the lane in this order. Unknown ids are ignored, units not listed follow, an empty list does nothing.
    pub fn reorder(&self, kind: Kind, unit_ids: Vec<i64>) {
        let _ = self.tx.send(Command::Reorder { kind, unit_ids });
    }

    /// Removes units from the queue. Returns the ids that were queued. Other units are left as they are: a failed unit
    /// stays failed.
    pub async fn cancel(&self, kind: Kind, target: CancelTarget) -> Result<Vec<i64>, DownloadError> {
        let (reply, answer) = oneshot::channel();
        self.tx.send(Command::Cancel { kind, target, reply }).map_err(|_| gone())?;
        answer.await.map_err(|_| gone())
    }

    /// Stops the lane when it is running a unit of this entry, so the disk can change under it. Drop the hold to let
    /// the lane run on.
    pub async fn hold_entry(&self, kind: Kind, entry_id: i64) -> Result<EntryHold, DownloadError> {
        let (reply, answer) = oneshot::channel();
        self.tx.send(Command::Hold { kind, entry_id, reply }).map_err(|_| gone())?;
        let registered = answer.await.map_err(|_| gone())?;
        Ok(EntryHold { tx: self.tx.clone(), kind, registered })
    }

    /// The set of installed sources changed: every waiting flag is asked again.
    pub fn sources_changed(&self) {
        let _ = self.tx.send(Command::SourcesChanged);
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<Snapshot>> {
        self.snapshot.clone()
    }

    pub fn queued(&self, kind: Kind, unit_id: i64) -> Option<Arc<QueuedUnit>> {
        self.snapshot.borrow().find(kind, unit_id).cloned()
    }

    pub fn events(&self) -> broadcast::Receiver<DownloadEvent> {
        self.events.subscribe()
    }
}

/// The units an enqueue adds, in queue order: highest source order first (the sort is stable). Dropped: fillermarked
/// units when `skip_fillermarked`, units already queued, units already downloaded, and repeats.
pub fn plan_enqueue(
    units: &[UnitRef],
    skip_fillermarked: bool,
    mut queued: impl FnMut(i64) -> bool,
    mut downloaded: impl FnMut(&UnitRef) -> bool,
) -> Vec<UnitRef> {
    let mut candidates: Vec<&UnitRef> = units.iter().filter(|u| !(skip_fillermarked && u.fillermark)).collect();
    candidates.sort_by_key(|u| std::cmp::Reverse(u.source_order));
    let mut taken = HashSet::new();
    let mut picked = Vec::new();
    for unit in candidates {
        if queued(unit.id) || taken.contains(&unit.id) || downloaded(unit) {
            continue;
        }
        taken.insert(unit.id);
        picked.push(unit.clone());
    }
    picked
}

/// The warning for a large queue: more than [`BULK_TOTAL_THRESHOLD`] queued units, or more than
/// [`BULK_PER_SOURCE_THRESHOLD`] of one source. `queue` is `(source id, unmetered)` per queued unit; units of an
/// unmetered source count for neither.
pub fn bulk_warning(queue: impl IntoIterator<Item = (i64, bool)>) -> Option<BulkWarning> {
    let mut total = 0usize;
    let mut per_source: HashMap<i64, usize> = HashMap::new();
    for (source_id, unmetered) in queue {
        if unmetered {
            continue;
        }
        total += 1;
        *per_source.entry(source_id).or_default() += 1;
    }
    let widest = per_source.values().copied().max().unwrap_or(0);
    (total > BULK_TOTAL_THRESHOLD || widest > BULK_PER_SOURCE_THRESHOLD).then_some(BulkWarning {
        text: BULK_WARNING_TEXT,
        timeout_secs: WARNING_TIMEOUT_SECS,
        help_url: BULK_WARNING_HELP_URL,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(id: i64, source_order: i64, fillermark: bool) -> UnitRef {
        UnitRef {
            id,
            entry_id: 1,
            url: format!("/u{id}"),
            name: format!("Unit {id}"),
            scanlator: None,
            done: false,
            bookmark: false,
            fillermark,
            source_order,
            date_upload: 0,
            number: id as f64,
        }
    }

    fn ids(units: Vec<UnitRef>) -> Vec<i64> {
        units.iter().map(|u| u.id).collect()
    }

    #[test]
    fn the_plan_drops_downloaded_queued_and_repeated_units_and_sorts_newest_first() {
        let units = vec![unit(1, 1, false), unit(3, 3, false), unit(4, 4, false), unit(4, 4, false), unit(2, 2, false)];
        let picked = plan_enqueue(&units, false, |id| id == 2, |u| u.id == 1);
        assert_eq!(ids(picked), vec![4, 3], "1 is on disk, 2 is queued, 4 is listed twice");
    }

    #[test]
    fn fillermarked_units_are_skipped_only_when_asked() {
        let units = vec![unit(1, 1, false), unit(2, 2, true)];
        assert_eq!(ids(plan_enqueue(&units, true, |_| false, |_| false)), vec![1], "the automatic download skips them");
        assert_eq!(ids(plan_enqueue(&units, false, |_| false, |_| false)), vec![2, 1], "a hand pick keeps them");
    }

    #[test]
    fn equal_source_orders_keep_their_input_order() {
        let units = vec![unit(7, 5, false), unit(3, 5, false), unit(9, 5, false)];
        assert_eq!(ids(plan_enqueue(&units, false, |_| false, |_| false)), vec![7, 3, 9]);
    }

    #[test]
    fn the_bulk_warning_needs_more_than_the_total_or_the_per_source_threshold() {
        let one_source = |n: usize| (0..n).map(|_| (1, false));
        assert!(bulk_warning(one_source(15)).is_none(), "15 of one source is not more than 15");
        assert!(bulk_warning(one_source(16)).is_some(), "16 of one source warns");
        let many_sources = |n: i64| (0..n).map(|i| (i, false));
        assert!(bulk_warning(many_sources(30)).is_none(), "30 in all is not more than 30");
        let warning = bulk_warning(many_sources(31)).expect("31 in all warns");
        assert_eq!((warning.timeout_secs, warning.text), (WARNING_TIMEOUT_SECS, BULK_WARNING_TEXT));
    }

    #[test]
    fn unmetered_sources_never_trigger_the_warning() {
        assert!(bulk_warning((0..100).map(|_| (1, true))).is_none());
        let mixed = (0..16).map(|_| (1, true)).chain((0..15).map(|_| (2, false)));
        assert!(bulk_warning(mixed).is_none());
    }
}
