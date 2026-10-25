//! The header bar of a source listing: back, the source name, the shared search toggle, the view options (the look
//! sections shared with the Library, `widgets::view_look`) and the overflow menu (T-40a-002). The overflow entries are
//! in `header`. A picked look is stored in `sources.display_mode`, one key for every source.

use std::rc::Rc;

use gtk::prelude::*;
use kuro_i18n::{MsgKey, tr};
use kuro_prefs::library::LibraryDisplayMode;

use super::header::{OverflowItem, overflow_icon, overflow_items, overflow_label};
use super::prefs::BrowsePrefs;
use crate::widgets::view_look;
use crate::widgets::view_options::ViewOptions;

pub struct HeaderDeps {
    pub source_name: String,
    pub prefs: Rc<BrowsePrefs>,
    pub is_local: bool,
    /// The source has settings of its own.
    pub configurable: bool,
    pub on_back: Rc<dyn Fn()>,
    /// The page's search toggle (`ui::search_row`); it goes at the end of the row, left of the other actions.
    pub search: gtk::ToggleButton,
    /// A look was picked and already stored.
    pub on_display_mode: Rc<dyn Fn(LibraryDisplayMode)>,
    pub on_overflow: Rc<dyn Fn(OverflowItem)>,
    /// The refresh button: loads the listing again from its first page.
    pub on_refresh: Rc<dyn Fn()>,
}

#[allow(dead_code)] // kept for the shape of the type; not built or read yet
pub struct SourceHeader {
    pub bar: gtk::HeaderBar,
    pub search: gtk::ToggleButton,
    pub display: ViewOptions,
    pub overflow: Option<gtk::MenuButton>,
}

/// The narrowest a source's name shows in its header: room for a few letters and the "…".
const TITLE_MIN_PX: i32 = 48;

pub fn build(deps: HeaderDeps) -> SourceHeader {
    let bar = crate::ui::header_bar();
    bar.set_decoration_layout(Some(&crate::ui::decoration(false)));
    // an Inscription, not a Label: an ellipsized label still needs the width of "…", which kept the window from 360 px
    // the natural width counts average-width characters, so a name of wide letters ("AnimePahe") needs more than its count
    let chars = u32::try_from(deps.source_name.chars().count()).unwrap_or(u32::MAX).saturating_mul(3) / 2 + 1;
    let title = gtk::Inscription::builder().text(&deps.source_name).css_classes(["title"]).min_chars(0).nat_chars(chars).xalign(0.5).text_overflow(gtk::InscriptionOverflow::EllipsizeEnd).valign(gtk::Align::Center).build();
    // with too little room the name would shrink to a lone "…": it is hidden then (it keeps its place)
    title.add_tick_callback(|title, _| {
        title.set_opacity(if title.width() < TITLE_MIN_PX { 0.0 } else { 1.0 });
        gtk::glib::ControlFlow::Continue
    });
    bar.set_title_widget(Some(&title));

    bar.pack_start(&back_button(Rc::clone(&deps.on_back)));

    // the page actions are buttons at the end; in the phone layout, where they do not fit, the ⋮ menu after Back stands
    // in for them (`ui::pack_lead`), as on a title's page (user, 2026-10-25)
    let overflow = overflow_menu(&deps);
    let actions = overflow_buttons(&deps);
    let lead = crate::ui::pack_lead(&bar, overflow.as_ref().map(|menu| menu.upcast_ref()));

    let display = display_menu(&deps);
    bar.pack_end(&display.widget);
    for button in actions.iter().rev() {
        bar.pack_end(button);
    }
    fold_actions(&bar, actions, overflow.clone());

    // the actions at the end, left to right: search (at the start in the phone layout), refresh, the page actions, then
    // the view options
    let on_refresh = Rc::clone(&deps.on_refresh);
    let refresh = crate::ui::refresh_button(&tr(MsgKey::action_webview_refresh));
    refresh.connect_clicked(move |button| {
        crate::ui::spin_once(button);
        on_refresh();
    });
    bar.pack_end(&refresh);
    let search = deps.search.clone();
    let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bar.pack_end(&slot);
    crate::ui::search_follows_width(&bar, search.upcast_ref(), &lead, &slot);
    crate::ui::no_sticky_focus(&bar);

    SourceHeader { bar, search, display, overflow }
}

/// The header of a page that has nothing but its title and a way back: the missing-source page (T-41a-008).
pub fn stub_header(title: &str, on_back: Rc<dyn Fn()>) -> gtk::HeaderBar {
    let bar = crate::ui::header_bar();
    bar.set_decoration_layout(Some(&crate::ui::decoration(false)));
    bar.set_title_widget(Some(&gtk::Label::builder().label(title).css_classes(["title"]).ellipsize(gtk::pango::EllipsizeMode::End).build()));
    bar.pack_start(&back_button(on_back));
    crate::ui::no_sticky_focus(&bar);
    bar
}

/// The Back button of both headers: the translated tooltip, which is also its accessible name.
fn back_button(on_back: Rc<dyn Fn()>) -> gtk::Button {
    let back = crate::ui::icon_button("go-previous-symbolic", &tr(MsgKey::action_webview_back));
    back.connect_clicked(move |_| on_back());
    back
}

/// The view options: the look sections shared with the Library. A pick is stored, shown, and passed to
/// `on_display_mode`.
fn display_menu(deps: &HeaderDeps) -> ViewOptions {
    let menu = ViewOptions::new(&tr(MsgKey::ui_view_options), &view_look::sections());
    let pref = deps.prefs.sources.display_mode.clone();
    view_look::sync(&menu, pref.get());
    view_look::install(&menu, &deps.prefs.cover_size);
    let (on_mode, shown) = (Rc::clone(&deps.on_display_mode), menu.downgrade());
    menu.connect(move |pick| {
        let Some(mode) = view_look::picked(&pick, pref.get()) else { return };
        if let Err(e) = pref.set(mode) {
            tracing::warn!(error = %e, "the source view could not be saved");
        }
        if let Some(menu) = shown() {
            view_look::sync(&menu, mode);
        }
        on_mode(mode);
    });
    menu
}

