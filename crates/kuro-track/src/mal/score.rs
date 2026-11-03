//! MAL score model. Scores are 0 to 10, and the
//! stored score equals the index in [`SCORE_LIST`].

/// Labels the score picker offers (`MyAnimeList.kt:49-51`).
pub const SCORE_LIST: [&str; 11] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"];

/// D-11 switch. `false` keeps upstream's `Double.toString` form (`"8.0"`). `true` sends the integer (`"8"`).
pub const WIRE_INTEGER_SCORE: bool = false;

pub fn index_to_score(index: usize) -> f64 {
    index as f64
}

/// The score of a picker label. `None` for labels outside the list (D-12: no -1 sentinel).
pub fn score_from_label(label: &str) -> Option<f64> {
    SCORE_LIST
        .iter()
        .position(|s| *s == label)
        .map(index_to_score)
}

/// Shown score: truncated, as Kotlin `toInt()` does (`MyAnimeList.kt:109-115`).
pub fn display_score(score: f64) -> String {
    (score as i64).to_string()
}

/// Score as sent to MAL, using [`WIRE_INTEGER_SCORE`].
pub fn wire_score(score: f64) -> String {
    wire_score_with(score, WIRE_INTEGER_SCORE)
}

pub fn wire_score_with(score: f64, integer: bool) -> String {
    if integer {
        (score as i64).to_string()
    } else {
        format!("{score:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_index_roundtrip() {
        for (i, label) in SCORE_LIST.iter().enumerate() {
            assert_eq!(index_to_score(i), i as f64);
            assert_eq!(display_score(index_to_score(i)), *label);
            assert_eq!(score_from_label(label), Some(i as f64));
        }
        assert_eq!(SCORE_LIST.len(), 11);
    }

    #[test]
    fn score_unknown_label() {
        assert_eq!(score_from_label("11"), None);
        assert_eq!(score_from_label(""), None);
        assert_eq!(score_from_label("foo"), None);
    }

    #[test]
    fn score_wire_default() {
        assert_eq!(wire_score(8.0), "8.0");
        assert_eq!(wire_score(0.0), "0.0");
    }

    #[test]
    fn score_wire_integer_switch() {
        assert_eq!(wire_score_with(8.0, true), "8");
        assert_eq!(wire_score_with(8.0, false), "8.0");
    }

    #[test]
    fn score_display_truncates() {
        assert_eq!(display_score(8.9), "8");
        assert_eq!(display_score(7.9), "7");
        assert_eq!(display_score(0.0), "0");
    }
}
