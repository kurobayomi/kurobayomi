//! Read-side query facades of both kinds (`GetManga.kt`, `GetAnime.kt`, `GetMangaFavorites.kt`,
//! `GetDuplicateLibraryManga.kt`, `GetMangaWithChapters.kt`, `GetAnimeWithEpisodesAndSeasons.kt` and the anime twins), over
//! the shared repositories; the kind is a parameter. Error contracts:
//! `get_entry_or_none` logs and returns `None` on any error, the others pass the repository result through.

use kuro_data::DataResult;
use kuro_data::entry::{Entry, EntryRepo, LibraryEntry, SeasonEntry};
use kuro_data::item::{Item, ItemRepo};
use kuro_platform::kind::Kind;

use super::combine::{Joined, Pair, Triple, combine2, combine3};

/// `GetManga.await(id)`, `GetAnime.await(id)`: `None` on any error, including a missing row. The error is logged.
pub fn get_entry_or_none(repo: &EntryRepo, id: i64) -> Option<Entry> {
    match repo.by_id(id) {
        Ok(entry) => Some(entry),
        Err(e) => {
            tracing::error!(id, error = %e, "cannot read entry");
            None
        }
    }
}

/// `GetMangaByUrlAndSourceId.await(url, sourceId)` and its twin: propagates the error, `None` when absent.
pub fn get_entry_by_url_and_source(repo: &EntryRepo, kind: Kind, url: &str, source: i64) -> DataResult<Option<Entry>> {
    repo.by_url_and_source(kind, url, source)
}

/// `GetMangaFavorites.await()`: every row of the kind with `favorite = 1`.
pub fn get_favorites(repo: &EntryRepo, kind: Kind) -> DataResult<Vec<Entry>> {
    repo.favorites(kind)
}

/// `GetMangaFavorites.subscribe(sourceId)` as a one-shot read.
pub fn get_favorites_by_source(repo: &EntryRepo, kind: Kind, source: i64) -> DataResult<Vec<Entry>> {
    repo.favorites_by_source(kind, source)
}

/// `GetDuplicateLibraryManga.await(manga)`: favourites of the kind with the same lower-cased title and a different id.
pub fn get_duplicate_library(repo: &EntryRepo, kind: Kind, e: &Entry) -> DataResult<Vec<Entry>> {
    repo.duplicates(kind, e.id, &e.title.to_lowercase())
}

/// `GetAnimeSeasonsByParentId.await(animeId)`: the seasons of a parent, or an empty list when the read fails
/// (the error is logged).
pub fn get_seasons_by_parent_id(repo: &EntryRepo, parent_id: i64) -> Vec<SeasonEntry> {
    match repo.seasons_by_parent(parent_id) {
        Ok(seasons) => seasons,
        Err(e) => {
            tracing::error!(parent_id, error = %e, "cannot read seasons");
            Vec::new()
        }
    }
}

/// `GetMangaWithChapters.awaitManga(id)`: the row, or the `NotFound` error when it is missing.
pub fn await_entry(repo: &EntryRepo, id: i64) -> DataResult<Entry> {
    repo.by_id(id)
}

/// `GetMangaWithChapters.awaitChapters(id, applyScanlatorFilter)`, `awaitEpisodes(id)`: the items, errors passed through.
pub fn await_items(repo: &ItemRepo, entry_id: i64, apply_scanlator_filter: bool) -> DataResult<Vec<Item>> {
    repo.by_entry_id(entry_id, apply_scanlator_filter)
}

/// `GetAnimeWithEpisodesAndSeasons.awaitSeasons(id)`: the seasons of a parent, errors passed through.
pub fn await_seasons(repo: &EntryRepo, parent_id: i64) -> DataResult<Vec<SeasonEntry>> {
    repo.seasons_by_parent(parent_id)
}

/// `GetMangaWithChapters.subscribe(id, applyScanlatorFilter)`: `(entry, items)`, published once both have a value.
pub fn subscribe_entry_with_items(
    entries: &EntryRepo,
    items: &ItemRepo,
    id: i64,
    apply_scanlator_filter: bool,
) -> Joined<Result<Pair<Option<Entry>, Vec<Item>>, String>> {
    combine2(entries.subscribe_by_id(id), items.subscribe_by_entry_id(id, apply_scanlator_filter))
}

