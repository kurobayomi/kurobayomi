//! Keyboard shortcut table (area 46 `p1-ui-parity.md` section 10.2, T-46a-043). Pure: the window's
//! key controller passes the keyval and modifiers and acts on the returned `Action`. Shortcuts are
//! off while a text entry has focus. Alt and Meta combinations are left to the desktop.
//!
//! The desktop player's keys (user, 2026-11-02): Space pause, Left/Right seek (Shift: long), Up/Down
//! volume, M mute, [ and ] speed, Backspace normal speed, F or F11 fullscreen, E episodes, Escape.
//! The phone's lock, controls toggle, sheets, picture-in-picture, aspect, frame step and screenshot
//! keys are not offered.

use crate::keys::Modifiers;

/// What a shortcut does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    TogglePause,
    /// Seeks the skip length, or 60 seconds when `long`.
    Seek {
        forward: bool,
        long: bool,
    },
    /// Device volume by this many steps (Up is +1).
    VolumeStep(i32),
    /// Sound off, or back to the level it had.
    ToggleMute,
    /// Speed by one 0.25 step, with no wrap for the keyboard.
    SpeedStep(i32),
    SpeedReset,
    OpenEpisodeList,
    Escape,
    ToggleFullscreen,
}

// X11 keysyms (`X11/keysymdef.h`).
const KS_BACKSPACE: u32 = 0xff08;
const KS_ESCAPE: u32 = 0xff1b;
const KS_LEFT: u32 = 0xff51;
const KS_UP: u32 = 0xff52;
const KS_RIGHT: u32 = 0xff53;
const KS_DOWN: u32 = 0xff54;
const KS_F11: u32 = 0xffc8;
const KS_SPACE: u32 = 0x0020;
const KS_BRACKET_LEFT: u32 = 0x005b;
const KS_BRACKET_RIGHT: u32 = 0x005d;

/// The action for a key press, or `None` when the key is not a shortcut here.
pub fn action_for(keyval: u32, mods: Modifiers, text_entry_focused: bool) -> Option<Action> {
    if text_entry_focused || mods.alt || mods.meta {
        return None;
    }
    if let Some(action) = arrow_or_symbol(keyval, mods) {
        return Some(action);
    }
    // Letters ignore case, so Shift+M and m are the same shortcut. Ctrl combinations are left to the window.
    if mods.ctrl {
        return None;
    }
    let letter = if (0x41..=0x5a).contains(&keyval) {
        keyval + 0x20
    } else {
        keyval
    };
    Some(match letter {
        KS_SPACE => Action::TogglePause,
        KS_BACKSPACE => Action::SpeedReset,
        KS_ESCAPE => Action::Escape,
        KS_F11 => Action::ToggleFullscreen,
        l if l == u32::from(b'm') => Action::ToggleMute,
        l if l == u32::from(b'e') => Action::OpenEpisodeList,
        l if l == u32::from(b'f') => Action::ToggleFullscreen,
        _ => return None,
    })
}

fn arrow_or_symbol(keyval: u32, mods: Modifiers) -> Option<Action> {
    let shift = mods.shift && !mods.ctrl;
    let plain = !mods.shift && !mods.ctrl;
    Some(match keyval {
        KS_LEFT | KS_RIGHT if mods.ctrl => return None,
        KS_LEFT | KS_RIGHT => Action::Seek {
            forward: keyval == KS_RIGHT,
            long: shift,
        },
        KS_UP if plain => Action::VolumeStep(1),
        KS_DOWN if plain => Action::VolumeStep(-1),
        KS_BRACKET_LEFT if plain => Action::SpeedStep(-1),
        KS_BRACKET_RIGHT if plain => Action::SpeedStep(1),
        _ => return None,
    })
}

/// Names an action can have in `player.toml`, `[shortcuts]` section (`"Shift+Left" = "seek-back-long"`).
const ACTION_NAMES: &[(&str, Action)] = &[
    ("pause", Action::TogglePause),
    (
        "seek-back",
        Action::Seek {
            forward: false,
            long: false,
        },
    ),
    (
        "seek-forward",
        Action::Seek {
            forward: true,
            long: false,
        },
    ),
    (
        "seek-back-long",
        Action::Seek {
            forward: false,
            long: true,
        },
    ),
    (
        "seek-forward-long",
        Action::Seek {
            forward: true,
            long: true,
        },
    ),
    ("volume-up", Action::VolumeStep(1)),
    ("volume-down", Action::VolumeStep(-1)),
    ("mute", Action::ToggleMute),
    ("speed-down", Action::SpeedStep(-1)),
    ("speed-up", Action::SpeedStep(1)),
    ("speed-reset", Action::SpeedReset),
    ("episodes", Action::OpenEpisodeList),
    ("escape", Action::Escape),
    ("fullscreen", Action::ToggleFullscreen),
];

