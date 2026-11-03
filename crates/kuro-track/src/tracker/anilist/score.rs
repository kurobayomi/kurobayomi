//! Score formats, picker lists and conversions. The stored score is always on
//! the 0 to 100 scale. The picker works on indices, so an unscored POINT_3 opens at index 0 (E-13).

use super::error::AniListError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoreFormat {
    Point100,
    Point10,
    Point10Decimal,
    Point5,
    Point3,
}

impl ScoreFormat {
    /// Parses the AniList `ScoreFormat` string. Other text is [`AniListError::UnknownScoreFormat`].
    pub fn from_api(s: &str) -> Result<Self, AniListError> {
        match s {
            "POINT_100" => Ok(ScoreFormat::Point100),
            "POINT_10" => Ok(ScoreFormat::Point10),
            "POINT_10_DECIMAL" => Ok(ScoreFormat::Point10Decimal),
            "POINT_5" => Ok(ScoreFormat::Point5),
            "POINT_3" => Ok(ScoreFormat::Point3),
            other => Err(AniListError::UnknownScoreFormat(other.to_owned())),
        }
    }

    /// Like [`ScoreFormat::from_api`], but an unknown format falls back to POINT_10 with a warning.
    pub fn from_api_or_default(s: &str) -> Self {
        Self::from_api(s).unwrap_or_else(|e| {
            tracing::warn!(target: "kurobayomi::tracker::anilist", error = %e, "falling back to POINT_10");
            ScoreFormat::Point10
        })
    }

