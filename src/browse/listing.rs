//! The state of one source listing: the items loaded so far, the pager that keeps a single request in flight, and the
//! two error slots (T-40a-003, T-40a-004). The widget in `listing_view` only draws what `plan` says and asks for pages
//! through `ask`. No GTK here.

use std::future::Future;
use std::pin::Pin;

use kuro_domain::source::browse::BrowseQuery;
use kuro_i18n::{MsgKey, tr};
use kuro_prefs::library::LibraryDisplayMode;
use kuro_source::filter_list::FilterList;

use super::content::{ContentInput, ContentPlan, content_plan};
use super::grid::{PageAsk, Pager, prefetch_due};

/// One cover of a listing. `id` is the library row of the entry, found or inserted by `network_to_local` when the
/// page was loaded (0 when a caller has none, as in global search).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ListItem {
    pub id: i64,
    pub source_id: i64,
    /// The entry's url in its source; with `source_id` it identifies the row.
    pub url: String,
    pub title: String,
    /// The entry is already in the library: its cover is dimmed and it gets the bookmark badge.
    pub favorite: bool,
    /// Where the cover comes from (a URL or a file path); the app's cover loader decides what to do with it.
    pub cover: Option<String>,
}

/// One page a source returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedPage {
    pub items: Vec<ListItem>,
    pub has_next: bool,
}

pub type LoadFuture = Pin<Box<dyn Future<Output = Result<ListedPage, String>> + Send>>;

/// A pasted link being resolved to the library row of the entry it names; `None` when no source resolves it (T-20a-031).
pub type LinkFuture = Pin<Box<dyn Future<Output = Option<ListItem>> + Send>>;

/// Loads the pages of one source. Runs on the tokio runtime; the GTK thread never awaits it.
pub trait PageLoader: Send + Sync + 'static {
    /// Page `page` (1-based) of `query`. `filters` is the filter list a search uses (B§4.6.1); Popular and Latest
    /// ignore it.
    fn load(&self, query: BrowseQuery, filters: FilterList, page: u32) -> LoadFuture;
}

/// The loaded items and the paging state of one listing.
pub struct Listing {
    query: BrowseQuery,
    filters: FilterList,
    pager: Pager,
    items: Vec<Option<ListItem>>,
    loading: bool,
    refresh_error: Option<String>,
    append_error: Option<String>,
    /// `browse_hide_in_library_items`, read when the page was made (B§1.1, T-40b-034): favourites are not kept.
    hide_in_library: bool,
    /// The items are the saved results of the last visit (`results_cache`): the first page the site answers replaces
    /// them, and no later page is asked before it.
    provisional: bool,
}

impl Listing {
    pub fn new(query: BrowseQuery) -> Self {
        Self {
            query,
            filters: FilterList::default(),
            pager: Pager::new(),
            items: Vec::new(),
            loading: false,
            refresh_error: None,
            append_error: None,
            hide_in_library: false,
            provisional: false,
        }
    }

    /// Sets the hide-in-library flag. It applies to the pages delivered after the call; a favourite already on screen stays
    /// until the listing changes (B-EC-15).
    pub fn set_hide_in_library(&mut self, hide: bool) {
        self.hide_in_library = hide;
    }

    pub fn query(&self) -> &BrowseQuery {
        &self.query
    }

    /// The filters the listing's search runs with.
    pub fn filters(&self) -> &FilterList {
        &self.filters
    }

    #[cfg(test)]
    /// Starts a new listing (a search, a Popular or Latest switch, a refresh) with the filters it had. Pages still in
    /// flight are dropped.
    pub fn restart(&mut self, query: BrowseQuery) {
        let filters = std::mem::take(&mut self.filters);
        self.restart_with(query, filters);
    }

