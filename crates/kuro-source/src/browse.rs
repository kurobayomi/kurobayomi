//! Browse state of one source (`BrowseMangaSourceScreenModel.kt`; T-15a-040).
//! It holds the listing that runs and the sheet's filters. It makes no source call: the screen runs the call for the
//! listing it holds. A running listing owns its own clone of the filters, so sheet edits never change it (RC-06, RC-07).

use crate::filter::Filter;
use crate::filter_list::FilterList;
use crate::listing::Listing;

/// The listing and sheet state of one browse screen.
#[derive(Debug, Clone)]
pub struct BrowseState {
    listing: Listing<FilterList>,
    sheet: FilterList,
}

impl BrowseState {
    /// A new screen: the popular list, with the sheet at the source's fresh filters.
    pub fn new(fresh: FilterList) -> Self {
        Self { listing: Listing::Popular, sheet: fresh }
    }

    pub fn listing(&self) -> &Listing<FilterList> {
        &self.listing
    }

    pub fn sheet(&self) -> &FilterList {
        &self.sheet
    }

    /// The text of the running search, for the toolbar. `None` for popular, latest, and a search without text.
    pub fn query(&self) -> Option<&str> {
        match &self.listing {
            Listing::Search { query, .. } => query.as_deref(),
            _ => None,
        }
    }

    /// A sheet edit. Changes only the sheet; the running listing is untouched until Filter is pressed (RC-06).
    pub fn set_sheet(&mut self, filters: FilterList) {
        self.sheet = filters;
    }

    pub fn popular(&mut self) {
        self.listing = Listing::Popular;
    }

    pub fn latest(&mut self) {
        self.listing = Listing::Latest;
    }

    /// `search(query, filters)` (topic 3.2). A `None` argument keeps what the running search had. When no search runs
    /// yet, the search starts from `fresh`, the source's new filters.
    pub fn search(&mut self, query: Option<String>, filters: Option<FilterList>, fresh: impl FnOnce() -> FilterList) {
        let (old_query, old_filters) = match &self.listing {
            Listing::Search { query, filters } => (query.clone(), filters.clone()),
            _ => (None, fresh()),
        };
        self.listing = Listing::Search { query: query.or(old_query), filters: filters.unwrap_or(old_filters) };
    }

    /// The Filter button: a search with the sheet's current state.
    pub fn filter_press(&mut self, fresh: impl FnOnce() -> FilterList) {
        let sheet = self.sheet.clone();
        self.search(None, Some(sheet), fresh);
    }

    /// The Reset button: the sheet gets `fresh`. The running listing and its filters stay (EC-FI-10).
    pub fn reset_filters(&mut self, fresh: FilterList) {
        self.sheet = fresh;
    }

    /// `searchGenre(genre)` (topic 3.3). Sets the matching filter of `defaults` when one exists, and searches with no text.
    /// Otherwise the genre name is searched as text with the default filters.
    pub fn search_genre(&mut self, genre: &str, mut defaults: FilterList) {
        let exists = apply_genre(&mut defaults.0, genre);
        let query = if exists { None } else { Some(genre.to_owned()) };
        self.sheet = defaults.clone();
        self.listing = Listing::Search { query, filters: defaults };
    }
}

