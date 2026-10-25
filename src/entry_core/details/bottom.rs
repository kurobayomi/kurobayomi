//! The bottom action bar of the anime episode selection (T-39a-039). Upstream: `SharedAnimeBottomActionMenu`
//! (AnimeScreen.kt:964-1020), predicates as in behaviour 1.4. Pure: `bottom_actions` returns the rows to draw, in
//! the order the shared menu shows them. The bar is hidden when nothing is selected.

use kuro_i18n::MsgKey;
use kuro_platform::kind::Kind;

use crate::entry_core::wording;

use super::actions::Mark;

/// The fields of one selected episode the predicates read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selected {
    pub bookmark: bool,
    pub fillermark: bool,
    pub seen: bool,
    pub last_second_seen: i64,
    /// `downloadState == DOWNLOADED`.
    pub downloaded: bool,
}

/// What the page supplies besides the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuContext {
    pub kind: Kind,
    /// `onDownloadEpisode != null`: the page can start, cancel or delete downloads.
    pub has_download_handler: bool,
    pub always_external: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BottomAction {
    Bookmark,
    RemoveBookmark,
    Fillermark,
    RemoveFillermark,
    MarkViewed,
    MarkUnviewed,
    MarkPreviousViewed,
    Download,
    Delete,
    External,
    Internal,
}

/// What a row of the bar does when it is pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Sets or clears a mark on the selected episodes that differ (`actions::bulk_mark_targets`).
    Mark(Mark, bool),
    /// Marks the selected episodes seen or unseen.
    Seen(bool),
    /// Marks every episode before the selected one, in list order, as seen.
    PreviousSeen,
    Download,
    Delete,
    /// Opens the one selected episode in the other player: `external` is the player to use.
    Open { external: bool },
}

impl BottomAction {
    pub fn effect(self) -> Effect {
        match self {
            Self::Bookmark => Effect::Mark(Mark::Bookmark, true),
            Self::RemoveBookmark => Effect::Mark(Mark::Bookmark, false),
            Self::Fillermark => Effect::Mark(Mark::Fillermark, true),
            Self::RemoveFillermark => Effect::Mark(Mark::Fillermark, false),
            Self::MarkViewed => Effect::Seen(true),
            Self::MarkUnviewed => Effect::Seen(false),
            Self::MarkPreviousViewed => Effect::PreviousSeen,
            Self::Download => Effect::Download,
            Self::Delete => Effect::Delete,
            Self::External => Effect::Open { external: true },
            Self::Internal => Effect::Open { external: false },
        }
    }

    /// The label of the row.
    pub fn label(self, kind: Kind) -> MsgKey {
        match self {
            Self::Bookmark => wording::bookmark(kind, true),
            Self::RemoveBookmark => wording::bookmark(kind, false),
            Self::Fillermark => MsgKey::action_fillermark_episode,
            Self::RemoveFillermark => MsgKey::action_remove_fillermark_episode,
            Self::MarkViewed => wording::mark_done(kind, true),
            Self::MarkUnviewed => wording::mark_done(kind, false),
            Self::MarkPreviousViewed => wording::mark_previous(kind),
            Self::Download => MsgKey::action_download,
            Self::Delete => MsgKey::action_delete,
            Self::External => MsgKey::action_play_externally,
            Self::Internal => MsgKey::action_play_internally,
        }
    }

    /// The icon of the row.
    pub fn icon(self) -> &'static str {
        match self {
            Self::Bookmark => "starred-symbolic",
            Self::RemoveBookmark => "non-starred-symbolic",
            Self::Fillermark => "emblem-important-symbolic",
            Self::RemoveFillermark => "list-remove-symbolic",
            Self::MarkViewed => "object-select-symbolic",
            Self::MarkUnviewed => "edit-undo-symbolic",
            Self::MarkPreviousViewed => "go-up-symbolic",
            Self::Download => "folder-download-symbolic",
            Self::Delete => "user-trash-symbolic",
            Self::External | Self::Internal => "media-playback-start-symbolic",
        }
    }
}

