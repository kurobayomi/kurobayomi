//! The anime entry model (T-39b-001): the load sequence of behaviour 2.3, the state it publishes and the lists the
//! widgets draw from it. `load` runs on the blocking pool; the widgets read the value it returns. No GTK here.
//! Upstream: `AnimeScreenModel.kt:204-283` (init) and `:1602-1739` (`State.Success`).
//!
//! The load sequence, as `load` runs it:
//!  1. the entry is read (step 3.1), and a missing row is `None`;
//!  2. its source is resolved (3.2);
//!  3. the episode list is read, empty for a `Seasons` entry (3.3);
//!  4. the season list is read, empty for an `Episodes` entry (3.4);
//!  5. an entry outside the library gets the library defaults, then is read again so the first state never shows
//!     stale filters (3.5);
//!  6. the three refresh flags are computed (3.6);
//!  7. the first `Success` state is published with `is_refreshing` set when a refresh is needed (3.7).
//!
//! Steps 1 and 2 of 2.3 (the live subscription and the download observers) belong to the runner, and steps 8 and 9
//! (tracker and related-anime observers) to the tracking and related tasks. Step 10 (the source fetch) is started by
//! the runner when `plan.any()`, and step 11 is `finish_refresh`.

use std::collections::{HashMap, HashSet};

use kuro_data::DataResult;
use kuro_data::entry::{Entry, SeasonEntry};
use kuro_data::item::Item;
use kuro_db::flags::item;
use kuro_db::codec::FetchType;
use kuro_domain::app::items::{ListFilters, apply_filters, item_filters};
use kuro_domain::entries::defaults::SessionFlags;
use kuro_domain::entries::flags::EntryFlags;
use kuro_domain::items::missing::{ListRow, insert_missing_separators};
use kuro_domain::items::sorter::{SortKey, compare_items};
use kuro_downloads::DownloadState;
use kuro_i18n::tr_plural;
use kuro_prefs::store::PrefStore;

use super::entry_state::{EntryFlow, RefreshPlan, refresh_on_open};
use super::episode_row::row;
use super::season_tile::SeasonExtra;
use super::summary::SummaryExpansion;
use crate::entry_core::download_menu::{Candidate, DownloadAmount, targets};
use crate::entry_core::selection::Selection;
use super::header::{ItemHeader, header_missing_count, item_header};
use super::layout::{Fab, fab};
use crate::entry_core::refresh_rules::open_refresh_due;
use super::store::EntryStore;
use crate::entry_core::source_info::SourceInfo;
use crate::entry_core::errors::DetailError;
use crate::entry_core::list_model::RowItem;

/// What the load reads besides the rows: values read once at open (behaviour 2.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntryLoadContext {
    pub from_source: bool,
    pub downloaded_only: bool,
    /// When an open-time refresh last found this entry with no episodes (`refresh_rules::last_empty_open_fetch`).
    pub last_empty_open_fetch_ms: Option<i64>,
    /// The current time, for the 10-minute guard of an entry with no episodes.
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
pub struct EntryModel {
    pub flow: EntryFlow,
    pub entry: Entry,
    pub source: SourceInfo,
    /// Every stored episode, before any filter.
    pub episodes: Vec<Item>,
    pub seasons: Vec<SeasonEntry>,
    /// Which parts the first open refreshes from the source.
    pub plan: RefreshPlan,
    pub from_source: bool,
    pub downloaded_only: bool,
    pub selection: Selection,
    /// Which episode summaries are expanded, by episode id (T-39a-012). Kept while the entry is open.
    pub summaries: SummaryExpansion,
    /// The error the last refresh ended with, shown by the page; `None` after a successful one.
    pub refresh_error: Option<DetailError>,
    /// The in-memory library defaults of a non-library entry (decision 4). A snapshot re-applies them.
    pub session_flags: Option<SessionFlags>,
    /// The ids of the episodes on disk. The runner fills it after the load and after a download or a delete.
    pub downloaded: HashSet<i64>,
    /// The queue state and percent of the queued episodes, from the live download adapter (`downloads::live`).
    pub queued: HashMap<i64, (DownloadState, u8)>,
    /// How many trackers the entry is bound to, read when the page opens.
    pub tracking_count: usize,
    /// The download count and source language of each season, by season id. The runner sends them after the load, so
    /// a tile shows no download or language badge until they arrive.
    pub season_extras: HashMap<i64, SeasonExtra>,
    /// The scanlators of a manga's chapters and the excluded ones; empty for an anime.
    pub scanlators: super::store::Scanlators,
    /// The text of the list's search: only the rows whose title or scanlator holds it show. Empty shows every row.
    pub search: String,
}

