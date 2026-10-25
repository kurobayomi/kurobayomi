//! Tests of the shared repos (`EntryRepo`, `ItemRepo`, `HistoryRepo`, `TrackRepo`, `StubSourceRepo`): what the unified
//! model must do the same for both kinds, and what a kind must not leak into the other.

use std::time::Duration;

use kuro_db::flags::{item, season};
use kuro_db::handle::Db;
use kuro_db::codec::{FetchType, Memo};
use kuro_platform::kind::Kind;

use crate::entry::{Entry, EntryRepo, EntryUpdate};
use crate::history::HistoryRepo;
use crate::error::DataErrorExt;
use crate::item::{Item, ItemRepo};
use crate::source::StubSourceRepo;
use crate::support::test_db;
use crate::track::{Track, TrackRepo};
use crate::{DataError, LookupOrInsert};

fn entry(kind: Kind, url: &str, favorite: bool) -> Entry {
    Entry { source: 1, status: 1, url: url.into(), title: url.into(), favorite, ..Entry::create(kind) }
}

fn item_of(kind: Kind, entry_id: i64, url: &str) -> Item {
    Item { entry_id, url: url.into(), name: url.into(), ..Item::create(kind) }
}

fn column(db: &Db, sql: &str, id: i64) -> i64 {
    db.read(|c| Ok(c.query_row(sql, [id], |r| r.get(0))?)).unwrap()
}

fn flags_of(db: &Db, id: i64) -> i64 {
    column(db, "SELECT item_flags FROM entries WHERE _id = ?1", id)
}

fn version_of(db: &Db, id: i64) -> i64 {
    column(db, "SELECT version FROM entries WHERE _id = ?1", id)
}

#[test]
fn an_entry_round_trips_for_both_kinds_and_manga_keeps_the_anime_columns_none() {
    let (_d, db) = test_db();
    let repo = EntryRepo::new(db);
    let manga = entry(Kind::Manga, "m", true);
    let anime = Entry { season_number: Some(2.5), ..entry(Kind::Anime, "a", true) };
    let (m, a) = (repo.insert(&manga).unwrap(), repo.insert(&anime).unwrap());
    assert_eq!(repo.by_id(m).unwrap(), Entry { id: m, ..manga });
    assert_eq!(repo.by_id(a).unwrap(), Entry { id: a, ..anime });
    let stored = repo.by_id(m).unwrap();
    assert_eq!((stored.fetch_type, stored.parent_id, stored.season_flags, stored.memo), (None, None, None, None));
    let stored = repo.by_id(a).unwrap();
    assert_eq!((stored.fetch_type, stored.memo), (Some(FetchType::Episodes), Some(Memo::default())));
}

#[test]
fn an_anime_only_field_on_a_manga_entry_fails_the_check() {
    let (_d, db) = test_db();
    let repo = EntryRepo::new(db);
    let m = repo.insert(&entry(Kind::Manga, "m", false)).unwrap();
    let err = repo.update(&EntryUpdate { season_flags: Some(1), ..EntryUpdate::new(m) }).unwrap_err();
    assert!(err.is_constraint(), "{err}");
    let err = repo.insert(&Entry { fetch_type: Some(FetchType::Episodes), ..entry(Kind::Manga, "x", false) }).unwrap_err();
    assert!(err.is_constraint(), "{err}");
}

#[test]
fn lookups_and_lists_are_scoped_by_kind() {
    let (_d, db) = test_db();
    let repo = EntryRepo::new(db);
    let m = repo.insert(&entry(Kind::Manga, "same", true)).unwrap();
    let a = repo.insert(&entry(Kind::Anime, "same", true)).unwrap();
    assert_eq!(repo.by_url_and_source(Kind::Manga, "same", 1).unwrap().unwrap().id, m);
    assert_eq!(repo.by_url_and_source(Kind::Anime, "same", 1).unwrap().unwrap().id, a);
    assert_eq!(repo.favorites(Kind::Manga).unwrap().iter().map(|e| e.id).collect::<Vec<_>>(), [m]);
    assert_eq!(repo.favorites_by_source(Kind::Anime, 1).unwrap().iter().map(|e| e.id).collect::<Vec<_>>(), [a]);
    assert!(repo.by_source(Kind::Anime, 1).unwrap().iter().any(|e| e.id == a), "every entry of the source, favourites too");
    assert_eq!(repo.favorite_counts_by_source(Kind::Manga).unwrap(), [(1, 1)]);
    assert_eq!(repo.all_source_and_url(Kind::Anime).unwrap(), [(1, "same".to_owned())]);
    assert!(matches!(repo.lookup_or_insert(&entry(Kind::Anime, "same", true)).unwrap(), LookupOrInsert::Found(e) if e.id == a));
    assert!(matches!(repo.lookup_or_insert(&entry(Kind::Anime, "new", true)).unwrap(), LookupOrInsert::Inserted(_)));
}

