//! The global search engine (T-40b-060, T-40b-061, T-40b-062, T-40b-063, T-40b-064, T-40b-065, T-40b-067). The query and
//! source-filter state, the plan a search starts, the results applied by one writer, their order, the single-result
//! auto-open and the entry routes. Pure: the page runs the fetches (at most `MAX_IN_FLIGHT` at once) and calls `apply`
//! on the GTK thread, so no two results write the rows at the same time (GS-EC-05). Ports `MangaSearchScreenModel` and
//! its subclasses.

use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, OnceLock};

use kuro_domain::source::browse::BrowseQuery;
use kuro_domain::source::model::Source;

use super::listing::ListItem;
use super::pages::{GlobalSearchSnapshot, SearchRow, SearchState, SourceFilter};
#[cfg(test)]
use super::pages::{SearchChips, visible_rows};
use super::route::BrowseRoute;
use kuro_platform::kind::Kind;

/// Source fetches that run at once (R§5: a 5-permit semaphore, held only for the source call).
pub const MAX_IN_FLIGHT: usize = 5;

/// The gate every global search screen shares (E-GS-08): at most `MAX_IN_FLIGHT` source fetches run at once across all of
/// them, not per screen.
pub fn shared_gate() -> Arc<tokio::sync::Semaphore> {
    static GATE: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    Arc::clone(GATE.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT))))
}

/// A catalogue source offered to a search, with the extension package it came from (`""` when it has none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub source: Source,
    pub package: String,
}

/// The fetches one search starts: `sources` are the rows still loading, in row order, under `generation`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchPlan {
    pub generation: u64,
    pub sources: Vec<Source>,
}

/// The global search state of one page. `candidates` and the enabled and hidden sets are the snapshot taken at creation
/// (B§1.1): a later preference change does not reach an open search.
#[derive(Debug)]
pub struct GlobalSearch {
    candidates: Vec<Candidate>,
    enabled_languages: BTreeSet<String>,
    hidden: BTreeSet<i64>,
    /// The source the search started from (a migrate search). Its row sorts first.
    origin: Option<i64>,
    /// The extension package the search is limited to; `None` and `""` both mean no restriction.
    extension: Option<String>,
    query: Option<String>,
    last: Option<(String, SourceFilter)>,
    filter: SourceFilter,
    only_has_results: bool,
    generation: u64,
    rows: Vec<SearchRow>,
}

impl GlobalSearch {
    /// A new search: every enabled source, the has-results toggle as the caller gives it, no results until a search runs.
    pub fn new(
        candidates: Vec<Candidate>,
        enabled_languages: BTreeSet<String>,
        hidden: BTreeSet<i64>,
        origin: Option<i64>,
        only_has_results: bool,
    ) -> Self {
        Self {
            candidates,
            enabled_languages,
            hidden,
            origin,
            extension: None,
            query: None,
            last: None,
            filter: SourceFilter::All,
            only_has_results,
            generation: 0,
            rows: Vec::new(),
        }
    }

    /// The search box text (`updateSearchQuery`): state only, nothing is searched.
    pub fn set_query(&mut self, query: Option<&str>) {
        self.query = query.map(str::to_owned);
    }

    /// The Pinned and All chips (`setSourceFilter`): stores the filter and searches again with the same query.
    pub fn set_filter(&mut self, filter: SourceFilter) -> Option<SearchPlan> {
        self.filter = filter;
        self.search()
    }

    /// The has-results toggle. It filters the rows that are already here and runs no search.
    pub fn set_only_has_results(&mut self, only: bool) {
        self.only_has_results = only;
    }

    /// An intent or deep link (`eu.kanade.tachiyomi.SEARCH`): sets the query and the extension filter, then searches.
    /// Any extension filter, even an empty one, forces `All` (GS-EC-11).
    pub fn request(&mut self, query: &str, extension: Option<&str>) -> Option<SearchPlan> {
        self.query = Some(query.to_owned());
        if let Some(package) = extension {
            self.extension = (!package.is_empty()).then(|| package.to_owned());
            self.filter = SourceFilter::All;
        }
        self.search()
    }

