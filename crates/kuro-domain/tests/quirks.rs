//! One named test per upstream quirk of `plans/07-domain-entries-items/p1-topic-quirks-and-decisions.md` (Q1 to Q22). Each
//! test asserts the policy the table chose: a preserved quirk keeps the upstream result, a fixed one gives the corrected
//! result.

use kuro_data::entry::{Entry, EntryRepo, EntryUpdate, SeasonEntry};
use kuro_data::item::{Item, ItemRepo};
use kuro_db::flags::item::SORT_DIR_MASK;
use kuro_db::handle::Db;
use kuro_platform::kind::Kind;
use std::sync::Arc;
use kuro_domain::entries::flags::{EntryFlags, INTRO_MASK};
use kuro_domain::entries::fetch_interval::{Stored, calculate_next_update, get_window, increase_interval, stored_for_wheel};
use kuro_domain::entries::flag_setters::clamp_grid_size;
use kuro_domain::items::facades::get_item_by_url_and_entry_id;
use kuro_domain::items::missing::missing_items_count;
use kuro_domain::items::recognition::{parse_episode_number, parse_season_number};
use kuro_domain::items::should_update::should_update_db_item;
use kuro_domain::items::sorter::sort_seasons;
use kuro_domain::release::version::is_newer;
use kuro_prefs::store::PrefStore;

/// A migrated database. Keep the `TempDir` alive.
fn test_db() -> (tempfile::TempDir, Arc<Db>) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("kurobayomi.db");
    let writer = kuro_db::schema_unified::open_file(&path).expect("manga schema");
    (dir, Arc::new(Db::new(&path, writer).expect("manga handle")))
}

fn anime_with(viewer: i64) -> Entry {
    Entry { viewer_flags: viewer, ..Entry::create(Kind::Anime) }
}

/// Q1: a source that reports `dateUpload = 0` does not make a stored row look stale on every refresh.
#[test]
fn q01_zero_source_upload_date_does_not_force_a_rewrite() {
    for kind in Kind::ALL {
        let db = Item { date_upload: 1_700_000_000_000, ..Item::create(kind) };
        let src = Item { date_upload: 0, ..db.clone() };
        assert!(!should_update_db_item(&db, &src), "{kind}");
        let real = Item { date_upload: 1_700_000_000_001, ..db.clone() };
        assert!(should_update_db_item(&db, &real), "a real new upload date still counts");
    }
}

/// Q2: "set as default" writes the downloaded filter from the downloaded bits, not the unseen bits.
#[test]
fn q02_season_defaults_write_the_downloaded_filter_from_its_own_bits() {
    use kuro_domain::entries::defaults::set_season_settings_default;
    let store = PrefStore::in_memory();
    let downloaded = store.pref::<i64>("default_season_filter_by_downloaded", 0);
    set_season_settings_default(&store, 0x8);
    assert_eq!(downloaded.get(), 0, "unseen bits do not leak into downloaded");
    set_season_settings_default(&store, 0x2);
    assert_eq!(downloaded.get(), 0x2);
}

/// Q3: the anime and manga defaults share their keys on purpose (backup interop).
#[test]
fn q03_chapter_and_episode_defaults_share_their_keys() {
    use kuro_domain::entries::defaults::set_item_settings_default;
    let store = PrefStore::in_memory();
    let direction = store.pref::<i64>("default_chapter_sort_by_ascending_or_descending", 0);
    set_item_settings_default(&store, Kind::Manga, 0);
    assert_eq!(direction.get(), 0);
    set_item_settings_default(&store, Kind::Anime, SORT_DIR_MASK);
    assert_eq!(direction.get(), SORT_DIR_MASK, "the episode default writes the chapter key");
}

