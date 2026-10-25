//! What an entry page hands to the screens it opens (T-38a-016, T-38a-033, T-38a-017). `ui::open` takes a screen id and
//! nothing else, so a request that carries data (the genre to search, the entry to migrate, the source whose settings
//! open) is parked here first; the screen that opens takes it. A request is read once. No GTK here except the
//! `open_*` helpers, which push the screen after parking the request.

use std::cell::RefCell;

use kuro_platform::kind::Kind;

use crate::entry_core::header::SearchRequest;
use crate::ui;

/// The entry a migrate flow starts from. `anime` picks the library the old entry lives in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrateRequest {
    pub entry_id: i64,
    pub kind: Kind,
    /// The old entry's title, shown in the migrate dialog; a click on it opens the old entry (`open_old`).
    pub title: String,
}

/// The source whose preferences the Settings item of the overflow menu opens. `kind` picks the registry the id lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourcePrefsRequest {
    pub source_id: i64,
    pub kind: Kind,
}

thread_local! {
    static SOURCE_PREFS: RefCell<Option<SourcePrefsRequest>> = const { RefCell::new(None) };
    static SEARCH: RefCell<Option<SearchRequest>> = const { RefCell::new(None) };
    static MIGRATE: RefCell<Option<MigrateRequest>> = const { RefCell::new(None) };
}

pub fn park_search(request: SearchRequest) {
    SEARCH.with(|s| *s.borrow_mut() = Some(request));
}

/// The search the opened screen should run, if a page parked one. It is handed out once.
pub fn take_search() -> Option<SearchRequest> {
    SEARCH.with(|s| s.borrow_mut().take())
}

pub fn park_source_prefs(request: SourcePrefsRequest) {
    SOURCE_PREFS.with(|s| *s.borrow_mut() = Some(request));
}

/// The source the opened source-preferences screen is for, handed out once.
pub fn take_source_prefs() -> Option<SourcePrefsRequest> {
    SOURCE_PREFS.with(|s| s.borrow_mut().take())
}

pub fn park_migrate(request: MigrateRequest) {
    MIGRATE.with(|s| *s.borrow_mut() = Some(request));
}

pub fn take_migrate() -> Option<MigrateRequest> {
    MIGRATE.with(|s| s.borrow_mut().take())
}

/// The page under the entry page on the navigation stack, as `performSearch` reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Previous {
    /// The entry page is the root: there is no page under it.
    None,
    /// The home tabs, where the library lives.
    Home,
    Browse,
    Other,
}

/// What a search click does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchPlan {
    Nothing,
    /// Push the global search.
    Global,
    /// Go back to the library of the entry's kind and search it.
    Library,
    /// The Browse page below runs the text in its own source.
    BrowseText,
    /// The Browse page below runs the genre as a filter (an HTTP source).
    BrowseGenre,
}

/// `previous` for the page stack `ui::page_stack` reports: the names from the root up and the index of the page on show.
pub fn previous_of(names: &[String], shown: usize) -> Previous {
    match shown.checked_sub(1) {
        None => Previous::None,
        Some(0) => Previous::Home,
        Some(below) if names.get(below).is_some_and(|name| name == "screen:browse_source") => Previous::Browse,
        Some(_) => Previous::Other,
    }
}

/// `performSearch` and `performGenreSearch` (behaviour 18.1, 18.2). A global search always pushes the global search;
/// with no page below there is nothing to do; Home searches the library and a Browse page its own source (a genre goes
/// as a filter only when the entry's source is an HTTP one); any other page falls back to the global search (fix 23.14).
pub fn plan_search(request: &SearchRequest, http_source: bool, previous: Previous) -> SearchPlan {
    if request.global {
        return SearchPlan::Global;
    }
    match previous {
        Previous::None => SearchPlan::Nothing,
        Previous::Home => SearchPlan::Library,
        Previous::Browse if request.genre && http_source => SearchPlan::BrowseGenre,
        Previous::Browse => SearchPlan::BrowseText,
        Previous::Other => SearchPlan::Global,
    }
}

/// A genre, author, title or source click on an entry page. `kind` picks the library to search and `http_source` says
/// whether the entry's source is an HTTP one. The request is parked for the screen that opens (`take_search`).
pub fn open_search(request: SearchRequest, kind: Kind, http_source: bool) {
    let (names, shown) = ui::page_stack();
    match plan_search(&request, http_source, previous_of(&names, shown)) {
        SearchPlan::Nothing => {}
        SearchPlan::Global => {
            park_search(request);
            ui::open("global_search");
        }
        SearchPlan::Library => {
            crate::library::mount::search_library(kind, &request.query);
        }
        SearchPlan::BrowseGenre => {
            park_search(request);
            ui::open("browse_source");
        }
        // a genre on a source that is not an HTTP one is a plain text search of the source
        SearchPlan::BrowseText => {
            park_search(SearchRequest { genre: false, ..request });
            ui::open("browse_source");
        }
    }
}

