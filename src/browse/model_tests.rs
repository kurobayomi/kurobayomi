//! The unit tests of the browse model. The code lives in the modules named in each section header below, so the tests
//! sit here, one module per model module, and call the code where it is. None of them needs a display, a runtime or the network (T-40b-084). The
//! property tests of the same task are at the end of this file (`group_key_cmp`) and in `names` (`primary_cmp`) and
//! `global_search` (`sort_items`, `apply`).

use std::cmp::Ordering;
use std::collections::BTreeSet;

use kuro_domain::source::model::Source;

fn source(id: i64, name: &str, lang: &str) -> Source {
    Source::new(id, name, lang)
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// Module `source_list`.
mod source_list {
    use super::*;
    use crate::browse::sources::{ListInput, Section, SourceRow, source_list};

    /// One word per row: the header kind or the source name, with `*` on the copy under Last used.
    fn shape(rows: &[SourceRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                SourceRow::Header(Section::LastUsed) => "last".to_owned(),
                SourceRow::Header(Section::Language(lang)) => format!("lang:{lang}"),
                SourceRow::Item(source) if source.is_used_last => format!("{}*", source.name),
                SourceRow::Item(source) => source.name.clone(),
            })
            .collect()
    }

    fn build(sources: &[Source], last_used: i64) -> Vec<SourceRow> {
        let (enabled, disabled) = (set(&["en", "ja", "de", ""]), set(&[]));
        source_list(&ListInput { sources, enabled_languages: &enabled, disabled: &disabled, last_used })
    }

    #[test]
    fn build_source_list_groups_and_order() {
        let sources = [
            source(1, "alpha", "en"),
            source(2, "beta", "ja"),
            source(3, "gamma", ""),
            source(4, "delta", "en"),
            source(5, "echo", "de"),
            source(6, "foxtrot", "en"),
        ];
        let rows = build(&sources, 6);
        assert_eq!(
            shape(&rows),
            ["last", "foxtrot*", "lang:de", "echo", "lang:en", "alpha", "delta", "foxtrot", "lang:ja", "beta", "lang:", "gamma"],
            "last used, languages in code order, the empty language last; the header keys are exact"
        );
    }

    #[test]
    fn build_source_list_last_used_duplicate() {
        let rows = build(&[source(1, "x", "en")], 1);
        assert_eq!(shape(&rows), ["last", "x*", "lang:en", "x"], "E-SL-04: a last-used source is listed under both");
    }
}

/// Module `source_filter`.
mod source_filter {
    use super::*;
    use crate::browse::lang_pages::sources_filter_groups;
    use crate::browse::names::compare_languages;
    use kuro_domain::source::toggles::{flip_disabled, toggle_language};

    #[test]
    fn languages_with_sources_enabled_first() {
        let sources = [source(1, "A", "en"), source(2, "B", "ja"), source(3, "C", "de"), source(4, "D", "all")];
        let groups = sources_filter_groups(&sources, &set(&["ja", "all"]));
        let langs: Vec<&str> = groups.iter().map(|g| g.lang.as_str()).collect();
        assert_eq!(&langs[..2], ["all", "ja"], "the enabled languages first, `all` first among them");
        assert!(groups[..2].iter().all(|g| g.enabled) && groups[2..].iter().all(|g| !g.enabled));
        assert!(compare_languages(langs[2], langs[3]) != Ordering::Greater, "the others follow in language order");
    }

    #[test]
    fn languages_with_sources_no_merge() {
        let sources = [source(1, "A", "zh-CN"), source(2, "B", "zh-Hans")];
        let groups = sources_filter_groups(&sources, &set(&["zh-CN", "zh-Hans"]));
        assert_eq!(groups.len(), 2, "E-SF-03: two tags with one display name keep both groups");
    }

    #[test]
    fn toggle_language_and_source() {
        let mut languages = set(&["en"]);
        toggle_language(&mut languages, "ja");
        assert_eq!(languages, set(&["en", "ja"]), "a toggle adds");
        toggle_language(&mut languages, "ja");
        assert_eq!(languages, set(&["en"]), "and a second one removes: the pair is idempotent");
        let mut hidden = BTreeSet::new();
        assert!(flip_disabled(&mut hidden, 5), "the first flip hides");
        assert_eq!(hidden, set(&["5"]), "as a decimal string");
        assert!(!flip_disabled(&mut hidden, 5), "the second shows again");
        assert!(hidden.is_empty());
    }
}

