//! Golden order of the library sorts on the four-entry library fixture. The expected
//! orders live in `golden/sort_orders.json` and were derived by hand from the vectors, not by running the code.

use std::collections::BTreeMap;

use kuro_platform::kind::Kind;
use kuro_domain::library::model::LibraryRow;
use kuro_domain::library::order::{SortContext, sort_page};
use kuro_domain::library::page::LibraryItem;
use kuro_platform::sort::{Direction, LibrarySort, SortType};

/// (id, title, total, read, last read, latest upload, fetched, date added); every entry is in the Default page
/// except id 3, and id 2 is also in Hold.
fn fixture() -> Vec<LibraryItem> {
    let rows = [
        (1, "Alpha", 10, 10, 500, 100, 10, 1),
        (2, "beta", 8, 2, 900, 300, 20, 2),
        (3, "Gamma", 5, 0, 0, 200, 30, 3),
        (4, "Émile", 3, 3, 100, 50, 5, 4),
    ];
    rows.iter()
        .map(|&(id, title, total, done, last, latest, fetched, added)| LibraryItem {
            row: LibraryRow {
                id,
                source: 7,
                title: title.into(),
                category: 0,
                total,
                done,
                bookmarks: 0,
                fillermarks: 0,
                latest_upload: latest,
                item_fetched_at: fetched,
                last_activity: last,
                last_update: 0,
                date_added: added,
                status: 1,
                fetch_interval: 0,
                viewer_flags: 0,
                always_update: true,
                next_update: 0,
            },
            download_count: -1,
            unread_count: -1,
            is_local: false,
            source_language: String::new(),
        })
        .collect()
}

fn token_sort(token: &str) -> LibrarySort {
    let (kind_token, dir) = token.split_once(',').expect("ASCENDING or DESCENDING");
    let ty = SortType::from_token(Kind::Manga, kind_token).expect("known sort token");
    let dir = if dir == "ASCENDING" { Direction::Ascending } else { Direction::Descending };
    LibrarySort { ty, dir }
}

fn ids_for(page: Vec<LibraryItem>, sort: LibrarySort) -> Vec<i64> {
    let scores = BTreeMap::new();
    let ctx = SortContext { tracker_scores: &scores, random_seed: 0 };
    sort_page(page, sort, &ctx).iter().map(|i| i.row.id).collect()
}

#[test]
fn every_golden_order_matches_the_sort_engine() {
    let golden: serde_json::Value = serde_json::from_str(include_str!("golden/sort_orders.json")).expect("valid JSON");
    let all = fixture();
    let default_page: Vec<LibraryItem> = all.iter().filter(|i| [1, 2, 4].contains(&i.row.id)).cloned().collect();
    let hold_page: Vec<LibraryItem> = all.iter().filter(|i| [2, 3].contains(&i.row.id)).cloned().collect();
    for (page_name, page) in [("default_page", default_page), ("hold_page", hold_page)] {
        let expected = golden[page_name].as_object().expect("page object");
        assert!(!expected.is_empty());
        for (token, ids) in expected {
            let want: Vec<i64> = ids.as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
            let got = ids_for(page.clone(), token_sort(token));
            assert_eq!(got, want, "{page_name} {token}");
        }
    }
}
