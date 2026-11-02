//! The settings shell (T-47b-001 to T-47b-003): the page that `body("settings")` builds. It starts on the page the router
//! picks, lists the settings in one pane or two, and owns what the back action does while settings are on screen. Back is
//! one action: Escape, Alt+Left and the mouse back button all end in [`go_back`].

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use kuro_i18n::{MsgKey, tr};

use super::router::{self, Back, Page, Shell};
use crate::screens::Built;
use crate::theme::tokens::{SPACE_L, SPACE_M, SPACE_S, SPACE_XS};

thread_local! {
    static SHELL: RefCell<Shell> = RefCell::new(Shell::default());
    /// The widget name of the screen that is the shell's own page in the window's stack.
    static ROOT: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn widget_name(screen_id: &str) -> String {
    format!("screen:{screen_id}")
}

/// The spec screen a router page shows.
pub fn page_screen_id(page: Page) -> &'static str {
    match page {
        Page::Main | Page::Inner => "settings",
        Page::Appearance => "settings_appearance",
        Page::Dest(d) => d.screen_id(),
    }
}

/// What a back action does with the window's stack.
#[derive(Debug, PartialEq, Eq)]
pub enum BackStep {
    /// The shell is not on screen; the caller does its own back.
    NotMine,
    /// Pop one page: an inner one, or the shell's own.
    Pop(Back),
}

/// Decides one back action. `names` is the window's stack from the root and `shown` the index of the page on show.
/// An inner page is any page above the shell's own, so the stack, not a counter, says how deep the user is.
pub fn plan_back(shell: &mut Shell, root: Option<&str>, names: &[String], shown: usize) -> BackStep {
    let Some(root) = root else { return BackStep::NotMine };
    let Some(at) = names.iter().rposition(|n| n == root) else {
        shell.close();
        return BackStep::NotMine;
    };
    if shown < at {
        return BackStep::NotMine;
    }
    shell.sync(shown - at);
    BackStep::Pop(shell.back())
}

/// Runs one back action for the settings shell. `false` means settings are not on screen and the caller pops as usual.
pub fn back() -> bool {
    let (names, shown) = crate::ui::page_stack();
    let step = ROOT.with(|root| SHELL.with(|shell| plan_back(&mut shell.borrow_mut(), root.borrow().as_deref(), &names, shown)));
    match step {
        BackStep::NotMine => false,
        BackStep::Pop(back) => {
            crate::ui::pop();
            if back == Back::Closed {
                ROOT.with(|root| *root.borrow_mut() = None);
            }
            true
        }
    }
}

/// The window's back action: the shell's when settings are on screen, else the plain pop.
pub fn go_back() {
    if !back() {
        crate::ui::pop();
    }
}

fn window_size() -> (i32, i32) {
    crate::window_size::size_of(&crate::ui::window())
}

fn two_pane_now() -> bool {
    let (width, height) = window_size();
    router::two_pane(width, height)
}

/// The body of the `settings` screen. A bare open lands on the main list (one pane) or Appearance beside the list
/// (two panes).
pub fn body() -> Option<Built> {
    let wide = two_pane_now();
    SHELL.with(|s| s.borrow_mut().open(None, wide));
    ROOT.with(|r| *r.borrow_mut() = Some(widget_name("settings")));
    let search = crate::ui::icon_button("system-search-symbolic", &tr(MsgKey::action_search_settings));
    search.connect_clicked(|_| crate::ui::open("settings_search"));
    let current = SHELL.with(|s| s.borrow().current()).unwrap_or(Page::Main);
    let content = if wide { two_pane(current) } else { main_list(Rc::new(|id| crate::ui::open(id))) };
    // the layout follows the window: a resize across the two-pane width rebuilds the page, checked every frame so it holds
    // for any way the window changes size
    content.add_tick_callback(move |_, _| {
        if two_pane_now() == wide {
            return glib::ControlFlow::Continue;
        }
        glib::idle_add_local_once(rebuild_for_resize);
        glib::ControlFlow::Break
    });
    Some(Built { content, end: vec![search.upcast()] })
}