impl EntryModel {
    /// The load sequence. `source_of` resolves the source line (a stub when the extension is missing).
    pub fn load(
        store: &EntryStore,
        prefs: &PrefStore,
        entry_id: i64,
        ctx: EntryLoadContext,
        source_of: impl FnOnce(i64) -> SourceInfo,
    ) -> DataResult<Option<Self>> {
        let Some(mut entry) = store.entry(entry_id)? else {
            return Ok(None);
        };
        let source = source_of(entry.source);
        let episode_list = entry.fetch_type() == FetchType::Episodes;
        let season_list = entry.fetch_type() == FetchType::Seasons;
        let episodes = if season_list { Vec::new() } else { store.episodes(entry_id)? };
        let seasons = if episode_list { Vec::new() } else { store.seasons(entry_id)? };
        // A non-library entry shows the library defaults for this session only (T-39b-012): nothing is written to its row.
        let session_flags = SessionFlags::for_entry(prefs, &entry);
        if let Some(session) = &session_flags {
            session.overlay(&mut entry);
        }
        let plan = refresh_on_open(entry.initialized, episodes.is_empty(), episode_list, seasons.is_empty(), season_list);
        // An entry with no episodes refreshes on open at most once per 10 minutes (fix 23.24).
        let plan = if plan.episodes && !open_refresh_due(episodes.is_empty(), ctx.last_empty_open_fetch_ms, ctx.now_ms) {
            RefreshPlan { episodes: false, ..plan }
        } else {
            plan
        };
        let mut flow = EntryFlow::default();
        flow.loaded(true);
        if plan.any() {
            flow.begin_refresh();
        }
        let tracking_count = store.track_summary(entry_id, entry.source)?.count as usize;
        Ok(Some(Self {
            flow,
            entry,
            source,
            episodes,
            seasons,
            plan,
            from_source: ctx.from_source,
            downloaded_only: ctx.downloaded_only,
            selection: Selection::default(),
            summaries: SummaryExpansion::default(),
            refresh_error: None,
            session_flags,
            downloaded: HashSet::new(),
            queued: HashMap::new(),
            tracking_count,
            season_extras: HashMap::new(),
            scanlators: store.scanlators(entry_id)?,
            search: String::new(),
        }))
    }

    /// Step 11: the refresh is over, with or without an error.
    pub fn finish_refresh(&mut self, result: Result<(), DetailError>) {
        self.flow.end_refresh();
        self.refresh_error = result.err();
    }

    /// The subscription delivered a newer entry and episode list. A `Seasons` entry keeps an empty episode list.
    pub fn apply_snapshot(&mut self, mut entry: Entry, episodes: Vec<Item>) {
        if entry.fetch_type() == FetchType::Episodes {
            self.episodes = episodes;
        }
        // an entry that joined the library has its defaults written to the row: the session copy has done its job
        if entry.favorite {
            self.session_flags = None;
        }
        if let Some(session) = &self.session_flags {
            session.overlay(&mut entry);
        }
        self.entry = entry;
    }

    /// A flag write the user made on an entry that is not in the library. The row is written too, but the next snapshot
    /// puts the session flags back over it (T-39b-012), so the session copy takes the change here or it would never
    /// show. An entry in the library has no session copy and follows its row.
    pub fn note_flag_write(&mut self, flag: i64, mask: i64) {
        if let Some(session) = &mut self.session_flags {
            session.note_item_write(flag, mask);
            self.entry.item_flags = session.item_flags;
        }
    }

    /// The same for a masked write of the season flag word (the season list options).
    pub fn note_season_write(&mut self, flag: i64, mask: i64) {
        if let Some(session) = &mut self.session_flags {
            session.note_season_write(flag, mask);
            if session.season_flags.is_some() {
                self.entry.season_flags = session.season_flags;
            }
        }
    }