    /// Starts a new listing with its own filter list (`search(query, filters)`, B§4.6.1).
    pub fn restart_with(&mut self, query: BrowseQuery, filters: FilterList) {
        self.query = query;
        self.filters = filters;
        self.pager.reset();
        self.items.clear();
        self.loading = false;
        self.refresh_error = None;
        self.append_error = None;
        self.provisional = false;
    }

    /// Shows the saved results of the last visit while the first page loads. Only a listing with nothing loaded takes
    /// them.
    pub fn prefill(&mut self, page: ListedPage) {
        if !self.items.is_empty() || page.items.is_empty() {
            return;
        }
        let hide = self.hide_in_library;
        self.items = page.items.into_iter().filter(|item| !(hide && item.favorite)).map(Some).collect();
        self.provisional = !self.items.is_empty();
    }

    /// The items on screen are the saved results, not yet replaced by the site's answer.
    pub fn is_provisional(&self) -> bool {
        self.provisional
    }

    /// The next page to load, or `None` while one is in flight or the listing has ended.
    pub fn ask(&mut self) -> Option<PageAsk> {
        let ask = self.pager.ask()?;
        self.loading = true;
        Some(ask)
    }

    /// A page arrived. Returns how many items it added, or `None` when it belongs to an older listing. An empty page is
    /// a failure, as in `MangaSourcePagingSource`: with nothing loaded it is the refresh error `no_results_found`
    /// (B-EC-03), after items it is an appended error and the same page can be asked again (B-EC-04). The failure is
    /// reported as `Some(0)`. With the hide-in-library flag on, favourites are not kept, so a page of only favourites adds 0.
    pub fn deliver(&mut self, generation: u64, page: ListedPage) -> Option<usize> {
        if page.items.is_empty() {
            return self.fail(generation, tr(MsgKey::no_results_found)).then_some(0);
        }
        if !self.pager.deliver(generation, page.has_next) {
            return None;
        }
        if std::mem::take(&mut self.provisional) {
            self.items.clear();
        }
        self.loading = false;
        self.append_error = None;
        self.refresh_error = None;
        let hide = self.hide_in_library;
        let kept: Vec<ListItem> = page.items.into_iter().filter(|item| !(hide && item.favorite)).collect();
        let added = kept.len();
        self.items.extend(kept.into_iter().map(Some));
        Some(added)
    }

    /// A page failed. Before any item loaded it is a refresh error, after that an append error. Stale failures are
    /// dropped and return `false`.
    pub fn fail(&mut self, generation: u64, message: String) -> bool {
        if !self.pager.fail(generation) {
            return false;
        }
        self.loading = false;
        if self.items.is_empty() {
            self.refresh_error = Some(message);
        } else {
            self.append_error = Some(message);
        }
        true
    }

    /// Sets the favourite flag of one loaded item, as the library changed it. False when the slot is not an item.
    pub fn set_favorite(&mut self, index: usize, favorite: bool) -> bool {
        match self.items.get_mut(index) {
            Some(Some(item)) => {
                item.favorite = favorite;
                true
            }
            _ => false,
        }
    }

    /// Retry: forget the errors so the same page can be asked again with `ask`.
    pub fn clear_errors(&mut self) {
        self.refresh_error = None;
        self.append_error = None;
    }

    /// `last_visible` is the index of the last cell on screen. True when the next page should be asked for.
    pub fn wants_more(&self, last_visible: usize) -> bool {
        !self.pager.is_done()
            && !self.provisional
            && !self.loading
            && self.append_error.is_none()
            && self.refresh_error.is_none()
            && prefetch_due(last_visible, self.items.len())
    }

    /// A page is loading while items are already shown: the loading row under them (T-41a-006, T-41b-018).
    pub fn loading_more(&self) -> bool {
        self.loading && !self.items.is_empty()
    }

    /// The slots of the grid. A `None` slot is a placeholder and draws nothing.
    pub fn slots(&self) -> &[Option<ListItem>] {
        &self.items
    }

    pub fn loaded(&self) -> usize {
        self.items.iter().flatten().count()
    }