/// A key with its Shift and Ctrl state; letters are held lower case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Chord {
    keyval: u32,
    shift: bool,
    ctrl: bool,
}

impl Chord {
    fn of(keyval: u32, mods: Modifiers) -> Self {
        let keyval = if (0x41..=0x5a).contains(&keyval) {
            keyval + 0x20
        } else {
            keyval
        };
        Self {
            keyval,
            shift: mods.shift,
            ctrl: mods.ctrl,
        }
    }

    /// `Ctrl+Shift+Left`, `x`, `bracketleft`. `None` for an unknown key name.
    fn parse(text: &str) -> Option<Self> {
        let mut mods = Modifiers::default();
        let mut parts = text.split('+').map(str::trim).peekable();
        let mut key = None;
        while let Some(part) = parts.next() {
            if parts.peek().is_some() {
                match part.to_ascii_lowercase().as_str() {
                    "shift" => mods.shift = true,
                    "ctrl" | "control" => mods.ctrl = true,
                    _ => return None,
                }
            } else {
                key = Some(part);
            }
        }
        let name = key?;
        let keyval = match name.to_ascii_lowercase().as_str() {
            "left" => KS_LEFT,
            "right" => KS_RIGHT,
            "up" => KS_UP,
            "down" => KS_DOWN,
            "space" => KS_SPACE,
            "backspace" => KS_BACKSPACE,
            "escape" => KS_ESCAPE,
            "f11" => KS_F11,
            "bracketleft" => KS_BRACKET_LEFT,
            "bracketright" => KS_BRACKET_RIGHT,
            _ => {
                let mut chars = name.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if c.is_ascii_graphic() => u32::from(c),
                    _ => return None,
                }
            }
        };
        Some(Self::of(keyval, mods))
    }
}

/// The shortcut table: the defaults of `action_for` with the overrides of `player.toml`. The
/// window asks this one table. An override of `"none"` unbinds the key.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShortcutMap {
    overrides: Vec<(Chord, Option<Action>)>,
}

impl ShortcutMap {
    /// Reads the `[shortcuts]` table of the player settings text. A key or action name that is
    /// not known is logged and skipped; text that is not TOML gives the defaults.
    pub fn from_toml(text: &str) -> Self {
        let mut map = Self::default();
        let doc = match text.parse::<toml_edit::DocumentMut>() {
            Ok(doc) => doc,
            Err(error) => {
                tracing::warn!(%error, "player settings are not valid TOML; using default shortcuts");
                return map;
            }
        };
        let Some(table) = doc.get("shortcuts").and_then(|t| t.as_table_like()) else {
            return map;
        };
        for (key, value) in table.iter() {
            let chord = Chord::parse(key);
            let action = value.as_str().and_then(|name| {
                if name == "none" {
                    Some(None)
                } else {
                    ACTION_NAMES
                        .iter()
                        .find(|(n, _)| *n == name)
                        .map(|(_, a)| Some(*a))
                }
            });
            match (chord, action) {
                (Some(chord), Some(action)) => map.set(chord, action),
                _ => tracing::warn!(key, "unknown shortcut entry in the player settings"),
            }
        }
        map
    }

