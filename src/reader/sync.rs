//! Settings change propagation to the open reader (T-043a-080). A preference change is mapped to the reader parts
//! it affects, and the reader applies those parts while it stays open. Only the theme and the incognito flag are still
//! live: the other reader settings were cut from the settings and are hard-coded.

use std::rc::Weak;
use std::time::Duration;

use gtk::glib;
use kuro_prefs::store::PrefStore;

/// The reader parts a preference change can touch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Theme,
    /// The global incognito flag changed; the reader closes when it is off (E-INC-2).
    Incognito,
}

/// The preference key of the global incognito flag (`BasePrefs::incognito_mode`).
pub const INCOGNITO_KEY: &str = "__APP_STATE_incognito_mode";

/// Whether the reader closes for a new value of the global incognito flag (E-INC-2).
pub fn closes_on_incognito(now_on: bool) -> bool {
    !now_on
}

/// The effects of a changed preference key. Keys that touch no open reader part give nothing.
pub fn effects_for(key: &str) -> Vec<Effect> {
    match key {
        // the default and any manga's own background: the reader reads the one that applies to its manga
        "pref_reader_theme_key" => vec![Effect::Theme],
        key if key.starts_with("manga_reader_theme_") => vec![Effect::Theme],
        INCOGNITO_KEY => vec![Effect::Incognito],
        _ => Vec::new(),
    }
}

/// Polls the store's change feed on the GTK thread and calls `apply` for each effect while the reader lives. The
/// poll ends when `owner` is gone, which is when the reader closes.
pub fn follow<T: 'static>(store: &PrefStore, owner: Weak<T>, apply: impl Fn(&T, Effect) + 'static) {
    let changes = store.subscribe();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let Some(owner) = owner.upgrade() else { return glib::ControlFlow::Break };
        while let Ok(change) = changes.try_recv() {
            for effect in change.key.as_deref().map(effects_for).unwrap_or_default() {
                apply(&owner, effect);
            }
        }
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_live_settings_map_to_their_reader_parts() {
        assert_eq!(effects_for("pref_reader_theme_key"), [Effect::Theme]);
        assert_eq!(effects_for("manga_reader_theme_42"), [Effect::Theme]);
        assert!(effects_for("pref_show_page_number_key").is_empty(), "cut from the settings");
        assert!(effects_for("reader_navigation_mode_pager").is_empty(), "cut from the settings");
    }

    #[test]
    fn the_incognito_flag_maps_to_its_effect_and_only_turning_it_off_closes_the_reader() {
        assert_eq!(effects_for(INCOGNITO_KEY), [Effect::Incognito]);
        assert!(closes_on_incognito(false));
        assert!(!closes_on_incognito(true));
    }

    #[test]
    fn the_read_at_open_settings_and_unrelated_keys_give_no_effect() {
        assert!(effects_for("pref_download_ahead_amount").is_empty());
        assert!(effects_for("reader_long_tap").is_empty());
        assert!(effects_for("skip_read").is_empty(), "the chapter skip rules are hard-coded");
    }
}