#[test]
fn upcoming_reads_the_statuses_of_the_kind() {
    let (_d, db) = test_db();
    let repo = EntryRepo::new(db);
    let now = 1_700_000_000_000;
    let due = |kind, url: &str| Entry { status: 7, next_update: now + 86_400_000, ..entry(kind, url, true) };
    repo.insert(&due(Kind::Manga, "m")).unwrap();
    let a = repo.insert(&due(Kind::Anime, "a")).unwrap();
    assert!(repo.upcoming(Kind::Manga, now).unwrap().is_empty(), "status 7 is an anime upcoming status only");
    assert_eq!(repo.upcoming(Kind::Anime, now).unwrap().iter().map(|e| e.id).collect::<Vec<_>>(), [a]);
}

#[test]
fn duplicates_compare_titles_in_sql_for_both_kinds() {
    let (_d, db) = test_db();
    let repo = EntryRepo::new(db);
    for kind in Kind::ALL {
        let first = repo.insert(&Entry { title: "Éclair".into(), ..entry(kind, "one", true) }).unwrap();
        let second = repo.insert(&Entry { title: "éclair".into(), ..entry(kind, "two", true) }).unwrap();
        let found = repo.duplicates(kind, first, "ÉCLAIR").unwrap();
        assert_eq!(found.iter().map(|e| e.id).collect::<Vec<_>>(), [second], "{kind}");
    }
}

#[test]
fn a_flag_write_is_one_masked_statement_that_bumps_the_version_for_both_kinds() {
    let (_d, db) = test_db();
    let repo = EntryRepo::new(db.clone());
    for kind in Kind::ALL {
        let id = repo.insert(&entry(kind, "e", true)).unwrap();
        let before = version_of(&db, id);
        assert!(repo.set_item_flags_masked(id, item::SHOW_DONE, item::UNDONE_MASK).unwrap());
        assert!(repo.set_item_flags_masked(id, item::SORT_ASC, item::SORT_DIR_MASK).unwrap());
        assert_eq!(flags_of(&db, id), item::SHOW_DONE | item::SORT_ASC, "{kind}: the other group survives");
        assert_eq!(version_of(&db, id), before + 2, "{kind}");
        assert!(repo.update_item_flags(id, |w| w | item::SORTING_NUMBER).unwrap());
        assert_eq!(flags_of(&db, id), item::SHOW_DONE | item::SORT_ASC | item::SORTING_NUMBER);
        assert!(!repo.update_item_flags(999_999, |w| w).unwrap(), "a missing entry changes nothing");
    }
}

#[test]
fn a_manga_row_only_takes_the_manga_groups_and_has_no_season_flags() {
    let (_d, db) = test_db();
    let repo = EntryRepo::new(db.clone());
    let m = repo.insert(&entry(Kind::Manga, "m", true)).unwrap();
    let a = repo.insert(&entry(Kind::Anime, "a", true)).unwrap();
    repo.set_item_flags_masked(m, item::SHOW_FILLERMARKED, item::FILLERMARKED_MASK).unwrap();
    repo.set_item_flags_masked(a, item::SHOW_FILLERMARKED, item::FILLERMARKED_MASK).unwrap();
    assert_eq!(flags_of(&db, m), 0, "the fillermark group is anime only");
    assert_eq!(flags_of(&db, a), item::SHOW_FILLERMARKED);
    assert!(!repo.set_season_flags_masked(m, season::SORT_ASC, season::SORT_DIR_MASK).unwrap());
    assert!(repo.set_season_flags_masked(a, season::SORT_ASC, season::SORT_DIR_MASK).unwrap());
}

#[test]
fn items_of_both_kinds_share_one_repo() {
    let (_d, db) = test_db();
    let entries = EntryRepo::new(db.clone());
    let items = ItemRepo::new(db);
    let m = entries.insert(&entry(Kind::Manga, "m", false)).unwrap();
    let a = entries.insert(&entry(Kind::Anime, "a", false)).unwrap();
    let chapter = items.add_all(&[item_of(Kind::Manga, m, "same")]).unwrap().remove(0);
    let episode = items
        .add_all(&[Item { total_seconds: Some(1400), summary: Some("s".into()), ..item_of(Kind::Anime, a, "same") }])
        .unwrap()
        .remove(0);
    let stored = items.by_id(chapter.id).unwrap().unwrap();
    assert_eq!((stored.kind, stored.total_seconds, stored.summary, stored.memo), (Kind::Manga, None, None, None));
    let stored = items.by_id(episode.id).unwrap().unwrap();
    assert_eq!((stored.kind, stored.total_seconds, stored.summary.as_deref()), (Kind::Anime, Some(1400), Some("s")));
    assert_eq!(items.by_url(Kind::Manga, "same").unwrap().iter().map(|i| i.id).collect::<Vec<_>>(), [chapter.id]);
    assert_eq!(items.by_url(Kind::Anime, "same").unwrap().iter().map(|i| i.id).collect::<Vec<_>>(), [episode.id]);
}

