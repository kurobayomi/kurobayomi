//! Info box rules of the anime entry (T-39a-023, T-39a-025, T-39a-063). Title, author and artist text reuse the manga
//! rules in `entry_core::text`, because they are identical. Upstream: `AnimeInfoHeader.kt:433-579` (text block),
//! `AnimeSourceExtensions.kt:18-31` (source name).

use kuro_data::entry::Entry;
use kuro_i18n::{MsgKey, tr};
use kuro_platform::kind::Kind;

use crate::entry_core::source_info::SourceInfo;
use crate::entry_core::header::HeaderModel;

/// The icon and label of a status value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusView {
    /// Adwaita symbolic icon name. The widget checks it with `gtk::IconTheme::has_icon`.
    pub icon: &'static str,
    pub text: String,
}

/// `SAnime` status values 0..7 (behaviour 6.3). Any other value shows "Unknown" with the block icon.
pub fn status(value: i64) -> StatusView {
    let (icon, key) = match value {
        1 => ("document-open-recent-symbolic", MsgKey::ongoing),
        2 => ("emblem-ok-symbolic", MsgKey::completed),
        3 => ("money-symbolic", MsgKey::licensed),
        4 => ("object-select-symbolic", MsgKey::publishing_finished),
        5 => ("window-close-symbolic", MsgKey::cancelled),
        6 => ("media-playback-pause-symbolic", MsgKey::on_hiatus),
        7 => ("x-office-calendar-symbolic", MsgKey::upcoming),
        _ => ("action-unavailable-symbolic", MsgKey::unknown),
    };
    StatusView { icon, text: tr(key) }
}

/// The info box lines of an entry: title, author and artist with their placeholders (`entry_core::text`), the status
/// table, and the source line with its stub flag. The widget is `entry_core::header_view`, for anime and manga alike.
pub fn header_model(entry: &Entry, source: &SourceInfo) -> HeaderModel {
    let shown = status(entry.status);
    let mut model = HeaderModel::from_parts(
        &entry.title,
        entry.author.as_deref(),
        entry.artist.as_deref(),
        (shown.icon, shown.text),
        &source.name,
        source.stub,
    )
    .with_genres(entry.genre.as_deref());
    // T-39a-029: an anime genre chip searches every source (global), where a manga chip searches the entry's source
    if entry.kind == Kind::Anime {
        for chip in &mut model.genres {
            chip.search.global = true;
        }
    }
    // the cover is the entry's own kind: its cache key and cover file are the ones the listing and the library use
    model.with_cover(
        entry.kind,
        entry.id,
        entry.source,
        entry.thumbnail_url.as_deref(),
        entry.favorite,
        entry.cover_last_modified,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_values_map_to_the_spec_labels() {
        assert_eq!(status(1).text, tr(MsgKey::ongoing));
        assert_eq!(status(2).text, tr(MsgKey::completed));
        assert_eq!(status(7).text, tr(MsgKey::upcoming));
        assert_eq!(status(7).icon, "x-office-calendar-symbolic");
    }

    #[test]
    fn unknown_and_out_of_range_status_fall_back_to_unknown() {
        for v in [0, 8, -1, 999] {
            assert_eq!(status(v).text, tr(MsgKey::unknown), "value {v}");
            assert_eq!(status(v).icon, "action-unavailable-symbolic", "value {v}");
        }
    }

    #[test]
    fn the_info_box_uses_the_placeholders_the_anime_status_and_the_stub_flag() {
        let entry = Entry { status: 7, ..Entry::create(Kind::Anime) };
        let model = header_model(&entry, &SourceInfo { name: "Src".into(), stub: true });
        assert_eq!(model.title, tr(MsgKey::unknown_title));
        assert_eq!(model.author, tr(MsgKey::unknown_author));
        assert_eq!(model.artist, None);
        assert_eq!((model.status_icon, model.status.as_str()), ("x-office-calendar-symbolic", tr(MsgKey::upcoming).as_str()));
        assert!(model.stub);
        let known = Entry { title: "Frieren".into(), author: Some("A".into()), artist: Some("A".into()), ..Entry::create(Kind::Anime) };
        let model = header_model(&known, &SourceInfo::default());
        assert_eq!((model.title.as_str(), model.author.as_str(), model.artist), ("Frieren", "A", None));
        assert!(!model.stub);
    }

    #[test]
    fn anime_genre_chips_search_every_source() {
        // T-39a-029: the chip search is global, unlike the manga chip
        let entry = Entry { genre: Some(vec!["Action".into(), "Drama".into()]), ..Entry::create(Kind::Anime) };
        let model = header_model(&entry, &SourceInfo { name: "Src".into(), stub: false });
        assert_eq!(model.genres.len(), 2);
        assert!(model.genres.iter().all(|c| c.search.global));
    }

    #[test]
    fn a_manga_keeps_its_own_kind_for_the_cover_and_its_source_for_the_chips() {
        // the listing caches a manga cover under the manga key: an anime key missed it and fetched the picture again
        let entry = Entry { genre: Some(vec!["Action".into()]), thumbnail_url: Some("https://x/c.jpg".into()), ..Entry::create(Kind::Manga) };
        let model = header_model(&entry, &SourceInfo { name: "Src".into(), stub: false });
        assert_eq!(model.cover.as_ref().map(|art| art.owner), Some(Kind::Manga));
        assert!(model.genres.iter().all(|c| !c.search.global));
    }
}
