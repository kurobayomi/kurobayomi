//! Theme preferences over the area 02 store: the mode key and its reading.
//!
//! `pref_app_theme` held the colour palettes, which the settings diet cut. Its stored value is no longer read, so a
//! palette name there has no effect and the mode is whatever `pref_theme_mode_key` holds.

use kuro_prefs::store::PrefStore;
use kuro_prefs::ui::{AppColours, ThemeMode};
use kuro_prefs::values::PrefValue;

use super::{AppTheme, ThemeSettings};

pub const KEY_THEME_MODE: &str = "pref_theme_mode_key";
/// The colours of the app (`UiPrefs::colours`). Not Aniyomi's `pref_app_theme`, which stays unread.
pub const KEY_COLOURS: &str = "app_colours";

/// Reads the theme settings: the stored mode, System when unset or unknown, and the stored colours, the desktop's when
/// unset or unknown.
pub fn read_settings(store: &PrefStore) -> ThemeSettings {
    let mode = match store.get_raw(KEY_THEME_MODE) {
        Some(PrefValue::Str(raw)) => AppTheme::from_stored(&raw).mode(),
        _ => ThemeMode::SYSTEM,
    };
    let colours = match store.get_raw(KEY_COLOURS) {
        Some(PrefValue::Str(raw)) => AppColours::from_name(&raw).unwrap_or(AppColours::DESKTOP),
        _ => AppColours::DESKTOP,
    };
    ThemeSettings { mode, colours }
}

#[cfg(test)]
/// Writes the mode choice.
pub fn write_mode(store: &PrefStore, mode: ThemeMode) {
    store.set_raw(KEY_THEME_MODE, PrefValue::Str(mode.name().to_string()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_mode(raw: &str) -> PrefStore {
        let store = PrefStore::in_memory();
        store.set_raw(KEY_THEME_MODE, PrefValue::Str(raw.to_string()));
        store
    }

    #[test]
    fn unset_mode_is_system() {
        assert_eq!(read_settings(&PrefStore::in_memory()).mode, ThemeMode::SYSTEM);
    }

    #[test]
    fn a_written_mode_round_trips() {
        let store = PrefStore::in_memory();
        for mode in [ThemeMode::LIGHT, ThemeMode::DARK, ThemeMode::SYSTEM] {
            write_mode(&store, mode);
            assert_eq!(read_settings(&store).mode, mode);
        }
    }

    #[test]
    fn the_stored_colours_are_read_and_anything_else_is_the_desktops() {
        let store = PrefStore::in_memory();
        assert_eq!(read_settings(&store).colours, AppColours::DESKTOP, "unset");
        store.set_raw(KEY_COLOURS, PrefValue::Str("TOKYO_NIGHT".to_string()));
        assert_eq!(read_settings(&store).colours, AppColours::TOKYO_NIGHT);
        store.set_raw(KEY_COLOURS, PrefValue::Str("MOCHA".to_string()));
        assert_eq!(read_settings(&store).colours, AppColours::DESKTOP, "a name that is none of ours");
        assert_eq!(kuro_prefs::ui::UiPrefs::new(store).colours.key(), KEY_COLOURS, "the key the Colours row writes");
    }

    #[test]
    fn unknown_mode_reads_as_system() {
        assert_eq!(read_settings(&store_with_mode("FOO")).mode, ThemeMode::SYSTEM);
    }

    #[test]
    fn a_cut_palette_name_in_the_old_key_is_ignored() {
        let store = PrefStore::in_memory();
        store.set_raw("pref_app_theme", PrefValue::Str("MOCHA".to_string()));
        assert_eq!(read_settings(&store).mode, ThemeMode::SYSTEM);
        assert_eq!(store.get_raw("pref_app_theme"), Some(PrefValue::Str("MOCHA".to_string())), "not rewritten");
    }
}