#[test]
fn an_item_must_have_the_kind_of_its_entry_and_a_bad_batch_stores_nothing() {
    let (_d, db) = test_db();
    let entries = EntryRepo::new(db.clone());
    let items = ItemRepo::new(db);
    let m = entries.insert(&entry(Kind::Manga, "m", false)).unwrap();
    let err = items.add_all(&[item_of(Kind::Manga, m, "ok"), item_of(Kind::Anime, m, "wrong")]).unwrap_err();
    assert!(err.is_constraint(), "{err}");
    assert!(items.by_entry_id(m, false).unwrap().is_empty());
}

#[test]
fn a_duplicate_url_in_one_entry_gives_the_lowest_id_for_both_kinds() {
    let (_d, db) = test_db();
    let entries = EntryRepo::new(db.clone());
    let items = ItemRepo::new(db);
    for kind in Kind::ALL {
        let e = entries.insert(&entry(kind, "e", false)).unwrap();
        let added = items.add_all(&[item_of(kind, e, "dup"), item_of(kind, e, "dup")]).unwrap();
        assert_eq!(items.by_url_and_entry_id("dup", e).unwrap().unwrap().id, added[0].id, "{kind}");
    }
}

#[test]
fn history_is_one_repo_whose_lists_and_totals_are_per_kind() {
    let (_d, db) = test_db();
    let entries = EntryRepo::new(db.clone());
    let items = ItemRepo::new(db.clone());
    let history = HistoryRepo::new(db);
    let m = entries.insert(&entry(Kind::Manga, "m", true)).unwrap();
    let a = entries.insert(&entry(Kind::Anime, "a", true)).unwrap();
    let chapter = items.add_all(&[item_of(Kind::Manga, m, "c")]).unwrap()[0].id;
    let episode = items.add_all(&[item_of(Kind::Anime, a, "e")]).unwrap()[0].id;
    history.upsert(chapter, 100, 5).unwrap();
    history.upsert(episode, 200, 7).unwrap();
    history.upsert(episode, 300, 3).unwrap();
    assert_eq!(history.list(Kind::Manga, "").unwrap().iter().map(|h| (h.entry_id, h.last_at)).collect::<Vec<_>>(), [(m, 100)]);
    let anime = history.list(Kind::Anime, "").unwrap();
    assert_eq!((anime[0].entry_id, anime[0].last_at, anime[0].duration_ms), (a, 300, 10), "an upsert adds its delta");
    assert_eq!((history.total_duration(Kind::Manga).unwrap(), history.total_duration(Kind::Anime).unwrap()), (5, 10));
    history.delete_all(Kind::Manga).unwrap();
    assert!(history.list(Kind::Manga, "").unwrap().is_empty());
    assert_eq!(history.by_entry_id(a).unwrap().len(), 1, "the other kind is untouched");
}

#[test]
fn tracks_are_scoped_by_kind() {
    let (_d, db) = test_db();
    let entries = EntryRepo::new(db.clone());
    let tracks = TrackRepo::new(db.clone());
    let m = entries.insert(&entry(Kind::Manga, "m", true)).unwrap();
    let a = entries.insert(&entry(Kind::Anime, "a", true)).unwrap();
    let mt = tracks.insert(&Track { entry_id: m, last_item: 4.5, ..Track::new(Kind::Manga, 2) }).unwrap();
    let at = tracks.insert(&Track { entry_id: a, total_items: 12, ..Track::new(Kind::Anime, 2) }).unwrap();
    assert_eq!(tracks.all(Kind::Manga).unwrap().iter().map(|t| t.id).collect::<Vec<_>>(), [mt]);
    assert_eq!(tracks.all(Kind::Anime).unwrap().iter().map(|t| t.id).collect::<Vec<_>>(), [at]);
    assert_eq!(tracks.by_id(mt).unwrap().unwrap().kind, Kind::Manga);
    assert_eq!(tracks.by_id(at).unwrap().unwrap().total_items, 12);
    let json = serde_json::to_string(&tracks.by_id(mt).unwrap().unwrap()).unwrap();
    assert_eq!(serde_json::from_str::<Track>(&json).unwrap(), tracks.by_id(mt).unwrap().unwrap());
}

