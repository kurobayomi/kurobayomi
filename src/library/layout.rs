//! Grid geometry and badge text (T-37a-018 to T-37a-020, T-37a-024). Pure numbers and strings, so the card widgets
//! only place them.

use kuro_i18n::{MsgKey, tr};
use kuro_platform::kind::Kind;
use kuro_prefs::library::LibraryDisplayMode;

/// Cover height for a cover of `width` pixels: the 2:3 book ratio.
pub fn cover_height(width: i32) -> i32 {
    width * 3 / 2
}

/// Whether the category tabs and the toolbar's tab-aware title show: the setting, or any active search (E-19, E-44).
/// The tab strip and the toolbar read this one answer.
pub fn tabs_visible(category_tabs: bool, search_active: bool) -> bool {
    category_tabs || search_active
}

/// The hint under the header while the search text starts with `-` (E-04).
pub fn exclude_hint() -> String {
    tr(MsgKey::library_exclude_hint)
}

/// Whether the exclude hint shows for this search text.
pub fn shows_exclude_hint(text: &str) -> bool {
    text.starts_with('-')
}

/// Height of one list row when `entries` rows share `page_height` pixels: `height / n - 3 / n` in floating point,
/// at least 48 (E-23, an intended difference from the upstream integer division). A list size of 0 (Auto) gives the
/// fixed 76 px row (T-37a-021).
pub fn list_row_height(page_height: f64, entries: usize) -> f64 {
    if entries == 0 {
        return 76.0;
    }
    let n = entries as f64;
    (page_height / n - 3.0 / n).max(48.0)
}

/// The list size of the page: the portrait or landscape slider value for a window `width` pixels wide (the same
/// split the grids read), clamped to `0..=10`. 0 is Auto.
pub fn list_size(width: i32, portrait: i32, landscape: i32) -> usize {
    use kuro_domain::library::columns::{PORTRAIT_BREAKPOINT_PX, clamp_columns};
    clamp_columns(if width < PORTRAIT_BREAKPOINT_PX { portrait } else { landscape }) as usize
}

/// The tab label: the category name, with its count after it when counts are shown.
pub fn tab_label(name: &str, count: Option<usize>) -> String {
    match count {
        Some(n) => format!("{name} ({n})"),
        None => name.to_owned(),
    }
}

/// Edge of the continue button in pixels. The cover-only and comfortable grids use the large size (B-8.3, B-8.5).
pub fn continue_size(mode: LibraryDisplayMode) -> i32 {
    match mode {
        LibraryDisplayMode::CoverOnlyGrid | LibraryDisplayMode::ComfortableGrid => 32,
        LibraryDisplayMode::CompactGrid | LibraryDisplayMode::List => 28,
    }
}

/// Where a grid card puts its title: under the cover (comfortable), on it (compact, B-8.3, B-8.4), or nowhere (cover
/// only). List rows always show it beside the cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitlePlace {
    Below,
    OnCover,
    Hidden,
}

pub fn title_place(mode: LibraryDisplayMode) -> TitlePlace {
    match mode {
        LibraryDisplayMode::CompactGrid => TitlePlace::OnCover,
        LibraryDisplayMode::CoverOnlyGrid => TitlePlace::Hidden,
        LibraryDisplayMode::ComfortableGrid | LibraryDisplayMode::List => TitlePlace::Below,
    }
}

/// The spacing scale of the main window in px (docs/design/main-window.md).
pub const SPACE_XS: i32 = 4;
pub const SPACE_S: i32 = 8;
pub const SPACE_M: i32 = 12;
pub const SPACE_L: i32 = 16;
/// Where a library card's edge sits from the page edge: the grid's padding (12, `.kb-library-grid`), a cell's padding
/// (4, `.kb-library-grid > child`) and the card's own inset (`SPACE_S - 2`). The category tabs line up with it.
pub const CARD_EDGE: i32 = 12 + 4 + (SPACE_S - 2);

/// The text of the local badge.
pub fn local_badge() -> String {
    tr(MsgKey::local_source_badge)
}

/// The placeholder tile text: the first letter or digit of the first two words, upper case; "?" when there is none.
pub fn initials(title: &str) -> String {
    let text: String = title.split_whitespace().filter_map(|w| w.chars().find(|c| c.is_alphanumeric())).take(2).flat_map(char::to_uppercase).collect();
    if text.is_empty() { "?".to_owned() } else { text }
}

/// The unread badge: just the count (`6`); the tooltip says what it counts. None when hidden.
pub fn unread_badge(count: i64, _kind: Kind) -> Option<String> {
    count_badge(count).map(|n| n.to_string())
}

