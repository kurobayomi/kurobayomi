//! Counts written short, for the likes and replies of comments (docs/design/comments.md): `999`, `1.2K`, `34K`,
//! `1.2M`. The digits and the decimal separator come from the UI locale (ICU4X, Latin digits as everywhere in the app);
//! the `K` and `M` are messages, so a language can write its own.
//!
//! ICU4X's own compact formatter sits behind its `unstable` feature, so the app does not use it.

use icu_decimal::DecimalFormatter;
use icu_decimal::input::Decimal;

use crate::date::locale_or_english;
use crate::{Arg, MsgKey, current_locale, trf};

/// A count split into its digits (in the locale's writing) and its unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Compact {
    /// Under a thousand: the count as it is.
    Plain(String),
    Thousands(String),
    Millions(String),
}

/// `count` in `locale`: whole units from ten of them up (`34K`), one decimal under ten (`1.2K`), never rounded up, so
/// 999,999 is `999K` and not `1000K`. A negative count is 0.
pub fn compact_in(locale: &str, count: i64) -> Compact {
    let count = count.max(0);
    let (unit, wrap): (i64, fn(String) -> Compact) = match count {
        0..1_000 => return Compact::Plain(digits(locale, Decimal::from(count))),
        1_000..1_000_000 => (1_000, Compact::Thousands),
        _ => (1_000_000, Compact::Millions),
    };
    let value = if count < 10 * unit {
        let tenths = count * 10 / unit;
        if tenths % 10 == 0 {
            Decimal::from(tenths / 10)
        } else {
            let mut value = Decimal::from(tenths);
            value.multiply_pow10(-1);
            value
        }
    } else {
        Decimal::from(count / unit)
    };
    wrap(digits(locale, value))
}

/// `value` written in `locale`; plain ASCII digits when ICU has no data for it.
// ponytail: builds the formatter per call; cache it per locale if a long list redraws slowly.
fn digits(locale: &str, value: Decimal) -> String {
    match DecimalFormatter::try_new(locale_or_english(locale).into(), Default::default()) {
        Ok(formatter) => formatter.format_to_string(&value),
        Err(_) => value.to_string(),
    }
}

/// `count` written short in the UI language.
pub fn compact_count(count: i64) -> String {
    match compact_in(current_locale().tag, count) {
        Compact::Plain(text) => text,
        Compact::Thousands(value) => trf(MsgKey::number_thousands, &[("value", Arg::Str(&value))]),
        Compact::Millions(value) => trf(MsgKey::number_millions, &[("value", Arg::Str(&value))]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn en(count: i64) -> Compact {
        compact_in("en", count)
    }

    #[test]
    fn small_counts_stay_as_they_are() {
        assert_eq!(en(0), Compact::Plain("0".into()));
        assert_eq!(en(999), Compact::Plain("999".into()));
        assert_eq!(en(-3), Compact::Plain("0".into()));
    }

    #[test]
    fn thousands_have_one_decimal_under_ten_and_none_above() {
        assert_eq!(en(1_000), Compact::Thousands("1".into()));
        assert_eq!(en(1_250), Compact::Thousands("1.2".into()), "never rounded up");
        assert_eq!(en(9_999), Compact::Thousands("9.9".into()));
        assert_eq!(en(34_000), Compact::Thousands("34".into()));
        assert_eq!(en(999_999), Compact::Thousands("999".into()));
    }

    #[test]
    fn millions_follow_the_same_rule() {
        assert_eq!(en(1_200_000), Compact::Millions("1.2".into()));
        assert_eq!(en(12_000_000), Compact::Millions("12".into()));
    }

    #[test]
    fn the_decimal_separator_is_the_locales_and_digits_stay_latin() {
        assert_eq!(compact_in("de", 1_250), Compact::Thousands("1,2".into()));
        assert_eq!(compact_in("fr", 1_200_000), Compact::Millions("1,2".into()));
        assert_eq!(compact_in("ar", 1_250), compact_in("ar", 1_250), "builds");
        let Compact::Thousands(arabic) = compact_in("ar", 1_250) else { panic!("a thousand") };
        assert!(arabic.starts_with('1') && arabic.ends_with('2'), "Latin digits: {arabic}");
    }

    #[test]
    fn english_writes_k_and_m() {
        // before `init` the catalog is English
        assert_eq!(compact_count(1_250), "1.2K");
        assert_eq!(compact_count(1_200_000), "1.2M");
        assert_eq!(compact_count(42), "42");
    }
}
