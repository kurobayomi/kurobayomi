//! T2-SEARCH-01 to T2-SEARCH-09: the library search grammar.

use kuro_domain::library::search::{SearchFields, matches};

fn fields<'a>(id: i64, title: &'a str, genres: &'a [String]) -> SearchFields<'a> {
    SearchFields { id, title, author: None, artist: None, description: None, source_name: "Src", genres }
}

fn hit(query: &str, f: &SearchFields<'_>) -> bool {
    matches(query, f)
}

#[test]
fn t2_search_01_id_form_matches_only_that_id() {
    let f = fields(42, "x", &[]);
    assert!(hit("id:42", &f));
    assert!(!hit("id:42", &fields(43, "x", &[])));
    // `ID:42` does not match, as upstream: the prefix is read case-insensitively
    // and the value is cut at a case-sensitive `id:`, so the upper-case form never parses and matches nothing
    assert!(!hit("ID:42", &f));
}

#[test]
fn t2_search_02_malformed_id_forms_match_nothing() {
    let f = fields(42, "x", &[]);
    assert!(!hit("id:", &f));
    assert!(!hit("id:abc", &f));
    assert!(!hit("id: 42", &f));
}

#[test]
fn t2_search_03_empty_id_matches_nothing() {
    assert!(!hit("id:", &fields(1, "id", &[])));
}

#[test]
fn t2_search_04_genres_match_exactly_and_the_source_name_by_substring() {
    let genres = vec!["Action Adventure".to_owned()];
    let mut f = fields(1, "x", &genres);
    assert!(!hit("action", &f), "a genre is matched whole");
    assert!(hit("Action Adventure", &f));
    f.source_name = "HiAnime (EN)";
    assert!(!hit("hianime", &fields(2, "x", &[])), "another source name does not match");
    assert!(hit("hianime", &f));
}

#[test]
fn t2_search_05_a_negated_genre_excludes_case_insensitively() {
    let romance = vec!["Romance".to_owned()];
    let drama = vec!["Drama".to_owned()];
    assert!(!hit("-romance", &fields(1, "x", &romance)));
    assert!(hit("-romance", &fields(2, "x", &drama)));
}

#[test]
fn t2_search_06_an_empty_query_matches_every_row() {
    assert!(hit("", &fields(1, "anything", &[])));
}

#[test]
fn t2_search_07_a_lone_dash_matches_only_text_containing_a_dash() {
    assert!(hit("-", &fields(1, "Re-Zero", &[])));
    assert!(!hit("-", &fields(2, "Berserk", &[])));
}

#[test]
fn t2_search_08_the_whole_string_matches_across_a_comma() {
    assert!(hit("zero, starting", &fields(1, "Re:Zero, Starting Life", &[])));
}

#[test]
fn t2_search_09_an_empty_token_matches_every_row() {
    // only the source name and genres see the comma-separated tokens; the title is matched whole
    let mut f = fields(1, "zzz", &[]);
    f.source_name = "alphabet";
    assert!(hit("a,,b", &f));
    assert_eq!(hit("a,,b", &f), hit("a,b", &f), "the empty token adds no constraint");
    assert!(!hit("a,,q", &f));
}