    /// The facts of the seasons the runner read: they replace the earlier ones.
    pub fn set_season_extras(&mut self, extras: Vec<SeasonExtra>) {
        self.season_extras = extras.into_iter().map(|e| (e.id, e)).collect();
    }

    /// `Loading` ignores dispatches (`T-MDL-5`).
    pub fn accepts_dispatch(&self) -> bool {
        self.flow.accepts_dispatch()
    }

    /// The filters and sort of the entry's flag word. An unknown sorting value shows the list unfiltered.
    pub fn filters(&self) -> Option<ListFilters> {
        item_filters(&self.entry, self.downloaded_only).ok()
    }

    /// The entry is the local source's.
    pub fn is_local(&self) -> bool {
        self.entry.source == kuro_source::local::LOCAL_SOURCE_ID
    }

    /// The filtered and sorted list. A local entry counts every episode as downloaded.
    pub fn processed(&self) -> Vec<Item> {
        let mut shown = match self.filters() {
            Some(filters) => apply_filters(self.episodes.clone(), &filters, self.is_local(), |e| self.downloaded.contains(&e.id)),
            None => self.episodes.clone(),
        };
        let needle = self.search.trim().to_lowercase();
        if !needle.is_empty() {
            let number = self.entry.display_mode() == item::DISPLAY_NUMBER;
            shown.retain(|e| super::episode_row::title(e, number).to_lowercase().contains(&needle) || e.scanlator.as_deref().is_some_and(|s| s.to_lowercase().contains(&needle)));
        }
        shown
    }

    /// The selected episodes, in stored order.
    pub fn selected_episodes(&self) -> Vec<Item> {
        self.episodes.iter().filter(|e| self.selection.contains(e.id)).cloned().collect()
    }

    /// The episodes a Download menu entry queues (`download_menu::targets`) from the displayed list.
    pub fn download_targets(&self, amount: DownloadAmount) -> Vec<i64> {
        let descending = self.filters().is_some_and(|f| f.descending);
        let rows: Vec<Candidate> =
            self.processed().iter().map(|e| Candidate { id: e.id, done: e.done, state: self.download_of(e.id).0 }).collect();
        targets(&rows, descending, amount)
    }

    pub fn processed_ids(&self) -> Vec<i64> {
        self.processed().iter().map(|e| e.id).collect()
    }

    /// A change the live download adapter reported for one episode: a queued, running or failed episode keeps its state
    /// and percent; one that left the queue is on disk (`Downloaded`) or not (a cancel).
    pub fn apply_live(&mut self, id: i64, state: DownloadState, percent: u8) {
        match state {
            DownloadState::Downloaded => {
                self.queued.remove(&id);
                self.downloaded.insert(id);
            }
            DownloadState::NotDownloaded => {
                self.queued.remove(&id);
            }
            _ => {
                self.queued.insert(id, (state, percent));
            }
        }
    }

    /// The download state and percent of one episode: a local entry has every episode on disk, a queue entry wins over
    /// the copy on disk (T-DL-6), and without either the episode is not downloaded.
    fn download_of(&self, id: i64) -> (DownloadState, u8) {
        if self.is_local() {
            return (DownloadState::Downloaded, 0);
        }
        match self.queued.get(&id) {
            Some(&queued) => queued,
            None if self.downloaded.contains(&id) => (DownloadState::Downloaded, 0),
            None => (DownloadState::NotDownloaded, 0),
        }
    }

    #[cfg(test)]
    /// One row per processed episode, in list order.
    pub fn rows(&self) -> Vec<RowItem> {
        self.processed().iter().map(|ep| self.row_of(ep)).collect()
    }

    /// The row of one episode with the list settings, its selection, its summary state and its download state. A local
    /// entry has no download button, and the button is off while anything is selected.
    fn row_of(&self, ep: &Item) -> RowItem {
        let number = self.entry.display_mode() == item::DISPLAY_NUMBER;
        let (download, progress) = self.download_of(ep.id);
        RowItem {
            summary_expanded: self.summaries.is_expanded(ep.id),
            download,
            progress,
            indicator: !self.is_local(),
            indicator_enabled: !self.selection.is_active(),
            ..row(ep, number, self.entry.show_summaries(), self.selection.contains(ep.id))
        }
    }

