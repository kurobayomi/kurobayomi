//! Planning for done / undone and bookmark writes of both pages (read and unread for a chapter, seen and unseen for an
//! episode). The store writes the rows in one update.
//! Upstream: `app/src/main/java/eu/kanade/domain/items/chapter/interactor/SetReadStatus.kt`,
//! `MangaScreenModel.kt:832-840` (bookmarkChapters).

use kuro_data::item::Item;
use kuro_domain::items::status::items_to_change;

/// The done plan: the ids that were written, and the changed ones whose downloads go after a mark done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadPlan {
    pub changed: Vec<i64>,
    pub delete_downloads: Vec<i64>,
}

/// Plans a mark done (`done = true`) or mark undone over `items`, by the domain rule (`items_to_change`): mark done
/// writes only rows that are not done yet, mark undone writes rows that are done or have progress and resets their
/// progress. With `remove_after_read` a mark done deletes the downloads of the rows it changed. Returns `None` when
/// nothing changes (the upstream `Result.NoChapters`).
pub fn plan_done(done: bool, items: &[Item], remove_after_read: bool) -> Option<ReadPlan> {
    let changed: Vec<i64> = items_to_change(done, items).iter().map(|i| i.id).collect();
    if changed.is_empty() {
        return None;
    }
    let delete_downloads = if done && remove_after_read { changed.clone() } else { Vec::new() };
    Some(ReadPlan { changed, delete_downloads })
}

#[cfg(test)]
mod tests {
    use kuro_platform::kind::Kind;

    use super::*;

    fn row(id: i64, done: bool, progress: i64) -> Item {
        Item { id, done, progress, ..Item::create(Kind::Manga) }
    }

    #[test]
    fn mark_read_writes_only_unread_rows() {
        let rows = [row(1, true, 0), row(2, false, 5), row(3, false, 0)];
        let plan = plan_done(true, &rows, false).expect("two rows change");
        assert_eq!(plan.changed, vec![2, 3]);
        assert!(plan.delete_downloads.is_empty());
    }

    #[test]
    fn mark_unread_takes_read_or_partly_read_rows() {
        let rows = [row(1, false, 0), row(2, false, 9), row(3, true, 0), row(4, false, 0)];
        let plan = plan_done(false, &rows, true).expect("rows 2 and 3 change");
        assert_eq!(plan.changed, vec![2, 3]);
        assert!(plan.delete_downloads.is_empty(), "removal happens only after mark read");
    }

    #[test]
    fn nothing_to_change_is_none() {
        assert!(plan_done(true, &[row(1, true, 0)], true).is_none());
        assert!(plan_done(false, &[row(1, false, 0)], true).is_none());
        assert!(plan_done(true, &[], true).is_none());
    }

    #[test]
    fn remove_after_read_deletes_only_changed_chapters() {
        let rows = [row(1, true, 0), row(2, false, 0)];
        let plan = plan_done(true, &rows, true).expect("row 2 changes");
        assert_eq!(plan.delete_downloads, vec![2]);
    }
}
