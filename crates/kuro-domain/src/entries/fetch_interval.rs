//! The fetch interval ("smart update") engine, shared by manga and anime (`MangaFetchInterval.kt`,
//! `AnimeFetchInterval.kt`). Algorithm and vectors: `p1-topic-fetch-interval.md`.
//!
//! Every calendar computation takes a `chrono::TimeZone`, so tests run with UTC and a fixed Europe/Berlin
//! rule. The clock is passed in as epoch milliseconds (`clock_now_ms`), where Kotlin read `Instant.now()`.
//! Quirk 4 is kept: the result converts back to epoch with the offset of `now`, not of the target date.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Offset, TimeZone};
use kuro_data::DataResult;
use kuro_data::entry::{Entry, EntryRepo, EntryUpdate};
use kuro_data::item::{Item, ItemRepo};

use crate::items::facades::get_items_by_entry;

/// Largest interval in days (`MAX_INTERVAL`).
pub const MAX_INTERVAL: i32 = 28;

/// `increaseWhenOver`: a step doubles after more than this many cycles.
const INCREASE_WHEN_OVER: i64 = 10;

/// Interval used when the sample has fewer than three distinct days.
const DEFAULT_INTERVAL: i32 = 7;

const MS_PER_DAY: i64 = 86_400_000;

/// The two timestamps of one chapter or episode that the estimate reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemDates {
    pub date_upload: i64,
    pub date_fetch: i64,
}

impl From<&Item> for ItemDates {
    fn from(i: &Item) -> Self {
        Self {
            date_upload: i.date_upload,
            date_fetch: i.date_fetch,
        }
    }
}

/// The stored interval fields of an entry, as the update reads them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stored {
    pub fetch_interval: i32,
    pub next_update: i64,
    pub last_update: i64,
}

/// The two fields `to_update` writes back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntervalUpdate {
    pub next_update: i64,
    pub fetch_interval: i32,
}

/// Local midnight of `date` in `tz`. The earliest instant is used when midnight is ambiguous. A midnight
/// that falls in a DST gap has no local instant, so it is read as UTC there (no zone here has one).
fn start_of_day<Tz: TimeZone>(tz: &Tz, date: NaiveDate) -> DateTime<Tz> {
    let midnight: NaiveDateTime = date.and_hms_opt(0, 0, 0).unwrap_or_default();
    tz.from_local_datetime(&midnight)
        .earliest()
        .unwrap_or_else(|| tz.from_utc_datetime(&midnight))
}

/// The local calendar date of an epoch-millisecond instant in `tz`.
fn local_date<Tz: TimeZone>(ms: i64, tz: &Tz) -> NaiveDate {
    DateTime::from_timestamp_millis(ms)
        .map(|utc| utc.with_timezone(tz).date_naive())
        .unwrap_or(NaiveDate::MIN)
}

/// `getWindow(dateTime)`: `(yesterday 00:00, end of tomorrow)` in epoch ms, the upper bound being one
/// millisecond before tomorrow's midnight.
pub fn get_window<Tz: TimeZone>(now: &DateTime<Tz>) -> (i64, i64) {
    let tz = now.timezone();
    let today = now.date_naive();
    let lower = start_of_day(&tz, today.pred_opt().unwrap_or(today));
    let upper = start_of_day(&tz, today.succ_opt().unwrap_or(today));
    (lower.timestamp_millis(), upper.timestamp_millis() - 1)
}

/// `calculateInterval(items, zone)`: the average gap in days between the most recent distinct release days
/// (by upload date, or by fetch date when upload dates give fewer than three days). Lazy over the sorted
/// items: the distinct days are taken as the iterator is consumed.
pub fn calculate_interval<Tz: TimeZone>(items: &[ItemDates], tz: &Tz) -> i32 {
    let window = if items.len() <= 8 { 3 } else { 10 };
    let distinct_days =
        |key: fn(&ItemDates) -> i64, keep: fn(&ItemDates) -> bool| -> Vec<NaiveDate> {
            let mut sorted: Vec<&ItemDates> = items.iter().filter(|i| keep(i)).collect();
            sorted.sort_by_key(|i| std::cmp::Reverse(key(i)));
            let mut days: Vec<NaiveDate> = sorted.iter().map(|i| local_date(key(i), tz)).collect();
            days.dedup();
            days.truncate(window);
            days
        };

    let uploads = distinct_days(|i| i.date_upload, |i| i.date_upload > 0);
    let days = if uploads.len() >= 3 {
        uploads
    } else {
        distinct_days(|i| i.date_fetch, |_| true)
    };
    let interval = if days.len() >= 3 {
        // `days` is newest first, so the oldest day is last and its index is the number of gaps.
        let oldest = days[days.len() - 1];
        let newest = days[0];
        let periods = (days.len() - 1) as i64;
        ((newest - oldest).num_days() / periods) as i32
    } else {
        DEFAULT_INTERVAL
    };
    interval.clamp(1, MAX_INTERVAL)
}

