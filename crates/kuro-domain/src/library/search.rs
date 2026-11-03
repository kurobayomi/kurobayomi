//! Library search grammar (`MangaLibraryItem.matches`, behaviour 2.6) and the empty-page rules around it.
//! Pure. The UI owns the debounce timer; [`SEARCH_DEBOUNCE`] is the delay it must wait before running a query.

use std::time::Duration;

/// Quiet time of the search entry before the query is applied (`SEARCH_DEBOUNCE_MILLIS`).
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(crate::SEARCH_DEBOUNCE_MS);

/// The fields a query can match. `source_name` is the display name of the entry's source, resolved by the caller.
#[derive(Clone, Copy, Debug)]
pub struct SearchFields<'a> {
    pub id: i64,
    pub title: &'a str,
    pub author: Option<&'a str>,
    pub artist: Option<&'a str>,
    pub description: Option<&'a str>,
    pub source_name: &'a str,
    pub genres: &'a [String],
}

/// Case-insensitive substring test (Kotlin `contains(_, ignoreCase = true)`).
fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// Case-insensitive equality (Kotlin `equals(_, ignoreCase = true)`).
fn equals_ci(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// `id:<n>` lookup. The prefix is matched case-insensitively, but the value is cut at the first `id:` with the
/// case-sensitive `substringAfter`, so `ID:5` yields the whole text and matches nothing (kept from upstream).
fn id_query(query: &str) -> Option<i64> {
    let is_prefix = query.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("id:"));
    if !is_prefix {
        return None;
    }
    let rest = match query.find("id:") {
        Some(at) => &query[at + 3..],
        None => query,
    };
    rest.parse::<i64>().ok()
}

/// `checkNegatableConstraint`: a leading `-` inverts the predicate on the text after it (trimmed at the start).
fn negatable(constraint: &str, predicate: impl Fn(&str) -> bool) -> bool {
    match constraint.strip_prefix('-') {
        Some(rest) => !predicate(rest.trim_start()),
        None => predicate(constraint),
    }
}

/// Whether the entry matches the query. An empty or blank query matches everything.
pub fn matches(query: &str, item: &SearchFields<'_>) -> bool {
    if query.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("id:")) {
        return id_query(query) == Some(item.id);
    }
    contains_ci(item.title, query)
        || item.author.is_some_and(|a| contains_ci(a, query))
        || item.artist.is_some_and(|a| contains_ci(a, query))
        || item.description.is_some_and(|d| contains_ci(d, query))
        || query.split(',').map(str::trim).all(|sub| {
            negatable(sub, |c| contains_ci(item.source_name, c) || item.genres.iter().any(|g| equals_ci(g, c)))
        })
}

/// The text of an empty page while a search, filters or the library itself leave nothing to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyPage {
    NoResults,
    NoMatch,
    CategoryEmpty,
    LibraryEmpty,
}

impl EmptyPage {
    /// The English text of the message.
    pub fn text(self) -> &'static str {
        match self {
            Self::NoResults => "No results found",
            Self::NoMatch => "No match found",
            Self::CategoryEmpty => "Category is empty",
            Self::LibraryEmpty => "Your library is empty",
        }
    }
}

/// Picks the empty-page message in the upstream order: search first, then filters, then an empty category, and
/// the whole library only when nothing else applies.
pub fn empty_page(search_active: bool, filters_active: bool, library_empty: bool) -> EmptyPage {
    if search_active {
        EmptyPage::NoResults
    } else if filters_active {
        EmptyPage::NoMatch
    } else if library_empty {
        EmptyPage::LibraryEmpty
    } else {
        EmptyPage::CategoryEmpty
    }
}

/// The "Search globally" affordance is shown on empty search results and opens global search with the query.
pub fn shows_global_search(search_active: bool, page_is_empty: bool) -> bool {
    search_active && page_is_empty
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item<'a>(genres: &'a [String]) -> SearchFields<'a> {
        SearchFields {
            id: 2,
            title: "Alpha Beta",
            author: Some("Writer X"),
            artist: None,
            description: Some("a story about SPACE"),
            source_name: "Example Source",
            genres,
        }
    }

    #[test]
    fn plain_text_matches_title_author_and_description_case_insensitively() {
        let genres = vec!["Action".to_owned(), "Drama".to_owned()];
        let it = item(&genres);
        assert!(matches("alp", &it));
        assert!(matches("ALP", &it));
        assert!(matches("writer", &it));
        assert!(matches("space", &it), "description");
        assert!(!matches("zzz", &it));
    }

    #[test]
    fn id_prefix_matches_only_that_id_and_nothing_for_bad_numbers() {
        let genres: Vec<String> = vec![];
        let it = item(&genres);
        assert!(matches("id:2", &it));
        assert!(!matches("id:3", &it));
        assert!(!matches("id:abc", &it));
        assert!(!matches("id:", &it));
        assert!(matches("ID:2", &it) == false, "upstream substringAfter is case-sensitive");
    }

    #[test]
    fn comma_separated_constraints_are_anded_and_negatable() {
        let genres = vec!["Action".to_owned(), "Drama".to_owned()];
        let it = item(&genres);
        assert!(matches("action, -isekai", &it));
        assert!(!matches("zzz, action", &it), "every sub-constraint must hold");
        assert!(!matches("-drama", &it), "genre branch negated; title has no literal -drama");
        assert!(matches("example source", &it), "source name by contains");
        assert!(matches("  ", &it), "blank query matches everything");
        assert!(matches("", &it));
    }

    #[test]
    fn empty_page_messages_follow_the_upstream_order() {
        assert_eq!(empty_page(true, true, true).text(), "No results found");
        assert_eq!(empty_page(false, true, false).text(), "No match found");
        assert_eq!(empty_page(false, false, true).text(), "Your library is empty");
        assert_eq!(empty_page(false, false, false).text(), "Category is empty");
        assert!(shows_global_search(true, true));
        assert!(!shows_global_search(true, false));
        assert_eq!(SEARCH_DEBOUNCE, Duration::from_millis(250));
    }
}
