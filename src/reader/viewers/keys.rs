//! Keyboard tables for the pager and the webtoon (T-44a-013, T-44a-014, T-44a-015, B§4.2, B§4.4, E-15, E-25, E-26).
//!
//! Each key is claimed on press and on release (the release is swallowed, E-26). The action runs on the press edge, so
//! the reader root (which the adapter feeds press events only) and a focused viewer behave the same. Volume keys are
//! claimed only when the feature is on and the menu is hidden (E-25).

use super::pager::nav::PageCmd;
use crate::reader::page_zoom::ZoomKey;

/// The keys the viewers know. `Other` is never claimed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKey {
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Menu,
    VolumeUp,
    VolumeDown,
    /// Space turns the page forward, Shift+Space back (T-43b-058). The pager only; the webtoon scrolls.
    Space,
    ShiftSpace,
    Home,
    End,
    /// Plus zooms in, minus zooms out, 0 and Ctrl+0 go back to the fit (T-43b-058). Both viewers.
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Other,
}

/// One key event. `released` is true for the release edge. Actions run on the press edge only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: InputKey,
    pub ctrl: bool,
    pub released: bool,
}

/// The settings and state a key decision reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyContext {
    pub volume_enabled: bool,
    pub volume_inverted: bool,
    pub menu_visible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Page(PageCmd),
    ToggleMenu,
    /// `down` scrolls the webtoon down; otherwise up.
    Scroll { down: bool },
    Zoom(ZoomKey),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyOutcome {
    pub claimed: bool,
    /// Runs on the press edge only; the release edge is claimed and does nothing.
    pub action: Option<KeyAction>,
}

const UNCLAIMED: KeyOutcome = KeyOutcome { claimed: false, action: None };

fn claimed(action: Option<KeyAction>, ev: KeyEvent) -> KeyOutcome {
    KeyOutcome { claimed: true, action: if ev.released { None } else { action } }
}

/// A zoom key acts on the press, so a held key keeps zooming, and its release is claimed and ignored.
fn zoom(key: ZoomKey, ev: KeyEvent) -> KeyOutcome {
    KeyOutcome { claimed: true, action: (!ev.released).then_some(KeyAction::Zoom(key)) }
}

/// The zoom key an input is, if it is one.
fn zoom_key(key: InputKey) -> Option<ZoomKey> {
    match key {
        InputKey::ZoomIn => Some(ZoomKey::In),
        InputKey::ZoomOut => Some(ZoomKey::Out),
        InputKey::ZoomReset => Some(ZoomKey::Reset),
        _ => None,
    }
}

/// Whether a volume key is claimed at all (E-25).
fn volume_claimed(ctx: KeyContext) -> bool {
    ctx.volume_enabled && !ctx.menu_visible
}

/// The pager table (B§4.2). Volume keys page with `Prev` for down unless inverted. Ctrl with the horizontal
/// arrows moves by reading order; plain arrows are physical.
pub fn pager_key(ev: KeyEvent, ctx: KeyContext) -> KeyOutcome {
    if let Some(key) = zoom_key(ev.key) {
        return zoom(key, ev);
    }
    let action = match ev.key {
        InputKey::VolumeDown if volume_claimed(ctx) => Some(page(if ctx.volume_inverted { PageCmd::Prev } else { PageCmd::Next })),
        InputKey::VolumeUp if volume_claimed(ctx) => Some(page(if ctx.volume_inverted { PageCmd::Next } else { PageCmd::Prev })),
        InputKey::Right => Some(page(if ev.ctrl { PageCmd::Next } else { PageCmd::Right })),
        InputKey::Left => Some(page(if ev.ctrl { PageCmd::Prev } else { PageCmd::Left })),
        InputKey::Down | InputKey::PageDown => Some(page(PageCmd::Next)),
        InputKey::Up | InputKey::PageUp => Some(page(PageCmd::Prev)),
        InputKey::Space => Some(page(PageCmd::Next)),
        InputKey::ShiftSpace => Some(page(PageCmd::Prev)),
        InputKey::Home => Some(page(PageCmd::First)),
        InputKey::End => Some(page(PageCmd::Last)),
        InputKey::Menu => Some(KeyAction::ToggleMenu),
        _ => return UNCLAIMED,
    };
    claimed(action, ev)
}

/// The webtoon table (B§4.4). Ctrl is ignored. Volume keys scroll.
pub fn webtoon_key(ev: KeyEvent, ctx: KeyContext) -> KeyOutcome {
    if let Some(key) = zoom_key(ev.key) {
        return zoom(key, ev);
    }
    let scroll = |down| Some(KeyAction::Scroll { down });
    let action = match ev.key {
        // `volumeKeysInverted ? scrollUp() : scrollDown()` on the volume-down release.
        InputKey::VolumeDown if volume_claimed(ctx) => scroll(!ctx.volume_inverted),
        InputKey::VolumeUp if volume_claimed(ctx) => scroll(ctx.volume_inverted),
        InputKey::Left | InputKey::Up | InputKey::PageUp => scroll(false),
        InputKey::Right | InputKey::Down | InputKey::PageDown => scroll(true),
        InputKey::Space => scroll(true),
        InputKey::ShiftSpace => scroll(false),
        InputKey::Menu => Some(KeyAction::ToggleMenu),
        _ => return UNCLAIMED,
    };
    claimed(action, ev)
}

