//! The Sources tab of either kind: language header rows, then source rows; Latest is opened from the listing (T-40a-005,
//! T-40b-003, T-41b-005). Sources are not pinned or hidden here: the language filter decides which rows show.
//! `sources::source_list` decides what is listed; this file draws it and routes the clicks.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use gtk::prelude::*;
use kuro_domain::source::browse::BrowseQuery;
use kuro_domain::source::model::Source;
use kuro_i18n::{MsgKey, tr};
use tokio::sync::watch;

use super::glue::{follow_ready, keep_alive};
use super::icons::IconCache;
use crate::theme::tokens::{SPACE_XS};
use crate::theme::tokens::{SPACE_S};
#[cfg(test)]
use super::icons::SourceIcon;
use super::sources::{ListInput, LOCAL_SOURCE_ID, Section, SourceRow, language_line, section_title, source_list};
use super::view::{list_row, source_row_with};
use crate::widgets::context_menu::Item;

/// The installed sources and the preferences that shape the list, as one snapshot of the models.
#[derive(Clone, Debug, Default)]
pub struct SourcesSnapshot {
    pub sources: Vec<Source>,
    /// `source_languages`.
    pub enabled_languages: BTreeSet<String>,
    /// Decimal ids of sources the list leaves out. The page never writes it (the hide toggle is cut); the caller passes
    /// none.
    pub disabled: BTreeSet<String>,
    /// `last_catalogue_source`, -1 for none.
    pub last_used: i64,
    /// Icon files of the extensions that ship one, by source id.
    pub icon_files: BTreeMap<i64, String>,
    /// The sites of the sources that have one, by source id: a row's Open website.
    pub home_urls: BTreeMap<i64, String>,
}

/// What a click on the page does.
pub struct SourcesNav {
    /// A row opens its popular listing; the Latest button opens its latest listing.
    pub open_listing: Rc<dyn Fn(i64, BrowseQuery)>,
    /// A row menu's Open website: the source's site in the browser.
    pub open_url: Rc<dyn Fn(&str)>,
    /// The kind of the page's sources, for a row's Check now.
    pub kind: kuro_platform::kind::Kind,
}

struct Inner {
    stack: gtk::Stack,
    column: gtk::Box,
    scroller: gtk::ScrolledWindow,
    state: RefCell<State>,
    nav: SourcesNav,
}

struct State {
    snapshot: SourcesSnapshot,
    ready: bool,
    icons: IconCache,
}

/// Cloning shares the page; the widget keeps it alive.
#[derive(Clone)]
pub struct SourcesPage {
    inner: Rc<Inner>,
    root: gtk::Box,
}

impl SourcesPage {
    /// `ready` is `SourceRegistry::subscribe_initialized()`: the page shows a spinner until it is true and fills in
    /// when it turns true, without being reopened.
    pub fn new(nav: SourcesNav, ready: watch::Receiver<bool>) -> Self {
        let column = crate::ui::column();
        let scroller = crate::ui::scrolled(&column);
        let empty = sources_empty();
        let stack = gtk::Stack::new();
        stack.add_named(&loading_sources(), Some("spinner"));
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&scroller, Some("list"));
        let inner = Rc::new(Inner {
            stack,
            column,
            scroller,
            state: RefCell::new(State {
                snapshot: SourcesSnapshot { last_used: -1, ..SourcesSnapshot::default() },
                ready: false,
                icons: IconCache::default(),
            }),
            nav,
        });
        // the root owns the state; the state holds only the stack inside it, never the root
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        inner.stack.set_vexpand(true);
        root.append(&inner.stack);
        keep_alive(&root, &inner);
        let weak = Rc::downgrade(&inner);
        follow_ready(ready, move |ready| {
            let Some(inner) = weak.upgrade() else { return false };
            inner.state.borrow_mut().ready = ready;
            inner.render();
            true
        });
        Self { inner, root }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// A new snapshot of the models: re-renders the list.
    pub fn update(&self, snapshot: SourcesSnapshot) {
        self.inner.state.borrow_mut().snapshot = snapshot;
        self.inner.render();
    }

