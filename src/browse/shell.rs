//! The Browse tab: the anime and manga source lists behind one header row, with the global search and the header
//! actions (T-40a-001, docs/design/other-tabs.md). The tab has no extensions or migrate pages: a source comes with its
//! repository link, and the library is the only place that migrates.
//!
//! The header has the Anime | Manga switch at the start and the search toggle, the view options (the languages) and the menu at the end; the
//! search bar (`ui::search_row`) sits under it. Enter in the search entry runs a global search of the visible kind, and so does re-selecting the tab.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use kuro_i18n::{MsgKey, tr};

use super::glue::keep_alive;
use crate::widgets::view_options::{Item, Pick, Section, ViewOptions};
use super::keys::{BrowseShortcut, EscapeStep, browse_shortcut, escape_step};
use super::route::{BrowsePage, BrowseRoute};
use kuro_platform::kind::Kind;

/// What the tab needs from the app.
pub struct ShellDeps {
    /// The two source pages, anime then manga (`BrowsePage::AnimeSources`, `BrowsePage::MangaSources`).
    pub pages: Vec<(BrowsePage, gtk::Widget)>,
    /// A route to open: global search from the entry, or from re-selecting the tab.
    pub on_route: Rc<dyn Fn(BrowseRoute)>,
    /// The languages of a kind's installed sources for the view-options menu: `(code, name, shown)`.
    pub languages: Rc<dyn Fn(Kind) -> Vec<(String, String, bool)>>,
    /// A language of the menu was switched on or off.
    pub on_language: Rc<dyn Fn(String, bool)>,
    /// The menu's repository entry, for the visible kind: where source repository links are added.
    pub on_repos: Rc<dyn Fn(Kind)>,
    /// The refresh button: fetches the repositories again for new and updated sources. It gets the button, to turn it
    /// off while that runs.
    pub on_refresh: Rc<dyn Fn(&gtk::Button)>,
    /// Escape with an empty search: back out of the Browse page.
    pub on_leave: Rc<dyn Fn()>,
}

/// The pages of the tab, in tab order: anime (switch 0) and manga (switch 1).
const SOURCE_PAGES: [BrowsePage; 2] = [BrowsePage::AnimeSources, BrowsePage::MangaSources];

struct Inner {
    stack: gtk::Stack,
    search: gtk::SearchEntry,
    /// The app's shared view-options menu, with the languages of the visible kind.
    options: ViewOptions,
    deps: ShellDeps,
}

/// The menu action of a language. The menu names actions with `&'static str`, so each code is leaked once.
/// ponytail: one small string per language code ever listed, a few dozen at most.
fn lang_action(lang: &str) -> &'static str {
    thread_local! {
        static NAMES: std::cell::RefCell<std::collections::HashMap<String, &'static str>> = std::cell::RefCell::default();
    }
    NAMES.with(|names| *names.borrow_mut().entry(lang.to_owned()).or_insert_with(|| Box::leak(format!("{LANG}{lang}").into_boxed_str())))
}

/// The prefix of the language actions.
const LANG: &str = "lang-";

#[allow(dead_code)] // kept for the shape of the type; not built or read yet
/// The tab shell. `root` is the whole page, header included; `end` stays empty because the header is in `root`.
pub struct BrowseShell {
    pub root: gtk::Box,
    pub end: Vec<gtk::Widget>,
    inner: Rc<Inner>,
}

