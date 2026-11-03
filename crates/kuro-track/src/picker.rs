//! Logic of the pickers in the per-entry sheet (`TrackInfoDialogSelector.kt`). The GUI draws
//! the dialogs; this module decides the rows, the start position, the bounds and the value that OK sends. The
//! value is then written through the remote setters in `remote.rs`.

use crate::entry_kind::KindExt;
use std::ops::RangeInclusive;

use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, Offset, TimeZone};

use kuro_platform::kind::Kind;
use crate::tracker::{AnimeTracker, MangaTracker};

/// Ceiling of the number wheel when the total is unknown (behaviour 14.3).
pub const UNKNOWN_TOTAL_MAX: i64 = 10_000;

/// The range of the progress picker: `0..=total` when the total is known, `0..=10000` otherwise (B§6).
pub fn progress_range(total: i64) -> RangeInclusive<i64> {
    let max = if total > 0 { total } else { UNKNOWN_TOTAL_MAX };
    0..=max
}

/// Title key of the number picker: "Chapters" for manga, "Episodes" for anime.
pub fn number_title_key(kind: Kind) -> &'static str {
    kind.progress_label_key()
}

// ---- status (14.2)

/// One row of the status list. `label_key` is the tracker's message key, resolved by the GUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRow {
    pub status: i64,
    pub label_key: Option<&'static str>,
}

/// The status list of an anime tracker, in the tracker's order.
pub fn anime_status_rows(tr: &dyn AnimeTracker) -> Vec<StatusRow> {
    tr.status_list()
        .into_iter()
        .map(|status| StatusRow {
            status,
            label_key: tr.status_label(status),
        })
        .collect()
}

/// The status list of a manga tracker, in the tracker's order.
pub fn manga_status_rows(tr: &dyn MangaTracker) -> Vec<StatusRow> {
    tr.status_list()
        .into_iter()
        .map(|status| StatusRow {
            status,
            label_key: tr.status_label(status),
        })
        .collect()
}

/// Status picker. The selection starts at the current status. OK returns it; Cancel drops the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusPicker {
    rows: Vec<StatusRow>,
    selection: i64,
}

impl StatusPicker {
    pub fn new(rows: Vec<StatusRow>, current: i64) -> Self {
        Self {
            rows,
            selection: current,
        }
    }

    pub fn rows(&self) -> &[StatusRow] {
        &self.rows
    }

    pub fn selection(&self) -> i64 {
        self.selection
    }

    /// A click or a Space press on a row.
    pub fn select(&mut self, status: i64) {
        if self.rows.iter().any(|r| r.status == status) {
            self.selection = status;
        }
    }

    /// Up (`-1`) and Down (`+1`) move the selection by one row, clamped to the list. From a status that is not
    /// in the list, Down picks the first row and Up picks the last.
    pub fn move_by(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as i32 - 1;
        let next = match self.rows.iter().position(|r| r.status == self.selection) {
            Some(i) => (i as i32 + delta).clamp(0, last),
            None if delta >= 0 => 0,
            None => last,
        };
        self.selection = self.rows[next as usize].status;
    }

    /// The status OK sends to the remote setter.
    pub fn confirm(&self) -> i64 {
        self.selection
    }
}

// ---- number (14.3)

/// A key of the progress picker (`U§4`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberKey {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

/// Progress picker for chapters or episodes. Range `0..=max`, where `max` is the total when known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberPicker {
    max: i64,
    /// The value the picker opened with, after truncation and clamping.
    start: i64,
    value: i64,
}

impl NumberPicker {
    /// Range from the total (`0` when unknown). The value starts at the current progress, truncated, and is
    /// clamped into the range.
    pub fn new(total: i64, current: f64) -> Self {
        let max = *progress_range(total).end();
        let start = (current as i64).clamp(0, max);
        Self {
            max,
            start,
            value: start,
        }
    }

    /// The integer to store on OK, or `None` when the number is still the one the picker opened with. A stored
    /// fraction (12.9) therefore stays as it is unless the user changes the number (D-7, E-17, T-27b-037).
    pub fn committed(&self) -> Option<i64> {
        (self.value != self.start).then_some(self.value)
    }

    /// Applies a key: Up and Down move one, Page Up and Page Down move ten, Home is `0`, End is the maximum.
    pub fn press(&mut self, key: NumberKey) {
        match key {
            NumberKey::Up => self.step(1),
            NumberKey::Down => self.step(-1),
            NumberKey::PageUp => self.step(10),
            NumberKey::PageDown => self.step(-10),
            NumberKey::Home => self.set(0),
            NumberKey::End => self.set(self.max),
        }
    }

