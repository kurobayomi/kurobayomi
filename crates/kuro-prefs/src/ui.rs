//! `[ui]` preferences (Aniyomi `UiPreferences`), the UI enums they select, and the theme choice.
//!
//! The enums are stored by their Kotlin constant names. Titles are string-resource keys, which
//! the i18n area resolves; the GTK application is not touched here.

use crate::pref_enum;
use crate::registry::define_prefs;

pref_enum! {
    /// `pref_theme_mode_key`.
    pub enum ThemeMode { LIGHT, DARK, SYSTEM }
}

pref_enum! {
    /// `pref_app_theme`. The three deprecated themes stay valid stored values but are not offered. A stored `MONET`
    /// (the Android dynamic colours theme, not available here) is no name any more and reads as `DEFAULT`.
    pub enum AppTheme {
        DEFAULT, CLOUDFLARE, COTTONCANDY, DOOM, GREEN_APPLE, LAVENDER, MATRIX,
        MIDNIGHT_DUSK, MOCHA, SAPPHIRE, NORD, STRAWBERRY_DAIQUIRI, TAKO, TEALTURQUOISE,
        TIDAL_WAVE, YINYANG, YOTSUBA, MONOCHROME, DARK_BLUE, HOT_PINK, BLUE,
    }
}

pref_enum! {
    /// `app_colours`: the colours of the whole app, a short list of palettes people know and one of Kurobayomi's own,
    /// `BLACK` (the Colours row of the Appearance page). `DESKTOP` leaves them to the desktop, as before there was a
    /// choice. Aniyomi's palettes (`AppTheme`) are not these: their key stays a dropped one. A stored name that is no
    /// longer a choice reads as `DESKTOP`.
    pub enum AppColours { DESKTOP, TOKYO_NIGHT, CATPPUCCIN, DRACULA, NORD, GRUVBOX, ROSE_PINE, BLACK }
}

impl AppTheme {
    /// The string-resource key of the picker title, or `None` for a deprecated theme.
    pub fn title_key(self) -> Option<&'static str> {
        Some(match self {
            Self::DEFAULT => "label_default",
            Self::CLOUDFLARE => "theme_cloudflare",
            Self::COTTONCANDY => "theme_cottoncandy",
            Self::DOOM => "theme_doom",
            Self::GREEN_APPLE => "theme_greenapple",
            Self::LAVENDER => "theme_lavender",
            Self::MATRIX => "theme_matrix",
            Self::MIDNIGHT_DUSK => "theme_midnightdusk",
            Self::MOCHA => "theme_mocha",
            Self::SAPPHIRE => "theme_sapphire",
            Self::NORD => "theme_nord",
            Self::STRAWBERRY_DAIQUIRI => "theme_strawberrydaiquiri",
            Self::TAKO => "theme_tako",
            Self::TEALTURQUOISE => "theme_tealturquoise",
            Self::TIDAL_WAVE => "theme_tidalwave",
            Self::YINYANG => "theme_yinyang",
            Self::YOTSUBA => "theme_yotsuba",
            Self::MONOCHROME => "theme_monochrome",
            Self::DARK_BLUE | Self::HOT_PINK | Self::BLUE => return None,
        })
    }
}

/// A top-level destination (Kotlin `Tab`). Aniyomi's More tab is cut: its places are the panel items and the app
/// menu. The old `bottom_rail_nav_style` key is a dropped key, so a stored value still loads. Home is the start tab;
/// Updates and History are pages Home opens, not tabs (docs/design/home.md), and the `start_screen` setting that chose
/// another start tab is a dropped key (user, 2026-11-21).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Home,
    AnimeLibrary,
    MangaLibrary,
    Browse,
}

/// What the GTK layer must do for a theme mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    /// Set `gtk-application-prefer-dark-theme` to false.
    Light,
    /// Set `gtk-application-prefer-dark-theme` to true.
    Dark,
    /// Leave the choice to the desktop colour scheme, and follow its changes.
    FollowSystem,
}

/// The theme chosen by `mode`. Only the choice is returned; GTK is not called.
pub fn apply_theme(mode: ThemeMode) -> ThemeChoice {
    match mode {
        ThemeMode::LIGHT => ThemeChoice::Light,
        ThemeMode::DARK => ThemeChoice::Dark,
        ThemeMode::SYSTEM => ThemeChoice::FollowSystem,
    }
}

define_prefs! {
    /// `[ui]` section: the theme mode of Aniyomi `UiPreferences`, and Kurobayomi's own colours and switch for the colour
    /// a title's page takes from its cover (`entry_cover_colours`).
    pub struct UiPrefs in "ui" {
        theme_mode: ThemeMode = "pref_theme_mode_key" => ThemeMode::SYSTEM;
        cover_colours: bool = "entry_cover_colours" => true;
        colours: AppColours = "app_colours" => AppColours::DESKTOP;
    }
}

