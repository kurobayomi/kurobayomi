//! Reader colours (T-043a-052) and the reader stylesheet. The page background follows the reader theme preference.
//! Chrome colours come from the active GTK theme, not from these values (ui-parity section 8).

use gtk::prelude::*;

/// The reader background choices, by `pref_reader_theme_key`: 0 white, 1 black (default), 2 gray, 3 automatic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Background {
    White,
    Black,
    Gray,
}

impl Background {
    /// The CSS class that paints the background.
    pub fn class(self) -> &'static str {
        match self {
            Self::White => "reader-bg-white",
            Self::Black => "reader-bg-black",
            Self::Gray => "reader-bg-gray",
        }
    }
}

/// The background for a stored theme value. Automatic is gray when the system prefers dark, else white.
/// Unknown values fall back to black, the default.
pub fn background_for(theme: i32, system_dark: bool) -> Background {
    match theme {
        0 => Background::White,
        2 => Background::Gray,
        3 if system_dark => Background::Gray,
        3 => Background::White,
        _ => Background::Black,
    }
}

/// Whether the desktop asks for a dark theme. Read through GTK settings, since libadwaita is not used.
pub fn system_prefers_dark() -> bool {
    gtk::Settings::default().is_some_and(|s| s.is_gtk_application_prefer_dark_theme())
}

/// Whether the reader's text is light (`isDarkBackground`, B§1.4): themes 1 and 2 are dark, automatic is dark when
/// the system prefers dark, and anything else (white) is light text off.
pub fn dark_background(theme: i32, system_dark: bool) -> bool {
    match theme {
        1 | 2 => true,
        3 => system_dark,
        _ => false,
    }
}

/// The class of the text colour that goes with a background (`reader-text-*`).
pub fn text_class(theme: i32, system_dark: bool) -> &'static str {
    if dark_background(theme, system_dark) { "reader-text-light" } else { "reader-text-dark" }
}

/// Sets the background and text classes of a reader widget. Earlier classes are removed first. Pages, cards and
/// error states inside the widget take the same palette (T-44a-059).
pub fn apply_background(widget: &impl IsA<gtk::Widget>, theme: i32) {
    let background = background_for(theme, system_prefers_dark());
    for class in [Background::White, Background::Black, Background::Gray] {
        widget.remove_css_class(class.class());
    }
    widget.add_css_class(background.class());
    for class in ["reader-text-light", "reader-text-dark"] {
        widget.remove_css_class(class);
    }
    widget.add_css_class(text_class(theme, system_prefers_dark()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_theme_values_map_to_the_four_choices() {
        assert_eq!(background_for(0, false), Background::White);
        assert_eq!(background_for(1, false), Background::Black);
        assert_eq!(background_for(2, true), Background::Gray);
        assert_eq!(background_for(3, true), Background::Gray);
        assert_eq!(background_for(3, false), Background::White);
    }

    #[test]
    fn the_text_is_light_on_the_dark_backgrounds_and_dark_on_white() {
        assert!(!dark_background(0, true), "white takes dark text whatever the system says");
        assert!(dark_background(1, false));
        assert!(dark_background(2, false));
        assert!(dark_background(3, true));
        assert!(!dark_background(3, false));
        assert_eq!(text_class(0, false), "reader-text-dark");
        assert_eq!(text_class(1, false), "reader-text-light");
    }

    #[test]
    fn an_unknown_theme_value_is_black() {
        assert_eq!(background_for(9, false), Background::Black);
    }
}
