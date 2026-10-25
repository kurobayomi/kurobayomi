//! Window-level key bindings (area 46, `PlayerActivity.kt` key handling,
//! T-45a-025). Pure: the GTK window maps its key event to a keyval and acts on the result.
//! Keys that are not listed here are forwarded to `keys::map_key`.

use crate::keys::Modifiers;
use crate::shortcuts::{Action, ShortcutMap};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowKey {
    /// A key of the shortcut table (`shortcuts`): Space, the arrows, letters and so on.
    Shortcut(Action),
    /// Media stop: closes the window.
    CloseWindow,
    /// Not bound here; the event goes to `keys::map_key` and is consumed.
    Forward,
}

// X11 keysyms (`X11/XF86keysym.h`).
const KS_XF86_AUDIO_STOP: u32 = 0x1008ff15;

/// The window answers from one table: the shortcut map, then the stop media key.
pub fn window_key(
    map: &ShortcutMap,
    keyval: u32,
    mods: Modifiers,
    text_entry_focused: bool,
) -> WindowKey {
    if let Some(action) = map.action_for(keyval, mods, text_entry_focused) {
        return WindowKey::Shortcut(action);
    }
    match keyval {
        KS_XF86_AUDIO_STOP => WindowKey::CloseWindow,
        _ => WindowKey::Forward,
    }
}

/// Window key bindings with per-key overrides (T-45b-057). The defaults are `window_key`. An
/// override replaces the default for one key; `unbind` makes a key fall through to `Forward`.
/// Conflicts with desktop shortcuts are not detected here: the window only sees keys while it has
/// focus, so global KDE shortcuts take precedence over these bindings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyBindings {
    overrides: Vec<(u32, WindowKey)>,
}

impl KeyBindings {
    /// Binds `keyval` to `key`, replacing any earlier binding for it.
    pub fn bind(&mut self, keyval: u32, key: WindowKey) {
        self.overrides.retain(|(k, _)| *k != keyval);
        self.overrides.push((keyval, key));
    }

    /// Removes a binding, so the key uses the default.
    pub fn reset(&mut self, keyval: u32) {
        self.overrides.retain(|(k, _)| *k != keyval);
    }

    pub fn lookup(
        &self,
        map: &ShortcutMap,
        keyval: u32,
        mods: Modifiers,
        text_entry_focused: bool,
    ) -> WindowKey {
        self.overrides
            .iter()
            .find(|(k, _)| *k == keyval)
            .map_or_else(
                || window_key(map, keyval, mods, text_entry_focused),
                |(_, key)| *key,
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KS_LEFT: u32 = 0xff51;
    const KS_UP: u32 = 0xff52;
    const KS_SPACE: u32 = 0x0020;

    const NONE: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: false,
        meta: false,
    };
    const SHIFT: Modifiers = Modifiers {
        shift: true,
        ..NONE
    };

    fn key(keyval: u32, mods: Modifiers) -> WindowKey {
        window_key(&ShortcutMap::default(), keyval, mods, false)
    }

    #[test]
    fn volume_arrows_and_space_come_from_the_shortcut_table() {
        assert_eq!(key(KS_UP, NONE), WindowKey::Shortcut(Action::VolumeStep(1)));
        assert_eq!(
            key(KS_SPACE, NONE),
            WindowKey::Shortcut(Action::TogglePause)
        );
        assert_eq!(
            key(u32::from('f'), NONE),
            WindowKey::Shortcut(Action::ToggleFullscreen)
        );
    }

    #[test]
    fn shift_left_is_the_long_seek_and_plain_left_the_short_one() {
        assert_eq!(
            key(KS_LEFT, SHIFT),
            WindowKey::Shortcut(Action::Seek {
                forward: false,
                long: true
            })
        );
        assert_eq!(
            key(KS_LEFT, NONE),
            WindowKey::Shortcut(Action::Seek {
                forward: false,
                long: false
            })
        );
    }

    #[test]
    fn media_stop_closes() {
        assert_eq!(key(KS_XF86_AUDIO_STOP, NONE), WindowKey::CloseWindow);
    }

    #[test]
    fn other_keys_are_forwarded_and_text_focus_forwards_shortcuts() {
        assert_eq!(key(u32::from('z'), NONE), WindowKey::Forward);
        assert_eq!(
            window_key(&ShortcutMap::default(), KS_SPACE, NONE, true),
            WindowKey::Forward
        );
    }

    #[test]
    fn the_settings_file_changes_what_the_window_answers() {
        let map = ShortcutMap::from_toml("[shortcuts]\n\"Shift+Left\" = \"seek-back\"\n");
        assert_eq!(
            window_key(&map, KS_LEFT, SHIFT, false),
            WindowKey::Shortcut(Action::Seek {
                forward: false,
                long: false
            })
        );
    }

    #[test]
    fn bindings_default_to_window_key_and_can_be_overridden() {
        let map = ShortcutMap::default();
        let look = |b: &KeyBindings| b.lookup(&map, KS_SPACE, NONE, false);
        let mut bindings = KeyBindings::default();
        assert_eq!(look(&bindings), WindowKey::Shortcut(Action::TogglePause));
        bindings.bind(KS_SPACE, WindowKey::CloseWindow);
        assert_eq!(look(&bindings), WindowKey::CloseWindow);
        bindings.bind(KS_SPACE, WindowKey::Forward);
        assert_eq!(look(&bindings), WindowKey::Forward);
        bindings.reset(KS_SPACE);
        assert_eq!(look(&bindings), WindowKey::Shortcut(Action::TogglePause));
    }

    #[test]
    fn a_new_binding_can_use_any_key() {
        let map = ShortcutMap::default();
        let mut bindings = KeyBindings::default();
        bindings.bind(u32::from('x'), WindowKey::CloseWindow);
        assert_eq!(
            bindings.lookup(&map, u32::from('x'), NONE, false),
            WindowKey::CloseWindow
        );
        assert_eq!(
            bindings.lookup(&map, u32::from('y'), NONE, false),
            WindowKey::Forward
        );
    }
}