#[test]
fn stub_sources_are_per_kind() {
    let (_d, db) = test_db();
    let manga = StubSourceRepo::new(db.clone(), Kind::Manga);
    let anime = StubSourceRepo::new(db, Kind::Anime);
    manga.upsert(5, "en", "M").unwrap();
    anime.upsert(5, "ja", "A").unwrap();
    assert_eq!(manga.get(5).unwrap().unwrap().name, "M");
    assert_eq!(anime.get(5).unwrap().unwrap().name, "A");
    assert_eq!(manga.all().unwrap().iter().filter(|s| s.id == 5).count(), 1);
}

#[test]
fn clear_non_library_keeps_reset_history_out_and_orphans_favourite_seasons_for_anime_too() {
    let (_d, db) = test_db();
    let entries = EntryRepo::new(db.clone());
    let items = ItemRepo::new(db.clone());
    let history = HistoryRepo::new(db.clone());
    let parent = entries.insert(&Entry { fetch_type: Some(FetchType::Seasons), ..entry(Kind::Anime, "parent", false) }).unwrap();
    let child = entries.insert(&Entry { parent_id: Some(parent), ..entry(Kind::Anime, "child", true) }).unwrap();
    let episode = items.add_all(&[item_of(Kind::Anime, child, "e")]).unwrap()[0].id;
    history.upsert(episode, 0, 4).unwrap();
    let other_manga = entries.insert(&entry(Kind::Manga, "manga", false)).unwrap();

    assert_eq!(entries.clear_non_library(Kind::Anime, &[]).unwrap(), 0, "an empty list changes nothing");
    assert_eq!(entries.clear_non_library(Kind::Anime, &[1]).unwrap(), 1, "the reset history row goes");
    assert!(matches!(entries.by_id(parent), Err(DataError::NotFound)));
    assert_eq!(entries.by_id(child).unwrap().parent_id, None, "the favourite season is orphaned, never deleted");
    assert!(entries.by_id(other_manga).is_ok(), "the other kind is untouched");
    assert!(history.by_entry_id(child).unwrap().is_empty());
}

#[test]
fn deletable_parents_and_the_tree_are_per_kind() {
    let (_d, db) = test_db();
    let entries = EntryRepo::new(db.clone());
    let m = entries.insert(&entry(Kind::Manga, "m", false)).unwrap();
    let parent = entries.insert(&Entry { fetch_type: Some(FetchType::Seasons), ..entry(Kind::Anime, "p", false) }).unwrap();
    let kept = entries.insert(&Entry { parent_id: Some(parent), ..entry(Kind::Anime, "kept", true) }).unwrap();
    let gone = entries.insert(&Entry { parent_id: Some(parent), ..entry(Kind::Anime, "gone", false) }).unwrap();
    assert_eq!(entries.deletable_parents(Kind::Manga).unwrap().iter().map(|d| d.entry_id).collect::<Vec<_>>(), [m]);
    let anime = entries.deletable_parents(Kind::Anime).unwrap();
    assert_eq!((anime[0].entry_id, anime[0].fetch_type), (parent, FetchType::Seasons));
    let (ids, orphaned) = db.read(|c| Ok(crate::entry::non_library_tree(c, Kind::Anime)?)).unwrap();
    assert_eq!((ids, orphaned), (vec![(1, parent), (1, gone)], vec![(1, kept)]));
}

#[tokio::test]
async fn a_subscription_publishes_nothing_for_a_write_of_the_other_kind() {
    let (_d, db) = test_db();
    let repo = EntryRepo::new(db);
    let mut sub = repo.subscribe_favorites_by_source(Kind::Manga, 1);
    let next = async |sub: &mut kuro_db::subscribe::Subscription<Vec<Entry>>| {
        tokio::time::timeout(Duration::from_millis(400), sub.changed()).await.ok().flatten().map(|r| r.expect("query"))
    };
    assert!(next(&mut sub).await.expect("first value").is_empty());
    repo.insert(&entry(Kind::Anime, "a", true)).unwrap();
    assert!(next(&mut sub).await.is_none(), "the wake-up is compared and not published");
    repo.insert(&entry(Kind::Manga, "m", true)).unwrap();
    assert_eq!(next(&mut sub).await.expect("a manga joined").len(), 1);
}

#[test]
fn the_entry_update_helpers_log_and_return_false() {
    let (_d, db) = test_db();
    let repo = EntryRepo::new(db);
    let m = repo.insert(&entry(Kind::Manga, "m", false)).unwrap();
    assert!(crate::compat::update_entry(&repo, &EntryUpdate { title: Some("t".into()), ..EntryUpdate::new(m) }));
    assert!(!crate::compat::update_entry(&repo, &EntryUpdate { parent_id: Some(1), ..EntryUpdate::new(m) }));
    assert!(crate::compat::reset_viewer_flags(&repo, Kind::Manga));
}
