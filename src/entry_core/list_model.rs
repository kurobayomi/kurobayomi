//! The rows of the chapter and episode list as the widget sees them: one `RowItem` per chapter or episode (and per
//! missing-count marker), the shared second line, a diff between two lists that says which rows changed, and the focus
//! rule. Both kinds build the same `RowItem`, so the list draws them the same way. Nothing here imports GTK.
//! Upstream: `MangaScreenModel.kt` (`chapterListItems`), `MangaScreen.kt` (`ChapterItem` subtitle).

use kuro_downloads::DownloadState;
use kuro_i18n::MsgKey;
use kuro_platform::kind::Kind;


/// The side of the unread dot.
pub const DOT_SIZE: i32 = 8;

/// One row of the list, a chapter or an episode, built from the models with its download state and its selection.
#[derive(Clone, Debug, PartialEq)]
pub struct RowItem {
    pub id: i64,
    pub title: String,
    /// The dim second line: date, scanlator and progress (`subtitle`).
    pub subtitle: String,
    /// An episode summary under the second line, when the entry shows summaries.
    pub summary: Option<String>,
    /// The summary shows all its lines rather than three.
    pub summary_expanded: bool,
    /// Read or seen: the title takes the `dim-label` look and the unread dot is off.
    pub read: bool,
    pub bookmark: bool,
    /// An episode marked filler.
    pub filler: bool,
    pub selected: bool,
    pub download: DownloadState,
    pub progress: u8,
    /// The row has a download button. A local entry's items are always on disk and have none.
    pub indicator: bool,
    /// The button answers presses: not while any row is selected (E-27).
    pub indicator_enabled: bool,
    /// `Some(count)` on a missing-count marker row: no chapter or episode stands behind it, and its `id` is negative.
    pub missing: Option<i32>,
}

impl RowItem {
    /// A plain row: unread, unmarked, unselected, not downloaded and with no download button.
    pub fn new(id: i64, title: String) -> Self {
        Self {
            id,
            title,
            subtitle: String::new(),
            summary: None,
            summary_expanded: false,
            read: false,
            bookmark: false,
            filler: false,
            selected: false,
            download: DownloadState::NotDownloaded,
            progress: 0,
            indicator: false,
            indicator_enabled: false,
            missing: None,
        }
    }

    /// The marker of `count` missing items between two neighbours. `key` is the upstream `"{lower|null}-{higher}"` id
    /// and `text` the line it shows ("Missing 2 chapters").
    pub fn marker(key: String, count: i32, text: String) -> Self {
        // the marker has no item: a negative id keeps it apart from them, and `title` carries its key id
        Self { id: -marker_number(&key), subtitle: text, missing: Some(count), ..Self::new(0, key) }
    }

}