/// The rows of the bar for `selected`. An empty selection shows no row.
pub fn bottom_actions(selected: &[Selected], ctx: MenuContext) -> Vec<BottomAction> {
    if selected.is_empty() {
        return Vec::new();
    }
    let any = |f: fn(&Selected) -> bool| selected.iter().any(f);
    let all = |f: fn(&Selected) -> bool| selected.iter().all(f);
    let single = selected.len() == 1;
    let mut rows = Vec::new();
    let mut push = |on: bool, action| {
        if on {
            rows.push(action);
        }
    };
    push(any(|e| !e.bookmark), BottomAction::Bookmark);
    push(all(|e| e.bookmark), BottomAction::RemoveBookmark);
    // a filler mark and a player choice exist for anime only
    let anime = ctx.kind.is_anime();
    push(anime && any(|e| !e.fillermark), BottomAction::Fillermark);
    push(anime && all(|e| e.fillermark), BottomAction::RemoveFillermark);
    push(any(|e| !e.seen), BottomAction::MarkViewed);
    push(any(|e| e.seen || e.last_second_seen > 0), BottomAction::MarkUnviewed);
    push(single, BottomAction::MarkPreviousViewed);
    push(ctx.has_download_handler && any(|e| !e.downloaded), BottomAction::Download);
    push(ctx.has_download_handler && any(|e| e.downloaded), BottomAction::Delete);
    push(anime && !ctx.always_external && single, BottomAction::External);
    push(anime && ctx.always_external && single, BottomAction::Internal);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> Selected {
        Selected::default()
    }

    fn ctx(has_download: bool, always_external: bool) -> MenuContext {
        MenuContext { kind: Kind::Anime, has_download_handler: has_download, always_external }
    }

    #[test]
    fn every_row_has_an_effect_a_label_and_an_icon() {
        assert_eq!(BottomAction::Bookmark.effect(), Effect::Mark(Mark::Bookmark, true));
        assert_eq!(BottomAction::RemoveFillermark.effect(), Effect::Mark(Mark::Fillermark, false));
        assert_eq!(BottomAction::MarkUnviewed.effect(), Effect::Seen(false));
        assert_eq!(BottomAction::External.effect(), Effect::Open { external: true });
        assert_eq!(BottomAction::Internal.effect(), Effect::Open { external: false });
        let all = [
            BottomAction::Bookmark,
            BottomAction::RemoveBookmark,
            BottomAction::Fillermark,
            BottomAction::RemoveFillermark,
            BottomAction::MarkViewed,
            BottomAction::MarkUnviewed,
            BottomAction::MarkPreviousViewed,
            BottomAction::Download,
            BottomAction::Delete,
            BottomAction::External,
            BottomAction::Internal,
        ];
        for action in all {
            assert!(!kuro_i18n::tr(action.label(Kind::Anime)).is_empty() && !action.icon().is_empty(), "{action:?}");
        }
        assert_eq!(kuro_i18n::tr(BottomAction::MarkPreviousViewed.label(Kind::Anime)), "Mark previous as watched");
    }

    #[test]
    fn empty_selection_hides_the_bar() {
        assert!(bottom_actions(&[], ctx(true, false)).is_empty());
    }

    #[test]
    fn one_plain_unseen_episode_offers_mark_and_open_rows() {
        let rows = bottom_actions(&[s()], ctx(false, false));
        assert_eq!(
            rows,
            vec![
                BottomAction::Bookmark,
                BottomAction::Fillermark,
                BottomAction::MarkViewed,
                BottomAction::MarkPreviousViewed,
                BottomAction::External,
            ]
        );
    }

    #[test]
    fn bookmark_rows_are_exclusive_per_state() {
        let all_marked = [Selected { bookmark: true, ..s() }, Selected { bookmark: true, ..s() }];
        let rows = bottom_actions(&all_marked, ctx(false, false));
        assert!(rows.contains(&BottomAction::RemoveBookmark));
        assert!(!rows.contains(&BottomAction::Bookmark));
        let mixed = [Selected { bookmark: true, ..s() }, s()];
        let rows = bottom_actions(&mixed, ctx(false, false));
        assert!(rows.contains(&BottomAction::Bookmark));
        assert!(!rows.contains(&BottomAction::RemoveBookmark));
    }

    #[test]
    fn fillermark_rows_follow_the_same_rule() {
        let all = [Selected { fillermark: true, ..s() }];
        let rows = bottom_actions(&all, ctx(false, false));
        assert!(rows.contains(&BottomAction::RemoveFillermark));
        assert!(!rows.contains(&BottomAction::Fillermark));
    }

    #[test]
    fn unviewed_when_seen_or_partly_watched() {
        let partly = [Selected { last_second_seen: 5, seen: false, ..s() }];
        assert!(bottom_actions(&partly, ctx(false, false)).contains(&BottomAction::MarkUnviewed));
        assert!(!bottom_actions(&[s()], ctx(false, false)).contains(&BottomAction::MarkUnviewed));
    }

    #[test]
    fn mark_previous_only_for_exactly_one_episode() {
        let two = [s(), s()];
        assert!(!bottom_actions(&two, ctx(false, false)).contains(&BottomAction::MarkPreviousViewed));
    }

    #[test]
    fn download_and_delete_need_a_handler_and_the_right_states() {
        let mixed = [Selected { downloaded: true, ..s() }, s()];
        let rows = bottom_actions(&mixed, ctx(true, false));
        assert!(rows.contains(&BottomAction::Download), "one not downloaded is enough");
        assert!(rows.contains(&BottomAction::Delete), "one downloaded is enough");
        let rows = bottom_actions(&mixed, ctx(false, false));
        assert!(!rows.contains(&BottomAction::Download));
        assert!(!rows.contains(&BottomAction::Delete));
        let done = [Selected { downloaded: true, ..s() }];
        assert!(!bottom_actions(&done, ctx(true, false)).contains(&BottomAction::Download));
    }

    #[test]
    fn external_shows_for_one_episode_only() {
        assert!(bottom_actions(&[s()], ctx(false, false)).contains(&BottomAction::External));
        assert!(!bottom_actions(&[s()], ctx(false, false)).contains(&BottomAction::Internal));
        assert!(!bottom_actions(&[s(), s()], ctx(false, false)).contains(&BottomAction::External));
    }
}