/// `increaseInterval(delta, timeSinceLatest, increaseWhenOver)`: doubles `delta` while more than ten cycles
/// have passed since the last change, up to `MAX_INTERVAL`.
pub fn increase_interval(delta: i32, time_since_latest: i64) -> i32 {
    if delta >= MAX_INTERVAL {
        return MAX_INTERVAL;
    }
    // A non-positive step never grows, so it is returned unchanged rather than doubled forever.
    if delta < 1 {
        return delta;
    }
    let cycle = time_since_latest.div_euclid(i64::from(delta)) + 1;
    if cycle > INCREASE_WHEN_OVER {
        increase_interval(delta * 2, time_since_latest)
    } else {
        delta
    }
}

/// `calculateNextUpdate(entry, interval, dateTime, window)`: the next predicted check in epoch ms.
pub fn calculate_next_update<Tz: TimeZone>(
    stored: Stored,
    interval: i32,
    now: &DateTime<Tz>,
    window: (i64, i64),
    clock_now_ms: i64,
) -> i64 {
    if (window.0..=window.1 + 1).contains(&stored.next_update) {
        return stored.next_update;
    }
    let tz = now.timezone();
    let latest_ms = if stored.last_update > 0 {
        stored.last_update
    } else {
        clock_now_ms
    };
    let latest_midnight: NaiveDateTime = local_date(latest_ms, &tz)
        .and_hms_opt(0, 0, 0)
        .unwrap_or_default();
    // Whole days, truncated toward zero like `DAYS.between` (a last update in the future gives <= 0).
    let time_since_latest = (now.naive_local() - latest_midnight).num_days();

    let step = if interval < 0 {
        interval.abs()
    } else {
        increase_interval(interval, time_since_latest)
    };
    let cycle = time_since_latest.div_euclid(i64::from(step));
    let days = (cycle + 1) * i64::from(interval.abs());
    // Quirk 4: the offset of `now`, not of the target date.
    let target = latest_midnight + Duration::days(days);
    (target.and_utc().timestamp() - i64::from(now.offset().fix().local_minus_utc())) * 1000
}

/// `toMangaUpdate` / `toAnimeUpdate` for the interval fields. A negative stored interval is kept and
/// used as `abs(value)`. Any other value is recomputed from `items`. A window of `(0, 0)` means "compute from
/// now", which is the clock instant `clock_now_ms` in the zone of `now`.
pub fn to_update<Tz: TimeZone>(
    stored: Stored,
    items: &[ItemDates],
    now: &DateTime<Tz>,
    window: (i64, i64),
    clock_now_ms: i64,
) -> IntervalUpdate {
    let tz = now.timezone();
    let interval = if stored.fetch_interval < 0 {
        stored.fetch_interval
    } else {
        calculate_interval(items, &tz)
    };
    let current_window = if window == (0, 0) {
        let clock = DateTime::from_timestamp_millis(clock_now_ms)
            .map(|utc| utc.with_timezone(&tz))
            .unwrap_or_else(|| now.clone());
        get_window(&clock)
    } else {
        window
    };
    IntervalUpdate {
        next_update: calculate_next_update(stored, interval, now, current_window, clock_now_ms),
        fetch_interval: interval,
    }
}

/// The interval fields of an entry from its stored interval and a sample of its items. The sample is read only when the
/// stored interval is automatic (a negative one is a user override and needs no items).
pub fn interval_update<Tz: TimeZone>(
    stored: Stored,
    sample: impl FnOnce() -> Vec<ItemDates>,
    now: &DateTime<Tz>,
    window: (i64, i64),
    clock_now_ms: i64,
) -> IntervalUpdate {
    let items = if stored.fetch_interval < 0 { Vec::new() } else { sample() };
    to_update(stored, &items, now, window, clock_now_ms)
}

