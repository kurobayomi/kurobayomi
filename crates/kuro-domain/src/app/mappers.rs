//! The field rules of the entry and item mappers (`Manga.kt`, `Anime.kt`, `Chapter.kt`, `Episode.kt`; T-10a-005,
//! T-10a-006). The mappers themselves convert the source-api models `SManga`, `SAnime`, `SChapter` and `SEpisode`, which
//! areas 15 and 16 define; only the shared rules are here so far.

/// `getGenres()`: the genre text split on the two characters `", "`, each part trimmed, blanks dropped and duplicates
/// removed in first-seen order, exactly as `Anime.kt`/`Manga.kt` `getGenres` does (EC-19, EC-20). `"A, B, ,A"` gives
/// `["A", "B", ",A"]`: the split leaves `,A` as a part, and only blank parts are dropped.
pub fn split_genres(genre: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in genre.split(", ") {
        let part = part.trim();
        if !part.is_empty() && !out.iter().any(|g| g == part) {
            out.push(part.to_owned());
        }
    }
    out
}

/// `toSManga`: the genres joined with `", "`.
pub fn join_genres(genres: &[String]) -> String {
    genres.join(", ")
}

/// `ifBlank { null }`: a blank text becomes `None`, any other text is kept as it is.
pub fn blank_to_none(text: &str) -> Option<String> {
    (!text.trim().is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genres_split_trim_drop_blanks_and_keep_first_seen_order() {
        assert_eq!(split_genres("A, B, ,A"), ["A", "B", ",A"], "the Kotlin algorithm keeps \",A\"");
        assert_eq!(split_genres(""), Vec::<String>::new());
        assert_eq!(split_genres("  Action ,Drama"), ["Action ,Drama"], "only the two-character separator splits");
    }

    #[test]
    fn genres_join_with_the_two_character_separator() {
        assert_eq!(join_genres(&["A".into(), "B".into()]), "A, B");
        assert_eq!(join_genres(&[]), "");
        assert_eq!(split_genres(&join_genres(&["X".into(), "Y".into()])), ["X", "Y"]);
    }

    #[test]
    fn blank_text_becomes_none_and_other_text_is_kept() {
        assert_eq!(blank_to_none("   "), None);
        assert_eq!(blank_to_none(""), None);
        assert_eq!(blank_to_none(" Scan "), Some(" Scan ".to_owned()), "kept as written; the caller trims if it must");
    }
}
