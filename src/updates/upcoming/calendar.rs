//! Calendar math of the Upcoming page (T-42b-027, T-42b-028, T-42b-029, T-42b-038, T-42b-045). Pure: the widgets draw
//! the grid these functions describe. Months are `(year, month)` pairs, which order the way `YearMonth` does.

use chrono::{Datelike, Duration, Months, NaiveDate, Weekday};

/// The most dots a day shows (E-C7).
pub const MAX_DOTS: u32 = 3;
/// Past days are drawn at this alpha (E-C16).
pub const PAST_DAY_ALPHA: f64 = 0.38;
/// Diameter of one event dot, in pixels (56 / 12 dp).
pub const DOT_DIAMETER: f64 = 56.0 / 12.0;

/// The first day of the week of a POSIX locale such as `en_US.UTF-8` or `ar_SA`. Sunday for the listed regions,
/// Saturday for AF and IR, Monday otherwise (R§8.2).
pub fn week_start(locale: &str) -> Weekday {
    let base = locale.split('.').next().unwrap_or("");
    let region = base.split(['_', '-']).nth(1).unwrap_or("").to_ascii_uppercase();
    match region.as_str() {
        "AF" | "IR" => Weekday::Sat,
        "US" | "CA" | "JP" | "IL" | "BR" | "IN" | "KR" | "PH" | "MX" | "TW" | "ZA" | "SA" | "HK" => Weekday::Sun,
        _ => Weekday::Mon,
    }
}

/// The `upcoming.week_start` setting: `monday` and `sunday` override the locale, anything else is `auto`.
pub fn week_start_for(pref: &str, locale: &str) -> Weekday {
    match pref {
        "monday" => Weekday::Mon,
        "sunday" => Weekday::Sun,
        _ => week_start(locale),
    }
}

/// The seven weekdays from `first` on.
pub fn week_days(first: Weekday) -> [Weekday; 7] {
    let mut days = [first; 7];
    for i in 1..7 {
        days[i] = days[i - 1].succ();
    }
    days
}

/// Blank cells before the first of the month in a grid that starts on `week[0]`.
pub fn leading_blanks(week: &[Weekday; 7], year: i32, month: u32) -> usize {
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("a valid month");
    week.iter().position(|w| *w == first.weekday()).unwrap_or(0)
}

/// The cells of a month grid: leading blanks as `None`, then every day, padded to whole weeks.
pub fn month_grid(year: i32, month: u32, first: Weekday) -> Vec<Option<NaiveDate>> {
    let week = week_days(first);
    let mut cells: Vec<Option<NaiveDate>> = vec![None; leading_blanks(&week, year, month)];
    cells.extend((1..=days_in_month(year, month)).map(|d| NaiveDate::from_ymd_opt(year, month, d)));
    while cells.len() % 7 != 0 {
        cells.push(None);
    }
    cells
}

/// The number of days of a month (`YearMonth.lengthOfMonth`), leap years included.
pub fn days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = shift_month(year, month, 1);
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("a valid month");
    let next = NaiveDate::from_ymd_opt(next_year, next_month, 1).expect("a valid month");
    (next - first).num_days() as u32
}

/// `YearMonth.plusMonths(delta)` as a `(year, month)` pair. The year rolls over at the year boundary (E-C11).
pub fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let total = year * 12 + (month as i32 - 1) + delta;
    (total.div_euclid(12), (total.rem_euclid(12) + 1) as u32)
}

/// The month title: `MMMM yyyy` in English (E-C19).
pub fn month_title(year: i32, month: u32) -> String {
    NaiveDate::from_ymd_opt(year, month, 1).expect("a valid month").format("%B %Y").to_string()
}

/// A keyboard step on the calendar (Proposal, U§2.7): arrows move by day and week, PgUp and PgDn by month.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalKey {
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
}