/// `toMangaUpdate` and `toAnimeUpdate`: the interval fields of an entry of either kind. Items are read only when the stored
/// interval is automatic, and the scanlator-filtered list is the sample (filter on; an anime has no exclusions).
pub fn to_entry_update<Tz: TimeZone>(
    items: &ItemRepo,
    e: &Entry,
    now: &DateTime<Tz>,
    window: (i64, i64),
    clock_now_ms: i64,
) -> EntryUpdate {
    let stored = Stored { fetch_interval: e.fetch_interval, next_update: e.next_update, last_update: e.last_update };
    let sample = || get_items_by_entry(items, e.id, true).iter().map(ItemDates::from).collect();
    let out = interval_update(stored, sample, now, window, clock_now_ms);
    EntryUpdate { next_update: Some(out.next_update), fetch_interval: Some(out.fetch_interval), ..EntryUpdate::new(e.id) }
}

/// `setFetchInterval(entry, wheel)`: `wheel` is stored as `-wheel` (0 means automatic), the interval fields are
/// recomputed and the row is written.
pub fn set_fetch_interval<Tz: TimeZone>(
    repo: &EntryRepo,
    items: &ItemRepo,
    e: &Entry,
    wheel: i32,
    now: &DateTime<Tz>,
    window: (i64, i64),
    clock_now_ms: i64,
) -> DataResult<()> {
    let entry = Entry { fetch_interval: stored_for_wheel(wheel), ..e.clone() };
    repo.update(&to_entry_update(items, &entry, now, window, clock_now_ms))
}

/// The user override of the interval, as [`set_fetch_interval`] stores it: the
/// wheel value 0..=28 is stored as `-wheel`. Zero stores 0, which is automatic mode.
pub fn stored_for_wheel(wheel: i32) -> i32 {
    -wheel
}

/// Whole days from `now_ms` to `expected_next_update`, truncated and clamped at 0 (`next_update_days`).
/// `None` has no days.
pub fn next_update_days(expected_next_update: Option<i64>, now_ms: i64) -> Option<i64> {
    expected_next_update.map(|expected| ((expected - now_ms) / MS_PER_DAY).max(0))
}

