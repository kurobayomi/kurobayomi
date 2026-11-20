//! The colour a title's page takes from its cover. The page (its `adw::NavigationPage`) gets a class of its own and a
//! small style sheet of rules under that class, so its accents (the action row, the unread dots and bookmarks, the
//! Resume button, links, and the switches and bars of the page) take the cover's colour and nothing outside the page
//! changes. The sheet goes with the page.
//!
//! The rules name every selector by hand: the app runs on GTK 4.14, where a redeclared colour applies to the whole
//! window (colour variables a widget can carry came with 4.16). The colour is made readable against the window's
//! background of the moment (`palette::readable`), and made again when the app goes from light to dark.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use gtk::prelude::*;
use gtk::{CssProvider, gdk};

use super::palette::{self, Rgb};

/// libadwaita's window backgrounds, light and dark (its `window_bg_color`): what a page's colour must read on.
pub const LIGHT_WINDOW: Rgb = Rgb::new(0xfa, 0xfa, 0xfa);
pub const DARK_WINDOW: Rgb = Rgb::new(0x24, 0x24, 0x24);

/// The class of a tinted page starts with this; a number makes it that page's own.
const CLASS_PREFIX: &str = "kb-tint-";

/// The style rules of one tinted page: `seed` made readable on `background` for what is drawn in the colour, and black
/// or white for what is written on it. Every rule is under `.class`, so it reaches that page and no other.
pub fn rules(class: &str, seed: Rgb, background: Rgb) -> String {
    let accent = palette::readable(seed, background);
    let (colour, written) = (accent.hex(), palette::on(accent).hex());
    [
        // text and icons in the accent: the action row, the unread dot and the bookmark of a row, links
        format!(".{class} .accent {{ color: {colour}; }}"),
        format!(".{class} link {{ color: {colour}; }}"),
        // filled in the accent: the Resume button, and the controls of the page
        format!(".{class} button.suggested-action {{ background-color: {colour}; color: {written}; }}"),
        format!(".{class} switch:checked {{ background-color: {colour}; }}"),
        format!(".{class} check:checked, .{class} radio:checked {{ background-color: {colour}; color: {written}; }}"),
        format!(".{class} progressbar > trough > progress {{ background-color: {colour}; }}"),
        format!(".{class} scale > trough > highlight {{ background-color: {colour}; }}"),
    ]
    .join("\n")
}

/// The sheet of one tinted page.
struct Tinted {
    provider: CssProvider,
    display: gdk::Display,
    /// The cover's colour, kept to make the rules again when the window's background changes.
    seed: Rgb,
}

thread_local! {
    /// The tinted pages by their class.
    static PAGES: RefCell<HashMap<String, Tinted>> = RefCell::default();
    static NEXT: Cell<u32> = const { Cell::new(0) };
    static WATCHING: Cell<bool> = const { Cell::new(false) };
}

/// Whether a title's page takes its cover's colour: the switch of the Appearance page (`entry_cover_colours`), on
/// unless it is turned off. On before the app's data exists (the tests).
fn enabled() -> bool {
    !crate::data::is_installed() || kuro_prefs::ui::UiPrefs::new(crate::data::app().base.store().clone()).cover_colours.get()
}

/// The window's background now: libadwaita's dark one in the dark style, its light one otherwise.
fn background() -> Rgb {
    if adw::is_initialized() && adw::StyleManager::default().is_dark() { DARK_WINDOW } else { LIGHT_WINDOW }
}

fn class_of(page: &gtk::Widget) -> Option<String> {
    page.css_classes().iter().find(|class| class.starts_with(CLASS_PREFIX)).map(|class| class.to_string())
}

