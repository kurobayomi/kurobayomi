//! The colours of the whole app, for the choices other than the desktop's own (`AppColours`, the Colours row of the
//! Appearance page). A choice is two things, an accent and a tint for the surfaces; the full set of libadwaita's colour
//! names is worked out from them for the light and for the dark style, so every choice works in both and a new one is
//! one line in [`scheme`]. Pure: `apply` loads the sheet.
//!
//! libadwaita lets an app redeclare its named colours (`doc/named-colors.md` of 1.5: "Applications can override any of
//! these colors by re-declaring them"). The names the app's own sheet uses (`theme_bg_color` and the like) are its
//! aliases of those; they are redeclared too, so nothing depends on how an alias follows.
//!
//! Text reads on every surface by construction, and a test checks it for every choice in both styles: the surfaces are
//! far from their text in lightness, and the accent is made readable on the surface it reads worst on
//! (`palette::readable`).

use kuro_prefs::ui::AppColours;

use super::palette::{self, Rgb};

/// What a choice of colours is made from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scheme {
    /// The accent, before it is made readable on the surfaces.
    pub accent: Rgb,
    /// The hue of the surfaces, in degrees, and how much of it they carry (a saturation, 0 for grey).
    pub hue: f64,
    pub tint: f64,
    /// The dark style starts at black, not at dark grey.
    pub black: bool,
}

/// The scheme of a choice. `None` for the desktop's own colours: nothing is redeclared.
pub fn scheme(colours: AppColours) -> Option<Scheme> {
    Some(match colours {
        AppColours::DESKTOP => return None,
        AppColours::TOKYO_NIGHT => Scheme { accent: Rgb::new(0x7a, 0xa2, 0xf7), hue: 235.0, tint: 0.24, black: false },
        AppColours::ROSE => Scheme { accent: Rgb::new(0xeb, 0x6f, 0x92), hue: 250.0, tint: 0.22, black: false },
        AppColours::FOREST => Scheme { accent: Rgb::new(0x7f, 0xbf, 0x8e), hue: 150.0, tint: 0.16, black: false },
        AppColours::BLACK => Scheme { accent: Rgb::new(0x8a, 0xb4, 0xf8), hue: 0.0, tint: 0.0, black: true },
        AppColours::SAND => Scheme { accent: Rgb::new(0xc8, 0x7a, 0x2e), hue: 38.0, tint: 0.26, black: false },
    })
}

/// The choices in the order the Colours row lists them, with their names.
pub const CHOICES: [(AppColours, &str); 6] = [
    (AppColours::DESKTOP, "Desktop"),
    (AppColours::TOKYO_NIGHT, "Tokyo Night"),
    (AppColours::ROSE, "Rose"),
    (AppColours::FOREST, "Forest"),
    (AppColours::BLACK, "Black"),
    (AppColours::SAND, "Sand"),
];

/// The colours of one scheme in one style: each surface with what is written on it, and the accent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colours {
    pub window: (Rgb, Rgb),
    pub view: (Rgb, Rgb),
    pub headerbar: (Rgb, Rgb),
    pub sidebar: (Rgb, Rgb),
    pub card: (Rgb, Rgb),
    pub dialog: (Rgb, Rgb),
    pub popover: (Rgb, Rgb),
    /// The accent as text or an icon, and as a fill with what is written on it.
    pub accent: Rgb,
    pub accent_fill: (Rgb, Rgb),
}

