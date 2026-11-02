//! Episode rows (T-39a-010, 014, 019): the title and the shared list row of one episode. The row is the same
//! `list_model::RowItem` a chapter has, so the list draws both kinds alike. Upstream: `AnimeEpisodeListItem.kt` and
//! `AnimeScreen.kt:1082-1099`.

use kuro_data::item::Item;
use kuro_i18n::{Arg, MsgKey, trf};
use kuro_platform::kind::Kind;

use super::format::{format_episode_number, watch_progress};
use crate::entry_core::list_model::{RowItem, subtitle};

/// The title: `Episode {n}` in episode-number mode, otherwise the episode name.
pub fn title(ep: &Item, display_number: bool) -> String {
    if display_number {
        trf(crate::entry_core::wording::numbered(ep.kind), &[("arg1", Arg::Str(&format_episode_number(ep.number)))])
    } else {
        ep.name.clone()
    }
}

/// The progress of a partly consumed item: "Page: N" of a chapter (`progress` is the last page read, from zero), the time
/// left of an episode.
fn progress(ep: &Item) -> Option<String> {
    match ep.kind {
        Kind::Manga => (!ep.done && ep.progress > 0).then(|| trf(MsgKey::chapter_progress, &[("arg1", Arg::Int(ep.progress + 1))])),
        // the row stores seconds; `watch_progress` counts milliseconds
        Kind::Anime => watch_progress(ep.done, ep.progress * 1000, ep.total_seconds() * 1000),
    }
}

/// The row of one episode. The second line is the date, the scanlator and the progress of a partly read or watched item;
/// the summary shows only when summaries are on and it is not blank. `last_second_seen` and `total_seconds` hold
/// milliseconds, as the player stores them (behaviour 3.4). The download state and the button are the page's to set.
pub fn row(ep: &Item, display_number: bool, show_summaries: bool, selected: bool) -> RowItem {
    RowItem {
        subtitle: subtitle(ep.date_upload, ep.scanlator.as_deref(), progress(ep)),
        summary: ep.summary.clone().filter(|t| show_summaries && !t.trim().is_empty()),
        read: ep.done,
        bookmark: ep.bookmark,
        filler: ep.fillermark,
        selected,
        ..RowItem::new(ep.id, title(ep, display_number))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partly_watched_episode_shows_the_time_left_from_its_stored_seconds() {
        let mut ep = Item::create(kuro_platform::kind::Kind::Anime);
        ep.progress = 690;
        ep.total_seconds = Some(1440);
        assert_eq!(progress(&ep).as_deref(), Some("12:30 left"));
        ep.progress = 0;
        assert_eq!(progress(&ep), None, "never started");
        let mut chapter = Item::create(kuro_platform::kind::Kind::Manga);
        chapter.progress = 4;
        assert_eq!(progress(&chapter).as_deref(), Some("Page: 5"), "a chapter shows its page, whatever its length");
    }

    #[test]
    fn title_follows_the_display_mode() {
        let mut ep = Item::create(kuro_platform::kind::Kind::Anime);
        ep.name = "The Start".into();
        ep.number = 12.5;
        assert_eq!(title(&ep, false), "The Start");
        assert_eq!(title(&ep, true), "Episode 12.5");
    }

    #[test]
    fn a_row_resolves_title_second_line_marks_and_summary() {
        let mut ep = Item::create(kuro_platform::kind::Kind::Anime);
        ep.id = 4;
        ep.name = "Pilot".into();
        ep.scanlator = Some("grp".into());
        ep.progress = 690;
        ep.total_seconds = Some(1_440);
        ep.summary = Some("plot".into());
        ep.bookmark = true;
        ep.fillermark = true;
        let shown = row(&ep, false, true, false);
        assert_eq!(shown.title, "Pilot");
        assert_eq!(shown.subtitle, "grp \u{b7} 12:30 left");
        assert_eq!(shown.summary.as_deref(), Some("plot"));
        assert!(shown.bookmark && shown.filler && !shown.read);
        let hidden = row(&ep, true, false, true);
        assert_eq!(hidden.title, "Episode -1");
        assert_eq!(hidden.summary, None, "summaries are off");
        assert!(hidden.selected);
    }

    /// The second line reads date, group, progress; a chapter shows the page it stopped at, and a read one no progress.
    #[test]
    fn a_chapter_row_reads_date_group_then_page_and_a_read_one_has_no_page() {
        let mut chapter = Item::create(kuro_platform::kind::Kind::Manga);
        chapter.scanlator = Some("Official".into());
        chapter.date_upload = 86_400_000;
        chapter.progress = 11;
        let shown = row(&chapter, true, false, false);
        assert!(!shown.read);
        assert!(shown.subtitle.starts_with("1970-01-0") && shown.subtitle.ends_with(" \u{b7} Official \u{b7} Page: 12"), "{}", shown.subtitle);
        chapter.done = true;
        let read = row(&chapter, true, false, false);
        assert!(read.read && read.subtitle.ends_with("Official"), "a read chapter shows no page: {}", read.subtitle);
    }

    #[test]
    fn a_seen_episode_is_read_with_no_progress_and_an_unknown_scanlator_is_hidden() {
        let mut ep = Item::create(kuro_platform::kind::Kind::Anime);
        ep.done = true;
        ep.progress = 5_000;
        ep.total_seconds = Some(9_000);
        ep.scanlator = Some("Unknown".into());
        ep.summary = Some("  ".into());
        let shown = row(&ep, false, true, false);
        assert!(shown.read);
        assert_eq!(shown.subtitle, "");
        assert_eq!(shown.summary, None, "a blank summary is no summary");
    }
}
