//! Time formatting helpers (area 45 and 46). `pretty_time` follows mpv's `Utils.prettyTime`: `mm:ss`
//! under one hour and `h:mm:ss` from one hour on. Negative input is formatted as its magnitude.

/// `mm:ss` below one hour, `h:mm:ss` otherwise. Seconds are truncated toward zero.
pub fn pretty_time(seconds: i64) -> String {
    let total = seconds.unsigned_abs();
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// `pretty_time` with a leading `+` or `-`. Zero is `+`.
pub fn pretty_time_signed(seconds: i64) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    format!("{sign}{}", pretty_time(seconds))
}

/// Episode number as `DecimalFormat("#.###")` prints it: at most three decimals and no trailing
/// zeros. The leading zero is kept (`0.5` gives `0.5`, D-10 and E-46-022). Rounds half to even on
/// the binary value, so `1.2345` gives `1.234`.
pub fn format_episode_number(value: f64) -> String {
    let fixed = format!("{:.3}", value.abs());
    let body = fixed.trim_end_matches('0').trim_end_matches('.');
    if value < 0.0 && body != "0" {
        format!("-{body}")
    } else {
        body.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_an_hour_is_minutes_and_seconds() {
        assert_eq!(pretty_time(0), "00:00");
        assert_eq!(pretty_time(75), "01:15");
        assert_eq!(pretty_time(3599), "59:59");
    }

    #[test]
    fn an_hour_or_more_has_hours() {
        assert_eq!(pretty_time(3600), "1:00:00");
        assert_eq!(pretty_time(3725), "1:02:05");
    }

    #[test]
    fn signed_form_prefixes_the_sign() {
        assert_eq!(pretty_time_signed(65), "+01:05");
        assert_eq!(pretty_time_signed(-65), "-01:05");
        assert_eq!(pretty_time_signed(0), "+00:00");
    }

    #[test]
    fn episode_numbers_drop_trailing_zeros_but_keep_the_leading_zero() {
        assert_eq!(format_episode_number(1.0), "1");
        assert_eq!(format_episode_number(12.5), "12.5");
        assert_eq!(format_episode_number(1.2345), "1.234");
        assert_eq!(format_episode_number(0.5), "0.5");
        assert_eq!(format_episode_number(0.0), "0");
        assert_eq!(format_episode_number(-2.25), "-2.25");
    }
}
