//! Theme mode (task T-47a-011), the colours and the cover colours of a title's page. The App theme strip, the app lock
//! and the start screen are cut (settings diet; the app always opens on Home, docs/design/home.md).
//!
//! Theme mode changes the look through one path, `theme::apply` (`update_settings`): the
//! window marks and the GTK dark preference are set together, with no restart.
use kuro_i18n::{MsgKey, tr};
use adw::prelude::*;
use std::rc::Rc;

use kuro_prefs::prefs::Prefs;

use crate::pref_model::model::{Preference, PreferenceItem};
use crate::pref_model::widgets::theme_mode;
use crate::settings_ui::rows::{enum_choice, group, switch_with};

/// The Theme mode row: System, Light and Dark as linked toggle buttons (`widgets::theme_mode`). A click stores the
/// mode and applies it through `theme::apply`; a click on the selected segment writes nothing.
pub fn theme_mode_row(prefs: &Prefs) -> PreferenceItem {
    let (pref, title) = (prefs.ui.theme_mode.clone(), tr(MsgKey::ui_theme_mode));
    PreferenceItem::Custom {
        title: title.to_owned(),
        build: Rc::new(move || {
            let built = theme_mode::row(&crate::pref_model::model::ItemCommon::new(&title), pref.clone());
            built.upcast()
        }),
    }
}

/// The colours of the whole app: the desktop's own, or one of a short list (`theme::colours`). The choice is applied
/// at once, from the stored value (`theme::follow_colours`), in the light and in the dark style.
pub fn colours_row(prefs: &Prefs) -> PreferenceItem {
    use crate::theme::colours;
    let named = colours::ORDER.map(|choice| (choice, colours::name(choice)));
    let options: Vec<_> = named.iter().map(|(choice, name)| (*choice, name.as_str())).collect();
    enum_choice(&tr(MsgKey::ui_app_colours), &prefs.ui.colours, &options, true)
}

/// The switch of the colour a title's page takes from its cover (`theme::tint`). Turning it off takes the colour off
/// the pages that are open; turned on, a page is coloured the next time it is drawn.
pub fn cover_colours_row(prefs: &Prefs) -> PreferenceItem {
    let on_change = crate::pref_model::bind::veto(|on: bool| async move {
        if !on {
            crate::theme::tint::clear_all();
        }
        Ok(true)
    });
    switch_with(&tr(MsgKey::ui_cover_colours_on_a_titles_page), None, &prefs.ui.cover_colours, true, on_change)
}

/// The rows of the Appearance page this module binds to the typed preferences: the Theme group. The rest of the page
/// (the Display group's App language link) stays as the spec builds it.
pub fn page_rows(prefs: &Prefs) -> Vec<Preference> {
    vec![
        group(&tr(MsgKey::pref_category_theme), vec![theme_mode_row(prefs), colours_row(prefs), cover_colours_row(prefs)]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_prefs::ui::ThemeMode;

    fn titles_of(tree: &[Preference], group_title: &str) -> Vec<String> {
        let Some(Preference::Group(found)) = tree.iter().find(|e| matches!(e, Preference::Group(g) if g.title == group_title)) else {
            panic!("the {group_title} group")
        };
        found.items.iter().map(|i| i.title().to_owned()).collect()
    }

    #[test]
    fn the_theme_group_holds_the_mode_the_colours_and_the_cover_colours_switch() {
        let prefs = Prefs::in_memory();
        assert_eq!(titles_of(&page_rows(&prefs), "Theme"), ["Theme mode", "Colours", "Cover colours on a title's page"]);
        let PreferenceItem::BasicList { value, entries, .. } = colours_row(&prefs) else { panic!("a list") };
        assert_eq!(value, "DESKTOP", "the desktop's colours until another choice is made");
        assert_eq!(entries.iter().map(|(_, label)| label.as_str()).collect::<Vec<_>>(), ["Desktop", "Tokyo Night", "Catppuccin", "Dracula", "Nord", "Gruvbox", "Rosé Pine", "Black"]);
        let PreferenceItem::Switch { pref, .. } = cover_colours_row(&prefs) else { panic!("a switch") };
        assert!(pref.get(), "on unless it is turned off");
    }

    #[test]
    fn the_start_screen_is_cut_and_the_display_group_keeps_the_app_language() {
        let prefs = Prefs::in_memory();
        assert_eq!(page_rows(&prefs).len(), 1, "only the Theme group is typed");
        let tree = crate::settings_ui::route::tree_with("settings_appearance", &prefs, &[], &[], crate::settings_ui::browse::RepoCounts::default());
        assert_eq!(titles_of(&tree, "Display"), ["App language"]);
    }

    #[test]
    fn the_merged_page_shows_the_mode_in_the_theme_group() {
        let prefs = Prefs::in_memory();
        prefs.ui.theme_mode.set(ThemeMode::DARK).unwrap();
        let tree = crate::settings_ui::route::tree_with(
            "settings_appearance",
            &prefs,
            &[],
            &[],
            crate::settings_ui::browse::RepoCounts::default(),
        );
        assert_eq!(titles_of(&tree, "Theme"), ["Theme mode", "Colours", "Cover colours on a title's page"]);
    }
}
