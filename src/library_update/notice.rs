//! Notification ids and the content of the library update notifications (T-26a-034, T-26a-035, T-26a-036). Pure values:
//! the notifier posts them, this module decides what they say.
//!
//! Ids: Kotlin keys a per-entry notification by `manga.id.hashCode()` and shares the progress, error and warning ids
//! between the two kinds. Here an entry id is FNV-1a over the kind and the entry id, and each kind has its own pass
//! ids (progress, error, warning). Documented deviation: the shared ids of Kotlin are split per kind.

use kuro_i18n::{MsgKey, tr};
use kuro_platform::kind::Kind;

use crate::library_update::pass::EntryUpdate;
use crate::library_update::source::SourceItem;
use crate::library_update::text;

/// FNV-1a offset basis and prime for 32-bit hashes.
const FNV_OFFSET: u32 = 0x811c_9dc5;
const FNV_PRIME: u32 = 0x0100_0193;

/// Manga items at most this many: the "Download" action is offered (`CHAPTERS_PER_SOURCE_QUEUE_WARNING_THRESHOLD`).
pub const MANGA_DOWNLOAD_ACTION_LIMIT: usize = 15;
/// Anime items at most this many: the "Download" action is offered.
pub const ANIME_DOWNLOAD_ACTION_LIMIT: usize = 10;

#[cfg(test)]
pub const PROGRESS_CHANNEL: &str = "progress";
#[cfg(test)]
pub const ERROR_CHANNEL: &str = "error";
#[cfg(test)]
pub const WARNING_CHANNEL: &str = "warning";

/// The "N update(s) failed" notification text and its tap line.
pub fn error_text() -> String {
    tr(MsgKey::action_show_errors)
}

/// 32-bit FNV-1a.
pub fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(FNV_OFFSET, |hash, byte| (hash ^ u32::from(*byte)).wrapping_mul(FNV_PRIME))
}

fn kind_tag(kind: Kind) -> &'static str {
    match kind {
        Kind::Manga => "manga",
        Kind::Anime => "anime",
    }
}

/// The notification id of one entry: FNV-1a over the kind and the entry id, so a manga and an anime with the same id
/// get different ids (P-11).
pub fn entry_id(kind: Kind, entry_id: i64) -> u32 {
    let mut bytes = format!("{}:", kind_tag(kind)).into_bytes();
    bytes.extend_from_slice(&entry_id.to_be_bytes());
    fnv1a(&bytes)
}

#[cfg(test)]
/// The id of a pass notification (`channel` is one of the `*_CHANNEL` names) of one kind.
pub fn pass_id(channel: &str, kind: Kind) -> u32 {
    fnv1a(format!("{channel}:{}", kind_tag(kind)).as_bytes())
}

/// The actions a per-entry notification offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryAction {
    MarkRead,
    ViewItems,
    Download,
}

impl EntryAction {
    /// The button label of the action for `kind`.
    pub fn label(self, kind: Kind) -> String {
        match (self, kind) {
            (Self::MarkRead, Kind::Manga) => tr(MsgKey::action_mark_as_read),
            (Self::MarkRead, Kind::Anime) => tr(MsgKey::action_mark_as_seen),
            (Self::ViewItems, Kind::Manga) => tr(MsgKey::action_view_chapters),
            (Self::ViewItems, Kind::Anime) => tr(MsgKey::action_view_episodes),
            (Self::Download, _) => "Download".to_string(),
        }
    }
}

/// One per-entry notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryNotice {
    pub id: u32,
    pub title: String,
    pub text: String,
    pub actions: Vec<EntryAction>,
    /// Index into the update's items of the item the tap opens: the newest new one (P-8).
    pub tap_item: usize,
}

/// The newest item of a list: the smallest `source_order` (0 is the newest on the source), the first one on a tie.
pub fn newest_item(items: &[SourceItem]) -> Option<usize> {
    items.iter().enumerate().min_by_key(|(index, item)| (item.source_order, *index)).map(|(index, _)| index)
}

/// The per-entry notification of an update, or `None` when content is hidden (`hide_content` suppresses it entirely)
/// or the entry has no new items.
pub fn entry_notice(kind: Kind, update: &EntryUpdate, hide_content: bool) -> Option<EntryNotice> {
    if hide_content || update.items.is_empty() {
        return None;
    }
    let numbers: Vec<f64> = update.items.iter().map(|item| item.number).collect();
    let limit = match kind {
        Kind::Manga => MANGA_DOWNLOAD_ACTION_LIMIT,
        Kind::Anime => ANIME_DOWNLOAD_ACTION_LIMIT,
    };
    let mut actions = vec![EntryAction::MarkRead, EntryAction::ViewItems];
    if update.items.len() <= limit {
        actions.push(EntryAction::Download);
    }
    Some(EntryNotice {
        id: entry_id(kind, update.entry_id),
        title: update.title.clone(),
        text: text::new_items_description(kind, &numbers),
        actions,
        tap_item: newest_item(&update.items)?,
    })
}