    #[cfg(test)]
    /// The vertical scroll offset of the list.
    pub fn scroll_position(&self) -> f64 {
        self.inner.scroller.vadjustment().value()
    }

    #[cfg(test)]
    pub fn set_scroll_position(&self, value: f64) {
        self.inner.scroller.vadjustment().set_value(value);
    }
}

impl Inner {
    fn render(self: &Rc<Self>) {
        let rows = {
            let state = self.state.borrow();
            if !state.ready {
                self.stack.set_visible_child_name("spinner");
                return;
            }
            let snapshot = &state.snapshot;
            source_list(&ListInput {
                sources: &snapshot.sources,
                enabled_languages: &snapshot.enabled_languages,
                disabled: &snapshot.disabled,
                last_used: snapshot.last_used,
            })
        };
        let scroll = self.scroller.vadjustment().value();
        while let Some(child) = self.column.first_child() {
            self.column.remove(&child);
        }
        if rows.is_empty() {
            self.stack.set_visible_child_name("empty");
            return;
        }
        let mut section = Section::LastUsed;
        let mut list: Option<gtk::ListBox> = None;
        for row in rows {
            match row {
                SourceRow::Header(next) => {
                    self.column.append(&header_label(&section_title(&next)));
                    list = None;
                    section = next;
                }
                SourceRow::Item(source) => {
                    let row = self.row(&section, source);
                    section_list(&self.column, &mut list).append(&row);
                }
            }
        }
        self.stack.set_visible_child_name("list");
        // a rebuild resets the offset: put it back, so a refresh of the rows never jumps the list
        self.scroller.vadjustment().set_value(scroll);
    }

    fn row(self: &Rc<Self>, section: &Section, source: Source) -> gtk::ListBoxRow {
        let (icon, file, home) = {
            let mut state = self.state.borrow_mut();
            let file = state.snapshot.icon_files.get(&source.id).cloned();
            let home = state.snapshot.home_urls.get(&source.id).cloned();
            let icon = state.icons.get(source.id, false, source.is_stub, file.is_some(), source.id == LOCAL_SOURCE_ID);
            (icon, file, home)
        };
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, SPACE_XS);
        if let Some(status) = crate::recipe_sources::status(source.id) {
            actions.append(&status_dot(status));
        }

        // the language (Last used only), then the repository's description of the source
        let language = match (language_line(section, &source), crate::recipe_sources::description(source.id)) {
            (Some(language), Some(about)) => Some(format!("{language} · {about}")),
            (language, about) => language.or(about),
        };
        let picture = crate::recipe_sources::icon_path(source.id);
        let row = source_row_with(&source.name, language.as_deref(), icon, file.as_deref(), picture.as_deref(), source.is_stub, crate::browse::errors::is_failing(source.id), Some(actions.upcast_ref()));
        let (open, menu, id, name) = (Rc::downgrade(self), Rc::downgrade(self), source.id, source.name.clone());
        list_row(
            &row,
            move || {
                if let Some(inner) = open.upgrade() {
                    (inner.nav.open_listing)(id, BrowseQuery::Popular);
                }
            },
            move || {
                let Some(inner) = menu.upgrade() else { return Vec::new() };
                // a repository source has a switch; a recipe of the scrapers folder and the local source do not
                let switch = crate::source_switch::key_of(id).map(|key| (key, name.clone()));
                row_menu(&inner.nav, id, home.clone(), switch)
            },
        )
    }
}