    pub fn max(&self) -> i64 {
        self.max
    }

    pub fn value(&self) -> i64 {
        self.value
    }

    /// Up, Down (one step) and Page Up, Page Down (ten steps); the value stays in the range.
    pub fn step(&mut self, delta: i64) {
        self.set(self.value + delta);
    }

    /// Home (`0`) and End (`max`) go through [`NumberPicker::set`].
    pub fn set(&mut self, value: i64) {
        self.value = value.clamp(0, self.max);
    }
}

// ---- score (14.4)

/// Score drop-down. Empty score lists hide the cell, so there is no picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScorePicker {
    items: Vec<String>,
    selected: usize,
}

impl ScorePicker {
    /// `None` when the tracker has no score list. Starts at the index of `current`, including index 0 (defects
    /// D-9, E-20, T-14). A value not in the list starts at the first item (U§5).
    pub fn new(items: &[String], current: &str) -> Option<Self> {
        if items.is_empty() {
            return None;
        }
        let selected = items.iter().position(|s| s == current).unwrap_or(0);
        Some(Self {
            items: items.to_vec(),
            selected,
        })
    }

    pub fn items(&self) -> &[String] {
        &self.items
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn select(&mut self, index: usize) {
        if index < self.items.len() {
            self.selected = index;
        }
    }

    /// The score text OK sends to `set_*_score`.
    pub fn confirm(&self) -> &str {
        &self.items[self.selected]
    }
}

// ---- dates (14.5, 14.6)

/// The local day of epoch millis `ms` in `zone`.
pub fn local_day<Tz: TimeZone>(ms: i64, zone: &Tz) -> Option<NaiveDate> {
    DateTime::from_timestamp_millis(ms).map(|dt| dt.with_timezone(zone).date_naive())
}

/// Moves an instant to the UTC instant with the same wall clock in `zone`
/// (`convertEpochMillisZone(zone, UTC)`).
pub fn to_utc_wall<Tz: TimeZone>(ms: i64, zone: &Tz) -> i64 {
    DateTime::from_timestamp_millis(ms)
        .map(|dt| {
            dt.with_timezone(zone)
                .naive_local()
                .and_utc()
                .timestamp_millis()
        })
        .unwrap_or(ms)
}

/// Inverse of [`to_utc_wall`]: the instant in `zone` whose wall clock is the UTC wall clock of `utc_ms`
/// (`convertEpochMillisZone(UTC, zone)`). An ambiguous wall clock takes the earlier instant. A wall clock in a
/// DST gap moves forward by the gap, as `ZonedDateTime` does.
pub fn from_utc_wall<Tz: TimeZone>(utc_ms: i64, zone: &Tz) -> i64 {
    let Some(wall) = DateTime::from_timestamp_millis(utc_ms).map(|dt| dt.naive_utc()) else {
        return utc_ms;
    };
    match zone.from_local_datetime(&wall) {
        LocalResult::Single(dt) | LocalResult::Ambiguous(dt, _) => dt.timestamp_millis(),
        LocalResult::None => {
            let before = zone
                .offset_from_utc_datetime(&(wall - Duration::days(1)))
                .fix()
                .local_minus_utc() as i64;
            utc_ms - before * 1000
        }
    }
}

/// UTC midnight of `day`, in epoch millis. This is what the date picker hands back.
pub fn utc_midnight_millis(day: NaiveDate) -> i64 {
    day.and_hms_opt(0, 0, 0)
        .map(|dt| dt.and_utc().timestamp_millis())
        .unwrap_or(0)
}

/// The calendar day the picker treats as today: the local day of `now_ms` in `zone`. The picker reads days as UTC
/// calendar days after [`to_utc_wall`], so a device west of UTC never shifts a day (D-10, T-27b-036).
pub fn picker_today(now_ms: i64, zone: &impl TimeZone) -> NaiveDate {
    local_day(to_utc_wall(now_ms, zone), &chrono::Utc).unwrap_or(NaiveDate::MIN)
}

/// The day a stored date shows as in the picker: the UTC date of the converted value. `None` for unset (0).
pub fn picker_day(stored_ms: i64, zone: &impl TimeZone) -> Option<NaiveDate> {
    if stored_ms == 0 {
        return None;
    }
    local_day(to_utc_wall(stored_ms, zone), &chrono::Utc)
}

/// Whether Remove is offered for a date (`startDate > 0`, `finishDate > 0`).
pub fn remove_offered(stored_ms: i64) -> bool {
    stored_ms > 0
}

/// Start or finish date picker. `other` is the finish day for the start picker and the start day for the finish
/// picker, when that date is set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatePicker {
    is_start: bool,
    selected: NaiveDate,
    other: Option<NaiveDate>,
    today: NaiveDate,
}

