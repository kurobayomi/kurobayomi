//! Next chapter and next episode (`GetNextChapters`, `GetNextEpisodes`, behaviour 3.5). The ordering is the entry's
//! own sort, read in ascending reading order (`item_cmp(sorting, false)`). The filtering is pure, so the rules
//! can be tested on plain lists; the repository reads of the entry happen in [`next_items`].

use kuro_platform::kind::Kind;
use std::sync::Arc;

use kuro_data::entry::EntryRepo;
use kuro_data::item::ItemRepo;
use kuro_db::handle::Db;

use crate::entries::flags::EntryFlags;
use crate::items::facades::get_items_by_entry;
use crate::items::sorter::item_cmp;

/// What the history screen and the download-ahead need about a next item.
#[derive(Clone, Debug, PartialEq)]
pub struct NextItem {
    pub id: i64,
    pub entry_id: i64,
    pub number: f64,
    pub name: String,
    pub done: bool,
}

/// The filter of `GetNextChapters` over an already ordered list.
///
/// `only_pending` keeps unread items only, and then the `from` anchor just moves the start. Without it, the list
/// starts at the anchor. An anchor that is missing, or read, is passed over (the first element is dropped). With no
/// anchor at all nothing is dropped.
pub fn next_in_order<T: Clone>(
    ordered: &[T],
    id: impl Fn(&T) -> i64,
    done: impl Fn(&T) -> bool,
    from: Option<i64>,
    only_pending: bool,
) -> Vec<T> {
    let list: Vec<T> =
        if only_pending { ordered.iter().filter(|x| !done(x)).cloned().collect() } else { ordered.to_vec() };
    let (start, drop_first) = match from {
        None => (0, false),
        Some(f) => match list.iter().position(|x| id(x) == f) {
            Some(i) => (i, !only_pending && done(&list[i])),
            None => (0, !only_pending),
        },
    };
    let mut out: Vec<T> = list.get(start..).unwrap_or_default().to_vec();
    if drop_first && !out.is_empty() {
        out.remove(0);
    }
    out
}

/// The next items of one entry, from the repositories. An unknown entry (or one of the other kind) or a failed read gives
/// an empty list and a log line.
pub fn next_items(
    db: &Arc<Db>,
    kind: Kind,
    entry_id: i64,
    from: Option<i64>,
    only_pending: bool,
) -> Vec<NextItem> {
    let entry = match EntryRepo::new(db.clone()).by_id(entry_id) {
        Ok(e) if e.kind == kind => e,
        Ok(_) => {
            tracing::error!(entry_id, %kind, "the entry is of the other kind; no next item");
            return Vec::new();
        }
        Err(e) => {
            tracing::error!(error = %e, entry_id, %kind, "cannot read the entry for the next item");
            return Vec::new();
        }
    };
    let mut items = get_items_by_entry(&ItemRepo::new(db.clone()), entry_id, true);
    items.sort_by(item_cmp(entry.sorting(), false));
    next_in_order(&items, |i| i.id, |i| i.done, from, only_pending)
        .into_iter()
        .map(|i| NextItem { id: i.id, entry_id: i.entry_id, number: i.number, name: i.name, done: i.done })
        .collect()
}

/// The next items after the newest history row of any entry of this kind. No history gives an empty list.
pub fn next_after_last_history(db: &Arc<Db>, kind: Kind, only_pending: bool) -> Vec<NextItem> {
    let latest = match crate::history::repo::HistoryRepo::new(db.clone()).latest(kind) {
        Ok(Some(row)) => row,
        Ok(None) => return Vec::new(),
        Err(e) => {
            tracing::error!(error = %e, "cannot read latest history");
            return Vec::new();
        }
    };
    next_items(db, kind, latest.entry_id, Some(latest.item_id), only_pending)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (id, read) in ascending reading order: c1 read, c2 read, c3 unread, c4 unread, c5 read, c6 unread.
    fn fixture() -> Vec<(i64, bool)> {
        vec![(1, true), (2, true), (3, false), (4, false), (5, true), (6, false)]
    }

    fn run(from: Option<i64>, only_pending: bool) -> Vec<i64> {
        next_in_order(&fixture(), |x| x.0, |x| x.1, from, only_pending).into_iter().map(|x| x.0).collect()
    }

    #[test]
    fn the_eight_rows_of_the_topic_table() {
        assert_eq!(run(None, true), vec![3, 4, 6]);
        assert_eq!(run(None, false), vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(run(Some(4), true), vec![4, 6]);
        assert_eq!(run(Some(2), true), vec![3, 4, 6]);
        assert_eq!(run(Some(4), false), vec![4, 5, 6]);
        assert_eq!(run(Some(2), false), vec![3, 4, 5, 6]);
        assert_eq!(run(Some(99), false), vec![2, 3, 4, 5, 6], "unknown from-id skips the first item");
        assert_eq!(run(Some(2), false).first(), Some(&3), "history resume after the read c2");
    }

    #[test]
    fn an_empty_list_stays_empty() {
        let none: Vec<(i64, bool)> = vec![];
        assert!(next_in_order(&none, |x| x.0, |x| x.1, Some(1), false).is_empty());
    }
}