/// The overflow menu's Settings: the preferences screen of the entry's source.
pub fn open_source_preferences(request: SourcePrefsRequest) {
    park_source_prefs(request);
    ui::open("source_preferences");
}

/// The overflow menu's Migrate: the search of the migrate flow, for this entry.
pub fn open_migrate(request: MigrateRequest) {
    park_migrate(request);
    ui::open("migrate_search");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parked_search_is_handed_out_once_and_a_new_one_replaces_it() {
        assert_eq!(take_search(), None);
        park_search(SearchRequest { query: "Action".into(), global: false, genre: true });
        park_search(SearchRequest { query: "Drama".into(), global: true, genre: false });
        assert_eq!(take_search(), Some(SearchRequest { query: "Drama".into(), global: true, genre: false }));
        assert_eq!(take_search(), None);
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn text(global: bool) -> SearchRequest {
        SearchRequest { query: "Frieren".into(), global, genre: false }
    }

    fn genre() -> SearchRequest {
        SearchRequest { query: "Action".into(), global: false, genre: true }
    }

    #[test]
    fn the_previous_page_is_read_from_the_stack_below_the_page_on_show() {
        let stack = names(&["GtkStack", "screen:browse_source", "screen:anime_entry_details"]);
        assert_eq!(previous_of(&stack, 0), Previous::None);
        assert_eq!(previous_of(&stack, 1), Previous::Home);
        assert_eq!(previous_of(&stack, 2), Previous::Browse);
        let other = names(&["GtkStack", "screen:settings", "screen:anime_entry_details"]);
        assert_eq!(previous_of(&other, 2), Previous::Other);
        // a page that was just popped stays in the list above the one on show
        assert_eq!(previous_of(&stack, 1), Previous::Home);
    }

    #[test]
    fn a_global_search_is_global_whatever_is_below() {
        for previous in [Previous::None, Previous::Home, Previous::Browse, Previous::Other] {
            assert_eq!(plan_search(&text(true), true, previous), SearchPlan::Global);
        }
    }

    #[test]
    fn a_source_search_goes_to_the_library_from_home_and_to_the_source_from_browse() {
        assert_eq!(plan_search(&text(false), true, Previous::Home), SearchPlan::Library);
        assert_eq!(plan_search(&text(false), true, Previous::Browse), SearchPlan::BrowseText);
        assert_eq!(plan_search(&text(false), true, Previous::None), SearchPlan::Nothing);
    }

    #[test]
    fn any_other_previous_page_falls_back_to_the_global_search() {
        assert_eq!(plan_search(&text(false), true, Previous::Other), SearchPlan::Global);
        assert_eq!(plan_search(&genre(), true, Previous::Other), SearchPlan::Global);
    }

    #[test]
    fn a_genre_is_a_filter_only_from_browse_with_an_http_source() {
        assert_eq!(plan_search(&genre(), true, Previous::Browse), SearchPlan::BrowseGenre);
        assert_eq!(plan_search(&genre(), false, Previous::Browse), SearchPlan::BrowseText);
        assert_eq!(plan_search(&genre(), true, Previous::Home), SearchPlan::Library);
    }

    #[test]
    fn a_parked_migrate_request_carries_the_entry_and_the_kind() {
        park_migrate(MigrateRequest { entry_id: 7, kind: Kind::Anime, title: "Frieren".into() });
        let request = take_migrate().expect("parked");
        assert_eq!((request.entry_id, request.kind, request.title.as_str()), (7, Kind::Anime, "Frieren"));
        assert_eq!(take_migrate(), None);
    }

    #[test]
    fn a_parked_source_prefs_request_names_the_source_and_is_handed_out_once() {
        assert_eq!(take_source_prefs(), None);
        park_source_prefs(SourcePrefsRequest { source_id: 4, kind: Kind::Anime });
        park_source_prefs(SourcePrefsRequest { source_id: 9, kind: Kind::Manga });
        assert_eq!(take_source_prefs(), Some(SourcePrefsRequest { source_id: 9, kind: Kind::Manga }));
        assert_eq!(take_source_prefs(), None);
    }

}
