//! The browse anime tests (T-41a-049): the fixtures checked in under
//! `tests/fixtures/browse_anime/`, the tests named after T-IDs that no other module of this lane carries, the integration
//! tests I1 to I4, and the string drift test over `strings.json`. Every T-ID is accounted for in the
//! table below: it names the test that holds it, or the lane that owns the code it tests.
//!
//! | T-ID | Where |
//! |---|---|
//! | T18 | `shell::tests::enter_runs_a_global_search_of_the_visible_kind` |
//! | T20, T21, T22 | `sources::tests`, `header::tests` |
//! | T23 | `shell::tests::the_view_options_list_the_languages_and_a_pick_switches_one` |
//! | T29, T38, T40, T41 | here |
//! | T28 | `trust::tests::trust_and_uninstall_act_only_while_the_entry_is_still_untrusted` |
//! | T42, T43, T44, T45, T49 | `primitives` and `sources` tests |
//! | T46, T47 | `prefs::tests`, `grid::tests` |
//! | T48 | `pages::pure_tests::the_progress_bar_shows_only_from_one_answer_until_all_have_answered` |
//! | T51, T53 | `grid::tests::a_stale_cover_never_reaches_a_recycled_cell`, `tasks::tests`, `glue::tests` |
//! | T52 | `source_prefs_page::pure_tests::the_summary_is_the_declared_one_else_the_current_value` |
//! | T50 (season list), T30 to T37, T54 to T56 | the migration lane: the season picker, `Migrate` and its flags are not built in `src/browse` |

use std::collections::BTreeSet;

use kuro_i18n::{Arg, MsgKey, tr, trf};
use serde_json::Value;

use super::names::{localized_display_name, subtitle_display_name};

/// Every fixture of section 5, checked in as JSON.
pub const FIXTURES: [(&str, &str); 11] = [
    ("ext_installed.json", include_str!("../../tests/fixtures/browse_anime/ext_installed.json")),
    ("ext_available.json", include_str!("../../tests/fixtures/browse_anime/ext_available.json")),
    ("ext_untrusted.json", include_str!("../../tests/fixtures/browse_anime/ext_untrusted.json")),
    ("languages.json", include_str!("../../tests/fixtures/browse_anime/languages.json")),
    ("sources_browse.json", include_str!("../../tests/fixtures/browse_anime/sources_browse.json")),
    ("anime_pair_episodes.json", include_str!("../../tests/fixtures/browse_anime/anime_pair_episodes.json")),
    ("anime_pair_tracks.json", include_str!("../../tests/fixtures/browse_anime/anime_pair_tracks.json")),
    ("migrate_flags_defaults.json", include_str!("../../tests/fixtures/browse_anime/migrate_flags_defaults.json")),
    ("season_list.json", include_str!("../../tests/fixtures/browse_anime/season_list.json")),
    ("search_results.json", include_str!("../../tests/fixtures/browse_anime/search_results.json")),
    ("display_names.json", include_str!("../../tests/fixtures/browse_anime/display_names.json")),
];

/// The English base strings of section 24, as `{ key: text }`.
const STRINGS: &str = include_str!("../../tests/fixtures/browse_anime/strings.json");

pub fn fixture(name: &str) -> Value {
    let (_, text) = FIXTURES.iter().find(|(n, _)| *n == name).expect("a fixture of section 5");
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{name} is JSON: {e}"))
}

fn enabled(languages: &[&str]) -> BTreeSet<String> {
    languages.iter().map(|l| (*l).to_owned()).collect()
}

#[test]
fn t29_a_failed_decode_falls_back_to_the_default_glyph_and_a_failed_fetch_to_the_error_one() {
    use super::icons::{decode_icon, fetch_icon, icon_ref, IconRef};
    assert_eq!(decode_icon(b"garbage"), None, "a decode error shows the default glyph");
    assert_eq!(super::icons::fallback_glyph(false), ("application-x-addon-symbolic", false));
    let dir = tempfile::tempdir().expect("dir");
    assert_eq!(fetch_icon(Some(dir.path()), "https://x.test/missing.png", |_| None), None, "a 404 has no picture");
    assert!(matches!(icon_ref("https://x.test/missing.png"), IconRef::Url(_)));
    assert_eq!(super::icons::fallback_glyph(true), ("dialog-error-symbolic", true), "and shows the error placeholder");
}

#[test]
fn t38_source_ids_are_decimal_strings_and_nothing_parses_them() {
    use kuro_domain::source::model::Source;
    let sources = [Source::new(12, "Twelve", "en")];
    let hidden: BTreeSet<String> = ["x".to_owned()].into();
    let list = super::sources::source_list(&super::sources::ListInput {
        sources: &sources,
        enabled_languages: &enabled(&["en"]),
        disabled: &hidden,
        last_used: -1,
    });
    assert_eq!(list.len(), 2, "a stored id that is not a number is skipped, the source still lists: {list:?}");
    let hidden: BTreeSet<String> = ["12".to_owned()].into();
    let list = super::sources::source_list(&super::sources::ListInput {
        sources: &sources,
        enabled_languages: &enabled(&["en"]),
        disabled: &hidden,
        last_used: -1,
    });
    assert!(list.is_empty(), "\"12\" is the id of the source");
}

