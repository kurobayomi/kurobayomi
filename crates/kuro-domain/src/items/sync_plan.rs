//! The pure planner of a chapter, episode or season list refresh (`SyncChaptersWithSource.await` steps 3 to 10 and
//! `SyncEpisodesWithSource.await`; T-10a-008, T-10a-009). No IO: the input is the source list and the stored list, the
//! output is what to delete, insert, update and rename, plus the URLs that must not be reported as new.
//!
//! The applier ([`crate::items::chapter_sync`], [`crate::items::episode_sync`]) sanitises the source list, prepares
//! each row (the source hook and the number recognition) and writes the plan in one transaction.

use std::collections::{HashMap, HashSet};

/// The fields the planner reads and writes on a chapter or episode row. `read` is `read` for a chapter and `seen` for an
/// episode, so one planner serves both.
pub trait SyncRow: Clone {
    fn id(&self) -> i64;
    fn url(&self) -> &str;
    fn number(&self) -> f64;
    fn read(&self) -> bool;
    fn bookmark(&self) -> bool;
    fn date_fetch(&self) -> i64;
    fn date_upload(&self) -> i64;
    fn set_source_order(&mut self, order: i64);
    fn set_date_upload(&mut self, date: i64);
    fn set_date_fetch(&mut self, date: i64);
    fn set_read(&mut self, read: bool);
    fn set_bookmark(&mut self, bookmark: bool);
    /// Whether the stored row must be rewritten for this source row (`ShouldUpdateDbChapter` and friends).
    fn needs_update(db: &Self, src: &Self) -> bool;
    /// The stored row after the refresh: the fields the source owns, with the rest of `db` kept (`toChange`).
    fn merged(db: &Self, src: &Self) -> Self;
}

/// What one refresh does to the stored list.
#[derive(Clone, Debug, PartialEq)]
pub struct SyncPlan<T> {
    /// New rows, in source order, with their fetch dates and carried state. Not yet inserted (no ids).
    pub added: Vec<T>,
    /// Stored rows to rewrite, in source order, already merged with the source.
    pub updated: Vec<T>,
    /// Ids of stored rows whose URL the source no longer lists.
    pub removed_ids: Vec<i64>,
    /// `(stored, prepared source)` for every row that is rewritten. The download folder is renamed for these, before the
    /// write.
    pub renames: Vec<(T, T)>,
    /// URLs whose row is marked read as a duplicate, or inherited state from a removed twin. Not reported as new.
    pub changed_or_duplicate_urls: HashSet<String>,
}

impl<T> SyncPlan<T> {
    /// True when nothing is added, updated or removed: the caller then only refreshes the fetch interval.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.updated.is_empty() && self.removed_ids.is_empty()
    }
}

/// The input of one plan. `source` is already distinct-free and sanitised; the planner de-duplicates by URL and assigns
/// `source_order` (EC-03). `prepare` runs the source hook and the number recognition on each row (step 6).
pub struct PlanInput<'a, T> {
    pub source: Vec<T>,
    pub db: &'a [T],
    pub now_ms: i64,
    /// When the entry joined the library (0 when it is not in it). A first sync, of an entry with no stored rows,
    /// stamps its rows with this instead of `now_ms`: they are what the entry already had, not updates.
    pub added_ms: i64,
    pub mark_duplicate_as_read: bool,
    pub prepare: &'a dyn Fn(T) -> T,
}

/// A number as a set key. `-0.0` and `0.0` are the same key; every number comes from the same widening, so equal numbers
/// have equal bits.
fn number_key(number: f64) -> u64 {
    if number == 0.0 { 0 } else { number.to_bits() }
}

