//! Installs the two stylesheets on the default display (T-49a-012, T-49b-008, T-49ba-001).
//!
//! The base stylesheet (`style.css`) is added once at APPLICATION priority. The palette is one
//! `CssProvider`, also added once; `reapply` replaces its text and never adds a second provider.
//! Text is parse-checked first, so a parse error keeps the previous text and is logged once (F-49-01).
//! `apply` is not re-entrant (R-49-02): a nested request sets a pending flag and the outer apply
//! runs `apply_current` once more when it finishes, so the latest state is shown.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib::prelude::Cast as _;
use gtk::gio::prelude::ListModelExt as _;
use gtk::prelude::WidgetExt as _;
use gtk::{gdk, CssProvider};

use kuro_prefs::ui::ThemeMode;

use super::{dark, resolve, style, Resolved, ThemeSettings};

thread_local! {
    /// The tint provider and the display it was added to. Added once; only its text changes.
    static TINTS: RefCell<Option<(gdk::Display, CssProvider)>> = const { RefCell::new(None) };
    /// The last parse error logged, so a broken sheet is reported once, not on every apply.
    static LAST_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Re-entrancy state: `applying` while `reapply` runs, `pending` for a nested request.
    static GUARD: Guard = const { Guard { applying: Cell::new(false), pending: Cell::new(false) } };
    /// The display and settings last applied, so a system change can re-apply them.
    static CURRENT: RefCell<Option<(gdk::Display, ThemeSettings)>> = const { RefCell::new(None) };
    /// The last known system dark flag (portal, else GTK fallback). `None` until a read arrives.
    static SYSTEM_DARK: Cell<Option<bool>> = const { Cell::new(None) };
    /// The dark flag last marked on the windows (T-49ba-003), for windows created later.
    static WINDOW_DARK: Cell<bool> = const { Cell::new(false) };
}

/// The re-entrancy guard: one apply at a time, and at most one queued behind it.
struct Guard {
    applying: Cell<bool>,
    pending: Cell<bool>,
}

impl Guard {
    /// True when the caller may run. A nested caller gets false and leaves a pending request.
    fn enter(&self) -> bool {
        if self.applying.replace(true) {
            self.pending.set(true);
            false
        } else {
            true
        }
    }

    /// Ends the outer run. True when a nested request arrived and one more apply is owed.
    fn leave(&self) -> bool {
        self.applying.set(false);
        self.pending.replace(false)
    }
}

/// Parses `css` into a fresh provider. Returns the first parse error, if any.
pub fn build_provider(css: &str) -> Result<CssProvider, String> {
    let provider = CssProvider::new();
    let error: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    {
        let error = error.clone();
        provider.connect_parsing_error(move |_, _section, err| {
            let mut slot = error.borrow_mut();
            if slot.is_none() {
                *slot = Some(err.message().to_string());
            }
        });
    }
    provider.load_from_string(css);
    let first = error.borrow_mut().take();
    match first {
        Some(message) => Err(message),
        None => Ok(provider),
    }
}

/// Replaces the tint text with `css`. Returns false, and keeps the previous text, when the CSS
/// does not parse. A call nested inside another `reapply` returns false and is queued: when the
/// outer call finishes it re-applies the current settings once (R-49-02).
pub fn reapply(display: &gdk::Display, css: &str) -> bool {
    if !GUARD.with(Guard::enter) {
        tracing::debug!("theme apply re-entered; queued");
        return false;
    }
    let ok = set_tints(display, css);
    if GUARD.with(Guard::leave) {
        apply_current();
    }
    ok
}

/// Parse-checks `css`, then loads it into the tint provider. The provider is added to the display
/// the first time only.
fn set_tints(display: &gdk::Display, css: &str) -> bool {
    load_into(&TINTS, display, css)
}