impl BrowseShell {
    pub fn new(deps: ShellDeps) -> Self {
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_vexpand(true);
        let mut pages = deps.pages.iter().map(|(page, widget)| (*page, widget.clone())).collect::<Vec<_>>();
        pages.sort_by_key(|(page, _)| page.index());
        for (page, widget) in &pages {
            let title = tr(if page.kind() == Kind::Manga { MsgKey::label_manga } else { MsgKey::label_anime });
            stack.add_titled(widget, Some(&page.index().to_string()), &title);
        }

        let options = ViewOptions::new(&tr(MsgKey::ui_view_options), &[]);
        let inner = Rc::new(Inner { stack, search: gtk::SearchEntry::new(), options, deps });
        // the shared header row: the Anime | Manga switch at the start, the search toggle and the actions at the end, the bar under it
        let options = inner.header_actions();
        let on_refresh = Rc::clone(&inner.deps.on_refresh);
        let refresh = crate::ui::refresh_button(&tr(MsgKey::browse_check_sources));
        refresh.connect_clicked(move |button| on_refresh(button));
        // search, Refresh, then the page's own buttons, as on every page (user, 2026-10-25): Repositories first of them
        let end = crate::ui::HeaderActions {
            search: Some(inner.search.clone()),
            refresh: Some(refresh.clone().upcast()),
            extra: vec![inner.repos_button().upcast(), gtk::StackSwitcher::builder().stack(&inner.stack).build().upcast()],
            options: Some(options),
            ..Default::default()
        }
        .widgets();
        let root = crate::ui::page("", &inner.stack, end, false);
        keep_alive(&root, &inner);

        let weak = Rc::downgrade(&inner);
        inner.stack.connect_visible_child_name_notify(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.page_changed();
            }
        });
        // after the handler above, so a kept Manga choice loads its page
        crate::shell::kind_choice::follow(&inner.stack, ["0", "1"]);
        // Enter searches every source of the visible kind (the global search is the way in, not a filter of the list)
        let (weak, search) = (Rc::downgrade(&inner), inner.search.clone());
        search.connect_activate(move |entry| {
            if let Some(inner) = weak.upgrade() {
                inner.search_globally(&entry.text());
            }
        });
        // Escape and Ctrl+1 and Ctrl+2 reach the shell from anywhere on the tab (T-40a-035); Ctrl+F is the shared search's
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&inner);
        keys.connect_key_pressed(move |_, key, _, modifiers| match weak.upgrade() {
            Some(inner) if inner.key(key, modifiers) => glib::Propagation::Stop,
            _ => glib::Propagation::Proceed,
        });
        root.add_controller(keys);
        inner.page_changed();
        Self { root, end: Vec::new(), inner }
    }

    #[cfg(test)]
    /// The tab that is showing.
    pub fn visible(&self) -> BrowsePage {
        self.inner.visible()
    }
}

impl Inner {
    fn visible(&self) -> BrowsePage {
        self.stack
            .visible_child_name()
            .and_then(|name| name.parse::<usize>().ok())
            .and_then(|index| BrowsePage::ALL.get(index).copied())
            .unwrap_or(BrowsePage::AnimeSources)
    }

    /// One key of the tab. True when it was ours.
    fn key(&self, key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
        if key == gtk::gdk::Key::Escape {
            return match escape_step(self.search_open()) {
                EscapeStep::CloseSearch => {
                    self.close_search();
                    true
                }
                EscapeStep::Leave => {
                    (self.deps.on_leave)();
                    true
                }
            };
        }
        match browse_shortcut(key, modifiers) {
            Some(BrowseShortcut::SubTab(n)) if (1..=2).contains(&n) => {
                self.select(SOURCE_PAGES[usize::from(n) - 1]);
                true
            }
            _ => false,
        }
    }

    /// Whether the shared search bar (`ui::search_row`, built by `ui::page`) is showing.
    fn search_open(&self) -> bool {
        self.search.ancestor(gtk::SearchBar::static_type()).and_downcast::<gtk::SearchBar>().is_some_and(|bar| bar.is_search_mode())
    }

    /// Escape in the entry: closes the bar and clears the text.
    fn close_search(&self) {
        self.search.set_text("");
        self.search.emit_by_name::<()>("stop-search", &[]);
    }

    fn select(&self, page: BrowsePage) {
        self.stack.set_visible_child_name(&page.index().to_string());
    }

    /// Follows the page now showing: the entry's hint and the menu's languages.
    fn page_changed(&self) {
        self.search.set_placeholder_text(Some(&tr(MsgKey::action_global_search_hint)));
        self.refresh_languages();
    }

    /// Rebuilds the menu from the visible kind's sources: one check per language, on while its sources show.
    fn refresh_languages(&self) {
        let languages = (self.deps.languages)(self.visible().kind());
        let items = languages.iter().map(|(code, name, _)| Item::toggle(lang_action(code), name.clone())).collect();
        self.options.set_sections(&[Section::new(tr(MsgKey::ext_info_language), items)]);
        for (code, _, shown) in &languages {
            self.options.set_toggle(lang_action(code), *shown);
        }
    }

    /// Enter in the entry: a global search of the visible kind for the text.
    fn search_globally(&self, text: &str) {
        (self.deps.on_route)(BrowseRoute::GlobalSearch { kind: self.visible().kind(), source_id: None, query: text.trim().to_owned() });
        self.close_search();
    }

    /// The view-options menu of the header (the shared row is `ui::HeaderActions`).
    fn header_actions(self: &Rc<Self>) -> gtk::Widget {
        let on_language = Rc::clone(&self.deps.on_language);
        self.options.connect(move |pick| {
            if let Pick::Toggle(name, on) = pick
                && let Some(code) = name.strip_prefix(LANG)
            {
                on_language(code.to_owned(), on);
            }
        });
        // the sources may have loaded since the page was shown: the menu is read again as it opens
        let opening = gtk::GestureClick::new();
        opening.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        opening.connect_pressed(move |_, _, _, _| {
            if let Some(inner) = weak.upgrade() {
                inner.refresh_languages();
            }
        });
        self.options.widget.add_controller(opening);

        // no "more" menu: the search button is the global search, and Repositories is a button of its own
        self.options.widget.clone().upcast()
    }

