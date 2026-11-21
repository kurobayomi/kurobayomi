//! `[base]` preferences (Aniyomi `BasePreferences`) that the desktop keeps.
//!
//! `downloaded_only` and `incognito_mode` are app state and live in `state.toml`. The keys the
//! settings diet cut (display profile, long-strip decoding, the release check's last lookup) are dropped: a backed-up
//! value is skipped on restore. `check_app_updates` came back as the one update toggle (user, 2026-10-14).

use std::collections::BTreeSet;

use crate::pref_enum;
use crate::registry::define_prefs;

pref_enum! {
    /// `kinds_shown`: the kinds the app shows (user, 2026-11-21: for people who only watch anime or only read manga).
    /// With one kind, the pages show only it and lose their Anime | Manga switch; the other kind's data stays, and
    /// `BOTH` brings it back. A stored name that is no choice reads as `BOTH`.
    pub enum KindsShown { BOTH, ANIME, MANGA }
}

define_prefs! {
    /// `[base]` section: the keys that the desktop keeps of Aniyomi `BasePreferences`, plus the muted channels.
    pub struct BasePrefs in "base" {
        downloaded_only: bool = "__APP_STATE_pref_downloaded_only" => false;
        incognito_mode: bool = "__APP_STATE_incognito_mode" => false;
        // Linux addition: the notification channel ids (`notifications::CATEGORIES`) the user muted
        muted_notification_channels: BTreeSet<String> = "muted_notification_channels" => BTreeSet::new();
        // look for a newer release at start; an AppImage then updates itself (on by default)
        check_app_updates: bool = "check_app_updates" => true;
        // the Anime | Manga choice every page's switch shares, kept across runs: "anime" or "manga"
        last_kind: String = "__APP_STATE_last_kind" => String::from("anime");
        // the user took the AppImage out of the applications menu (About), so it does not add itself again
        appimage_menu_removed: bool = "__APP_STATE_appimage_menu_removed" => false;
        // the reader's and the player's bars hide by themselves while reading or watching (off by default)
        auto_hide_bars: bool = "auto_hide_bars" => true;
        // the kinds the pages show: both, or anime or manga alone (Settings > Appearance > Show)
        kinds_shown: KindsShown = "kinds_shown" => KindsShown::BOTH;
    }
}

#[cfg(test)]
mod tests {
    use kuro_platform::paths::Paths;

    use super::*;
    use crate::toml_backend::{config_file, state_file};
    use crate::settings_load::open;
    use crate::store::PrefStore;
    use crate::testutil::{TestDir, TestPaths, assert_section_registered, read_toml};
    use crate::values::PrefValue;
    use std::fs;

    #[test]
    fn keys_are_the_aniyomi_keys_in_their_base_fields() {
        assert_eq!(
            BasePrefs::FIELDS,
            &[
                ("downloaded_only", "__APP_STATE_pref_downloaded_only"),
                ("incognito_mode", "__APP_STATE_incognito_mode"),
                ("muted_notification_channels", "muted_notification_channels"),
                ("check_app_updates", "check_app_updates"),
                ("last_kind", "__APP_STATE_last_kind"),
                ("appimage_menu_removed", "__APP_STATE_appimage_menu_removed"),
                ("auto_hide_bars", "auto_hide_bars"),
                ("kinds_shown", "kinds_shown"),
            ]
        );
        assert_eq!(BasePrefs::TABLE, "base");
    }

    #[test]
    fn every_field_is_registered_under_the_base_table() {
        let base = BasePrefs::new(PrefStore::in_memory());
        assert_section_registered(BasePrefs::TABLE, BasePrefs::FIELDS, &base.defaults());
    }

    #[test]
    fn defaults_are_the_plan_defaults() {
        let base = BasePrefs::new(PrefStore::in_memory());
        assert!(!base.downloaded_only.get());
        assert!(!base.incognito_mode.get());
        assert!(base.muted_notification_channels.get().is_empty(), "no channel is muted by default");
        assert!(base.check_app_updates.get(), "updates are on by default");
        assert_eq!(base.kinds_shown.get(), KindsShown::BOTH, "both kinds until the user picks one");
    }

    #[test]
    fn the_kinds_shown_are_stored_by_name_and_an_unknown_name_shows_both() {
        let store = PrefStore::in_memory();
        let base = BasePrefs::new(store.clone());
        base.kinds_shown.set(KindsShown::MANGA).unwrap();
        assert_eq!(store.get_raw("kinds_shown"), Some(PrefValue::Str("MANGA".into())));
        store.set_raw("kinds_shown", PrefValue::Str("NOVELS".into()));
        assert_eq!(BasePrefs::new(store).kinds_shown.get(), KindsShown::BOTH);
    }

    #[test]
    fn base_values_round_trip_and_app_state_goes_to_state_toml() {
        let paths = TestPaths(TestDir::new("base-round-trip"));
        {
            let store = open(&paths).unwrap();
            let base = BasePrefs::new(store.clone());
            base.downloaded_only.set(true).unwrap();
            base.incognito_mode.set(true).unwrap();
            store.flush().unwrap();
        }
        let store = open(&paths).unwrap();
        let base = BasePrefs::new(store.clone());
        assert!(base.downloaded_only.get());
        assert!(base.incognito_mode.get());
        assert_eq!(
            store.get_raw("__APP_STATE_pref_downloaded_only"),
            Some(PrefValue::Bool(true))
        );

        let config = read_toml(&config_file(&paths.config_dir()));
        assert!(config.get("base").is_none_or(|t| t.get("downloaded_only").is_none()), "app state is not in config.toml");

        let state = read_toml(&state_file(&paths.state_dir()));
        assert_eq!(state["base"]["downloaded_only"].as_bool(), Some(true));
        assert_eq!(state["base"]["incognito_mode"].as_bool(), Some(true));
    }

    #[test]
    fn a_file_with_only_unknown_keys_gives_every_default_and_keeps_its_keys() {
        let paths = TestPaths(TestDir::new("bootstrap-unknown"));
        fs::create_dir_all(paths.config_dir()).unwrap();
        fs::create_dir_all(paths.state_dir()).unwrap();
        fs::write(
            config_file(&paths.config_dir()),
            "# from a newer build\nfuture_key = 1\n\n[future]\nx = \"y\"\n",
        )
        .unwrap();
        fs::write(state_file(&paths.state_dir()), "[future]\nz = true\n").unwrap();

        let store = open(&paths).unwrap();
        let base = BasePrefs::new(store.clone());
        assert!(!base.downloaded_only.get());

        base.incognito_mode.set(true).unwrap();
        store.flush().unwrap();
        let config = fs::read_to_string(config_file(&paths.config_dir())).unwrap();
        assert!(config.contains("# from a newer build"));
        assert!(config.contains("future_key = 1"));
        assert!(config.contains("[future]") && config.contains("x = \"y\""));
        let state = fs::read_to_string(state_file(&paths.state_dir())).unwrap();
        assert!(state.contains("[future]") && state.contains("z = true"));
    }
}
