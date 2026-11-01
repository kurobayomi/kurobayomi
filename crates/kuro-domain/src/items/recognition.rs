//! Chapter, episode and season number recognition from titles (`ChapterRecognition.kt`,
//! `EpisodeRecognition.kt`, `SeasonRecognition.kt`). Algorithm and vectors: `p1-topic-item-number-recognition.md`.
//!
//! The Kotlin regexes use look-behind; the `regex` crate has none, so each look-behind is rewritten as a
//! consuming prefix with the same capture groups. `\b` and `\s` are forced to ASCII to match Java.

use std::sync::LazyLock;

use regex::{Captures, Regex};

/// `NUMBER_PATTERN`: integer part, optional `.digits` decimal, optional alpha suffix.
const NUMBER: &str = r"([0-9]+)(\.[0-9]+)?(\.?[a-z]+)?";

/// `tagRegex`: leading or trailing `[...]` and `(...)` groups (episode and season only).
const TAG: &str = r"^\[[^\]]+\]|\[[^\]]+\](?-u:\s)*$|^\([^)]+\)|\([^)]+\)(?-u:\s)*$";

/// `unwantedWhiteSpace`, as a consuming rewrite: `\s(?=extra|special|omake)` becomes `\s(extra|special|omake)`
/// and is replaced by its keyword.
const UNWANTED_WHITESPACE: &str = r"(?-u:\s)(extra|special|omake)";

/// The three patterns of one recogniser.
struct Recogniser {
    /// Prefix form of the look-behind, groups 1-3 are the number parts.
    basic: Regex,
    /// `NUMBER` on its own.
    number: Regex,
    /// Tags that carry no number (`v2`, `ver002`, `1080p`, ...).
    unwanted: Regex,
    /// Bracketed tags stripped before matching. `None` for chapters.
    tag: Option<Regex>,
}

impl Recogniser {
    fn new(basic_prefix: &str, unwanted: &str, strip_tags: bool) -> Self {
        let compile = |p: &str| Regex::new(p).expect("static recogniser pattern");
        Self {
            basic: compile(&format!("{basic_prefix}{NUMBER}")),
            number: compile(NUMBER),
            unwanted: compile(unwanted),
            tag: strip_tags.then(|| compile(TAG)),
        }
    }
}

static UNWANTED_WS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(UNWANTED_WHITESPACE).expect("static pattern"));

static CHAPTER: LazyLock<Recogniser> = LazyLock::new(|| {
    Recogniser::new(
        r"ch\. *",
        r"(?-u:\b)(?:v|ver|vol|version|volume|season|s)[^a-z]?[0-9]+",
        false,
    )
});

static EPISODE: LazyLock<Recogniser> = LazyLock::new(|| {
    Recogniser::new(
        r"(?:(?-u:\b)e\.|(?-u:\b)e|episode|(?-u:\b)ep) *",
        r"(?-u:\b)(?:v|ver|version|season|s)[^a-z]?[0-9]+|(?-u:\b)[0-9]+p(?-u:\b)|hi10",
        true,
    )
});

static SEASON: LazyLock<Recogniser> = LazyLock::new(|| {
    Recogniser::new(
        r"(?:(?-u:\b)s\.|(?-u:\b)s|season) *",
        r"(?-u:\b)[0-9]+p(?-u:\b)|[0-9]+x[0-9]+|Hi10|\([0-9]+\)",
        true,
    )
});

/// `alphaPostfix`: `a` is 0.1 through `i` is 0.9, `j` and later are 0.0.
fn alpha_postfix(c: char) -> f64 {
    let n = u32::from(c).saturating_sub(u32::from('a')) + 1;
    if n >= 10 { 0.0 } else { f64::from(n) / 10.0 }
}

/// `checkForDecimal`: the fractional part of one match.
fn addition(decimal: Option<&str>, alpha: Option<&str>) -> f64 {
    if let Some(d) = decimal.filter(|d| !d.is_empty()) {
        return d.parse().unwrap_or(0.0);
    }
    if let Some(a) = alpha.filter(|a| !a.is_empty()) {
        if a.contains("extra") {
            return 0.99;
        }
        if a.contains("omake") {
            return 0.98;
        }
        if a.contains("special") {
            return 0.97;
        }
        let trimmed = a.trim_start_matches('.');
        let mut chars = trimmed.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return alpha_postfix(c);
        }
    }
    0.0
}