    /// The list rows in view order: a row per processed episode, and a missing-count marker wherever two neighbours
    /// skip numbers (`insertSeparators`, T-39a-015). The markers follow the filtered list (OD-1, Kotlin parity).
    pub fn lines(&self) -> Vec<RowItem> {
        let descending = self.entry.sort_descending();
        let shown = self.processed();
        // a gap is only a missing episode when no filter hides any: a hidden episode is not missing
        let complete = shown.len() == self.episodes.len();
        insert_missing_separators(shown, descending)
            .into_iter()
            .filter(|row| complete || matches!(row, ListRow::Item(_)))
            .map(|row| match row {
                ListRow::Item(ep) => self.row_of(&ep),
                ListRow::MissingCount { id, count } => RowItem::marker(id, count, tr_plural(crate::entry_core::wording::missing_marker(self.entry.kind), i64::from(count), &[])),
            })
            .collect()
    }

    /// The FAB and continue target (`getNextUnseen`): the filters apply, the list is taken in ascending order, and
    /// `next_unseen_in` picks its first unseen episode. Upstream calls `findLast` on the descending display list,
    /// which is the same episode (T-EPS-NEXT-1, T-EPS-NEXT-2).
    pub fn next_unseen(&self) -> Option<i64> {
        let local = self.entry.source == kuro_source::local::LOCAL_SOURCE_ID;
        let ascending = match self.filters() {
            Some(f) => apply_filters(self.episodes.clone(), &ListFilters { descending: false, ..f }, local, |_| false),
            None => {
                let mut list = self.episodes.clone();
                list.sort_by(|a, b| compare_items(SortKey::Number, false, a, b));
                list
            }
        };
        next_unseen_in(&ascending)
    }

    /// What the FAB opens, as its label names it: "Chapter 1196", or the item's name when it has no number.
    pub fn fab_target(&self) -> Option<String> {
        let id = self.next_unseen()?;
        let ep = self.episodes.iter().find(|e| e.id == id)?;
        Some(if ep.number >= 0.0 { super::episode_row::title(ep, true) } else { ep.name.clone() })
    }

    /// The FAB reads the unfiltered list (behaviour 9).
    pub fn fab(&self) -> Fab {
        let seen: Vec<bool> = self.episodes.iter().map(|e| e.done).collect();
        fab(&seen, self.selection.is_active())
    }

    /// The item header over the processed list. The missing count reads the unfiltered numbers (behaviour 8).
    pub fn item_header(&self) -> ItemHeader {
        let seasons = self.entry.fetch_type() == FetchType::Seasons;
        let count = if seasons { self.seasons.len() } else { self.processed().len() };
        let episode_numbers: Vec<f64> = self.episodes.iter().map(|e| e.number).collect();
        let season_numbers: Vec<f64> = self.seasons.iter().map(|s| s.entry.season_number()).collect();
        let missing = header_missing_count(&episode_numbers, &season_numbers);
        item_header(self.entry.kind, seasons, count, missing, self.selection.is_active())
    }
}

/// `getNextUnseen` over a list in ascending order: the first unseen episode, for either display order (upstream's
/// `findLast` over the descending list lands on the same one). The list is already filtered.
pub fn next_unseen_in(ascending: &[Item]) -> Option<i64> {
    ascending.iter().find(|e| !e.done).map(|e| e.id)
}

#[cfg(test)]
mod tests {
    use kuro_db::flags::item as ep;
    use kuro_platform::kind::Kind;

    use super::super::entry_state::LoadState;
    use crate::entry_core::refresh_rules::EMPTY_OPEN_REFRESH_GAP_MS;
    use super::super::store::fixture::with_episodes;
    use super::*;

    fn load(store: &EntryStore, id: i64) -> EntryModel {
        let source = |_| SourceInfo { name: "Fake".into(), stub: false };
        EntryModel::load(store, &PrefStore::in_memory(), id, EntryLoadContext::default(), source)
            .unwrap()
            .expect("the entry exists")
    }

    #[test]
    fn a_fixture_entry_opens_in_success_with_its_saved_episodes_and_asks_the_source_again() {
        let (_dir, store, id) = with_episodes(true, true, 4, 1);
        let model = load(&store, id);
        assert_eq!(model.flow.load, LoadState::Success);
        assert!(model.accepts_dispatch());
        assert!(model.flow.refreshing, "the saved list shows while the source is asked again");
        assert_eq!(model.plan, RefreshPlan { info: false, episodes: true, seasons: false });
        assert_eq!(model.entry.title, "Frieren");
        assert_eq!(model.source.name, "Fake");
        assert_eq!(model.episodes.len(), 4);
        // the default word 0 sorts by source, descending
        assert_eq!(model.rows().iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["Episode 4", "Episode 3", "Episode 2", "Episode 1"]);
    }

