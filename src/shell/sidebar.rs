//! The left panel and its narrow-window twin, the bottom bar (docs/design/app-flow.md). The panel lists Library, Browse,
//! Updates, History and, while something is queued, Downloads with a filling circle and a count. Settings is pinned at
//! the bottom. The "Kurobayomi" title is the app menu (the GTK primary menu): Incognito mode, Help and About, with a small icon beside the title while Incognito mode is on. Clicking the item that is already shown only
//! scrolls its page to the top.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{Box as GBox, Orientation as O, gio, glib};
use kuro_i18n::{MsgKey, tr};

use crate::ui;
use crate::widgets::{download_button, rail};

/// The tab page of the download queue. Its item shows while something is queued, and while the page is open.
pub const DOWNLOADS: &str = "download_queue";

/// How often the Downloads item reads the queue.
const POLL: Duration = Duration::from_millis(500);

/// The side of the Downloads circle in px, the size of the symbolic icons beside it.
const RING: i32 = 16;

/// What the Downloads item shows for the percent of each queued unit: the count and the mean percent done, or `None`
/// when nothing is queued (the item then hides).
pub fn downloads_summary(percents: &[u8]) -> Option<(usize, u8)> {
    if percents.is_empty() {
        return None;
    }
    let sum: usize = percents.iter().map(|p| usize::from((*p).min(100))).sum();
    Some((percents.len(), (sum / percents.len()) as u8))
}

/// The percent done of every unit in the queue that is still going (a unit waiting for its source counts as 0), and then
/// how many units failed. A failed unit is not "downloading": it waits for a retry on the Downloads page. Empty when no
/// download service runs.
fn queued_percents() -> (Vec<u8>, usize) {
    let Some(downloads) = crate::downloads::live::downloads() else { return (Vec::new(), 0) };
    let snapshot = downloads.snapshot();
    let failed = snapshot.items.iter().filter(|unit| unit.state == kuro_downloads::DownloadState::Error).count();
    let percents = snapshot.items.iter().filter(|unit| unit.state != kuro_downloads::DownloadState::Error).map(|unit| unit.progress.percent()).collect();
    (percents, failed)
}

/// One panel row: the icon, the title, then `extra` (badges).
fn nav_row(icon: &str, title: &str, extra: &[&gtk::Widget]) -> gtk::ListBoxRow {
    let h = GBox::new(O::Horizontal, 12);
    ui::margins(&h, rail::ROW_PAD);
    h.append(&gtk::Image::from_icon_name(icon));
    h.append(&gtk::Label::builder().label(title).xalign(0.0).hexpand(true).build());
    for w in extra {
        h.append(*w);
    }
    let row = gtk::ListBoxRow::builder().child(&h).build();
    row.set_margin_top(rail::ITEM_SPACING / 2);
    row.set_margin_bottom(rail::ITEM_SPACING / 2);
    row
}

/// The Updates count pill. It starts empty and follows the shared counts.
fn updates_badge() -> gtk::Label {
    let badge = gtk::Label::builder().css_classes(["kb-nav-badge", "numeric"]).valign(gtk::Align::Center).opacity(0.0).build();
    if crate::data::is_installed() {
        let store = crate::data::app().base.store().clone();
        crate::misc::badge::bind(&badge, &kuro_prefs::library::LibraryPrefs::new(store));
    }
    badge
}