/// The date a key moves the focus to. A month step keeps the day of the month, clamped to the shorter month.
pub fn key_move(focus: NaiveDate, key: CalKey) -> NaiveDate {
    match key {
        CalKey::Left => focus - Duration::days(1),
        CalKey::Right => focus + Duration::days(1),
        CalKey::Up => focus - Duration::days(7),
        CalKey::Down => focus + Duration::days(7),
        CalKey::PageUp => add_months(focus, -1),
        CalKey::PageDown => add_months(focus, 1),
    }
}

fn add_months(date: NaiveDate, delta: i32) -> NaiveDate {
    let months = Months::new(delta.unsigned_abs());
    let moved = if delta < 0 { date.checked_sub_months(months) } else { date.checked_add_months(months) };
    moved.unwrap_or(date)
}

/// The one-letter column header of a weekday (`TextStyle.NARROW`): M, T, W, T, F, S, S.
pub fn weekday_narrow(day: Weekday) -> String {
    day.to_string().chars().take(1).collect()
}

/// Whether a month change moves forward in time, which decides the title's slide: forward slides in from below,
/// backward from above (E-C10, B§8.2).
pub fn moving_forward(from: (i32, u32), to: (i32, u32)) -> bool {
    to > from
}

/// Whether the point `(x, y)` of a `width` by `height` cell lies in the circle that is its click area (U§2.2): the
/// circle through the middle of the cell with the shorter side as its diameter.
pub fn in_circle(x: f64, y: f64, width: f64, height: f64) -> bool {
    let radius = width.min(height) / 2.0;
    let (dx, dy) = (x - width / 2.0, y - height / 2.0);
    dx * dx + dy * dy <= radius * radius
}

/// A past day is dimmed; today and the future are not (E-C16).
pub fn is_past(day: NaiveDate, today: NaiveDate) -> bool {
    day < today
}

/// The alpha of a day number: dimmed when past, else full.
pub fn day_alpha(day: NaiveDate, today: NaiveDate) -> f64 {
    if is_past(day, today) { PAST_DAY_ALPHA } else { 1.0 }
}