/// Module `migrate_sources`.
mod migrate_sources {
    use super::*;
    use kuro_domain::source::migration::sort_migration_sources;
    use kuro_prefs::source::{MigrationDirection, MigrationSortingMode};

    fn stub(id: i64) -> Source {
        Source { is_stub: true, ..Source::new(id, "", "") }
    }

    fn ids(list: &[(Source, i64)]) -> Vec<i64> {
        list.iter().map(|(source, _)| source.id).collect()
    }

    #[test]
    fn sort_migrate_sources_asc_desc_stub() {
        let list = || vec![(source(1, "Beta", "en"), 1), (stub(9), 1), (source(2, "Alpha", "en"), 1), (source(3, "Same", "en"), 1), (source(4, "Same", "en"), 1)];
        let asc = sort_migration_sources(list(), MigrationSortingMode::ALPHABETICAL, MigrationDirection::ASCENDING);
        assert_eq!(ids(&asc), [9, 2, 1, 3, 4], "E-MS-03: stubs first ascending; ties keep their input order");
        let desc = sort_migration_sources(list(), MigrationSortingMode::ALPHABETICAL, MigrationDirection::DESCENDING);
        assert_eq!(ids(&desc), [3, 4, 1, 2, 9], "stubs last descending; ties still keep their input order");
    }

    #[test]
    fn sort_migrate_total_mode() {
        let list = || vec![(source(1, "A", "en"), 5), (source(2, "B", "en"), 1), (source(3, "C", "en"), 3)];
        assert_eq!(ids(&sort_migration_sources(list(), MigrationSortingMode::TOTAL, MigrationDirection::ASCENDING)), [2, 3, 1]);
        assert_eq!(ids(&sort_migration_sources(list(), MigrationSortingMode::TOTAL, MigrationDirection::DESCENDING)), [1, 3, 2]);
    }
}

/// Module `collate`.
mod collate {
    use super::*;
    use crate::browse::names::case_insensitive_cmp;

    #[test]
    fn case_insensitive_cmp_matches_java_fixtures() {
        assert_eq!(case_insensitive_cmp("abc", "ABD"), Ordering::Less);
        assert_eq!(case_insensitive_cmp("a", "A"), Ordering::Equal);
        assert_eq!(case_insensitive_cmp("Straße", "strasse"), Ordering::Greater, "a per-character fold: U+00DF is above U+0073, as in Java");
    }
}

/// Module `listing`.
mod listing {
    use kuro_domain::source::browse::BrowseQuery;

    
    use crate::browse::route::search_listing;

    #[test]
    fn to_remote_blank_query() {
        assert_eq!(search_listing(""), BrowseQuery::Search(String::new()), "B-EC-09: a blank search still goes out as an empty text");
    }
}

/// Module `filter_tree`.
mod filter_tree {
    use kuro_source::filter::{Filter, SortSelection, TriStateState};
    use kuro_source::filter_list::FilterList;
    use kuro_source::filter_sheet::sort_descending;

    use crate::browse::filters::genre_search;

    #[test]
    fn tristate_cycle_and_raw() {
        assert_eq!(TriStateState::Ignore.next(), TriStateState::Include);
        assert_eq!(TriStateState::Include.next(), TriStateState::Exclude);
        assert_eq!(TriStateState::Exclude.next(), TriStateState::Ignore);
        for raw in 0..=2 {
            assert_eq!(TriStateState::from_raw(raw).expect("0..2 are states").raw(), raw);
        }
        assert!(TriStateState::from_raw(3).is_err(), "B-EC-20: 3 is an error");
        assert!(TriStateState::from_raw(-1).is_err());
    }

    #[test]
    fn sort_click_matrix() {
        let first = SortSelection::tap(None, 2);
        assert_eq!(first, SortSelection { index: 2, ascending: true }, "D-BR-01: with no selection the row is ascending");
        let flipped = SortSelection::tap(Some(first), 2);
        assert!(!flipped.ascending, "E-FL-03: the same row flips");
        let moved = SortSelection::tap(Some(flipped), 0);
        assert_eq!(moved, SortSelection { index: 0, ascending: false }, "E-FL-04: another row takes the selection and keeps the direction");
    }