    /// `search()`: a blank query, or the same query with the same filter, is a no-op. A new query resets every row to
    /// Loading; the same query keeps the finished rows of sources still selected and fetches only the rest.
    pub fn search(&mut self) -> Option<SearchPlan> {
        let query = self.query.clone().unwrap_or_default();
        if query.trim().is_empty() {
            return None;
        }
        let same_query = self.last.as_ref().is_some_and(|(last, _)| *last == query);
        if same_query && self.last.as_ref().is_some_and(|(_, filter)| *filter == self.filter) {
            return None;
        }
        self.last = Some((query, self.filter));
        self.generation += 1;
        let previous = std::mem::take(&mut self.rows);
        self.rows = self
            .selected()
            .into_iter()
            .map(|source| {
                let state = match previous.iter().find(|row| row.source.id == source.id) {
                    Some(row) if same_query && !row.state.waiting() => row.state.clone(),
                    _ => SearchState::Loading,
                };
                let origin = Some(source.id) == self.origin;
                SearchRow { source, state, origin }
            })
            .collect();
        self.sort_rows();
        let sources = self.rows.iter().filter(|row| row.state.waiting()).map(|row| row.source.clone()).collect();
        Some(SearchPlan { generation: self.generation, sources })
    }

    /// The sources a search selects, in the order a stable sort keeps: enabled sources, then by name. Every enabled source
    /// is searched; an extension filter keeps that package's sources (GS-EC-12 gives an empty list when it has none).
    fn selected(&self) -> Vec<Source> {
        let mut enabled: Vec<&Candidate> = self
            .candidates
            .iter()
            .filter(|c| self.enabled_languages.contains(&c.source.lang) && !self.hidden.contains(&c.source.id))
            .collect();
        enabled.sort_by(|a, b| name_key(&a.source).cmp(&name_key(&b.source)));
        enabled
            .into_iter()
            .filter(|c| match (&self.extension, self.filter) {
                (Some(package), _) => c.package == *package,
                (None, SourceFilter::All) => true,
            })
            .map(|c| c.source.clone())
            .collect()
    }

    /// `apply(generation, source, result)`: one result for one row. A result from an older generation, or for a row that is
    /// not Loading, is dropped and returns false. A failure becomes an Error row (GS-EC-07, GS-EC-08).
    pub fn apply(&mut self, generation: u64, source_id: i64, result: Result<Vec<ListItem>, String>) -> bool {
        if generation != self.generation {
            return false;
        }
        let Some(row) = self.rows.iter_mut().find(|row| row.source.id == source_id) else { return false };
        if !row.state.waiting() {
            return false;
        }
        // a source that fails keeps the saved results it showed
        row.state = match (result, std::mem::replace(&mut row.state, SearchState::Loading)) {
            (Ok(items), _) => SearchState::Results(items),
            (Err(_), SearchState::Refreshing(saved)) => SearchState::Results(saved),
            (Err(message), _) => SearchState::Error(message),
        };
        self.sort_rows();
        true
    }

    /// Shows the saved results of the last same search on a row that is still loading (`results_cache`). False when the
    /// plan is old, the row is gone or answered, or nothing was saved.
    pub fn prefill(&mut self, generation: u64, source_id: i64, items: Vec<ListItem>) -> bool {
        if generation != self.generation || items.is_empty() {
            return false;
        }
        let Some(row) = self.rows.iter_mut().find(|row| row.source.id == source_id && matches!(row.state, SearchState::Loading)) else { return false };
        row.state = SearchState::Refreshing(items);
        self.sort_rows();
        true
    }

    #[cfg(test)]
    /// Whether a fetch of `generation` is still the current search (a task of an older one is aborted).
    pub fn is_current(&self, generation: u64) -> bool {
        generation == self.generation
    }

    /// The order of the result map (B§9.5, D-GS-01): the origin first, then non-empty results, then
    /// `"name (lang)"` in UTF-16 order. The sort is stable, so two sources with equal keys are both kept (GS-EC-04).
    fn sort_rows(&mut self) {
        self.rows.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
    }

    /// `(answered, total)`: a row answers once it is no longer Loading, an error included.
    pub fn progress(&self) -> (usize, usize) {
        (self.rows.iter().filter(|row| !row.state.waiting()).count(), self.rows.len())
    }

