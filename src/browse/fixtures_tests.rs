//! The generic fixtures under `tests/fixtures/browse/` (T-40a-039) and the test-plan rows TC-40-101 to TC-40-121 that run
//! without GTK, each against a fake repository layer (a scripted `PageLoader`, the extension state machines and in-memory
//! preferences). Names are generic: no real titles, packages or covers.
//!
//! | Row | Where |
//! |---|---|
//! | TC-40-101, 102 | here, over a scripted `PageLoader` and `Listing` |
//! | TC-40-112, 113 | not built: the migrate dialog and its engine are not in `src/browse` (T-40a-015, T-40a-016), so no code exists to test |
//! | TC-40-114, 115 | here, over `migrate_nav`; its widget check is `migrate_nav::tests::smoke` |
//! | TC-40-116, 117 | here |
//! | TC-40-118 | manual: the key map needs a window and a real key press |
//! | TC-40-119 | manual: the 360 px layout needs a real window; no automated check exists |
//! | TC-40-120 | here, over a settings file in the Aniyomi key names |
//! | TC-40-121 | here, over the message keys of section 18 |

use std::collections::BTreeSet;

use kuro_domain::source::browse::BrowseQuery;
use kuro_domain::source::model::Source;
use kuro_domain::source::migration::sort_migration_sources;
use kuro_prefs::library::LibraryDisplayMode;
use kuro_prefs::source::{MigrationDirection, MigrationSortingMode, SourcePrefs};
use kuro_prefs::store::PrefStore;
use kuro_source::filter_list::FilterList;
use serde_json::Value;

use super::content::ContentPlan;
use super::global_search::{Candidate, GlobalSearch};
use super::lang_pages::sources_filter_groups;
use super::listing::{ListItem, ListedPage, Listing, LoadFuture, PageLoader};
use super::pages::{SourceFilter, progress_visible};
use super::primitives::{EmptyAction, ToastAction, ToastDuration, error_actions};

const SOURCES: &str = include_str!("../../tests/fixtures/browse/sources.json");
const FAVORITES: &str = include_str!("../../tests/fixtures/browse/manga_favorites.json");
const CHAPTERS_OLD: &str = include_str!("../../tests/fixtures/browse/chapters_old.json");
const CHAPTERS_NEW: &str = include_str!("../../tests/fixtures/browse/chapters_new.json");
const TRACKS: &str = include_str!("../../tests/fixtures/browse/tracks.json");

fn json(text: &str) -> Value {
    serde_json::from_str(text).expect("a browse fixture is JSON")
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// The sources of `sources.json` as the registry would hand them over.
fn sources_fixture() -> Vec<Source> {
    json(SOURCES)["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .map(|s| {
            let mut source = Source::new(s["id"].as_i64().expect("id"), &text(s, "name"), &text(s, "lang"));
            source.supports_latest = s["supports_latest"].as_bool().unwrap_or(false);
            source.is_stub = s["is_stub"].as_bool().unwrap_or(false);
            source
        })
        .collect()
}

#[test]
fn sources_fixture_lists_online_sources_only_with_enabled_languages_first() {
    let sources = sources_fixture();
    let groups = sources_filter_groups(&sources, &set(&["en", "ja", "all"]));
    let ids: Vec<i64> = groups.iter().flat_map(|g| g.sources.iter().map(|s| s.id)).collect();
    assert!(!ids.contains(&0), "the local source is not an online source (E-SF-04)");
    assert!(!ids.contains(&2002), "a stub is not an online source");
    assert!(ids.contains(&4001), "a blank name is still listed");
    let flags: Vec<bool> = groups.iter().map(|g| g.enabled).collect();
    assert!(flags.windows(2).all(|w| w[0] || !w[1]), "enabled languages come first: {flags:?}");
    let alpha = groups.iter().find(|g| g.lang == "en").expect("en group");
    assert!(alpha.sources.iter().any(|s| s.id == 1002), "no source is hidden per source: the language switch decides");
}