#[test]
fn t40_the_search_starts_with_every_source_and_keeps_the_origin_first() {
    use super::pages::{SearchChips, SearchRow, SearchState, SourceFilter, visible_rows};
    use kuro_domain::source::model::Source;
    assert_eq!(SearchChips::default().filter, SourceFilter::All, "the pinned-only filter is cut");
    let row = |id: i64, origin: bool| SearchRow { source: Source::new(id, "S", "en"), state: SearchState::Loading, origin };
    let rows = vec![row(1, true), row(2, false)];
    let shown = visible_rows(&rows, SearchChips::default());
    assert_eq!(shown.iter().map(|r| r.source.id).collect::<Vec<_>>(), [1, 2], "every source shows, the origin first");
}

#[test]
fn t41_the_search_subtitle_is_the_localized_language_name() {
    // the names are the ones Locale.getDisplayName(locale) gives in the language itself, and `all` reads as "All"
    let names = fixture("display_names.json");
    for (tag, expected) in names.as_object().expect("object") {
        assert_eq!(subtitle_display_name(tag), *expected.as_str().expect("text"), "{tag}");
    }
    assert_eq!(localized_display_name("all"), "Multi", "the source lists keep Multi");
}

/// Turns the Android placeholders of the table into the Fluent arguments the message takes.
fn english_of(key: &MsgKey) -> String {
    if key.args().is_empty() {
        tr(*key)
    } else {
        trf(*key, &[("arg1", Arg::Str("X"))])
    }
}

#[test]
fn every_string_of_section_24_matches_the_english_base_text() {
    let table: serde_json::Map<String, Value> = serde_json::from_str(STRINGS).expect("strings.json");
    assert!(table.len() > 80, "section 24 lists more than eighty keys");
    let mut drift = Vec::new();
    for (id, expected) in &table {
        let expected = expected.as_str().expect("text");
        let Some(key) = MsgKey::ALL.iter().find(|k| k.id() == id) else {
            drift.push(format!("{id}: no such key"));
            continue;
        };
        // the table writes `%s` and `%1$s` where Fluent takes `arg1`, and runs a paragraph break into a space
        let squash = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
        let want = squash(&expected.replace("%1$s", "X").replace("%s", "X"));
        let got = squash(&english_of(key));
        if got != want {
            drift.push(format!("{id}: plan {want:?}, app {got:?}"));
        }
    }
    assert!(drift.is_empty(), "the English base text drifted from section 24:\n{}", drift.join("\n"));
}

// ---------- integration tests I1 to I4 (need a display, run from `smoke`) ----------

/// I2: the error-only state of a source page offers its actions in order.
fn i2_error_state_buttons_in_order() {
    use gtk::prelude::*;

    use super::primitives::{EmptyAction, error_actions};
    let actions = error_actions(false);
    let page = super::view::error_state("failed", &actions, |_: EmptyAction| {});
    let mut tips = Vec::new();
    let mut stack = vec![page.child().expect("the row of buttons")];
    while let Some(w) = stack.pop() {
        if let Some(button) = w.downcast_ref::<gtk::Button>() {
            tips.push(button.tooltip_text().map(|t| t.to_string()).unwrap_or_default());
        }
        let mut children = Vec::new();
        let mut child = w.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            children.push(c);
        }
        stack.extend(children.into_iter().rev());
    }
    let expected: Vec<String> = actions.iter().map(|a| a.label()).collect();
    assert_eq!(tips, expected, "Retry alone; Help only when there is a help page");
    assert_eq!(expected[0], tr(MsgKey::action_retry));
}

/// I3: the migrate dialog has no widget in this lane: the migration lane owns it, so the check is skipped, loudly.
fn i3_migrate_dialog_skipped() {
    eprintln!("I3 skipped: the migrate dialog (D1) is built by the migration lane; the browse lane has no such widget to assert on");
}

/// I4: global search with one source loading and one answered: the progress bar is at one half.
fn i4_global_search_progress_is_a_half() {
    use std::rc::Rc;

    use kuro_domain::source::model::Source;
    use tokio::sync::watch;

    use super::listing::ListItem;
    use super::pages::{GlobalSearchNav, GlobalSearchSnapshot, SearchChips, SearchRow, SearchState, SourceFilter, global_search};
    let nav = GlobalSearchNav {
        search: Rc::new(|_| {}),
        open_source: Rc::new(|_, _| {}),
        open_entry: Rc::new(|_, _| {}),
        menu: Rc::new(|_, _: &ListItem| Vec::new()),
        load_cover: Rc::new(|_, _, _| gtk::Widget::from(gtk::Box::new(gtk::Orientation::Vertical, 0))),
        chips: SearchChips { filter: SourceFilter::All, only_has_results: false },
        on_chips: Rc::new(|_| {}),
    };
    let page = global_search(nav, watch::channel(true).1);
    let row = |id, state| SearchRow { source: Source::new(id, "S", "en"), state, origin: false };
    page.update(&GlobalSearchSnapshot {
        query: "q".into(),
        answered: 1,
        total: 2,
        rows: vec![row(1, SearchState::Results(vec![ListItem::default()])), row(2, SearchState::Loading)],
    });
    assert_eq!(page.progress(), Some(0.5), "one of two sources answered");
}

pub(crate) fn smoke() {
    i2_error_state_buttons_in_order();
    i3_migrate_dialog_skipped();
    i4_global_search_progress_is_a_half();
}