impl DatePicker {
    pub fn new(
        is_start: bool,
        selected: NaiveDate,
        other: Option<NaiveDate>,
        today: NaiveDate,
    ) -> Self {
        Self {
            is_start,
            selected,
            other,
            today,
        }
    }

    pub fn selected(&self) -> NaiveDate {
        self.selected
    }

    /// Title key: "Start date" or "Finish date".
    pub fn title_key(&self) -> &'static str {
        if self.is_start {
            "track_started_reading_date"
        } else {
            "track_finished_reading_date"
        }
    }

    /// Future days are never selectable. Start may not be after the finish day; finish may not be before the start
    /// day (behaviour 14.6, rules 1 to 4).
    pub fn is_selectable(&self, day: NaiveDate) -> bool {
        if day > self.today {
            return false;
        }
        match (self.is_start, self.other) {
            (true, Some(finish)) => day <= finish,
            (false, Some(start)) => day >= start,
            _ => true,
        }
    }

    /// The same rules at year granularity, for the year drop-down.
    pub fn is_selectable_year(&self, year: i32) -> bool {
        if year > self.today.year() {
            return false;
        }
        match (self.is_start, self.other) {
            (true, Some(finish)) => year <= finish.year(),
            (false, Some(start)) => year >= start.year(),
            _ => true,
        }
    }

    /// Selects `day` when it is allowed. Returns whether the selection changed.
    pub fn select(&mut self, day: NaiveDate) -> bool {
        if !self.is_selectable(day) {
            return false;
        }
        self.selected = day;
        true
    }

    /// Epoch millis OK stores: local midnight of the chosen day in `zone`. `None` when the day is not selectable.
    pub fn confirm(&self, zone: &impl TimeZone) -> Option<i64> {
        self.is_selectable(self.selected)
            .then(|| from_utc_wall(utc_midnight_millis(self.selected), zone))
    }
}

/// Remove-date confirmation (behaviour 14.7). Confirm stores `0`, which clears the date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateRemoval {
    pub tracker: String,
    pub is_start: bool,
}

impl DateRemoval {
    pub fn title_key(&self) -> &'static str {
        "track_remove_date_conf_title"
    }

    /// The text, with the tracker name in place of `%s`.
    pub fn text(&self) -> String {
        if self.is_start {
            format!(
                "This will remove your previously selected start date from {}",
                self.tracker
            )
        } else {
            format!(
                "This will remove your previously selected finish date from {}",
                self.tracker
            )
        }
    }

    pub fn value(&self) -> i64 {
        0
    }
}

#[cfg(test)]
mod tests {
    use chrono::FixedOffset;

    use super::*;
    use crate::fakes::{FakeAnime, MemAnimeRepo, MemSecrets, STATUS_COMPLETED, new_log};
    use crate::model::{AnimeTrack, TrackerId};
    use crate::remote::{set_anime_score, set_last_episode_seen, update_remote_anime};
    use crate::repo::AnimeTrackRepo;

    #[test]
    fn tr_27b_037_an_unchanged_number_commits_nothing_so_a_fraction_is_kept() {
        let mut p = NumberPicker::new(24, 12.9);
        assert_eq!(p.value(), 12);
        assert_eq!(p.committed(), None, "OK without a change writes nothing");
        p.press(NumberKey::Up);
        assert_eq!(
            p.committed(),
            Some(13),
            "a changed number commits the integer"
        );
        p.press(NumberKey::Down);
        assert_eq!(p.committed(), None, "moving back to the start is no change");
    }

    #[test]
    fn tr_27b_036_day_is_the_same_in_five_zones_at_nine_in_the_morning() {
        // UTC, Berlin (+2 in October), Kolkata (+5:30), Auckland (+13 after the September change), New York (-4).
        let zones = [0, 2 * 3600, 19_800, 13 * 3600, -4 * 3600];
        let oct8 = day(2026, 10, 8);
        let oct9 = day(2026, 10, 9);
        for secs in zones {
            let zone = FixedOffset::east_opt(secs).unwrap();
            let now = zone
                .with_ymd_and_hms(2026, 10, 8, 9, 0, 0)
                .unwrap()
                .timestamp_millis();
            let today = picker_today(now, &zone);
            assert_eq!(today, oct8, "zone offset {secs}");
            let mut picker = DatePicker::new(false, oct8, None, today);
            assert!(!picker.select(oct9), "tomorrow is refused in zone {secs}");
            assert!(picker.select(oct8));
            let stored = picker.confirm(&zone).unwrap();
            assert_eq!(
                picker_day(stored, &zone),
                Some(oct8),
                "the stored day reads back as the picked day in zone {secs}"
            );
        }
    }