/// Case-insensitive name equality (`equals(other, ignoreCase = true)`).
fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// The genre rule of topic 3.3. Returns whether a filter matched the genre. A `Group` is searched one level deep:
/// a matching tri-state is set to include, a matching checkbox is checked, and any other matching kind only counts as a
/// match. A top-level `Select` is set to the index of the matching value. Other top-level kinds are never matched.
fn apply_genre(filters: &mut [Filter], genre: &str) -> bool {
    for filter in filters.iter_mut() {
        match filter {
            Filter::Group { state, .. } => {
                for nested in state.iter_mut() {
                    if !same_name(nested.name(), genre) {
                        continue;
                    }
                    match nested {
                        Filter::TriState { state, .. } => *state = crate::filter::TriStateState::Include,
                        Filter::CheckBox { state, .. } => *state = true,
                        _ => {}
                    }
                    return true;
                }
            }
            Filter::Select { values, state, .. } => {
                if let Some(index) = values.iter().position(|value| same_name(value, genre)) {
                    if let Ok(index) = i32::try_from(index) {
                        *state = index;
                    }
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::TriStateState;

    fn fresh() -> FilterList {
        FilterList(vec![Filter::text("Query"), Filter::group("Genres", vec![Filter::tri_state("Action")])])
    }

    fn genres_select() -> FilterList {
        FilterList(vec![Filter::select("Genre", vec!["Any".into(), "Comedy".into()])])
    }

    #[test]
    fn a_new_screen_starts_on_popular_with_the_fresh_sheet() {
        let state = BrowseState::new(fresh());
        assert_eq!(state.listing(), &Listing::Popular);
        assert_eq!(state.sheet(), &fresh());
        assert_eq!(state.query(), None);
    }

    #[test]
    fn popular_latest_and_search_are_distinct_listings() {
        let mut state = BrowseState::new(fresh());
        state.latest();
        assert_eq!(state.listing(), &Listing::Latest);
        state.search(Some("naruto".into()), None, fresh);
        assert_eq!(state.query(), Some("naruto"));
        assert_ne!(state.listing(), &Listing::Latest);
        state.popular();
        assert_eq!(state.listing(), &Listing::Popular);
    }

    #[test]
    fn search_keeps_the_old_query_or_filters_when_given_none() {
        let mut state = BrowseState::new(fresh());
        state.search(Some("one".into()), Some(genres_select()), fresh);
        state.search(None, None, fresh);
        assert_eq!(state.listing(), &Listing::Search { query: Some("one".into()), filters: genres_select() });
        state.search(Some("two".into()), None, fresh);
        assert_eq!(state.query(), Some("two"));
    }

    #[test]
    fn search_from_popular_starts_with_the_fresh_filters() {
        let mut state = BrowseState::new(fresh());
        state.search(Some("x".into()), None, fresh);
        assert_eq!(state.listing(), &Listing::Search { query: Some("x".into()), filters: fresh() });
    }

    #[test]
    fn filter_press_searches_with_the_sheet_state() {
        let mut state = BrowseState::new(fresh());
        let mut sheet = fresh();
        sheet.0[0] = Filter::Text { name: "Query".into(), state: "bleach".into() };
        state.set_sheet(sheet.clone());
        state.filter_press(fresh);
        assert_eq!(state.listing(), &Listing::Search { query: None, filters: sheet });
    }

    #[test]
    fn a_running_listing_is_not_changed_by_sheet_edits() {
        let mut state = BrowseState::new(fresh());
        state.filter_press(fresh);
        let running = state.listing().clone();
        let mut edited = fresh();
        edited.0[0] = Filter::Text { name: "Query".into(), state: "typed later".into() };
        state.set_sheet(edited);
        assert_eq!(state.listing(), &running, "RC-06: only Filter starts a new search");
    }

    #[test]
    fn reset_replaces_the_sheet_and_does_not_start_a_search() {
        let mut state = BrowseState::new(fresh());
        state.search(Some("running".into()), None, fresh);
        let running = state.listing().clone();
        state.set_sheet(genres_select());
        state.reset_filters(fresh());
        assert_eq!(state.sheet(), &fresh(), "EC-FI-10: the sheet is reset");
        assert_eq!(state.listing(), &running, "EC-FI-10: the running search is unchanged");
    }

    #[test]
    fn search_genre_sets_a_matching_tri_state_and_searches_with_no_text() {
        let mut state = BrowseState::new(fresh());
        state.search_genre("action", fresh());
        let Listing::Search { query, filters } = state.listing() else { panic!("expected search") };
        assert_eq!(query, &None, "EC-FI-09: a match searches with no text");
        match &filters.0[1] {
            Filter::Group { state, .. } => {
                assert_eq!(state[0], Filter::TriState { name: "Action".into(), state: TriStateState::Include });
            }
            _ => panic!("expected group"),
        }
        assert_eq!(state.sheet(), filters, "the sheet takes the same defaults");
    }

    #[test]
    fn search_genre_without_a_match_searches_the_genre_name_as_text() {
        let mut state = BrowseState::new(fresh());
        state.search_genre("romance", fresh());
        assert_eq!(state.listing(), &Listing::Search { query: Some("romance".into()), filters: fresh() });
    }

    #[test]
    fn search_genre_selects_the_matching_value_of_a_top_level_select() {
        let mut state = BrowseState::new(fresh());
        state.search_genre("COMEDY", genres_select());
        let Listing::Search { query, filters } = state.listing() else { panic!("expected search") };
        assert_eq!(query, &None);
        assert_eq!(filters.0[0].select_value(), Some("Comedy"));
    }

    #[test]
    fn search_genre_matches_a_nested_checkbox_and_ignores_top_level_text() {
        let defaults =
            FilterList(vec![Filter::text("action"), Filter::group("Genres", vec![Filter::check_box("Action")])]);
        let mut state = BrowseState::new(fresh());
        state.search_genre("action", defaults);
        let Listing::Search { query, filters } = state.listing() else { panic!("expected search") };
        assert_eq!(query, &None);
        assert_eq!(
            filters.0[1],
            Filter::group("Genres", vec![Filter::CheckBox { name: "Action".into(), state: true }])
        );
        assert_eq!(filters.0[0], Filter::text("action"), "top-level text is never matched");
    }
}
