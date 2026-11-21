//! Absolute date formats for the `app_date_format` preference (plan 51-i18n T-51a-017).
//!
//! Numeric patterns come from chrono. Month names and the locale short date come from ICU4X
//! (`icu_datetime`), so they follow the resolved UI locale (test T24).

use chrono::{Datelike, NaiveDate};
use icu_datetime::input::Date as IcuDate;
use icu_datetime::{DateTimeFormatter, fieldsets};
use icu_locale::Locale;

/// One choice of the Appearance "Date format" list. The stored value of each choice is the
/// Java pattern it came from, so existing preferences load unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateFormat {
    /// `""` (Default): the locale's short date.
    LocaleShort,
    /// `MM/dd/yy`
    MonthDayYear,
    /// `dd/MM/yy`
    DayMonthYear,
    /// `yyyy-MM-dd`
    IsoDate,
    /// `dd MMM yyyy`
    DayMonthNameYear,
    /// `MMM dd, yyyy`
    MonthNameDayYear,
}

impl DateFormat {
    /// The format for a stored `app_date_format` value. An unknown value is the locale short date.
    pub fn from_pref(raw: &str) -> Self {
        match raw {
            "MM/dd/yy" => Self::MonthDayYear,
            "dd/MM/yy" => Self::DayMonthYear,
            "yyyy-MM-dd" => Self::IsoDate,
            "dd MMM yyyy" => Self::DayMonthNameYear,
            "MMM dd, yyyy" => Self::MonthNameDayYear,
            _ => Self::LocaleShort,
        }
    }

    /// The value to store in `app_date_format`.
    pub fn pref(self) -> &'static str {
        match self {
            Self::LocaleShort => "",
            Self::MonthDayYear => "MM/dd/yy",
            Self::DayMonthYear => "dd/MM/yy",
            Self::IsoDate => "yyyy-MM-dd",
            Self::DayMonthNameYear => "dd MMM yyyy",
            Self::MonthNameDayYear => "MMM dd, yyyy",
        }
    }

    /// `date` in this format, with month names in `locale` (a BCP 47 tag such as `de`).
    pub fn format(self, date: NaiveDate, locale: &str) -> String {
        let day = date.format("%d").to_string();
        let year = date.format("%Y").to_string();
        match self {
            Self::LocaleShort => short_date(date, locale),
            Self::MonthDayYear => date.format("%m/%d/%y").to_string(),
            Self::DayMonthYear => date.format("%d/%m/%y").to_string(),
            Self::IsoDate => date.format("%Y-%m-%d").to_string(),
            Self::DayMonthNameYear => format!("{day} {} {year}", month_abbr(date, locale)),
            Self::MonthNameDayYear => format!("{} {day}, {year}", month_abbr(date, locale)),
        }
    }
}

/// The locale for a tag, or English when the tag does not parse. The numbering system is forced to
/// Latin digits (`-u-nu-latn`), so ICU's native digits (Arabic-Indic and the like) never appear
/// (T-51a-033: ASCII digits in every locale).
pub(crate) fn locale_or_english(tag: &str) -> Locale {
    Locale::try_from_str(&format!("{tag}-u-nu-latn"))
        .or_else(|_| Locale::try_from_str("en-u-nu-latn"))
        .expect("en is a valid locale")
}

/// The locale short date (ICU4X, length Short). Falls back to ISO when ICU data is unavailable.
// ponytail: builds the formatter per call; cache per locale if date lists redraw slowly.
fn short_date(date: NaiveDate, locale: &str) -> String {
    let fallback = || date.format("%Y-%m-%d").to_string();
    let Ok(input) = IcuDate::try_new_iso(date.year(), date.month() as u8, date.day() as u8) else {
        return fallback();
    };
    match DateTimeFormatter::try_new(locale_or_english(locale).into(), fieldsets::YMD::short()) {
        Ok(formatter) => formatter.format(&input).to_string(),
        Err(_) => fallback(),
    }
}

/// The abbreviated month name for `date` in `locale` (ICU4X, standalone month).
fn month_abbr(date: NaiveDate, locale: &str) -> String {
    let fallback = || date.format("%b").to_string();
    let Ok(input) = IcuDate::try_new_iso(date.year(), date.month() as u8, date.day() as u8) else {
        return fallback();
    };
    match DateTimeFormatter::try_new(locale_or_english(locale).into(), fieldsets::M::medium()) {
        Ok(formatter) => formatter.format(&input).to_string(),
        Err(_) => fallback(),
    }
}