/// Which "next update" sentence applies (`manga_interval_expected_update` or `..._null`): the predicted
/// text needs a non-negative interval and a known date.
pub fn expected_update_uses_prediction(interval: i32, expected_next_update: Option<i64>) -> bool {
    interval >= 0 && expected_next_update.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, FixedOffset, LocalResult, Utc, Weekday};

    /// 2020-01-01T00:00Z, the `testTime` of the upstream tests.
    const T: i64 = 1_577_836_800_000;
    const DAY: i64 = 86_400_000;
    const HOUR: i64 = 3_600_000;

    /// Europe/Berlin under the EU rule (summer time from the last Sunday of March to the last Sunday of
    /// October, both at 01:00 UTC). Stands in for `chrono-tz`, which the crate map rejects. Exact away from
    /// the two transitions, which the tests do not touch.
    #[derive(Clone, Copy, Debug)]
    struct Berlin;

    fn last_sunday(year: i32, month: u32) -> NaiveDate {
        let mut d = NaiveDate::from_ymd_opt(year, month, 31).expect("valid month end");
        while d.weekday() != Weekday::Sun {
            d = d.pred_opt().expect("date in range");
        }
        d
    }

    fn offset_at_utc(utc: NaiveDateTime) -> FixedOffset {
        let y = utc.year();
        let start = last_sunday(y, 3).and_hms_opt(1, 0, 0).expect("time");
        let end = last_sunday(y, 10).and_hms_opt(1, 0, 0).expect("time");
        let summer = utc >= start && utc < end;
        FixedOffset::east_opt(if summer { 7200 } else { 3600 }).expect("offset")
    }

    impl TimeZone for Berlin {
        type Offset = FixedOffset;

        fn from_offset(_: &FixedOffset) -> Self {
            Berlin
        }

        fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<FixedOffset> {
            self.offset_from_local_datetime(&local.and_hms_opt(0, 0, 0).expect("time"))
        }

        fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<FixedOffset> {
            LocalResult::Single(offset_at_utc(*local - Duration::hours(1)))
        }

        fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
            offset_at_utc(utc.and_hms_opt(0, 0, 0).expect("time"))
        }

        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
            offset_at_utc(*utc)
        }
    }

    fn at(ms: i64) -> ItemDates {
        ItemDates {
            date_upload: T + ms,
            date_fetch: T + ms,
        }
    }

    fn stored(fetch_interval: i32, next_update: i64, last_update: i64) -> Stored {
        Stored {
            fetch_interval,
            next_update,
            last_update,
        }
    }

    fn utc_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 3, 15, 10, 30, 0)
            .single()
            .expect("instant")
    }

    /// The `nowMs` of the worked vectors: 2024-03-15T10:30Z.
    const NOW_MS: i64 = 1_710_498_600_000;

    #[test]
    fn window_vectors_utc_and_berlin() {
        assert_eq!(
            get_window(&utc_now()),
            (1_710_374_400_000, 1_710_547_199_999)
        );
        let berlin = Berlin
            .with_ymd_and_hms(2024, 3, 31, 12, 0, 0)
            .single()
            .expect("instant");
        assert_eq!(get_window(&berlin), (1_711_753_200_000, 1_711_922_399_999));
    }

    #[test]
    fn next_update_worked_vectors() {
        let now = utc_now();
        let window = get_window(&now);
        let rows: [(Stored, i32, i64); 6] = [
            (stored(0, 0, 1_710_028_800_000), 7, 1_710_633_600_000),
            (stored(0, 0, 1_710_028_800_000), 1, 1_710_547_200_000),
            (stored(0, 0, 0), 7, 1_711_065_600_000),
            (stored(0, 0, 1_701_388_800_000), 1, 1_701_993_600_000),
            (stored(-3, 0, 1_710_028_800_000), -3, 1_710_547_200_000),
            (stored(0, 1_710_374_400_005, 0), 7, 1_710_374_400_005),
        ];
        for (row, interval, expected) in rows {
            assert_eq!(
                calculate_next_update(row, interval, &now, window, NOW_MS),
                expected,
                "{row:?}"
            );
        }
    }

    #[test]
    fn next_update_dst_row_uses_the_offset_of_now() {
        let now = Berlin
            .with_ymd_and_hms(2024, 3, 29, 10, 0, 0)
            .single()
            .expect("instant");
        let window = get_window(&now);
        let row = stored(7, 0, 1_711_666_800_000);
        assert_eq!(
            calculate_next_update(row, 7, &now, window, NOW_MS),
            1_712_271_600_000
        );
    }

    #[test]
    fn last_update_in_the_future_gives_the_latest_midnight() {
        let now = utc_now();
        let window = get_window(&now);
        // Last update on 2024-03-18: the whole-day difference truncates to -2, the cycle is -1, and the result
        // is that day's midnight.
        let future_last = 1_710_720_000_000 + HOUR;
        assert_eq!(
            calculate_next_update(stored(7, 0, future_last), 7, &now, window, NOW_MS),
            1_710_720_000_000
        );
    }

    #[test]
    fn increase_interval_table() {
        assert_eq!(increase_interval(1, 105), 16);
        assert_eq!(increase_interval(7, 5), 7);
        assert_eq!(increase_interval(28, 1000), 28);
        assert_eq!(increase_interval(20, 400), 28);
    }

    #[test]
    fn to_update_negative_interval_is_kept_and_used_as_absolute() {
        let now = utc_now();
        let out = to_update(stored(-5, 0, 1_710_028_800_000), &[], &now, (0, 0), NOW_MS);
        assert_eq!(out.fetch_interval, -5);
        // Step 5, five days since the last change, cycle 1, so two periods of five days after March 10.
        assert_eq!(out.next_update, 1_710_892_800_000);
    }

    #[test]
    fn to_update_zero_window_is_computed_from_the_clock() {
        let out = to_update(stored(0, 0, 0), &[], &utc_now(), (0, 0), NOW_MS);
        assert_eq!(out.fetch_interval, 7);
        assert_eq!(
            out.next_update, 1_711_065_600_000,
            "latest is today, so 7 days from today"
        );
    }

    #[test]
    fn empty_and_single_item_lists_give_seven_days() {
        let utc = Utc;
        assert_eq!(calculate_interval(&[], &utc), 7);
        assert_eq!(calculate_interval(&[at(DAY)], &utc), 7);
    }

    #[test]
    fn upstream_1_fifty_items_one_day_then_no_upload_dates() {
        let fifty = vec![at(DAY); 50];
        assert_eq!(calculate_interval(&fifty, &Utc), 7);
        let no_upload: Vec<ItemDates> = fifty
            .iter()
            .map(|i| ItemDates {
                date_upload: 0,
                ..*i
            })
            .collect();
        assert_eq!(calculate_interval(&no_upload, &Utc), 7);
    }

    #[test]
    fn upstream_2_recent_items_decide_the_interval() {
        let old: Vec<ItemDates> = (1..=5).map(|i| at(i * 7 * DAY)).collect();
        let last = old.last().expect("five items").date_upload - T;
        let new: Vec<ItemDates> = (1..=10).map(|i| at(last + i * DAY)).collect();
        let list: Vec<ItemDates> = old.into_iter().chain(new).collect();
        assert_eq!(calculate_interval(&list, &Utc), 1);
    }

    #[test]
    fn upstream_3_few_recent_items_use_window_three() {
        let old: Vec<ItemDates> = (1..=3).map(|i| at(i * 7 * DAY)).collect();
        let last = old.last().expect("three items").date_upload - T;
        let new: Vec<ItemDates> = (1..=3)
            .map(|i| at(last + 365 * DAY + i * 7 * DAY))
            .collect();
        let list: Vec<ItemDates> = old.into_iter().chain(new).collect();
        assert_eq!(calculate_interval(&list, &Utc), 7);
    }

    #[test]
    fn upstream_4_many_items_in_one_day() {
        assert_eq!(calculate_interval(&[at(10 * HOUR); 10], &Utc), 7);
    }

    #[test]
    fn upstream_5_items_on_two_days() {
        let mut list = vec![at(DAY); 2];
        list.extend(vec![at(2 * DAY); 5]);
        assert_eq!(calculate_interval(&list, &Utc), 7);
    }

    #[test]
    fn upstream_6_daily_releases() {
        let list: Vec<ItemDates> = (1..=20).map(|i| at(i * DAY)).collect();
        assert_eq!(calculate_interval(&list, &Utc), 1);
    }

    #[test]
    fn upstream_7_delta_under_one_day() {
        let list: Vec<ItemDates> = (1..=20).map(|i| at(15 * i * HOUR)).collect();
        assert_eq!(calculate_interval(&list, &Utc), 1);
    }

    #[test]
    fn upstream_8_every_two_days() {
        let list: Vec<ItemDates> = (1..=20).map(|i| at(2 * i * DAY)).collect();
        assert_eq!(calculate_interval(&list, &Utc), 2);
    }

    #[test]
    fn upstream_9_decimal_interval_is_floored_and_fetch_dates_are_used() {
        let list: Vec<ItemDates> = (1..=5).map(|i| at(25 * i * HOUR)).collect();
        assert_eq!(calculate_interval(&list, &Utc), 1);
        let no_upload: Vec<ItemDates> = list
            .iter()
            .map(|i| ItemDates {
                date_upload: 0,
                ..*i
            })
            .collect();
        assert_eq!(calculate_interval(&no_upload, &Utc), 1);
    }

    #[test]
    fn upstream_10_just_below_two_days() {
        let list: Vec<ItemDates> = (1..=20).map(|i| at(43 * i * HOUR)).collect();
        assert_eq!(calculate_interval(&list, &Utc), 1);
    }

    #[test]
    fn interval_is_computed_in_the_given_zone() {
        // Items at 23:30 UTC on consecutive days are the next local day in Berlin, so the gap stays 1 day.
        let list: Vec<ItemDates> = (0..20)
            .map(|i| at(i * DAY + 23 * HOUR + HOUR / 2))
            .collect();
        assert_eq!(calculate_interval(&list, &Berlin), 1);
    }

    #[test]
    fn wheel_and_next_update_text_helpers() {
        assert_eq!(stored_for_wheel(5), -5);
        assert_eq!(
            stored_for_wheel(0),
            0,
            "Default stores 0, which is automatic"
        );
        assert_eq!(next_update_days(Some(T + 3 * DAY + HOUR), T), Some(3));
        assert_eq!(
            next_update_days(Some(T - 5 * DAY), T),
            Some(0),
            "clamped at zero"
        );
        assert_eq!(next_update_days(None, T), None);
        assert!(expected_update_uses_prediction(0, Some(T)));
        assert!(
            !expected_update_uses_prediction(-3, Some(T)),
            "user override uses the null text"
        );
        assert!(!expected_update_uses_prediction(7, None));
    }
}