/// The Downloads row: the download button's circle, filled with the mean progress, and the count of queued items. The
/// row and its tab page (so the bottom bar's button too) show while something is queued and while the page is open.
fn downloads_row(icon: &str, title: &str) -> gtk::ListBoxRow {
    let percent = Rc::new(Cell::new(0u8));
    let ring = gtk::DrawingArea::builder().content_width(RING).content_height(RING).valign(gtk::Align::Center).build();
    ring.set_draw_func({
        let percent = Rc::clone(&percent);
        move |area, cr, width, height| download_button::draw(area, cr, width, height, download_button::Look::Downloading(percent.get()))
    });
    let count = gtk::Label::builder().css_classes(["kb-nav-badge", "numeric"]).valign(gtk::Align::Center).build();
    // failed downloads: a warning sign with their number, instead of a ring that would never fill
    let warning = gtk::Image::builder().icon_name("dialog-warning-symbolic").css_classes(["warning"]).valign(gtk::Align::Center).build();
    let row = nav_row(icon, title, &[ring.upcast_ref(), warning.upcast_ref(), count.upcast_ref()]);
    let weak = row.downgrade();
    let tick = move || {
        if weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        // the item is always there (user, 2026-10-12); only its circle and count follow the queue
        let (percents, failed) = queued_percents();
        let (queued, done) = downloads_summary(&percents).unwrap_or((0, 0));
        ring.set_visible(queued > 0);
        warning.set_visible(queued == 0 && failed > 0);
        warning.set_tooltip_text(Some(&format!("{failed} failed")));
        count.set_visible(queued > 0 || failed > 0);
        count.set_label(&(if queued > 0 { queued } else { failed }).to_string());
        if percent.replace(done) != done {
            ring.queue_draw();
        }
        glib::ControlFlow::Continue
    };
    let _ = tick();
    glib::timeout_add_local(POLL, tick);
    row
}

/// Scrolls every scrolled view under `root` back to its top.
fn scroll_to_top(root: &gtk::Widget) {
    if let Some(view) = root.downcast_ref::<gtk::ScrolledWindow>() {
        let adjustment = view.vadjustment();
        adjustment.set_value(adjustment.lower());
    }
    let mut child = root.first_child();
    while let Some(w) = child {
        scroll_to_top(&w);
        child = w.next_sibling();
    }
}

/// A click on the panel item `id`: when its page is already on show (nothing pushed over it), the page scrolls to the
/// top; otherwise the pushed pages go and the item's page shows.
fn select(tabs: &gtk::Stack, id: &str) {
    if ui::page_stack().1 == 0 && tabs.visible_child_name().as_deref() == Some(id) {
        if let Some(page) = tabs.visible_child() {
            scroll_to_top(&page);
        }
        return;
    }
    ui::pop_to_root();
    tabs.set_visible_child_name(id);
}

/// The widget name of the Settings page (`ui::screen_page`).
const SETTINGS_PAGE: &str = "screen:settings";

/// Settings, from the panel: nothing moves while it is on screen, a page inside it goes back to it, and from anywhere
/// else it slides in once.
fn open_settings() {
    let (names, shown) = ui::page_stack();
    match names.iter().take(shown + 1).position(|n| n == SETTINGS_PAGE) {
        Some(at) => ui::pop_to(at),
        None => ui::over_root(|| ui::open("settings")),
    }
}

fn open_help() {
    crate::ui::open_link(crate::pref_model::widgets::HELP_URL, None);
}

/// A menu button with the app menu: Incognito mode (the `app.incognito` check item), then Help and About, whose actions (`appmenu.*`) live on the button itself.
/// The name of the Incognito switch, in the panel and in the narrow window's menu.
fn incognito_label() -> String {
    tr(MsgKey::shell_incognito_label)
}

/// The app's own pages under the places, in the panel and in the narrow window's menu: icon, label, action name, opener.
/// About is a page of Settings only (user, 2026-10-15: one place for it). The keyboard shortcuts open with F1 or `?` on a
/// title page and are listed in the online help, not here (user, 2026-10-24).
fn app_pages() -> [(&'static str, String, &'static str, fn()); 2] {
    [
        ("help-browser-symbolic", tr(MsgKey::label_help), "help", open_help),
        ("emblem-system-symbolic", tr(MsgKey::label_settings), "settings", open_settings),
    ]
}

/// The app menu (☰) of the narrow window's bar.
fn app_menu_button() -> gtk::MenuButton {
    let menu = app_menu();
    menu.set_icon_name("open-menu-symbolic");
    menu.set_tooltip_text(Some(&tr(MsgKey::action_menu)));
    menu
}

fn app_menu() -> gtk::MenuButton {
    let toggles = gio::Menu::new();
    toggles.append(Some(&incognito_label()), Some(crate::incognito::TOGGLE_ACTION));
    let pages = gio::Menu::new();
    let actions = gio::SimpleActionGroup::new();
    for (_, label, name, run) in app_pages() {
        pages.append(Some(&label), Some(&format!("appmenu.{name}")));
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| run());
        actions.add_action(&action);
    }
    let model = gio::Menu::new();
    model.append_section(None, &toggles);
    model.append_section(None, &pages);
    let button = ui::quiet_menu(gtk::MenuButton::builder().menu_model(&model).build());
    button.insert_action_group("appmenu", Some(&actions));
    button
}