fn overflow_buttons(deps: &HeaderDeps) -> Vec<gtk::Button> {
    overflow_items(deps.is_local, deps.configurable)
        .into_iter()
        .map(|item| {
            let button = crate::ui::icon_button(overflow_icon(item), &overflow_label(item));
            let on_overflow = Rc::clone(&deps.on_overflow);
            button.connect_clicked(move |_| on_overflow(item));
            button
        })
        .collect()
}

/// Shows the page actions as buttons in a wide window and as the ⋮ menu in the phone layout.
fn fold_actions(bar: &gtk::HeaderBar, buttons: Vec<gtk::Button>, menu: Option<gtk::MenuButton>) {
    let show = move |narrow: bool| {
        for button in &buttons {
            button.set_visible(!narrow);
        }
        if let Some(menu) = &menu {
            menu.set_visible(narrow);
        }
    };
    show(crate::ui::window_width().is_some_and(|w| w < crate::shell::layout::WIDE_MIN_WIDTH_PX));
    bar.add_tick_callback(move |bar, _| {
        if let Some(width) = bar.root().map(|root| root.width()).filter(|w| *w > 0) {
            show(width < crate::shell::layout::WIDE_MIN_WIDTH_PX);
        }
        gtk::glib::ControlFlow::Continue
    });
}

fn overflow_menu(deps: &HeaderDeps) -> Option<gtk::MenuButton> {
    let entries = overflow_items(deps.is_local, deps.configurable)
        .into_iter()
        .map(|item| {
            let on_overflow = Rc::clone(&deps.on_overflow);
            crate::ui::MenuEntry::new(overflow_label(item), move || on_overflow(item))
        })
        .collect();
    crate::ui::menu_button(vec![entries])
}

#[cfg(test)]
pub(crate) mod tests {
    use kuro_platform::kind::Kind;
    use std::cell::RefCell;

    use kuro_prefs::store::PrefStore;

    use super::*;

    fn deps(prefs: Rc<BrowsePrefs>, log: &Rc<RefCell<Vec<String>>>, local: bool, configurable: bool) -> HeaderDeps {
        let (back, mode, overflow) = (Rc::clone(log), Rc::clone(log), Rc::clone(log));
        HeaderDeps {
            source_name: "Source".into(),
            prefs,
            is_local: local,
            configurable,
            on_back: Rc::new(move || back.borrow_mut().push("back".into())),
            search: gtk::ToggleButton::new(),
            on_display_mode: Rc::new(move |m| mode.borrow_mut().push(format!("mode {m:?}"))),
            on_overflow: Rc::new(move |i| overflow.borrow_mut().push(format!("overflow {i:?}"))),
            on_refresh: Rc::new(|| {}),
        }
    }

    fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut out = Vec::new();
        let mut child = root.first_child();
        while let Some(w) = child {
            out.extend(descendants(&w));
            child = w.next_sibling();
            out.push(w);
        }
        out
    }

    fn header_shows_the_stored_look_and_stores_a_pick() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let store = PrefStore::in_memory();
        let prefs = Rc::new(BrowsePrefs::new(store.clone(), Kind::Manga));
        let header = build(deps(Rc::clone(&prefs), &log, false, true));
        assert_eq!(header.display.radio("mode").as_deref(), Some("grid"), "the compact grid is the default");
        assert_eq!(header.display.radio("style").as_deref(), Some("compact"));

        header.display.group().activate_action("style", Some(&"comfortable".to_variant()));
        header.display.group().activate_action("mode", Some(&"list".to_variant()));
        assert_eq!(*log.borrow(), ["mode ComfortableGrid", "mode List"]);
        assert_eq!(prefs.sources.display_mode.get(), LibraryDisplayMode::List, "the pick is stored");
        assert_eq!(header.display.radio("style").as_deref(), Some(""), "no grid style is marked in the list view");
        let library = kuro_prefs::library::LibraryPrefs::new(store);
        assert_eq!(library.display_mode.get(), LibraryDisplayMode::CompactGrid, "the library's view is its own");

        let reopened = build(deps(Rc::clone(&prefs), &log, false, true));
        assert_eq!(reopened.display.radio("mode").as_deref(), Some("list"), "the stored look is shown again");
        assert!(header.search.is_ancestor(&header.bar), "the search toggle is in the header");
        // a wide window: the page actions are buttons and the ⋮ menu is hidden
        let webview = descendants(header.bar.upcast_ref())
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Button>().ok())
            .find(|b| b.tooltip_text().as_deref() == Some(overflow_label(OverflowItem::OpenInWebView).as_str()))
            .expect("the Open in WebView button");
        assert!(webview.is_visible() && !header.overflow.as_ref().expect("the ⋮ menu").is_visible());
        webview.emit_clicked();
        assert_eq!(log.borrow().last().map(String::as_str), Some("overflow OpenInWebView"));
        let slot = header.search.parent().expect("the search's slot");
        let next = slot.next_sibling().and_downcast::<gtk::Button>().expect("a button after the search");
        assert_eq!(next.tooltip_text().as_deref(), Some(tr(MsgKey::action_webview_refresh).as_str()), "the search is first of the actions, before Refresh");
    }

    pub(crate) fn smoke() {
        header_shows_the_stored_look_and_stores_a_pick();
    }
}