    #[test]
    fn sort_arrow_matrix() {
        let selection = Some(SortSelection { index: 1, ascending: true });
        assert_eq!(sort_descending(selection, 1), Some(false), "the selected row shows its direction");
        assert_eq!(sort_descending(Some(SortSelection { index: 1, ascending: false }), 1), Some(true));
        assert_eq!(sort_descending(selection, 0), None, "another row has no arrow");
        assert_eq!(sort_descending(None, 0), None);
    }

    #[test]
    fn select_text_clamp() {
        let values = || vec!["A".to_owned(), "B".to_owned()];
        assert_eq!(Filter::select_at("S", values(), 1).select_value(), Some("B"));
        assert_eq!(Filter::select_at("S", values(), 7).select_value(), Some("A"), "B-EC-21: an index out of range falls back to 0");
        assert_eq!(Filter::select_at("S", values(), -1).select_value(), Some("A"));
        assert_eq!(Filter::select_at("S", Vec::new(), 3).select_value(), None, "no values, no value");
    }

    #[test]
    fn apply_genre_group_include() {
        let defaults = FilterList(vec![Filter::group("Genres", vec![Filter::tri_state("Action"), Filter::check_box("Drama")])]);
        let (query, tri) = genre_search(&defaults, "action");
        assert_eq!(query, None, "a matching genre filter is searched with no text");
        let Filter::Group { state, .. } = &tri.0[0] else { panic!("the group stays") };
        assert!(matches!(state[0], Filter::TriState { state: TriStateState::Include, .. }), "a tri-state is set to include");
        let (_, check) = genre_search(&defaults, "DRAMA");
        let Filter::Group { state, .. } = &check.0[0] else { panic!("the group stays") };
        assert!(matches!(state[1], Filter::CheckBox { state: true, .. }), "a checkbox is checked");
    }

    #[test]
    fn apply_genre_select_index_into_values() {
        let defaults = FilterList(vec![Filter::select("Genre", vec!["Any".into(), "Action".into(), "Drama".into()])]);
        let (query, filters) = genre_search(&defaults, "drama");
        assert_eq!(query, None);
        assert_eq!(filters.0[0].select_value(), Some("Drama"), "B-EC-22: the index is into the full values list, `Any` included");
    }

    #[test]
    fn apply_genre_first_match_wins() {
        let defaults = FilterList(vec![
            Filter::group("One", vec![Filter::check_box("Drama")]),
            Filter::group("Two", vec![Filter::check_box("Drama")]),
        ]);
        let (_, filters) = genre_search(&defaults, "Drama");
        let checked = |at: usize| matches!(&filters.0[at], Filter::Group { state, .. } if matches!(state[0], Filter::CheckBox { state: true, .. }));
        assert!(checked(0) && !checked(1), "the search leaves the outer loop at the first match");
    }

    #[test]
    fn apply_genre_no_match() {
        let defaults = FilterList(vec![Filter::group("Genres", vec![Filter::check_box("Action")]), Filter::text("Author")]);
        let (query, filters) = genre_search(&defaults, "Slice of Life");
        assert_eq!(query.as_deref(), Some("Slice of Life"), "no filter holds it: the name is the search text");
        assert_eq!(filters, defaults, "and the tree is unchanged");
    }
}

/// Module `browse_source`.
mod browse_source {
    use kuro_domain::source::browse::BrowseQuery;
    use kuro_source::filter::Filter;
    use kuro_source::filter_list::FilterList;

    use crate::browse::filters::SourceFilters;
    use crate::browse::listing::Listing;
    use crate::browse::nav::{BackAction, FilterSeed, back_action, filter_seed};

    fn defaults() -> FilterList {
        FilterList(vec![Filter::check_box("Flag"), Filter::text("Author")])
    }

    #[test]
    fn browse_state_search_restarts_generation() {
        let mut listing = Listing::new(BrowseQuery::Popular);
        let first = listing.ask().expect("the first page").generation;
        listing.restart(BrowseQuery::Search("a".into()));
        let second = listing.ask().expect("a new listing asks again").generation;
        listing.restart(BrowseQuery::Search("a".into()));
        let third = listing.ask().expect("and again").generation;
        assert!(first < second && second < third, "B-EC-08: every search is a new generation, the same text included");
    }