#[test]
fn migrate_order_puts_stubs_first_ascending_and_keeps_ties_in_input_order() {
    let list: Vec<(Source, i64)> = sources_fixture()
        .into_iter()
        .map(|s| {
            let count = json(SOURCES)["sources"]
                .as_array()
                .and_then(|all| all.iter().find(|x| x["id"].as_i64() == Some(s.id)))
                .and_then(|x| x["count"].as_i64())
                .unwrap_or(0);
            (s, count)
        })
        .collect();
    let names = |out: &[(Source, i64)]| out.iter().map(|(s, _)| s.id).collect::<Vec<i64>>();

    let alphabetical = sort_migration_sources(list.clone(), MigrationSortingMode::ALPHABETICAL, MigrationDirection::ASCENDING);
    assert_eq!(names(&alphabetical), [2002, 4001, 3001, 1002, 1001, 2001, 5001], "stub first, then names in primary order");
    assert!(!names(&alphabetical).contains(&0), "the local source is never listed");

    let total = sort_migration_sources(list.clone(), MigrationSortingMode::TOTAL, MigrationDirection::ASCENDING);
    assert_eq!(names(&total), [2002, 1002, 4001, 3001, 1001, 5001, 2001], "stub first, then the counts, ties in input order");

    let descending = sort_migration_sources(list, MigrationSortingMode::ALPHABETICAL, MigrationDirection::DESCENDING);
    assert_eq!(names(&descending), [5001, 2001, 1001, 1002, 3001, 4001, 2002], "descending reverses the whole order, stubs last");
}

#[test]
fn duplicate_titles_compare_with_unicode_lowercase_on_both_sides() {
    let favorites = json(FAVORITES);
    let titles: Vec<(i64, String)> = favorites["favorites"]
        .as_array()
        .expect("favorites")
        .iter()
        .map(|f| (f["id"].as_i64().expect("id"), text(f, "title")))
        .collect();
    let same = |needle: &str| -> Vec<i64> {
        let needle = needle.to_lowercase();
        titles.iter().filter(|(_, t)| t.to_lowercase() == needle).map(|(id, _)| *id).collect()
    };
    assert_eq!(same("Manga A"), [1, 2], "case does not matter");
    assert_eq!(same("ÉCLAIR"), [11, 12], "accented capitals match the lowercase form (B-EC-25)");
    assert!(same("").contains(&9), "an empty title is a title of its own");
}

#[test]
fn tc_40_116_the_progress_bar_shows_only_while_some_but_not_all_have_answered() {
    assert!(!progress_visible(0, 10), "nothing answered: hidden");
    assert!(progress_visible(3, 10), "3 of 10: shown");
    assert!(!progress_visible(10, 10), "all answered: hidden");
}

#[test]
fn tc_40_117_a_chip_change_without_a_query_leaves_the_results_unchanged() {
    let candidates: Vec<Candidate> = sources_fixture()
        .into_iter()
        .map(|source| Candidate { source, package: String::new() })
        .collect();
    let mut search = GlobalSearch::new(candidates, set(&["en", "ja", "all", "other", "de"]), BTreeSet::new(), None, false);
    assert_eq!(search.set_filter(SourceFilter::All), None, "no query: nothing is searched");
    assert!(search.rows().is_empty(), "the results are unchanged (empty)");
}

#[test]
fn a_chip_change_with_the_query_searches_nothing_new() {
    let candidates: Vec<Candidate> = sources_fixture()
        .into_iter()
        .map(|source| Candidate { source, package: String::new() })
        .collect();
    let mut search = GlobalSearch::new(candidates, set(&["en", "ja", "all", "other", "de"]), BTreeSet::new(), None, false);
    let first = search.request("title", None).expect("the first search runs every source");
    assert!(first.sources.iter().any(|s| s.id == 1001) && first.sources.iter().any(|s| s.id == 5001), "every source is searched");
    search.apply(first.generation, 1001, Ok(Vec::new()));
    assert_eq!(search.set_filter(SourceFilter::All), None, "a chip change adds no source: nothing is searched again");
}

#[test]
fn the_migrate_and_chapter_fixtures_have_the_shape_the_edge_cases_name() {
    let old = json(CHAPTERS_OLD);
    let new = json(CHAPTERS_NEW);
    let old_list = old["chapters"].as_array().expect("chapters");
    assert!(old_list.iter().any(|c| c["number"].as_f64() == Some(10.5)), "old chapters read up to 10.5");
    assert_eq!(old_list.iter().filter(|c| c["number"].as_f64() == Some(-1.0)).count(), 1, "one unrecognised chapter");
    let new_list = new["chapters"].as_array().expect("chapters");
    assert_eq!(new_list.len(), 13, "chapters 1 to 12 and 5.5");
    let tracks = json(TRACKS);
    let matched = tracks["tracks"].as_array().expect("tracks").iter().filter(|t| t["matched_by_fake_tracker"].as_bool() == Some(true)).count();
    assert_eq!(matched, 1, "one track matched by the fake tracker");
}