#[cfg(test)]
mod entry_updates {
    use chrono::Utc;
    use kuro_platform::kind::Kind;

    use super::*;
    use crate::testdb::test_db;

    /// 2020-01-01T00:00Z, as in the upstream tests.
    const T0: i64 = 1_577_836_800_000;
    const DAY_MS: i64 = 86_400_000;

    /// "Now" is 2020-01-08T12:00Z, one day after the newest sample item.
    fn now() -> DateTime<Utc> {
        Utc.timestamp_millis_opt(T0 + 7 * DAY_MS + 12 * 3_600_000).single().expect("instant")
    }

    fn day(d: i64) -> i64 {
        T0 + d * DAY_MS
    }

    fn item(kind: Kind, entry_id: i64, url: &str, scanlator: &str, upload: i64) -> Item {
        Item {
            entry_id,
            url: url.into(),
            name: url.into(),
            scanlator: Some(scanlator.into()),
            number: 1.0,
            date_fetch: upload,
            date_upload: upload,
            version: 0,
            ..Item::create(kind)
        }
    }

    /// An entry whose kept scanlator has uploads on days 0, 2, 4 and 6; the excluded scanlator uploads on every day
    /// 0 to 6. Counting the excluded items would give interval 1 instead of 2.
    fn fixture(kind: Kind) -> (tempfile::TempDir, std::sync::Arc<kuro_db::handle::Db>, EntryRepo, ItemRepo, Entry) {
        let (dir, db) = test_db();
        let entries = EntryRepo::new(db.clone());
        let id = entries.insert(&Entry { url: "/m".into(), title: "M".into(), source: 1, ..Entry::create(kind) }).expect("entry");
        let mut rows: Vec<Item> = [0, 2, 4, 6].iter().map(|&d| item(kind, id, &format!("kept{d}"), "Kept", day(d))).collect();
        rows.extend((0..7).map(|d| item(kind, id, &format!("gone{d}"), "Excluded", day(d))));
        ItemRepo::new(db.clone()).add_all(&rows).expect("items");
        entries.set_excluded_scanlators(id, &["Excluded".to_owned()].into()).expect("excluded");
        let entry = entries.by_id(id).expect("row");
        (dir, db.clone(), entries, ItemRepo::new(db), entry)
    }

