//! The updates tests (T-42a-060). They live here and call the same public code of those crates. [`fixtures`] is
//! the shared fixture of area 42: temporary manga and anime databases and their rows. The tests that already exist under
//! another name in the module of the code they cover are not repeated here: `history_next_units_*` (history),
//! `selection_*` (selection), `group_by_local_date_*` (kuro-domain) and the storage ones named below.
//!
//! Sections: [`domain`] is 5.1, [`data`] is 5.2, [`ui`] is 5.3 and [`shared`] is 5.4.

pub mod fixtures {
    //! The database and row builders of area 42.

    use std::sync::Arc;

    use kuro_data::entry::{Entry, EntryRepo};
    use kuro_data::item::{Item, ItemRepo};
    use kuro_db::handle::Db;
    use kuro_platform::kind::Kind;

    pub struct Fixture {
        pub _dir: tempfile::TempDir,
        pub db: Arc<Db>,
    }

    pub fn test_db() -> Fixture {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("test.db");
        let writer = kuro_db::schema_unified::open_file(&path).expect("open db");
        Fixture { db: Arc::new(Db::new(&path, writer).expect("db handle")), _dir: dir }
    }

    /// An entry row of `kind`; `favorite` says whether it is in the library.
    pub fn entry(db: &Arc<Db>, kind: Kind, title: &str, favorite: bool) -> i64 {
        EntryRepo::new(db.clone())
            .insert(&Entry { favorite, source: 7, url: format!("u/{title}"), title: title.into(), ..Entry::create(kind) })
            .expect("insert entry")
    }

    /// `n` items of an entry in reading order (the first is the oldest).
    pub fn items(db: &Arc<Db>, kind: Kind, entry_id: i64, n: i64) -> Vec<i64> {
        let rows: Vec<Item> = (0..n)
            .map(|i| Item {
                entry_id,
                url: format!("{entry_id}/c{i}"),
                name: format!("Item {i}"),
                number: i as f64,
                source_order: n - i,
                ..Item::create(kind)
            })
            .collect();
        ItemRepo::new(db.clone()).add_all(&rows).expect("add items").into_iter().map(|c| c.id).collect()
    }
}

mod domain {

    use chrono::{FixedOffset, TimeZone, Utc};

    use crate::updates::storage::{Selection as Pick, StorageRow, build_snapshot, donut_arcs, entry_color};

    // 6: the updates window

    #[test]
    fn window_is_three_calendar_months() {
        assert_eq!(kuro_domain::UPDATES_WINDOW_MONTHS, 3);
        let now = Utc.with_ymd_and_hms(2024, 5, 31, 10, 0, 0).single().expect("a time");
        let expected = Utc.with_ymd_and_hms(2024, 2, 29, 10, 0, 0).single().expect("a time").timestamp_millis();
        assert_eq!(kuro_domain::updates::window::three_months_ago(&now), expected, "31 May of a leap year gives 29 Feb");
        let zone = FixedOffset::east_opt(2 * 3600).expect("a zone");
        let local = zone.with_ymd_and_hms(2026, 10, 8, 0, 30, 0).single().expect("a time");
        let back = kuro_domain::updates::window::three_months_ago(&local);
        assert_eq!(back, zone.with_ymd_and_hms(2026, 7, 8, 0, 30, 0).single().expect("a time").timestamp_millis(), "calendar months on the local wall clock");
    }

    // 21 to 27: the storage vectors

    fn row(entry_id: i64, size: i64, category_id: i64) -> StorageRow {
        StorageRow::new(entry_id, format!("title {entry_id}"), size, None, 1, category_id)
    }

    #[test]
    fn color_is_deterministic() {
        // EC-42-T-06: the same id gives the same colour every time and in every process (golden values)
        assert_eq!(entry_color(7), entry_color(7));
        assert_eq!(entry_color(1), [2, 92, 193, 255]);
        assert_eq!(entry_color(2), [151, 86, 206, 255]);
        assert_eq!(entry_color(7), [50, 13, 215, 255]);
    }

    #[test]
    fn donut_empty_total_has_no_arcs() {
        // EC-42-T-11
        assert!(donut_arcs(&build_snapshot(Vec::new(), Pick::All, &[0])).is_empty());
        assert!(donut_arcs(&build_snapshot(vec![row(1, 0, 0)], Pick::All, &[0])).is_empty(), "entries of 0 bytes make no arc");
    }

    #[test]
    fn donut_angles_sum_to_180() {
        let snapshot = build_snapshot(vec![row(1, 300, 0), row(2, 100, 0), row(3, 600, 0)], Pick::All, &[0]);
        let degrees: f64 = donut_arcs(&snapshot).iter().map(|arc| arc.sweep * 180.0).sum();
        assert!((degrees - 180.0).abs() < 1e-9, "the half ring is 180 degrees, got {degrees}");
    }
}