/// `getXNumberFromMatch`: integer part plus the fractional part, as one f64 sum.
fn value_of(caps: &Captures<'_>) -> f64 {
    // Group 1 is `[0-9]+`, so it always parses; the fallback only satisfies the type.
    let initial = caps
        .get(1)
        .and_then(|m| m.as_str().parse::<f64>().ok())
        .unwrap_or(-1.0);
    let decimal = caps.get(2).map(|m| m.as_str());
    let alpha = caps.get(3).map(|m| m.as_str());
    initial + addition(decimal, alpha)
}

/// The 7-step algorithm of `p1-topic-item-number-recognition.md` section 2. `known` is returned as is when
/// it is `-2` or above `-1`.
fn parse(rec: &Recogniser, title: &str, name: &str, known: Option<f64>) -> f64 {
    if let Some(n) = known {
        if n == -2.0 || n > -1.0 {
            return n;
        }
    }

    let title_lc = title.to_lowercase();
    let mut s = name.to_lowercase();
    if !title_lc.is_empty() {
        s = s.replace(&title_lc, "");
    }
    let s = s.trim().replace([',', '-'], ".");
    let mut s = UNWANTED_WS.replace_all(&s, "$1").into_owned();

    if let Some(tag) = &rec.tag {
        // Each pass removes at least one non-empty match, so the loop ends.
        while tag.is_match(&s) {
            s = tag.replace_all(&s, "").into_owned();
        }
    }

    let mut matches = rec.number.captures_iter(&s);
    let Some(first) = matches.next() else {
        return known.unwrap_or(-1.0);
    };
    if matches.next().is_some() {
        let cleaned = rec.unwanted.replace_all(&s, "");
        if let Some(caps) = rec.basic.captures(&cleaned) {
            return value_of(&caps);
        }
        if let Some(caps) = rec.number.captures(&cleaned) {
            return value_of(&caps);
        }
    }
    value_of(&first)
}

/// `parseChapterNumber(mangaTitle, chapterName, chapterNumber)`.
pub fn parse_chapter_number(title: &str, name: &str, known: Option<f64>) -> f64 {
    parse(&CHAPTER, title, name, known)
}

/// `parseEpisodeNumber(animeTitle, episodeName, episodeNumber)`.
pub fn parse_episode_number(title: &str, name: &str, known: Option<f64>) -> f64 {
    parse(&EPISODE, title, name, known)
}

/// `parseSeasonNumber(animeTitle, seasonName, seasonNumber)`.
pub fn parse_season_number(title: &str, name: &str, known: Option<f64>) -> f64 {
    parse(&SEASON, title, name, known)
}

#[cfg(test)]
mod tests {
    use super::*;

    include!("../../tests/data/recognition_vectors.rs");

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// Upstream ChapterRecognitionTest vectors (`p1-topic-item-number-recognition.md` 5.1).
    #[test]
    fn upstream_chapter_vectors() {
        let rows = CHAPTER_VECTORS;
        assert_eq!(rows.len(), 57);
        for &(title, name, expected) in rows {
            let got = parse_chapter_number(title, name, None);
            assert_eq!(got.to_bits(), expected.to_bits(), "{name:?}: got {got}, expected {expected}");
        }
    }

    #[test]
    fn chapter_known_shortcut_and_no_match() {
        assert_eq!(parse_chapter_number("X", "Foo", Some(-2.0)), -2.0);
        assert_eq!(parse_chapter_number("X", "Foo", None), -1.0);
        assert_eq!(
            parse_chapter_number("X", "Foo", Some(-1.5)),
            -1.5,
            "no match returns the known value"
        );
        assert_eq!(parse_chapter_number("X", "Foo 7", Some(-0.5)), -0.5);
        assert_eq!(parse_chapter_number("X", "Foo 7", Some(5.0)), 5.0);
        assert_eq!(parse_chapter_number("X", "Foo 7", Some(-2.0)), -2.0);
        assert_eq!(parse_chapter_number("X", "Foo 7", Some(0.0)), 0.0);
        assert!(
            close(parse_chapter_number("X", "Foo 7", Some(-1.5)), 7.0),
            "-1.5 is parsed, not returned"
        );
    }