    fn window() -> (i64, i64) {
        get_window(&now())
    }

    #[test]
    fn the_update_samples_only_the_included_scanlators() {
        for kind in Kind::ALL {
            let (_d, _db, _entries, items, entry) = fixture(kind);
            let update = to_entry_update(&items, &entry, &now(), window(), day(7));
            assert_eq!(update.fetch_interval, Some(2), "{kind}");
            assert_eq!(update.id, entry.id);
        }
    }

    #[test]
    fn a_negative_stored_interval_is_kept_without_reading_items() {
        let (_d, _db, _entries, items, entry) = fixture(Kind::Manga);
        let entry = Entry { fetch_interval: -3, ..entry };
        let update = to_entry_update(&items, &entry, &now(), window(), day(7));
        assert_eq!(update.fetch_interval, Some(-3));
    }

    #[test]
    fn a_failed_item_read_samples_nothing_and_gives_the_seven_day_default() {
        let (_d, db, _entries, items, entry) = fixture(Kind::Manga);
        db.write(|tx| Ok(tx.execute_batch("ALTER TABLE items RENAME TO items_gone;")?)).expect("rename");
        let update = to_entry_update(&items, &entry, &now(), window(), day(7));
        assert_eq!(update.fetch_interval, Some(7), "an empty sample gives the default");
    }

    #[test]
    fn set_fetch_interval_wheel_five_stores_minus_five() {
        for kind in Kind::ALL {
            let (_d, _db, entries, items, entry) = fixture(kind);
            set_fetch_interval(&entries, &items, &entry, 5, &now(), window(), day(7)).expect("set");
            let stored = entries.by_id(entry.id).expect("row");
            assert_eq!(stored.fetch_interval, -5, "{kind}");
            assert!(stored.next_update > 0, "the next update is recomputed");
        }
    }

    #[test]
    fn set_fetch_interval_wheel_zero_restores_automatic_mode() {
        let (_d, _db, entries, items, entry) = fixture(Kind::Manga);
        set_fetch_interval(&entries, &items, &entry, 5, &now(), window(), day(7)).expect("set 5");
        let overridden = entries.by_id(entry.id).expect("row");
        set_fetch_interval(&entries, &items, &overridden, 0, &now(), window(), day(7)).expect("set 0");
        assert_eq!(entries.by_id(entry.id).expect("row").fetch_interval, 2, "automatic: recomputed from the sample");
    }

    #[test]
    fn stored_for_wheel_negates() {
        assert_eq!(stored_for_wheel(5), -5);
        assert_eq!(stored_for_wheel(0), 0);
    }
}