/// One row of the Appearance date-format list with today's date in it (T-51a-041).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatePreview {
    /// The stored `app_date_format` value of this choice.
    pub pref: &'static str,
    /// `Default (<date>)` for the locale short date, otherwise `<pattern> (<date>)`.
    pub label: String,
}

/// The six choices in list order, each with `today` formatted in it. The Settings screen calls this
/// once when it opens, not on every frame.
pub fn previews(today: NaiveDate, locale: &str) -> Vec<DatePreview> {
    const CHOICES: [DateFormat; 6] = [
        DateFormat::LocaleShort,
        DateFormat::MonthDayYear,
        DateFormat::DayMonthYear,
        DateFormat::IsoDate,
        DateFormat::DayMonthNameYear,
        DateFormat::MonthNameDayYear,
    ];
    CHOICES
        .into_iter()
        .map(|format| {
            let shown = format.format(today, locale);
            let name = match format {
                DateFormat::LocaleShort => crate::tr(crate::MsgKey::label_default),
                other => other.pref().to_owned(),
            };
            DatePreview { pref: format.pref(), label: format!("{name} ({shown})") }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 9).expect("valid date")
    }

    #[test]
    fn fixed_patterns_use_chrono_digits() {
        assert_eq!(DateFormat::MonthDayYear.format(day(), "en"), "10/09/26");
        assert_eq!(DateFormat::DayMonthYear.format(day(), "en"), "09/10/26");
        assert_eq!(DateFormat::IsoDate.format(day(), "en"), "2026-10-09");
    }

    #[test]
    fn month_names_come_from_the_locale() {
        // T24: dd MMM yyyy and MMM dd, yyyy in de use the German month name.
        let de = DateFormat::DayMonthNameYear.format(day(), "de");
        assert!(de.starts_with("09 ") && de.ends_with(" 2026"), "{de}");
        assert!(de.contains("Okt"), "{de}");
        let en = DateFormat::MonthNameDayYear.format(day(), "en");
        assert_eq!(en, "Oct 09, 2026");
    }

    #[test]
    fn locale_short_date_is_locale_specific_and_non_empty() {
        // T23: the short date is ICU output for the locale (snapshot for en-US).
        let us = DateFormat::LocaleShort.format(day(), "en-US");
        assert_eq!(us, "10/9/26");
        assert!(!DateFormat::LocaleShort.format(day(), "de").is_empty());
    }

    #[test]
    fn stored_values_round_trip_and_unknown_is_default() {
        for format in [
            DateFormat::LocaleShort,
            DateFormat::MonthDayYear,
            DateFormat::DayMonthYear,
            DateFormat::IsoDate,
            DateFormat::DayMonthNameYear,
            DateFormat::MonthNameDayYear,
        ] {
            assert_eq!(DateFormat::from_pref(format.pref()), format);
        }
        assert_eq!(DateFormat::from_pref("bogus"), DateFormat::LocaleShort);
    }

    #[test]
    fn previews_show_each_choice_with_todays_date_in_order() {
        let rows = previews(day(), "en");
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels[0], "Default (10/9/26)");
        assert_eq!(labels[1], "MM/dd/yy (10/09/26)");
        assert_eq!(labels[2], "dd/MM/yy (09/10/26)");
        assert_eq!(labels[3], "yyyy-MM-dd (2026-10-09)");
        assert_eq!(labels[4], "dd MMM yyyy (09 Oct 2026)");
        assert_eq!(labels[5], "MMM dd, yyyy (Oct 09, 2026)");
        assert_eq!(rows[0].pref, "");
    }

    #[test]
    fn short_dates_use_ascii_digits_in_every_offered_locale() {
        for tag in ["ar", "fa", "bn", "ne", "mr", "my", "km", "am", "sd", "hi", "th"] {
            let text = DateFormat::LocaleShort.format(day(), tag);
            assert!(!text.chars().any(|c| c.is_numeric() && !c.is_ascii_digit()), "{tag}: {text}");
            let month = DateFormat::DayMonthNameYear.format(day(), tag);
            assert!(!month.chars().any(|c| c.is_numeric() && !c.is_ascii_digit()), "{tag}: {month}");
        }
    }
}