mod data {
    use kuro_domain::history::repo::HistoryRepo;
    use kuro_platform::kind::Kind;
    use kuro_domain::updates::repo::UpdatesRepo;

    use super::fixtures::{entry, items, test_db};
    use crate::updates::storage_data::library_entries;

    fn titles(repo: &HistoryRepo) -> Vec<String> {
        repo.list(Kind::Manga, "").expect("list").into_iter().map(|r| r.title).collect()
    }

    fn searched(repo: &HistoryRepo, query: &str) -> Vec<String> {
        repo.list(Kind::Manga, query).expect("list").into_iter().map(|r| r.title).collect()
    }

    #[test]
    fn manga_list_latest_chapter_only() {
        // EC-42-H-01: one row per manga, its newest chapter
        let f = test_db();
        let m = entry(&f.db, Kind::Manga, "M", true);
        let ids = items(&f.db, Kind::Manga, m, 3);
        let repo = HistoryRepo::new(f.db.clone());
        repo.upsert(Kind::Manga, ids[0], 100, 1).expect("record");
        repo.upsert(Kind::Manga, ids[1], 200, 1).expect("record");
        let rows = repo.list(Kind::Manga, "").expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].item_id, ids[1]);
    }

    #[test]
    fn manga_list_search_escapes_wildcards() {
        // EC-42-H-02: `%` and `_` in the search text are literal
        let f = test_db();
        let repo = HistoryRepo::new(f.db.clone());
        for (n, title) in ["100%", "1000", "a_b", "axb"].into_iter().enumerate() {
            let m = entry(&f.db, Kind::Manga, title, true);
            repo.upsert(Kind::Manga, items(&f.db, Kind::Manga, m, 1)[0], 100 + n as i64, 1).expect("record");
        }
        assert_eq!(searched(&repo, "%"), ["100%"]);
        assert_eq!(searched(&repo, "a_b"), ["a_b"]);
    }

    #[test]
    fn manga_list_search_ascii_case_insensitive() {
        // EC-42-H-03
        let f = test_db();
        let m = entry(&f.db, Kind::Manga, "Berserk", true);
        let repo = HistoryRepo::new(f.db.clone());
        repo.upsert(Kind::Manga, items(&f.db, Kind::Manga, m, 1)[0], 100, 1).expect("record");
        assert_eq!(searched(&repo, "berserk"), ["Berserk"]);
        assert_eq!(searched(&repo, "BERSERK"), ["Berserk"]);
        assert!(searched(&repo, "vinland").is_empty());
    }

    #[test]
    fn reset_row_hides_from_list_keeps_time() {
        // EC-42-H-11 and EC-42-S-01: a reset row leaves the list but its time stays in the total
        let f = test_db();
        let m = entry(&f.db, Kind::Manga, "M", true);
        let ids = items(&f.db, Kind::Manga, m, 1);
        let repo = HistoryRepo::new(f.db.clone());
        repo.upsert(Kind::Manga, ids[0], 100, 5_000).expect("record");
        let history_id = repo.by_entry(Kind::Manga, m).expect("rows")[0].id;
        repo.reset_one(Kind::Manga, history_id).expect("reset");
        assert!(titles(&repo).is_empty());
        assert_eq!(repo.total_duration_of(Kind::Manga).expect("total"), 5_000);
    }

    #[test]
    fn reset_entry_resets_all_rows() {
        // EC-42-H-12
        let f = test_db();
        let m = entry(&f.db, Kind::Manga, "M", true);
        let ids = items(&f.db, Kind::Manga, m, 2);
        let repo = HistoryRepo::new(f.db.clone());
        repo.upsert(Kind::Manga, ids[0], 100, 1).expect("record");
        repo.upsert(Kind::Manga, ids[1], 200, 1).expect("record");
        repo.reset_entry(Kind::Manga, m).expect("reset");
        assert!(titles(&repo).is_empty());
        assert!(repo.by_entry(Kind::Manga, m).expect("rows").iter().all(|r| r.at == 0));
    }

    #[test]
    fn delete_all_drops_time_read() {
        // EC-42-H-10: clearing the history takes the read duration with it
        let f = test_db();
        let m = entry(&f.db, Kind::Manga, "M", true);
        let repo = HistoryRepo::new(f.db.clone());
        repo.upsert(Kind::Manga, items(&f.db, Kind::Manga, m, 1)[0], 100, 9_000).expect("record");
        assert_eq!(repo.total_duration_of(Kind::Manga).expect("total"), 9_000);
        assert!(repo.clear_all(Kind::Manga));
        assert_eq!(repo.total_duration_of(Kind::Manga).expect("total"), 0);
    }

    #[test]
    fn latest_manga_not_limited_to_max_row() {
        // behaviour 1.1: the newest read decides, not the chapter with the highest number or the highest row id
        let f = test_db();
        let (a, b) = (entry(&f.db, Kind::Manga, "A", true), entry(&f.db, Kind::Manga, "B", true));
        let (ca, cb) = (items(&f.db, Kind::Manga, a, 2), items(&f.db, Kind::Manga, b, 1));
        let repo = HistoryRepo::new(f.db.clone());
        repo.upsert(Kind::Manga, ca[1], 100, 1).expect("record");
        repo.upsert(Kind::Manga, cb[0], 200, 1).expect("record");
        repo.upsert(Kind::Manga, ca[0], 300, 1).expect("record");
        let latest = repo.latest(Kind::Manga).expect("latest").expect("a row");
        assert_eq!((latest.entry_id, latest.item_id), (a, ca[0]), "the lower chapter of A was read last");
    }

    /// The feed cut-off and the date a manga joined the library in the fixtures below.
    const AFTER: i64 = 1_700_000_000_000;
    const ADDED: i64 = 1_700_000_100_000;

    fn dated(db: &std::sync::Arc<kuro_db::handle::Db>, manga_id: i64, name: &str, fetch: i64, upload: i64) -> i64 {
        use kuro_data::item::{Item, ItemRepo};
        let row = Item { entry_id: manga_id, url: name.into(), name: name.into(), date_fetch: fetch, date_upload: upload, ..Item::create(Kind::Manga) };
        ItemRepo::new(db.clone()).add_all(&[row]).expect("add chapter")[0].id
    }

    fn library_manga(db: &std::sync::Arc<kuro_db::handle::Db>, title: &str, favorite: bool) -> i64 {
        use kuro_data::entry::{Entry, EntryRepo};
        EntryRepo::new(db.clone())
            .insert(&Entry { favorite, date_added: ADDED, source: 1, url: title.into(), title: title.into(), ..Entry::create(Kind::Manga) })
            .expect("insert manga")
    }

    fn feed(db: &std::sync::Arc<kuro_db::handle::Db>) -> Vec<i64> {
        UpdatesRepo::new(db.clone()).recent(Kind::Manga, AFTER).expect("feed").iter().map(|r| r.item_id).collect()
    }

    #[test]
    fn window_uses_upload_date() {
        // EC-42-U-01: the window cuts on the upload date, so an old upload fetched now is not an update
        let f = test_db();
        let m = library_manga(&f.db, "M", true);
        let old = dated(&f.db, m, "old upload", ADDED + 10, AFTER - 10);
        let unknown = dated(&f.db, m, "unknown upload", ADDED + 10, 0);
        let new = dated(&f.db, m, "new upload", ADDED + 10, AFTER + 10);
        let got = feed(&f.db);
        assert_eq!(got, vec![new]);
        assert!(!got.contains(&old) && !got.contains(&unknown));
    }

    #[test]
    fn favorites_only_and_fetched_after_added() {
        let f = test_db();
        let (kept, left) = (library_manga(&f.db, "Kept", true), library_manga(&f.db, "Left", false));
        let fresh = dated(&f.db, kept, "fresh", ADDED + 1, AFTER + 10);
        dated(&f.db, kept, "backfill", ADDED - 1, AFTER + 10);
        dated(&f.db, left, "not in the library", ADDED + 1, AFTER + 10);
        assert_eq!(feed(&f.db), vec![fresh]);
    }

    #[test]
    fn order_by_fetch_desc_and_limit_500() {
        let f = test_db();
        let m = library_manga(&f.db, "M", true);
        let ids: Vec<i64> = (0..600).map(|i| dated(&f.db, m, &format!("c{i}"), ADDED + 1 + i, AFTER + 1 + i)).collect();
        let got = feed(&f.db);
        assert_eq!(got.len(), 500);
        assert_eq!(got[0], ids[599], "newest fetch first");
        assert_eq!(got[499], ids[100], "the 500th is the 501st oldest: the first hundred are cut");
        assert!(got.windows(2).all(|pair| pair[0] > pair[1] || pair[0] != pair[1]));
    }

    #[test]
    fn library_rows_dedupe_across_categories() {
        // an entry in two categories is one storage row
        let f = test_db();
        let m = entry(&f.db, Kind::Manga, "M", true);
        let (a, b) = (kuro_testkit::insert_category(&f.db, Kind::Manga, "a"), kuro_testkit::insert_category(&f.db, Kind::Manga, "b"));
        kuro_data::entry::EntryRepo::new(f.db.clone()).set_categories(m, &[a, b]).expect("links");
        let entries = library_entries(&f.db, Kind::Manga).expect("entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].category_id, a.min(b), "the lowest category stands for it");
    }
}

