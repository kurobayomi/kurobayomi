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
/// menu. The old `bottom_rail_nav_style` key is a dropped key, so a stored value still loads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    AnimeLibrary,
    MangaLibrary,
    Updates,
    History,
    Browse,
}

pref_enum! {
    /// `start_screen`: the tab shown at launch.
    pub enum StartScreen { ANIME, MANGA, UPDATES, HISTORY, BROWSE }
}

impl StartScreen {
    /// The destination this screen opens.
    pub fn tab(self) -> Tab {
        match self {
            Self::ANIME => Tab::AnimeLibrary,
            Self::MANGA => Tab::MangaLibrary,
            Self::UPDATES => Tab::Updates,
            Self::HISTORY => Tab::History,
            Self::BROWSE => Tab::Browse,
        }
    }
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
    /// `[ui]` section: the seven keys of Aniyomi `UiPreferences` that a desktop has, and Kurobayomi's own switch for the
    /// colour a title's page takes from its cover (`entry_cover_colours`).
    pub struct UiPrefs in "ui" {
        theme_mode: ThemeMode = "pref_theme_mode_key" => ThemeMode::SYSTEM;
        start_screen: StartScreen = "start_screen" => StartScreen::ANIME;
        cover_colours: bool = "entry_cover_colours" => true;
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
                ("start_screen", "start_screen"),
                ("cover_colours", "entry_cover_colours"),
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
        assert_eq!(ui.start_screen.get(), StartScreen::ANIME);
        assert!(ui.cover_colours.get(), "a title's page takes its cover's colour unless it is turned off");
    }

    #[test]
    fn enums_are_stored_by_their_constant_names() {
        let store = PrefStore::in_memory();
        let ui = UiPrefs::new(store.clone());
        ui.theme_mode.set(ThemeMode::DARK).unwrap();
        ui.start_screen.set(StartScreen::BROWSE).unwrap();
        assert_eq!(
            store.get_raw("pref_theme_mode_key"),
            Some(PrefValue::Str("DARK".into()))
        );
        assert_eq!(
            store.get_raw("start_screen"),
            Some(PrefValue::Str("BROWSE".into()))
        );
    }

    #[test]
    fn an_unknown_stored_name_gives_the_default_and_stays_stored() {
        let store = PrefStore::in_memory();
        store.set_raw("start_screen", PrefValue::Str("NOPE".into()));
        let ui = UiPrefs::new(store.clone());
        assert_eq!(ui.start_screen.get(), StartScreen::ANIME);
        assert!(ui.start_screen.is_set());
    }

    #[test]
    fn a_stored_theme_name_and_an_old_tablet_key_do_not_break_loading() {
        let store = PrefStore::in_memory();
        store.set_raw("pref_app_theme", PrefValue::Str("MONET".into()));
        store.set_raw("tablet_ui_mode", PrefValue::Str("ALWAYS".into()));
        let ui = UiPrefs::new(store);
        assert_eq!(ui.theme_mode.get(), ThemeMode::SYSTEM);
        assert_eq!(ui.start_screen.get(), StartScreen::ANIME);
    }

    #[test]
    fn start_screens_open_their_tabs() {
        assert_eq!(StartScreen::ANIME.tab(), Tab::AnimeLibrary);
        assert_eq!(StartScreen::MANGA.tab(), Tab::MangaLibrary);
        assert_eq!(StartScreen::UPDATES.tab(), Tab::Updates);
        assert_eq!(StartScreen::HISTORY.tab(), Tab::History);
        assert_eq!(StartScreen::BROWSE.tab(), Tab::Browse);
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
            ui.start_screen.set(StartScreen::UPDATES).unwrap();
            store.flush().unwrap();
        }
        let store = open(&paths).unwrap();
        let ui = UiPrefs::new(store);
        assert_eq!(ui.theme_mode.get(), ThemeMode::LIGHT);
        assert_eq!(ui.start_screen.get(), StartScreen::UPDATES);
        let doc = read_toml(&config_file(&paths.config_dir()));
        assert_eq!(doc["ui"]["theme_mode"].as_str(), Some("LIGHT"));
        assert_eq!(doc["ui"]["start_screen"].as_str(), Some("UPDATES"));
    }
}
