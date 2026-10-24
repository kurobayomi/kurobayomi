//! Theme mode (task T-47a-011) and the start screen (T-48a-005). The App theme strip and the app lock are cut (settings
//! diet).
//!
//! Theme mode changes the look through one path, `theme::apply` (`update_settings`): the
//! window marks and the GTK dark preference are set together, with no restart.
use kuro_i18n::{MsgKey, tr};
use std::rc::Rc;

use kuro_prefs::prefs::Prefs;
use kuro_prefs::ui::StartScreen;

use crate::pref_model::model::{Preference, PreferenceItem};
use crate::pref_model::widgets::theme_mode;
use crate::settings_ui::rows::{enum_choice, group, int_choice, requires_restart};

/// The Theme mode row: System, Light and Dark as linked toggle buttons (`widgets::theme_mode`). A click stores the
/// mode and applies it through `theme::apply`; a click on the selected segment writes nothing.
pub fn theme_mode_row(prefs: &Prefs) -> PreferenceItem {
    let (pref, title) = (prefs.ui.theme_mode.clone(), tr(MsgKey::ui_theme_mode));
    PreferenceItem::Custom {
        title: title.to_owned(),
        build: Rc::new(move || {
            let built = theme_mode::row(&crate::pref_model::model::ItemCommon::new(&title), pref.clone());
            custom_content(built.row)
        }),
    }
}

/// The content of a built list row, taken out of it so it can be a `Custom` row's widget.
fn custom_content(row: gtk::ListBoxRow) -> gtk::Widget {
    use gtk::prelude::*;
    let content = row.child().expect("the row content");
    row.set_child(None::<&gtk::Widget>);
    content
}

/// The five start screens in list order, with their titles.
pub const START_SCREENS: [(StartScreen, &str); 5] = [
    (StartScreen::ANIME, "Anime"),
    (StartScreen::MANGA, "Manga"),
    (StartScreen::UPDATES, "Updates"),
    (StartScreen::HISTORY, "History"),
    (StartScreen::BROWSE, "Browse"),
];

/// The start screen list. It applies at the next launch, so its subtitle says so (task T-47a-010).
pub fn start_screen_row(prefs: &Prefs) -> PreferenceItem {
    let mut row = enum_choice(&tr(MsgKey::pref_start_screen), &prefs.ui.start_screen, &START_SCREENS, true);
    if let PreferenceItem::BasicList { common, .. } = &mut row {
        common.subtitle = Some(requires_restart());
    }
    row
}

/// The cover size of the Library and Browse grids: Small, Medium or Large (`kuro_domain::library::columns::column_width`).
/// The grids follow a change at once; the column count still follows the window.
pub fn cover_size_row(prefs: &Prefs) -> PreferenceItem {
    let options = [
        (0, tr(MsgKey::cover_size_small)),
        (1, tr(MsgKey::cover_size_medium)),
        (2, tr(MsgKey::cover_size_large)),
    ];
    int_choice(&tr(MsgKey::pref_cover_size), &prefs.library.cover_size, &options, true)
}

/// The nav destination a start screen opens: its tab id in the app window and, for the two libraries, the page of the
/// Anime | Manga switcher inside it.
pub fn nav_target(screen: StartScreen) -> (&'static str, Option<&'static str>) {
    match screen {
        StartScreen::ANIME => ("library", Some("anime")),
        StartScreen::MANGA => ("library", Some("manga")),
        StartScreen::UPDATES => ("updates", None),
        StartScreen::HISTORY => ("history", None),
        StartScreen::BROWSE => ("browse", None),
    }
}

/// Selects the tab `prefs.ui.start_screen` names (task T-48a-005). A tab the window does not have is left alone.
pub fn open_start_tab(tabs: &gtk::Stack, prefs: &Prefs) {
    let (tab, kind) = nav_target(prefs.ui.start_screen.get());
    let Some(page) = tabs.child_by_name(tab) else { return };
    tabs.set_visible_child(&page);
    if let Some(kind) = kind {
        crate::settings_ui::view::show_kind(&page, kind);
    }
}

/// The rows of the Appearance page this module binds to the typed preferences: Theme mode, the Start screen, and the
/// app lock group. The rest of the page stays as the spec builds it.
pub fn page_rows(prefs: &Prefs) -> Vec<Preference> {
    vec![
        group(&tr(MsgKey::pref_category_theme), vec![theme_mode_row(prefs)]),
        group(&tr(MsgKey::pref_category_display), vec![start_screen_row(prefs), cover_size_row(prefs)]),
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
    fn the_theme_group_holds_only_the_mode() {
        let prefs = Prefs::in_memory();
        assert_eq!(titles_of(&page_rows(&prefs), "Theme"), ["Theme mode"]);
    }

    #[test]
    fn the_display_group_holds_the_start_screen_and_the_cover_size() {
        let prefs = Prefs::in_memory();
        assert_eq!(titles_of(&page_rows(&prefs), "Display"), ["Start Screen", "Cover size"]);
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
        assert_eq!(titles_of(&tree, "Theme"), ["Theme mode"]);
    }

    #[test]
    fn each_start_screen_maps_to_a_nav_tab_and_the_libraries_pick_their_kind() {
        assert_eq!(nav_target(StartScreen::ANIME), ("library", Some("anime")));
        assert_eq!(nav_target(StartScreen::MANGA), ("library", Some("manga")));
        assert_eq!(nav_target(StartScreen::UPDATES), ("updates", None));
        assert_eq!(nav_target(StartScreen::HISTORY), ("history", None));
        assert_eq!(nav_target(StartScreen::BROWSE), ("browse", None));
        crate::ui::with_spec(|spec| {
            for (screen, _) in START_SCREENS {
                let (tab, _) = nav_target(screen);
                assert!(spec.nav.iter().any(|n| n.id == tab), "{tab} is a nav destination");
            }
        });
        let tabs: Vec<_> = START_SCREENS.iter().map(|(s, _)| s.tab()).collect();
        assert_eq!(tabs.len(), 5, "StartScreen::tab names a destination for every start screen");
    }

    #[test]
    fn the_page_rows_have_the_spec_titles_and_the_start_screen_reads_its_preference() {
        let prefs = Prefs::in_memory();
        prefs.ui.start_screen.set(StartScreen::UPDATES).unwrap();
        let tree = page_rows(&prefs);
        let Preference::Group(display) = &tree[1] else { panic!("group") };
        assert_eq!(display.title, "Display");
        let PreferenceItem::BasicList { common, value, .. } = &display.items[0] else { panic!("list") };
        assert_eq!((common.title.as_str(), value.as_str()), ("Start Screen", "UPDATES"));
        crate::ui::with_spec(|spec| {
            let screen = spec.screen("settings_appearance").expect("screen");
            let has = |group: &str, title: &str| {
                screen.groups.iter().any(|g| {
                    g.title.as_deref() == Some(group) && g.items.iter().any(|i| crate::pref_model::spec_convert::item_title(i) == title)
                })
            };
            assert!(has("Display", "Start Screen"));
        });
    }

    #[test]
    fn start_screen_labels_match_the_spec_and_default_is_anime() {
        let labels: Vec<&str> = START_SCREENS.iter().map(|(_, l)| *l).collect();
        assert_eq!(labels, ["Anime", "Manga", "Updates", "History", "Browse"]);
        let prefs = Prefs::in_memory();
        let PreferenceItem::BasicList { value, common, .. } = start_screen_row(&prefs) else { panic!("list") };
        assert_eq!(value, "ANIME");
        assert_eq!(common.subtitle.as_deref(), Some(requires_restart().as_str()));
    }
}
