//! The keyboard and mouse matrix of the library (T4-UI-01).
//! Pure, so the view only applies the answer. Escape clears the selection first, then the search text.

use gtk::gdk::Key;

/// A key chord the library handles on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    SelectAll,
    Invert,
    /// Ctrl+Page_Down is `+1`, Ctrl+Page_Up is `-1`: move to that neighbouring category tab.
    Tab(i64),
    /// The Delete key opens the delete action for the selection (section 7 proposal). It never fires with an empty
    /// selection or while the search entry is typing, where it edits text.
    DeleteSelection,
    /// F5 starts an update of the current category, as the refresh button does (T-37a-035).
    RefreshCategory,
}

pub fn shortcut(key: Key, ctrl: bool, has_selection: bool, typing: bool) -> Option<Shortcut> {
    match key {
        Key::a | Key::A if ctrl => Some(Shortcut::SelectAll),
        Key::i | Key::I if ctrl => Some(Shortcut::Invert),
        Key::Page_Down if ctrl => Some(Shortcut::Tab(1)),
        Key::Page_Up if ctrl => Some(Shortcut::Tab(-1)),
        Key::Delete if has_selection && !typing => Some(Shortcut::DeleteSelection),
        Key::F5 => Some(Shortcut::RefreshCategory),
        _ => None,
    }
}

/// What a primary click on a cover does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Open,
    Toggle,
    Range,
}

/// Shift+click selects a range; Ctrl+click, or any click while something is selected, toggles; otherwise the entry
/// opens. A right click (or a touch long press) opens the cover's context menu and does not come through here.
pub fn press(shift: bool, ctrl: bool, selecting: bool) -> Press {
    if shift {
        Press::Range
    } else if ctrl || selecting {
        Press::Toggle
    } else {
        Press::Open
    }
}

/// What Escape does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Escape {
    ClearSelection,
    ClearSearch,
    /// Nothing to clear: the key goes on to the focused widget.
    Ignore,
}

pub fn escape(has_selection: bool, has_query: bool) -> Escape {
    if has_selection {
        Escape::ClearSelection
    } else if has_query {
        Escape::ClearSearch
    } else {
        Escape::Ignore
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_chords_map_to_select_all_invert_and_tab_steps() {
        assert_eq!(shortcut(Key::a, true, false, false), Some(Shortcut::SelectAll));
        assert_eq!(shortcut(Key::i, true, false, false), Some(Shortcut::Invert));
        assert_eq!(shortcut(Key::I, true, false, false), Some(Shortcut::Invert));
        assert_eq!(shortcut(Key::Page_Down, true, false, false), Some(Shortcut::Tab(1)));
        assert_eq!(shortcut(Key::Page_Up, true, false, false), Some(Shortcut::Tab(-1)));
        assert_eq!(shortcut(Key::i, false, false, false), None, "a bare I types into the search entry");
        assert_eq!(shortcut(Key::Page_Down, false, false, false), None);
        assert_eq!(shortcut(Key::F5, false, false, false), Some(Shortcut::RefreshCategory));
        assert_eq!(shortcut(Key::F5, false, true, true), Some(Shortcut::RefreshCategory), "F5 is not text input");
    }

    #[test]
    fn delete_needs_a_selection_and_no_typing() {
        assert_eq!(shortcut(Key::Delete, false, true, false), Some(Shortcut::DeleteSelection));
        assert_eq!(shortcut(Key::Delete, false, false, false), None);
        assert_eq!(shortcut(Key::Delete, false, true, true), None);
    }

    #[test]
    fn press_matrix_follows_section_7() {
        assert_eq!(press(false, false, false), Press::Open);
        assert_eq!(press(false, false, true), Press::Toggle);
        assert_eq!(press(false, true, false), Press::Toggle);
        assert_eq!(press(true, false, false), Press::Range);
    }

    #[test]
    fn selection_is_cleared_before_the_search() {
        assert_eq!(escape(true, true), Escape::ClearSelection);
        assert_eq!(escape(true, false), Escape::ClearSelection);
    }

    #[test]
    fn search_is_cleared_when_nothing_is_selected() {
        assert_eq!(escape(false, true), Escape::ClearSearch);
    }

    #[test]
    fn escape_with_nothing_to_clear_is_ignored() {
        assert_eq!(escape(false, false), Escape::Ignore);
    }
}