    #[test]
    fn browse_state_search_uses_defaults_not_state_filters() {
        let filters = SourceFilters::new(defaults());
        filters.edit(|sheet| sheet.toggle_check(&[0]).expect("check"));
        assert_eq!(filter_seed(false), FilterSeed::SourceDefaults);
        assert_eq!(filters.for_search(false), defaults(), "B§4.6.1: from Popular the search starts from the defaults");
        assert_eq!(filter_seed(true), FilterSeed::Keep, "a running search keeps its own");
    }

    #[test]
    fn browse_state_chip_resets_filters() {
        let filters = SourceFilters::new(defaults());
        filters.edit(|sheet| sheet.set_text(&[1], "Oda".into()).expect("text"));
        filters.reset();
        assert_eq!(filters.current(), defaults(), "B§4.6: a Popular or Latest chip puts the sheet back to the defaults");
    }

    #[test]
    fn browse_state_navigate_up() {
        assert_eq!(back_action(true, "text"), BackAction::ClearQuery, "B-EC-10: an active search with text clears it first");
        assert_eq!(back_action(true, ""), BackAction::Leave);
        assert_eq!(back_action(false, "text"), BackAction::Leave);
        assert_eq!(back_action(false, ""), BackAction::Leave);
    }

}

/// Module `paging`.
mod paging {
    use kuro_domain::source::browse::BrowseQuery;
    use kuro_i18n::{MsgKey, tr};
    use kuro_prefs::library::LibraryDisplayMode;

    use crate::browse::content::ContentPlan;
    use crate::browse::grid::{PAGE_SIZE, prefetch_due};
    use crate::browse::listing::{ListItem, ListedPage, Listing};

    fn page(count: usize, has_next: bool) -> ListedPage {
        ListedPage { items: (0..count).map(|i| ListItem { id: i as i64, url: format!("/{i}"), title: format!("T{i}"), ..ListItem::default() }).collect(), has_next }
    }

    #[test]
    fn paging_load_outcome() {
        let plan = |listing: &Listing| listing.plan(true, false, false, LibraryDisplayMode::CompactGrid);
        let mut listing = Listing::new(BrowseQuery::Popular);
        let ask = listing.ask().expect("page 1");
        assert_eq!(listing.deliver(ask.generation, page(0, false)), Some(0), "an empty page is a failure");
        assert!(matches!(plan(&listing), ContentPlan::Error { ref message, .. } if *message == tr(MsgKey::no_results_found)), "B-EC-03: no results");

        let mut listing = Listing::new(BrowseQuery::Popular);
        let ask = listing.ask().expect("page 1");
        assert!(listing.fail(ask.generation, "offline".into()));
        assert!(matches!(plan(&listing), ContentPlan::Error { ref message, .. } if message == "offline"), "an error before any item is the source's message");

        let mut listing = Listing::new(BrowseQuery::Popular);
        let first = listing.ask().expect("page 1");
        listing.deliver(first.generation, page(3, true));
        let second = listing.ask().expect("page 2 follows");
        assert_eq!((first.page, second.page), (1, 2), "B-EC-05: the next key is the next page number");
        listing.deliver(second.generation, page(3, false));
        assert_eq!(listing.ask(), None, "no next page: the listing has ended");
    }

    #[test]
    fn should_prefetch_matrix() {
        assert!(prefetch_due(0, PAGE_SIZE), "the last visible plus a page reaches the total");
        assert!(!prefetch_due(0, PAGE_SIZE + 1));
        assert!(prefetch_due(1, PAGE_SIZE + 1));
        let mut listing = Listing::new(BrowseQuery::Popular);
        let ask = listing.ask().expect("page 1");
        listing.deliver(ask.generation, page(30, true));
        assert!(!listing.wants_more(4), "E-PG-01: with 30 loaded, the 5th cell is too early");
        assert!(listing.wants_more(5));
        let ask = listing.ask().expect("page 2");
        assert!(!listing.wants_more(29), "E-PG-02: not while a page is in flight");
        listing.fail(ask.generation, "offline".into());
        assert!(!listing.wants_more(29), "nor while an append error shows");
        listing.clear_errors();
        assert!(listing.wants_more(29), "Retry clears it");
    }
}

/// Module `favorites`.
mod favorites {
    use kuro_domain::category::target::{AddTarget, resolve_add_target};