/// The dot of a repo source's status: green when it works, orange when its site blocks the app for now, red when broken.
fn status_dot(status: kuro_extensions::repo::Status) -> gtk::Box {
    use kuro_extensions::repo::Status;
    let (class, text) = match status {
        Status::Active => ("kb-status-active", tr(MsgKey::source_status_active)),
        Status::Blocked => ("kb-status-blocked", tr(MsgKey::source_status_blocked)),
        Status::Broken => ("kb-status-broken", tr(MsgKey::source_status_broken)),
    };
    let dot = gtk::Box::builder().valign(gtk::Align::Center).margin_end(SPACE_S).css_classes(["kb-status", class]).tooltip_text(&text).build();
    dot.update_property(&[gtk::accessible::Property::Label(&text)]);
    dot
}

/// The loading state of a Sources tab: the page spinner with its "Loading sources…" line under it.
fn loading_sources() -> gtk::Widget {
    crate::widgets::status::loading(Some(&super::texts::loading_sources()))
}

/// The right-click menu of a source row: Open (its Popular listing), Open website when the source has a site, and for a
/// repository source (`switch`: its entry id and name) Check now and Turn off.
fn row_menu(nav: &SourcesNav, id: i64, home: Option<String>, switch: Option<(String, String)>) -> Vec<Item> {
    let open = Rc::clone(&nav.open_listing);
    let mut items = vec![Item::new(super::texts::OPEN, 0, move || open(id, BrowseQuery::Popular))];
    if let Some(url) = home {
        let open_url = Rc::clone(&nav.open_url);
        items.push(Item::new(super::texts::open_website(), 0, move || open_url(&url)));
    }
    if let Some((key, name)) = switch {
        let (kind, check_name) = (nav.kind, name.clone());
        items.push(Item::new(tr(MsgKey::source_switch_check_now), 0, move || crate::source_switch::check_now(kind, id, check_name.clone())));
        items.push(Item::new(tr(MsgKey::source_switch_turn_off), 1, move || crate::source_switch::confirm_off(key.clone(), &name, |_| {})));
    }
    items
}

fn header_label(text: &str) -> gtk::Label {
    crate::ui::heading(text)
}

/// The boxed list of the current section, started under its heading on first use.
fn section_list(column: &gtk::Box, list: &mut Option<gtk::ListBox>) -> gtk::ListBox {
    list.get_or_insert_with(|| {
        let list = crate::ui::boxed_list();
        column.append(&list);
        list
    })
    .clone()
}

/// The empty Sources tab: no source is listed yet, so its one action opens the repository list.
pub(super) fn sources_empty() -> adw::StatusPage {
    let repos = gtk::Button::with_label(&tr(MsgKey::label_extension_repos));
    repos.connect_clicked(move |_| super::mount::open_repos());
    crate::widgets::status::empty_with("system-search-symbolic", &tr(MsgKey::source_empty_screen), None, std::slice::from_ref(&repos))
}