/// The error notification of a pass: its title and text, and whether its tap opens the error log. Shown only when
/// `failed` is above zero. Without the log file there is no action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorNotice {
    pub title: String,
    pub text: String,
    pub opens_log: bool,
}

pub fn error_notice(failed: usize, log_exists: bool) -> Option<ErrorNotice> {
    (failed > 0).then(|| ErrorNotice { title: format!("{failed} update(s) failed"), text: error_text(), opens_log: log_exists })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(number: f64, source_order: i64) -> SourceItem {
        SourceItem { number, source_order, ..SourceItem::default() }
    }

    fn update(entry_id: i64, items: Vec<SourceItem>) -> EntryUpdate {
        EntryUpdate { entry_id, title: "Berserk".into(), items }
    }

    #[test]
    fn fnv1a_matches_the_published_vectors() {
        assert_eq!(fnv1a(b""), 0x811c_9dc5);
        assert_eq!(fnv1a(b"a"), 0xe40c_292c);
    }

    #[test]
    fn manga_and_anime_with_the_same_entry_id_get_different_ids() {
        // P-11: no collision between kinds in the fixture.
        assert_ne!(entry_id(Kind::Manga, 7), entry_id(Kind::Anime, 7));
        let mut seen = std::collections::HashSet::new();
        for id in 0..1_000 {
            assert!(seen.insert(entry_id(Kind::Manga, id)));
            assert!(seen.insert(entry_id(Kind::Anime, id)));
        }
    }

    #[test]
    fn pass_ids_are_distinct_per_kind_and_per_channel() {
        let ids = [
            pass_id(PROGRESS_CHANNEL, Kind::Manga),
            pass_id(PROGRESS_CHANNEL, Kind::Anime),
            pass_id(ERROR_CHANNEL, Kind::Manga),
            pass_id(ERROR_CHANNEL, Kind::Anime),
            pass_id(WARNING_CHANNEL, Kind::Manga),
            pass_id(WARNING_CHANNEL, Kind::Anime),
        ];
        let unique: std::collections::HashSet<u32> = ids.iter().copied().collect();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn download_is_offered_up_to_the_kind_limit() {
        let fifteen: Vec<SourceItem> = (1..=15).map(|n| item(f64::from(n), 0)).collect();
        let manga = entry_notice(Kind::Manga, &update(1, fifteen.clone()), false).expect("shown");
        assert!(manga.actions.contains(&EntryAction::Download));
        let sixteen: Vec<SourceItem> = (1..=16).map(|n| item(f64::from(n), 0)).collect();
        let too_many = entry_notice(Kind::Manga, &update(1, sixteen), false).expect("shown");
        assert!(!too_many.actions.contains(&EntryAction::Download));
        let eleven: Vec<SourceItem> = (1..=11).map(|n| item(f64::from(n), 0)).collect();
        let anime = entry_notice(Kind::Anime, &update(1, eleven), false).expect("shown");
        assert!(!anime.actions.contains(&EntryAction::Download), "anime keeps 10 as its limit");
        assert_eq!(anime.actions[0].label(Kind::Anime), "Mark as watched");
    }

    #[test]
    fn the_tap_opens_the_newest_new_item_not_the_oldest() {
        // P-8: source_order 0 is the newest; the first listed item is not necessarily it.
        let items = vec![item(5.0, 2), item(6.0, 0), item(4.0, 3)];
        let notice = entry_notice(Kind::Manga, &update(1, items), false).expect("shown");
        assert_eq!(notice.tap_item, 1);
    }

    #[test]
    fn an_update_without_items_has_no_entry_notification() {
        assert!(entry_notice(Kind::Manga, &update(1, Vec::new()), false).is_none());
    }

    #[test]
    fn error_notice_shows_only_for_failures_and_opens_the_log_only_when_it_exists() {
        // P-16
        assert!(error_notice(0, true).is_none());
        let with_log = error_notice(3, true).expect("shown");
        assert_eq!(with_log.title, "3 update(s) failed");
        assert_eq!(with_log.text, "Click to see details");
        assert!(with_log.opens_log);
        assert!(!error_notice(1, false).expect("shown").opens_log);
    }
}