    use crate::browse::favorite::{FavoriteWrite, LongClick, cover_stamp_after_remove, long_click, toggle_write};

    #[test]
    fn favorite_change_add_remove() {
        assert_eq!(toggle_write(false, 500), FavoriteWrite { favorite: true, date_added: 500 }, "adding stamps the time");
        assert_eq!(toggle_write(true, 500), FavoriteWrite { favorite: false, date_added: 0 }, "removing clears it to 0");
        assert_eq!(cover_stamp_after_remove(2, 7), Some(7), "the cover stamp is set when files were deleted");
        assert_eq!(cover_stamp_after_remove(0, 7), None);
    }

    #[test]
    fn add_favorite_plan_matrix() {
        assert_eq!(resolve_add_target(-1, &[1, 2]), AddTarget::Ask, "default -1 with categories: the chooser");
        assert_eq!(resolve_add_target(-1, &[]), AddTarget::Categories(Vec::new()), "no categories: nothing to ask");
        assert_eq!(resolve_add_target(0, &[1, 2]), AddTarget::Categories(Vec::new()), "0: the default category");
        assert_eq!(resolve_add_target(2, &[1, 2]), AddTarget::Categories(vec![2]), "an existing id: that one");
        assert_eq!(resolve_add_target(9, &[1, 2]), AddTarget::Ask, "B-EC-23: a missing id with categories asks");
    }

    #[test]
    fn long_click_matrix() {
        assert_eq!(long_click(true, true), LongClick::RemoveConfirm, "B-EC-26: a favourite wins over a duplicate");
        assert_eq!(long_click(true, false), LongClick::RemoveConfirm);
        assert_eq!(long_click(false, true), LongClick::DuplicateDialog);
        assert_eq!(long_click(false, false), LongClick::Add);
    }

    #[test]
    fn duplicate_title_unicode() {
        // both sides of the comparison are lowercased with the Unicode rules (B-EC-25)
        assert_eq!("Éclair".to_lowercase(), "éCLAIR".to_lowercase());
        assert_eq!("ΟΔΟΣ".to_lowercase(), "οδος", "a final capital sigma lowercases to the final form, not to a plain sigma");
        assert_ne!("İ".to_lowercase(), "i", "the dotted capital I is not a plain i");
    }
}

/// Modules `global_search` and `migrate_search`.
mod global_search {
    use super::*;
    use crate::browse::global_search::{Candidate, GlobalSearch};
    use crate::browse::listing::ListItem;
    use crate::browse::pages::{SearchState, SourceFilter};

    fn cand(id: i64, name: &str) -> Candidate {
        let source = source(id, name, "en");
        Candidate { source, package: String::new() }
    }

    fn item(id: i64) -> ListItem {
        ListItem { id, title: format!("M{id}"), ..ListItem::default() }
    }

    fn search_of(candidates: Vec<Candidate>, origin: Option<i64>) -> GlobalSearch {
        GlobalSearch::new(candidates, set(&["en"]), BTreeSet::new(), origin, false)
    }

    fn row_ids(search: &GlobalSearch) -> Vec<i64> {
        search.rows().iter().map(|row| row.source.id).collect()
    }

    #[test]
    fn global_sort_items_nonempty_first() {
        let mut search = search_of(vec![cand(1, "Alpha"), cand(2, "Beta"), cand(3, "Gamma")], None);
        search.set_query(Some("q"));
        let plan = search.set_filter(SourceFilter::All).expect("a search");
        search.apply(plan.generation, 3, Ok(vec![item(30)]));
        search.apply(plan.generation, 2, Ok(Vec::new()));
        assert_eq!(row_ids(&search), [3, 1, 2], "non-empty first, then by name");
    }

    #[test]
    fn global_sort_keeps_collisions() {
        let mut search = search_of(vec![cand(1, "Same"), cand(2, "Same")], None);
        search.set_query(Some("q"));
        search.set_filter(SourceFilter::All).expect("a search");
        assert_eq!(row_ids(&search), [1, 2], "GS-EC-04: two sources with one key are both kept");
    }

    #[test]
    fn global_begin_search_noop_rules() {
        let mut search = search_of(vec![cand(1, "A")], None);
        search.set_query(Some("   "));
        assert_eq!(search.search(), None, "GS-EC-01: a blank query is a no-op");
        search.set_query(Some("one"));
        assert!(search.search().is_some());
        assert_eq!(search.search(), None, "GS-EC-02: the same query and filter do nothing");
    }

