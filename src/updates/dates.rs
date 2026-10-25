//! Date headers of the History and Updates lists (T-42a-002). The absolute date is the `app_date_format` preference
//! through `kuro_i18n::date::DateFormat` (an empty value is the locale short date); the relative words are the
//! `relative_time_today` and `relative_time` messages. Pure: the caller passes today, the preference and the locale.

use chrono::{NaiveDate, TimeZone};
use kuro_domain::dates::{ListRow, local_date};
use kuro_i18n::date::DateFormat;
use kuro_i18n::{MsgKey, tr, tr_plural};

use crate::updates::format::{RelativeDate, relative_date};

/// The text of a date header: Today, Yesterday and "N days ago" (days 2 to 6) while `relative` is on, otherwise the
/// absolute date in the stored `pref` format.
pub fn header_text(date: NaiveDate, today: NaiveDate, relative: bool, pref: &str, locale: &str) -> String {
    // the thresholds are `format::relative_date`'s (T-42a-002); a date after today has no relative word in these lists
    match relative_date(date, today, relative, "") {
        RelativeDate::Today => tr(MsgKey::relative_time_today),
        RelativeDate::Ago(days) => tr_plural(MsgKey::relative_time, days, &[]),
        RelativeDate::InDays(_) | RelativeDate::Absolute(_) => DateFormat::from_pref(pref).format(date, locale),
    }
}

/// Groups `(item, epoch ms)` pairs, already in display order, under a header per local date. An item whose timestamp
/// cannot be placed on a date stays in the list under the previous header.
pub fn grouped<T, Tz: TimeZone>(
    items: impl IntoIterator<Item = (T, i64)>,
    zone: &Tz,
    today: NaiveDate,
    relative: bool,
    pref: &str,
    locale: &str,
) -> Vec<ListRow<T>> {
    let dated = items.into_iter().map(|(item, ms)| (item, local_date(ms, zone)));
    crate::updates::list::group_by_date(dated.collect(), |date| header_text(date, today, relative, pref, locale))
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, TimeZone, Utc};

    use super::*;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    #[test]
    fn dates_relative_words() {
        assert_eq!(header_text(d(9), d(9), true, "", "en"), "Today");
        assert_eq!(header_text(d(8), d(9), true, "", "en"), "Yesterday");
        assert_eq!(header_text(d(4), d(9), true, "", "en"), "5 days ago");
    }

    #[test]
    fn dates_a_week_ago_or_a_later_date_is_absolute() {
        assert_eq!(header_text(d(2), d(9), true, "yyyy-MM-dd", "en"), "2026-10-02", "7 days back is absolute");
        assert_eq!(header_text(d(3), d(9), true, "yyyy-MM-dd", "en"), "6 days ago");
        assert_eq!(header_text(d(10), d(9), true, "yyyy-MM-dd", "en"), "2026-10-10", "a date after today has no relative word here");
    }

    #[test]
    fn dates_empty_pattern_is_the_locale_short_date() {
        let text = header_text(d(1), d(30), true, "", "en");
        assert_eq!(text, DateFormat::LocaleShort.format(d(1), "en"));
        assert_ne!(text, "2026-10-01", "the short date of the locale is not the ISO date");
    }

    #[test]
    fn dates_group_under_one_header_per_local_day() {
        let ms = |day: u32, hour: u32| Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0).unwrap().timestamp_millis();
        let rows = grouped([("a", ms(9, 12)), ("b", ms(9, 1)), ("c", ms(8, 12))], &Utc, d(9), true, "", "en");
        assert_eq!(
            rows,
            vec![
                ListRow::Header("Today".into()),
                ListRow::Item("a"),
                ListRow::Item("b"),
                ListRow::Header("Yesterday".into()),
                ListRow::Item("c"),
            ]
        );
    }

    #[test]
    fn grouping_splits_at_local_midnight() {
        // E-U17: 23:59:59.999 and 00:00:00.001 local are on different days, so they get two headers.
        let zone = FixedOffset::east_opt(3 * 3600).unwrap();
        let local = |day: u32, h: u32, m: u32, s: u32, ms: i64| {
            zone.with_ymd_and_hms(2026, 10, day, h, m, s).unwrap().timestamp_millis() + ms
        };
        let rows = grouped([("early", local(10, 0, 0, 0, 1)), ("late", local(9, 23, 59, 59, 999))], &zone, d(10), false, "yyyy-MM-dd", "en");
        assert_eq!(
            rows,
            vec![
                ListRow::Header("2026-10-10".into()),
                ListRow::Item("early"),
                ListRow::Header("2026-10-09".into()),
                ListRow::Item("late"),
            ]
        );
    }

    #[test]
    fn grouping_keeps_a_long_day_under_its_local_dates() {
        // E-U16 (shape only): a 25-hour span crosses two local dates, one header each. The DST zone itself needs a
        // tz database, which the crate set does not carry; a fixed offset covers the grouping rule.
        let zone = FixedOffset::east_opt(0).unwrap();
        let at = |day: u32, hour: u32| zone.with_ymd_and_hms(2026, 10, day, hour, 0, 0).unwrap().timestamp_millis();
        let rows = grouped([("a", at(10, 23)), ("b", at(11, 0)), ("c", at(11, 23))], &zone, d(11), false, "yyyy-MM-dd", "en");
        let headers = rows.iter().filter(|r| matches!(r, ListRow::Header(_))).count();
        assert_eq!(headers, 2);
    }
}