    /// What the content area shows now.
    pub fn plan(&self, initialized: bool, stub: bool, is_local: bool, display: LibraryDisplayMode) -> ContentPlan {
        content_plan(&ContentInput {
            initialized,
            stub,
            is_local,
            loading: self.loading,
            items: self.loaded(),
            refresh_error: self.refresh_error.as_deref(),
            append_error: self.append_error.as_deref(),
            display,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::primitives::EmptyAction;

    fn item(title: &str) -> ListItem {
        ListItem { title: title.into(), ..ListItem::default() }
    }

    fn page(titles: &[&str], has_next: bool) -> ListedPage {
        ListedPage { items: titles.iter().map(|t| item(t)).collect(), has_next }
    }

    fn plan(listing: &Listing) -> ContentPlan {
        listing.plan(true, false, false, LibraryDisplayMode::CompactGrid)
    }

    #[test]
    fn saved_results_show_at_once_and_the_first_answer_replaces_them_in_place() {
        let mut listing = Listing::new(BrowseQuery::Popular);
        listing.prefill(page(&["old a", "old b"], false));
        assert!(listing.is_provisional());
        assert_eq!(listing.loaded(), 2);
        let ask = listing.ask().expect("the site is still asked");
        assert!(!listing.wants_more(10), "no second page before the first answer");
        assert_eq!(listing.deliver(ask.generation, page(&["new"], true)), Some(1));
        assert!(!listing.is_provisional());
        let titles: Vec<&str> = listing.slots().iter().flatten().map(|item| item.title.as_str()).collect();
        assert_eq!(titles, ["new"]);
        listing.prefill(page(&["late"], false));
        assert_eq!(listing.loaded(), 1, "a listing with items keeps them");
    }

    #[test]
    fn a_failed_refresh_keeps_the_saved_results_and_the_retry_still_replaces_them() {
        let mut listing = Listing::new(BrowseQuery::Popular);
        listing.prefill(page(&["old"], false));
        let ask = listing.ask().unwrap();
        assert!(listing.fail(ask.generation, "offline".into()));
        assert_eq!(listing.loaded(), 1);
        listing.clear_errors();
        let ask = listing.ask().expect("the same page again");
        listing.deliver(ask.generation, page(&["new"], false));
        assert_eq!(listing.slots().iter().flatten().map(|item| item.title.as_str()).collect::<Vec<_>>(), ["new"]);
    }

    #[test]
    fn the_first_load_shows_the_spinner_then_the_items() {
        let mut listing = Listing::new(BrowseQuery::Popular);
        let ask = listing.ask().expect("first page");
        assert_eq!(plan(&listing), ContentPlan::Spinner);
        assert_eq!(listing.deliver(ask.generation, page(&["a", "b"], true)), Some(2));
        assert!(matches!(plan(&listing), ContentPlan::Items { mode: LibraryDisplayMode::CompactGrid, toast: None }));
    }

    #[test]
    fn only_one_request_is_in_flight_and_the_next_page_follows_a_delivery() {
        let mut listing = Listing::new(BrowseQuery::Popular);
        let first = listing.ask().expect("page 1");
        assert!(listing.ask().is_none(), "a second ask while one is in flight is refused");
        listing.deliver(first.generation, page(&["a"], true));
        assert_eq!(listing.ask().map(|a| a.page), Some(2));
    }

    #[test]
    fn the_loading_row_shows_only_while_a_later_page_loads_under_items() {
        let mut listing = Listing::new(BrowseQuery::Popular);
        let first = listing.ask().expect("page 1");
        assert!(!listing.loading_more(), "the first page has the centred spinner, not the loading row");
        listing.deliver(first.generation, page(&["a"], true));
        assert!(!listing.loading_more(), "nothing is in flight");
        let second = listing.ask().expect("page 2");
        assert!(listing.loading_more(), "a page is loading under the shown items");
        listing.deliver(second.generation, page(&["b"], true));
        assert!(!listing.loading_more());
    }

    #[test]
    fn a_late_page_of_an_older_listing_is_dropped() {
        let mut listing = Listing::new(BrowseQuery::Popular);
        let old = listing.ask().expect("page");
        listing.restart(BrowseQuery::Search("x".into()));
        assert_eq!(listing.deliver(old.generation, page(&["stale"], false)), None);
        assert!(!listing.fail(old.generation, "late".into()));
        assert_eq!(listing.loaded(), 0);
        assert_eq!(listing.query(), &BrowseQuery::Search("x".into()));
        assert_eq!(listing.ask().map(|a| a.page), Some(1));
    }

    #[test]
    fn a_first_page_failure_is_an_error_state_with_actions() {
        let mut listing = Listing::new(BrowseQuery::Popular);
        let ask = listing.ask().expect("page");
        assert!(listing.fail(ask.generation, "offline".into()));
        match plan(&listing) {
            ContentPlan::Error { message, actions } => {
                assert_eq!(message, "offline");
                assert!(actions.contains(&EmptyAction::Retry));
            }
            other => panic!("unexpected plan {other:?}"),
        }
    }

    #[test]
    fn a_later_page_failure_keeps_the_items_and_raises_a_retry_toast() {
        let mut listing = Listing::new(BrowseQuery::Popular);
        let first = listing.ask().expect("page 1");
        listing.deliver(first.generation, page(&["a"], true));
        let second = listing.ask().expect("page 2");
        listing.fail(second.generation, "page 2 failed".into());
        match plan(&listing) {
            ContentPlan::Items { toast: Some(toast), .. } => assert_eq!(toast.message, "page 2 failed"),
            other => panic!("unexpected plan {other:?}"),
        }
        assert!(!listing.wants_more(0), "no automatic retry while the toast is up");
    }

    #[test]
    fn retry_asks_the_failed_page_again_and_clears_the_error() {
        let mut listing = Listing::new(BrowseQuery::Popular);
        let ask = listing.ask().expect("page");
        listing.fail(ask.generation, "offline".into());
        listing.clear_errors();
        let again = listing.ask().expect("same page again");
        assert_eq!(again.page, 1);
        assert_eq!(plan(&listing), ContentPlan::Spinner);
    }

    #[test]
    fn the_next_page_is_wanted_one_page_before_the_end_and_never_after_the_last() {
        let mut listing = Listing::new(BrowseQuery::Popular);
        let ask = listing.ask().expect("page");
        let titles: Vec<String> = (0..30).map(|i| format!("t{i}")).collect();
        let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
        listing.deliver(ask.generation, page(&refs, true));
        assert!(!listing.wants_more(0));
        assert!(listing.wants_more(10), "10 + 25 reaches 30 loaded items");
        let last = listing.ask().expect("page 2");
        assert!(!listing.wants_more(29), "a request is in flight");
        listing.deliver(last.generation, page(&["z"], false));
        assert!(!listing.wants_more(30), "the listing has ended");
    }

    /// T-40b-085 `browse_loader_demand_driven`: a fake paged source that records the pages asked for. Page 1 is asked when
    /// the listing opens; each later page only when the viewer scrolls near the end, and never twice at once.
    #[test]
    fn browse_loader_demand_driven() {
        fn serve(listing: &mut Listing, asked: &mut Vec<u32>) {
            let ask = listing.ask().expect("a page is due");
            asked.push(ask.page);
            let titles: Vec<String> = (0..30).map(|i| format!("p{}-{i}", ask.page)).collect();
            let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
            listing.deliver(ask.generation, page(&refs, ask.page < 5));
        }
        let (mut listing, mut asked) = (Listing::new(BrowseQuery::Popular), Vec::new());
        serve(&mut listing, &mut asked);
        for top in [0, 3, 4] {
            assert!(!listing.wants_more(top), "near the top nothing more is asked for");
        }
        assert_eq!(asked, [1], "no eager fetch of page 2");
        assert!(listing.wants_more(10), "scrolling to the last 25 cells asks for page 2");
        serve(&mut listing, &mut asked);
        assert_eq!(asked, [1, 2]);
        assert!(!listing.wants_more(10), "with 60 items the same position is far from the end again");
        assert!(listing.wants_more(40));
        let in_flight = listing.ask().expect("page 3");
        assert_eq!(in_flight.page, 3);
        assert!(!listing.wants_more(59), "a request in flight blocks a second one");
        assert_eq!(asked.len(), 2, "pages 4 and 5 were never asked for");
    }

    #[test]
    fn an_empty_first_page_is_an_error_with_no_results_found_and_a_stub_never_lists() {
        let mut listing = Listing::new(BrowseQuery::Popular);
        let ask = listing.ask().expect("page");
        assert_eq!(listing.deliver(ask.generation, page(&[], false)), Some(0));
        match plan(&listing) {
            ContentPlan::Error { message, actions } => {
                assert_eq!(message, tr(MsgKey::no_results_found));
                assert!(actions.contains(&EmptyAction::Retry), "the error state offers retry");
            }
            other => panic!("unexpected plan {other:?}"),
        }
        assert_eq!(listing.plan(true, true, false, LibraryDisplayMode::List), ContentPlan::Stub);
    }

    #[test]
    fn an_empty_later_page_is_an_appended_error_even_when_the_source_says_more_follow() {
        let mut listing = Listing::new(BrowseQuery::Popular);
        let first = listing.ask().expect("page 1");
        listing.deliver(first.generation, page(&["a"], true));
        let second = listing.ask().expect("page 2");
        assert_eq!(listing.deliver(second.generation, page(&[], true)), Some(0));
        assert_eq!(listing.loaded(), 1, "the items stay");
        match plan(&listing) {
            ContentPlan::Items { toast: Some(toast), .. } => assert_eq!(toast.message, tr(MsgKey::no_results_found)),
            other => panic!("unexpected plan {other:?}"),
        }
        assert!(!listing.wants_more(0), "the pager does not keep asking");
        listing.clear_errors();
        assert_eq!(listing.ask().map(|a| a.page), Some(2), "retry asks the same page again");
    }

    #[test]
    fn a_stale_empty_page_is_dropped_like_any_stale_page() {
        let mut listing = Listing::new(BrowseQuery::Popular);
        let old = listing.ask().expect("page");
        listing.restart(BrowseQuery::Latest);
        assert_eq!(listing.deliver(old.generation, page(&[], false)), None);
        assert_eq!(plan(&listing), ContentPlan::Empty, "nothing was recorded for the new listing");
    }

    #[test]
    fn a_search_carries_its_own_filters_and_restart_keeps_them() {
        use kuro_source::filter::Filter;
        let mut listing = Listing::new(BrowseQuery::Popular);
        let filters = FilterList(vec![Filter::check_box("Flag")]);
        listing.restart_with(BrowseQuery::Search("x".into()), filters.clone());
        assert_eq!(listing.filters(), &filters);
        listing.restart(BrowseQuery::Search("y".into()));
        assert_eq!(listing.filters(), &filters, "a plain restart keeps the filter list");
        listing.restart_with(BrowseQuery::Popular, FilterList::default());
        assert!(listing.filters().is_empty());
    }

    /// The whole path of a source that does not answer: spinner, then the "not answering" sentence with Retry (never an
    /// endless spinner), then the content once a retry works.
    #[test]
    fn a_silent_source_ends_the_spinner_with_a_sentence_and_a_retry_shows_the_content() {
        use crate::browse::errors::settle;
        use std::time::Duration;
        let run = |future: LoadFuture| {
            let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().expect("runtime");
            rt.block_on(settle(Duration::from_millis(20), "Comix", future))
        };
        let mut listing = Listing::new(BrowseQuery::Popular);
        let ask = listing.ask().expect("page 1");
        assert_eq!(plan(&listing), ContentPlan::Spinner);
        let silent: LoadFuture = Box::pin(std::future::pending::<Result<ListedPage, String>>());
        let message = run(silent).expect_err("the limit ends the load");
        assert!(listing.fail(ask.generation, message));
        match plan(&listing) {
            ContentPlan::Error { message, actions } => {
                assert_eq!(message, "Comix is not answering. Try again in a moment.");
                assert!(actions.contains(&EmptyAction::Retry));
            }
            other => panic!("unexpected plan {other:?}"),
        }
        listing.clear_errors();
        let again = listing.ask().expect("retry asks page 1 again");
        assert_eq!(plan(&listing), ContentPlan::Spinner);
        let answered: LoadFuture = Box::pin(async { Ok(page(&["a"], false)) });
        listing.deliver(again.generation, run(answered).expect("the answer"));
        assert!(matches!(plan(&listing), ContentPlan::Items { .. }));
    }

    #[test]
    fn a_favourite_flag_changes_in_place() {
        let mut listing = Listing::new(BrowseQuery::Popular);
        let ask = listing.ask().expect("page");
        listing.deliver(ask.generation, page(&["a"], false));
        assert!(listing.set_favorite(0, true));
        assert!(listing.slots()[0].as_ref().is_some_and(|i| i.favorite));
        assert!(!listing.set_favorite(5, true));
    }

    #[test]
    fn the_registry_not_ready_shows_the_spinner_even_with_items() {
        let mut listing = Listing::new(BrowseQuery::Popular);
        let ask = listing.ask().expect("page");
        listing.deliver(ask.generation, page(&["a"], false));
        assert_eq!(listing.plan(false, false, false, LibraryDisplayMode::List), ContentPlan::Spinner);
    }

    #[test]
    fn first_load_with_no_items_and_more_to_come_is_still_the_no_results_error() {
        // page 1 with "0 animes, hasNextPage any"
        let mut listing = Listing::new(BrowseQuery::Popular);
        let ask = listing.ask().expect("page");
        assert_eq!(listing.deliver(ask.generation, page(&[], true)), Some(0));
        match plan(&listing) {
            ContentPlan::Error { message, actions } => {
                assert_eq!(message, tr(MsgKey::no_results_found));
                assert!(actions.contains(&EmptyAction::Retry));
            }
            other => panic!("unexpected plan {other:?}"),
        }
        assert!(!listing.wants_more(0), "the empty state does not page on by itself");
    }

    #[test]
    fn first_load_with_items_and_no_next_page_asks_for_nothing_more() {
        // Page-1 table row "n > 0 animes, hasNextPage = false" and E-BR-19: one page, no append request.
        let mut listing = Listing::new(BrowseQuery::Popular);
        let ask = listing.ask().expect("page");
        assert_eq!(listing.deliver(ask.generation, page(&["a", "b"], false)), Some(2));
        assert!(matches!(plan(&listing), ContentPlan::Items { toast: None, .. }));
        assert!(listing.ask().is_none(), "the source said there is no next page");
        assert!(!listing.wants_more(1));
    }

    #[test]
    fn a_source_that_always_says_more_follows_is_paged_without_a_cap() {
        // E-BR-18: hasNextPage = true forever. Kurobayomi keeps no cap (parity), so every page is asked in turn.
        let mut listing = Listing::new(BrowseQuery::Popular);
        let mut ask = listing.ask().expect("page 1");
        for expected in 1..=200_u32 {
            assert_eq!(ask.page, expected);
            listing.deliver(ask.generation, page(&["x"], true));
            assert!(listing.wants_more(listing.loaded() - 1), "page {expected} leaves the next one wanted");
            ask = listing.ask().expect("the next page");
        }
        assert_eq!(listing.loaded(), 200);
    }

    #[test]
    fn a_later_empty_page_without_more_is_an_appended_error_and_keeps_the_items() {
        // Append table, row "0 animes" with hasNextPage = false.
        let mut listing = Listing::new(BrowseQuery::Popular);
        let first = listing.ask().expect("page 1");
        listing.deliver(first.generation, page(&["a"], true));
        let second = listing.ask().expect("page 2");
        assert_eq!(listing.deliver(second.generation, page(&[], false)), Some(0));
        assert_eq!(listing.loaded(), 1);
        assert!(matches!(plan(&listing), ContentPlan::Items { toast: Some(_), .. }));
    }

    #[test]
    fn a_later_page_with_items_and_no_next_page_ends_the_listing() {
        // Append table, row "n > 0, hasNextPage = false".
        let mut listing = Listing::new(BrowseQuery::Popular);
        let first = listing.ask().expect("page 1");
        listing.deliver(first.generation, page(&["a"], true));
        let second = listing.ask().expect("page 2");
        assert_eq!(listing.deliver(second.generation, page(&["b", "c"], false)), Some(2));
        assert_eq!(listing.loaded(), 3);
        assert!(listing.ask().is_none(), "the listing has ended");
        assert!(!listing.wants_more(2));
    }

    #[test]
    fn a_later_page_with_items_and_more_schedules_the_page_after_it() {
        // Append table, row "n > 0, hasNextPage = true".
        let mut listing = Listing::new(BrowseQuery::Popular);
        let first = listing.ask().expect("page 1");
        listing.deliver(first.generation, page(&["a"], true));
        let second = listing.ask().expect("page 2");
        listing.deliver(second.generation, page(&["b"], true));
        assert_eq!(listing.ask().map(|a| a.page), Some(3));
    }

    #[test]
    fn a_refresh_during_an_append_drops_the_old_append_and_keeps_the_new_listing_empty() {
        // RC-2: the append of the old listing is in flight when the listing restarts.
        let mut listing = Listing::new(BrowseQuery::Popular);
        let first = listing.ask().expect("page 1");
        listing.deliver(first.generation, page(&["a"], true));
        let append = listing.ask().expect("page 2 in flight");
        listing.restart(BrowseQuery::Latest);
        assert_eq!(listing.deliver(append.generation, page(&["old"], false)), None);
        assert_eq!(listing.loaded(), 0, "the old append does not reach the new listing");
        assert_eq!(listing.ask().map(|a| a.page), Some(1));
    }

    #[test]
    fn a_placeholder_slot_is_not_counted_as_loaded() {
        let mut listing = Listing::new(BrowseQuery::Popular);
        listing.items.push(None);
        listing.items.push(Some(item("a")));
        assert_eq!(listing.slots().len(), 2);
        assert_eq!(listing.loaded(), 1);
    }

    #[test]
    fn hide_in_library_keeps_only_the_non_favourites_of_a_page_and_a_flag_off_keeps_all() {
        let favourite = |id: i64| ListItem { id, title: format!("F{id}"), favorite: true, ..ListItem::default() };
        let plain = |id: i64| ListItem { id, title: format!("P{id}"), ..ListItem::default() };
        let mut listing = Listing::new(BrowseQuery::Popular);
        listing.set_hide_in_library(true);
        let ask = listing.ask().expect("a first page is asked");
        assert_eq!(listing.deliver(ask.generation, ListedPage { items: vec![favourite(1), plain(2), favourite(3)], has_next: true }), Some(1));
        assert_eq!(listing.loaded(), 1, "T-40b-034: a favourite is not shown");
        let mut all = Listing::new(BrowseQuery::Popular);
        let ask = all.ask().expect("a first page is asked");
        assert_eq!(all.deliver(ask.generation, ListedPage { items: vec![favourite(1), plain(2)], has_next: false }), Some(2));
        assert_eq!(all.loaded(), 2, "the flag off keeps every row");
    }
}