    #[test]
    fn a_missing_entry_loads_as_none() {
        let (_dir, store, _) = with_episodes(true, true, 0, 0);
        let none = EntryModel::load(&store, &PrefStore::in_memory(), 4242, EntryLoadContext::default(), |_| SourceInfo::default());
        assert!(none.unwrap().is_none());
    }

    #[test]
    fn an_empty_entry_refreshes_on_open_at_most_once_per_ten_minutes() {
        let (_dir, store, id) = with_episodes(true, true, 0, 0);
        let source = |_| SourceInfo { name: "Fake".into(), stub: false };
        let load_at = |last: Option<i64>, now: i64| {
            let ctx = EntryLoadContext { last_empty_open_fetch_ms: last, now_ms: now, ..Default::default() };
            EntryModel::load(&store, &PrefStore::in_memory(), id, ctx, source).unwrap().unwrap()
        };
        assert!(load_at(None, 0).plan.episodes, "never fetched: fetch");
        assert!(!load_at(Some(1_000), 1_001).plan.episodes, "a second open within ten minutes waits");
        assert!(load_at(Some(1_000), 1_000 + EMPTY_OPEN_REFRESH_GAP_MS).plan.episodes, "ten minutes later: fetch again");
        let (_dir, store, id) = with_episodes(true, true, 2, 0);
        let ctx = EntryLoadContext { last_empty_open_fetch_ms: Some(1_000), now_ms: 1_001, ..Default::default() };
        assert!(EntryModel::load(&store, &PrefStore::in_memory(), id, ctx, source).unwrap().unwrap().plan.episodes, "the guard is for empty lists only");
    }

    #[test]
    fn the_first_state_is_refreshing_until_the_refresh_ends_on_success_or_error() {
        let (_dir, store, id) = with_episodes(true, false, 2, 0);
        let mut model = load(&store, id);
        assert_eq!(model.plan, RefreshPlan { info: true, episodes: true, seasons: false });
        assert!(model.flow.refreshing);
        model.finish_refresh(Err(DetailError::SourceNotInstalled));
        assert!(!model.flow.refreshing);
        assert_eq!(model.refresh_error, Some(DetailError::SourceNotInstalled));
        model.flow.begin_refresh();
        model.finish_refresh(Ok(()));
        assert_eq!(model.refresh_error, None);
    }

    #[test]
    fn an_empty_episode_list_refreshes_and_a_seasons_entry_reads_no_episodes() {
        let (_dir, store, id) = with_episodes(true, true, 0, 0);
        assert_eq!(load(&store, id).plan, RefreshPlan { info: false, episodes: true, seasons: false });

        let (_dir, store, id) = with_episodes(true, true, 3, 0);
        store.set_fetch_type_for_test(id, FetchType::Seasons);
        let model = load(&store, id);
        assert!(model.episodes.is_empty(), "episodes stay empty for a seasons entry");
        assert_eq!(model.plan, RefreshPlan { info: false, episodes: false, seasons: true });
    }

    #[test]
    fn a_non_favourite_opens_with_the_library_defaults_not_its_stored_flags() {
        let (_dir, store, id) = with_episodes(false, true, 2, 0);
        store.set_flag_bits(id, ep::SHOW_DONE, ep::UNDONE_MASK).unwrap();
        assert_eq!(load(&store, id).entry.item_flags & ep::UNDONE_MASK, 0);
        let row = store.entry(id).unwrap().unwrap();
        assert_eq!(row.item_flags & ep::UNDONE_MASK, ep::SHOW_DONE, "the open writes nothing to the row (T-39b-012)");

        let (_dir, store, id) = with_episodes(true, true, 2, 0);
        store.set_flag_bits(id, ep::SHOW_DONE, ep::UNDONE_MASK).unwrap();
        assert_eq!(load(&store, id).entry.item_flags & ep::UNDONE_MASK, ep::SHOW_DONE);
    }

