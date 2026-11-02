//! Tab host rules (Kotlin `HomeScreen`): the start tab and the back rule. The window's places are the panel items
//! (`shell::sidebar`); there is no More tab.
//!
//! The tab kinds and `StartScreen` come from `kuro_prefs::ui`; this module only holds the rules.

use std::cell::RefCell;
use std::rc::Rc;

use kuro_prefs::ui::{StartScreen, Tab};

/// The id of the window's tab page for a tab. Both libraries are one page with an Anime | Manga switcher.
pub fn nav_id(tab: Tab) -> &'static str {
    match tab {
        Tab::AnimeLibrary | Tab::MangaLibrary => "library",
        Tab::Updates => "updates",
        Tab::History => "history",
        Tab::Browse => "browse",
    }
}

/// The tab a page id stands for. `current` decides between the two libraries, so an id that still names the selected
/// library keeps it. Downloads is a page of its own, not a tab the start screen can name.
pub fn tab_for_id(id: &str, current: Tab) -> Option<Tab> {
    Some(match id {
        "library" if current == Tab::MangaLibrary => Tab::MangaLibrary,
        "library" => Tab::AnimeLibrary,
        "updates" => Tab::Updates,
        "history" => Tab::History,
        "browse" => Tab::Browse,
        _ => return None,
    })
}

/// Hosts `home` on the tab pages in `stack`: the stack shows the selected tab now, and every later change of page is
/// recorded in `home`. One `HomeTabs` serves every layout, so a switch between the sidebar and the bottom bar keeps
/// the selection (T-36a-004).
pub fn bind(stack: &adw::ViewStack, home: &Rc<RefCell<HomeTabs>>) {
    let wanted = nav_id(home.borrow().current());
    if stack.child_by_name(wanted).is_some() {
        stack.set_visible_child_name(wanted);
    }
    let home = home.clone();
    stack.connect_visible_child_name_notify(move |stack| {
        let Some(name) = stack.visible_child_name() else { return };
        let current = home.borrow().current();
        if let Some(tab) = tab_for_id(&name, current) {
            home.borrow_mut().select(tab);
        }
    });
}

/// Back or Escape on the root page: returns to the start tab when `home` handles back. Returns whether it did.
pub fn go_back(stack: &adw::ViewStack, home: &Rc<RefCell<HomeTabs>>) -> bool {
    let handled = home.borrow_mut().back();
    if handled {
        stack.set_visible_child_name(nav_id(home.borrow().current()));
    }
    handled
}

/// The selected tab of the home screen. It is kept in memory across layout changes (`HomeTabs` key).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HomeTabs {
    current: Tab,
    default: Tab,
}

impl HomeTabs {
    /// Reads the start tab. Built once per Home build, so a changed start screen applies there.
    pub fn new(start: StartScreen) -> Self {
        let default = start.tab();
        Self { current: default, default }
    }

    pub fn current(&self) -> Tab {
        self.current
    }

    pub fn select(&mut self, tab: Tab) {
        self.current = tab;
    }

    /// Back is the shell's on every tab but the start tab.
    pub fn back_enabled(&self) -> bool {
        self.current != self.default
    }

    /// Back or Escape: returns to the start tab when back is enabled. Returns whether it did.
    pub fn back(&mut self) -> bool {
        if !self.back_enabled() {
            return false;
        }
        self.current = self.default;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLACES: [Tab; 5] = [Tab::AnimeLibrary, Tab::MangaLibrary, Tab::Updates, Tab::History, Tab::Browse];

    #[test]
    fn home_tabs_keep_the_selection_and_go_back_to_start() {
        let mut tabs = HomeTabs::new(StartScreen::MANGA);
        assert_eq!(tabs.current(), Tab::MangaLibrary);
        assert!(!tabs.back_enabled(), "back on the start tab is not the shell's");
        tabs.select(Tab::Browse);
        assert!(tabs.back());
        assert_eq!(tabs.current(), Tab::MangaLibrary);
        assert!(!tabs.back(), "back on the start tab is not handled");
    }

    #[test]
    fn back_from_any_tab_returns_to_a_history_start_tab() {
        let mut tabs = HomeTabs::new(StartScreen::HISTORY);
        tabs.select(Tab::Updates);
        assert!(tabs.back());
        assert_eq!(tabs.current(), Tab::History);
    }

    #[test]
    fn every_place_has_a_page_in_the_spec_and_ids_map_back() {
        let ids: Vec<String> = crate::ui::with_spec(|s| s.nav.iter().map(|n| n.id.clone()).collect());
        for tab in PLACES {
            let id = nav_id(tab);
            assert!(ids.iter().any(|i| i == id), "{tab:?} has no page {id}");
            assert_eq!(tab_for_id(id, tab), Some(tab), "{tab:?}");
        }
        assert_eq!(tab_for_id("library", Tab::Updates), Some(Tab::AnimeLibrary));
        assert_eq!(tab_for_id("settings", Tab::Updates), None);
        assert_eq!(tab_for_id(crate::shell::sidebar::DOWNLOADS, Tab::Updates), None, "Downloads leaves the selection alone");
    }

    #[test]
    fn the_places_are_library_browse_updates_history_and_downloads_with_no_more() {
        let ids: Vec<String> = crate::ui::with_spec(|s| s.nav.iter().map(|n| n.id.clone()).collect());
        assert_eq!(ids, ["library", "browse", "updates", "history", crate::shell::sidebar::DOWNLOADS]);
    }
}