    #[cfg(test)]
    /// The rows the chips leave (`visible_rows`), in result order.
    pub fn visible(&self) -> Vec<&SearchRow> {
        visible_rows(&self.rows, SearchChips { filter: self.filter, only_has_results: self.only_has_results })
    }

    pub fn rows(&self) -> &[SearchRow] {
        &self.rows
    }

    #[cfg(test)]
    pub fn filter(&self) -> SourceFilter {
        self.filter
    }

    /// What the page draws from: the query as typed, the progress and the rows.
    pub fn snapshot(&self) -> GlobalSearchSnapshot {
        let (answered, total) = self.progress();
        GlobalSearchSnapshot { query: self.query.clone().unwrap_or_default(), answered, total, rows: self.rows.clone() }
    }
}

/// `"name (lang)"` lowercased, the name key of the order and the enabled list.
fn name_key(source: &Source) -> Vec<u16> {
    format!("{} ({})", source.name.to_lowercase(), source.lang).encode_utf16().collect()
}

/// The sort key of one row: origin, then a non-empty success, then the name key. `false` sorts first.
fn sort_key(row: &SearchRow) -> (bool, bool, Vec<u16>) {
    let non_empty = row.state.items().is_some_and(|items| !items.is_empty());
    (!row.origin, !non_empty, name_key(&row.source))
}

/// The page-level rule of the single-result auto-open (T-40b-064, GS-EC-13, GS-EC-14).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutoOpen {
    /// Still loading: the page shows a loading state.
    Wait,
    /// Exactly one manga came back: the page is replaced by its entry (library id).
    Open(i64),
    /// Anything else: the results page.
    Show,
}

/// Decides the auto-open. It runs only for a query with an extension filter and exactly one source; every other case
/// shows the results.
pub fn auto_open(query: &str, extension: &str, total_sources: usize, rows: &[SearchRow]) -> AutoOpen {
    if query.trim().is_empty() || extension.is_empty() || total_sources != 1 {
        return AutoOpen::Show;
    }
    match rows {
        [] => AutoOpen::Wait,
        [row] => match &row.state {
            SearchState::Loading | SearchState::Refreshing(_) => AutoOpen::Wait,
            SearchState::Results(items) if items.len() == 1 => AutoOpen::Open(items[0].id),
            _ => AutoOpen::Show,
        },
        _ => AutoOpen::Show,
    }
}

/// The same fallback for either catalogue: an anime entry page's search that no browse page can take opens the anime
/// global search.
pub fn deep_link_fallback_for(kind: Kind, query: &str) -> BrowseRoute {
    BrowseRoute::GlobalSearch { kind, source_id: None, query: query.to_owned() }
}

/// A source row tapped while the search box holds text that was never submitted: the source's browse page opens with that
/// text (GS-EC-15, a documented quirk).
pub fn row_text_route(kind: Kind, source_id: i64, typed: &str) -> BrowseRoute {
    BrowseRoute::BrowseSource { kind, source_id, query: BrowseQuery::Search(typed.to_owned()), migrating: false }
}

/// The start of a migrate search (T-40b-067): the origin source and the title the search starts from. A missing manga
/// is an error the page shows, not a panic (E-MG-02). A blank title gives no query, so nothing is searched (E-MG-03).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrateStart {
    Missing,
    Ready { origin: i64, title: String },
}

pub fn migrate_start(manga: Option<(i64, String)>) -> MigrateStart {
    match manga {
        None => MigrateStart::Missing,
        Some((origin, title)) => MigrateStart::Ready { origin, title },
    }
}