/// The download badge in words, None when hidden.
pub fn download_badge(count: i64) -> Option<String> {
    count_badge(count).map(|n| n.to_string())
}

/// What a card draws as its cover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoverKind {
    /// A `file://` image GTK draws itself: only before the app has an image service.
    File(String),
    /// An image the image service loads (only offered when the app installed one): a web cover, or a local title's
    /// `file://` cover, which then gets the service's decoded-image cache and its sized decode.
    Remote(String),
    /// The initials tile.
    Initials,
}

/// The cover of an entry with stored thumbnail `thumbnail`. `remote_loader` says whether remote images can load.
pub fn cover_kind(thumbnail: Option<&str>, remote_loader: bool) -> CoverKind {
    match thumbnail {
        Some(url) if url.starts_with("file://") && !remote_loader => CoverKind::File(url.to_owned()),
        Some(url) if url.starts_with("file://") => CoverKind::Remote(url.to_owned()),
        Some(url) if remote_loader && (url.starts_with("https://") || url.starts_with("http://") || url.starts_with(kuro_images::fetch::COVER_MARKER)) => CoverKind::Remote(url.to_owned()),
        _ => CoverKind::Initials,
    }
}

/// Everything one card shows, decided without widgets; `card.rs` only places it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardModel {
    pub title: String,
    pub title_place: TitlePlace,
    pub initials: String,
    pub unread: Option<String>,
    pub unread_tooltip: Option<&'static str>,
    pub downloads: Option<String>,
    /// The local badge; the language badge is dropped for local entries.
    pub local: bool,
    pub language: Option<String>,
}

pub fn card_model(item: &kuro_domain::library::page::LibraryItem, mode: LibraryDisplayMode, kind: Kind, season_parent: bool) -> CardModel {
    CardModel {
        title: item.row.title.clone(),
        title_place: title_place(mode),
        initials: initials(&item.row.title),
        unread: unread_badge(item.unread_count, kind),
        unread_tooltip: unread_badge_tooltip(season_parent),
        downloads: download_badge(item.download_count),
        local: item.is_local,
        language: if item.is_local { None } else { language_badge(&item.source_language) },
    }
}

/// The text of a count badge, or None when the badge is hidden (zero, or not computed because its switch is off).
pub fn count_badge(count: i64) -> Option<String> {
    (count > 0).then(|| count.to_string())
}

/// The tooltip of the unread badge: a season parent counts seasons, not episodes (T-37a-048). None keeps the default.
pub fn unread_badge_tooltip(season_parent: bool) -> Option<&'static str> {
    season_parent.then_some("unseen seasons")
}