/// A resize across the wide-window threshold: the shell's own page, when it is on top, is rebuilt for one pane or two.
fn rebuild_for_resize() {
    let (names, shown) = crate::ui::page_stack();
    let root = widget_name("settings");
    if names.get(shown) == Some(&root) && ROOT.with(|r| r.borrow().as_deref() == Some(root.as_str())) {
        crate::ui::pop();
        crate::ui::open("settings");
    }
}

/// A settings page as a row: the screen it opens, its title, its one-line summary and its icon.
pub struct NavEntry {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub icon: String,
}

/// The About subtitle (T-47a-003): `Kurobayomi Stable <version>` in a release build, `Debug <commit>` in any other.
fn about_subtitle() -> String {
    if crate::channel::NIGHTLY {
        return format!("{} {}", crate::channel::APP_NAME, crate::channel::short_commit());
    }
    let info = &*kuro_core::app_info::APP_INFO;
    crate::settings_ui::menu::about_subtitle("Kurobayomi", info.build_type == "release", env!("CARGO_PKG_VERSION"), info.git_sha)
}

/// The rows of the settings list, from the spec. About shows this app's build, not the Android one.
fn entries() -> Vec<NavEntry> {
    crate::ui::with_spec(|s| s.settings_main.clone())
        .into_iter()
        .map(|e| {
            let subtitle = if e.id == "settings_about" { Some(about_subtitle()) } else { e.subtitle };
            NavEntry { id: e.id, title: crate::spec::shown(&e.title), subtitle: subtitle.map(|s| crate::spec::shown(&s)), icon: e.icon }
        })
        .collect()
}

/// The settings list on a narrow window: one boxed list, a row per page (icon, title, summary, arrow).
/// The update marks of the About page's row, the same as the sidebar's Settings row carries: a spinner while the app
/// updates itself and the "New" pill while an update notice is unseen. Other rows have none.
fn update_marks(id: &str) -> Vec<gtk::Widget> {
    if id != "settings_about" {
        return Vec::new();
    }
    vec![crate::appimage_update::busy_spinner().upcast(), crate::whats_new::pill().upcast()]
}

fn main_list(on_select: Rc<dyn Fn(&str)>) -> gtk::Widget {
    use adw::prelude::*;
    let column = crate::ui::column();
    let list = crate::ui::boxed_list();
    for entry in entries() {
        let suffix = gtk::Box::new(gtk::Orientation::Horizontal, SPACE_S);
        for mark in update_marks(&entry.id) {
            suffix.append(&mark);
        }
        suffix.append(&crate::ui::chevron());
        let row = crate::ui::row(&entry.title, entry.subtitle.as_deref(), Some(suffix.upcast()));
        // the page's icon goes in front of its title
        row.add_prefix(&gtk::Image::from_icon_name(&entry.icon));
        row.set_activatable(true);
        let (id, on_select) = (entry.id.clone(), on_select.clone());
        crate::widgets::row_activate::on_activate(&row, move |_| on_select(&id));
        list.append(&row);
    }
    column.append(&list);
    crate::ui::scrolled(&column).upcast()
}

/// Width of the settings sidebar in the split view.
pub const SIDEBAR_WIDTH: i32 = 240;

