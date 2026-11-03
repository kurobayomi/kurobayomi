//! Key map: GDK keyval and modifiers to mpv key string
//! (`AniyomiMPVView.onKey`). Pure: the GTK layer passes the keyval (`gdk::Key::into_glib`, which
//! equals the X11 keysym) and the modifier state; the result is the mpv command to send.

/// Modifier state at key time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// Super / Meta.
    pub meta: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPhase {
    Press,
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyInput {
    pub keyval: u32,
    pub modifiers: Modifiers,
    pub phase: KeyPhase,
    /// True for auto-repeat presses.
    pub repeat: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    /// mpv command arguments, for example `["keydown", "shift+a"]`.
    Send(Vec<String>),
    /// Consumed without sending (auto-repeat, mpv has its own repeat).
    Swallow,
    /// Not handled here; the event goes on to the next handler.
    Ignore,
}

// X11 keysyms (`X11/keysymdef.h`, `X11/XF86keysym.h`).
const KS_BACKSPACE: u32 = 0xff08;
const KS_TAB: u32 = 0xff09;
const KS_RETURN: u32 = 0xff0d;
const KS_ESCAPE: u32 = 0xff1b;
const KS_HOME: u32 = 0xff50;
const KS_LEFT: u32 = 0xff51;
const KS_UP: u32 = 0xff52;
const KS_RIGHT: u32 = 0xff53;
const KS_DOWN: u32 = 0xff54;
const KS_PAGE_UP: u32 = 0xff55;
const KS_PAGE_DOWN: u32 = 0xff56;
const KS_END: u32 = 0xff57;
const KS_INSERT: u32 = 0xff63;
const KS_KP_0: u32 = 0xffb0;
const KS_KP_9: u32 = 0xffb9;
const KS_KP_DECIMAL: u32 = 0xffae;
const KS_KP_ENTER: u32 = 0xff8d;
const KS_F1: u32 = 0xffbe;
const KS_F24: u32 = 0xffd5;
const KS_DELETE: u32 = 0xffff;
const KS_ISO_LEFT_TAB: u32 = 0xfe20;
const KS_MODIFIER_FIRST: u32 = 0xffe1; // Shift_L
const KS_MODIFIER_LAST: u32 = 0xffee; // Hyper_R
const KS_ISO_LEVEL3_SHIFT: u32 = 0xfe03;
const KS_DEAD_FIRST: u32 = 0xfe50;
const KS_DEAD_LAST: u32 = 0xfe93;
const KS_XF86_AUDIO_LOWER_VOLUME: u32 = 0x1008ff11;
const KS_XF86_AUDIO_MUTE: u32 = 0x1008ff12;
const KS_XF86_AUDIO_RAISE_VOLUME: u32 = 0x1008ff13;
const KS_XF86_AUDIO_PLAY: u32 = 0x1008ff14;
const KS_XF86_AUDIO_STOP: u32 = 0x1008ff15;
const KS_XF86_AUDIO_PREV: u32 = 0x1008ff16;
const KS_XF86_AUDIO_NEXT: u32 = 0x1008ff17;
const KS_XF86_AUDIO_PAUSE: u32 = 0x1008ff31;
const KS_XF86_AUDIO_REWIND: u32 = 0x1008ff3e;
const KS_XF86_AUDIO_FORWARD: u32 = 0x1008ff97;
/// `XF86XK_MediaPlayPause` is `_EVDEVK(0x0a4)` = `0x10081000 + 0x0a4`.
const KS_XF86_MEDIA_PLAY_PAUSE: u32 = 0x100810a4;

/// Named keys. Printable characters are handled separately.
fn named_key(keyval: u32) -> Option<String> {
    let name: &str = match keyval {
        KS_LEFT => "LEFT",
        KS_RIGHT => "RIGHT",
        KS_UP => "UP",
        KS_DOWN => "DOWN",
        0x0020 => "SPACE",
        KS_RETURN => "ENTER",
        KS_KP_ENTER => "KP_ENTER",
        KS_ESCAPE => "ESC",
        KS_TAB | KS_ISO_LEFT_TAB => "TAB",
        KS_BACKSPACE => "BS",
        KS_DELETE => "DEL",
        KS_INSERT => "INS",
        KS_HOME => "HOME",
        KS_END => "END",
        KS_PAGE_UP => "PGUP",
        KS_PAGE_DOWN => "PGDWN",
        KS_KP_DECIMAL => "KP_DEC",
        KS_XF86_AUDIO_PLAY => "PLAY",
        KS_XF86_AUDIO_PAUSE => "PAUSE",
        KS_XF86_MEDIA_PLAY_PAUSE => "PLAYPAUSE",
        KS_XF86_AUDIO_STOP => "STOP",
        KS_XF86_AUDIO_NEXT => "NEXT",
        KS_XF86_AUDIO_PREV => "PREV",
        KS_XF86_AUDIO_FORWARD => "FORWARD",
        KS_XF86_AUDIO_REWIND => "REWIND",
        KS_XF86_AUDIO_RAISE_VOLUME => "VOLUME_UP",
        KS_XF86_AUDIO_LOWER_VOLUME => "VOLUME_DOWN",
        KS_XF86_AUDIO_MUTE => "MUTE",
        KS_F1..=KS_F24 => return Some(format!("F{}", keyval - KS_F1 + 1)),
        KS_KP_0..=KS_KP_9 => return Some(format!("KP{}", keyval - KS_KP_0)),
        _ => return None,
    };
    Some(name.to_owned())
}

/// Character a printable keysym produces, or `None` for non-printable keysyms.
fn printable(keyval: u32) -> Option<char> {
    match keyval {
        // Keypad operators have no named entry; they produce their glyph (`KP_Multiply` etc.).
        0xffaa => Some('*'),
        0xffab => Some('+'),
        0xffad => Some('-'),
        0xffaf => Some('/'),
        0x20..=0x7e | 0xa0..=0xff => char::from_u32(keyval),
        // Unicode keysyms: 0x01000000 + code point.
        0x0100_0000..=0x0110_ffff => char::from_u32(keyval - 0x0100_0000),
        _ => None,
    }
}

fn is_modifier(keyval: u32) -> bool {
    (KS_MODIFIER_FIRST..=KS_MODIFIER_LAST).contains(&keyval) || keyval == KS_ISO_LEVEL3_SHIFT
}

/// mpv key name for a keyval: the named table first, then the printable character. `#` becomes
/// `SHARP` so it cannot start a comment in `input.conf`.
fn mpv_key_name(keyval: u32) -> Option<String> {
    if let Some(name) = named_key(keyval) {
        return Some(name);
    }
    let ch = printable(keyval)?;
    Some(if ch == '#' {
        "SHARP".to_owned()
    } else {
        ch.to_string()
    })
}

/// Maps one key event to an mpv command, following `AniyomiMPVView.onKey` order.
pub fn map_key(input: &KeyInput) -> KeyOutcome {
    if is_modifier(input.keyval) {
        return KeyOutcome::Ignore;
    }
    let is_dead = (KS_DEAD_FIRST..=KS_DEAD_LAST).contains(&input.keyval);
    let Some(mapped) = mpv_key_name(input.keyval).filter(|_| !is_dead) else {
        return KeyOutcome::Ignore;
    };
    if input.repeat {
        return KeyOutcome::Swallow;
    }
    let m = input.modifiers;
    let mut parts: Vec<&str> = Vec::new();
    for (on, name) in [
        (m.shift, "shift"),
        (m.ctrl, "ctrl"),
        (m.alt, "alt"),
        (m.meta, "meta"),
    ] {
        if on {
            parts.push(name);
        }
    }
    let key = match input.phase {
        KeyPhase::Press => "keydown",
        KeyPhase::Release => "keyup",
    };
    let mut combined = parts.join("+");
    if !combined.is_empty() {
        combined.push('+');
    }
    combined.push_str(&mapped);
    KeyOutcome::Send(vec![key.to_owned(), combined])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(keyval: u32) -> KeyInput {
        KeyInput {
            keyval,
            modifiers: Modifiers::default(),
            phase: KeyPhase::Press,
            repeat: false,
        }
    }

    fn sent(outcome: KeyOutcome) -> String {
        match outcome {
            KeyOutcome::Send(args) => args.last().cloned().unwrap_or_default(),
            other => panic!("expected Send, got {other:?}"),
        }
    }

    #[test]
    fn plain_letter_is_char() {
        assert_eq!(sent(map_key(&press(u32::from('a')))), "a");
    }

    #[test]
    fn arrow_keys_are_uppercase() {
        assert_eq!(sent(map_key(&press(KS_LEFT))), "LEFT");
        assert_eq!(sent(map_key(&press(KS_DOWN))), "DOWN");
    }

    #[test]
    fn modifiers_in_order() {
        let input = KeyInput {
            keyval: u32::from('a'),
            modifiers: Modifiers {
                shift: true,
                ctrl: true,
                alt: true,
                meta: true,
            },
            phase: KeyPhase::Press,
            repeat: false,
        };
        assert_eq!(
            map_key(&input),
            KeyOutcome::Send(vec!["keydown".into(), "shift+ctrl+alt+meta+a".into()])
        );
    }

    #[test]
    fn release_sends_keyup() {
        let input = KeyInput {
            phase: KeyPhase::Release,
            ..press(0x20)
        };
        assert_eq!(
            map_key(&input),
            KeyOutcome::Send(vec!["keyup".into(), "SPACE".into()])
        );
    }

    #[test]
    fn repeat_is_swallowed() {
        let input = KeyInput {
            repeat: true,
            ..press(KS_RIGHT)
        };
        assert_eq!(map_key(&input), KeyOutcome::Swallow);
    }

    #[test]
    fn dead_key_is_ignored() {
        assert_eq!(map_key(&press(0xfe50)), KeyOutcome::Ignore);
    }

    #[test]
    fn modifier_only_is_ignored() {
        assert_eq!(map_key(&press(0xffe1)), KeyOutcome::Ignore);
        assert_eq!(map_key(&press(KS_ISO_LEVEL3_SHIFT)), KeyOutcome::Ignore);
    }

    #[test]
    fn unknown_nonprintable_is_none() {
        assert_eq!(map_key(&press(0xff0c)), KeyOutcome::Ignore);
    }

    #[test]
    fn hash_becomes_sharp() {
        assert_eq!(sent(map_key(&press(u32::from('#')))), "SHARP");
    }

    #[test]
    fn media_keys_map() {
        assert_eq!(sent(map_key(&press(KS_XF86_AUDIO_PLAY))), "PLAY");
        assert_eq!(sent(map_key(&press(KS_XF86_MEDIA_PLAY_PAUSE))), "PLAYPAUSE");
        assert_eq!(sent(map_key(&press(KS_XF86_AUDIO_STOP))), "STOP");
        assert_eq!(sent(map_key(&press(KS_XF86_AUDIO_REWIND))), "REWIND");
        assert_eq!(
            sent(map_key(&press(KS_XF86_AUDIO_RAISE_VOLUME))),
            "VOLUME_UP"
        );
        assert_eq!(sent(map_key(&press(KS_XF86_AUDIO_MUTE))), "MUTE");
    }

    #[test]
    fn function_and_keypad_keys() {
        assert_eq!(sent(map_key(&press(KS_F1))), "F1");
        assert_eq!(sent(map_key(&press(KS_F24))), "F24");
        assert_eq!(sent(map_key(&press(KS_KP_0 + 7))), "KP7");
        assert_eq!(sent(map_key(&press(KS_KP_DECIMAL))), "KP_DEC");
    }

    #[test]
    fn kp_operators_fall_back_to_glyph() {
        assert_eq!(sent(map_key(&press(0xffab))), "+");
    }
}