/// The language badge: the source language in upper case, or None when empty.
pub fn language_badge(language: &str) -> Option<String> {
    (!language.is_empty()).then(|| language.to_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_37a_012_the_exclude_hint_shows_for_a_query_that_starts_with_a_hyphen_only() {
        assert!(shows_exclude_hint("-"), "E-04: a lone hyphen");
        assert!(shows_exclude_hint("-isekai"));
        assert!(!shows_exclude_hint("isekai"));
        assert!(!shows_exclude_hint("a-b"));
        assert!(!shows_exclude_hint(""));
    }

    #[test]
    fn a_season_parent_unread_badge_says_unseen_seasons() {
        // T-37a-048: the tooltip names seasons for season parents only.
        assert_eq!(unread_badge_tooltip(true), Some("unseen seasons"));
        assert_eq!(unread_badge_tooltip(false), None);
    }

    #[test]
    fn covers_keep_the_book_ratio_and_tabs_show_counts_on_request() {
        assert_eq!(cover_height(128), 192);
        assert_eq!(tab_label("Reading", Some(4)), "Reading (4)");
        assert_eq!(tab_label("Reading", None), "Reading");
    }

    #[test]
    fn the_title_goes_under_the_cover_on_it_or_nowhere_by_mode() {
        assert_eq!(title_place(LibraryDisplayMode::ComfortableGrid), TitlePlace::Below);
        assert_eq!(title_place(LibraryDisplayMode::CompactGrid), TitlePlace::OnCover);
        assert_eq!(title_place(LibraryDisplayMode::CoverOnlyGrid), TitlePlace::Hidden);
        assert_eq!(title_place(LibraryDisplayMode::List), TitlePlace::Below);
    }

    #[test]
    fn initials_take_the_first_letter_of_two_words() {
        assert_eq!(initials("Core Concepts"), "CC");
        assert_eq!(initials("berserk"), "B");
        assert_eq!(initials("the promised neverland"), "TP");
        assert_eq!(initials("  [Oshi] no ko "), "ON");
        assert_eq!(initials("86 eighty-six"), "8E");
        assert_eq!(initials("ëlan vital"), "ËV");
        assert_eq!(initials(""), "?");
        assert_eq!(initials("!!! ---"), "?");
    }

    #[test]
    fn badges_say_what_they_count() {
        assert_eq!(unread_badge(6, Kind::Anime).as_deref(), Some("6"));
        assert_eq!(unread_badge(6, Kind::Manga).as_deref(), Some("6"));
        assert_eq!(unread_badge(0, Kind::Anime), None);
        assert_eq!(unread_badge(-1, Kind::Manga), None, "not computed while the badge switch is off");
        assert_eq!(download_badge(3).as_deref(), Some("3"));
        assert_eq!(download_badge(0), None);
        assert_eq!(local_badge(), "Local");
    }

    #[test]
    fn file_covers_draw_remote_ones_need_the_loader_and_the_rest_is_initials() {
        assert_eq!(cover_kind(Some("file:///a/cover.jpg"), false), CoverKind::File("file:///a/cover.jpg".into()));
        assert_eq!(cover_kind(Some("file:///a/cover.jpg"), true), CoverKind::Remote("file:///a/cover.jpg".into()), "a local cover uses the image cache");
        assert_eq!(cover_kind(Some("https://x/c.jpg"), true), CoverKind::Remote("https://x/c.jpg".into()));
        assert_eq!(cover_kind(Some("kb-cover:/a/1"), true), CoverKind::Remote("kb-cover:/a/1".into()), "a poster asked of the source");
        assert_eq!(cover_kind(Some("https://x/c.jpg"), false), CoverKind::Initials);
        assert_eq!(cover_kind(Some(""), true), CoverKind::Initials);
        assert_eq!(cover_kind(None, true), CoverKind::Initials);
    }

    #[test]
    fn the_card_model_of_a_local_anime_and_a_source_manga() {
        use kuro_domain::library::page::LibraryItem;
        let row = kuro_domain::library::model::LibraryRow { title: "Core Concepts".into(), ..crate::library::fixtures::row(1, 0) };
        let local = LibraryItem { row: row.clone(), download_count: 0, unread_count: 6, is_local: true, source_language: "en".into() };
        let model = card_model(&local, LibraryDisplayMode::ComfortableGrid, Kind::Anime, false);
        assert_eq!(model.title, "Core Concepts");
        assert_eq!(model.title_place, TitlePlace::Below);
        assert_eq!(model.initials, "CC");
        assert_eq!(model.unread.as_deref(), Some("6"));
        assert_eq!(model.downloads, None);
        assert!(model.local);
        assert_eq!(model.language, None, "a local entry shows Local, not a language");

        let source = LibraryItem { row, download_count: 2, unread_count: 0, is_local: false, source_language: "ja".into() };
        let model = card_model(&source, LibraryDisplayMode::CompactGrid, Kind::Manga, false);
        assert_eq!(model.title_place, TitlePlace::OnCover);
        assert_eq!(model.unread, None);
        assert_eq!(model.downloads.as_deref(), Some("2"));
        assert!(!model.local);
        assert_eq!(model.language.as_deref(), Some("JA"));
        assert_eq!(card_model(&source, LibraryDisplayMode::CoverOnlyGrid, Kind::Anime, true).unread_tooltip, Some("unseen seasons"));
    }

    #[test]
    fn badges_hide_on_zero_and_uppercase_the_language() {
        assert_eq!(count_badge(0), None);
        assert_eq!(count_badge(-1), None);
        assert_eq!(count_badge(12).as_deref(), Some("12"));
        assert_eq!(language_badge(""), None);
        assert_eq!(language_badge("en").as_deref(), Some("EN"));
    }

    #[test]
    fn tabs_show_for_the_setting_or_a_search() {
        assert!(tabs_visible(true, false));
        assert!(tabs_visible(false, true));
        assert!(!tabs_visible(false, false));
    }

    #[test]
    fn list_rows_split_the_page_without_integer_division_and_never_shrink_below_48() {
        assert_eq!(list_row_height(600.0, 4), 600.0 / 4.0 - 3.0 / 4.0);
        assert_eq!(list_row_height(600.0, 2), 600.0 / 2.0 - 1.5);
        assert_eq!(list_row_height(100.0, 10), 48.0);
        assert_eq!(list_row_height(100.0, 0), 76.0, "list size 0 is the fixed 76 px row");
        assert_eq!(list_row_height(0.0, 0), 76.0);
    }
}
