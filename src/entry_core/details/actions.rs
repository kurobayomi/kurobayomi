//! Episode actions that are pure decisions: bulk bookmark and fillermark (T-39b-015), the flag write of a row menu item, and the open routing of an episode
//! (T-39b-017). Upstream: `AnimeScreenModel.kt` (`onMultiBookmarkClicked`, `onMultiFillermarkClicked`) and
//! `AnimeScreen.kt:136-144`. Behaviour 11.2 and 17.4 and 17.5. The caller writes the flags and clears the selection.

use kuro_data::item::Item;

use crate::entry_core::list_model::RowAction;

/// The per-episode flags a bulk action sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Bookmark,
    Fillermark,
}

fn flag(episode: &Item, mark: Mark) -> bool {
    match mark {
        Mark::Bookmark => episode.bookmark,
        Mark::Fillermark => episode.fillermark,
    }
}

/// The ids to write `on` for. Only episodes whose flag differs are written (behaviour 11.2); an episode already at
/// `on` is left out, so a bulk call never writes a row that does not change.
pub fn bulk_mark_targets(selected: &[Item], mark: Mark, on: bool) -> Vec<i64> {
    selected.iter().filter(|e| flag(e, mark) != on).map(|e| e.id).collect()
}

/// A tap opens the episode externally when the setting says so, and `alt` inverts it (behaviour 17.4).
pub fn tap_opens_external(always_external: bool, alt: bool) -> bool {
    always_external ^ alt
}

/// Continue watching and the season continue button use the setting as it is, with no inversion (17.2, 17.3).
pub fn continue_opens_external(always_external: bool) -> bool {
    always_external
}

/// The torrent server starts before the player opens, and only for a torrent source (17.5).
pub fn starts_torrent_server(is_torrent: bool) -> bool {
    is_torrent
}

/// The write of a row menu item on one episode, or `None` for the items that are not a flag write (Mark previous,
/// Download and Select, which the caller runs). Bookmark goes through `bulk_mark_targets`, as the bottom bar does, so
/// a menu item and a bulk action write the same way.
pub fn menu_effect(action: RowAction, episode: &Item) -> Option<MenuEffect> {
    match action {
        RowAction::ToggleRead => Some(MenuEffect::Seen { id: episode.id, seen: !episode.done }),
        RowAction::ToggleBookmark => {
            let on = !episode.bookmark;
            Some(MenuEffect::Mark { ids: bulk_mark_targets(std::slice::from_ref(episode), Mark::Bookmark, on), mark: Mark::Bookmark, on })
        }
        RowAction::MarkPrevious | RowAction::Download | RowAction::Select | RowAction::Reload => None,
    }
}

/// The write a menu item asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEffect {
    Seen { id: i64, seen: bool },
    Mark { ids: Vec<i64>, mark: Mark, on: bool },
}

/// How an episode is opened: the player, and whether the torrent server starts first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenPlan {
    pub external: bool,
    pub torrent_server: bool,
}

/// What a tap on a row does (17.4): external is `always_external XOR alt`. The torrent server starts only when the
/// torrent setting is on and the source is a torrent source (17.5).
pub fn tap_plan(always_external: bool, alt: bool, torrent_enabled: bool, torrent_source: bool) -> OpenPlan {
    OpenPlan { external: tap_opens_external(always_external, alt), torrent_server: starts_torrent_server(torrent_enabled && torrent_source) }
}

/// What continue watching and the season continue button do (17.2, 17.3): the setting as it is, with no inversion.
pub fn continue_plan(always_external: bool, torrent_enabled: bool, torrent_source: bool) -> OpenPlan {
    OpenPlan { external: continue_opens_external(always_external), torrent_server: starts_torrent_server(torrent_enabled && torrent_source) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_menu_item_toggles_the_mark_it_names_through_the_same_targets_as_the_bar() {
        let marked = ep(1, true, false);
        assert_eq!(menu_effect(RowAction::ToggleBookmark, &marked), Some(MenuEffect::Mark { ids: vec![1], mark: Mark::Bookmark, on: false }));
        assert_eq!(menu_effect(RowAction::ToggleRead, &marked), Some(MenuEffect::Seen { id: 1, seen: true }));
        for other in [RowAction::Download, RowAction::MarkPrevious, RowAction::Select] {
            assert_eq!(menu_effect(other, &marked), None, "{other:?} is not a flag write");
        }
    }

    #[test]
    fn a_tap_plan_opens_externally_with_alt_and_starts_the_server_only_for_torrents() {
        assert_eq!(tap_plan(false, false, true, false), OpenPlan { external: false, torrent_server: false });
        assert_eq!(tap_plan(false, true, false, true), OpenPlan { external: true, torrent_server: false }, "the setting is off");
        assert_eq!(tap_plan(false, false, true, true), OpenPlan { external: false, torrent_server: true });
    }

    fn ep(id: i64, bookmark: bool, fillermark: bool) -> Item {
        let mut e = Item::create(kuro_platform::kind::Kind::Anime);
        e.id = id;
        e.bookmark = bookmark;
        e.fillermark = fillermark;
        e
    }

    #[test]
    fn bulk_bookmark_writes_only_the_episodes_that_differ() {
        let list = [ep(1, false, false), ep(2, true, false), ep(3, false, true)];
        assert_eq!(bulk_mark_targets(&list, Mark::Bookmark, true), vec![1, 3]);
        assert_eq!(bulk_mark_targets(&list, Mark::Bookmark, false), vec![2]);
    }

    #[test]
    fn bulk_fillermark_uses_its_own_flag() {
        let list = [ep(1, true, false), ep(2, false, true)];
        assert_eq!(bulk_mark_targets(&list, Mark::Fillermark, true), vec![1]);
        assert_eq!(bulk_mark_targets(&list, Mark::Fillermark, false), vec![2]);
    }

    #[test]
    fn nothing_to_write_when_every_flag_already_matches() {
        assert!(bulk_mark_targets(&[ep(1, true, false)], Mark::Bookmark, true).is_empty());
        assert!(bulk_mark_targets(&[], Mark::Bookmark, true).is_empty());
    }

    #[test]
    fn tap_opens_internal_by_default_and_external_with_alt() {
        assert!(!tap_opens_external(false, false), "internal by default");
        assert!(tap_opens_external(false, true), "alt opens external");
    }

    #[test]
    fn torrent_server_only_for_torrent_sources() {
        assert!(starts_torrent_server(true));
        assert!(!starts_torrent_server(false));
    }
}