/// `GetAnimeWithEpisodesAndSeasons.subscribe(id)`: `(entry, items, seasons)`, published once all three have a value.
pub fn subscribe_entry_with_items_and_seasons(
    entries: &EntryRepo,
    items: &ItemRepo,
    id: i64,
) -> Joined<Result<Triple<Option<Entry>, Vec<Item>, Vec<SeasonEntry>>, String>> {
    combine3(entries.subscribe_by_id(id), items.subscribe_by_entry_id(id, false), entries.subscribe_seasons_by_parent(id))
}

/// `GetLibraryManga.await()`, `GetLibraryAnime.await()`: every library row of the kind with its aggregates.
pub fn get_library(repo: &EntryRepo, kind: Kind) -> DataResult<Vec<LibraryEntry>> {
    repo.library(kind)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use kuro_data::DataError;
    use kuro_data::entry::EntryUpdate;
    use kuro_data::item::ItemUpdate;
    use kuro_db::codec::FetchType;
    use tokio::sync::watch;

    use super::*;
    use crate::testdb::test_db;

    fn entry(kind: Kind, url: &str, title: &str, favorite: bool) -> Entry {
        Entry { url: url.into(), title: title.into(), favorite, source: 1, ..Entry::create(kind) }
    }

    fn item(kind: Kind, entry_id: i64, url: &str) -> Item {
        Item { entry_id, url: url.into(), name: url.into(), number: 1.0, ..Item::create(kind) }
    }

    /// Waits until a published value satisfies `done`, skipping intermediate ones.
    async fn wait_for<T>(rx: &mut watch::Receiver<Option<Result<T, String>>>, done: impl Fn(&T) -> bool) -> Arc<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        loop {
            if let Some(value) = rx.borrow_and_update().clone() {
                let value = value.expect("the published value is not an error");
                if done(&value) {
                    return Arc::new(value);
                }
            }
            tokio::time::timeout(Duration::from_secs(5), rx.changed())
                .await
                .expect("the expected value within 5 s")
                .expect("sender alive");
        }
    }

    #[test]
    fn get_or_none_returns_the_row_and_swallows_a_missing_one() {
        let (_d, db) = test_db();
        let repo = EntryRepo::new(db);
        for kind in [Kind::Manga, Kind::Anime] {
            let id = repo.insert(&entry(kind, "/1", "One", true)).unwrap();
            assert_eq!(get_entry_or_none(&repo, id).map(|e| e.title), Some("One".into()));
            assert_eq!(get_entry_or_none(&repo, id + 999), None, "a missing row is logged and becomes None");
        }
    }

    #[test]
    fn by_url_favourites_and_duplicates_are_scoped_to_the_kind() {
        let (_d, db) = test_db();
        let repo = EntryRepo::new(db);
        for kind in [Kind::Manga, Kind::Anime] {
            let a = repo.insert(&entry(kind, "/a", "Same Title", true)).unwrap();
            let b = repo.insert(&entry(kind, "/b", "SAME TITLE", true)).unwrap();
            repo.insert(&entry(kind, "/c", "Other", false)).unwrap();
            assert_eq!(get_entry_by_url_and_source(&repo, kind, "/c", 1).unwrap().map(|e| e.title), Some("Other".into()));
            assert_eq!(get_entry_by_url_and_source(&repo, kind, "/none", 1).unwrap(), None);
            assert_eq!(get_favorites(&repo, kind).unwrap().len(), 2, "{kind} sees only its own favourites");
            assert_eq!(get_favorites_by_source(&repo, kind, 1).unwrap().len(), 2);
            assert!(get_favorites_by_source(&repo, kind, 7).unwrap().is_empty());
            let first = repo.by_id(a).unwrap();
            let dupes = get_duplicate_library(&repo, kind, &first).unwrap();
            assert_eq!(dupes.iter().map(|e| e.id).collect::<Vec<_>>(), vec![b], "same title, other id, any case, same kind");
            assert_eq!(get_library(&repo, kind).unwrap().len(), 2);
        }
    }

    #[test]
    fn seasons_facade_returns_children_and_an_empty_list_for_no_parent() {
        let (_d, db) = test_db();
        let repo = EntryRepo::new(db);
        let parent = repo.insert(&Entry { fetch_type: Some(FetchType::Seasons), ..entry(Kind::Anime, "/p", "Parent", true) }).unwrap();
        for n in ["/s1", "/s2"] {
            repo.insert(&Entry { parent_id: Some(parent), ..entry(Kind::Anime, n, n, false) }).unwrap();
        }
        assert_eq!(get_seasons_by_parent_id(&repo, parent).len(), 2);
        assert!(get_seasons_by_parent_id(&repo, parent + 999).is_empty());
        assert_eq!(await_seasons(&repo, parent).unwrap().len(), 2);
    }

    #[test]
    fn seasons_facade_is_empty_when_the_read_fails() {
        let (_d, db) = test_db();
        db.write(|tx| Ok(tx.execute_batch("ALTER TABLE entries RENAME TO entries_gone;")?)).expect("rename");
        assert!(get_seasons_by_parent_id(&EntryRepo::new(db), 1).is_empty(), "logged, then an empty list");
    }

    #[test]
    fn awaits_pass_results_and_errors_through() {
        let (_d, db) = test_db();
        let entries = EntryRepo::new(db.clone());
        let items = ItemRepo::new(db.clone());
        for kind in [Kind::Manga, Kind::Anime] {
            let id = entries.insert(&entry(kind, "/m", "T", false)).unwrap();
            assert_eq!(await_entry(&entries, id).unwrap().url, "/m");
            assert!(matches!(await_entry(&entries, id + 99), Err(DataError::NotFound)));
            items.add_all(&[item(kind, id, "c1")]).unwrap();
            assert_eq!(await_items(&items, id, false).unwrap().len(), 1);
            assert!(await_items(&items, id + 99, false).unwrap().is_empty());
        }
        db.write(|tx| Ok(tx.execute_batch("ALTER TABLE items RENAME TO items_gone;")?)).expect("rename");
        assert!(await_items(&items, 1, false).is_err(), "the error reaches the caller");
    }

    #[tokio::test]
    async fn entry_with_items_emits_after_both_then_on_a_toggle() {
        for kind in [Kind::Manga, Kind::Anime] {
            let (_d, db) = test_db();
            let entries = EntryRepo::new(db.clone());
            let items = ItemRepo::new(db.clone());
            let id = entries.insert(&entry(kind, "/m", "T", false)).unwrap();
            let stored = items.add_all(&[item(kind, id, "c1")]).unwrap();
            let (mut rx, _handle) = subscribe_entry_with_items(&entries, &items, id, false);
            let pair = wait_for(&mut rx, |(e, i)| e.is_some() && !i.is_empty()).await;
            assert_eq!(pair.0.as_ref().as_ref().map(|e| e.url.clone()), Some("/m".into()));
            assert!(!pair.1[0].done);
            items.update(&ItemUpdate { done: Some(true), ..ItemUpdate::new(stored[0].id) }).unwrap();
            let toggled = wait_for(&mut rx, |(_, i)| i.first().is_some_and(|i| i.done)).await;
            assert!(toggled.1[0].done, "{kind}: a change of the items publishes a new pair");
        }
    }

    #[tokio::test]
    async fn dropping_the_joined_receiver_stops_its_task() {
        let (_d, db) = test_db();
        let entries = EntryRepo::new(db.clone());
        let items = ItemRepo::new(db);
        let id = entries.insert(&entry(Kind::Manga, "/m", "T", false)).unwrap();
        let (rx, handle) = subscribe_entry_with_items(&entries, &items, id, false);
        drop(rx);
        tokio::time::timeout(Duration::from_secs(5), handle).await.expect("task ends").expect("no panic");
    }

    #[tokio::test]
    async fn entry_with_items_and_seasons_emits_after_all_three_then_on_an_item_insert() {
        let (_d, db) = test_db();
        let entries = EntryRepo::new(db.clone());
        let items = ItemRepo::new(db);
        let id = entries.insert(&entry(Kind::Anime, "/a", "A", false)).unwrap();
        let (mut rx, _handle) = subscribe_entry_with_items_and_seasons(&entries, &items, id);
        let first = wait_for(&mut rx, |(a, e, s)| a.is_some() && e.is_empty() && s.is_empty()).await;
        assert_eq!(first.0.as_ref().as_ref().map(|a| a.url.clone()), Some("/a".into()));
        items.add_all(&[item(Kind::Anime, id, "e1")]).unwrap();
        let next = wait_for(&mut rx, |(_, e, _)| e.len() == 1).await;
        assert_eq!(next.1.len(), 1, "an item insert publishes a new triple");
        entries.update(&EntryUpdate { title: Some("A2".into()), ..EntryUpdate::new(id) }).unwrap();
        let renamed = wait_for(&mut rx, |(a, _, _)| a.as_ref().as_ref().is_some_and(|a| a.title == "A2")).await;
        assert_eq!(renamed.0.as_ref().as_ref().map(|a| a.title.clone()), Some("A2".into()));
    }
}