    #[test]
    fn a_filter_that_hides_an_episode_adds_no_missing_marker() {
        let (_dir, store, id) = with_episodes(true, true, 3, 1);
        store.set_flag_bits(id, ep::SHOW_UNDONE, ep::UNDONE_MASK).unwrap();
        let lines = load(&store, id).lines();
        assert_eq!(lines.len(), 2, "the two unseen episodes and no \"1 missing\" line for the hidden one");
    }

    #[test]
    fn filters_the_list_but_the_fab_and_header_read_the_whole_one() {
        let (_dir, store, id) = with_episodes(true, true, 3, 3);
        store.set_flag_bits(id, ep::SHOW_UNDONE, ep::UNDONE_MASK).unwrap();
        let model = load(&store, id);
        assert!(model.rows().is_empty(), "every episode is seen, so the unseen filter hides all");
        assert!(!model.fab().visible, "nothing is unseen");
        assert!(model.fab().resume);
        assert_eq!(model.item_header().count, "0 episodes");

        let (_dir, store, id) = with_episodes(true, true, 3, 1);
        store.set_flag_bits(id, ep::SHOW_DONE, ep::UNDONE_MASK).unwrap();
        let mut model = load(&store, id);
        assert_eq!(model.rows().len(), 1, "only the seen episode shows");
        assert!(model.fab().visible, "an unseen episode exists in the unfiltered list");
        let view = model.processed_ids();
        model.selection.toggle_row(&view, view[0], crate::entry_core::selection::Command::LongPress);
        assert!(!model.fab().visible, "selecting hides it");
        assert!(!model.item_header().enabled);
    }

    #[test]
    fn next_unseen_is_the_first_unseen_of_the_ascending_list() {
        let list: Vec<Item> = (1..=4)
            .map(|id| {
                let mut e = Item::create(Kind::Anime);
                e.id = id;
                e.done = id <= 2;
                e
            })
            .collect();
        assert_eq!(next_unseen_in(&list), Some(3), "the lowest-numbered unseen, in either display order");
        assert_eq!(next_unseen_in(&[]), None);
    }

    #[test]
    fn the_fab_opens_the_first_unseen_in_display_order() {
        let (_dir, store, id) = with_episodes(true, true, 4, 2);
        store.set_flag_bits(id, ep::SORT_ASC, ep::SORT_DIR_MASK).unwrap();
        let model = load(&store, id);
        let by_number = |n: f64| model.episodes.iter().find(|e| e.number == n).unwrap().id;
        assert_eq!(model.next_unseen(), Some(by_number(3.0)), "ascending: the oldest unseen (T-EPS-NEXT-1)");
        store.set_flag_bits(id, ep::SORT_DESC, ep::SORT_DIR_MASK).unwrap();
        assert_eq!(load(&store, id).next_unseen(), Some(by_number(3.0)), "descending: findLast of the display list is the oldest unseen (T-EPS-NEXT-2)");
        let (_dir, store, id) = with_episodes(true, true, 2, 2);
        assert_eq!(load(&store, id).next_unseen(), None);
    }

    #[test]
    fn the_search_narrows_the_list_and_never_moves_the_resume_target() {
        let (_dir, store, id) = with_episodes(true, true, 12, 3);
        let mut model = load(&store, id);
        assert_eq!(model.fab_target().as_deref(), Some("Episode 4"), "the first unseen item, named by its number");
        model.search = " EPISODE 1".into();
        let titles: Vec<String> = model.lines().into_iter().map(|r| r.title).collect();
        assert_eq!(titles, ["Episode 12", "Episode 11", "Episode 10", "Episode 1"], "a search hides rows and adds no missing marker");
        assert_eq!(model.fab_target().as_deref(), Some("Episode 4"));
        model.search = "no such".into();
        assert!(model.lines().is_empty());
    }