    /// The Repositories button, after Refresh: opens the repository list for the visible kind.
    fn repos_button(self: &Rc<Self>) -> gtk::Button {
        let button = crate::ui::icon_button("package-x-generic-symbolic", &tr(MsgKey::label_extension_repos));
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                (inner.deps.on_repos)(inner.visible().kind());
            }
        });
        button
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;

    use super::*;

    struct Log {
        routes: Rc<RefCell<Vec<BrowseRoute>>>,
        left: Rc<RefCell<u32>>,
        repos: Rc<RefCell<Vec<Kind>>>,
        langs: Rc<RefCell<Vec<(String, bool)>>>,
    }

    fn shell() -> (BrowseShell, Log) {
        let log = Log { routes: Rc::default(), left: Rc::default(), repos: Rc::default(), langs: Rc::default() };
        let (routes, left, repos, langs) = (Rc::clone(&log.routes), Rc::clone(&log.left), Rc::clone(&log.repos), Rc::clone(&log.langs));
        let pages = SOURCE_PAGES.iter().map(|p| (*p, gtk::Label::new(Some(&p.title())).upcast::<gtk::Widget>())).collect();
        let deps = ShellDeps {
            pages,
            on_route: Rc::new(move |route| routes.borrow_mut().push(route)),
            languages: Rc::new(|_| vec![("en".to_owned(), "English".to_owned(), true), ("fr".to_owned(), "French".to_owned(), false)]),
            on_language: Rc::new(move |lang, on| langs.borrow_mut().push((lang, on))),
            on_repos: Rc::new(move |kind| repos.borrow_mut().push(kind)),
            on_refresh: Rc::new(|_| {}),
            on_leave: Rc::new(move || *left.borrow_mut() += 1),
        };
        (BrowseShell::new(deps), log)
    }

    fn walk(w: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
        let mut child = w.first_child();
        while let Some(c) = child {
            out.push(c.clone());
            walk(&c, out);
            child = c.next_sibling();
        }
    }

    /// The two buttons of the Anime | Manga switch in the header, anime first.
    fn switch_buttons(shell: &BrowseShell) -> Vec<gtk::ToggleButton> {
        let mut all = Vec::new();
        walk(shell.root.upcast_ref(), &mut all);
        all.iter().filter_map(|w| w.downcast_ref::<gtk::ToggleButton>().filter(|b| b.parent().is_some_and(|p| p.is::<gtk::StackSwitcher>())).cloned()).collect()
    }

    /// The tab has the two source pages and nothing else: no extensions, no migrate, one search in the header.
    fn the_tab_has_only_the_two_source_pages_and_one_header_search() {
        let (shell, _log) = shell();
        let names: Vec<String> = shell
            .inner
            .stack
            .pages()
            .iter::<gtk::StackPage>()
            .map(|page| page.expect("page").name().map(|n| n.to_string()).unwrap_or_default())
            .collect();
        assert_eq!(names, ["0", "1"], "anime sources, manga sources");
        let header = shell.root.first_child().and_downcast::<gtk::HeaderBar>().expect("the header row comes first");
        let mut all = Vec::new();
        walk(header.upcast_ref(), &mut all);
        assert_eq!(all.iter().filter(|w| w.is::<gtk::SearchEntry>()).count(), 0, "the entry is in the bar, not the header");
        assert!(header.next_sibling().and_then(|row| row.next_sibling()).is_some_and(|w| w.is::<gtk::SearchBar>()), "the search bar sits under the header and its narrow switch row");
        assert_eq!(all.iter().filter(|w| w.downcast_ref::<gtk::ToggleButton>().is_some_and(|b| b.icon_name().as_deref() == Some("system-search-symbolic"))).count(), 1, "one search toggle");
        assert!(!all.iter().any(|w| w.downcast_ref::<gtk::Label>().is_some_and(|l| l.label() == "Browse")), "no page title");
        assert!(shell.end.is_empty(), "the header is part of the page");
    }

    fn enter_runs_a_global_search_of_the_visible_kind() {
        let (shell, log) = shell();
        shell.inner.select(BrowsePage::MangaSources);
        assert_eq!(shell.inner.search.placeholder_text().as_deref(), Some(tr(MsgKey::action_global_search_hint).as_str()));
        shell.inner.search.set_text("  berserk ");
        shell.inner.search.emit_by_name::<()>("activate", &[]);
        assert_eq!(
            log.routes.borrow().last(),
            Some(&BrowseRoute::GlobalSearch { kind: Kind::Manga, source_id: None, query: "berserk".into() }),
            "the text is trimmed and every source of the kind is searched"
        );
        assert!(shell.inner.search.text().is_empty(), "the entry is cleared after the search");
    }

    fn the_switch_follows_the_page_and_ctrl_digits_pick_only_the_two_pages() {
        use gtk::gdk::{Key, ModifierType};
        let (shell, _log) = shell();
        let ctrl = ModifierType::CONTROL_MASK;
        assert!(shell.inner.key(Key::_2, ctrl));
        assert_eq!(shell.visible(), BrowsePage::MangaSources, "Ctrl+2 is the manga sources");
        assert!(switch_buttons(&shell)[1].is_active(), "the switch follows the page");
        assert!(shell.inner.key(Key::_1, ctrl));
        assert_eq!(shell.visible(), BrowsePage::AnimeSources);
        assert!(switch_buttons(&shell)[0].is_active());
        assert!(!shell.inner.key(Key::_3, ctrl), "there is no third page to pick");
        assert_eq!(shell.visible(), BrowsePage::AnimeSources);
        switch_buttons(&shell)[1].set_active(true);
        assert_eq!(shell.visible(), BrowsePage::MangaSources, "the switch picks the page");
    }

    fn escape_closes_the_search_then_leaves() {
        use gtk::gdk::{Key, ModifierType};
        let (shell, log) = shell();
        shell.inner.search.set_text("x");
        shell.inner.search.emit_by_name::<()>("search-changed", &[]);
        assert!(shell.inner.search_open(), "typing shows the bar");
        assert!(shell.inner.key(Key::Escape, ModifierType::empty()));
        assert!(!shell.inner.search_open() && shell.inner.search.text().is_empty(), "the first Escape closes the search and clears it");
        assert_eq!(*log.left.borrow(), 0);
        assert!(shell.inner.key(Key::Escape, ModifierType::empty()));
        assert_eq!(*log.left.borrow(), 1, "the second Escape leaves");
    }

    /// No "more" menu (the search button is the global search), and the Repositories button just after Refresh opens
    /// the repositories of the visible kind.
    fn the_repositories_button_follows_refresh_and_the_visible_kind() {
        let (shell, log) = shell();
        let mut all = Vec::new();
        walk(shell.root.upcast_ref(), &mut all);
        let menus = all.iter().filter_map(|w| w.downcast_ref::<gtk::MenuButton>()).filter(|b| b.tooltip_text().as_deref() != Some(tr(MsgKey::ui_view_options).as_str())).count();
        assert_eq!(menus, 0, "no more menu");
        let repos = all
            .iter()
            .filter_map(|w| w.downcast_ref::<gtk::Button>().cloned())
            .find(|b| b.tooltip_text().as_deref() == Some(tr(MsgKey::label_extension_repos).as_str()))
            .expect("the Repositories button");
        let refresh = repos.prev_sibling().and_downcast::<gtk::Button>().expect("Refresh comes before it");
        assert_eq!(refresh.tooltip_text().as_deref(), Some(tr(MsgKey::browse_check_sources).as_str()));
        shell.inner.select(BrowsePage::MangaSources);
        repos.emit_clicked();
        shell.inner.select(BrowsePage::AnimeSources);
        repos.emit_clicked();
        assert_eq!(*log.repos.borrow(), [Kind::Manga, Kind::Anime], "the button follows the visible kind");
    }

    fn the_view_options_list_the_languages_and_a_pick_switches_one() {
        let (shell, log) = shell();
        let mut all = Vec::new();
        walk(shell.root.upcast_ref(), &mut all);
        let options = all
            .iter()
            .filter_map(|w| w.downcast_ref::<gtk::MenuButton>().cloned())
            .find(|b| b.tooltip_text().as_deref() == Some(tr(MsgKey::ui_view_options).as_str()))
            .expect("the shared view-options button, where the Filter button was");
        let labels = crate::widgets::view_options::menu_labels(&options.menu_model().expect("the menu"));
        assert!(labels.contains(&"English".to_owned()) && labels.contains(&"French".to_owned()), "{labels:?}");
        options.activate_action("viewopt.lang-fr", None).expect("the French check");
        assert_eq!(*log.langs.borrow(), [("fr".to_owned(), true)], "an off language is switched on");
    }

    pub(crate) fn smoke() {
        the_view_options_list_the_languages_and_a_pick_switches_one();
        the_tab_has_only_the_two_source_pages_and_one_header_search();
        enter_runs_a_global_search_of_the_visible_kind();
        the_switch_follows_the_page_and_ctrl_digits_pick_only_the_two_pages();
        escape_closes_the_search_then_leaves();
        the_repositories_button_follows_refresh_and_the_visible_kind();
    }
}
