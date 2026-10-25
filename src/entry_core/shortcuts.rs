//! Header shortcuts of the entry screens (T-38a-046). The chapter and episode list keys are `selection::key_command`. In the
//! header: F opens the filter sheet, R or Ctrl+R refreshes (T-38a-050), Ctrl+M
//! migrates and W opens the title in the browser for an HTTP source. Upstream: `MangaScreen.kt` key handling. Pure: the window asks
//! `header_key` and shows [`help`].

use kuro_i18n::{MsgKey, tr};
use super::selection::Mods;

/// What a header shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderKey {
    Filter,
    Refresh,
    Migrate,
    Browser,
}

/// The header action of a lowercase letter with these modifiers. W needs an HTTP source (a page to open).
pub fn header_key(letter: char, mods: Mods, webview_available: bool) -> Option<HeaderKey> {
    match (letter, mods.ctrl) {
        ('f', false) => Some(HeaderKey::Filter),
        ('r', _) => Some(HeaderKey::Refresh),
        ('m', true) => Some(HeaderKey::Migrate),
        ('w', false) if webview_available => Some(HeaderKey::Browser),
        _ => None,
    }
}

/// The in-app help list: each shortcut and what it does (the list is a labelled proposal).
pub fn help() -> Vec<(String, String)> {
    let key = |name: &str| name.to_owned();
    vec![
        (tr(MsgKey::ui_up_down), tr(MsgKey::ui_move_focus_in_the_chapter_list)),
        (key("Enter"), tr(MsgKey::ui_open_the_focused_chapter)),
        (key("Space"), tr(MsgKey::ui_toggle_the_focused_chapter)),
        (key("Shift+click"), tr(MsgKey::ui_select_a_range_from_the_anchor)),
        (key("Ctrl+click"), tr(MsgKey::ui_toggle_one_chapter)),
        (key("Ctrl+A"), tr(MsgKey::action_select_all)),
        (key("Ctrl+I"), tr(MsgKey::ui_invert_the_selection)),
        (key("Escape"), tr(MsgKey::ui_clear_the_selection)),
        (key("F"), tr(MsgKey::ui_open_the_filter_sheet)),
        (key("R"), tr(MsgKey::action_webview_refresh)),
        (key("Ctrl+M"), tr(MsgKey::action_migrate)),
        (key("W"), tr(MsgKey::action_open_in_browser)),
        (tr(MsgKey::ui_f1_or), tr(MsgKey::ui_show_this_help)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_letters_map_to_their_actions() {
        let none = Mods::default();
        let ctrl = Mods { ctrl: true, shift: false };
        assert_eq!(header_key('f', none, false), Some(HeaderKey::Filter));
        assert_eq!(header_key('r', none, false), Some(HeaderKey::Refresh));
        assert_eq!(header_key('r', ctrl, false), Some(HeaderKey::Refresh), "Ctrl+R refreshes (T-38a-050)");
        assert_eq!(header_key('m', ctrl, false), Some(HeaderKey::Migrate));
        assert_eq!(header_key('m', none, false), None, "M alone does nothing");
        assert_eq!(header_key('x', none, true), None);
    }

    #[test]
    fn w_opens_the_browser_only_when_the_source_has_a_page() {
        let none = Mods::default();
        assert_eq!(header_key('w', none, true), Some(HeaderKey::Browser));
        assert_eq!(header_key('w', none, false), None);
    }

    #[test]
    fn the_help_lists_the_chapter_and_header_keys() {
        let help = help();
        assert!(help.iter().any(|(keys, _)| keys == "Ctrl+I"));
        assert!(help.iter().any(|(keys, _)| keys == "Ctrl+M"));
        assert!(help.len() >= 13);
        assert!(help.iter().any(|(keys, _)| keys == "F1 or ?"));
    }
}
