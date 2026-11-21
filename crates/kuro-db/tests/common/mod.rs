//! Row builders shared by the integration tests: a full `EntryRow` and `ItemRow` of either kind.

#![allow(dead_code)]

use kuro_db::codec::{FetchType, GenreList, Memo, UpdateStrategy};
use kuro_db::tables::{EntryRow, ItemRow};
use kuro_platform::kind::Kind;

pub fn entry(kind: Kind, source: i64, url: &str, title: &str) -> EntryRow {
    let anime = kind == Kind::Anime;
    EntryRow {
        id: 0,
        kind,
        source,
        url: url.into(),
        title: title.into(),
        artist: Some("artist".into()),
        author: None,
        description: Some("d".into()),
        genre: Some(GenreList(vec!["a".into(), "b".into()])),
        status: 1,
        thumbnail_url: Some("t".into()),
        favorite: true,
        last_update: Some(5),
        next_update: Some(9),
        initialized: true,
        viewer: 3,
        item_flags: 0x41,
        cover_last_modified: 7,
        date_added: 1_000,
        update_strategy: UpdateStrategy::OnlyFetchOnce,
        calculate_interval: 4,
        last_modified_at: 0,
        favorite_modified_at: None,
        version: 0,
        fetch_type: anime.then_some(FetchType::Episodes),
        parent_id: None,
        season_flags: anime.then_some(0x11),
        season_number: anime.then_some(2.5),
        season_source_order: anime.then_some(3),
        background_url: anime.then(|| "bg".to_owned()),
        background_last_modified: anime.then_some(8),
        memo: anime.then(|| Memo(serde_json::from_str(r#"{"k":1}"#).unwrap())),
    }
}

pub fn item_row(entry_id: i64, kind: Kind, url: &str, n: f64) -> ItemRow {
    let anime = kind == Kind::Anime;
    ItemRow {
        id: 0,
        entry_id,
        kind,
        url: url.into(),
        name: format!("item {url}"),
        scanlator: Some("grp".into()),
        done: false,
        bookmark: true,
        fillermark: anime,
        progress: 12,
        number: n,
        source_order: 1,
        date_fetch: 2_000,
        date_upload: 1_500,
        last_modified_at: 0,
        version: 0,
        total_seconds: anime.then_some(1400),
        summary: anime.then(|| "s".to_owned()),
        preview_url: anime.then(|| "p".to_owned()),
        memo: anime.then(|| Memo(serde_json::from_str(r#"{"m":true}"#).unwrap())),
        page_count: (!anime).then_some(20),
    }
}
