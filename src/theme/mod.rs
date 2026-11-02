//! Theme: the stored mode, libadwaita's colour scheme, GTK CSS, and the thin widgets built on them.
//!
//! The pure parts (`tokens`, `prefs`, `app_theme`, `style`, `icons`) have unit tests and no GTK calls. `apply`,
//! `components` and the widget functions of the other modules are the GTK layer.
//!
//! `startup` is the single call `main.rs` makes before the first window is presented (T-49a-058).
//! The other helpers are used by the screens as their lanes adopt them.
//!
//! Android theme items that Kurobayomi does not port (area 49b, T-49ba-007, 008, 023, 033, 039,
//! 040, 042, 047). Each has one reason here:
//! - Dynamic colour: the Android `m3_sys_color_dynamic_*` tokens are never read and there is no dynamic scheme.
//!   A stored `MONET` reads as the GTK theme (`prefs`).
//! - `lightStatusBar`, `windowLightStatusBar`, `lightNavigationBar`, `elevationOverlayEnabled`, `windowLightNavigationBar`
//!   and the API-27 navigation-bar overrides: GTK has no system bars and no forced dark mode; the
//!   dark decision comes from libadwaita's style manager (`apply::follow_system`).
//! - `forceDarkAllowed`, `statusBarColor`, `navigationBarColor`, `navigationBarDividerColor`,
//!   `enforceNavigationBarContrast`, `enforceStatusBarContrast`, `windowActionModeOverlay`,
//!   `elevationOverlayColor`: the same reason, no system bars and no elevation tint.
//! - The More-tab three-dot morph: decorative, and the Android `time_group` target has no children.
//! - `PreferenceThemeOverlay.Tachiyomi` items and `Switch` elevation: GTK has no preference overlay,
//!   and `gtk::Switch` has no elevation tint.
//! - `MinMaxNumberPicker` and `MaterialSpinnerView`: numeric settings use `gtk::SpinButton`.
//! - Widget corner radii (`appwidget_*`): there is no home-screen widget on Linux.
//! - `ActionButton.Overflow` and the unused AppCompat full-screen style: no reference in the app.

pub mod anim;
pub mod app_theme;
pub mod apply;
pub mod components;
pub mod icons;
pub mod motion;
pub mod prefs;
pub mod splash;
pub mod style;
pub mod tokens;

use kuro_prefs::ui::ThemeMode;

pub use app_theme::AppTheme;

/// Persisted theme inputs. There is no palette choice: the colours are libadwaita's own, so the only choice is the mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeSettings {
    pub mode: ThemeMode,
}

impl Default for ThemeSettings {
    fn default() -> Self {
        ThemeSettings { mode: ThemeMode::SYSTEM }
    }
}

/// Installs the theme before the first window: the icon bundle, the stored choice (read from the
/// preference store), the stylesheets and the portal follower (T-49a-058).
pub fn startup() {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    icons::register(&display);
    let store = crate::data::app().base.store().clone();
    apply::install(&display, prefs::read_settings(&store));
    follow_mode(&kuro_prefs::ui::UiPrefs::new(store).theme_mode);
    apply::follow_system();
}

/// Applies the theme mode again on every change of `pref`, from anywhere (a backup import, a second window), not
/// only while the Appearance page is open (T-01a-021). The apply is a no-op when the mode is already the applied one.
fn follow_mode(pref: &kuro_prefs::pref::Pref<ThemeMode>) {
    crate::pref_model::bind::follow(pref, || true, crate::pref_model::widgets::theme_mode::apply);
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-01a-021: a change of the stored mode from anywhere is applied, with no Appearance page open.
    #[test]
    fn a_stored_mode_change_is_applied_without_the_appearance_page() {
        crate::testing::with_display(|| {
            let Some(display) = gtk::gdk::Display::default() else { return };
            apply::install(&display, ThemeSettings::default());
            let pref = kuro_prefs::ui::UiPrefs::new(kuro_prefs::store::PrefStore::in_memory()).theme_mode;
            follow_mode(&pref);
            let prefers_dark = || adw::StyleManager::default().is_dark();
            let pump = |ms: u64| {
                let until = std::time::Instant::now() + std::time::Duration::from_millis(ms);
                while std::time::Instant::now() < until {
                    gtk::glib::MainContext::default().iteration(false);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            };
            pref.set(ThemeMode::DARK).unwrap();
            pump(600);
            assert!(prefers_dark());
            pref.set(ThemeMode::LIGHT).unwrap();
            pump(600);
            assert!(!prefers_dark());
        });
    }
}