mod ui {
    use std::cell::Cell;
    use std::rc::Rc;

    use gtk::prelude::*;
    use kuro_domain::history::repo::HistoryRepo;
    use kuro_domain::history::view::{HistoryLine, RemoveAction, remove_action};
    use kuro_domain::history::model::CoverKey;
    use kuro_platform::kind::Kind;

    use super::fixtures::{entry, items, test_db};
    use crate::updates::history::Resume;
    use crate::updates::history_data::{apply_remove, resume_row};
    use crate::updates::updates_page::{BackAction, back_action, cover_click_enabled};

    #[test]
    fn resume_opens_next_unit() {
        // clicking a row opens the unit that follows the one the row names
        let f = test_db();
        let m = entry(&f.db, Kind::Manga, "M", true);
        let ids = items(&f.db, Kind::Manga, m, 3);
        kuro_data::item::ItemRepo::new(f.db.clone())
            .update(&kuro_data::item::ItemUpdate { done: Some(true), ..kuro_data::item::ItemUpdate::new(ids[0]) })
            .expect("mark read");
        let Resume::Open(unit) = resume_row(&f.db, Kind::Manga, m, ids[0]) else { panic!("a unit follows") };
        assert_eq!(unit.id, ids[1], "the reader opens the next chapter");
    }

