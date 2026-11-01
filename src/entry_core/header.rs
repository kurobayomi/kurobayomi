//! The info header, GTK-free: the lines it shows, what a click searches, what a long press copies, and the cover size.
//! Upstream: `MangaInfoHeader.kt:336-414` (layouts), `:424-520` (title, author, artist, source rows).

use kuro_images::art::{ArtKind, ArtRef};
use kuro_platform::kind::Kind;

use crate::entry_core::text::{artist_row, display_author, display_title};

/// The small layout (the cover beside the text) draws the cover at this share of its column, between the two bounds: 100 px
/// on a phone, growing on a wider one-column window so the header is not a thumbnail on a large empty backdrop.
pub const SMALL_COVER_SHARE: f64 = 0.24;
pub const SMALL_COVER_MIN: i32 = 100;
pub const SMALL_COVER_MAX: i32 = 180;
/// The large layout draws the cover at this share of its column.
pub const LARGE_COVER_SHARE: f64 = 0.65;

/// A line of the header that can be clicked or copied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Title,
    Author,
    Artist,
    Source,
}

/// "Search for `query`". `global` searches every source, otherwise only the source of the manga. `genre` marks a genre
/// chip (`performGenreSearch`), which a Browse page with an HTTP source runs as a genre filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchRequest {
    pub query: String,
    pub global: bool,
    pub genre: bool,
}

/// The header lines, with placeholders for blank values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderModel {
    pub title: String,
    pub author: String,
    /// `None` when blank or equal to the author.
    pub artist: Option<String>,
    pub status_icon: &'static str,
    pub status: String,
    pub source: String,
    /// The source extension is missing: the warning icon is drawn in the error color.
    pub stub: bool,
    /// The genre chips under the description; empty hides the row (E-70).
    pub genres: Vec<GenreChip>,
    /// The cover to load. A newer `last_modified` in it makes the header load the cover again.
    pub cover: Option<ArtRef>,
    raw_title: String,
    raw_author: Option<String>,
}

impl HeaderModel {
    /// Equal for drawing. The cover's library flag only decides where its file is cached, so adding to or removing from
    /// the library must not rebuild the header and reload the cover.
    pub fn looks_like(&self, other: &Self) -> bool {
        let plain = |m: &Self| {
            let mut m = m.clone();
            if let Some(cover) = m.cover.as_mut() {
                cover.favorite = false;
            }
            m
        };
        plain(self) == plain(other)
    }