/// Tints `page` and everything in it with the colour `seed` of its cover. A page already tinted takes the new colour.
/// GTK main thread.
pub fn apply(page: &impl IsA<gtk::Widget>, seed: Rgb) {
    let page = page.upcast_ref::<gtk::Widget>();
    if !enabled() {
        return clear(page);
    }
    follow_the_mode();
    if let Some(class) = class_of(page) {
        let retinted = PAGES.with(|pages| match pages.borrow_mut().get_mut(&class) {
            Some(tinted) => {
                tinted.seed = seed;
                tinted.provider.load_from_string(&rules(&class, seed, background()));
                true
            }
            None => false,
        });
        if retinted {
            return;
        }
        // its sheet is gone (the switch was turned off and on again): the page starts over with a new one
        page.remove_css_class(&class);
    }
    let number = NEXT.with(|next| next.replace(next.get().wrapping_add(1)));
    let class = format!("{CLASS_PREFIX}{number}");
    let provider = CssProvider::new();
    provider.load_from_string(&rules(&class, seed, background()));
    let display = page.display();
    // above the app's own sheet, which is at the application's priority
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    page.add_css_class(&class);
    PAGES.with(|pages| pages.borrow_mut().insert(class.clone(), Tinted { provider, display, seed }));
    // the page's rules go with the page
    page.connect_destroy(move |_| forget(&class));
}

/// Takes the tint off `page`: its cover has no colour to speak of. A page that has none is left as it is.
pub fn clear(page: &impl IsA<gtk::Widget>) {
    let page = page.upcast_ref::<gtk::Widget>();
    if let Some(class) = class_of(page) {
        page.remove_css_class(&class);
        forget(&class);
    }
}

/// Takes the tint off every page: the switch was turned off. A page keeps its class, which no rule answers to any more.
pub fn clear_all() {
    let classes: Vec<String> = PAGES.with(|pages| pages.borrow().keys().cloned().collect());
    for class in classes {
        forget(&class);
    }
}

fn forget(class: &str) {
    let tinted = PAGES.with(|pages| pages.borrow_mut().remove(class));
    if let Some(tinted) = tinted {
        gtk::style_context_remove_provider_for_display(&tinted.display, &tinted.provider);
    }
}

/// Makes the rules of every tinted page again when the app goes from light to dark or back. Connected once.
fn follow_the_mode() {
    if !adw::is_initialized() || WATCHING.with(|watching| watching.replace(true)) {
        return;
    }
    adw::StyleManager::default().connect_dark_notify(|_| {
        let background = background();
        PAGES.with(|pages| {
            for (class, tinted) in pages.borrow().iter() {
                tinted.provider.load_from_string(&rules(class, tinted.seed, background));
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_of_a_page_is_under_that_pages_class_and_names_no_colour_of_the_theme() {
        let sheet = rules("kb-tint-7", Rgb::new(47, 95, 208), DARK_WINDOW);
        assert!(!sheet.is_empty());
        for rule in sheet.lines() {
            assert!(rule.starts_with(".kb-tint-7 "), "{rule}");
            assert!(!rule.contains('@'), "its colours are written out: {rule}");
        }
        assert!(sheet.contains(".kb-tint-7 .accent {"));
        assert!(sheet.contains(".kb-tint-7 button.suggested-action {"));
        // a selector list keeps the class on each of its selectors
        assert!(sheet.contains(".kb-tint-7 check:checked, .kb-tint-7 radio:checked {"));
    }

    #[test]
    fn the_colour_of_the_rules_reads_on_the_window_and_what_is_written_on_it_reads_too() {
        for (seed, window) in [
            (Rgb::new(20, 30, 90), DARK_WINDOW),
            (Rgb::new(255, 220, 0), LIGHT_WINDOW),
            (Rgb::new(111, 160, 255), DARK_WINDOW),
            (Rgb::new(240, 111, 176), LIGHT_WINDOW),
        ] {
            let accent = palette::readable(seed, window);
            let sheet = rules("kb-tint-1", seed, window);
            assert!(sheet.contains(&format!(".kb-tint-1 .accent {{ color: {}; }}", accent.hex())), "{sheet}");
            assert!(accent.contrast(window) >= palette::TEXT_CONTRAST, "{seed:?} on {window:?}");
            let written = palette::on(accent);
            assert!(written.contrast(accent) >= palette::TEXT_CONTRAST, "the label of the Resume button: {seed:?}");
            assert!(sheet.contains(&format!("background-color: {}; color: {}; }}", accent.hex(), written.hex())));
        }
    }

    #[test]
    fn the_window_backgrounds_are_libadwaitas() {
        assert_eq!((LIGHT_WINDOW.hex(), DARK_WINDOW.hex()), ("#fafafa".to_owned(), "#242424".to_owned()));
    }
}
