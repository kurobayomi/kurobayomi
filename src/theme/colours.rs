//! The colours of the whole app, for the choices other than the desktop's own (`AppColours`, the Colours row of the
//! Appearance page). A choice is a palette people already know, with its own colours for the dark and for the light
//! style, written here as its project publishes them (each constant names its source; `NOTICE` credits them). Black is
//! the one that is ours: greys that start at black, for a screen whose black pixels are off. Pure: `apply` loads the
//! sheet.
//!
//! A palette gives five surfaces, a text colour and an accent ([`Style`]); libadwaita's colour names are filled from
//! them ([`sheet`]). libadwaita lets an app redeclare its named colours (`doc/named-colors.md` of 1.5: "Applications
//! can override any of these colors by re-declaring them"). The names the app's own sheet uses (`theme_bg_color` and
//! the like) are its aliases of those; they are redeclared too, so nothing depends on how an alias follows.
//!
//! What is filled in the accent (a button, a switch, a selected row) has the palette's accent as it is. What is written
//! in the accent must read on every surface, so there it is the accent made readable (`palette::readable`), which is the
//! accent itself wherever it already reads. A test checks text and accent on every surface of every choice.

use kuro_i18n::{MsgKey, tr};
use kuro_prefs::ui::AppColours;

use super::palette::{self, Rgb};