    #[test]
    fn tr_27b_036_the_picked_midnight_reads_back_on_the_same_day_in_new_york() {
        let ny = FixedOffset::west_opt(4 * 3600).unwrap();
        let picked = utc_midnight_millis(day(2026, 10, 7));
        let stored = from_utc_wall(picked, &ny);
        assert_eq!(picker_day(stored, &ny), Some(day(2026, 10, 7)));
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn rows() -> Vec<StatusRow> {
        vec![
            StatusRow {
                status: 1,
                label_key: Some("status"),
            },
            StatusRow {
                status: 0,
                label_key: None,
            },
            StatusRow {
                status: 2,
                label_key: Some("score"),
            },
        ]
    }

    fn track(last: f64, total: i64) -> AnimeTrack {
        AnimeTrack {
            id: Some(1),
            anime_id: 9,
            tracker_id: TrackerId::ANILIST,
            remote_id: 3,
            library_id: None,
            title: "Show".into(),
            last_episode_seen: last,
            total_episodes: total,
            status: 1,
            score: 0.0,
            remote_url: String::new(),
            start_date: 0,
            finish_date: 0,
            private: false,
        }
    }

    #[test]
    fn tr_27_051_status_rows_keep_order_and_start_at_the_current_status() {
        let p = StatusPicker::new(rows(), 2);
        assert_eq!(
            p.rows().iter().map(|r| r.status).collect::<Vec<_>>(),
            [1, 0, 2]
        );
        assert_eq!(p.selection(), 2);
        assert_eq!(number_title_key(Kind::Anime), "episodes");
        assert_eq!(number_title_key(Kind::Manga), "chapters");
    }

    #[test]
    fn tr_27_051_up_down_move_and_clamp_and_space_selects() {
        let mut p = StatusPicker::new(rows(), 1);
        p.move_by(1);
        assert_eq!(p.selection(), 0);
        p.move_by(5);
        assert_eq!(p.selection(), 2, "clamped at the last row");
        p.move_by(-9);
        assert_eq!(p.selection(), 1, "clamped at the first row");
        p.select(2);
        assert_eq!(p.confirm(), 2);
        p.select(99);
        assert_eq!(p.confirm(), 2, "a status not in the list is ignored");
    }

    #[test]
    fn tr_27_051_a_status_outside_the_list_moves_to_an_end() {
        let mut down = StatusPicker::new(rows(), 42);
        down.move_by(1);
        assert_eq!(down.selection(), 1);
        let mut up = StatusPicker::new(rows(), 42);
        up.move_by(-1);
        assert_eq!(up.selection(), 2);
    }

    #[tokio::test]
    async fn tr_27_051_ok_sends_the_chosen_status_through_the_setter() {
        let tracker = FakeAnime::new(TrackerId::ANILIST, "AniList", MemSecrets::new(), new_log());
        let repo = MemAnimeRepo::default();
        let mut picker = StatusPicker::new(rows(), 1);
        picker.select(STATUS_COMPLETED);
        let mut t = track(3.0, 12);
        crate::remote::set_anime_status(&mut t, &tracker, picker.confirm());
        let stored = update_remote_anime(&t, &tracker, &repo).await.unwrap();
        assert_eq!(stored.status, STATUS_COMPLETED);
        assert_eq!(
            stored.last_episode_seen, 12.0,
            "completion sets progress to the total"
        );
        assert_eq!(
            repo.list_for_entry(9).await.unwrap()[0].status,
            STATUS_COMPLETED
        );
    }

    #[test]
    fn tr_27_052_number_range_is_the_total_or_ten_thousand() {
        let known = NumberPicker::new(24, 12.0);
        assert_eq!((known.max(), known.value()), (24, 12));
        let unknown = NumberPicker::new(0, 12.9);
        assert_eq!(unknown.max(), UNKNOWN_TOTAL_MAX);
        assert_eq!(unknown.value(), 12, "starts at the truncated progress");
    }

    #[test]
    fn tr_27b_008_range_is_zero_to_total_or_ten_thousand_and_keys_move_the_value() {
        assert_eq!(progress_range(24), 0..=24);
        assert_eq!(progress_range(0), 0..=10_000);
        assert_eq!(progress_range(-1), 0..=10_000);
        let mut p = NumberPicker::new(24, 5.0);
        p.press(NumberKey::PageUp);
        assert_eq!(p.value(), 15);
        p.press(NumberKey::PageDown);
        p.press(NumberKey::PageDown);
        assert_eq!(p.value(), 0, "clamped at the bottom");
        p.press(NumberKey::End);
        assert_eq!(p.value(), 24);
        p.press(NumberKey::Up);
        assert_eq!(p.value(), 24, "clamped at the top");
        p.press(NumberKey::Home);
        assert_eq!(p.value(), 0);
    }

    #[test]
    fn tr_27_052_steps_and_limits_stay_in_range() {
        let mut p = NumberPicker::new(24, 20.0);
        p.step(10);
        assert_eq!(p.value(), 24);
        p.step(-10);
        assert_eq!(p.value(), 14);
        p.set(-5);
        assert_eq!(p.value(), 0, "Home goes to the lower limit");
        p.set(500);
        assert_eq!(p.value(), 24, "End goes to the total");
        let start_past_total = NumberPicker::new(12, 14.0);
        assert_eq!(start_past_total.value(), 12);
    }

    #[tokio::test]
    async fn tr_27_052_ok_calls_the_progress_setter_with_the_wheel_value() {
        let tracker = FakeAnime::new(TrackerId::ANILIST, "AniList", MemSecrets::new(), new_log());
        let repo = MemAnimeRepo::default();
        let mut picker = NumberPicker::new(24, 2.0);
        picker.set(7);
        let mut t = track(2.0, 24);
        set_last_episode_seen(&mut t, &tracker, picker.value() as i32, 1_000);
        let stored = update_remote_anime(&t, &tracker, &repo).await.unwrap();
        assert_eq!(stored.last_episode_seen, 7.0);
    }

    #[test]
    fn tr_27_053_d08_score_starts_at_index_zero_when_current_is_first() {
        let items: Vec<String> = ["0", "1"].iter().map(|s| s.to_string()).collect();
        let p = ScorePicker::new(&items, "0").unwrap();
        assert_eq!(p.selected(), 0, "index 0 is a real start position");
        assert_eq!(p.confirm(), "0");
        let missing = ScorePicker::new(&items, "missing").unwrap();
        assert_eq!(missing.selected(), 0, "a value not in the list starts at the first item");
        let three: Vec<String> = ["0", "1", "2"].iter().map(|s| s.to_string()).collect();
        let found = ScorePicker::new(&three, "2").unwrap();
        assert_eq!(found.selected(), 2, "the current score is found at its own index");
    }

    #[test]
    fn tr_27_203_empty_score_list_hides_the_score_picker() {
        assert!(ScorePicker::new(&[], "7").is_none());
    }

    #[tokio::test]
    async fn tr_27_053_ok_stores_the_score_index_for_the_chosen_string() {
        let tracker = FakeAnime::new(TrackerId::ANILIST, "AniList", MemSecrets::new(), new_log());
        let repo = MemAnimeRepo::default();
        let items: Vec<String> = crate::fakes::SCORES.iter().map(|s| s.to_string()).collect();
        let mut picker = ScorePicker::new(&items, "a").unwrap();
        picker.select(2);
        let mut t = track(0.0, 0);
        set_anime_score(&mut t, &tracker, picker.confirm()).unwrap();
        assert_eq!(t.score, 2.0);
        update_remote_anime(&t, &tracker, &repo).await.unwrap();
    }

    #[test]
    fn tr_27_213_future_days_and_order_rules_for_start_and_finish() {
        let today = day(2026, 10, 9);
        let start = DatePicker::new(true, day(2026, 1, 1), Some(day(2026, 3, 1)), today);
        assert!(
            start.is_selectable(day(2026, 3, 1)),
            "start may equal finish"
        );
        assert!(
            !start.is_selectable(day(2026, 3, 2)),
            "start after finish is refused"
        );
        assert!(!start.is_selectable(day(2026, 10, 10)), "future is refused");
        let end = DatePicker::new(false, day(2026, 3, 1), Some(day(2026, 2, 1)), today);
        assert!(
            !end.is_selectable(day(2026, 1, 31)),
            "finish before start is refused"
        );
        assert!(end.is_selectable(day(2026, 2, 1)));
        let unset = DatePicker::new(true, today, None, today);
        assert!(
            unset.is_selectable(day(1990, 1, 1)),
            "unset finish allows any past day"
        );
        assert!(unset.is_selectable_year(1990));
        assert!(!unset.is_selectable_year(2027), "future years are refused");
        assert!(start.is_selectable_year(2026), "same year is allowed");
        assert!(!start.is_selectable_year(2027));
        assert_eq!(start.title_key(), "track_started_reading_date");
        assert_eq!(end.title_key(), "track_finished_reading_date");
    }

    #[test]
    fn tr_27_054_selecting_a_refused_day_changes_nothing() {
        let today = day(2026, 10, 9);
        let mut p = DatePicker::new(false, day(2026, 5, 5), None, today);
        assert!(!p.select(day(2026, 10, 10)));
        assert_eq!(p.selected(), day(2026, 5, 5));
        assert!(p.select(day(2026, 6, 1)));
        assert_eq!(p.selected(), day(2026, 6, 1));
    }

    #[test]
    fn tr_27_054_a_picked_day_round_trips_through_the_stored_value() {
        let zones = [
            FixedOffset::east_opt(2 * 3600).unwrap(),
            FixedOffset::west_opt(5 * 3600 + 1800).unwrap(),
            FixedOffset::east_opt(0).unwrap(),
        ];
        let today = day(2030, 1, 1);
        for zone in zones {
            for picked in [day(2026, 3, 1), day(2026, 12, 31), day(1969, 7, 20)] {
                let p = DatePicker::new(true, picked, None, today);
                let stored = p.confirm(&zone).unwrap();
                assert_eq!(picker_day(stored, &zone), Some(picked), "zone {zone}");
            }
        }
        assert_eq!(picker_day(0, &zones[0]), None, "epoch 0 is unset");
    }

    #[test]
    fn tr_27_305_wall_clock_conversion_round_trips_for_fixed_zones() {
        let zones = [
            FixedOffset::east_opt(14 * 3600).unwrap(),
            FixedOffset::west_opt(12 * 3600).unwrap(),
            FixedOffset::east_opt(5 * 3600 + 1800).unwrap(),
        ];
        let instants = [
            -86_400_000_i64 * 400,
            -1,
            0,
            1,
            1_780_000_000_123,
            2_000_000_000_000,
        ];
        for zone in zones {
            for ms in instants {
                let there = to_utc_wall(ms, &zone);
                assert_eq!(from_utc_wall(there, &zone), ms, "zone {zone} instant {ms}");
            }
        }
    }

    #[test]
    fn tr_27_305_local_zone_round_trips_except_in_gaps_and_overlaps() {
        let local = chrono::Local;
        let mut checked = 0;
        for k in 0..400_i64 {
            let ms = 1_700_000_000_000 + k * 3_601_000;
            let there = to_utc_wall(ms, &local);
            let wall = DateTime::from_timestamp_millis(there).unwrap().naive_utc();
            if local.from_local_datetime(&wall).single().is_some() {
                assert_eq!(from_utc_wall(there, &local), ms);
                checked += 1;
            }
        }
        assert!(checked > 0);
    }

    #[test]
    fn tr_27_055_remove_text_names_the_tracker_and_clears_to_zero() {
        let start = DateRemoval {
            tracker: "AniList".into(),
            is_start: true,
        };
        assert_eq!(start.title_key(), "track_remove_date_conf_title");
        assert_eq!(
            start.text(),
            "This will remove your previously selected start date from AniList"
        );
        let finish = DateRemoval {
            tracker: "Kitsu".into(),
            is_start: false,
        };
        assert!(finish.text().contains("finish date from Kitsu"));
        assert_eq!(finish.value(), 0);
        assert!(!remove_offered(0));
        assert!(remove_offered(5));
    }

    #[tokio::test]
    async fn tr_27_055_remove_stores_zero_through_the_remote_path() {
        let tracker = FakeAnime::new(TrackerId::ANILIST, "AniList", MemSecrets::new(), new_log());
        let repo = MemAnimeRepo::default();
        let mut t = track(1.0, 12);
        t.start_date = 1_700_000_000_000;
        let removal = DateRemoval {
            tracker: "AniList".into(),
            is_start: true,
        };
        crate::remote::set_anime_start_date(&mut t, removal.value());
        let stored = update_remote_anime(&t, &tracker, &repo).await.unwrap();
        assert_eq!(stored.start_date, 0);
    }
}