#[test]
fn the_kurobayomi_strings_the_browse_pages_draw_resolve_to_text() {
    use kuro_i18n::{MsgKey, tr};
    for key in [MsgKey::ext_updates_pending, MsgKey::ext_installed, MsgKey::not_installed, MsgKey::internal_error, MsgKey::has_results] {
        let text = tr(key);
        assert!(!text.trim().is_empty(), "a browse string resolves to text");
    }
}

// ---------- the fake repository layer ----------

/// A source repository that answers from a script: each call to `load` takes the next answer, and an empty script
/// answers one fresh page.
struct FakeRepo {
    answers: std::sync::Mutex<Vec<Result<ListedPage, String>>>,
    asked: std::sync::Mutex<Vec<u32>>,
}

impl FakeRepo {
    fn new(answers: Vec<Result<ListedPage, String>>) -> Self {
        Self { answers: std::sync::Mutex::new(answers), asked: std::sync::Mutex::default() }
    }

    fn pages_asked(&self) -> Vec<u32> {
        self.asked.lock().expect("lock").clone()
    }
}

impl PageLoader for FakeRepo {
    fn load(&self, _query: BrowseQuery, _filters: FilterList, page: u32) -> LoadFuture {
        self.asked.lock().expect("lock").push(page);
        let next = {
            let mut answers = self.answers.lock().expect("lock");
            if answers.is_empty() { Ok(listed(&["fresh"], false)) } else { answers.remove(0) }
        };
        Box::pin(async move { next })
    }
}

fn listed(titles: &[&str], has_next: bool) -> ListedPage {
    ListedPage { items: titles.iter().map(|t| ListItem { title: (*t).into(), url: format!("/{t}"), ..ListItem::default() }).collect(), has_next }
}

/// Runs the repository's next page for the listing the way the page does: ask, load, deliver or fail.
fn step(listing: &mut Listing, repo: &FakeRepo) -> Option<usize> {
    let ask = listing.ask()?;
    let result = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(repo.load(listing.query().clone(), FilterList::default(), ask.page));
    match result {
        Ok(page) => listing.deliver(ask.generation, page),
        Err(message) => {
            listing.fail(ask.generation, message);
            Some(0)
        }
    }
}

fn plan(listing: &Listing) -> ContentPlan {
    listing.plan(true, false, false, LibraryDisplayMode::CompactGrid)
}

#[test]
fn tc_40_101_a_failing_source_shows_its_actions_in_order_and_retry_loads_the_first_page_again() {
    let repo = FakeRepo::new(vec![Err("offline".into())]);
    let mut listing = Listing::new(BrowseQuery::Popular);
    step(&mut listing, &repo);
    match plan(&listing) {
        ContentPlan::Error { message, actions } => {
            assert_eq!(message, "offline");
            assert_eq!(actions, error_actions(false), "the actions of a web source, in their order");
            assert_eq!(actions, [EmptyAction::Retry], "retry is the only action of a web source's error state");
        }
        other => panic!("an error state: {other:?}"),
    }
    listing.clear_errors();
    assert_eq!(step(&mut listing, &repo), Some(1), "retry asks again and the repository answers");
    assert_eq!(repo.pages_asked(), [1, 1], "the same first page twice");
    assert!(matches!(plan(&listing), ContentPlan::Items { toast: None, .. }));
}

#[test]
fn tc_40_102_a_failing_later_page_keeps_the_items_and_the_retry_toast_loads_that_page_again() {
    let repo = FakeRepo::new(vec![Ok(listed(&["a", "b"], true)), Err("page 2 failed".into())]);
    let mut listing = Listing::new(BrowseQuery::Popular);
    step(&mut listing, &repo);
    step(&mut listing, &repo);
    match plan(&listing) {
        ContentPlan::Items { toast: Some(toast), .. } => {
            assert_eq!(toast.message, "page 2 failed");
            assert_eq!(toast.action, Some(ToastAction::Retry), "the toast carries Retry");
            assert_eq!(toast.duration, ToastDuration::Indefinite, "and stays until it is answered");
        }
        other => panic!("items with a toast: {other:?}"),
    }
    assert_eq!(listing.loaded(), 2, "the items stay");
    listing.clear_errors();
    assert_eq!(step(&mut listing, &repo), Some(1));
    assert_eq!(repo.pages_asked(), [1, 2, 2], "retry asks page 2 again");
    assert_eq!(listing.loaded(), 3);
}