    pub fn to_api(self) -> &'static str {
        match self {
            ScoreFormat::Point100 => "POINT_100",
            ScoreFormat::Point10 => "POINT_10",
            ScoreFormat::Point10Decimal => "POINT_10_DECIMAL",
            ScoreFormat::Point5 => "POINT_5",
            ScoreFormat::Point3 => "POINT_3",
        }
    }

    /// Picker labels in index order.
    pub fn options(self) -> Vec<String> {
        match self {
            ScoreFormat::Point10 => (0..=10).map(|i| i.to_string()).collect(),
            ScoreFormat::Point100 => (0..=100).map(|i| i.to_string()).collect(),
            ScoreFormat::Point5 => (0..=5).map(|i| format!("{i} ★")).collect(),
            ScoreFormat::Point3 => ["-", "😦", "😐", "😊"].map(String::from).to_vec(),
            // Kotlin formats the Float `i / 10f`; f32 Debug gives the same shortest text.
            ScoreFormat::Point10Decimal => (0..=100)
                .map(|i| format!("{:?}", i as f32 / 10.0))
                .collect(),
        }
    }

    /// Index in [`ScoreFormat::options`] to the stored 0 to 100 score (p1-behavior 3.4).
    pub fn index_to_score(self, index: usize) -> Result<f64, AniListError> {
        if index >= self.options().len() {
            return Err(AniListError::ScoreOutOfRange);
        }
        let i = index as f64;
        Ok(match self {
            ScoreFormat::Point10 => i * 10.0,
            ScoreFormat::Point100 | ScoreFormat::Point10Decimal => i,
            ScoreFormat::Point5 => {
                if index == 0 {
                    0.0
                } else {
                    i * 20.0 - 10.0
                }
            }
            ScoreFormat::Point3 => {
                if index == 0 {
                    0.0
                } else {
                    i * 25.0 + 10.0
                }
            }
        })
    }

    /// Stored score to the picker index that shows it. Matches [`ScoreFormat::display`]: the index shown
    /// by the picker is the one whose stored score displays the same text.
    pub fn score_to_index(self, score: f64) -> usize {
        let max = self.options().len() - 1;
        let raw: i64 = match self {
            ScoreFormat::Point3 => {
                if score <= 0.0 {
                    0
                } else if score <= 35.0 {
                    1
                } else if score <= 60.0 {
                    2
                } else {
                    3
                }
            }
            ScoreFormat::Point5 => {
                if score <= 0.0 {
                    0
                } else {
                    (((score + 10.0) / 20.0).floor() as i64).max(1)
                }
            }
            ScoreFormat::Point10 => (score / 10.0).floor() as i64,
            ScoreFormat::Point100 | ScoreFormat::Point10Decimal => score.floor() as i64,
        };
        raw.clamp(0, max as i64) as usize
    }

    /// Stored score to the text shown on the track row (p1-behavior 3.5, p1-data section 11).
    pub fn display(self, score: f64) -> String {
        match self {
            ScoreFormat::Point5 => {
                if score == 0.0 {
                    "0 ★".to_owned()
                } else {
                    format!("{} ★", ((score + 10.0) / 20.0).floor() as i64)
                }
            }
            ScoreFormat::Point3 => {
                if score == 0.0 {
                    "0".to_owned()
                } else if score <= 35.0 {
                    "😦".to_owned()
                } else if score <= 60.0 {
                    "😐".to_owned()
                } else {
                    "😊".to_owned()
                }
            }
            ScoreFormat::Point10 => ((score as i64) / 10).to_string(),
            ScoreFormat::Point100 => (score as i64).to_string(),
            ScoreFormat::Point10Decimal => format!("{:?}", score / 10.0),
        }
    }

    /// Stored score to the integer `scoreRaw` sent to AniList: truncated, clamped to 0 to 100 (E-14).
    pub fn to_score_raw(score: f64) -> i32 {
        if score.is_nan() {
            return 0;
        }
        (score.trunc() as i64).clamp(0, 100) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const ALL: [ScoreFormat; 5] = [
        ScoreFormat::Point100,
        ScoreFormat::Point10,
        ScoreFormat::Point10Decimal,
        ScoreFormat::Point5,
        ScoreFormat::Point3,
    ];

    #[test]
    fn t_28a_005_known_formats_parse_and_unknown_is_an_error() -> TestResult {
        for f in ALL {
            assert_eq!(ScoreFormat::from_api(f.to_api())?, f);
        }
        assert!(matches!(
            ScoreFormat::from_api("SOMETHING_NEW"),
            Err(AniListError::UnknownScoreFormat(s)) if s == "SOMETHING_NEW"
        ));
        assert_eq!(
            ScoreFormat::from_api_or_default("SOMETHING_NEW"),
            ScoreFormat::Point10,
            "E-15 fallback"
        );
        Ok(())
    }

    #[test]
    fn t_28a_005_option_counts_and_edge_labels() {
        assert_eq!(ScoreFormat::Point10.options().len(), 11);
        assert_eq!(ScoreFormat::Point100.options().len(), 101);
        assert_eq!(ScoreFormat::Point5.options().len(), 6);
        assert_eq!(ScoreFormat::Point3.options().len(), 4);
        assert_eq!(ScoreFormat::Point10Decimal.options().len(), 101);
        assert_eq!(ScoreFormat::Point3.options(), vec!["-", "😦", "😐", "😊"]);
        let five = ScoreFormat::Point5.options();
        assert_eq!(five.first().map(String::as_str), Some("0 ★"));
        assert_eq!(five.last().map(String::as_str), Some("5 ★"));
        let dec = ScoreFormat::Point10Decimal.options();
        assert_eq!(dec.first().map(String::as_str), Some("0.0"));
        assert_eq!(dec.last().map(String::as_str), Some("10.0"));
        assert_eq!(dec.get(1).map(String::as_str), Some("0.1"));
    }

    #[test]
    fn t_28a_006_index_to_score_table() -> TestResult {
        use ScoreFormat::*;
        assert_eq!(Point3.index_to_score(3)?, 85.0);
        assert_eq!(Point3.index_to_score(1)?, 35.0);
        assert_eq!(Point3.index_to_score(0)?, 0.0);
        assert_eq!(Point5.index_to_score(5)?, 90.0);
        assert_eq!(Point5.index_to_score(2)?, 30.0);
        assert_eq!(Point10.index_to_score(3)?, 30.0);
        assert_eq!(Point10.index_to_score(10)?, 100.0);
        assert_eq!(Point100.index_to_score(10)?, 10.0);
        assert_eq!(Point10Decimal.index_to_score(85)?, 85.0);
        Ok(())
    }

    #[test]
    fn t_28a_006_out_of_range_index_is_an_error() {
        assert!(matches!(
            ScoreFormat::Point3.index_to_score(4),
            Err(AniListError::ScoreOutOfRange)
        ));
        assert!(matches!(
            ScoreFormat::Point10.index_to_score(11),
            Err(AniListError::ScoreOutOfRange)
        ));
    }

    #[test]
    fn t_28a_006_display_table_rows() {
        use ScoreFormat::*;
        let rows: [(f64, [&str; 5]); 11] = [
            (0.0, ["0", "0", "0.0", "0 ★", "0"]),
            (10.0, ["10", "1", "1.0", "1 ★", "😦"]),
            (30.0, ["30", "3", "3.0", "2 ★", "😦"]),
            (35.0, ["35", "3", "3.5", "2 ★", "😦"]),
            (36.0, ["36", "3", "3.6", "2 ★", "😐"]),
            (50.0, ["50", "5", "5.0", "3 ★", "😐"]),
            (60.0, ["60", "6", "6.0", "3 ★", "😐"]),
            (61.0, ["61", "6", "6.1", "3 ★", "😊"]),
            (70.0, ["70", "7", "7.0", "4 ★", "😊"]),
            (90.0, ["90", "9", "9.0", "5 ★", "😊"]),
            (100.0, ["100", "10", "10.0", "5 ★", "😊"]),
        ];
        for (score, [p100, p10, dec, p5, p3]) in rows {
            assert_eq!(Point100.display(score), p100, "POINT_100 at {score}");
            assert_eq!(Point10.display(score), p10, "POINT_10 at {score}");
            assert_eq!(Point10Decimal.display(score), dec, "DECIMAL at {score}");
            assert_eq!(Point5.display(score), p5, "POINT_5 at {score}");
            assert_eq!(Point3.display(score), p3, "POINT_3 at {score}");
        }
    }

    #[test]
    fn t_28a_006_round_trip_keeps_the_shown_text() {
        let values = [
            0.0, 10.0, 30.0, 35.0, 36.0, 50.0, 60.0, 61.0, 70.0, 90.0, 100.0,
        ];
        for f in ALL {
            for v in values {
                let idx = f.score_to_index(v);
                let back = f.index_to_score(idx).unwrap_or(f64::NAN);
                assert_eq!(f.display(back), f.display(v), "{f:?} at {v}");
            }
        }
    }

    #[test]
    fn t_28a_006_point3_unscored_opens_at_zero() {
        assert_eq!(ScoreFormat::Point3.score_to_index(0.0), 0, "E-13");
    }

    #[test]
    fn t_28a_006_to_score_raw_clamps_and_truncates() {
        assert_eq!(ScoreFormat::to_score_raw(-10.0), 0, "E-14");
        assert_eq!(ScoreFormat::to_score_raw(150.0), 100);
        assert_eq!(ScoreFormat::to_score_raw(85.4), 85);
        assert_eq!(ScoreFormat::to_score_raw(f64::NAN), 0);
    }
}