fn load_into(
    slot: &'static std::thread::LocalKey<RefCell<Option<(gdk::Display, CssProvider)>>>,
    display: &gdk::Display,
    css: &str,
) -> bool {
    if let Err(message) = build_provider(css) {
        LAST_ERROR.with(|last| {
            let mut last = last.borrow_mut();
            if last.as_deref() != Some(message.as_str()) {
                tracing::warn!(error = %message, "generated theme CSS did not parse; keeping the previous theme");
                *last = Some(message);
            }
        });
        return false;
    }
    LAST_ERROR.with(|last| *last.borrow_mut() = None);
    slot.with(|slot| {
        let mut slot = slot.borrow_mut();
        let provider = match slot.as_ref() {
            Some((_, provider)) => provider.clone(),
            None => {
                let provider = CssProvider::new();
                gtk::style_context_add_provider_for_display(
                    display,
                    &provider,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
                *slot = Some((display.clone(), provider.clone()));
                provider
            }
        };
        provider.load_from_string(css);
    });
    true
}

/// Installs the palette and the base stylesheet, once, before the first window is presented (T-49ba-001). The palette
/// goes first: the base sheet names `kuro_*` colours that only the palette defines, so they must exist when it is parsed.
pub fn install(display: &gdk::Display, settings: ThemeSettings) -> bool {
    // before the first write, so System can restore what the desktop had
    dark::startup_prefer_dark();
    drop_unreadable_theme(display);
    let applied = set_settings(display, settings);
    let base = CssProvider::new();
    connect_parse_log(&base);
    base.load_from_string(style::BASE_CSS);
    gtk::style_context_add_provider_for_display(display, &base, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    applied
}

/// More parse errors than this means the GTK theme was written for a newer GTK. adw-gtk3 6 (CSS variables) gives about
/// 2900 on GTK 4.14; Tokyonight, which mostly works there, gives 36.
// ponytail: a count, not a check of what the theme paints; replace with a newer bundled GTK once the AppImage moves past 4.14
const UNREADABLE_THEME_ERRORS: u32 = 500;

/// The built-in theme that stands in for an unreadable one, keeping a `-dark` name dark (`dark::gtk_fallback_dark`).
pub fn builtin_theme_for(name: &str) -> &'static str {
    if name.to_ascii_lowercase().ends_with("-dark") {
        "Adwaita-dark"
    } else {
        "Adwaita"
    }
}

/// The `GTK_THEME` value as (name, variant): GTK loads it in place of the theme setting. `None` when unset or empty.
fn env_theme(value: Option<&str>) -> Option<(&str, Option<&str>)> {
    let value = value.filter(|v| !v.is_empty())?;
    Some(match value.split_once(':') {
        Some((name, variant)) => (name, Some(variant)),
        None => (value, None),
    })
}

/// A theme written for GTK 4.16 or newer (CSS variables, as in adw-gtk3 6) barely parses on an older GTK such as the
/// AppImage's 4.14: every background is lost and the window is see-through (issue #5). Such a theme is swapped for the
/// built-in one for this run. A theme forced with `GTK_THEME` wins over the setting, so that one is checked and
/// rewritten instead (issue #6).
fn drop_unreadable_theme(display: &gdk::Display) {
    if gtk::minor_version() >= 16 {
        return;
    }
    let settings = gtk::Settings::for_display(display);
    let env = std::env::var("GTK_THEME").ok();
    let forced = env_theme(env.as_deref());
    let Some(name) = forced.map(|(name, _)| name.into()).or_else(|| settings.gtk_theme_name()) else {
        return;
    };
    let errors = Rc::new(Cell::new(0u32));
    let probe = CssProvider::new();
    let count = errors.clone();
    probe.connect_parsing_error(move |_, _, _| count.set(count.get() + 1));
    probe.load_named(&name, None);
    if errors.get() > UNREADABLE_THEME_ERRORS {
        let builtin = builtin_theme_for(&name);
        tracing::warn!(theme = %name, errors = errors.get(), gtk = gtk::minor_version(), builtin, "GTK theme needs a newer GTK; using the built-in theme");
        if let Some((_, variant)) = forced {
            let value = variant.map_or(builtin.to_owned(), |v| format!("{builtin}:{v}"));
            // GTK reads GTK_THEME again on the theme-name change below.
            // SAFETY: runs once at startup on the main thread, before any screen starts work that reads the environment.
            // ponytail: a race with a getenv on another thread stays possible; check before GTK init if one ever shows up
            unsafe { std::env::set_var("GTK_THEME", value) };
        }
        settings.set_gtk_theme_name(Some(builtin));
    }
}

/// Logs each parse error of a stylesheet with its location (T-49ba-045).
fn connect_parse_log(provider: &CssProvider) {
    provider.connect_parsing_error(|_, section, err| {
        let at = section.start_location();
        tracing::error!(line = at.lines() + 1, column = at.chars() + 1, error = %err.message(), "stylesheet parse error");
    });
}

/// The tint text: the palette colours as `kuro_*` named colours, then the dark-aware `kb-selected` and
/// `kb-active` rules, which cannot live in the static base sheet.
pub fn tint_text(resolved: &Resolved) -> String {
    format!("{}{}", resolved.tokens.define_colors_css(), style::system_rules(resolved.tokens.dark))
}

/// Resolves the settings and installs the tint sheet on `display`. The player state layer (T-49a-010)
/// follows the system dark flag, so its rules ride on the same provider.
pub fn apply(display: &gdk::Display, settings: &ThemeSettings, system_dark: bool) -> bool {
    let resolved = resolve(settings, system_dark);
    let tints = format!("{}{}", tint_text(&resolved), style::widget_rules(system_dark));
    reapply(display, &tints)
}

/// The `gtk-application-prefer-dark-theme` value for a mode: Light and Dark force it, System puts
/// back the value the desktop had at startup (`startup`), so System never forces light.
pub fn gtk_prefer_dark(mode: ThemeMode, startup: bool) -> bool {
    match mode {
        ThemeMode::LIGHT => false,
        ThemeMode::DARK => true,
        ThemeMode::SYSTEM => startup,
    }
}

/// Sets the GTK dark preference for a mode. A no-op without a display.
pub fn sync_gtk_preference(mode: ThemeMode) {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    settings.set_gtk_application_prefer_dark_theme(gtk_prefer_dark(mode, dark::startup_prefer_dark()));
}

pub fn set_settings(display: &gdk::Display, settings: ThemeSettings) -> bool {
    CURRENT.with(|c| *c.borrow_mut() = Some((display.clone(), settings)));
    apply_current()
}

/// Changes part of the applied settings and applies them (palette, window marks, GTK preference). This is
/// the one path every theme-mode change takes; the caller stores the value. A no-op
/// without a display, before `install`, or when nothing changes.
pub fn update_settings(change: impl FnOnce(&mut ThemeSettings)) -> bool {
    let Some((display, mut settings)) = CURRENT.with(|c| c.borrow().clone()) else {
        return false;
    };
    let before = settings;
    change(&mut settings);
    settings == before || set_settings(&display, settings)
}

fn apply_current() -> bool {
    let current = CURRENT.with(|c| c.borrow().clone());
    let Some((display, settings)) = current else {
        return false;
    };
    let system_dark = SYSTEM_DARK.with(Cell::get).unwrap_or(false);
    sync_gtk_preference(settings.mode);
    let dark = resolve(&settings, system_dark).tokens.dark;
    WINDOW_DARK.with(|m| m.set(dark));
    mark_windows();
    apply(&display, &settings, system_dark)
}

/// Sets the palette class on one window: `kb-dark` for the dark flag.
fn mark_window(window: &gtk::Window) {
    if WINDOW_DARK.with(Cell::get) {
        window.add_css_class("kb-dark");
    } else {
        window.remove_css_class("kb-dark");
    }
}

/// Sets the palette classes on every toplevel, so a change applies without a restart.
fn mark_windows() {
    if !gtk::is_initialized() {
        return;
    }
    let windows = gtk::Window::toplevels();
    for i in 0..windows.n_items() {
        if let Some(window) = windows.item(i).and_then(|o| o.downcast::<gtk::Window>().ok()) {
            mark_window(&window);
        }
    }
}

/// Marks a window created after the last apply (called from `mod::attach_window`).
pub fn mark_new_window(window: &gtk::Window) {
    mark_window(window);
}

fn set_system(read: dark::PortalRead) {
    let last = SYSTEM_DARK.with(Cell::get);
    let dark = dark::next_system(read, last, dark::gtk_fallback());
    SYSTEM_DARK.with(|c| c.set(Some(dark)));
    apply_current();
}

/// Starts following the desktop colour scheme (T-49a-011): reads the portal once, then re-applies
/// on each `SettingChanged`. Call once after the window is realized.
pub fn follow_system() {
    dark::read_portal(set_system);
    dark::watch(set_system);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::style::BASE_CSS;
    use kuro_prefs::ui::ThemeMode;

    #[test]
    fn an_unreadable_dark_theme_falls_back_to_the_dark_builtin() {
        assert_eq!(builtin_theme_for("adw-gtk3-dark"), "Adwaita-dark");
        assert_eq!(builtin_theme_for("Adw-Gtk3-Dark"), "Adwaita-dark");
        assert_eq!(builtin_theme_for("adw-gtk3"), "Adwaita");
    }

    #[test]
    fn a_forced_gtk_theme_is_split_into_name_and_variant() {
        assert_eq!(env_theme(None), None);
        assert_eq!(env_theme(Some("")), None);
        assert_eq!(env_theme(Some("adw-gtk3")), Some(("adw-gtk3", None)));
        assert_eq!(env_theme(Some("adw-gtk3:dark")), Some(("adw-gtk3", Some("dark"))));
    }

    // T-01a-021: Light and Dark force the GTK preference, System puts back the startup value.
    #[test]
    fn system_restores_the_startup_value_and_never_forces_light() {
        for startup in [false, true] {
            assert!(!gtk_prefer_dark(ThemeMode::LIGHT, startup));
            assert!(gtk_prefer_dark(ThemeMode::DARK, startup));
            assert_eq!(gtk_prefer_dark(ThemeMode::SYSTEM, startup), startup);
        }
    }

    // T-01a-021: no display, no panic, nothing applied.
    #[test]
    fn a_mode_change_without_an_installed_theme_is_a_no_op() {
        assert!(!update_settings(|s| s.mode = ThemeMode::DARK));
    }

    #[test]
    fn a_nested_request_is_queued_and_run_once() {
        let g = Guard { applying: Cell::new(false), pending: Cell::new(false) };
        assert!(g.enter());
        // Two nested requests collapse into one owed apply.
        assert!(!g.enter());
        assert!(!g.enter());
        assert!(g.leave());
        // The queue is empty afterwards and the guard is free again.
        assert!(g.enter());
        assert!(!g.leave());
    }

    #[test]
    fn no_nested_request_owes_nothing() {
        let g = Guard { applying: Cell::new(false), pending: Cell::new(false) };
        assert!(g.enter());
        assert!(!g.leave());
    }

    /// One tint provider for the life of the process, replaced text on each apply, and a broken sheet keeps
    /// the previous text. Needs a display, and skips itself without one.
    #[test]
    fn tint_provider_keeps_one_provider_and_the_last_good_text() {
        crate::testing::with_display(|| {
            let display = gdk::Display::default().expect("a display");
            assert!(build_provider(BASE_CSS).is_ok());
            let light = tint_text(&resolve(&ThemeSettings::default(), false));
            let dark = tint_text(&resolve(&ThemeSettings { mode: ThemeMode::DARK, ..ThemeSettings::default() }, false));

            assert!(reapply(&display, &light));
            let first = TINTS.with(|slot| slot.borrow().as_ref().map(|(_, p)| p.clone())).expect("tint provider");
            assert!(reapply(&display, &dark));
            let second = TINTS.with(|slot| slot.borrow().as_ref().map(|(_, p)| p.clone())).expect("tint provider");
            assert_eq!(first, second, "the tint provider is added once and reused");
            // GTK stores the parsed sheet, so compare with a reference provider loaded from the same text.
            let reference = CssProvider::new();
            reference.load_from_string(&dark);
            assert_eq!(second.to_str(), reference.to_str(), "the text is replaced");

            assert!(!reapply(&display, "}}}} not css"), "a broken sheet is refused");
            assert_eq!(second.to_str(), reference.to_str(), "the previous text is kept");
        });
    }

    /// Only `theme` installs a stylesheet: the base sheet and the tint provider. A screen that adds its own provider
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
