//! Item behaviour of both kinds (`Chapter.kt`, `Episode.kt`): recognised-number test, `copy_from` for
//! source refreshes, and the full `to_update` snapshot.

use kuro_data::item::{Item, ItemUpdate};
use kuro_platform::kind::Kind;
use kuro_source::model::normalize_scanlator;

/// `s` as `None` when it is empty or only whitespace (`ifBlank { null }`).
pub(crate) fn blank_to_none(s: Option<&str>) -> Option<String> {
    s.filter(|v| !v.trim().is_empty()).map(str::to_owned)
}

/// A source's chapter or episode number, parsed as `f32` and widened to `f64` through its shortest decimal text, so
/// `1.1f32` is stored as `1.1` and not `1.100000023841858`. This deliberately corrects Kotlin's `Float.toDouble()`
/// (EC-CH-07, T-15a-004, T-06a-034). The one widening rule of the app: `kuro_source::model::domain_chapter_number`.
pub fn widen_source_number(n: f32) -> f64 {
    kuro_source::model::domain_chapter_number(n)
}

/// The scanlator of a row: chapters are trimmed (`copyFromSChapter`), episodes only blanked (`copyFromSEpisode`). The one
/// per-kind scanlator rule; the two source adapters and the copy of a refresh call it.
pub fn scanlator_of(kind: Kind, raw: Option<&str>) -> Option<String> {
    match kind {
        Kind::Manga => normalize_scanlator(raw),
        Kind::Anime => blank_to_none(raw),
    }
}

/// Behaviour of an item row of either kind (`Chapter.kt`, `Episode.kt`).
pub trait ItemExt {
    /// `number >= 0`. `-1` (unknown) and `-2` (numberless sentinel) are not recognised.
    fn is_recognized_number(&self) -> bool;
    /// Copies the source's name, url, upload date, number and scanlator (per the kind's scanlator rule), and for an
    /// anime item also the fillermark, summary and preview url (blank becomes `None`). Every other field of `self` is kept.
    fn copy_from(&self, other: &Item) -> Item;
    /// `toChapterUpdate` / `toEpisodeUpdate`: every field filled.
    fn to_update(&self) -> ItemUpdate;
}

impl ItemExt for Item {
    fn is_recognized_number(&self) -> bool {
        self.number >= 0.0
    }

    fn copy_from(&self, other: &Item) -> Item {
        let anime = self.kind.is_anime();
        Item {
            name: other.name.clone(),
            url: other.url.clone(),
            date_upload: other.date_upload,
            number: other.number,
            scanlator: scanlator_of(self.kind, other.scanlator.as_deref()),
            fillermark: if anime { other.fillermark } else { self.fillermark },
            summary: if anime { blank_to_none(other.summary.as_deref()) } else { self.summary.clone() },
            preview_url: if anime { blank_to_none(other.preview_url.as_deref()) } else { self.preview_url.clone() },
            ..self.clone()
        }
    }

    fn to_update(&self) -> ItemUpdate {
        let anime = self.kind.is_anime();
        ItemUpdate {
            id: self.id,
            entry_id: Some(self.entry_id),
            done: Some(self.done),
            bookmark: Some(self.bookmark),
            fillermark: anime.then_some(self.fillermark),
            progress: Some(self.progress),
            total_seconds: self.total_seconds,
            date_fetch: Some(self.date_fetch),
            source_order: Some(self.source_order),
            url: Some(self.url.clone()),
            name: Some(self.name.clone()),
            date_upload: Some(self.date_upload),
            number: Some(self.number),
            scanlator: self.scanlator.clone(),
            summary: self.summary.clone(),
            preview_url: self.preview_url.clone(),
            version: Some(self.version),
            memo: self.memo.clone(),
            page_count: self.page_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_numbers_widen_through_their_decimal_form() {
        assert_eq!(widen_source_number(1.5), 1.5);
        assert_eq!(widen_source_number(1.1), 1.1);
        let parsed: f32 = "0.1".parse().unwrap();
        assert_eq!(widen_source_number(parsed), 0.1f64);
        assert_eq!(widen_source_number(-1.0), -1.0);
    }

    #[test]
    fn the_item_rules_serve_both_kinds() {
        let manga = Item { name: "mine".into(), done: true, progress: 9, scanlator: Some("old".into()), ..Item::create(Kind::Manga) };
        let source = Item { name: "new".into(), number: 4.5, scanlator: Some("  Team  ".into()), ..Item::create(Kind::Manga) };
        let out = manga.copy_from(&source);
        assert_eq!((out.name.as_str(), out.number, out.scanlator.as_deref(), out.done, out.progress), ("new", 4.5, Some("Team"), true, 9), "T-15a-004: trimmed");
        let blank = Item { scanlator: Some("   ".into()), ..source.clone() };
        assert_eq!(manga.copy_from(&blank).scanlator, None, "a blank scanlator becomes None");
        assert_eq!((out.id, out.entry_id, out.date_fetch, out.source_order, out.version), (manga.id, manga.entry_id, manga.date_fetch, manga.source_order, manga.version));
        let anime = Item { done: true, progress: 30, total_seconds: Some(1400), ..Item::create(Kind::Anime) };
        let from_source = Item { fillermark: true, scanlator: Some("  Sub ".into()), summary: Some(" ".into()), preview_url: Some("p".into()), ..Item::create(Kind::Anime) };
        let out = anime.copy_from(&from_source);
        assert_eq!((out.fillermark, out.scanlator.as_deref(), out.summary, out.preview_url.as_deref()), (true, Some("  Sub "), None, Some("p")));
        assert_eq!((out.done, out.progress, out.total_seconds), (true, 30, Some(1400)), "the user's state is kept");
        let blank = Item { fillermark: false, scanlator: Some("".into()), summary: Some("  ".into()), ..from_source.clone() };
        let out = anime.copy_from(&blank);
        assert_eq!((out.scanlator, out.summary, out.fillermark), (None, None, false));
        assert!(Item { number: 0.0, ..Item::create(Kind::Manga) }.is_recognized_number());
        assert!(!Item { number: -2.0, ..Item::create(Kind::Anime) }.is_recognized_number());
        let update = anime.to_update();
        assert_eq!((update.fillermark, update.total_seconds, update.memo.is_some()), (Some(false), Some(1400), true));
        assert_eq!(manga.to_update().fillermark, None, "a manga item has no fillermark column to write");
    }
}