/// The small icon beside the panel title while Incognito mode is on. It follows the state of `app.incognito`.
/// The panel's title: the app's name, and a small channel line under it: "nightly", or "stable" with the version (a
/// property of the build, not part of the name; it also leaves the header room for the search, the menu and the
/// Incognito icon).
fn title() -> GBox {
    let title = GBox::new(O::Horizontal, 6);
    let names = GBox::new(O::Vertical, 0);
    names.set_valign(gtk::Align::Center);
    names.append(&gtk::Label::builder().label("Kurobayomi").css_classes(["title"]).build());
    let channel = if crate::channel::NIGHTLY { "nightly".to_string() } else { format!("stable {}", env!("CARGO_PKG_VERSION")) };
    names.append(&gtk::Label::builder().label(channel).css_classes(["kb-subtitle", "dim-label"]).build());
    title.append(&names);
    title
}

/// The rows at the bottom of the panel: Incognito mode with its switch, Help and Settings. Only
/// Settings is ever selected: while it is the page on screen (`follow_pages`).
fn bottom_rows() -> gtk::ListBox {
    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    list.set_margin_top(rail::PAD_V);
    list.set_margin_bottom(rail::PAD_V);
    // the switch is bound to app.incognito, so it shows the state and flips it like the menu item did
    let switch = gtk::Switch::builder().valign(gtk::Align::Center).action_name(crate::incognito::TOGGLE_ACTION).build();
    let incognito = nav_row(crate::incognito::ICON, &incognito_label(), &[switch.upcast_ref()]);
    incognito.set_selectable(false);
    list.append(&incognito);
    let rows = app_pages();
    let runs: Vec<fn()> = rows.iter().map(|(_, _, _, run)| *run).collect();
    // the Settings row (About is in it) turns a spinner while the app updates itself, then carries the "New" pill while
    // an update notice is unseen (`whats_new`)
    let (update_spinner, update_pill) = (crate::appimage_update::busy_spinner(), crate::whats_new::pill());
    for (icon, label, name, _) in rows {
        let extra: &[&gtk::Widget] = if name == "settings" { &[update_spinner.upcast_ref(), update_pill.upcast_ref()] } else { &[] };
        let row = nav_row(icon, &label, extra);
        row.set_selectable(name == "settings");
        list.append(&row);
    }
    list.connect_row_activated(move |_, row| {
        match usize::try_from(row.index()).ok() {
            // a click anywhere on the Incognito row flips its switch
            Some(0) => {
                switch.activate();
            }
            Some(i) => {
                if let Some(run) = runs.get(i - 1) {
                    run();
                }
            }
            None => {}
        }
    });
    list
}

/// Keeps one row of the panel selected, the place on screen: Settings while it (or a page inside it) shows, else the open
/// tab, also over a page opened from the tab (an entry is still "in" Browse). Follows the tabs and the pushed pages, so the
/// bottom bar, a link or Back moves it too.
fn follow_pages(tabs: &gtk::Stack, places: &gtk::ListBox, bottom: &gtk::ListBox, ids: Vec<String>) {
    let sync = {
        let (tabs, places, bottom) = (tabs.clone(), places.clone(), bottom.clone());
        Rc::new(move || {
            let (names, shown) = ui::page_stack();
            if names.iter().take(shown + 1).any(|n| n == SETTINGS_PAGE) {
                places.unselect_all();
                bottom.select_row(bottom.last_child().and_downcast::<gtk::ListBoxRow>().as_ref());
                return;
            }
            bottom.unselect_all();
            let tab = tabs.visible_child_name().and_then(|n| ids.iter().position(|x| *x == n)).unwrap_or(0);
            places.select_row(places.row_at_index(i32::try_from(tab).unwrap_or(0)).as_ref());
        })
    };
    let on_tab = Rc::clone(&sync);
    tabs.connect_visible_child_name_notify(move |_| on_tab());
    let on_page = Rc::clone(&sync);
    ui::stack().connect_visible_child_notify(move |_| on_page());
    sync();
}