/// The upload date as `YYYY-MM-DD`; an unknown date (zero or negative) has no text.
fn date_text(date_upload_ms: i64) -> Option<String> {
    if date_upload_ms <= 0 {
        return None;
    }
    let date = chrono::DateTime::from_timestamp_millis(date_upload_ms)?;
    Some(date.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
}

/// The dim second line of a chapter or an episode: the upload date, the scanlator or language and the progress of a
/// partly read item, joined by a middle dot. A missing part leaves no dot behind, and an unknown scanlator is never shown.
pub fn subtitle(date_upload_ms: i64, scanlator: Option<&str>, progress: Option<String>) -> String {
    [date_text(date_upload_ms), crate::entry_core::text::shown_scanlator(scanlator).map(str::to_owned), progress]
        .into_iter()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

/// The higher chapter id of a marker id `"{lower|null}-{higher}"`. Each chapter is the higher side of at most one
/// marker, so the id is unique per marker.
fn marker_number(id: &str) -> i64 {
    id.rsplit('-').next().and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// What a model must do to go from one list to the next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Diff {
    Same,
    /// Same rows in the same order: these indices changed (ascending). One row is one `items_changed(i, 1, 1)`;
    /// several are one splice over their span (`changed_span`), so select all is one signal, not one per row.
    Rows(Vec<usize>),
    /// Rows were added, removed or reordered: replace the whole list with one splice.
    Reset,
}

/// What an item of a row's right-click menu does. Chapters and episodes have the same menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    ToggleRead,
    MarkPrevious,
    ToggleBookmark,
    /// Downloads the row, cancels its download while it is queued, or deletes it once it is on disk.
    Download,
    /// Selects the row: the menu's Select item, and what a long press on a row reports.
    Select,
    /// Forgets the row's cached files (chapter pages, subtitle files) and opens it, so they are fetched again from the
    /// site (for a site that replaced them behind the same address).
    Reload,
}

/// The right-click menu of `row` as `(label, section, action)`, in section order. Each label says what a press does
/// now (Mark as read or Mark as unread). A marker row has no menu, and Download shows only on a row with a download
/// button. `kind` words read as watched.
pub fn row_menu(row: &RowItem, kind: Kind) -> Vec<(MsgKey, u8, RowAction)> {
    if row.missing.is_some() {
        return Vec::new();
    }
    let read = match (kind, row.read) {
        (Kind::Manga, false) => MsgKey::action_mark_as_read,
        (Kind::Manga, true) => MsgKey::action_mark_as_unread,
        (Kind::Anime, false) => MsgKey::action_mark_as_seen,
        (Kind::Anime, true) => MsgKey::action_mark_as_unseen,
    };
    let previous = if kind.is_anime() { MsgKey::action_mark_previous_as_seen } else { MsgKey::action_mark_previous_as_read };
    let bookmark = match (kind, row.bookmark) {
        (Kind::Manga, false) => MsgKey::action_bookmark,
        (Kind::Manga, true) => MsgKey::action_remove_bookmark,
        (Kind::Anime, false) => MsgKey::action_bookmark_episode,
        (Kind::Anime, true) => MsgKey::action_remove_bookmark_episode,
    };
    let mut items = vec![(read, 0, RowAction::ToggleRead), (previous, 0, RowAction::MarkPrevious), (bookmark, 1, RowAction::ToggleBookmark)];
    if row.indicator {
        let download = match row.download {
            DownloadState::Downloaded => MsgKey::action_delete_download,
            DownloadState::Queue | DownloadState::Downloading => MsgKey::action_cancel,
            DownloadState::NotDownloaded | DownloadState::Error => MsgKey::action_download,
        };
        items.push((download, 1, RowAction::Download));
    }
    if !matches!(row.download, DownloadState::Downloaded) {
        items.push((MsgKey::action_reload, 2, RowAction::Reload));
    }
    items.push((MsgKey::action_select, 2, RowAction::Select));
    items
}

/// The span `(start, len)` that covers every index of a `Diff::Rows`. `None` for no index.
pub fn changed_span(indices: &[usize]) -> Option<(usize, usize)> {
    let (first, last) = (indices.iter().min()?, indices.iter().max()?);
    Some((*first, last - first + 1))
}

pub fn diff(old: &[RowItem], new: &[RowItem]) -> Diff {
    if old.len() != new.len() || old.iter().zip(new).any(|(a, b)| a.id != b.id) {
        return Diff::Reset;
    }
    let changed: Vec<usize> = (0..old.len()).filter(|&i| old[i] != new[i]).collect();
    if changed.is_empty() { Diff::Same } else { Diff::Rows(changed) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i64) -> RowItem {
        RowItem { indicator: true, indicator_enabled: true, ..RowItem::new(id, format!("c{id}")) }
    }

    #[test]
    fn the_second_line_joins_only_present_parts() {
        assert_eq!(subtitle(0, None, None), "");
        assert_eq!(subtitle(0, Some("grp"), Some("14:20 left".into())), "grp \u{b7} 14:20 left", "the progress comes last");
        assert_eq!(subtitle(-5, Some(" "), None), "", "an unknown date and a blank scanlator leave nothing");
        let full = subtitle(86_400_000, Some("grp"), Some("Page: 5".into()));
        assert!(full.starts_with("1970-01-0") && full.ends_with("grp \u{b7} Page: 5"), "date, scanlator, progress: {full}");
    }

    #[test]
    fn diff_reports_one_row_per_change_and_a_reset_for_structure() {
        let old = vec![item(1), item(2), item(3)];
        assert_eq!(diff(&old, &old), Diff::Same);
        let mut new = old.clone();
        new[1].selected = true;
        new[2].progress = 40;
        assert_eq!(diff(&old, &new), Diff::Rows(vec![1, 2]));
        assert_eq!(diff(&old, &old[..2]), Diff::Reset, "a removed row");
        assert_eq!(diff(&old, &[item(3), item(2), item(1)]), Diff::Reset, "a reorder");
    }

    #[test]
    fn the_row_menu_is_one_menu_for_both_kinds_with_labels_of_what_a_press_does() {
        let actions = |items: Vec<(MsgKey, u8, RowAction)>| items.into_iter().map(|(_, _, a)| a).collect::<Vec<_>>();
        let all = [RowAction::ToggleRead, RowAction::MarkPrevious, RowAction::ToggleBookmark, RowAction::Download, RowAction::Reload, RowAction::Select];
        assert_eq!(actions(row_menu(&item(1), Kind::Manga)), all);
        assert_eq!(actions(row_menu(&item(1), Kind::Anime)), all, "an episode has the same items");
        let sections: Vec<u8> = row_menu(&item(1), Kind::Manga).iter().map(|(_, s, _)| *s).collect();
        assert!(sections.windows(2).all(|w| w[0] <= w[1]), "already in section order, as the menu shows them");

        let chapter = row_menu(&item(1), Kind::Manga);
        assert_eq!((chapter[0].0, chapter[2].0, chapter[3].0), (MsgKey::action_mark_as_read, MsgKey::action_bookmark, MsgKey::action_download));
        let done = RowItem { read: true, bookmark: true, download: DownloadState::Downloaded, ..item(1) };
        let episode = row_menu(&done, Kind::Anime);
        assert_eq!((episode[0].0, episode[1].0, episode[2].0, episode[3].0), (MsgKey::action_mark_as_unseen, MsgKey::action_mark_previous_as_seen, MsgKey::action_remove_bookmark_episode, MsgKey::action_delete_download));
        assert!(!actions(episode).contains(&RowAction::Reload), "a downloaded row has no cached files to reload");
        let queued = RowItem { download: DownloadState::Queue, ..item(1) };
        assert_eq!(row_menu(&queued, Kind::Manga)[3].0, MsgKey::action_cancel);

        let local = RowItem { indicator: false, ..item(1) };
        assert!(!actions(row_menu(&local, Kind::Manga)).contains(&RowAction::Download), "no download button, no Download item");
        assert!(row_menu(&RowItem::marker("1-4".into(), 2, "Missing".into()), Kind::Manga).is_empty(), "a marker has no menu");
    }

    #[test]
    fn the_span_covers_the_first_to_the_last_changed_row() {
        assert_eq!(changed_span(&[]), None);
        assert_eq!(changed_span(&[4]), Some((4, 1)));
        assert_eq!(changed_span(&[2, 3, 9]), Some((2, 8)));
    }
}