/// The event dots of a day: `min(count, 3)`, each dot `i` at alpha `(i + 1) * 0.3` (B§8.4).
pub fn dots(count: u32) -> Vec<f64> {
    (0..count.min(MAX_DOTS)).map(|i| f64::from(i + 1) * 0.3).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    /// Vectors of p2-edge-cases-and-tests.md section 3.3.
    #[test]
    fn leading_blanks_match_the_vectors() {
        let mon = week_days(Weekday::Mon);
        let sun = week_days(Weekday::Sun);
        assert_eq!((leading_blanks(&mon, 2026, 10), leading_blanks(&sun, 2026, 10)), (3, 4));
        assert_eq!((leading_blanks(&mon, 2026, 2), leading_blanks(&sun, 2026, 2)), (6, 0));
        assert_eq!((leading_blanks(&mon, 2026, 3), leading_blanks(&sun, 2026, 3)), (6, 0));
        assert_eq!((leading_blanks(&mon, 2027, 1), leading_blanks(&sun, 2027, 1)), (4, 5));
    }

    #[test]
    fn week_days_sunday_first_starts_on_sunday() {
        let days = week_days(Weekday::Sun);
        assert_eq!(days[0], Weekday::Sun);
        assert_eq!(days[6], Weekday::Sat);
    }

    #[test]
    fn week_start_by_locale() {
        assert_eq!(week_start("en_US.UTF-8"), Weekday::Sun);
        assert_eq!(week_start("en_GB.UTF-8"), Weekday::Mon);
        assert_eq!(week_start("ja_JP"), Weekday::Sun);
        assert_eq!(week_start("de_DE"), Weekday::Mon);
        assert_eq!(week_start("ar_SA"), Weekday::Sun);
        assert_eq!(week_start("fa_IR"), Weekday::Sat);
        assert_eq!(week_start("en"), Weekday::Mon, "no region falls back to Monday");
    }

    #[test]
    fn days_in_month_handles_leap_years() {
        assert_eq!(days_in_month(2028, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2026, 12), 31);
    }

    #[test]
    fn shift_month_crosses_the_year_boundary() {
        assert_eq!(shift_month(2026, 12, 1), (2027, 1));
        assert_eq!(shift_month(2027, 1, -1), (2026, 12));
        assert_eq!(shift_month(2026, 10, 0), (2026, 10));
    }

    #[test]
    fn month_title_is_english_month_and_year() {
        assert_eq!(month_title(2026, 10), "October 2026");
    }

    #[test]
    fn grid_has_whole_weeks_and_the_days_in_order() {
        let cells = month_grid(2026, 10, Weekday::Mon);
        assert_eq!(cells.len() % 7, 0);
        assert_eq!(cells[3], Some(d(2026, 10, 1)), "October 1 is a Thursday, the fourth Monday-first cell");
        assert_eq!(cells.iter().flatten().count(), 31);
    }

    #[test]
    fn keys_move_by_day_week_and_month() {
        let focus = d(2026, 10, 8);
        assert_eq!(key_move(focus, CalKey::Right), d(2026, 10, 9));
        assert_eq!(key_move(focus, CalKey::Left), d(2026, 10, 7));
        assert_eq!(key_move(focus, CalKey::Down), d(2026, 10, 15));
        assert_eq!(key_move(focus, CalKey::Up), d(2026, 10, 1));
        assert_eq!(key_move(focus, CalKey::PageDown), d(2026, 11, 8));
        assert_eq!(key_move(d(2026, 1, 31), CalKey::PageDown), d(2026, 2, 28), "day clamps to the shorter month");
        assert_eq!(key_move(d(2026, 1, 15), CalKey::PageUp), d(2025, 12, 15));
    }

    #[test]
    fn past_days_dim_and_today_does_not() {
        let today = d(2026, 10, 8);
        assert_eq!(day_alpha(d(2026, 10, 7), today), PAST_DAY_ALPHA);
        assert_eq!(day_alpha(today, today), 1.0);
        assert_eq!(day_alpha(d(2026, 10, 9), today), 1.0);
    }

    #[test]
    fn dots_cap_at_three_with_rising_alpha() {
        assert!(dots(0).is_empty());
        assert_eq!(dots(5).len(), 3, "five entries draw three dots");
        let alphas = dots(3);
        assert!((alphas[0] - 0.3).abs() < 1e-9 && (alphas[1] - 0.6).abs() < 1e-9 && (alphas[2] - 0.9).abs() < 1e-9);
    }

    #[test]
    fn weekday_headers_are_one_letter() {
        let letters: Vec<String> = week_days(Weekday::Mon).iter().map(|d| weekday_narrow(*d)).collect();
        assert_eq!(letters, ["M", "T", "W", "T", "F", "S", "S"]);
    }

    #[test]
    fn the_title_slides_forward_only_towards_a_later_month() {
        assert!(moving_forward((2026, 10), (2026, 11)));
        assert!(moving_forward((2026, 12), (2027, 1)));
        assert!(!moving_forward((2026, 10), (2026, 9)));
        assert!(!moving_forward((2027, 1), (2026, 12)));
    }

    #[test]
    fn the_click_area_is_a_circle_not_the_square_cell() {
        assert!(in_circle(20.0, 20.0, 40.0, 40.0), "the middle");
        assert!(in_circle(1.0, 20.0, 40.0, 40.0), "the left edge of the circle");
        assert!(!in_circle(2.0, 2.0, 40.0, 40.0), "a corner of the cell is outside the circle");
        assert!(!in_circle(39.5, 39.5, 40.0, 40.0));
        assert!(in_circle(30.0, 5.0, 60.0, 40.0), "a wide cell uses the shorter side as the diameter");
        assert!(!in_circle(5.0, 20.0, 60.0, 40.0));
    }
}