    #[test]
    fn global_begin_search_same_query_keeps_results() {
        let mut search = search_of(vec![cand(1, "A"), cand(2, "B")], None);
        search.set_query(Some("one"));
        let plan = search.search().expect("first search");
        search.apply(plan.generation, 1, Ok(vec![item(10)]));
        assert_eq!(search.set_filter(SourceFilter::All), None, "the Pinned chip is cut: every source was already searched");
        assert!(matches!(&search.rows().iter().find(|r| r.source.id == 1).expect("A").state, SearchState::Results(items) if items.len() == 1), "A keeps its result");
    }

    #[test]
    fn global_apply_stale_generation_dropped() {
        let mut search = search_of(vec![cand(1, "A")], None);
        search.set_query(Some("one"));
        let old = search.search().expect("generation 1");
        search.set_query(Some("two"));
        let new = search.search().expect("generation 2");
        assert!(!search.apply(old.generation, 1, Ok(vec![item(1)])), "R-01: an old generation changes nothing");
        assert!(search.apply(new.generation, 1, Ok(vec![item(2)])));
        assert!(!search.apply(new.generation, 1, Ok(vec![item(3)])), "R-08: an answered row is not changed again");
    }

    #[test]
    fn global_visible_has_results() {
        let mut search = search_of(vec![cand(1, "A"), cand(2, "B"), cand(3, "C")], None);
        search.set_query(Some("q"));
        let plan = search.search().expect("a search");
        search.apply(plan.generation, 1, Ok(vec![item(1)]));
        search.apply(plan.generation, 2, Ok(Vec::new()));
        assert_eq!(search.visible().len(), 3, "off: every row shows");
        search.set_only_has_results(true);
        assert_eq!(search.visible().iter().map(|r| r.source.id).collect::<Vec<_>>(), [1], "GS-EC-09: only rows with results; empty and loading rows go");
    }

    #[test]
    fn global_progress_total() {
        let mut search = search_of(vec![cand(1, "A"), cand(2, "B"), cand(3, "C")], None);
        search.set_query(Some("q"));
        let plan = search.search().expect("a search");
        assert_eq!(search.progress(), (0, 3));
        search.apply(plan.generation, 1, Ok(Vec::new()));
        search.apply(plan.generation, 2, Err("timeout".into()));
        assert_eq!(search.progress(), (2, 3), "an error counts as an answer");
    }

    #[test]
    fn migrate_search_origin_first() {
        let mut search = search_of(vec![cand(1, "Alpha"), cand(2, "Zulu"), cand(3, "Mid")], Some(2));
        search.set_query(Some("title"));
        search.set_filter(SourceFilter::All).expect("a search over every source");
        assert_eq!(row_ids(&search)[0], 2, "E-MG-01: the origin source's row is first, though it sorts last by name");
        assert!(search.rows()[0].origin);
    }
}

/// The property test of `group_key_cmp` (T-40b-084).
mod props {
    use proptest::prelude::*;

    use super::*;
    use crate::browse::sources::{Section, section_cmp};

    fn section() -> impl Strategy<Value = Section> {
        prop_oneof![
            Just(Section::LastUsed),
            proptest::sample::select(vec!["", "en", "ja", "de", "pt-BR", "zh-CN", "all", "\u{1F600}", "\u{FF21}"]).prop_map(|lang| Section::Language(lang.to_owned())),
            proptest::collection::vec(any::<char>(), 0..4).prop_map(|chars| Section::Language(chars.into_iter().collect())),
        ]
    }

    proptest! {
        #[test]
        fn group_key_cmp_is_a_total_order(a in section(), b in section(), c in section()) {
            prop_assert_eq!(section_cmp(&a, &a), Ordering::Equal);
            prop_assert_eq!(section_cmp(&a, &b), section_cmp(&b, &a).reverse());
            if section_cmp(&a, &b) != Ordering::Greater && section_cmp(&b, &c) != Ordering::Greater {
                prop_assert!(section_cmp(&a, &c) != Ordering::Greater);
            }
            // equal keys are the same group: a total order, not only a preorder
            if section_cmp(&a, &b) == Ordering::Equal {
                prop_assert_eq!(&a, &b);
            }
        }
    }
}
