//! Reader UI state (T-043a-075, T-043a-076, and the menu, fullscreen and escape rules of T-043a-050). Pure: the
//! GTK side reads these values and applies them. Every change computes the new value first, then assigns it.

/// The one sheet or dialog the reader can show. `Loading` is the marker for an adjacent chapter load; it is
/// non-modal on Linux (the spinner is in the navigator), so it never hides another sheet (E-UI-5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialog {
    PageActions,
}

/// What Escape does: it dismisses an open sheet first, then leaves fullscreen, then closes the reader
/// (T-043a-050, T-043a-079).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscapeAction {
    DismissSheet,
    LeaveFullscreen,
    CloseReader,
}

/// The reader's visible state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiState {
    pub dialog: Option<Dialog>,
    pub menu_visible: bool,
    /// The software brightness overlay, kept in `-100..=100`.
    pub brightness: i32,
}

impl Default for UiState {
    fn default() -> Self {
        Self { dialog: None, menu_visible: true, brightness: 0 }
    }
}

impl UiState {
    /// Opens `dialog`, replacing whatever was open. The caller opens one sheet at a time.
    pub fn open(&mut self, dialog: Dialog) {
        self.dialog = Some(dialog);
    }

    #[cfg(test)]
    /// Closes `dialog` only when it is the one open. Closing a different sheet changes nothing.
    pub fn close(&mut self, dialog: Dialog) {
        if self.dialog == Some(dialog) {
            self.dialog = None;
        }
    }


    #[cfg(test)]
    /// A tap or a key toggles the chrome. Returns the new visibility.
    pub fn toggle_menu(&mut self) -> bool {
        self.menu_visible = !self.menu_visible;
        self.menu_visible
    }

    pub fn set_menu_visible(&mut self, visible: bool) {
        self.menu_visible = visible;
    }

}

/// Whether the window should be fullscreen for a menu state: hidden menu plus the fullscreen preference.
pub fn fullscreen_for_menu(menu_visible: bool, fullscreen_pref: bool) -> bool {
    fullscreen_pref && !menu_visible
}

/// F11 flips the window's fullscreen state.
pub fn toggle_fullscreen(current: bool) -> bool {
    !current
}

/// Escape dismisses an open sheet, else leaves fullscreen when the window is fullscreen, and closes the reader
/// otherwise. The sheet comes first wherever the focus is, so a sheet opened from the bottom bar is dismissed by
/// Escape and never closes the reader.
pub fn escape_action(sheet_open: bool, window_fullscreen: bool) -> EscapeAction {
    if sheet_open {
        EscapeAction::DismissSheet
    } else if window_fullscreen {
        EscapeAction::LeaveFullscreen
    } else {
        EscapeAction::CloseReader
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_the_open_sheet_clears_it() {
        let mut s = UiState::default();
        s.open(Dialog::PageActions);
        s.close(Dialog::PageActions);
        assert_eq!(s.dialog, None);
    }


    #[test]
    fn menu_toggles_and_reports_the_new_visibility() {
        let mut s = UiState::default();
        assert!(!s.toggle_menu());
        assert!(s.toggle_menu());
    }

    #[test]
    fn fullscreen_follows_the_menu_only_with_the_preference_on() {
        assert!(fullscreen_for_menu(false, true));
        assert!(!fullscreen_for_menu(true, true));
        assert!(!fullscreen_for_menu(false, false));
        assert!(toggle_fullscreen(false));
    }

    #[test]
    fn escape_dismisses_a_sheet_then_leaves_fullscreen_then_closes() {
        assert_eq!(escape_action(true, true), EscapeAction::DismissSheet);
        assert_eq!(escape_action(true, false), EscapeAction::DismissSheet);
        assert_eq!(escape_action(false, true), EscapeAction::LeaveFullscreen);
        assert_eq!(escape_action(false, false), EscapeAction::CloseReader);
    }

}
