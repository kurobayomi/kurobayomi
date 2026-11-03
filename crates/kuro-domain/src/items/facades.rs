//! Item facades of both kinds (`GetChapter.kt`, `GetChapterByUrlAndMangaId.kt`, `GetChaptersByMangaId.kt`,
//! `UpdateChapter.kt` and the episode twins). Error contracts: reads log and
//! return `None` or an empty list, the by-url variant without a log stays silent, and writes log and return `()`.

use kuro_data::item::{Item, ItemRepo, ItemUpdate};

/// `GetChapter.await(id)` and `GetEpisode.await(id)`: `None` for a missing row or on error (the error is logged).
pub fn get_item(repo: &ItemRepo, id: i64) -> Option<Item> {
    match repo.by_id(id) {
        Ok(item) => item,
        Err(e) => {
            tracing::error!(id, error = %e, "cannot read item");
            None
        }
    }
}

/// `GetChapterByUrlAndMangaId.await(url, mangaId)` and its episode twin: `None` on error, and the error is not logged
/// (Kotlin swallows it). With several rows of one url the lowest id answers (the repository warns).
pub fn get_item_by_url_and_entry_id(repo: &ItemRepo, url: &str, entry_id: i64) -> Option<Item> {
    repo.by_url_and_entry_id(url, entry_id).ok().flatten()
}

/// `GetChaptersByMangaId.await` and `GetEpisodesByAnimeId.await`: an empty list on error (the error is logged).
pub fn get_items_by_entry(repo: &ItemRepo, entry_id: i64, apply_scanlator_filter: bool) -> Vec<Item> {
    match repo.by_entry_id(entry_id, apply_scanlator_filter) {
        Ok(items) => items,
        Err(e) => {
            tracing::error!(entry_id, error = %e, "cannot read items");
            Vec::new()
        }
    }
}

/// `UpdateChapter.await(update)` and its episode twin: the error is logged and swallowed, so the caller cannot tell a
/// failed write.
pub fn update_item(repo: &ItemRepo, update: &ItemUpdate) {
    if let Err(e) = repo.update(update) {
        tracing::error!(id = update.id, error = %e, "cannot update item");
    }
}

/// `UpdateChapter.awaitAll(updates)` and its episode twin: one transaction. An error is logged and swallowed.
pub fn update_all_items(repo: &ItemRepo, updates: &[ItemUpdate]) {
    if let Err(e) = repo.update_all(updates) {
        tracing::error!(count = updates.len(), error = %e, "cannot update items");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kuro_data::entry::{Entry, EntryRepo};
    use kuro_db::handle::Db;
    use kuro_platform::kind::Kind;

    use super::*;
    use crate::testdb::test_db;

    fn item(kind: Kind, entry_id: i64, url: &str, scanlator: Option<&str>) -> Item {
        Item { entry_id, url: url.into(), name: url.into(), number: 1.0, scanlator: scanlator.map(str::to_owned), ..Item::create(kind) }
    }

    /// An entry of `kind` with two items, one of them from scanlator `Excluded` (a manga rule, but the table takes
    /// both). Returns the kept item's id.
    fn entry_with_items(kind: Kind) -> (tempfile::TempDir, Arc<Db>, i64, i64) {
        let (dir, db) = test_db();
        let entries = EntryRepo::new(db.clone());
        let entry_id = entries.insert(&Entry { url: "/m".into(), title: "M".into(), source: 1, ..Entry::create(kind) }).expect("entry");
        let added = ItemRepo::new(db.clone())
            .add_all(&[item(kind, entry_id, "kept", Some("Kept")), item(kind, entry_id, "gone", Some("Excluded"))])
            .expect("items");
        entries.set_excluded_scanlators(entry_id, &["Excluded".to_owned()].into()).expect("excluded");
        (dir, db, entry_id, added[0].id)
    }

    /// Makes every read and write of the items table fail (its name no longer exists).
    fn break_items(db: &Db) {
        db.write(|tx| Ok(tx.execute_batch("ALTER TABLE items RENAME TO items_gone;")?)).expect("rename");
    }

    #[test]
    fn reads_by_id_and_by_url() {
        for kind in Kind::ALL {
            let (_d, db, entry_id, kept) = entry_with_items(kind);
            let repo = ItemRepo::new(db);
            assert_eq!(get_item(&repo, kept).map(|c| c.url), Some("kept".into()));
            assert_eq!(get_item(&repo, 9_999), None);
            assert_eq!(get_item_by_url_and_entry_id(&repo, "kept", entry_id).map(|c| c.id), Some(kept));
            assert_eq!(get_item_by_url_and_entry_id(&repo, "kept", entry_id + 1), None);
        }
    }

    #[test]
    fn reads_fail_quietly_to_none_on_error() {
        for kind in Kind::ALL {
            let (_d, db, entry_id, kept) = entry_with_items(kind);
            break_items(&db);
            let repo = ItemRepo::new(db);
            assert_eq!(get_item(&repo, kept), None);
            assert_eq!(get_item_by_url_and_entry_id(&repo, "kept", entry_id), None);
            assert!(get_items_by_entry(&repo, entry_id, false).is_empty());
        }
    }

    #[test]
    fn items_by_entry_apply_the_scanlator_filter_and_empty_on_error() {
        let (_d, db, entry_id, _) = entry_with_items(Kind::Manga);
        let repo = ItemRepo::new(db.clone());
        assert_eq!(get_items_by_entry(&repo, entry_id, false).len(), 2);
        let filtered = get_items_by_entry(&repo, entry_id, true);
        assert_eq!(filtered.iter().map(|c| c.url.as_str()).collect::<Vec<_>>(), ["kept"]);
        break_items(&db);
        assert!(get_items_by_entry(&repo, entry_id, true).is_empty());
    }

    #[test]
    fn update_item_writes_and_swallows_errors() {
        for kind in Kind::ALL {
            let (_d, db, _, kept) = entry_with_items(kind);
            let repo = ItemRepo::new(db.clone());
            update_item(&repo, &ItemUpdate { done: Some(true), ..ItemUpdate::new(kept) });
            assert!(get_item(&repo, kept).expect("row").done);
            break_items(&db);
            update_item(&repo, &ItemUpdate { done: Some(false), ..ItemUpdate::new(kept) });
        }
    }

    #[test]
    fn update_all_items_writes_a_batch_and_swallows_errors() {
        for kind in Kind::ALL {
            let (_d, db, _, kept) = entry_with_items(kind);
            let repo = ItemRepo::new(db.clone());
            update_all_items(&repo, &[ItemUpdate { bookmark: Some(true), ..ItemUpdate::new(kept) }]);
            assert!(get_item(&repo, kept).expect("row").bookmark);
            break_items(&db);
            update_all_items(&repo, &[ItemUpdate { bookmark: Some(false), ..ItemUpdate::new(kept) }]);
        }
    }
}