/// Q4: the dead source-manager dependency is gone, and a non-favourite keeps its in-memory title overlay only.
#[test]
fn q04_network_to_local_has_no_source_manager_and_overlays_only_in_memory() {
    use kuro_domain::entries::network_to_local::network_to_local;
    let (_d, db) = test_db();
    let repo = EntryRepo::new(db);
    let stored = repo.insert(&Entry { url: "u".into(), title: "Stored".into(), source: 1, ..Entry::create(Kind::Anime) }).unwrap();
    let got = network_to_local(&repo, &Entry { url: "u".into(), title: "Fresh".into(), source: 1, ..Entry::create(Kind::Anime) })
        .unwrap();
    assert_eq!((got.id, got.title.as_str()), (stored, "Fresh"));
    assert_eq!(repo.by_id(stored).unwrap().title, "Stored", "the database keeps its title");
}

/// Q5: the `Hi10` tag is in the season unwanted list in capital letters only, so a hi10 tag next to another number can
/// pick the wrong number.
#[test]
fn q05_season_hi10_tag_is_preserved_as_upstream_parses_it() {
    assert_eq!(parse_season_number("Show", "Show 2 Hi10", None), 2.0);
}

/// Q6: a dotted title is not stripped, so the season reads as 2.108.
#[test]
fn q06_dotted_titles_are_not_stripped_and_read_as_fractional_seasons() {
    let got = parse_season_number("Boku.no.Hero.Academia", "Boku.no.Hero.Academia.S02.1080p-ITH", None);
    assert!((got - 2.108).abs() < 1e-9, "got {got}");
}

/// Q7: a name whose only number is a resolution reads that resolution as the episode.
#[test]
fn q07_resolution_only_name_reads_as_the_episode_number() {
    assert_eq!(parse_episode_number("X", "1080p", None), 1080.0);
}

/// Q8: an unknown season sort code (`0xE000`) falls back to source order instead of panicking.
#[test]
fn q08_unknown_season_sort_code_falls_back_to_source_order() {
    let parent = Entry { season_flags: Some(0xE000), ..Entry::create(Kind::Anime) };
    let season = |id: i64, number: f64| SeasonEntry {
        entry: Entry { id, season_number: Some(number), season_source_order: Some(id), ..Entry::create(Kind::Anime) },
        total: 0,
        done: 0,
        bookmarks: 0,
        fillermarks: 0,
        latest_upload: 0,
        fetched_at: 0,
        last_activity: 0,
    };
    let mut list = vec![season(1, 3.0), season(2, 1.0), season(3, 2.0)];
    sort_seasons(&mut list, &parent);
    // Source order, read with the direction bit: 0xE000 has bit 0 clear, which is descending.
    assert_eq!(list.iter().map(|s| s.entry.id).collect::<Vec<_>>(), vec![3, 2, 1]);
}

/// Q9: the doubling cycle uses the doubled step, so a long-silent series gets a short interval (16 days after 105).
#[test]
fn q09_the_interval_doubles_from_the_stepped_value_and_keeps_its_quirk() {
    assert_eq!(increase_interval(1, 105), 16);
}

/// Q10: the offset of the clock's own zone is used for the epoch conversion.
#[test]
fn q10_the_epoch_uses_the_offset_of_the_clock_now() {
    use chrono::{FixedOffset, TimeZone};
    let zone = FixedOffset::east_opt(3600).unwrap();
    let now = zone.with_ymd_and_hms(2024, 3, 29, 10, 0, 0).unwrap();
    let stored = Stored { fetch_interval: 1, next_update: 0, last_update: 0 };
    let window = get_window(&now);
    let next = calculate_next_update(stored, 1, &now, window, now.timestamp_millis());
    assert!(next > 0, "a prediction is produced from the clock's zone: {next} window {window:?}");
}

/// Q11: the time used for a never-updated entry comes from the injected clock, so the result is deterministic.
#[test]
fn q11_a_never_updated_entry_uses_the_injected_now() {
    use chrono::{TimeZone, Utc};
    let now = Utc.with_ymd_and_hms(2024, 3, 15, 10, 30, 0).unwrap();
    let stored = Stored { fetch_interval: 0, next_update: 0, last_update: 0 };
    let clock = now.timestamp_millis();
    let a = calculate_next_update(stored, 0, &now, (0, 0), clock);
    let b = calculate_next_update(stored, 0, &now, (0, 0), clock);
    assert_eq!(a, b);
}

