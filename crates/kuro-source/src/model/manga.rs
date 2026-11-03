//! `SManga`, its status codes and the genre parser (`SManga.kt`, `SMangaImpl.kt`, T-15a-002, T-15a-003).

use super::UpdateStrategy;

pub const UNKNOWN: i32 = 0;
pub const ONGOING: i32 = 1;
pub const COMPLETED: i32 = 2;
pub const LICENSED: i32 = 3;
pub const PUBLISHING_FINISHED: i32 = 4;
pub const CANCELLED: i32 = 5;
pub const ON_HIATUS: i32 = 6;

/// A manga as a source reports it. `status` stays the raw stored value; read it as [`MangaStatus`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SManga {
    pub url: String,
    pub title: String,
    pub artist: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    /// Genres joined with `", "`. [`SManga::genres`] parses it.
    pub genre: Option<String>,
    pub status: i32,
    pub thumbnail_url: Option<String>,
    pub update_strategy: UpdateStrategy,
    pub initialized: bool,
}

impl SManga {
    /// A manga with the source defaults: status `UNKNOWN`, always updated, not initialised, no optional fields.
    pub fn new(url: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            title: title.into(),
            artist: None,
            author: None,
            description: None,
            genre: None,
            status: UNKNOWN,
            thumbnail_url: None,
            update_strategy: UpdateStrategy::AlwaysUpdate,
            initialized: false,
        }
    }

    /// The genres in order of first occurrence. `None` for a missing or blank genre string (EC-GN-01..03).
    /// The split is on the two characters `", "` only, so `"Action,Drama"` is one genre (EC-GN-05).
    pub fn genres(&self) -> Option<Vec<String>> {
        parse_genres(self.genre.as_deref())
    }

    /// The status as a known variant. Any other stored value is `Unknown`; `status` keeps the raw value.
    pub fn status_kind(&self) -> MangaStatus {
        MangaStatus::from_raw(self.status)
    }
}

/// Splits a genre text the way `getGenres()` does (`SAnime.kt:49-52`, `SManga.kt`): on the two characters `", "`,
/// trimmed, blanks dropped, first occurrence kept. `None` for a missing or blank text. Shared by manga and anime.
pub fn parse_genres(raw: Option<&str>) -> Option<Vec<String>> {
    let raw = raw?;
    if raw.trim().is_empty() {
        return None;
    }
    let mut genres: Vec<String> = Vec::new();
    for part in raw.split(", ") {
        let genre = part.trim();
        if !genre.is_empty() && !genres.iter().any(|g| g == genre) {
            genres.push(genre.to_owned());
        }
    }
    Some(genres)
}

/// The publication status shown to the user. `from_raw` maps unknown codes to `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MangaStatus {
    Unknown,
    Ongoing,
    Completed,
    Licensed,
    PublishingFinished,
    Cancelled,
    OnHiatus,
}

impl MangaStatus {
    pub fn from_raw(raw: i32) -> Self {
        match raw {
            ONGOING => Self::Ongoing,
            COMPLETED => Self::Completed,
            LICENSED => Self::Licensed,
            PUBLISHING_FINISHED => Self::PublishingFinished,
            CANCELLED => Self::Cancelled,
            ON_HIATUS => Self::OnHiatus,
            _ => Self::Unknown,
        }
    }

    /// The code stored for this status. `Unknown` is `UNKNOWN`.
    pub fn raw(self) -> i32 {
        match self {
            Self::Unknown => UNKNOWN,
            Self::Ongoing => ONGOING,
            Self::Completed => COMPLETED,
            Self::Licensed => LICENSED,
            Self::PublishingFinished => PUBLISHING_FINISHED,
            Self::Cancelled => CANCELLED,
            Self::OnHiatus => ON_HIATUS,
        }
    }

    /// The message key of the status text (area 51, `MR.strings`).
    pub fn text_key(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Ongoing => "ongoing",
            Self::Completed => "completed",
            Self::Licensed => "licensed",
            Self::PublishingFinished => "publishing_finished",
            Self::Cancelled => "cancelled",
            Self::OnHiatus => "on_hiatus",
        }
    }

    /// The icon of the status badge (`MangaInfoHeader.kt`), by Material icon name. `Unknown`
    /// uses the block icon, which is also the icon of any other stored code.
    pub fn icon_name(self) -> &'static str {
        match self {
            Self::Unknown => "block",
            Self::Ongoing => "schedule",
            Self::Completed => "done_all",
            Self::Licensed => "attach_money",
            Self::PublishingFinished => "done",
            Self::Cancelled => "close",
            Self::OnHiatus => "pause",
        }
    }

    /// The English text for the key.
    pub fn english_text(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Ongoing => "Ongoing",
            Self::Completed => "Completed",
            Self::Licensed => "Licensed",
            Self::PublishingFinished => "Publishing finished",
            Self::Cancelled => "Cancelled",
            Self::OnHiatus => "On hiatus",
        }
    }
}