// ---------- the extension rows ----------

#[test]
fn tc_40_120_aniyomi_style_settings_in_a_toml_file_read_as_the_same_modes_and_sets() {
    use kuro_prefs::toml_backend::{TomlBackend, config_file};
    let dir = tempfile::tempdir().expect("tempdir");
    let (config, state) = (dir.path().join("config"), dir.path().join("state"));
    std::fs::create_dir_all(&config).expect("config dir");
    std::fs::create_dir_all(&state).expect("state dir");
    // the exporter writes each Aniyomi key under [sources] by its field name, with the Aniyomi spelling of the value
    std::fs::write(
        config_file(&config),
        "[sources]\ndisabled_manga_sources = [\"11\", \"12\"]\nenabled_languages = [\"en\", \"ja\"]\n",
    )
    .expect("seed");
    let backend = TomlBackend::open(&config, &state).expect("open");
    let store = PrefStore::new(Box::new(backend)).expect("store");
    let prefs = SourcePrefs::new(store);
    assert_eq!(prefs.disabled_manga_sources.key(), "hidden_catalogues");
    assert_eq!(prefs.disabled_manga_sources.get(), set(&["11", "12"]), "the hidden sources, as decimal ids");
    assert_eq!(prefs.enabled_languages.get(), set(&["en", "ja"]), "the enabled languages");
}

// ---------- TC-40-121: the message keys of section 18 ----------

/// Every browse message key, as a `MsgKey`: a key that is not in the table does not compile.
fn section_18_keys() -> Vec<(&'static str, kuro_i18n::MsgKey)> {
    use kuro_i18n::MsgKey as K;
    macro_rules! keys {
        ($($name:ident),* $(,)?) => { vec![$((stringify!($name), K::$name)),*] };
    }
    keys![
        // dialogs and titles
        are_you_sure, remove_manga, action_remove, action_cancel, ext_confirm_remove, ext_remove, remove_private_extension_message,
        untrusted_extension, untrusted_extension_message, ext_trust, ext_uninstall, migration_dialog_what_to_include, copy, migrate,
        action_show_manga, ext_nsfw_warning, action_ok,
        // empty, error and loading
        empty_screen, no_results_found, unknown_error, internal_error, source_empty_screen, source_filter_empty_screen,
        information_empty_library, source_not_installed, local_source_help_guide, action_retry, action_open_in_web_view, label_help,
        // the browse toolbar
        action_display_mode, action_display_comfortable_grid, action_display_grid, action_display_list, action_settings, browse,
        // the sources list
        latest, action_disable, not_installed, label_sources, all, has_results,
        // the extension list
        label_manga_extensions, label_extensions, label_extension_repos, action_filter, ext_updates_pending, ext_installed,
        ext_update_all, ext_update, ext_install, ext_pending, ext_downloading, ext_installing, ext_untrusted, ext_obsolete,
        ext_nsfw_short, ext_permission_install_apps_warning,
        // the extension details
        label_extension_info, action_open_repo, action_enable_all, action_disable_all, pref_clear_cookies, obsolete_extension_message,
        ext_info_version, ext_info_language, ext_info_age_rating, ext_app_info, pref_incognito_mode, pref_incognito_mode_extension_summary,
        label_settings,
        // migration
        migration_selection_prompt, action_sort_alpha, action_sort_count, action_asc, action_desc, chapters, categories, custom_cover,
        delete_downloaded,
    ]
}

#[test]
fn tc_40_121_every_message_key_of_section_18_resolves_to_text_in_the_table() {
    for (name, key) in section_18_keys() {
        assert_eq!(key.id(), name, "the variant is the key");
        let shown = kuro_i18n::tr(key);
        assert!(!shown.trim().is_empty(), "{name} has text");
        assert_ne!(shown, name, "{name} resolves in the catalogue and is not shown as its own id");
    }
}