/// Steps 3 to 10 of the sync as a pure function.
pub fn plan_sync<T: SyncRow>(input: PlanInput<'_, T>) -> SyncPlan<T> {
    let PlanInput { source, db, now_ms, added_ms, mark_duplicate_as_read, prepare } = input;
    let first_sync = db.is_empty();

    // Step 3: distinct by URL, first occurrence wins, then source_order is the position after de-duplication (EC-03).
    let mut seen_urls: HashSet<String> = HashSet::new();
    let distinct: Vec<T> = source.into_iter().filter(|row| seen_urls.insert(row.url().to_owned())).collect();
    let source_urls: HashSet<String> = distinct.iter().map(|row| row.url().to_owned()).collect();

    // Step 5: stored rows the source no longer lists.
    let removed: Vec<&T> = db.iter().filter(|row| !source_urls.contains(row.url())).collect();

    // Step 6: walk the source in order. A new row with no date takes `now` until a dated row is seen, and then the largest
    // date seen so far. Stored rows whose URL matches are compared and merged, not inserted.
    let mut max_seen_upload = 0_i64;
    let mut new_rows: Vec<T> = Vec::new();
    let mut updated: Vec<T> = Vec::new();
    let mut renames: Vec<(T, T)> = Vec::new();
    for (position, mut raw) in distinct.into_iter().enumerate() {
        raw.set_source_order(i64::try_from(position).unwrap_or(i64::MAX));
        let raw_date = raw.date_upload();
        let prepared = prepare(raw);
        match db.iter().find(|row| row.url() == prepared.url()) {
            None => {
                let mut row = prepared;
                if row.date_upload() == 0 {
                    let date = if max_seen_upload == 0 { now_ms } else { max_seen_upload };
                    row.set_date_upload(date);
                } else {
                    max_seen_upload = max_seen_upload.max(raw_date);
                }
                new_rows.push(row);
            }
            Some(stored) => {
                if T::needs_update(stored, &prepared) {
                    updated.push(T::merged(stored, &prepared));
                    renames.push((stored.clone(), prepared));
                }
            }
        }
    }

    if new_rows.is_empty() && updated.is_empty() && removed.is_empty() {
        return SyncPlan { added: Vec::new(), updated, removed_ids: Vec::new(), renames, changed_or_duplicate_urls: HashSet::new() };
    }

    // Step 8: the helper sets. Read numbers come from stored rows that are read and recognised (number >= 0). The removed
    // rows contribute their numbers, read and bookmark flags, and their fetch dates (the smallest fetch date wins).
    let read_numbers: HashSet<u64> = db.iter().filter(|r| r.read() && r.number() >= 0.0).map(|r| number_key(r.number())).collect();
    let mut deleted_numbers: HashSet<u64> = HashSet::new();
    let mut deleted_read: HashSet<u64> = HashSet::new();
    let mut deleted_bookmark: HashSet<u64> = HashSet::new();
    for row in &removed {
        deleted_numbers.insert(number_key(row.number()));
        if row.read() {
            deleted_read.insert(number_key(row.number()));
        }
        if row.bookmark() {
            deleted_bookmark.insert(number_key(row.number()));
        }
    }
    let mut by_fetch: Vec<&T> = removed.clone();
    by_fetch.sort_by_key(|row| std::cmp::Reverse(row.date_fetch()));
    let mut deleted_fetch: HashMap<u64, i64> = HashMap::new();
    for row in by_fetch {
        deleted_fetch.insert(number_key(row.number()), row.date_fetch());
    }

    // Step 10: fetch dates count down from now + count, so the first listed (newest) row gets the highest value. Then the
    // duplicate-read rule and the removed-twin rule, which overrides it.
    let mut changed_or_duplicate_urls: HashSet<String> = HashSet::new();
    let count = new_rows.len();
    let mut added = Vec::with_capacity(count);
    for (position, mut row) in new_rows.into_iter().enumerate() {
        // a first sync's rows are the back catalogue: stamped at or before the time the entry was added, newest highest,
        // so Updates (`date_fetch > date_added`) never lists them whether the entry was added before or after the fetch
        let stamp = if first_sync { (added_ms - position as i64).max(0) } else { now_ms + (count - position) as i64 };
        row.set_date_fetch(stamp);
        let key = number_key(row.number());
        if mark_duplicate_as_read && read_numbers.contains(&key) {
            changed_or_duplicate_urls.insert(row.url().to_owned());
            row.set_read(true);
        }
        let recognised = row.number() >= 0.0;
        if recognised && deleted_numbers.contains(&key) {
            row.set_read(deleted_read.contains(&key));
            row.set_bookmark(deleted_bookmark.contains(&key));
            if let Some(fetch) = deleted_fetch.get(&key) {
                row.set_date_fetch(*fetch);
            }
            changed_or_duplicate_urls.insert(row.url().to_owned());
        }
        added.push(row);
    }

    SyncPlan {
        added,
        updated,
        removed_ids: removed.iter().map(|row| row.id()).collect(),
        renames,
        changed_or_duplicate_urls,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stored or source row with the fields the planner reads.
    #[derive(Clone, Debug, PartialEq)]
    struct Row {
        id: i64,
        url: String,
        name: String,
        number: f64,
        read: bool,
        bookmark: bool,
        date_fetch: i64,
        date_upload: i64,
        source_order: i64,
        scanlator: Option<String>,
    }

    fn row(id: i64, url: &str, number: f64) -> Row {
        Row {
            id,
            url: url.into(),
            name: format!("Ch {number}"),
            number,
            read: false,
            bookmark: false,
            date_fetch: 0,
            date_upload: 0,
            source_order: 0,
            scanlator: None,
        }
    }

    impl SyncRow for Row {
        fn id(&self) -> i64 {
            self.id
        }
        fn url(&self) -> &str {
            &self.url
        }
        fn number(&self) -> f64 {
            self.number
        }
        fn read(&self) -> bool {
            self.read
        }
        fn bookmark(&self) -> bool {
            self.bookmark
        }
        fn date_fetch(&self) -> i64 {
            self.date_fetch
        }
        fn date_upload(&self) -> i64 {
            self.date_upload
        }
        fn set_source_order(&mut self, order: i64) {
            self.source_order = order;
        }
        fn set_date_upload(&mut self, date: i64) {
            self.date_upload = date;
        }
        fn set_date_fetch(&mut self, date: i64) {
            self.date_fetch = date;
        }
        fn set_read(&mut self, read: bool) {
            self.read = read;
        }
        fn set_bookmark(&mut self, bookmark: bool) {
            self.bookmark = bookmark;
        }
        fn needs_update(db: &Self, src: &Self) -> bool {
            db.scanlator != src.scanlator
                || db.name != src.name
                || (src.date_upload != 0 && db.date_upload != src.date_upload)
                || db.number != src.number
                || db.source_order != src.source_order
        }
        fn merged(db: &Self, src: &Self) -> Self {
            Row {
                name: src.name.clone(),
                number: src.number,
                scanlator: src.scanlator.clone().or_else(|| db.scanlator.clone()),
                source_order: src.source_order,
                date_upload: if src.date_upload != 0 { src.date_upload } else { db.date_upload },
                ..db.clone()
            }
        }
    }

    fn identity(r: Row) -> Row {
        r
    }

    #[test]
    fn a_first_sync_is_the_back_catalogue_and_a_later_sync_brings_updates() {
        let prepare: &dyn Fn(Row) -> Row = &identity;
        let src = |n: usize| (0..n).map(|i| row(0, &format!("/{i}"), i as f64)).collect::<Vec<_>>();
        // added at 1_000 and fetched later (the Add pressed before the list loaded): stamped at or before 1_000
        let first = plan_sync(PlanInput { source: src(3), db: &[], now_ms: 5_000, added_ms: 1_000, mark_duplicate_as_read: false, prepare });
        let stamps: Vec<i64> = first.added.iter().map(|r| r.date_fetch).collect();
        assert_eq!(stamps, [1_000, 999, 998], "never after the time added, newest highest");
        // not in the library yet: 0, still never an update once it is added
        let browsing = plan_sync(PlanInput { source: src(2), db: &[], now_ms: 5_000, added_ms: 0, mark_duplicate_as_read: false, prepare });
        assert!(browsing.added.iter().all(|r| r.date_fetch == 0));
        // a later sync with one new item stamps it now: a real update
        let stored = first.added.clone();
        let later = plan_sync(PlanInput { source: src(4), db: &stored, now_ms: 9_000, added_ms: 1_000, mark_duplicate_as_read: false, prepare });
        assert_eq!(later.added.len(), 1);
        assert!(later.added[0].date_fetch > 1_000, "a new chapter after the first sync is an update");
    }

    fn run(source: Vec<Row>, db: &[Row], now_ms: i64, mark: bool) -> SyncPlan<Row> {
        let prepare: &dyn Fn(Row) -> Row = &identity;
        plan_sync(PlanInput { source, db, now_ms, added_ms: 0, mark_duplicate_as_read: mark, prepare })
    }

    /// The chapter and episode sync worked example, with the stored sourceOrder values that make
    /// `/c2` and `/c1` update.
    #[test]
    fn worked_example() {
        let db = [
            Row { read: true, date_fetch: 100, ..row(1, "/c1", 1.0) },
            Row { date_fetch: 200, source_order: 1, ..row(2, "/c2", 2.0) },
            Row { read: true, bookmark: true, date_fetch: 500, ..row(3, "/c3", 3.0) },
        ];
        let source = vec![
            Row { date_upload: 0, ..row(0, "/c5", 5.0) },
            Row { date_upload: 900, ..row(0, "/c4", 4.0) },
            Row { date_upload: 0, ..row(0, "/c3b", 3.0) },
            Row { source_order: 3, ..row(0, "/c2", 2.0) },
            Row { source_order: 4, ..row(0, "/c1", 1.0) },
        ];
        let plan = run(source, &db, 1_000_000, true);
        let added: Vec<(&str, i64, i64)> = plan.added.iter().map(|r| (r.url.as_str(), r.date_upload, r.date_fetch)).collect();
        assert_eq!(added, [("/c5", 1_000_000, 1_000_003), ("/c4", 900, 1_000_002), ("/c3b", 900, 500)]);
        let c3b = &plan.added[2];
        assert!(c3b.read && c3b.bookmark, "c3b inherits read and bookmark from the removed twin");
        assert_eq!(plan.removed_ids, [3]);
        let updated: Vec<&str> = plan.updated.iter().map(|r| r.url.as_str()).collect();
        assert_eq!(updated, ["/c2", "/c1"]);
        assert!(plan.changed_or_duplicate_urls.contains("/c3b"));
        assert!(!plan.changed_or_duplicate_urls.contains("/c5"));
        assert_eq!(plan.renames.len(), 2, "one rename candidate per rewritten row");
    }

    #[test]
    fn duplicate_urls_keep_the_first_occurrence_and_its_position() {
        let source = vec![
            Row { name: "first".into(), ..row(0, "/x", 1.0) },
            Row { name: "second".into(), ..row(0, "/x", 1.0) },
            Row { ..row(0, "/y", 2.0) },
        ];
        let plan = run(source, &[], 1000, false);
        assert_eq!(plan.added.len(), 2);
        assert_eq!(plan.added[0].name, "first");
        assert_eq!(plan.added[1].source_order, 1, "the second distinct URL is at position 1");
    }

    #[test]
    fn a_removed_twin_with_the_smallest_fetch_date_wins_for_the_same_number() {
        let db = [
            Row { date_fetch: 500, ..row(1, "/a", 3.0) },
            Row { date_fetch: 400, ..row(2, "/b", 3.0) },
        ];
        let source = vec![row(0, "/new", 3.0)];
        let plan = run(source, &db, 1000, false);
        assert_eq!(plan.added[0].date_fetch, 400, "the map keeps the lowest dateFetch");
        assert_eq!(plan.removed_ids.len(), 2);
    }

    #[test]
    fn a_read_number_is_marked_read_only_when_the_duplicate_rule_is_on() {
        let db = [Row { read: true, ..row(1, "/old", 4.0) }];
        let source = vec![row(0, "/old", 4.0), row(0, "/alt", 4.0)];
        let on = run(source.clone(), &db, 1000, true);
        assert!(on.added.iter().any(|r| r.url == "/alt" && r.read), "duplicate marked read");
        let off = run(source, &db, 1000, false);
        assert!(off.added.iter().all(|r| !r.read), "no marking when the rule is off");
    }

    #[test]
    fn an_unchanged_list_plans_nothing() {
        let db = [Row { date_upload: 5, source_order: 0, ..row(1, "/c1", 1.0) }];
        let source = vec![Row { date_upload: 5, ..row(0, "/c1", 1.0) }];
        let plan = run(source, &db, 1000, false);
        assert!(plan.is_empty());
        assert!(plan.renames.is_empty());
    }

    /// A chapter number widened from f32 is equal to itself on the next sync (no requeue).
    #[test]
    fn a_widened_f32_number_is_not_updated_twice() {
        let number = crate::items::model::widen_source_number(12.1_f32);
        let db = [Row { source_order: 0, ..row(1, "/c", number) }];
        let source = vec![Row { ..row(0, "/c", number) }];
        assert!(run(source, &db, 1000, false).is_empty());
    }

    #[test]
    fn a_stored_row_with_a_new_scanlator_is_updated_but_keeps_the_stored_one_when_source_is_null() {
        let db = [Row { scanlator: Some("Team".into()), ..row(1, "/c", 1.0) }];
        let source = vec![Row { name: "Ch 1".into(), ..row(0, "/c", 1.0) }];
        let plan = run(source, &db, 1000, false);
        assert_eq!(plan.updated.len(), 1, "a null scanlator differs from the stored one every run");
        assert_eq!(plan.updated[0].scanlator.as_deref(), Some("Team"), "EC-08: the stored scanlator stays");
    }

    /// EC-06: a new row without a date takes `now` until a dated row is seen, then the largest date seen so far.
    #[test]
    fn a_new_row_without_a_date_takes_now_then_the_largest_date_seen() {
        let dates = |raw: &[i64]| -> Vec<i64> {
            let source = raw.iter().enumerate().map(|(i, d)| Row { date_upload: *d, ..row(0, &format!("/n{i}"), -1.0) }).collect();
            run(source, &[], 500, false).added.iter().map(|r| r.date_upload).collect()
        };
        assert_eq!(dates(&[0, 0, 0]), [500, 500, 500]);
        assert_eq!(dates(&[0, 1000, 0]), [500, 1000, 1000]);
        assert_eq!(dates(&[2000, 0, 1000]), [2000, 2000, 1000]);
    }

    /// EC-04: new rows of a later sync (the entry already has rows) get fetch dates counting down from `now + count`,
    /// so the first listed is the highest. A first sync is the back catalogue (see the first-sync test).
    #[test]
    fn new_rows_get_fetch_dates_counting_down_from_now_plus_count() {
        let stored = vec![row(1, "/old", -1.0)];
        let source = vec![row(0, "/a", -1.0), row(0, "/b", -1.0), row(0, "/c", -1.0), row(0, "/old", -1.0)];
        let fetch: Vec<i64> = run(source, &stored, 1000, false).added.iter().map(|r| r.date_fetch).collect();
        assert_eq!(fetch, [1003, 1002, 1001]);
    }

    /// EC-05: the removed-twin state overrides the duplicate-read marking. A read stored row marks its duplicate read, but
    /// a removed unread twin of the same number sets the new row back to unread.
    #[test]
    fn a_removed_twin_overrides_the_duplicate_read_marking() {
        let db = [Row { read: true, ..row(1, "/kept", 4.0) }, Row { read: false, date_fetch: 300, ..row(2, "/gone", 4.0) }];
        let source = vec![row(0, "/kept", 4.0), row(0, "/new", 4.0)];
        let plan = run(source, &db, 1000, true);
        let new = plan.added.iter().find(|r| r.url == "/new").expect("new row");
        assert!(!new.read, "the removed twin was unread, so the duplicate-read mark is overridden");
        assert_eq!(new.date_fetch, 300);
    }

    /// An unrecognised number (-1) never inherits from a removed twin or is marked as a duplicate.
    #[test]
    fn an_unrecognised_number_inherits_nothing_from_a_removed_row() {
        let db = [Row { read: true, bookmark: true, date_fetch: 300, ..row(1, "/gone", -1.0) }];
        let plan = run(vec![row(0, "/new", -1.0)], &db, 1000, true);
        let new = &plan.added[0];
        assert!(!new.read && !new.bookmark);
        assert_eq!(new.date_fetch, 1001);
    }
}