/// The split view of the settings shell and the player settings: a sidebar in the app sidebar's look
/// (`navigation-sidebar`, icon and title, the summary as tooltip) and the selected page beside it. The selected row
/// names the page, so the page shows no title of its own (docs/design/other-tabs.md).
pub fn split(entries: Vec<NavEntry>, current: &str) -> gtk::Widget {
    let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
    right.set_hexpand(true);
    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    list.set_valign(gtk::Align::Start);
    list.set_margin_top(crate::widgets::rail::PAD_V);
    list.set_margin_bottom(crate::widgets::rail::PAD_V);
    for entry in &entries {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, SPACE_M);
        line.append(&gtk::Image::from_icon_name(&entry.icon));
        line.append(&gtk::Label::builder().label(&entry.title).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build());
        for mark in update_marks(&entry.id) {
            line.append(&mark);
        }
        let row = crate::widgets::rail::row(&line);
        row.set_tooltip_text(entry.subtitle.as_deref());
        list.append(&row);
    }
    let ids: Vec<String> = entries.into_iter().map(|e| e.id).collect();
    {
        let (right, ids) = (right.clone(), ids.clone());
        // selection follows the mouse and the arrow keys alike, so the page always matches the highlighted row
        list.connect_row_selected(move |_, row| {
            let Some(id) = row.and_then(|r| usize::try_from(r.index()).ok()).and_then(|i| ids.get(i)) else { return };
            while let Some(child) = right.first_child() {
                right.remove(&child);
            }
            right.append(&pane(id));
        });
    }
    let at = ids.iter().position(|id| id == current).unwrap_or(0);
    list.select_row(list.row_at_index(i32::try_from(at).unwrap_or(0)).as_ref());
    let left = gtk::ScrolledWindow::builder().child(&list).hscrollbar_policy(gtk::PolicyType::Never).build();
    left.set_size_request(SIDEBAR_WIDTH, -1);
    let both = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    both.append(&left);
    both.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    both.append(&right);
    both.upcast()
}

/// The main list on the left and the open page on the right.
fn two_pane(current: Page) -> gtk::Widget {
    split(entries(), page_screen_id(current))
}

