//! The item name sanitizer (`ChapterSanitizer.kt` and `EpisodeSanitizer.kt`, which are the same algorithm with the same
//! trim array): the name a source gives a chapter or episode, cleaned of the title and of the separators around it.
//! Applied where items are created from source data, not by SQL. Pure.
//! This is the only copy; `kuro_data::sanitize::sanitize_item_name` is the public entry point the sync code calls.

/// The characters stripped from both ends in the last step: the 30 entries of `EPISODE_TRIM_CHARS`
/// (space appears twice there, so 29 distinct characters).
pub const TRIM_CHARS: [char; 29] = [
    '\u{0020}', '\u{0009}', '\u{000A}', '\u{000B}', '\u{000C}', '\u{000D}', '\u{0085}', '\u{00A0}', '\u{1680}',
    '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}',
    '\u{2009}', '\u{200A}', '\u{2028}', '\u{2029}', '\u{202F}', '\u{205F}', '\u{3000}', '-', '_', ',', ':',
];

/// Kotlin `Char.isWhitespace()`. It differs from `char::is_whitespace`: U+001C to U+001F are in, U+0085 is out.
fn is_kotlin_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'..='\u{000D}'
            | '\u{001C}'..='\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
    )
}

/// `name.sanitize(title)`: trim, drop one leading `title` (case-sensitive), then trim [`TRIM_CHARS`].
pub fn sanitize(name: &str, title: &str) -> String {
    let trimmed = name.trim_matches(is_kotlin_whitespace);
    let without_title = trimmed.strip_prefix(title).unwrap_or(trimmed);
    without_title.trim_matches(|c| TRIM_CHARS.contains(&c)).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_prefix_and_the_separators_around_it_are_removed() {
        let cases: &[(&str, &str, &str)] = &[
            ("One Piece 1000", "One Piece", "1000"),
            ("One Piece - 1000", "One Piece", "1000"),
            ("Episode 5 -", "X", "Episode 5"),
            ("_-:Special,", "", "Special"),
            ("One Piece", "One Piece", ""),
            ("one piece 3", "One Piece", "one piece 3"),
            ("  One Piece 3", "One Piece", "3"),
            ("One Piece  One Piece 3", "One Piece", "One Piece 3"),
            ("\u{00A0}Ep 1\u{3000}", "X", "Ep 1"),
            ("Ep. 1.", "X", "Ep. 1."),
            ("\u{001F}One Piece 1", "One Piece", "1"),
        ];
        for (name, title, want) in cases {
            assert_eq!(sanitize(name, title), *want, "name {name:?} title {title:?}");
        }
    }

    #[test]
    fn kotlin_whitespace_includes_the_separator_controls_and_excludes_next_line() {
        assert!(is_kotlin_whitespace('\u{001F}'));
        assert!(is_kotlin_whitespace('\u{3000}'));
        assert!(!is_kotlin_whitespace('\u{0085}'));
        assert!(!is_kotlin_whitespace('.'));
    }
}