impl Colours {
    /// Every surface with its text, by name.
    pub fn surfaces(&self) -> [(&'static str, (Rgb, Rgb)); 7] {
        [
            ("window", self.window),
            ("view", self.view),
            ("headerbar", self.headerbar),
            ("sidebar", self.sidebar),
            ("card", self.card),
            ("dialog", self.dialog),
            ("popover", self.popover),
        ]
    }
}

/// The lightness of each surface (window, view, header bar, sidebar, card, dialog, popover) and of the text, in the
/// order of libadwaita's own: in the dark style a view is darker than the window and what floats is lighter; in the
/// light style a view is white and the bars are a shade darker.
const DARK: ([f64; 7], f64) = ([0.130, 0.105, 0.160, 0.150, 0.185, 0.190, 0.200], 0.93);
const BLACK_DARK: ([f64; 7], f64) = ([0.0, 0.0, 0.045, 0.035, 0.080, 0.100, 0.100], 0.92);
const LIGHT: ([f64; 7], f64) = ([0.965, 0.995, 0.925, 0.935, 0.995, 0.965, 0.995], 0.14);

/// The colours of `scheme` in the dark or the light style.
pub fn colours(scheme: &Scheme, dark: bool) -> Colours {
    let (lightness, text) = match (dark, scheme.black) {
        (true, true) => BLACK_DARK,
        (true, false) => DARK,
        (false, _) => LIGHT,
    };
    // the text carries a little of the tint, so it does not look pasted on a tinted surface
    let written = Rgb::from_hsl(scheme.hue, scheme.tint * 0.5, text);
    let surface = |index: usize| (Rgb::from_hsl(scheme.hue, scheme.tint, lightness[index]), written);
    let surfaces: [(Rgb, Rgb); 7] = std::array::from_fn(surface);
    // the accent reads worst on the surface nearest to it in lightness: the lightest of the dark style, the darkest of
    // the light one. Readable there, it is readable on all of them.
    let hardest = surfaces
        .iter()
        .map(|(background, _)| *background)
        .max_by(|a, b| if dark { a.luminance().total_cmp(&b.luminance()) } else { b.luminance().total_cmp(&a.luminance()) })
        .unwrap_or(surfaces[0].0);
    let accent = palette::readable(scheme.accent, hardest);
    Colours {
        window: surfaces[0],
        view: surfaces[1],
        headerbar: surfaces[2],
        sidebar: surfaces[3],
        card: surfaces[4],
        dialog: surfaces[5],
        popover: surfaces[6],
        accent,
        accent_fill: (accent, palette::on(accent)),
    }
}

/// The style sheet of `scheme` in one style: libadwaita's colour names, redeclared, and its GTK 3 aliases with them.
pub fn sheet(scheme: &Scheme, dark: bool) -> String {
    let c = colours(scheme, dark);
    let pairs: [(&str, Rgb); 30] = [
        ("accent_color", c.accent),
        ("accent_bg_color", c.accent_fill.0),
        ("accent_fg_color", c.accent_fill.1),
        ("window_bg_color", c.window.0),
        ("window_fg_color", c.window.1),
        ("view_bg_color", c.view.0),
        ("view_fg_color", c.view.1),
        ("headerbar_bg_color", c.headerbar.0),
        ("headerbar_fg_color", c.headerbar.1),
        ("headerbar_backdrop_color", c.window.0),
        ("sidebar_bg_color", c.sidebar.0),
        ("sidebar_fg_color", c.sidebar.1),
        ("sidebar_backdrop_color", c.window.0),
        ("secondary_sidebar_bg_color", c.sidebar.0),
        ("secondary_sidebar_fg_color", c.sidebar.1),
        ("secondary_sidebar_backdrop_color", c.window.0),
        ("card_bg_color", c.card.0),
        ("card_fg_color", c.card.1),
        ("thumbnail_bg_color", c.dialog.0),
        ("thumbnail_fg_color", c.dialog.1),
        ("dialog_bg_color", c.dialog.0),
        ("dialog_fg_color", c.dialog.1),
        ("popover_bg_color", c.popover.0),
        ("popover_fg_color", c.popover.1),
        // the GTK 3 names the app's own sheet uses, which libadwaita keeps as aliases of the ones above
        ("theme_bg_color", c.window.0),
        ("theme_fg_color", c.window.1),
        ("theme_base_color", c.view.0),
        ("theme_text_color", c.view.1),
        ("theme_selected_bg_color", c.accent_fill.0),
        ("theme_selected_fg_color", c.accent_fill.1),
    ];
    pairs.iter().map(|(name, colour)| format!("@define-color {name} {};", colour.hex())).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::palette::TEXT_CONTRAST;

    #[test]
    fn the_desktop_redeclares_nothing_and_every_other_choice_has_a_scheme_and_a_name() {
        assert_eq!(scheme(AppColours::DESKTOP), None);
        assert_eq!(CHOICES[0].0, AppColours::DESKTOP, "the desktop's colours come first: they are what the app had");
        for colours in AppColours::ALL {
            assert_eq!(CHOICES.iter().filter(|(choice, _)| choice == colours).count(), 1, "{colours:?} is listed once");
            assert_eq!(scheme(*colours).is_none(), *colours == AppColours::DESKTOP, "{colours:?}");
        }
        assert_eq!(CHOICES.len(), AppColours::ALL.len());
        assert!(CHOICES.iter().all(|(_, name)| !name.is_empty()));
    }

    /// The check that stands in for looking at every screen: whatever the choice and the style, text reads on every
    /// surface, the accent reads on every surface, and what is written on the accent reads on it.
    #[test]
    fn text_and_the_accent_read_on_every_surface_of_every_choice_in_both_styles() {
        for (choice, name) in CHOICES {
            let Some(scheme) = scheme(choice) else { continue };
            for dark in [true, false] {
                let c = colours(&scheme, dark);
                for (surface, (background, text)) in c.surfaces() {
                    assert!(text.contrast(background) >= 7.0, "{name}, dark {dark}: the text on the {surface} ({})", text.contrast(background));
                    assert!(c.accent.contrast(background) >= TEXT_CONTRAST, "{name}, dark {dark}: the accent on the {surface} ({})", c.accent.contrast(background));
                }
                assert!(c.accent_fill.1.contrast(c.accent_fill.0) >= TEXT_CONTRAST, "{name}, dark {dark}: what is written on the accent");
            }
        }
    }

    #[test]
    fn a_style_is_dark_or_light_as_asked_and_black_starts_at_black() {
        for (choice, name) in CHOICES {
            let Some(scheme) = scheme(choice) else { continue };
            let (dark, light) = (colours(&scheme, true), colours(&scheme, false));
            assert!(dark.window.0.luminance() < 0.05 && dark.window.1.luminance() > 0.6, "{name}: light text on a dark window");
            assert!(light.window.0.luminance() > 0.8 && light.window.1.luminance() < 0.1, "{name}: dark text on a light window");
            assert_ne!(dark, light, "{name}");
        }
        let black = colours(&scheme(AppColours::BLACK).expect("a scheme"), true);
        assert_eq!((black.window.0, black.view.0), (palette::BLACK, palette::BLACK));
        assert!(black.popover.0.luminance() > 0.0, "what floats is told apart from the window");
    }

    #[test]
    fn the_sheet_redeclares_libadwaitas_names_and_the_ones_the_apps_sheet_uses() {
        let scheme = scheme(AppColours::TOKYO_NIGHT).expect("a scheme");
        let (dark, c) = (sheet(&scheme, true), colours(&scheme, true));
        assert_eq!(dark.lines().count(), 30);
        for line in dark.lines() {
            assert!(line.starts_with("@define-color ") && line.ends_with(';'), "{line}");
        }
        assert!(dark.contains(&format!("@define-color window_bg_color {};", c.window.0.hex())));
        assert!(dark.contains(&format!("@define-color accent_bg_color {};", c.accent_fill.0.hex())));
        // the four names the base sheet may use (`style::base_sheet_references_only_gtk_named_colours`)
        for name in ["theme_bg_color", "theme_fg_color", "theme_selected_bg_color", "theme_selected_fg_color"] {
            assert!(dark.contains(&format!("@define-color {name} #")), "{name}");
        }
        assert_ne!(dark, sheet(&scheme, false), "the light style has its own colours");
    }
}