#[cfg(test)]
/// The `SourceIcon` a row resolves to, for callers that need to know without building the page.
pub fn icon_of(source: &Source, has_custom_icon: bool) -> SourceIcon {
    super::icons::resolve(source.is_stub, has_custom_icon, source.id == LOCAL_SOURCE_ID)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn source(id: i64, name: &str, lang: &str, latest: bool) -> Source {
        let mut s = Source::new(id, name, lang);
        s.supports_latest = latest;
        s
    }

    fn snapshot() -> SourcesSnapshot {
        SourcesSnapshot {
            sources: vec![source(1, "Alpha", "en", true), source(2, "Beta", "en", false), source(3, "Gamma", "ja", false)],
            enabled_languages: ["en".to_owned(), "ja".to_owned()].into(),
            last_used: -1,
            ..SourcesSnapshot::default()
        }
    }

    fn nav(log: &Rc<RefCell<Vec<String>>>) -> SourcesNav {
        let (open, site) = (Rc::clone(log), Rc::clone(log));
        SourcesNav {
            open_listing: Rc::new(move |id, q| open.borrow_mut().push(format!("open {id} {q:?}"))),
            open_url: Rc::new(move |url: &str| site.borrow_mut().push(format!("site {url}"))),
            kind: kuro_platform::kind::Kind::Manga,
        }
    }

    #[test]
    fn a_source_row_menu_opens_the_listing_and_the_site_only_when_there_is_one() {
        let log: Rc<RefCell<Vec<String>>> = Rc::default();
        let nav = nav(&log);
        let plain = row_menu(&nav, 1, None, None);
        assert_eq!(plain.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), [crate::browse::texts::OPEN]);
        let both = row_menu(&nav, 2, Some("https://two.test".into()), None);
        assert_eq!(both.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), [crate::browse::texts::OPEN, &crate::browse::texts::open_website()]);
        for item in &both {
            (item.run)();
        }
        assert_eq!(*log.borrow(), ["open 2 Popular", "site https://two.test"]);
    }

    #[test]
    fn a_repository_source_row_offers_check_now_and_turn_off() {
        let log: Rc<RefCell<Vec<String>>> = Rc::default();
        let menu = row_menu(&nav(&log), 3, None, Some(("alpha".into(), "Alpha".into())));
        let check = tr(MsgKey::source_switch_check_now);
        let off = tr(MsgKey::source_switch_turn_off);
        assert_eq!(menu.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), [crate::browse::texts::OPEN, check.as_str(), off.as_str()]);
    }

    fn labels(column: &gtk::Box) -> Vec<String> {
        let mut out = Vec::new();
        let mut child = column.first_child();
        while let Some(widget) = child {
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                out.push(label.label().to_string());
            } else if let Some(list) = widget.downcast_ref::<gtk::ListBox>() {
                let mut row = list.first_child();
                while let Some(r) = row {
                    out.push("row".into());
                    row = r.next_sibling();
                }
            } else {
                out.push("row".into());
            }
            child = widget.next_sibling();
        }
        out
    }

    fn buttons(root: &gtk::Widget, out: &mut Vec<gtk::Button>) {
        if let Some(button) = root.downcast_ref::<gtk::Button>() {
            out.push(button.clone());
        }
        let mut child = root.first_child();
        while let Some(widget) = child {
            buttons(&widget, out);
            child = widget.next_sibling();
        }
    }

    fn the_page_waits_for_the_registry_then_lists_headers_and_rows() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (tx, rx) = watch::channel(false);
        let page = SourcesPage::new(nav(&log), rx);
        page.update(snapshot());
        assert_eq!(page.inner.stack.visible_child_name().as_deref(), Some("spinner"), "no list before the registry is ready");
        tx.send_replace(true);
        let context = gtk::glib::MainContext::default();
        let start = std::time::Instant::now();
        while page.inner.stack.visible_child_name().as_deref() != Some("list") && start.elapsed().as_secs() < 2 {
            while context.iteration(false) {}
        }
        assert_eq!(page.inner.stack.visible_child_name().as_deref(), Some("list"), "initialisation finishing updates the open page");
        // English (Alpha, Beta), then Japanese (Gamma): headers carry the display names
        let texts = labels(&page.inner.column);
        assert_eq!(texts.iter().filter(|t| t.as_str() == "row").count(), 3);
        assert_eq!(texts.first().map(String::as_str), Some("English"));
        assert!(texts.iter().any(|t| t == "English") && texts.iter().any(|t| *t == crate::browse::names::localized_display_name("ja")), "{texts:?}");
    }

    /// T-40b-085 `pre_init_sources_spinner`: a registry that is not initialised keeps the list loading, however many
    /// snapshots arrive, until it says it is ready (D-SS-01).
    fn pre_init_sources_spinner() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (tx, rx) = watch::channel(false);
        let page = SourcesPage::new(nav(&log), rx);
        for _ in 0..3 {
            page.update(snapshot());
            let context = gtk::glib::MainContext::default();
            while context.iteration(false) {}
            assert_eq!(page.inner.stack.visible_child_name().as_deref(), Some("spinner"), "still loading before initialisation");
        }
        assert!(page.inner.column.first_child().is_none(), "no row is drawn before the registry is ready");
        tx.send_replace(true);
        let context = gtk::glib::MainContext::default();
        let start = std::time::Instant::now();
        while page.inner.stack.visible_child_name().as_deref() != Some("list") && start.elapsed().as_secs() < 2 {
            while context.iteration(false) {}
        }
        assert_eq!(page.inner.stack.visible_child_name().as_deref(), Some("list"));
    }

    fn no_row_has_a_latest_or_pin_button() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (_tx, rx) = watch::channel(true);
        let page = SourcesPage::new(nav(&log), rx);
        page.update(snapshot());
        let mut all = Vec::new();
        buttons(page.inner.column.upcast_ref(), &mut all);
        let latest = tr(MsgKey::latest);
        assert!(!all.iter().any(|b| b.label().as_deref() == Some(latest.as_str())), "Latest is opened from the listing, not the row");
        assert!(!all.iter().any(|b| b.icon_name().as_deref() == Some("view-pin-symbolic")), "sources are not pinned from the list");
    }

    fn a_stub_row_says_not_installed_in_the_error_colour() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (_tx, rx) = watch::channel(true);
        let page = SourcesPage::new(nav(&log), rx);
        let mut stub = source(9, "Gone", "en", false);
        stub.is_stub = true;
        page.update(SourcesSnapshot { sources: vec![stub, source(1, "Alpha", "en", false)], ..snapshot() });
        fn errors(w: &gtk::Widget, out: &mut Vec<String>) {
            if let Some(label) = w.downcast_ref::<gtk::Label>() {
                if label.has_css_class("error") {
                    out.push(label.label().to_string());
                }
            }
            let mut child = w.first_child();
            while let Some(c) = child {
                errors(&c, out);
                child = c.next_sibling();
            }
        }
        let mut found = Vec::new();
        errors(page.inner.column.upcast_ref(), &mut found);
        assert_eq!(found, [tr(MsgKey::not_installed)], "only the stub row carries it");
    }

    fn no_sources_shows_the_empty_screen_and_a_stub_gets_the_warning_icon() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (_tx, rx) = watch::channel(true);
        let page = SourcesPage::new(nav(&log), rx);
        page.update(SourcesSnapshot { last_used: -1, ..SourcesSnapshot::default() });
        assert_eq!(page.inner.stack.visible_child_name().as_deref(), Some("empty"));
        assert_eq!(icon_of(&Source { is_stub: true, ..source(9, "Gone", "en", false) }, false), SourceIcon::Warning);
    }

    fn a_refresh_of_the_rows_keeps_the_scroll_offset() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (_tx, rx) = watch::channel(true);
        let page = SourcesPage::new(nav(&log), rx);
        let window = gtk::Window::new();
        window.set_default_size(300, 80);
        window.set_child(Some(page.widget()));
        window.present();
        let many = SourcesSnapshot { sources: (1..=12).map(|id| source(id, &format!("Source {id}"), "en", false)).collect(), ..snapshot() };
        page.update(many.clone());
        let context = gtk::glib::MainContext::default();
        while context.iteration(false) {}
        let adjustment = page.inner.scroller.vadjustment();
        let target = (adjustment.upper() - adjustment.page_size()).min(40.0);
        page.set_scroll_position(target);
        page.update(many);
        assert_eq!(page.scroll_position(), target, "the list is rebuilt at the same offset");
        window.close();
    }

    pub(crate) fn smoke() {
        a_refresh_of_the_rows_keeps_the_scroll_offset();
        the_page_waits_for_the_registry_then_lists_headers_and_rows();
        pre_init_sources_spinner();
        no_row_has_a_latest_or_pin_button();
        a_stub_row_says_not_installed_in_the_error_colour();
        no_sources_shows_the_empty_screen_and_a_stub_gets_the_warning_icon();
    }
}
