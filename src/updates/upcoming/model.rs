//! The upcoming entry model and its grouping (T-42b-025, T-42b-026, T-42b-035). Pure: `build` turns the query rows
//! into the day groups, the per-day counts for the calendar dots, and the header index of the day jump.

use std::collections::BTreeMap;

use chrono::{NaiveDate, TimeZone};

use kuro_domain::dates::local_date;

/// One favourite entry with an upcoming update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: i64,
    pub title: String,
    pub status: i64,
    pub source_id: i64,
    pub next_update_ms: i64,
    pub cover_url: Option<String>,
    pub cover_last_modified: i64,
}

/// A line of the list: a day header with its count, or the entry at this index of the sorted entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Header { date: NaiveDate, count: u32 },
    Item(usize),
}

/// The grouped list with its calendar data. `events` is the per-day count for the dots; `header_index` maps a day to
/// its header's position in `rows` for the day jump.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index {
    pub rows: Vec<Row>,
    pub events: BTreeMap<NaiveDate, u32>,
    pub header_index: BTreeMap<NaiveDate, usize>,
}

/// The entries in display order (ascending `next_update`, ties by id: E-C6) and their grouping.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Upcoming {
    pub entries: Vec<Entry>,
    pub index: Index,
}

/// Groups the entries by their local date in `zone`. The rows are `Header(date, n), Item(..)...` per day in ascending
/// order, so `Item` indexes into the sorted `entries`. An entry whose time cannot be placed on a date is dropped.
pub fn build<Tz: TimeZone>(mut entries: Vec<Entry>, zone: &Tz) -> Upcoming {
    entries.sort_by(|a, b| (a.next_update_ms, a.id).cmp(&(b.next_update_ms, b.id)));
    let dated: Vec<(usize, NaiveDate)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| local_date(e.next_update_ms, zone).map(|d| (i, d)))
        .collect();
    let mut index = Index::default();
    let mut start = 0;
    while start < dated.len() {
        let day = dated[start].1;
        let end = dated[start..].iter().position(|(_, d)| *d != day).map_or(dated.len(), |p| start + p);
        let count = (end - start) as u32;
        index.header_index.insert(day, index.rows.len());
        index.rows.push(Row::Header { date: day, count });
        index.events.insert(day, count);
        index.rows.extend(dated[start..end].iter().map(|(i, _)| Row::Item(*i)));
        start = end;
    }
    Upcoming { entries, index }
}

/// The list position a click on a calendar day scrolls to (E-C8, E-C9): the day's header, plus one on the single
/// column where the calendar row comes first. A day without a header gives `None` and nothing scrolls.
pub fn day_jump(index: &Index, day: NaiveDate, single_column: bool) -> Option<usize> {
    index.header_index.get(&day).map(|at| at + usize::from(single_column))
}

/// The page re-queries when it is shown on a later day than the one it was loaded for (E-C18).
pub fn needs_requery(loaded_for: NaiveDate, today: NaiveDate) -> bool {
    loaded_for != today
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, TimeZone, Utc};

    use super::*;

    fn at(day: u32, hour: u32) -> i64 {
        Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0).single().unwrap().timestamp_millis()
    }

    fn entry(id: i64, next_update_ms: i64) -> Entry {
        Entry { id, title: format!("e{id}"), status: 1, source_id: 7, next_update_ms, cover_url: None, cover_last_modified: 0 }
    }

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    /// A test vector, in UTC so the local dates are the ones written here.
    #[test]
    fn build_upcoming_matches_the_vector() {
        let entries = vec![entry(1, at(8, 1)), entry(2, at(8, 20)), entry(3, at(9, 5)), entry(4, at(15, 12))];
        let up = build(entries, &Utc);
        assert_eq!(
            up.index.rows,
            vec![
                Row::Header { date: d(8), count: 2 },
                Row::Item(0),
                Row::Item(1),
                Row::Header { date: d(9), count: 1 },
                Row::Item(2),
                Row::Header { date: d(15), count: 1 },
                Row::Item(3),
            ]
        );
        assert_eq!(up.index.events.get(&d(8)), Some(&2));
        assert_eq!(up.index.events.get(&d(9)), Some(&1));
        assert_eq!(up.index.events.get(&d(15)), Some(&1));
        assert_eq!(up.index.header_index.get(&d(8)), Some(&0));
        assert_eq!(up.index.header_index.get(&d(9)), Some(&3));
        assert_eq!(up.index.header_index.get(&d(15)), Some(&5));
    }

    #[test]
    fn build_upcoming_ties_sort_by_id() {
        let up = build(vec![entry(9, at(8, 3)), entry(2, at(8, 3))], &Utc);
        let ids: Vec<i64> = up.entries.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![2, 9]);
    }

    #[test]
    fn build_upcoming_empty_has_no_rows() {
        let up = build(Vec::new(), &Utc);
        assert!(up.index.rows.is_empty() && up.index.events.is_empty());
    }

    #[test]
    fn build_upcoming_groups_by_the_local_date_of_the_zone() {
        let zone = chrono::FixedOffset::east_opt(3 * 3600).unwrap();
        // 23:30 UTC on the 8th is already the 9th at UTC+3
        let up = build(vec![entry(1, at(8, 23)), entry(2, at(9, 1))], &zone);
        assert_eq!(up.index.events.get(&d(9)), Some(&2));
    }

    #[test]
    fn dots_are_capped_by_the_count_not_the_map() {
        let entries = (1..=5).map(|i| entry(i, at(8, i as u32))).collect();
        let up = build(entries, &Utc);
        assert_eq!(up.index.events.get(&d(8)), Some(&5), "the count keeps all five");
        assert_eq!(crate::updates::upcoming::calendar::dots(5).len(), 3, "three dots are drawn");
    }

    #[test]
    fn status_sets_match_the_spec() {
        assert_eq!(kuro_platform::kind::Kind::Manga.upcoming_statuses(), [1, 4], "ongoing and publishing finished");
        assert_eq!(kuro_platform::kind::Kind::Anime.upcoming_statuses(), [1, 4, 7], "and upcoming");
    }

    #[test]
    fn requery_only_after_the_day_changes() {
        assert!(!needs_requery(d(8), d(8)));
        assert!(needs_requery(d(8), d(9)));
    }

    #[test]
    fn day_jump_adds_one_on_the_single_column() {
        let up = build(vec![entry(1, at(8, 1)), entry(2, at(9, 1))], &Utc);
        assert_eq!(day_jump(&up.index, d(9), false), Some(2), "two-panel: the header itself");
        assert_eq!(day_jump(&up.index, d(9), true), Some(3), "single column: the calendar row is first");
        assert_eq!(day_jump(&up.index, d(10), true), None, "a day with no header does nothing");
    }
}