    /// Reads the settings file. A missing or unreadable file gives the defaults.
    pub fn load(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path).map_or_else(|_| Self::default(), |t| Self::from_toml(&t))
    }

    fn set(&mut self, chord: Chord, action: Option<Action>) {
        self.overrides.retain(|(c, _)| *c != chord);
        self.overrides.push((chord, action));
    }

    /// The action for a key press: an override first, then the default table.
    pub fn action_for(
        &self,
        keyval: u32,
        mods: Modifiers,
        text_entry_focused: bool,
    ) -> Option<Action> {
        if text_entry_focused || mods.alt || mods.meta {
            return None;
        }
        let chord = Chord::of(keyval, mods);
        match self.overrides.iter().find(|(c, _)| *c == chord) {
            Some((_, action)) => *action,
            None => action_for(keyval, mods, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn m(shift: bool, ctrl: bool) -> Modifiers {
        Modifiers {
            shift,
            ctrl,
            alt: false,
            meta: false,
        }
    }

    #[test]
    fn arrows_seek_and_change_volume() {
        let none = m(false, false);
        assert_eq!(
            action_for(KS_LEFT, none, false),
            Some(Action::Seek {
                forward: false,
                long: false
            })
        );
        assert_eq!(
            action_for(KS_RIGHT, m(true, false), false),
            Some(Action::Seek {
                forward: true,
                long: true
            })
        );
        assert_eq!(action_for(KS_UP, none, false), Some(Action::VolumeStep(1)));
        assert_eq!(
            action_for(KS_DOWN, none, false),
            Some(Action::VolumeStep(-1))
        );
        assert_eq!(action_for(KS_DOWN, m(true, false), false), None);
    }

    #[test]
    fn letters_and_symbols_map_to_their_actions() {
        let none = m(false, false);
        assert_eq!(action_for(KS_SPACE, none, false), Some(Action::TogglePause));
        assert_eq!(
            action_for(u32::from(b'M'), m(true, false), false),
            Some(Action::ToggleMute)
        );
        assert_eq!(
            action_for(u32::from(b'e'), none, false),
            Some(Action::OpenEpisodeList)
        );
        assert_eq!(
            action_for(KS_F11, none, false),
            Some(Action::ToggleFullscreen)
        );
        assert_eq!(
            action_for(KS_BRACKET_RIGHT, none, false),
            Some(Action::SpeedStep(1))
        );
        assert_eq!(
            action_for(KS_BACKSPACE, none, false),
            Some(Action::SpeedReset)
        );
        assert_eq!(action_for(KS_ESCAPE, none, false), Some(Action::Escape));
    }

    #[test]
    fn the_phone_keys_and_ctrl_combinations_are_not_shortcuts() {
        for key in [b'l', b'c', b'a', b's', b'd', b'q', b'p', b'r', b',', b'.'] {
            assert_eq!(action_for(u32::from(key), m(false, false), false), None, "{}", key as char);
        }
        assert_eq!(action_for(u32::from(b's'), m(false, true), false), None);
    }

    #[test]
    fn text_focus_and_alt_disable_shortcuts() {
        assert_eq!(action_for(KS_SPACE, m(false, false), true), None);
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        assert_eq!(action_for(KS_SPACE, alt, false), None);
    }

    #[test]
    fn unbound_keys_pass_through() {
        assert_eq!(action_for(u32::from(b'z'), m(false, false), false), None);
    }

    #[test]
    fn settings_override_the_default_table() {
        let map = ShortcutMap::from_toml(
            "[shortcuts]\n\"Shift+Left\" = \"seek-forward\"\nx = \"mute\"\nm = \"none\"\n",
        );
        let left = map.action_for(KS_LEFT, m(true, false), false);
        assert_eq!(
            left,
            Some(Action::Seek {
                forward: true,
                long: false
            })
        );
        assert_eq!(map.action_for(u32::from(b'X'), m(true, false), false), None);
        assert_eq!(
            map.action_for(u32::from(b'x'), m(false, false), false),
            Some(Action::ToggleMute)
        );
        assert_eq!(
            map.action_for(u32::from(b'm'), m(false, false), false),
            None
        );
        // Unlisted keys keep the default, and Shift+Right is still the long seek.
        assert_eq!(
            map.action_for(KS_RIGHT, m(true, false), false),
            Some(Action::Seek {
                forward: true,
                long: true
            })
        );
        assert_eq!(map.action_for(KS_SPACE, m(false, false), true), None);
    }

    #[test]
    fn bad_entries_and_bad_toml_fall_back_to_the_defaults() {
        let map = ShortcutMap::from_toml("[shortcuts]\nNope = \"mute\"\nx = \"fly\"\n");
        assert_eq!(map, ShortcutMap::default());
        assert_eq!(ShortcutMap::from_toml("= = ="), ShortcutMap::default());
        assert_eq!(
            ShortcutMap::load(std::path::Path::new("/absent/player.toml")),
            ShortcutMap::default()
        );
    }

    #[test]
    fn the_file_is_read_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("player.toml");
        std::fs::write(&path, "[shortcuts]\nz = \"fullscreen\"\n").unwrap();
        assert_eq!(
            ShortcutMap::load(&path).action_for(u32::from(b'z'), m(false, false), false),
            Some(Action::ToggleFullscreen)
        );
    }
}