    #[test]
    fn remove_dialog_checkbox_maps_to_entry_reset() {
        let f = test_db();
        let m = entry(&f.db, Kind::Manga, "M", true);
        let ids = items(&f.db, Kind::Manga, m, 2);
        let repo = HistoryRepo::new(f.db.clone());
        repo.upsert(Kind::Manga, ids[0], 100, 1).expect("record");
        repo.upsert(Kind::Manga, ids[1], 200, 1).expect("record");
        let rows = repo.by_entry(Kind::Manga, m).expect("rows");
        let line = HistoryLine {
            history_id: rows[0].id,
            item_id: ids[0],
            entry_id: m,
            title: "M".into(),
            text: String::new(),
            cover: CoverKey::create(),
        };
        assert_eq!(remove_action(&line, false), RemoveAction::ResetOne { history_id: rows[0].id });
        assert_eq!(remove_action(&line, true), RemoveAction::ResetEntry { entry_id: m });
        apply_remove(&f.db, Kind::Manga, remove_action(&line, false)).expect("reset one");
        assert_eq!(repo.by_entry(Kind::Manga, m).expect("rows").iter().filter(|r| r.at > 0).count(), 1, "without the box one row resets");
        apply_remove(&f.db, Kind::Manga, remove_action(&line, true)).expect("reset entry");
        assert!(repo.by_entry(Kind::Manga, m).expect("rows").iter().all(|r| r.at == 0), "with the box every row resets");
    }

    #[test]
    fn selection_mode_hides_cover_and_indicator_actions() {
        assert!(cover_click_enabled(false));
        assert!(!cover_click_enabled(true), "the cover tap is off while selecting");
    }

    #[test]
    fn escape_clears_selection_before_navigation() {
        assert_eq!(back_action(true), BackAction::ClearSelection);
        assert_eq!(back_action(false), BackAction::Navigate);
    }

    /// Finds a button by its label under `widget`.
    fn find_button(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
        if let Some(button) = widget.downcast_ref::<gtk::Button>() {
            if button.label().as_deref() == Some(label) {
                return Some(button.clone());
            }
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            if let Some(found) = find_button(&c, label) {
                return Some(found);
            }
            child = c.next_sibling();
        }
        None
    }

    #[test]
    fn delete_confirm_calls_delete_once() {
        crate::testing::with_display(|| {
            let _nav = crate::testing::nav();
            let calls = Rc::new(Cell::new(0));
            let counter = Rc::clone(&calls);
            let copy = crate::updates::history_dialog::ConfirmCopy {
                title: "Delete".into(),
                text: "Sure?".into(),
                checkbox: None,
                confirm: "OK".into(),
                cancel: "Cancel".into(),
            };
            crate::updates::history_dialog::show(copy, true, move |_| counter.set(counter.get() + 1), || {});
            let ok = gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|w| find_button(&w, "OK"))
                .next()
                .expect("the dialog has an OK button");
            ok.emit_clicked();
            ok.emit_clicked();
            assert_eq!(calls.get(), 1, "a double click deletes once");
        });
    }
}

mod shared {
    use crate::updates::signals::CacheSignals;

    #[test]
    fn download_cache_broadcast_two_subscribers() {
        // EC-42-T-01: both subscribers see the one change, and a late one reads the flag at once
        let signals = CacheSignals::new();
        let (mut a, mut b) = (signals.subscribe(), signals.subscribe());
        signals.set_initializing(false);
        signals.notify_changed();
        assert!(a.try_recv().is_ok() && b.try_recv().is_ok());
        let late = signals.initializing();
        assert!(!*late.borrow(), "a late subscriber sees `initializing == false`");
    }
}