#[cfg(test)]
mod tests {
    use kuro_platform::paths::Paths;

    use super::*;
    use crate::toml_backend::config_file;
    use crate::settings_load::open;
    use crate::store::PrefStore;
    use crate::testutil::{TestDir, TestPaths, assert_section_registered, read_toml};
    use crate::values::PrefValue;

    #[test]
    fn keys_are_the_aniyomi_keys_in_their_ui_fields() {
        assert_eq!(
            UiPrefs::FIELDS,
            &[
                ("theme_mode", "pref_theme_mode_key"),
                ("cover_colours", "entry_cover_colours"),
                ("colours", "app_colours"),
            ]
        );
        assert_eq!(UiPrefs::TABLE, "ui");
    }

    #[test]
    fn every_field_is_registered_under_the_ui_table() {
        let ui = UiPrefs::new(PrefStore::in_memory());
        assert_section_registered(UiPrefs::TABLE, UiPrefs::FIELDS, &ui.defaults());
    }

    #[test]
    fn defaults_are_the_plan_defaults() {
        let ui = UiPrefs::new(PrefStore::in_memory());
        assert_eq!(ui.theme_mode.get(), ThemeMode::SYSTEM);
        assert!(ui.cover_colours.get(), "a title's page takes its cover's colour unless it is turned off");
        assert_eq!(ui.colours.get(), AppColours::DESKTOP, "the desktop's colours until another choice is made");
        assert_eq!(AppColours::TOKYO_NIGHT.name(), "TOKYO_NIGHT");
        assert_eq!(AppColours::from_name("ROSE_PINE"), Some(AppColours::ROSE_PINE));
        assert_eq!(AppColours::from_name("SAND"), None, "a choice of the first list, which a nightly offered for a day");
    }

    #[test]
    fn enums_are_stored_by_their_constant_names() {
        let store = PrefStore::in_memory();
        let ui = UiPrefs::new(store.clone());
        ui.theme_mode.set(ThemeMode::DARK).unwrap();
        ui.colours.set(AppColours::NORD).unwrap();
        assert_eq!(
            store.get_raw("pref_theme_mode_key"),
            Some(PrefValue::Str("DARK".into()))
        );
        assert_eq!(
            store.get_raw("app_colours"),
            Some(PrefValue::Str("NORD".into()))
        );
    }

    #[test]
    fn an_unknown_stored_name_gives_the_default_and_stays_stored() {
        let store = PrefStore::in_memory();
        store.set_raw("app_colours", PrefValue::Str("NOPE".into()));
        let ui = UiPrefs::new(store.clone());
        assert_eq!(ui.colours.get(), AppColours::DESKTOP);
        assert!(ui.colours.is_set());
    }

    #[test]
    fn a_stored_theme_name_and_an_old_tablet_key_do_not_break_loading() {
        let store = PrefStore::in_memory();
        store.set_raw("pref_app_theme", PrefValue::Str("MONET".into()));
        store.set_raw("tablet_ui_mode", PrefValue::Str("ALWAYS".into()));
        let ui = UiPrefs::new(store);
        assert_eq!(ui.theme_mode.get(), ThemeMode::SYSTEM);
    }

    #[test]
    fn an_old_start_screen_is_a_dropped_key() {
        assert!(crate::registry::is_dropped("start_screen"), "Home is always first; a stored start screen still loads");
    }

    #[test]
    fn theme_modes_choose_the_gtk_setting() {
        assert_eq!(apply_theme(ThemeMode::LIGHT), ThemeChoice::Light);
        assert_eq!(apply_theme(ThemeMode::DARK), ThemeChoice::Dark);
        assert_eq!(apply_theme(ThemeMode::SYSTEM), ThemeChoice::FollowSystem);
    }

    #[test]
    fn ui_values_round_trip_through_config_toml_in_the_ui_table() {
        let paths = TestPaths(TestDir::new("ui-round-trip"));
        {
            let store = open(&paths).unwrap();
            let ui = UiPrefs::new(store.clone());
            ui.theme_mode.set(ThemeMode::LIGHT).unwrap();
            ui.colours.set(AppColours::GRUVBOX).unwrap();
            store.flush().unwrap();
        }
        let store = open(&paths).unwrap();
        let ui = UiPrefs::new(store);
        assert_eq!(ui.theme_mode.get(), ThemeMode::LIGHT);
        assert_eq!(ui.colours.get(), AppColours::GRUVBOX);
        let doc = read_toml(&config_file(&paths.config_dir()));
        assert_eq!(doc["ui"]["theme_mode"].as_str(), Some("LIGHT"));
        assert_eq!(doc["ui"]["colours"].as_str(), Some("GRUVBOX"));
    }
}
