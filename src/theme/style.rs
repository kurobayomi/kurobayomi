//! The base stylesheet of the app: the few rules for its own components that libadwaita has no class for.

/// The base stylesheet, loaded once at APPLICATION priority.
pub const BASE_CSS: &str = include_str!("style.css");

#[cfg(test)]
mod tests {
    use super::BASE_CSS;

    #[test]
    fn base_stylesheet_braces_balance() {
        assert_eq!(BASE_CSS.matches('{').count(), BASE_CSS.matches('}').count());
    }

    /// T-49ba-045: the base sheet names only GTK named colours, so no rule refers to a colour that does not exist.
    #[test]
    fn base_sheet_references_only_gtk_named_colours() {
        // the GTK named colours: GTK themes define them, and so does libadwaita (its GTK 3 compatibility names), so they
        // resolve under libadwaita's own stylesheet and under distributions whose libadwaita keeps the GTK theme
        const GTK: [&str; 5] = ["theme_bg_color", "theme_fg_color", "theme_selected_bg_color", "theme_selected_fg_color", "borders"];
        let mut rest = BASE_CSS;
        while let Some(at) = rest.find('@') {
            let name: String = rest[at + 1..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            // an at-rule, not a colour: the refresh arrow's animation
            if name == "keyframes" {
                rest = &rest[at + 1..];
                continue;
            }
            assert!(GTK.contains(&name.as_str()), "undefined colour @{name} in the base sheet");
            rest = &rest[at + 1..];
        }
    }

    /// The base sheet parses under `gtk::CssProvider`. Needs a display, and skips itself without one.
    #[test]
    fn base_sheet_parses() {
        crate::testing::with_display(|| {
            let provider = gtk::CssProvider::new();
            let failed = std::rc::Rc::new(std::cell::Cell::new(false));
            let flag = std::rc::Rc::clone(&failed);
            provider.connect_parsing_error(move |_, _, _| flag.set(true));
            provider.load_from_string(BASE_CSS);
            assert!(!failed.get(), "style.css did not parse");
        });
    }
}