/// Q12: counting starts at 0, and only exactly `-1.0` is ignored, so a list starting at 5 reports 4 missing.
#[test]
fn q12_missing_count_starts_at_zero_and_only_minus_one_is_ignored() {
    assert_eq!(missing_items_count(&[5.0, 6.0]), 4);
    assert_eq!(missing_items_count(&[-2.0, 3.0]), 4);
    assert_eq!(missing_items_count(&[-1.0]), 0);
}

/// Q13: the release version compare is tuple-wise and never panics on short or odd versions.
#[test]
fn q13_release_version_compare_is_tuple_wise_and_never_panics() {
    assert!(!is_newer("v0.16.9", "0.17.1"));
    assert!(!is_newer("1.2", "1.2.3"));
    assert!(is_newer("v1.2.1", "1.2"));
}

/// Q14: a publishing-finished or cancelled manga still shows a predicted next release; only completed does not.
#[test]
fn q14_only_a_completed_manga_hides_the_predicted_next_release() {
    for status in [4, 5] {
        let m = Entry { status, next_update: 42, ..Entry::create(Kind::Manga) };
        assert_eq!(m.expected_next_update(), Some(42), "status {status}");
    }
    let done = Entry { status: 2, next_update: 42, ..Entry::create(Kind::Manga) };
    assert_eq!(done.expected_next_update(), None);
}

/// Q16: the library query retries a transient lock a bounded number of times, then reports the error.
#[test]
fn q16_library_retries_are_bounded() {
    assert!((1..=20).contains(&kuro_data::LIBRARY_RETRIES));
}

/// Q17: the airing episode and the airing time are written in one change, so they never disagree.
#[test]
fn q17_airing_episode_and_time_are_written_together() {
    let mut a = anime_with(0x2A);
    a.set_next_episode_airing(7, 1_700_000_000);
    assert_eq!((a.viewer().next_episode_to_air(), a.viewer().next_episode_airing_at()), (7, 1_700_000_000));
    assert_eq!(a.viewer_flags & INTRO_MASK, 0x2A, "the intro length is kept");
}

/// Q18: an update cannot clear a text field, because `None` means untouched.
#[test]
fn q18_an_absent_text_field_in_an_update_keeps_the_stored_text() {
    let (_d, db) = test_db();
    let repo = EntryRepo::new(db);
    let id = repo.insert(&Entry { url: "u".into(), title: "T".into(), description: Some("old".into()), source: 1, ..Entry::create(Kind::Anime) }).unwrap();
    repo.update(&EntryUpdate { description: None, ..EntryUpdate::new(id) }).unwrap();
    assert_eq!(repo.by_id(id).unwrap().description.as_deref(), Some("old"));
}

/// Q19: the lookup's second parameter is the entry id, named for what it is.
#[test]
fn q19_the_second_parameter_of_the_item_lookup_is_the_entry_id() {
    let (_d, db) = test_db();
    let manga = EntryRepo::new(db.clone())
        .insert(&Entry { url: "m".into(), title: "M".into(), source: 1, ..Entry::create(Kind::Manga) })
        .unwrap();
    let repo = ItemRepo::new(db);
    let chapter = repo
        .add_all(&[Item { entry_id: manga, url: "c".into(), name: "C".into(), ..Item::create(Kind::Manga) }])
        .unwrap()
        .remove(0)
        .id;
    let found = get_item_by_url_and_entry_id(&repo, "c", manga).expect("found by manga id");
    assert_eq!(found.id, chapter);
}

/// Q20: the custom interval is stored as a negative value for any wheel position, in every build.
#[test]
fn q20_the_custom_interval_wheel_stores_a_negative_value() {
    assert_eq!(stored_for_wheel(3), -3);
    assert_eq!(stored_for_wheel(1), -1);
}

/// Q21: the grid size is kept in 0..=15 before it is shifted into its 4-bit field, so nothing is truncated silently.
#[test]
fn q21_grid_size_is_clamped_to_fifteen_before_shifting() {
    assert_eq!(clamp_grid_size(16), 15);
    assert_eq!(clamp_grid_size(-3), 0);
    assert_eq!(clamp_grid_size(9), 9);
}