    /// Additional chapter vectors (`p1-topic-item-number-recognition.md` 5.2).
    #[test]
    fn additional_chapter_vectors() {
        assert!(
            close(parse_chapter_number("X", "Ch.4.j", None), 4.0),
            "j is not a single-letter suffix"
        );
        assert!(close(parse_chapter_number("X", "Ch.4.i", None), 4.9));
        assert!(
            close(parse_chapter_number("X", "Ch.4.ab", None), 4.0),
            "two letters, not a keyword"
        );
        assert!(close(parse_chapter_number("X", "Chapter 12", None), 12.0));
        assert!(close(parse_chapter_number("X", "12", None), 12.0));
        assert!(close(parse_chapter_number("X", "Ch.0", None), 0.0));
        assert!(close(
            parse_chapter_number("X", "Vol.2 Ch.10 Part 3", None),
            10.0
        ));
        assert!(close(parse_chapter_number("X", "Ch.10-15", None), 10.15));
        assert!(close(
            parse_chapter_number("Ayame", "Ayame 14 - 7", None),
            14.0
        ));
        assert!(
            close(parse_chapter_number("", "Ch. 5", None), 5.0),
            "empty title is a no-op"
        );
    }

    /// Episode vectors (`p1-topic-item-number-recognition.md` 5.3).
    #[test]
    fn episode_vectors() {
        let rows = EPISODE_VECTORS;
        assert_eq!(rows.len(), 20);
        for &(title, name, expected) in rows {
            let got = parse_episode_number(title, name, None);
            assert_eq!(got.to_bits(), expected.to_bits(), "{name:?}: got {got}, expected {expected}");
        }
    }

    #[test]
    fn episode_known_shortcut_and_tags() {
        assert_eq!(parse_episode_number("x", "Foo", Some(5.0)), 5.0);
        assert_eq!(parse_episode_number("x", "Foo", Some(-2.0)), -2.0);
        assert_eq!(parse_episode_number("x", "Foo", Some(-1.0)), -1.0);
        assert_eq!(parse_episode_number("x", "Foo", Some(-0.5)), -0.5);
        assert_eq!(parse_episode_number("x", "Foo 3", Some(-1.0)), 3.0);
    }

    #[test]
    fn episode_tag_loop_strips_leading_and_trailing_brackets_until_none_remain() {
        assert_eq!(
            parse_episode_number("x", "[A] (B) [C] 09 [D] (E)", None),
            9.0
        );
        assert_eq!(parse_episode_number("x", "(2020) [HD] Show 04", None), 4.0);
    }

    /// Season vectors (`p1-topic-item-number-recognition.md` 5.4).
    #[test]
    fn season_vectors() {
        let rows = SEASON_VECTORS;
        assert_eq!(rows.len(), 18);
        for &(title, name, expected) in rows {
            let got = parse_season_number(title, name, None);
            assert_eq!(got.to_bits(), expected.to_bits(), "{name:?}: got {got}, expected {expected}");
        }
        assert_eq!(
            parse_season_number(
                "Boku no Hero Academia",
                "Boku no Hero Academia (2017)",
                None
            ),
            -1.0
        );
    }

    #[test]
    fn season_known_shortcut_matches_chapters() {
        assert_eq!(parse_season_number("x", "Foo", Some(-2.0)), -2.0);
        assert_eq!(parse_season_number("x", "Foo", Some(-1.5)), -1.5);
        assert_eq!(parse_season_number("x", "Foo 2", Some(-0.5)), -0.5);
    }

    #[test]
    fn quirk_q5_capital_hi10_never_matches_lowercased_input() {
        // The season unwanted list has `Hi10`, the input is lowercased first, so the branch never fires.
        assert_eq!(parse_season_number("Show", "Show Hi10 s02", None), 2.0);
    }

    #[test]
    fn non_latin_titles_keep_ascii_word_boundaries() {
        // A kana directly before `s2` must not move the boundary: `\b` stays ASCII.
        assert_eq!(parse_chapter_number("X", "ありがとうs2 ch.9", None), 9.0);
        assert!(close(parse_chapter_number("X", "é5", None), 5.0));
    }
}