    /// The header of any entry kind: the caller supplies the status icon and text, because manga and anime have
    /// different status tables.
    pub fn from_parts(
        title: &str,
        author: Option<&str>,
        artist: Option<&str>,
        status: (&'static str, String),
        source: &str,
        stub: bool,
    ) -> Self {
        Self {
            title: display_title(title),
            author: display_author(author),
            artist: artist_row(author, artist),
            status_icon: status.0,
            status: status.1,
            source: source.to_owned(),
            stub,
            genres: Vec::new(),
            cover: None,
            raw_title: title.to_owned(),
            raw_author: author.map(str::to_owned),
        }
    }

    /// The same header with the cover of an entry: `(id, source, thumbnail url, favorite, cover_last_modified)`.
    pub fn with_cover(mut self, owner: Kind, id: i64, source: i64, url: Option<&str>, favorite: bool, last_modified: i64) -> Self {
        self.cover = Some(ArtRef { owner, entry_id: id, source_id: source, kind: ArtKind::Cover, url: url.map(str::to_owned), favorite, last_modified });
        self
    }

    /// The same header with the chips of a genre list (`genre_chips`).
    pub fn with_genres(mut self, genres: Option<&[String]>) -> Self {
        self.genres = genre_chips(genres);
        self
    }

    /// The raw text of a field when it is non-blank. Placeholders are never copied or searched.
    fn raw(&self, field: Field) -> Option<&str> {
        let text = match field {
            Field::Title => Some(self.raw_title.as_str()),
            Field::Author => self.raw_author.as_deref(),
            Field::Artist => self.artist.as_deref(),
            Field::Source => Some(self.source.as_str()),
        };
        text.filter(|t| !t.trim().is_empty())
    }

    /// A click: title, author and artist search every source, the source name searches its own source.
    pub fn search(&self, field: Field) -> Option<SearchRequest> {
        Some(SearchRequest {
            query: self.raw(field)?.to_owned(),
            global: field != Field::Source,
            genre: false,
        })
    }

    /// A long press or right click: the text to copy.
    pub fn copy(&self, field: Field) -> Option<String> {
        self.raw(field).map(str::to_owned)
    }
}

/// The cover size (width, height; 2:3) for a layout. `column_width` is the width of the column that holds it.
pub fn cover_size(large: bool, column_width: i32) -> (i32, i32) {
    let width = if large {
        (f64::from(column_width) * LARGE_COVER_SHARE).round() as i32
    } else {
        // in 20 px steps, so a window being resized rebuilds the header only a few times
        let share = (f64::from(column_width) * SMALL_COVER_SHARE).round() as i32;
        (share / 20 * 20).clamp(SMALL_COVER_MIN, SMALL_COVER_MAX).min(column_width)
    };
    (width, width * 3 / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(title: &str, author: Option<&str>, artist: Option<&str>, stub: bool) -> HeaderModel {
        HeaderModel::from_parts(title, author, artist, ("action-unavailable-symbolic", "Unknown".to_owned()), "Source A (EN)", stub)
    }

    #[test]
    fn blank_title_and_author_show_placeholders() {
        let m = model("", None, None, false);
        assert_eq!(m.title, "Unknown title");
        assert_eq!(m.author, "Unknown author");
        assert_eq!(m.artist, None);
    }

    #[test]
    fn artist_row_appears_only_when_it_differs_from_the_author() {
        assert_eq!(model("t", Some("A"), Some("A"), false).artist, None);
        assert_eq!(model("t", Some("A"), Some("B"), false).artist.as_deref(), Some("B"));
    }

    #[test]
    fn clicks_search_globally_except_the_source_which_stays_in_its_source() {
        let m = model("Berserk", Some("Miura"), Some("Studio"), false);
        let q = |f| m.search(f).map(|r| (r.query, r.global));
        assert_eq!(q(Field::Title), Some(("Berserk".into(), true)));
        assert_eq!(q(Field::Author), Some(("Miura".into(), true)));
        assert_eq!(q(Field::Artist), Some(("Studio".into(), true)));
        assert_eq!(q(Field::Source), Some(("Source A (EN)".into(), false)));
    }

    #[test]
    fn placeholders_are_neither_searched_nor_copied() {
        let m = model(" ", None, None, false);
        assert_eq!(m.search(Field::Title), None);
        assert_eq!(m.copy(Field::Title), None);
        assert_eq!(m.search(Field::Author), None);
        assert_eq!(m.copy(Field::Artist), None);
        assert_eq!(m.copy(Field::Source).as_deref(), Some("Source A (EN)"));
    }

    #[test]
    fn status_line_comes_from_the_manga_status() {
        let m = model("t", None, None, false);
        assert_eq!(m.status, "Unknown");
        assert_eq!(m.status_icon, "action-unavailable-symbolic");
    }

    #[test]
    fn stub_flag_reaches_the_header() {
        assert!(model("t", None, None, true).stub);
        assert!(!model("t", None, None, false).stub);
    }

    #[test]
    fn small_cover_grows_from_100_to_180_with_its_column_and_large_is_65_percent() {
        assert_eq!(cover_size(false, 400), (100, 150), "a phone keeps 100 px");
        assert_eq!(cover_size(false, 80), (80, 120), "never wider than its column");
        assert_eq!(cover_size(false, 600), (140, 210), "in 20 px steps");
        assert_eq!(cover_size(false, 1200), (180, 270), "at most 180 px");
        assert_eq!(cover_size(true, 400), (260, 390));
    }
}

/// One genre chip of the header (T-38a-016). A click offers Search, which runs the genre in the manga's own source
/// (`global` false, as `performGenreSearch` does), and Copy to clipboard, which copies the genre text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenreChip {
    pub label: String,
    pub search: SearchRequest,
    pub copy: String,
}

/// The chips of a genre list. Blank genres are dropped; an absent or empty list gives no chips, and the row is hidden
/// (E-70). Chips never wrap the window width on their own: the widget lays them out in a flow box (E-71).
pub fn genre_chips(genres: Option<&[String]>) -> Vec<GenreChip> {
    genres
        .unwrap_or_default()
        .iter()
        .map(|g| g.trim())
        .filter(|g| !g.is_empty())
        .map(|g| GenreChip {
            label: g.to_owned(),
            search: SearchRequest { query: g.to_owned(), global: false, genre: true },
            copy: g.to_owned(),
        })
        .collect()
}

#[cfg(test)]
mod genre_tests {
    use super::*;

    #[test]
    fn an_absent_or_empty_genre_list_gives_no_chips() {
        assert!(genre_chips(None).is_empty());
        assert!(genre_chips(Some(&[])).is_empty());
        assert!(genre_chips(Some(&["  ".to_owned()])).is_empty(), "blank genres are dropped");
    }

    #[test]
    fn the_header_carries_the_cover_of_its_entry_and_a_newer_stamp_makes_it_differ() {
        let plain = HeaderModel::from_parts("t", None, None, ("x", String::new()), "S", false);
        assert_eq!(plain.cover, None);
        let at = |stamp| plain.clone().with_cover(Kind::Anime, 4, 9, Some("u"), true, stamp);
        let art = at(5).cover.expect("a cover");
        assert_eq!((art.entry_id, art.source_id, art.last_modified, art.owner), (4, 9, 5, Kind::Anime));
        assert_ne!(at(5), at(6), "the header is rebuilt, or its cover reloaded, when the stamp changes");
    }

    #[test]
    fn the_header_carries_the_chips_of_its_genre_list() {
        let genres = ["Action".to_owned()];
        let model = HeaderModel::from_parts("t", None, None, ("x", String::new()), "S", false).with_genres(Some(&genres));
        assert_eq!(model.genres.len(), 1);
        assert!(HeaderModel::from_parts("t", None, None, ("x", String::new()), "S", false).genres.is_empty());
    }

    #[test]
    fn each_chip_searches_its_own_source_and_copies_its_text() {
        let genres = ["Action".to_owned(), " Drama ".to_owned()];
        let chips = genre_chips(Some(&genres));
        assert_eq!(chips.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["Action", "Drama"]);
        assert_eq!(chips[1].copy, "Drama");
        assert_eq!(chips[0].search, SearchRequest { query: "Action".into(), global: false, genre: true });
    }
}