/// The left panel: the app menu in a split header, one row per place from the top, and Settings at the bottom. Always
/// full width; there is no collapsed mode (docs/design/main-window.md).
pub fn build(tabs: &gtk::Stack) -> GBox {
    let header = ui::header_bar();
    header.set_decoration_layout(Some(&ui::decoration(true)));
    header.set_title_widget(Some(&title()));
    // the open tab's search before the app name (`lead`)
    header.pack_start(&super::lead::host());
    ui::no_sticky_focus(&header);

    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    list.set_valign(gtk::Align::Start);
    list.set_margin_top(rail::PAD_V);
    list.set_margin_bottom(rail::PAD_V);
    let ids: Vec<String> = ui::with_spec(|s| s.nav.iter().map(|n| n.id.clone()).collect());
    for n in ui::with_spec(|s| s.nav.clone()) {
        let row = match n.id.as_str() {
            "updates" => nav_row(&n.icon, &crate::spec::shown(&n.title), &[updates_badge().upcast_ref()]),
            DOWNLOADS => downloads_row(&n.icon, &crate::spec::shown(&n.title)),
            _ => nav_row(&n.icon, &crate::spec::shown(&n.title), &[]),
        };
        list.append(&row);
    }
    {
        let (tabs, ids) = (tabs.clone(), ids.clone());
        list.connect_row_activated(move |_, r| {
            if let Some(id) = usize::try_from(r.index()).ok().and_then(|i| ids.get(i)) {
                select(&tabs, id);
            }
        });
    }
    let settings = bottom_rows();
    follow_pages(tabs, &list, &settings, ids);

    let b = GBox::new(O::Vertical, 0);
    b.set_size_request(rail::WIDTH, -1);
    b.append(&header);
    b.append(&gtk::ScrolledWindow::builder().child(&list).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build());
    b.append(&gtk::Separator::new(O::Horizontal));
    b.append(&settings);
    b
}

