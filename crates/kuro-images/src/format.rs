//! Chapter and episode number text (T-34a-056).
//!
//! Java's `DecimalFormat("#.###")` with `.` as the decimal separator: at most three fraction digits, no trailing
//! zeros, no grouping, rounding HALF_EVEN on the shortest decimal text of the number, ASCII digits.

/// `formatChapterNumber` and `formatEpisodeNumber`, which are the same function in Kotlin.
pub fn item_number_text(n: f64) -> String {
    if n.is_nan() {
        return "NaN".to_owned();
    }
    if n.is_infinite() {
        return if n > 0.0 { "∞" } else { "-∞" }.to_owned();
    }
    let negative = n < 0.0;
    // Rust prints the shortest text that reads back as the same number, and never uses an exponent.
    let text = n.abs().to_string();
    let (int, frac) = text.split_once('.').unwrap_or((&text, ""));
    let mut digits: Vec<u8> = int.bytes().collect();
    let kept = frac.len().min(3);
    digits.extend(frac.bytes().take(kept));
    if round_up(&digits, &frac[kept..]) {
        increment(&mut digits);
    }
    // The carry can add a leading digit, so the integer part is measured from the end.
    let split = digits.len() - kept;
    let (int, frac) = digits.split_at(split);
    let int = String::from_utf8_lossy(int);
    let frac = String::from_utf8_lossy(frac);
    let frac = frac.trim_end_matches('0');
    let body = if frac.is_empty() {
        int.into_owned()
    } else {
        format!("{int}.{frac}")
    };
    // `-0.0004` is `0`, never `-0` (G-NUM negative zero).
    if negative && body != "0" {
        format!("-{body}")
    } else {
        body
    }
}

/// HALF_EVEN: the dropped digits `rest` round up when they are above one half, or exactly one half and the last
/// kept digit is odd.
fn round_up(kept: &[u8], rest: &str) -> bool {
    let mut rest = rest.bytes();
    match rest.next() {
        None | Some(b'0'..=b'4') => false,
        Some(b'6'..=b'9') => true,
        Some(_) => rest.any(|d| d != b'0') || kept.last().is_some_and(|d| (d - b'0') % 2 == 1),
    }
}

/// Adds one to the last digit of a decimal digit string, carrying to the left.
fn increment(digits: &mut Vec<u8>) {
    for d in digits.iter_mut().rev() {
        if *d == b'9' {
            *d = b'0';
        } else {
            *d += 1;
            return;
        }
    }
    digits.insert(0, b'1');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g_num_table() {
        let rows: &[(f64, &str)] = &[
            (0.0, "0"),
            (0.5, "0.5"),
            (1.0, "1"),
            (1.5, "1.5"),
            (1.3333, "1.333"),
            (1.0005, "1"),
            (1.0015, "1.002"),
            (1.0025, "1.002"),
            (2.25, "2.25"),
            (-0.5, "-0.5"),
            (10.0, "10"),
            (0.0005, "0"),
            (0.0004, "0"),
            (100.1234, "100.123"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "∞"),
        ];
        for (n, want) in rows {
            assert_eq!(item_number_text(*n), *want, "{n}");
        }
    }

    #[test]
    fn negative_infinity_and_negative_zero() {
        assert_eq!(item_number_text(f64::NEG_INFINITY), "-∞");
        assert_eq!(item_number_text(-0.0), "0");
        assert_eq!(item_number_text(-0.0004), "0");
    }

    #[test]
    fn ties_go_to_the_even_digit_and_the_carry_propagates() {
        assert_eq!(item_number_text(0.0015), "0.002");
        assert_eq!(item_number_text(0.0025), "0.002");
        assert_eq!(item_number_text(0.0035), "0.004");
        assert_eq!(item_number_text(0.00051), "0.001");
        assert_eq!(item_number_text(9.9996), "10");
        assert_eq!(item_number_text(99.9995), "100");
        assert_eq!(item_number_text(0.9996), "1");
        assert_eq!(item_number_text(-1.0006), "-1.001");
    }

    #[test]
    fn large_and_small_values_have_no_exponent_or_grouping() {
        assert_eq!(item_number_text(1234567.0), "1234567");
        assert_eq!(item_number_text(1e21), "1000000000000000000000");
        assert_eq!(item_number_text(1e-7), "0");
        assert_eq!(item_number_text(12.0001), "12");
    }

    #[test]
    fn digits_are_ascii() {
        assert!(item_number_text(-123.456).is_ascii());
    }
}
