//! The entry kind of the tracking flows. It is the binding kind, the
//! shared `Kind`, so anime and manga share one type and one code path.

use kuro_platform::kind::Kind;

/// What tracking adds to the shared [`Kind`].
pub trait KindExt {
    /// Fluent id of the progress unit label: "Episodes" for anime, "Chapters" for manga (`U§4`).
    fn progress_label_key(self) -> &'static str;
}

impl KindExt for Kind {
    fn progress_label_key(self) -> &'static str {
        match self {
            Kind::Anime => "episodes",
            Kind::Manga => "chapters",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::EN_CATALOGUE;

    #[test]
    fn tr_27b_002_progress_label_follows_the_kind() {
        assert_eq!(Kind::Anime.progress_label_key(), "episodes");
        assert_eq!(Kind::Manga.progress_label_key(), "chapters");
    }

    #[test]
    fn tr_27b_002_progress_label_resolves_to_the_english_words() {
        assert!(EN_CATALOGUE.contains("\nepisodes = Episodes\n"));
        assert!(EN_CATALOGUE.contains("\nchapters = Chapters\n"));
    }
}