/// The narrow window's bar: the same items as the panel. The places switch the tab pages, then the app menu (Incognito,
/// Help, About and Settings).
pub fn bottom_bar(tabs: &gtk::Stack) -> GBox {
    let bar = GBox::new(O::Horizontal, 6);
    bar.add_css_class("kb-bottom-bar");
    bar.set_halign(gtk::Align::Center);
    bar.set_margin_top(6);
    bar.set_margin_bottom(6);
    let switcher = gtk::StackSwitcher::builder().stack(tabs).build();
    // the bar also shows over a pushed page: a tap leaves it for the tab pages, like the panel's rows
    let leave = gtk::GestureClick::new();
    leave.set_propagation_phase(gtk::PropagationPhase::Capture);
    leave.connect_pressed(|_, _, _, _| ui::pop_to_root());
    switcher.add_controller(leave);
    bar.append(&switcher);
    // Settings is in the menu with the app's other pages
    bar.append(&app_menu_button());
    ui::no_sticky_focus(&bar);
    bar
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(root: &gtk::Widget) -> Vec<String> {
        crate::screens::alone::descendants(root).iter().filter_map(|w| w.downcast_ref::<gtk::Label>().map(|l| l.label().to_string())).collect()
    }

    #[test]
    fn the_downloads_item_counts_the_queued_units_and_averages_their_progress() {
        assert_eq!(downloads_summary(&[]), None, "nothing queued hides the item");
        assert_eq!(downloads_summary(&[0]), Some((1, 0)), "a queued unit shows the empty circle");
        assert_eq!(downloads_summary(&[100, 0, 50, 50]), Some((4, 50)));
        assert_eq!(downloads_summary(&[250, 100]), Some((2, 100)), "a percent never counts past 100");
    }

    #[test]
    fn scrolling_to_the_top_reaches_every_scrolled_view_below() {
        crate::testing::with_display(|| {
            let adjustment = gtk::Adjustment::new(500.0, 0.0, 1000.0, 1.0, 10.0, 10.0);
            let view = gtk::ScrolledWindow::builder().vadjustment(&adjustment).build();
            let page = GBox::new(O::Vertical, 0);
            page.append(&view);
            scroll_to_top(page.upcast_ref());
            assert_eq!(adjustment.value(), 0.0);
        });
    }

    #[test]
    fn the_panel_has_the_places_and_the_app_rows_at_the_bottom() {
        crate::testing::with_display(|| {
            let _nav = crate::testing::nav();
            let panel = build(&gtk::Stack::new());
            let texts = labels(panel.upcast_ref());
            assert!(!texts.iter().any(|l| *l == tr(MsgKey::pref_category_about)), "About is in Settings only");
            let wanted = [incognito_label().to_owned(), tr(MsgKey::label_help)];
            for title in ["Library", "Browse", "Updates", "History", "Downloads", "Settings", "Kurobayomi"].map(str::to_owned).iter().chain(&wanted) {
                assert!(texts.iter().any(|l| l == title), "{title}");
            }
            assert!(!texts.iter().any(|l| l == "More"), "More is gone");
            let all = crate::screens::alone::descendants(panel.upcast_ref());
            assert!(!all.iter().any(|w| w.is::<gtk::MenuButton>()), "no menu of its own: a tab page's actions are buttons in its own header");
            assert!(all.iter().any(|w| w.downcast_ref::<gtk::Switch>().is_some_and(|s| s.action_name().as_deref() == Some(crate::incognito::TOGGLE_ACTION))), "Incognito has a switch");
            let downloads = all
                .iter()
                .filter_map(|w| w.downcast_ref::<gtk::ListBoxRow>())
                .find(|r| labels(r.upcast_ref()).iter().any(|l| l == "Downloads"))
                .expect("the Downloads row");
            assert!(downloads.is_visible(), "Downloads is always in the panel, also with an empty queue");
        });
    }

    #[test]
    fn the_bottom_bar_has_the_places_and_the_app_menu_with_settings_in_it() {
        crate::testing::with_display(|| {
            let bar = bottom_bar(&gtk::Stack::new());
            let all = crate::screens::alone::descendants(bar.upcast_ref());
            assert_eq!(all.iter().filter(|w| w.is::<gtk::StackSwitcher>()).count(), 1);
            let menus: Vec<_> = all.iter().filter_map(|w| w.downcast_ref::<gtk::MenuButton>()).collect();
            assert_eq!(menus.len(), 1, "the app menu, and no separate Settings button");
            let model = menus[0].menu_model().expect("a menu model");
            let mut labels = Vec::new();
            for section in 0..model.n_items() {
                let Some(part) = model.item_link(section, "section") else { continue };
                for i in 0..part.n_items() {
                    labels.extend(part.item_attribute_value(i, "label", None).and_then(|v| v.get::<String>()));
                }
            }
            assert_eq!(labels, [incognito_label().to_owned(), tr(MsgKey::label_help), tr(MsgKey::label_settings)]);
        });
    }

    #[test]
    fn one_row_is_selected_the_place_on_screen() {
        crate::testing::with_display(|| {
            let (stack, _) = crate::testing::nav();
            if stack.first_child().is_none() {
                stack.add_child(&gtk::Label::new(Some("root")));
            }
            ui::pop_to_root();
            let tabs = gtk::Stack::new();
            let ids: Vec<String> = ui::with_spec(|s| s.nav.iter().map(|n| n.id.clone()).collect());
            for id in &ids {
                tabs.add_named(&gtk::Label::new(Some(id)), Some(id));
            }
            let panel = build(&tabs);
            let lists: Vec<gtk::ListBox> = crate::screens::alone::descendants(panel.upcast_ref()).iter().filter_map(|w| w.downcast_ref::<gtk::ListBox>().cloned()).collect();
            let (places, bottom) = (&lists[0], &lists[1]);
            tabs.set_visible_child_name(&ids[1]);
            assert_eq!(places.selected_row().map(|r| r.index()), Some(1), "the open tab");
            assert!(bottom.selected_row().is_none());
            let settings = gtk::Label::new(None);
            settings.set_widget_name(SETTINGS_PAGE);
            ui::push(&settings);
            assert!(places.selected_row().is_none(), "no tab while Settings shows");
            assert_eq!(bottom.selected_row().map(|r| r.index()), Some(2), "the Settings row");
            let inside = gtk::Label::new(None);
            ui::push(&inside);
            assert_eq!(bottom.selected_row().map(|r| r.index()), Some(2), "a page inside Settings keeps it");
            ui::pop_to_root();
            assert_eq!(places.selected_row().map(|r| r.index()), Some(1), "back on the tab");
            assert!(bottom.selected_row().is_none());
        });
    }
}