/// The fetches a search can start at once: takes up to `MAX_IN_FLIGHT - running` sources off the front of the queue.
pub fn take_starts(queue: &mut VecDeque<Source>, running: usize) -> Vec<Source> {
    let room = MAX_IN_FLIGHT.saturating_sub(running);
    (0..room).map_while(|_| queue.pop_front()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: i64, name: &str, lang: &str, package: &str) -> Candidate {
        let source = Source::new(id, name, lang);
        Candidate { source, package: package.to_owned() }
    }

    fn langs(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn item(id: i64) -> ListItem {
        ListItem { id, title: format!("M{id}"), ..ListItem::default() }
    }

    /// Sources A (en), B (en), C (ja) with the extension "ext.a" owning A and B.
    fn engine() -> GlobalSearch {
        GlobalSearch::new(
            vec![cand(1, "A", "en", "ext.a"), cand(2, "B", "en", "ext.a"), cand(3, "C", "ja", "ext.c")],
            langs(&["en", "ja"]),
            BTreeSet::new(),
            None,
            false,
        )
    }

    fn ids(sources: &[Source]) -> Vec<i64> {
        sources.iter().map(|s| s.id).collect()
    }

    #[test]
    fn saved_results_show_while_loading_and_the_answer_or_a_failure_settles_the_row() {
        let mut search = engine();
        search.set_query(Some("one"));
        let plan = search.search().unwrap();
        assert!(search.prefill(plan.generation, 1, vec![item(7)]));
        assert!(search.prefill(plan.generation, 2, vec![item(8)]));
        assert!(!search.prefill(plan.generation, 3, Vec::new()), "nothing saved: the row keeps loading");
        assert_eq!(search.progress(), (0, 3), "a saved row still waits for its source");
        assert!(search.apply(plan.generation, 1, Ok(vec![item(9)])));
        assert!(search.apply(plan.generation, 2, Err("offline".into())));
        let state = |id: i64| search.rows().iter().find(|row| row.source.id == id).unwrap().state.clone();
        assert_eq!(state(1), SearchState::Results(vec![item(9)]), "the answer replaces the saved results");
        assert_eq!(state(2), SearchState::Results(vec![item(8)]), "a failure keeps them");
        assert!(!search.prefill(plan.generation, 1, vec![item(7)]), "an answered row is not overwritten");
    }

    #[test]
    fn a_new_search_defaults_to_all_and_selects_every_enabled_source() {
        let mut search = GlobalSearch::new(
            vec![cand(1, "A", "en", "ext.a"), cand(2, "B", "en", "ext.a"), cand(3, "C", "ja", "ext.c")],
            langs(&["en", "ja"]),
            BTreeSet::new(),
            None,
            false,
        );
        search.set_query(Some("one"));
        let plan = search.search().expect("a search starts");
        assert_eq!(search.filter(), SourceFilter::All, "every source is searched");
        assert_eq!(ids(&plan.sources), [1, 2, 3], "every enabled source, by name");
        assert_eq!(plan.generation, 1);
    }

    #[test]
    fn a_search_with_no_enabled_language_selects_nothing_and_has_no_rows() {
        let mut search = GlobalSearch::new(vec![cand(2, "B", "en", "")], langs(&["fr"]), BTreeSet::new(), None, false);
        search.set_query(Some("one"));
        let plan = search.search().expect("the search runs with nothing to fetch");
        assert!(plan.sources.is_empty(), "the language filter decides: a language that is off is not searched");
        assert!(search.rows().is_empty());
    }

    #[test]
    fn blank_query_and_same_query_with_same_filter_are_no_ops() {
        let mut search = engine();
        search.set_query(Some("  "));
        assert_eq!(search.search(), None, "GS-EC-01 and V1: a blank query is a no-op");
        search.set_query(Some("one"));
        assert!(search.search().is_some());
        assert_eq!(search.search(), None, "GS-EC-02: the same query and filter do nothing");
    }

    #[test]
    fn a_filter_change_with_the_same_query_fetches_only_the_newly_selected_sources() {
        let mut search = engine();
        search.set_query(Some("one"));
        let plan = search.search().expect("first search");
        assert!(search.apply(plan.generation, 1, Ok(vec![item(10), item(11)])));
        assert_eq!(search.set_filter(SourceFilter::All), None, "every source was already searched");
        let kept = search.rows().iter().find(|r| r.source.id == 1).expect("A stays");
        assert_eq!(kept.state, SearchState::Results(vec![item(10), item(11)]), "the finished result of A is kept");
    }

    #[test]
    fn a_new_query_resets_every_row_to_loading() {
        let mut search = engine();
        search.set_query(Some("one"));
        let first = search.search().expect("first");
        search.apply(first.generation, 1, Ok(vec![item(10)]));
        search.set_query(Some("two"));
        let second = search.search().expect("new query");
        assert_eq!(ids(&second.sources), [1, 2, 3], "every source, by name");
        assert!(search.rows().iter().all(|r| r.state == SearchState::Loading));
    }

    #[test]
    fn a_failed_source_is_an_error_row_and_does_not_retry_on_the_same_query() {
        let mut search = engine();
        search.set_query(Some("one"));
        let plan = search.search().expect("search");
        assert!(search.apply(plan.generation, 1, Err("timeout".into())));
        assert_eq!(search.search(), None, "GS-EC-07: an errored source is not retried without a change");
        assert_eq!(search.progress(), (1, 3));
    }

    #[test]
    fn results_from_an_older_generation_or_for_an_answered_row_are_dropped() {
        let mut search = engine();
        search.set_query(Some("one"));
        let old = search.search().expect("gen 1");
        search.set_query(Some("two"));
        let new = search.search().expect("gen 2");
        assert!(!search.apply(old.generation, 1, Ok(vec![item(1)])), "V4: an old generation changes nothing");
        assert!(search.apply(new.generation, 1, Ok(vec![item(2)])));
        assert!(!search.apply(new.generation, 1, Ok(vec![item(3)])), "V5: a row that answered is not changed again");
        assert!(!search.is_current(old.generation));
        assert!(search.is_current(new.generation));
    }

    #[test]
    fn rows_sort_by_origin_then_non_empty_then_name() {
        // V6 from the spec: Zed (empty), Alpha (ja, 2), Beta (loading), Gamma (1), Delta (error).
        let mut search = GlobalSearch::new(
            vec![
                cand(1, "Zed", "en", ""),
                cand(2, "Alpha", "ja", ""),
                cand(3, "Beta", "en", ""),
                cand(4, "Gamma", "en", ""),
                cand(5, "Delta", "en", ""),
            ],
            langs(&["en", "ja"]),
            BTreeSet::new(),
            None,
            false,
        );
        search.set_query(Some("x"));
        let plan = search.set_filter(SourceFilter::All).expect("search");
        assert_eq!(plan.sources.len(), 5);
        search.apply(plan.generation, 1, Ok(vec![]));
        search.apply(plan.generation, 2, Ok(vec![item(20), item(21)]));
        search.apply(plan.generation, 4, Ok(vec![item(40)]));
        search.apply(plan.generation, 5, Err("boom".into()));
        let names: Vec<&str> = search.rows().iter().map(|r| r.source.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "Gamma", "Beta", "Delta", "Zed"]);
    }

    #[test]
    fn two_sources_with_equal_keys_are_both_kept_in_insertion_order() {
        let mut search = GlobalSearch::new(
            vec![cand(1, "MangaX", "en", "p1"), cand(2, "MangaX", "en", "p2")],
            langs(&["en"]),
            BTreeSet::new(),
            None,
            false,
        );
        search.set_query(Some("x"));
        let plan = search.set_filter(SourceFilter::All).expect("search");
        assert_eq!(ids(&plan.sources), [1, 2], "GS-EC-04: no collision drops a source");
        assert_eq!(search.progress(), (0, 2));
    }

    #[test]
    fn only_has_results_filters_to_non_empty_successes_and_runs_no_search() {
        let mut search = GlobalSearch::new(
            vec![cand(1, "L", "en", ""), cand(2, "E", "en", ""), cand(3, "Err", "en", ""), cand(4, "One", "en", "")],
            langs(&["en"]),
            BTreeSet::new(),
            None,
            false,
        );
        search.set_query(Some("x"));
        let plan = search.search().expect("search");
        search.apply(plan.generation, 2, Ok(vec![]));
        search.apply(plan.generation, 3, Err("x".into()));
        search.apply(plan.generation, 4, Ok(vec![item(9)]));
        assert_eq!(search.visible().len(), 4, "toggle off shows every row");
        search.set_only_has_results(true);
        let visible: Vec<i64> = search.visible().iter().map(|r| r.source.id).collect();
        assert_eq!(visible, [4], "V8: only the non-empty success is visible");
        assert_eq!(search.search(), None, "the toggle runs no search");
    }

    #[test]
    fn progress_counts_answered_rows_of_all_rows() {
        let mut search = GlobalSearch::new(
            (1..=4).map(|i| cand(i, &format!("S{i}"), "en", "")).collect(),
            langs(&["en"]),
            BTreeSet::new(),
            None,
            false,
        );
        search.set_query(Some("x"));
        let plan = search.search().expect("search");
        search.apply(plan.generation, 1, Ok(vec![]));
        search.apply(plan.generation, 2, Ok(vec![item(1)]));
        search.apply(plan.generation, 3, Err("e".into()));
        assert_eq!(search.progress(), (3, 4), "V9 shape: 3 answered of 4");
        assert_eq!(search.snapshot().answered, 3);
        assert_eq!(search.snapshot().query, "x");
    }

    #[test]
    fn an_extension_filter_forces_all_and_selects_only_that_packages_enabled_sources() {
        let mut search = engine();
        let plan = search.request("one", Some("ext.a")).expect("the intent searches");
        assert_eq!(search.filter(), SourceFilter::All, "GS-EC-11: the extension filter forces All");
        assert_eq!(ids(&plan.sources), [1, 2], "the package's sources, by name");
    }

    #[test]
    fn an_empty_extension_filter_forces_all_but_restricts_nothing() {
        let mut search = engine();
        let plan = search.request("one", Some("")).expect("search");
        assert_eq!(search.filter(), SourceFilter::All);
        assert_eq!(ids(&plan.sources), [1, 2, 3], "by name");
    }

    #[test]
    fn an_extension_with_no_enabled_sources_gives_an_empty_selection() {
        let mut search = engine();
        let plan = search.request("one", Some("ext.none")).expect("search");
        assert!(plan.sources.is_empty(), "GS-EC-12");
    }

    #[test]
    fn hidden_and_disabled_languages_are_never_selected() {
        let mut search = GlobalSearch::new(
            vec![cand(1, "A", "en", ""), cand(2, "B", "ja", ""), cand(3, "C", "en", "")],
            langs(&["en"]),
            BTreeSet::from([3]),
            None,
            false,
        );
        let plan = search.request("x", Some("")).expect("search");
        assert_eq!(ids(&plan.sources), [1], "a disabled language and a hidden source are both left out");
    }

    #[test]
    fn the_origin_source_sorts_first_for_a_migrate_search() {
        let mut search = GlobalSearch::new(
            vec![cand(1, "Alpha", "en", ""), cand(2, "Zulu", "en", "")],
            langs(&["en"]),
            BTreeSet::new(),
            Some(2),
            false,
        );
        search.set_query(Some("x"));
        let plan = search.set_filter(SourceFilter::All).expect("search");
        search.apply(plan.generation, 1, Ok(vec![item(1)]));
        search.apply(plan.generation, 2, Ok(vec![]));
        let names: Vec<&str> = search.rows().iter().map(|r| r.source.name.as_str()).collect();
        assert_eq!(names, ["Zulu", "Alpha"], "E-MG-01: the origin first, even with no results");
        assert!(search.rows()[0].origin);
    }

    #[test]
    fn the_auto_open_waits_then_opens_one_manga_and_shows_everything_else() {
        let loading = SearchRow { source: Source::new(1, "A", "en"), state: SearchState::Loading, origin: false };
        let one = SearchRow { source: Source::new(1, "A", "en"), state: SearchState::Results(vec![item(42)]), origin: false };
        let many = SearchRow { source: Source::new(1, "A", "en"), state: SearchState::Results(vec![item(1), item(2)]), origin: false };
        let none = SearchRow { source: Source::new(1, "A", "en"), state: SearchState::Results(vec![]), origin: false };
        let error = SearchRow { source: Source::new(1, "A", "en"), state: SearchState::Error("x".into()), origin: false };
        assert_eq!(auto_open("q", "ext", 1, std::slice::from_ref(&loading)), AutoOpen::Wait);
        assert_eq!(auto_open("q", "ext", 1, std::slice::from_ref(&one)), AutoOpen::Open(42), "GS-EC-13");
        assert_eq!(auto_open("q", "ext", 1, std::slice::from_ref(&many)), AutoOpen::Show);
        assert_eq!(auto_open("q", "ext", 1, std::slice::from_ref(&none)), AutoOpen::Show);
        assert_eq!(auto_open("q", "ext", 1, std::slice::from_ref(&error)), AutoOpen::Show, "GS-EC-14");
        assert_eq!(auto_open("q", "", 1, std::slice::from_ref(&one)), AutoOpen::Show, "no extension filter: no auto-open");
        assert_eq!(auto_open("q", "ext", 2, std::slice::from_ref(&one)), AutoOpen::Show, "more than one source: results");
        assert_eq!(auto_open(" ", "ext", 1, std::slice::from_ref(&one)), AutoOpen::Show, "a blank query: results");
    }

    #[test]
    fn a_migrate_start_without_its_manga_is_an_error_and_a_blank_title_searches_nothing() {
        assert_eq!(migrate_start(None), MigrateStart::Missing, "E-MG-02: no panic");
        assert_eq!(migrate_start(Some((5, "T".into()))), MigrateStart::Ready { origin: 5, title: "T".into() });
        let mut search = engine();
        search.set_query(Some(""));
        assert_eq!(search.search(), None, "E-MG-03: an empty title does not search");
    }

    #[test]
    fn at_most_five_fetches_start_at_once() {
        let mut queue: VecDeque<Source> = (1..=7).map(|i| Source::new(i, "S", "en")).collect();
        assert_eq!(take_starts(&mut queue, 0).len(), MAX_IN_FLIGHT);
        assert_eq!(queue.len(), 2);
        assert!(take_starts(&mut queue, MAX_IN_FLIGHT).is_empty(), "no room while five run");
        assert_eq!(take_starts(&mut queue, 4).len(), 1, "one slot frees, one starts");
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn a_new_search_restarts_the_generation_and_keeps_the_origin_flag_only_for_the_origin() {
        let mut search = engine();
        search.set_query(Some("one"));
        assert_eq!(search.search().map(|p| p.generation), Some(1));
        search.set_query(Some("two"));
        assert_eq!(search.search().map(|p| p.generation), Some(2));
        assert!(search.rows().iter().all(|r| !r.origin), "no origin outside a migrate search");
    }
}

#[cfg(test)]
mod props {
    use super::*;
    use proptest::prelude::*;

    fn item(id: i64) -> ListItem {
        ListItem { id, title: format!("M{id}"), ..ListItem::default() }
    }

    proptest! {
        /// `sort_items` of the plan: the row order is a function of the rows, so sorting again changes nothing, and neither
        /// `apply` (which sorts) nor a stale or repeated result ever adds or drops a row or an item (T-40b-084).
        #[test]
        fn the_row_sort_is_idempotent_and_apply_never_changes_the_row_or_item_count(
            specs in proptest::collection::vec((0usize..4, any::<bool>()), 1..8),
            ops in proptest::collection::vec((0i64..9, 0u64..3, 0u8..3, 0usize..4), 0..14),
        ) {
            let names = ["Alpha", "Beta", "alpha", "Same"];
            let candidates: Vec<Candidate> = specs
                .iter()
                .enumerate()
                .map(|(at, (name, _))| {
                    let source = Source::new(at as i64 + 1, names[*name], "en");
                    Candidate { source, package: String::new() }
                })
                .collect();
            let mut search = GlobalSearch::new(candidates, ["en".to_owned()].into(), BTreeSet::new(), None, false);
            search.set_query(Some("q"));
            let plan = search.set_filter(SourceFilter::All).expect("a search");
            let total = search.rows().len();
            prop_assert_eq!(total, specs.len(), "every enabled source has a row, equal names included");
            for (source_id, offset, kind, count) in ops {
                let generation = plan.generation + offset - 1;
                let sent: Result<Vec<ListItem>, String> = match kind {
                    0 => Ok((0..count as i64).map(item).collect()),
                    1 => Ok(vec![item(1)]),
                    _ => Err("offline".to_owned()),
                };
                let sent_items = sent.as_ref().ok().map(Vec::len);
                let applied = search.apply(generation, source_id, sent);
                prop_assert_eq!(search.rows().len(), total, "apply never changes the number of rows");
                if let (true, Some(sent_items)) = (applied, sent_items) {
                    let row = search.rows().iter().find(|row| row.source.id == source_id).expect("the row that was answered");
                    prop_assert!(matches!(&row.state, SearchState::Results(items) if items.len() == sent_items), "the items arrive as they were sent");
                }
            }
            let before: Vec<i64> = search.rows().iter().map(|row| row.source.id).collect();
            search.sort_rows();
            let after: Vec<i64> = search.rows().iter().map(|row| row.source.id).collect();
            prop_assert_eq!(before, after, "sorting a sorted list changes nothing");
        }
    }
}