/// What the status badge of the entry screen shows (`MangaInfoHeader.kt`, T-15a-042): one icon and one line of text,
/// plus a warning icon when the entry comes from a stub source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusBadge {
    pub status: MangaStatus,
    pub icon_name: &'static str,
    pub text_key: &'static str,
    pub english_text: &'static str,
    pub stub_warning: bool,
}

impl StatusBadge {
    /// The badge for a stored status code. Any code outside 0 to 6 shows the unknown text and icon.
    pub fn for_raw(raw: i32, stub_source: bool) -> Self {
        let status = MangaStatus::from_raw(raw);
        Self {
            status,
            icon_name: status.icon_name(),
            text_key: status.text_key(),
            english_text: status.english_text(),
            stub_warning: stub_source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [MangaStatus; 7] = [
        MangaStatus::Unknown,
        MangaStatus::Ongoing,
        MangaStatus::Completed,
        MangaStatus::Licensed,
        MangaStatus::PublishingFinished,
        MangaStatus::Cancelled,
        MangaStatus::OnHiatus,
    ];

    fn with_genre(genre: Option<&str>) -> SManga {
        let mut m = SManga::new("/m", "M");
        m.genre = genre.map(str::to_owned);
        m
    }

    #[test]
    fn new_yields_the_source_defaults() {
        let m = SManga::new("/manga/1", "One");
        assert_eq!(m.status, 0);
        assert_eq!(m.update_strategy, UpdateStrategy::AlwaysUpdate);
        assert!(!m.initialized);
        assert!(m.artist.is_none() && m.author.is_none() && m.description.is_none());
        assert!(m.genre.is_none() && m.thumbnail_url.is_none());
        assert_eq!((m.url.as_str(), m.title.as_str()), ("/manga/1", "One"));
    }

    #[test]
    fn status_constants_are_zero_to_six() {
        let values = [UNKNOWN, ONGOING, COMPLETED, LICENSED, PUBLISHING_FINISHED, CANCELLED, ON_HIATUS];
        assert_eq!(values, [0, 1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn known_codes_map_to_their_variant_and_others_to_unknown_keeping_the_raw_value() {
        for (raw, kind) in ALL.iter().enumerate() {
            assert_eq!(MangaStatus::from_raw(raw as i32), *kind);
            assert_eq!(kind.raw(), raw as i32);
        }
        for stray in [7, -1, 999] {
            let mut m = SManga::new("/m", "M");
            m.status = stray;
            assert_eq!(m.status_kind(), MangaStatus::Unknown);
            assert_eq!(m.status, stray);
        }
    }

    #[test]
    fn every_status_has_a_text_key_and_an_english_text() {
        for kind in ALL {
            assert!(!kind.text_key().is_empty());
            assert!(!kind.english_text().is_empty());
        }
        assert_eq!(MangaStatus::OnHiatus.text_key(), "on_hiatus");
        assert_eq!(MangaStatus::PublishingFinished.english_text(), "Publishing finished");
    }

    #[test]
    fn genres_of_null_or_blank_are_none() {
        assert_eq!(with_genre(None).genres(), None);
        assert_eq!(with_genre(Some("")).genres(), None);
        assert_eq!(with_genre(Some("  ")).genres(), None);
    }

    #[test]
    fn genres_keep_the_first_occurrence_and_drop_duplicates() {
        assert_eq!(
            with_genre(Some("Action, Action, Drama")).genres(),
            Some(vec!["Action".to_owned(), "Drama".to_owned()])
        );
    }

    #[test]
    fn genres_split_only_on_comma_space() {
        assert_eq!(with_genre(Some("Action,Drama")).genres(), Some(vec!["Action,Drama".to_owned()]));
    }

    #[test]
    fn genres_are_trimmed_and_blank_parts_dropped() {
        assert_eq!(with_genre(Some(" , Action ,  ")).genres(), Some(vec!["Action".to_owned()]));
    }

    #[test]
    fn the_badge_has_the_icon_and_text_of_each_status() {
        let rows = [
            (0, "block", "unknown", "Unknown"),
            (1, "schedule", "ongoing", "Ongoing"),
            (2, "done_all", "completed", "Completed"),
            (3, "attach_money", "licensed", "Licensed"),
            (4, "done", "publishing_finished", "Publishing finished"),
            (5, "close", "cancelled", "Cancelled"),
            (6, "pause", "on_hiatus", "On hiatus"),
        ];
        for (raw, icon, key, text) in rows {
            let badge = StatusBadge::for_raw(raw, false);
            assert_eq!((badge.icon_name, badge.text_key, badge.english_text), (icon, key, text), "status {raw}");
            assert!(!badge.stub_warning);
        }
    }

    #[test]
    fn an_unknown_code_shows_the_unknown_badge_and_a_stub_source_adds_the_warning() {
        let badge = StatusBadge::for_raw(42, true);
        assert_eq!((badge.status, badge.icon_name, badge.text_key), (MangaStatus::Unknown, "block", "unknown"));
        assert!(badge.stub_warning);
    }
}
