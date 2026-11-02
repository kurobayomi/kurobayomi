//! Installs the base stylesheet (`style.css`) on the default display once, at APPLICATION priority (T-49ba-001), and sets
//! libadwaita's colour scheme from the stored mode. libadwaita follows the desktop's light or dark choice itself.

use std::cell::RefCell;

use gtk::{gdk, CssProvider};

use kuro_prefs::ui::ThemeMode;

use super::{style, ThemeSettings};

thread_local! {
    /// The display and settings last applied, so a later change can update them.
    static CURRENT: RefCell<Option<(gdk::Display, ThemeSettings)>> = const { RefCell::new(None) };
}

/// Installs the base stylesheet and applies `settings`, once, before the first window is presented (T-49ba-001).
pub fn install(display: &gdk::Display, settings: ThemeSettings) -> bool {
    let applied = set_settings(display, settings);
    let base = CssProvider::new();
    connect_parse_log(&base);
    base.load_from_string(style::BASE_CSS);
    gtk::style_context_add_provider_for_display(display, &base, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    applied
}

/// Logs each parse error of a stylesheet with its location (T-49ba-045).
fn connect_parse_log(provider: &CssProvider) {
    provider.connect_parsing_error(|_, section, err| {
        let at = section.start_location();
        tracing::error!(line = at.lines() + 1, column = at.chars() + 1, error = %err.message(), "stylesheet parse error");
    });
}

/// The libadwaita colour scheme for a mode: System follows the desktop, Light and Dark force it.
pub fn color_scheme(mode: ThemeMode) -> adw::ColorScheme {
    match mode {
        ThemeMode::LIGHT => adw::ColorScheme::ForceLight,
        ThemeMode::DARK => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::Default,
    }
}

/// Sets libadwaita's colour scheme for a mode. A no-op before libadwaita is initialised.
pub fn sync_color_scheme(mode: ThemeMode) {
    if adw::is_initialized() {
        adw::StyleManager::default().set_color_scheme(color_scheme(mode));
    }
}

pub fn set_settings(display: &gdk::Display, settings: ThemeSettings) -> bool {
    CURRENT.with(|c| *c.borrow_mut() = Some((display.clone(), settings)));
    sync_color_scheme(settings.mode);
    true
}

/// Changes part of the applied settings and applies them. This is the one path every theme-mode change takes; the caller
/// stores the value. A no-op without a display, before `install`, or when nothing changes.
pub fn update_settings(change: impl FnOnce(&mut ThemeSettings)) -> bool {
    let Some((display, mut settings)) = CURRENT.with(|c| c.borrow().clone()) else {
        return false;
    };
    let before = settings;
    change(&mut settings);
    settings == before || set_settings(&display, settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-01a-021: Light and Dark force the scheme, System follows the desktop.
    #[test]
    fn each_mode_maps_to_a_colour_scheme() {
        assert_eq!(color_scheme(ThemeMode::LIGHT), adw::ColorScheme::ForceLight);
        assert_eq!(color_scheme(ThemeMode::DARK), adw::ColorScheme::ForceDark);
        assert_eq!(color_scheme(ThemeMode::SYSTEM), adw::ColorScheme::Default);
    }

    // T-01a-021: no display, no panic, nothing applied.
    #[test]
    fn a_mode_change_without_an_installed_theme_is_a_no_op() {
        assert!(!update_settings(|s| s.mode = ThemeMode::DARK));
    }

    /// Only `theme` installs a stylesheet: the base sheet. A screen that adds its own provider
    /// brings back the per-screen sheets this module replaced. Reads the source files, so it needs no display.
    #[test]
    fn no_screen_adds_a_css_provider() {
        fn rust_files(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("a source directory") {
                let path = entry.expect("a directory entry").path();
                if path.is_dir() {
                    rust_files(&path, found);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    found.push(path);
                }
            }
        }
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        let offenders: Vec<_> = files
            .into_iter()
            .filter(|file| !file.starts_with(src.join("theme")))
            .filter(|file| {
                let text = std::fs::read_to_string(file).expect("a readable source file");
                text.contains("style_context_add_provider_for_display")
            })
            .collect();
        assert!(offenders.is_empty(), "install stylesheets through theme::apply, not here: {offenders:?}");
    }
}