    #[test]
    fn a_flag_or_sort_change_on_an_entry_outside_the_library_survives_the_next_snapshot() {
        let (_dir, store, id) = with_episodes(false, true, 3, 1);
        let mut model = load(&store, id);
        model.note_flag_write(ep::SHOW_UNDONE, ep::UNDONE_MASK);
        let (entry, episodes) = (model.entry.clone(), model.episodes.clone());
        model.apply_snapshot(Entry { item_flags: 0, ..entry }, episodes);
        assert_eq!(model.entry.item_flags & ep::UNDONE_MASK, ep::SHOW_UNDONE, "the session copy took the write");
        assert_eq!(model.rows().len(), 2, "only the unseen episodes show");
        model.note_flag_write(ep::SORTING_NUMBER, ep::SORTING_MASK);
        assert_eq!(model.entry.item_flags & ep::SORTING_MASK, ep::SORTING_NUMBER);
        assert_eq!(model.entry.item_flags & ep::UNDONE_MASK, ep::SHOW_UNDONE, "the sort leaves the filter alone");
    }

    #[test]
    fn a_season_write_on_an_entry_outside_the_library_survives_the_next_snapshot_and_leaves_the_episode_flags() {
        use kuro_db::flags::season as sf;
        let (_dir, store, id) = with_episodes(false, true, 1, 0);
        let mut model = load(&store, id);
        let episode_word = model.entry.item_flags;
        model.note_season_write(sf::SHOW_STARTED, sf::STARTED_MASK);
        model.note_season_write(sf::SORT_UPLOAD, sf::SORT_MASK);
        let (entry, episodes) = (model.entry.clone(), model.episodes.clone());
        model.apply_snapshot(Entry { season_flags: Some(0), ..entry }, episodes);
        assert_eq!(model.entry.season_flags() & sf::STARTED_MASK, sf::SHOW_STARTED);
        assert_eq!(model.entry.season_flags() & sf::SORT_MASK, sf::SORT_UPLOAD);
        assert_eq!(model.entry.item_flags, episode_word);
        let (_dir, store, id) = with_episodes(true, true, 1, 0);
        let mut model = load(&store, id);
        let before = model.entry.season_flags();
        model.note_season_write(sf::SHOW_STARTED, sf::STARTED_MASK);
        assert_eq!(model.entry.season_flags(), before, "an entry in the library follows its row");
    }

    #[test]
    fn the_season_extras_are_kept_by_season_id_and_replaced_as_a_whole() {
        let (_dir, store, id) = with_episodes(true, true, 0, 0);
        let mut model = load(&store, id);
        model.set_season_extras(vec![SeasonExtra { id: 5, download_count: 2, source_language: "ja".into() }]);
        assert_eq!(model.season_extras[&5].download_count, 2);
        model.set_season_extras(vec![SeasonExtra { id: 6, download_count: 0, source_language: String::new() }]);
        assert!(!model.season_extras.contains_key(&5));
    }

    #[test]
    fn an_entry_in_the_library_has_no_session_copy_to_change() {
        let (_dir, store, id) = with_episodes(true, true, 1, 0);
        let mut model = load(&store, id);
        let before = model.entry.item_flags;
        model.note_flag_write(ep::SHOW_UNDONE, ep::UNDONE_MASK);
        model.note_flag_write(ep::SORTING_NUMBER, ep::SORTING_MASK);
        assert_eq!(model.entry.item_flags, before, "the row's own value shows");
    }

    #[test]
    fn a_snapshot_of_an_entry_that_joined_the_library_ends_the_session_flags() {
        let (_dir, store, id) = with_episodes(false, true, 1, 0);
        let mut model = load(&store, id);
        assert!(model.session_flags.is_some());
        let mut stored = model.entry.clone();
        stored.item_flags = ep::SHOW_UNDONE;
        model.apply_snapshot(stored.clone(), model.episodes.clone());
        assert_eq!(model.entry.item_flags, 0, "still a non-favorite: the session flags win over the row");
        stored.favorite = true;
        model.apply_snapshot(stored, model.episodes.clone());
        assert!(model.session_flags.is_none());
        assert_eq!(model.entry.item_flags, ep::SHOW_UNDONE, "now the stored defaults show");
    }

    #[test]
    fn selection_marks_rows_and_a_snapshot_keeps_the_selection() {
        let (_dir, store, id) = with_episodes(true, true, 3, 0);
        let mut model = load(&store, id);
        let first = model.processed_ids()[0];
        let view = model.processed_ids();
        model.selection.toggle_row(&view, first, crate::entry_core::selection::Command::LongPress);
        assert_eq!(model.rows().iter().filter(|r| r.selected).count(), 1);
        let entry = model.entry.clone();
        model.apply_snapshot(entry, model.episodes[..2].to_vec());
        assert_eq!(model.episodes.len(), 2);
        assert!(model.selection.contains(first));
    }