fn page(cmd: PageCmd) -> KeyAction {
    KeyAction::Page(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFF: KeyContext = KeyContext { volume_enabled: false, volume_inverted: false, menu_visible: false };

    fn press(key: InputKey) -> KeyEvent {
        KeyEvent { key, ctrl: false, released: false }
    }

    fn release(key: InputKey) -> KeyEvent {
        KeyEvent { key, ctrl: false, released: true }
    }

    #[test]
    fn t058_the_zoom_keys_act_on_press_in_both_viewers_and_their_release_is_swallowed() {
        for (key, zoom) in [(InputKey::ZoomIn, ZoomKey::In), (InputKey::ZoomOut, ZoomKey::Out), (InputKey::ZoomReset, ZoomKey::Reset)] {
            for table in [pager_key, webtoon_key] {
                let down = table(press(key), OFF);
                assert!(down.claimed);
                assert_eq!(down.action, Some(KeyAction::Zoom(zoom)));
                let up = table(release(key), OFF);
                assert!(up.claimed && up.action.is_none());
                let with_ctrl = table(KeyEvent { ctrl: true, ..press(key) }, OFF);
                assert_eq!(with_ctrl.action, Some(KeyAction::Zoom(zoom)), "Ctrl does not change a zoom key");
            }
        }
    }

    #[test]
    fn pager_keys_are_claimed_on_both_edges_and_act_on_the_press_edge() {
        let down = pager_key(press(InputKey::Right), OFF);
        assert_eq!(down, KeyOutcome { claimed: true, action: Some(KeyAction::Page(PageCmd::Right)) });
        let up = pager_key(release(InputKey::Right), OFF);
        assert_eq!(up, KeyOutcome { claimed: true, action: None }, "the release is claimed and does nothing");
    }

    #[test]
    fn page_keys_turn_pages_on_press_for_every_key_the_pager_owns() {
        for (key, cmd) in [
            (InputKey::Left, PageCmd::Left),
            (InputKey::Right, PageCmd::Right),
            (InputKey::Up, PageCmd::Prev),
            (InputKey::Down, PageCmd::Next),
            (InputKey::PageUp, PageCmd::Prev),
            (InputKey::PageDown, PageCmd::Next),
            (InputKey::Space, PageCmd::Next),
            (InputKey::ShiftSpace, PageCmd::Prev),
            (InputKey::Home, PageCmd::First),
            (InputKey::End, PageCmd::Last),
        ] {
            assert_eq!(pager_key(press(key), OFF).action, Some(KeyAction::Page(cmd)), "{key:?}");
        }
    }

    #[test]
    fn ctrl_arrows_use_the_reading_order_and_plain_arrows_are_physical() {
        let ctrl_right = KeyEvent { key: InputKey::Right, ctrl: true, released: false };
        assert_eq!(pager_key(ctrl_right, OFF).action, Some(KeyAction::Page(PageCmd::Next)));
        let ctrl_left = KeyEvent { key: InputKey::Left, ctrl: true, released: false };
        assert_eq!(pager_key(ctrl_left, OFF).action, Some(KeyAction::Page(PageCmd::Prev)));
        assert_eq!(pager_key(press(InputKey::Left), OFF).action, Some(KeyAction::Page(PageCmd::Left)));
    }

    #[test]
    fn menu_key_toggles_the_menu_on_press() {
        assert_eq!(pager_key(press(InputKey::Menu), OFF).action, Some(KeyAction::ToggleMenu));
        assert_eq!(webtoon_key(press(InputKey::Menu), OFF).action, Some(KeyAction::ToggleMenu));
    }

    #[test]
    fn webtoon_scroll_keys_are_consumed_on_both_edges_and_scroll_on_press() {
        for key in [InputKey::Left, InputKey::Up, InputKey::PageUp] {
            assert_eq!(webtoon_key(press(key), OFF).action, Some(KeyAction::Scroll { down: false }));
            assert!(webtoon_key(release(key), OFF).claimed);
        }
        for key in [InputKey::Right, InputKey::Down, InputKey::PageDown] {
            assert_eq!(webtoon_key(press(key), OFF).action, Some(KeyAction::Scroll { down: true }));
        }
    }

    #[test]
    fn unlisted_keys_are_never_claimed() {
        assert_eq!(pager_key(press(InputKey::Other), OFF), UNCLAIMED);
        assert_eq!(webtoon_key(press(InputKey::Other), OFF), UNCLAIMED);
    }

    #[test]
    fn space_and_home_end_turn_pages_in_the_pager() {
        assert_eq!(pager_key(press(InputKey::Space), OFF).action, Some(KeyAction::Page(PageCmd::Next)));
        assert_eq!(pager_key(press(InputKey::ShiftSpace), OFF).action, Some(KeyAction::Page(PageCmd::Prev)));
        assert_eq!(pager_key(press(InputKey::Home), OFF).action, Some(KeyAction::Page(PageCmd::First)));
        assert_eq!(pager_key(press(InputKey::End), OFF).action, Some(KeyAction::Page(PageCmd::Last)));
    }

    #[test]
    fn space_scrolls_the_webtoon_and_home_end_are_not_claimed_there() {
        assert_eq!(webtoon_key(press(InputKey::Space), OFF).action, Some(KeyAction::Scroll { down: true }));
        assert_eq!(webtoon_key(press(InputKey::ShiftSpace), OFF).action, Some(KeyAction::Scroll { down: false }));
        assert_eq!(webtoon_key(press(InputKey::Home), OFF), UNCLAIMED);
    }
}