/// A colour as its palette's own files write it: `0xRRGGBB`.
const fn rgb(hex: u32) -> Rgb {
    Rgb::new((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// One style (dark or light) of a choice: the palette's colours, by the part each plays here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    /// The background of a page.
    pub window: Rgb,
    /// The background of a list or of text.
    pub view: Rgb,
    /// The header bar and the sidebar.
    pub bars: Rgb,
    /// A card or a boxed list.
    pub raised: Rgb,
    /// A dialog or a popover.
    pub floating: Rgb,
    pub text: Rgb,
    /// The accent as the palette has it.
    pub accent: Rgb,
}

impl Style {
    /// `surfaces` are the window, the view, the bars, what is raised and what floats.
    const fn new(surfaces: [u32; 5], text: u32, accent: u32) -> Self {
        Style {
            window: rgb(surfaces[0]),
            view: rgb(surfaces[1]),
            bars: rgb(surfaces[2]),
            raised: rgb(surfaces[3]),
            floating: rgb(surfaces[4]),
            text: rgb(text),
            accent: rgb(accent),
        }
    }
}

/// A choice of colours: its dark and its light style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scheme {
    pub dark: Style,
    pub light: Style,
}

/// Tokyo Night and Tokyo Night Light, from the theme's own files (tokyo-night/tokyo-night-vscode-theme, MIT): the
/// editor, the side bar, the selected row and the menu; the editor's text; the blue of its functions and of its links.
const TOKYO_NIGHT: Scheme = Scheme {
    dark: Style::new([0x1a1b26, 0x1a1b26, 0x16161e, 0x202330, 0x16161e], 0xa9b1d6, 0x7aa2f7),
    light: Style::new([0xe6e7ed, 0xe6e7ed, 0xd6d8df, 0xdcdee3, 0xd6d8df], 0x343b58, 0x2959aa),
};

/// Catppuccin Mocha and Latte (catppuccin/palette, MIT): base, mantle and surface 0; text; mauve.
const CATPPUCCIN: Scheme = Scheme {
    dark: Style::new([0x1e1e2e, 0x1e1e2e, 0x181825, 0x313244, 0x181825], 0xcdd6f4, 0xcba6f7),
    light: Style::new([0xeff1f5, 0xeff1f5, 0xe6e9ef, 0xccd0da, 0xe6e9ef], 0x4c4f69, 0x8839ef),
};

/// Dracula and its light theme, Alucard (dracula/vim and dracula/cursor, MIT; draculatheme.com/spec): the background
/// with its lighter and darker steps; the foreground; purple.
const DRACULA: Scheme = Scheme {
    dark: Style::new([0x282a36, 0x282a36, 0x21222c, 0x343746, 0x21222c], 0xf8f8f2, 0xbd93f9),
    light: Style::new([0xfffbeb, 0xfffbeb, 0xdedccf, 0xece9df, 0xdedccf], 0x1f1f1f, 0x644ac9),
};

/// Nord (nordtheme/nord, MIT). Dark: Polar Night (nord0 to nord2) under Snow Storm text (nord4), with the frost of
/// nord8. Light: Snow Storm (nord6 to nord4) under nord0 text, with nord10.
const NORD: Scheme = Scheme {
    dark: Style::new([0x2e3440, 0x2e3440, 0x3b4252, 0x3b4252, 0x434c5e], 0xd8dee9, 0x88c0d0),
    light: Style::new([0xeceff4, 0xeceff4, 0xe5e9f0, 0xe5e9f0, 0xd8dee9], 0x2e3440, 0x5e81ac),
};

/// Gruvbox dark and light (morhetz/gruvbox, MIT/X11): the medium background with its hard, soft and next steps; the
/// foreground; orange, bright on dark and faded on light.
const GRUVBOX: Scheme = Scheme {
    dark: Style::new([0x282828, 0x282828, 0x1d2021, 0x3c3836, 0x32302f], 0xebdbb2, 0xfe8019),
    light: Style::new([0xfbf1c7, 0xfbf1c7, 0xebdbb2, 0xf9f5d7, 0xf2e5bc], 0x3c3836, 0xaf3a03),
};

/// Rosé Pine and Rosé Pine Dawn (rose-pine/neovim, MIT): base, surface and overlay; text; rose.
const ROSE_PINE: Scheme = Scheme {
    dark: Style::new([0x191724, 0x191724, 0x1f1d2e, 0x1f1d2e, 0x26233a], 0xe0def4, 0xebbcba),
    light: Style::new([0xfaf4ed, 0xfaf4ed, 0xfffaf3, 0xfffaf3, 0xf2e9e1], 0x464261, 0xd7827e),
};

/// Ours: the dark style starts at black and steps up in greys; the light one is plain light greys.
const BLACK: Scheme = Scheme {
    dark: Style::new([0x000000, 0x000000, 0x0b0b0b, 0x141414, 0x1a1a1a], 0xebebeb, 0x8ab4f8),
    light: Style::new([0xfafafa, 0xffffff, 0xebebeb, 0xffffff, 0xffffff], 0x242424, 0x8ab4f8),
};

/// The scheme of a choice. `None` for the desktop's own colours: nothing is redeclared.
pub fn scheme(colours: AppColours) -> Option<Scheme> {
    Some(match colours {
        AppColours::DESKTOP => return None,
        AppColours::TOKYO_NIGHT => TOKYO_NIGHT,
        AppColours::CATPPUCCIN => CATPPUCCIN,
        AppColours::DRACULA => DRACULA,
        AppColours::NORD => NORD,
        AppColours::GRUVBOX => GRUVBOX,
        AppColours::ROSE_PINE => ROSE_PINE,
        AppColours::BLACK => BLACK,
    })
}

/// The choices in the order the Colours row lists them: the desktop's own first, as it is what the app had, and ours
/// last.
pub const ORDER: [AppColours; 8] = [
    AppColours::DESKTOP,
    AppColours::TOKYO_NIGHT,
    AppColours::CATPPUCCIN,
    AppColours::DRACULA,
    AppColours::NORD,
    AppColours::GRUVBOX,
    AppColours::ROSE_PINE,
    AppColours::BLACK,
];

/// The name of a choice as the Colours row shows it, from the string table like every word of the interface
/// (`i18n_guard`).
pub fn name(colours: AppColours) -> String {
    tr(match colours {
        AppColours::DESKTOP => MsgKey::ui_colours_desktop,
        AppColours::TOKYO_NIGHT => MsgKey::ui_colours_tokyo_night,
        AppColours::CATPPUCCIN => MsgKey::ui_colours_catppuccin,
        AppColours::DRACULA => MsgKey::ui_colours_dracula,
        AppColours::NORD => MsgKey::ui_colours_nord,
        AppColours::GRUVBOX => MsgKey::ui_colours_gruvbox,
        AppColours::ROSE_PINE => MsgKey::ui_colours_rose_pine,
        AppColours::BLACK => MsgKey::ui_colours_black,
    })
}

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
    /// Every surface with its text, by name: what the readability test walks.
    #[cfg(test)]
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

/// The colours of `scheme` in the dark or the light style.
pub fn colours(scheme: &Scheme, dark: bool) -> Colours {
    let style = if dark { scheme.dark } else { scheme.light };
    let written = |surface: Rgb| (surface, style.text);
    // the accent reads worst on the surface nearest to it in lightness: the lightest of the dark style, the darkest of
    // the light one. Readable there, it is readable on all of them.
    let hardest = [style.window, style.view, style.bars, style.raised, style.floating]
        .into_iter()
        .max_by(|a, b| if dark { a.luminance().total_cmp(&b.luminance()) } else { b.luminance().total_cmp(&a.luminance()) })
        .unwrap_or(style.window);
    Colours {
        window: written(style.window),
        view: written(style.view),
        headerbar: written(style.bars),
        sidebar: written(style.bars),
        card: written(style.raised),
        dialog: written(style.floating),
        popover: written(style.floating),
        accent: palette::readable(style.accent, hardest),
        accent_fill: (style.accent, palette::on(style.accent)),
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
        assert_eq!(ORDER[0], AppColours::DESKTOP, "the desktop's colours come first: they are what the app had");
        for colours in AppColours::ALL {
            assert_eq!(ORDER.iter().filter(|choice| *choice == colours).count(), 1, "{colours:?} is listed once");
            assert_eq!(scheme(*colours).is_none(), *colours == AppColours::DESKTOP, "{colours:?}");
        }
        assert_eq!(ORDER.len(), AppColours::ALL.len());
        assert_eq!(ORDER.map(name), ["Desktop", "Tokyo Night", "Catppuccin", "Dracula", "Nord", "Gruvbox", "Rosé Pine", "Black"]);
    }

    /// The check that stands in for looking at every screen: whatever the choice and the style, text reads on every
    /// surface, the accent reads on every surface, and what is written on the accent reads on it.
    #[test]
    fn text_and_the_accent_read_on_every_surface_of_every_choice_in_both_styles() {
        for choice in ORDER {
            let Some(scheme) = scheme(choice) else { continue };
            let name = name(choice);
            for dark in [true, false] {
                let c = colours(&scheme, dark);
                for (surface, (background, text)) in c.surfaces() {
                    assert!(text.contrast(background) >= TEXT_CONTRAST, "{name}, dark {dark}: the text on the {surface} ({})", text.contrast(background));
                    assert!(c.accent.contrast(background) >= TEXT_CONTRAST, "{name}, dark {dark}: the accent on the {surface} ({})", c.accent.contrast(background));
                }
                assert!(c.accent_fill.1.contrast(c.accent_fill.0) >= TEXT_CONTRAST, "{name}, dark {dark}: what is written on the accent");
            }
        }
    }

    #[test]
    fn a_style_is_dark_or_light_as_asked_and_black_starts_at_black() {
        for choice in ORDER {
            let Some(scheme) = scheme(choice) else { continue };
            let name = name(choice);
            let (dark, light) = (colours(&scheme, true), colours(&scheme, false));
            assert!(dark.window.0.luminance() < 0.05 && dark.window.1.luminance() > 0.4, "{name}: light text on a dark window");
            assert!(light.window.0.luminance() > 0.7 && light.window.1.luminance() < 0.1, "{name}: dark text on a light window");
            assert_ne!(dark, light, "{name}");
        }
        let black = colours(&scheme(AppColours::BLACK).expect("a scheme"), true);
        assert_eq!((black.window.0, black.view.0), (palette::BLACK, palette::BLACK));
        assert!(black.popover.0.luminance() > 0.0, "what floats is told apart from the window");
    }

    #[test]
    fn a_palette_keeps_its_own_colours_and_only_the_written_accent_may_move() {
        let of = |choice, dark| colours(&scheme(choice).expect("a scheme"), dark);
        let mocha = of(AppColours::CATPPUCCIN, true);
        assert_eq!((mocha.window.0.hex(), mocha.headerbar.0.hex(), mocha.card.0.hex()), ("#1e1e2e".to_owned(), "#181825".to_owned(), "#313244".to_owned()));
        assert_eq!((mocha.window.1.hex(), mocha.accent_fill.0.hex()), ("#cdd6f4".to_owned(), "#cba6f7".to_owned()));
        assert_eq!(mocha.accent, mocha.accent_fill.0, "mauve reads on every Mocha surface, so it is written as it is");
        assert_eq!(mocha.sidebar, mocha.headerbar, "the bars are one colour");
        assert_eq!(mocha.popover, mocha.dialog, "and so is what floats");

        let tokyo = of(AppColours::TOKYO_NIGHT, true);
        assert_eq!((tokyo.window.0.hex(), tokyo.sidebar.0.hex(), tokyo.accent.hex()), ("#1a1b26".to_owned(), "#16161e".to_owned(), "#7aa2f7".to_owned()));
        assert_eq!(of(AppColours::NORD, false).window, (rgb(0xeceff4), rgb(0x2e3440)));
        assert_eq!(of(AppColours::GRUVBOX, true).window, (rgb(0x282828), rgb(0xebdbb2)));
        assert_eq!(of(AppColours::DRACULA, true).accent_fill.0.hex(), "#bd93f9");

        // Dawn's rose is too light to read as text on Dawn's surfaces: a fill keeps it, text gets it darker
        let dawn = of(AppColours::ROSE_PINE, false);
        assert_eq!(dawn.accent_fill.0.hex(), "#d7827e");
        assert_ne!(dawn.accent, dawn.accent_fill.0);
        assert!(dawn.accent.luminance() < dawn.accent_fill.0.luminance());
    }

    #[test]
    fn a_colour_is_read_as_its_palette_writes_it() {
        assert_eq!(rgb(0x1a2b3c), Rgb::new(0x1a, 0x2b, 0x3c));
        assert_eq!(rgb(0xcba6f7).hex(), "#cba6f7");
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