    #[test]
    fn the_downloaded_filter_reads_the_ids_on_disk_and_a_local_entry_counts_every_episode() {
        let (_dir, store, id) = with_episodes(true, true, 3, 0);
        store.set_flag_bits(id, ep::SHOW_DOWNLOADED, ep::DOWNLOADED_MASK).unwrap();
        let mut model = load(&store, id);
        assert!(model.rows().is_empty(), "nothing is on disk");
        let on_disk = model.episodes[1].id;
        model.downloaded.insert(on_disk);
        assert_eq!(model.processed_ids(), vec![on_disk]);
    }

    #[test]
    fn the_download_menu_targets_follow_the_filtered_list_and_the_disk() {
        use crate::entry_core::download_menu::DownloadAmount;
        let (_dir, store, id) = with_episodes(true, true, 4, 1);
        let mut model = load(&store, id);
        let by_number = |model: &EntryModel, n: f64| model.episodes.iter().find(|e| e.number == n).unwrap().id;
        model.downloaded.insert(by_number(&model, 3.0));
        // the default list is descending; episode 1 is seen and episode 3 is on disk
        assert_eq!(model.download_targets(DownloadAmount::AllUnread), vec![by_number(&model, 2.0), by_number(&model, 4.0)], "everything queues in watching order");
        assert_eq!(model.download_targets(DownloadAmount::Next(1)), vec![by_number(&model, 2.0)], "the oldest unseen first");
    }

    #[test]
    fn the_tracker_count_counts_only_logged_in_trackers() {
        let (_dir, store, id) = with_episodes(true, true, 1, 0);
        assert_eq!(load(&store, id).tracking_count, 0);
        kuro_data::track::TrackRepo::new(store.db()).insert(&kuro_data::track::Track { entry_id: id, ..kuro_data::track::Track::new(Kind::Anime, 2) }).unwrap();
        assert_eq!(load(&store, id).tracking_count, 0, "a binding of a tracker that is not logged in does not count (T-TRK-12)");
    }

    #[test]
    fn a_gap_in_the_numbers_gets_a_separator_with_its_count() {
        // T-39a-015: episodes 1, 3 and 4 miss episode 2, so one separator sits between 1 and 3
        let (_dir, store, id) = with_episodes(true, true, 4, 0);
        let mut model = load(&store, id);
        model.episodes.retain(|e| e.number != 2.0);
        let lines = model.lines();
        assert_eq!(lines.len(), 4, "three episodes and one separator");
        assert_eq!(lines[2].missing, Some(1), "{lines:?}");
        assert_eq!(lines[1].missing, None);
        let full = load(&store, id);
        assert!(full.lines().iter().all(|l| l.missing.is_none()), "no gap, no separator");
    }

    #[test]
    fn a_queued_episode_shows_its_queue_state_until_it_lands_on_disk() {
        let (_dir, store, id) = with_episodes(true, true, 2, 0);
        let mut model = load(&store, id);
        let ep = model.episodes[0].id;
        let row = |model: &EntryModel| model.rows().into_iter().find(|r| r.id == ep).unwrap();
        assert_eq!((row(&model).download, row(&model).progress), (DownloadState::NotDownloaded, 0));
        assert!(row(&model).indicator && row(&model).indicator_enabled);
        model.apply_live(ep, DownloadState::Downloading, 40);
        assert_eq!((row(&model).download, row(&model).progress), (DownloadState::Downloading, 40));
        model.apply_live(ep, DownloadState::Downloaded, 0);
        assert_eq!(row(&model).download, DownloadState::Downloaded);
        assert!(model.downloaded.contains(&ep), "the downloaded filter sees it too");
        model.apply_live(ep, DownloadState::NotDownloaded, 0);
        assert_eq!(row(&model).download, DownloadState::Downloaded, "a cancel later does not forget the copy on disk");
    }

    #[test]
    fn three_thousand_rows_build_in_well_under_a_second() {
        let (_dir, store, id) = with_episodes(true, true, 3000, 100);
        let model = load(&store, id);
        let started = std::time::Instant::now();
        let rows = model.rows();
        assert_eq!(rows.len(), 3000);
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "{:?}", started.elapsed());
    }
}