/// A settings page inside the split view's right side: its content, with its header actions (if any) on a slim row
/// at the top end. The player settings show their plain list here, never a second sidebar.
fn pane(id: &str) -> gtk::Widget {
    let screen = crate::ui::screen(id);
    let built = if id == "settings_player" { crate::settings_ui::player::root::body(&crate::settings_ui::route::prefs()) } else { crate::screens::body(id) };
    let (content, end): (gtk::Widget, Vec<gtk::Widget>) = match built {
        Some(b) => (b.content, b.end),
        None => {
            let column = crate::ui::column();
            crate::ui::groups_into(&column, &screen.groups);
            (crate::ui::scrolled(&column).upcast(), crate::ui::toolbar(&screen))
        }
    };
    // the window's header already has the one settings search, so a page's own search button is not repeated here
    let end: Vec<gtk::Widget> = end
        .into_iter()
        .filter(|w| w.downcast_ref::<gtk::Button>().is_none_or(|b| b.icon_name().as_deref() != Some("system-search-symbolic")))
        .collect();
    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    crate::ui::tag(&page, id);
    if !end.is_empty() {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, SPACE_XS);
        bar.set_halign(gtk::Align::End);
        bar.set_margin_top(SPACE_S);
        bar.set_margin_end(SPACE_L);
        for widget in end {
            if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                button.add_css_class("flat");
            }
            bar.append(&widget);
        }
        page.append(&bar);
    }
    content.set_vexpand(true);
    page.append(&content);
    page.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::router::Destination;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| (*n).to_owned()).collect()
    }

    fn opened(wide: bool) -> Shell {
        let mut shell = Shell::default();
        shell.open(None, wide);
        shell
    }

    #[test]
    fn t_47b_003_back_pops_the_inner_page_first_then_closes_the_shell() {
        let stack = names(&["screen:tabs", "screen:settings", "screen:settings_about", "screen:settings_debug_info"]);
        let mut shell = opened(false);
        assert_eq!(plan_back(&mut shell, Some("screen:settings"), &stack, 3), BackStep::Pop(Back::Popped));
        // the page above the shell is gone; the next back is the shell's own page
        assert_eq!(plan_back(&mut shell, Some("screen:settings"), &stack, 2), BackStep::Pop(Back::Popped));
        assert_eq!(plan_back(&mut shell, Some("screen:settings"), &stack, 1), BackStep::Pop(Back::Closed));
        assert!(!shell.is_open());
    }

    #[test]
    fn t_47b_003_back_is_not_the_shells_when_settings_are_not_on_screen() {
        let mut shell = opened(false);
        assert_eq!(plan_back(&mut shell, None, &names(&["screen:tabs"]), 0), BackStep::NotMine);
        let gone = names(&["screen:tabs", "screen:browse"]);
        assert_eq!(plan_back(&mut shell, Some("screen:settings"), &gone, 1), BackStep::NotMine);
        assert!(!shell.is_open(), "a shell whose page left the stack is closed");
        let mut shell = opened(false);
        let below = names(&["screen:tabs", "screen:settings"]);
        assert_eq!(plan_back(&mut shell, Some("screen:settings"), &below, 0), BackStep::NotMine, "a page below the shell is showing");
    }

    #[test]
    fn t_47b_001_a_destination_opens_alone_and_unknown_ids_open_the_default_page() {
        let mut shell = Shell::default();
        shell.open(Destination::from_id(0), false);
        assert_eq!(shell.current(), Some(Page::Dest(Destination::About)));
        assert_eq!(page_screen_id(Page::Dest(Destination::About)), "settings_about");
        shell.open(Destination::from_id(9), false);
        assert_eq!(shell.current(), Some(Page::Main), "an unknown id opens the main list on a narrow window");
        shell.open(Destination::from_id(-4), true);
        assert_eq!(shell.current(), Some(Page::Appearance), "and Appearance on a wide one");
        assert_eq!(page_screen_id(Page::Appearance), "settings_appearance");
        assert_eq!(page_screen_id(Page::Main), "settings");
    }

    #[test]
    fn the_about_row_names_this_app_and_its_build() {
        let about = entries().into_iter().find(|e| e.id == "settings_about").expect("the About row");
        let subtitle = about.subtitle.expect("a subtitle");
        assert!(!subtitle.contains("Aniyomi"));
        let info = &*kuro_core::app_info::APP_INFO;
        if info.build_type == "release" {
            assert_eq!(subtitle, format!("Kurobayomi Stable {}", env!("CARGO_PKG_VERSION")));
        } else {
            assert_eq!(subtitle, format!("Debug {}", info.git_sha));
        }
    }

    // the eight rows and their subtitles are the same in the list and in the menu table of settings_ui::menu
    #[test]
    fn the_list_matches_the_menu_table() {
        let table = crate::settings_ui::menu::rows("x");
        let live = entries();
        assert_eq!(live.len(), table.len());
        for (live, row) in live.iter().zip(&table) {
            assert_eq!((live.id.as_str(), live.title.as_str(), live.icon.as_str()), (row.target, row.title, row.icon));
            if row.target != "settings_about" {
                assert_eq!(live.subtitle.as_deref(), Some(row.subtitle.as_str()), "{}", row.title);
            }
        }
    }

    #[test]
    fn t_47b_001_every_router_page_is_a_spec_screen() {
        crate::ui::with_spec(|spec| {
            for page in [Page::Main, Page::Appearance, Page::Inner] {
                assert!(spec.screen(page_screen_id(page)).is_some(), "{page:?}");
            }
        });
    }

    /// Needs a display, and skips itself without one.
    #[test]
    fn t_47b_003_back_walks_the_real_window_stack() {
        crate::testing::with_display(|| {
            let _ = crate::testing::nav();
            let tabs = gtk::Box::new(gtk::Orientation::Vertical, 0);
            tabs.set_widget_name("screen:tabs");
            crate::ui::push(&tabs);

            assert!(!back(), "no shell yet: the caller pops as usual");
            let page = |name: &str| {
                let w = gtk::Box::new(gtk::Orientation::Vertical, 0);
                w.set_widget_name(&format!("screen:{name}"));
                crate::ui::push(&w);
                w
            };
            SHELL.with(|s| s.borrow_mut().open(None, false));
            ROOT.with(|r| *r.borrow_mut() = Some("screen:settings".to_owned()));
            let settings = page("settings");
            let inner = page("settings_about");
            assert!(back(), "an inner page is popped first");
            assert_eq!(crate::ui::visible_child().as_ref(), Some(settings.upcast_ref()));
            assert!(SHELL.with(|s| s.borrow().is_open()), "the shell stays open under it");
            assert!(back(), "then the shell closes");
            assert_eq!(crate::ui::visible_child().as_ref(), Some(tabs.upcast_ref()));
            assert!(!SHELL.with(|s| s.borrow().is_open()));
            assert!(!back(), "and the next back is the caller's");
            drop(inner);
        });
    }
}
