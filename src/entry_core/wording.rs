//! The words of the entry page that follow the kind: a manga lists chapters and reads them, an anime lists episodes and
//! watches them. One place, so the page builds no text of its own for either kind.

use kuro_i18n::MsgKey;
use kuro_platform::kind::Kind;

/// The plural count over the list: "12 chapters", "1 episode".
pub fn count(kind: Kind) -> MsgKey {
    match kind {
        Kind::Manga => MsgKey::manga_num_chapters,
        Kind::Anime => MsgKey::anime_num_episodes,
    }
}

/// The title of a row in number mode: "Chapter 12.5", "Episode 12.5".
pub fn numbered(kind: Kind) -> MsgKey {
    match kind {
        Kind::Manga => MsgKey::display_mode_chapter,
        Kind::Anime => MsgKey::display_mode_episode,
    }
}

/// The marker row between two items whose numbers skip some: "Missing 2 chapters". The header over the list says
/// "Missing 2 items" for both kinds.
pub fn missing_marker(kind: Kind) -> MsgKey {
    match kind {
        Kind::Manga => MsgKey::missing_chapters,
        Kind::Anime => MsgKey::missing_items,
    }
}

/// The empty list text.
pub fn none_found(kind: Kind) -> MsgKey {
    match kind {
        Kind::Manga => MsgKey::no_chapters_error,
        Kind::Anime => MsgKey::no_episodes_error,
    }
}

/// The snackbar after "use as default" wrote the list options.
pub fn settings_updated(kind: Kind) -> MsgKey {
    match kind {
        Kind::Manga => MsgKey::chapter_settings_updated,
        Kind::Anime => MsgKey::episode_settings_updated,
    }
}

/// The label of the bookmark action on rows.
pub fn bookmark(kind: Kind, on: bool) -> MsgKey {
    match (kind, on) {
        (Kind::Manga, true) => MsgKey::action_bookmark,
        (Kind::Manga, false) => MsgKey::action_remove_bookmark,
        (Kind::Anime, true) => MsgKey::action_bookmark_episode,
        (Kind::Anime, false) => MsgKey::action_remove_bookmark_episode,
    }
}

/// The label of the mark-done actions: read for a manga, watched for an anime.
pub fn mark_done(kind: Kind, done: bool) -> MsgKey {
    match (kind, done) {
        (Kind::Manga, true) => MsgKey::action_mark_as_read,
        (Kind::Manga, false) => MsgKey::action_mark_as_unread,
        (Kind::Anime, true) => MsgKey::action_mark_as_seen,
        (Kind::Anime, false) => MsgKey::action_mark_as_unseen,
    }
}

/// The label of the mark-previous action.
pub fn mark_previous(kind: Kind) -> MsgKey {
    match kind {
        Kind::Manga => MsgKey::action_mark_previous_as_read,
        Kind::Anime => MsgKey::action_mark_previous_as_seen,
    }
}

/// The label of the list menu's jump to the first item still to read or watch.
pub fn jump_to_unread(kind: Kind) -> MsgKey {
    match kind {
        Kind::Manga => MsgKey::action_jump_to_unread,
        Kind::Anime => MsgKey::action_jump_to_unseen,
    }
}

#[cfg(test)]
mod tests {
    use kuro_i18n::tr;

    use super::*;

    #[test]
    fn each_kind_reads_in_its_own_words() {
        assert_ne!(tr(none_found(Kind::Manga)), tr(none_found(Kind::Anime)));
        assert_ne!(tr(mark_done(Kind::Manga, true)), tr(mark_done(Kind::Anime, true)));
        assert_eq!((tr(jump_to_unread(Kind::Manga)), tr(jump_to_unread(Kind::Anime))), ("Jump to first unread".to_owned(), "Jump to first unseen".to_owned()));
        assert!(tr(bookmark(Kind::Manga, true)).contains("chapter") && tr(bookmark(Kind::Anime, true)).contains("episode"));
    }
}
