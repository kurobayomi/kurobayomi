// Generic building blocks: navigation stack, spec item rows, sheets, dialogs, generic screens.
use kuro_i18n::{MsgKey, tr};
use crate::spec::{value_str, Dialog, Group, Item, Screen, Sheet, Spec};
use gtk::prelude::*;
use gtk::{glib, Align, Box as GBox, Button, Label, ListBox, Orientation as O};
use std::cell::OnceCell;
use std::rc::Rc;

thread_local! {
    pub static SPEC: Spec = Spec::load();
    static NAV: OnceCell<(gtk::Stack, gtk::ApplicationWindow)> = const { OnceCell::new() };
}

pub fn with_spec<R>(f: impl FnOnce(&Spec) -> R) -> R {
    SPEC.with(f)
}

pub fn screen(id: &str) -> Screen {
    with_spec(|s| s.screen(id).cloned()).unwrap_or_else(|| Screen { id: id.into(), title: id.into(), ..Default::default() })
}

// ---------- navigation ----------

pub fn nav_init(stack: &gtk::Stack, win: &gtk::ApplicationWindow) {
    NAV.with(|n| n.set((stack.clone(), win.clone())).ok());
}

fn nav() -> (gtk::Stack, gtk::ApplicationWindow) {
    NAV.with(|n| n.get().cloned().expect("nav not initialised"))
}

pub fn stack() -> gtk::Stack {
    nav().0
}

pub fn window() -> gtk::ApplicationWindow {
    nav().1
}

/// The main window, when it exists (a test builds pages without one).
pub fn window_opt() -> Option<gtk::ApplicationWindow> {
    NAV.with(|n| n.get().map(|(_, win)| win.clone()))
}

/// The main window's width, or the size it opens at while it is not shown yet. A page built before its first frame lays
/// out for this, so it never asks for more than the window has (a wide first frame would push a narrow window wider).
pub fn window_width() -> Option<i32> {
    let win = window_opt()?;
    [win.width(), win.default_width()].into_iter().find(|w| *w > 0)
}

thread_local! {
    /// Set inside [`over_root`]: the next push goes straight over the root, and a pop to the root waits for it.
    static OVER_ROOT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `open`, which leaves the pages on screen for one new page (a notification, a link, a launch request): the new
/// page slides in once from the page on screen, and the pages between go. Without it a pop to the root and then a push
/// played two slides. When `open` pushes nothing (a tab) the stack pops to the root as usual.
pub fn over_root(open: impl FnOnce()) {
    OVER_ROOT.with(|flag| flag.set(true));
    open();
    if OVER_ROOT.with(|flag| flag.replace(false)) {
        pop_to_root();
    }
}

pub fn push(page: &impl IsA<gtk::Widget>) {
    if OVER_ROOT.with(|flag| flag.replace(false)) {
        push_over_root(page);
        return;
    }
    let stack = nav().0;
    stack.add_child(page);
    crate::shell::transitions::apply(&stack, crate::shell::transitions::Direction::Push);
    stack.set_visible_child(page);
}

pub fn pop() {
    let stack = nav().0;
    let Some(cur) = stack.visible_child() else { return };
    let Some(prev) = cur.prev_sibling() else { return };
    crate::shell::transitions::apply(&stack, crate::shell::transitions::Direction::Pop);
    stack.set_visible_child(&prev);
    glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || stack.remove(&cur));
}

/// The widget names of the stacked pages from the root up, and the index of the page on show. A page that was just
/// popped stays in the list for a moment, so the index, not the length, says how deep the user is.
pub fn page_stack() -> (Vec<String>, usize) {
    let stack = nav().0;
    let mut names = Vec::new();
    let mut shown = 0;
    let visible = stack.visible_child();
    let mut child = stack.first_child();
    while let Some(c) = child {
        if visible.as_ref() == Some(&c) {
            shown = names.len();
        }
        names.push(c.widget_name().to_string());
        child = c.next_sibling();
    }
    (names, shown)
}

/// Shows the page at `index` of the stack (0 is the root) with one slide back, and drops the pages above it.
pub fn pop_to(index: usize) {
    let stack = nav().0;
    let mut target = stack.first_child();
    for _ in 0..index {
        target = target.and_then(|w| w.next_sibling());
    }
    let Some(target) = target else { return };
    if stack.visible_child().as_ref() == Some(&target) {
        return;
    }
    crate::shell::transitions::apply(&stack, crate::shell::transitions::Direction::Pop);
    stack.set_visible_child(&target);
    glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
        while let Some(w) = target.next_sibling() {
            stack.remove(&w);
        }
    });
}

/// Pushes `page` straight over the root: one slide from the page on screen, then the pages that were between go.
/// (Popping to the root first and then pushing played two slides.)
pub fn push_over_root(page: &impl IsA<gtk::Widget>) {
    let stack = nav().0;
    let between: Vec<gtk::Widget> = std::iter::successors(stack.first_child().and_then(|r| r.next_sibling()), |w| w.next_sibling()).collect();
    push(page);
    glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
        for w in between {
            stack.remove(&w);
        }
    });
}

/// Drop every pushed page and show the root (sidebar destination clicked).
pub fn pop_to_root() {
    // inside `over_root` the push that follows replaces the pages instead (one slide)
    if OVER_ROOT.with(std::cell::Cell::get) {
        return;
    }
    let stack = nav().0;
    let Some(root) = stack.first_child() else { return };
    crate::shell::transitions::apply(&stack, crate::shell::transitions::Direction::Pop);
    stack.set_visible_child(&root);
    while let Some(w) = root.next_sibling() {
        stack.remove(&w);
    }
}

/// Split the window-button layout like GNOME split headers: sidebar gets the left part,
/// content headers the right part.
pub fn decoration(sidebar: bool) -> String {
    let layout = gtk::Settings::default().and_then(|s| s.gtk_decoration_layout()).map(|l| l.to_string()).unwrap_or_default();
    let (left, right) = layout.split_once(':').unwrap_or(("", &layout));
    // the layout may carry the window icon ("icon:close"); the app shows no icon in its headers
    let without_icon = |side: &str| side.split(',').filter(|b| *b != "icon").collect::<Vec<_>>().join(",");
    if sidebar { format!("{}:", without_icon(left)) } else { format!(":{}", without_icon(right)) }
}

/// A clicked button keeps keyboard focus, and some GTK themes paint a focused button like a pressed one, so it looks stuck.
/// Covers `root` and every button under it: call it after the header or bar is packed. The app stylesheet paints a
/// focused button that has no keyboard focus ring as idle (style.css), so this only stops the focus from landing.
pub fn no_sticky_focus(root: &impl IsA<gtk::Widget>) {
    let root = root.as_ref();
    if root.is::<Button>() || root.is::<gtk::MenuButton>() {
        root.set_focus_on_click(false);
    }
    let mut child = root.first_child();
    while let Some(w) = child {
        no_sticky_focus(&w);
        child = w.next_sibling();
    }
}

/// A menu button that does not take focus on a mouse click. Its popover hands focus back to the button when it closes,
/// so every menu button is built through this (the inner toggle button is covered by [`no_sticky_focus`]).
pub fn quiet_menu(menu: gtk::MenuButton) -> gtk::MenuButton {
    no_sticky_focus(&menu);
    menu
}

/// A header bar of the main window. Every one joins one vertical size group, so the sidebar's bar and the page's bar
/// next to it are always the same height, whatever the page puts in its own bar (a search entry is taller than a title).
pub fn header_bar() -> gtk::HeaderBar {
    thread_local! {
        static SAME_HEIGHT: gtk::SizeGroup = gtk::SizeGroup::new(gtk::SizeGroupMode::Vertical);
    }
    let header = gtk::HeaderBar::new();
    SAME_HEIGHT.with(|group| group.add_widget(&header));
    header
}

/// The Anime | Manga switch of a page header, and the row under the header that takes it in a narrow window. Below the
/// width where the sidebar gives way to the bottom bar (`shell::layout::WIDE_MIN_WIDTH_PX`) the header keeps only its
/// buttons and the window buttons, so it fits a window down to the 360 px minimum; the switch then sits centred in its own
/// row, as phone apps put tabs under the toolbar. It also moves there while the sidebar shows once the header runs short
/// of room ([`switch_fits`]): a switch kept in a full header would set the window's minimum width above that breakpoint,
/// and the window could never get narrow. Append the returned row right under the header.
pub fn switch_with_narrow_row(header: &gtk::HeaderBar, switch: &gtk::StackSwitcher) -> GBox {
    let row = GBox::new(O::Horizontal, 0);
    row.set_halign(Align::Center);
    row.set_margin_top(6);
    row.set_margin_bottom(6);
    if let Some(stack) = switch.stack() {
        row.append(&gtk::StackSwitcher::builder().stack(&stack).build());
    }
    // the first frame already shows the right place: a switch in a narrow header pushes the window wider
    let narrow = window_width().is_some_and(|w| w < crate::shell::layout::WIDE_MIN_WIDTH_PX);
    row.set_visible(narrow);
    switch.set_visible(!narrow);
    let (switch, narrow_row) = (switch.clone(), row.clone());
    header.add_tick_callback(move |header, _| {
        // no width yet (the first frames) decides nothing: taken as wide, it put the switch back for a frame and the window
        // grew to fit it
        let Some(width) = header.root().map(|root| root.width()).filter(|w| *w > 0 && header.width() > 0) else { return glib::ControlFlow::Continue };
        // the header's minimum with the switch in it: a hidden widget measures 0, so the row's copy stands in for it
        let shown = switch.is_visible();
        let extra = if shown { 0 } else { narrow_row.measure(O::Horizontal, -1).0 };
        let need = header.measure(O::Horizontal, -1).0 + extra;
        let narrow = width < crate::shell::layout::WIDE_MIN_WIDTH_PX || !switch_fits(header.width(), need, shown);
        if shown == narrow {
            switch.set_visible(!narrow);
            narrow_row.set_visible(narrow);
        }
        glib::ControlFlow::Continue
    });
    row
}

/// Spare width the header keeps while the switch is in it: the switch leaves before the header is full, so the window's
/// minimum width stays below what the window is and the window can still be narrowed.
const SWITCH_ROOM_PX: i32 = 48;
/// The switch comes back only with this much more room again, so a width at the edge does not flip it every frame.
const SWITCH_RETURN_PX: i32 = 24;

/// Whether the switch stays in (or goes back to) a header `width` pixels wide that needs `need` with the switch in it.
fn switch_fits(width: i32, need: i32, shown: bool) -> bool {
    width >= need + SWITCH_ROOM_PX + if shown { 0 } else { SWITCH_RETURN_PX }
}

/// The Back button of a pushed page's header.
pub fn back_button() -> Button {
    let b = Button::from_icon_name("go-previous-symbolic");
    b.set_tooltip_text(Some(&tr(MsgKey::action_webview_back)));
    b.update_property(&[gtk::accessible::Property::Label("Back")]);
    b.connect_clicked(|_| pop());
    b
}

/// The one search of the app: a `gtk::SearchBar` for `entry` and the header toggle that shows it. The toggle is bound to
/// the bar's `search-mode-enabled`; typing anywhere in `page` opens the bar (key capture), Ctrl+F opens it and focuses
/// the entry, and Escape closes it. Closing clears the text, so a filter never stays on behind a hidden bar, and text put
/// in the entry from outside opens the bar. Put the bar under the header and the toggle among the header actions.
pub fn search_row(entry: &gtk::SearchEntry, page: &impl IsA<gtk::Widget>) -> (gtk::SearchBar, gtk::ToggleButton) {
    entry.set_hexpand(true);
    let bar = gtk::SearchBar::new();
    bar.set_child(Some(&clamp(entry, SEARCH_WIDTH)));
    bar.connect_entry(entry);
    bar.set_key_capture_widget(Some(page));
    let tip = kuro_i18n::tr(kuro_i18n::MsgKey::action_search);
    let toggle = gtk::ToggleButton::builder().icon_name("system-search-symbolic").tooltip_text(&tip).build();
    toggle.update_property(&[gtk::accessible::Property::Label(&tip)]);
    bar.bind_property("search-mode-enabled", &toggle, "active").bidirectional().sync_create().build();
    let (weak_bar, weak_entry) = (bar.downgrade(), entry.downgrade());
    let ctrl_f = gtk::Shortcut::new(
        gtk::ShortcutTrigger::parse_string("<Control>f"),
        Some(gtk::CallbackAction::new(move |_, _| {
            if let (Some(bar), Some(entry)) = (weak_bar.upgrade(), weak_entry.upgrade()) {
                bar.set_search_mode(true);
                entry.grab_focus();
            }
            glib::Propagation::Stop
        })),
    );
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.add_shortcut(ctrl_f);
    page.add_controller(shortcuts);
    let weak_entry = entry.downgrade();
    bar.connect_search_mode_enabled_notify(move |bar| {
        if let Some(entry) = weak_entry.upgrade().filter(|_| !bar.is_search_mode()) {
            entry.set_text("");
        }
    });
    let weak_bar = bar.downgrade();
    entry.connect_search_changed(move |entry| {
        if let Some(bar) = weak_bar.upgrade().filter(|_| !entry.text().is_empty()) {
            bar.set_search_mode(true);
        }
    });
    bar.set_search_mode(!entry.text().is_empty());
    (bar, toggle)
}

/// A navigation page: one header row above `content` (docs/design/other-tabs.md). A pushed page (`back`) shows the
/// back button and its title; a sidebar destination shows no title, because the sidebar already names it. When the
/// first `end` widget is a search entry the page gets the shared search ([`search_row`]): the toggle is the first of the
/// actions at the end and the bar sits under the header; a pushed page opens with the bar showing. An Anime | Manga switch (a stack
/// switcher) among `end` goes to the start of the row, after Back, where the Library has its switch.
pub fn page(title: &str, content: &impl IsA<gtk::Widget>, end: Vec<gtk::Widget>, back: bool) -> GBox {
    let header = header_bar();
    header.set_decoration_layout(Some(&decoration(false)));
    let b = GBox::new(O::Vertical, 0);
    let mut end = end;
    let mut bar = None;
    if let Some(entry) = end.first().and_then(|w| w.downcast_ref::<gtk::SearchEntry>()).cloned() {
        end.remove(0);
        let (search_bar, toggle) = search_row(&entry, &b);
        end.insert(0, toggle.upcast());
        if back {
            search_bar.set_search_mode(true);
        }
        bar = Some(search_bar);
    }
    let centre: gtk::Widget = if back {
        Label::builder().label(title).css_classes(["title"]).build().upcast()
    } else {
        // an empty box, not None: without a title widget GTK shows the window title
        GBox::new(O::Horizontal, 0).upcast()
    };
    header.set_title_widget(Some(&centre));
    if back {
        header.pack_start(&back_button());
    }
    // a pushed page leads with its ⋮ menu after Back (`pack_lead`); the search toggle is first of the actions, or leads
    // the row in the phone layout (`search_follows_width`)
    let search = bar.as_ref().map(|_| end.remove(0));
    let menu = end.iter().position(|w| w.has_css_class(MORE_MENU)).map(|i| end.remove(i));
    let lead = (search.is_some() || menu.is_some()).then(|| pack_lead(&header, menu.as_ref()));
    let (switches, end): (Vec<gtk::Widget>, Vec<gtk::Widget>) = end.into_iter().partition(|w| w.is::<gtk::StackSwitcher>());
    let mut narrow_rows = Vec::new();
    for w in &switches {
        header.pack_start(w);
        if let Some(switch) = w.downcast_ref::<gtk::StackSwitcher>() {
            narrow_rows.push(switch_with_narrow_row(&header, switch));
        }
    }
    for w in end.iter().rev() {
        header.pack_end(w);
    }
    if let (Some(search), Some(lead)) = (&search, &lead) {
        let slot = GBox::new(O::Horizontal, 0);
        header.pack_end(&slot);
        search_follows_width(&header, search, lead, &slot);
    }
    no_sticky_focus(&header);
    b.append(&header);
    for row in &narrow_rows {
        b.append(row);
    }
    if let Some(bar) = &bar {
        b.append(bar);
    }
    content.set_vexpand(true);
    b.append(content);
    b
}

thread_local! {
    static LAST_OPENED: std::cell::RefCell<std::collections::HashMap<&'static str, std::time::Instant>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// `true` when `what` (a reader or player open) has not been done in the last `window_ms`; a repeated click, or clicks
/// that queued while the window was busy, then open it once instead of stacking pages.
pub fn first_within(what: &'static str, window_ms: u64) -> bool {
    LAST_OPENED.with(|m| {
        let now = std::time::Instant::now();
        let mut map = m.borrow_mut();
        if map.get(what).is_some_and(|t| now.duration_since(*t).as_millis() < u128::from(window_ms)) {
            return false;
        }
        map.insert(what, now);
        true
    })
}

/// Open a screen by id: hand-built if one exists, otherwise rendered from its spec.
pub fn open(id: &str) {
    #[cfg(test)]
    if OPENED.with(|o| o.borrow_mut().as_mut().map(|v| v.push(id.to_owned())).is_some()) {
        return;
    }
    if with_spec(|s| s.nav.iter().any(|n| n.id == id)) {
        // sidebar destinations switch tabs instead of stacking a second copy; a hidden page (Downloads with an empty
        // queue) shows while it is open
        pop_to_root();
        if let Some(tabs) = nav().0.first_child().and_downcast::<gtk::Stack>() {
            if let Some(page) = tabs.child_by_name(id) {
                page.set_visible(true);
            }
            tabs.set_visible_child_name(id);
        }
        return;
    }
    push(&crate::screens::build(id));
}

#[cfg(test)]
thread_local! {
    static OPENED: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

/// Test hook: runs `body` with [`open`] recording the screen ids instead of navigating, and returns them.
#[cfg(test)]
pub fn opened_during(body: impl FnOnce()) -> Vec<String> {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            OPENED.with(|o| o.borrow_mut().take());
        }
    }
    OPENED.with(|o| *o.borrow_mut() = Some(Vec::new()));
    let _reset = Reset;
    body();
    OPENED.with(|o| o.borrow_mut().clone().unwrap_or_default())
}

// ---------- layout helpers ----------

/// The widest a list page's column gets (settings pages, the boxed lists); wider windows centre it.
pub const PAGE_WIDTH: i32 = 720;
/// The widest a header search entry gets.
pub const SEARCH_WIDTH: i32 = 420;

pub fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let child = child.as_ref();
    let child = if child.has_css_class("column") { clamp(child, PAGE_WIDTH).upcast() } else { child.clone() };
    gtk::ScrolledWindow::builder().child(&child).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build()
}

pub use crate::clamp::clamp;

/// The row of buttons at the top of a page, under its header: choices (the library's categories) or actions (Resume and
/// Clear of the download queue). One spacing and one top margin, so every page's top row looks the same; the caller sets
/// the start inset to line up with its own content.
pub fn pill_row() -> GBox {
    let row = GBox::new(O::Horizontal, 8);
    row.set_halign(Align::Start);
    row.set_margin_top(8);
    row.set_margin_bottom(4);
    row
}

/// Appends one toggle per label to `row` as a group (one active at a time), `active` pressed (none when it matches no
/// index), and returns them in order.
/// The buttons exist before any handler is connected, so setting the active one fires nothing. The library's categories
/// and Browse's Popular | Latest are built with it.
pub fn choice_pills(row: &GBox, labels: &[String], active: usize) -> Vec<gtk::ToggleButton> {
    let mut buttons: Vec<gtk::ToggleButton> = Vec::new();
    for (index, label) in labels.iter().enumerate() {
        let button = gtk::ToggleButton::with_label(label);
        if let Some(first) = buttons.first() {
            button.set_group(Some(first));
        }
        button.set_active(index == active);
        row.append(&button);
        buttons.push(button);
    }
    buttons
}

/// `child` shown at exactly `width` x `height`, whatever its own natural size: an image's size never reaches the layout.
/// The child sits on an overlay over an empty base, so only the requested size counts; it is clipped to it. Pass -1 for
/// both when a parent (a framed cover) sets the size.
pub fn sized(child: &impl IsA<gtk::Widget>, width: i32, height: i32) -> gtk::Overlay {
    let sized = gtk::Overlay::new();
    sized.set_child(Some(&GBox::new(O::Vertical, 0)));
    sized.add_overlay(child);
    sized.set_size_request(width, height);
    sized.set_overflow(gtk::Overflow::Hidden);
    sized
}

/// Opens a web link in the default browser through `gtk::UriLauncher`, the one way the app does it. A launch that fails
/// is logged and shown as a toast.
pub fn open_link(url: &str, parent: Option<&gtk::Window>) {
    open_link_or(url, parent, |error| crate::toast::notify(error));
}

/// [`open_link`] that runs `on_failed` with the error text instead of the toast.
pub fn open_link_or(url: &str, parent: Option<&gtk::Window>, on_failed: impl FnOnce(String) + 'static) {
    let link = url.to_owned();
    // links come from sources too (a title's page, a description): only web and mail links open, never a file or folder
    // of this computer (`file://`) or another program's scheme. The app's own folders open with `open_local`.
    if !web_link(url) {
        tracing::warn!(link = %link, "a link that is not a web link was not opened");
        return;
    }
    gtk::UriLauncher::new(url).launch(parent, gtk::gio::Cancellable::NONE, move |result| {
        if let Err(error) = result {
            tracing::warn!(%error, link = %link, "could not open a link");
            on_failed(error.to_string());
        }
    });
}

/// Whether `url` is a link `open_link` may open: http, https or mailto.
pub fn web_link(url: &str) -> bool {
    url.split_once(':').is_some_and(|(scheme, _)| ["http", "https", "mailto"].iter().any(|s| scheme.eq_ignore_ascii_case(s)))
}

/// Opens a folder or file of the app's own (the Local folder) in its default program, through the platform's opener:
/// GIO's launcher found no program for folders on Windows ("inode/directory") and, inside the AppImage, missed its
/// helper `gio-launch-desktop`. `on_failed` gets the error text. Never for a path a source gave.
pub fn open_local(path: &std::path::Path, on_failed: impl FnOnce(String) + 'static) {
    if let Err(error) = kuro_platform::opener::Opener::open_target(&kuro_platform::opener::SystemOpener, &path.to_string_lossy()) {
        tracing::warn!(%error, path = %path.display(), "could not open a local folder");
        on_failed(error.to_string());
    }
}

pub fn margins(w: &impl IsA<gtk::Widget>, m: i32) {
    w.set_margin_top(m);
    w.set_margin_bottom(m);
    w.set_margin_start(m);
    w.set_margin_end(m);
}

/// Centred column of boxed lists, at most [`PAGE_WIDTH`] wide once [`scrolled`] wraps it. The spacing scale is the
/// Library's (4, 8, 12, 16, 24): 16 px gutters, 12 px between a heading and its list, 24 px between groups.
pub fn column() -> GBox {
    let b = GBox::new(O::Vertical, 12);
    b.set_margin_start(16);
    b.set_margin_end(16);
    b.set_margin_top(16);
    b.set_margin_bottom(24);
    b.add_css_class("column");
    b
}

/// A group heading over a boxed list. With the column's 12 px spacing it leaves 24 px above and 12 px below.
pub fn heading(text: &str) -> Label {
    Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["heading"]).margin_top(12).build()
}

pub fn dim(text: &str) -> Label {
    Label::builder().label(text).xalign(0.0).wrap(true).css_classes(["dim-label", "caption"]).build()
}

pub fn boxed_list() -> ListBox {
    let l = ListBox::new();
    l.set_selection_mode(gtk::SelectionMode::None);
    l.add_css_class("boxed-list");
    l
}

/// Plain case-insensitive substring match of a row title against a search query.
pub fn title_matches(title: &str, query: &str) -> bool {
    title.to_lowercase().contains(&query.trim().to_lowercase())
}

/// The text of the first heading label under `w`: the title of a `browse::look::list_row`.
fn row_title_text(w: &gtk::Widget) -> Option<String> {
    if let Some(label) = w.downcast_ref::<Label>().filter(|l| l.has_css_class("heading")) {
        return Some(label.text().to_string());
    }
    std::iter::successors(w.first_child(), |c| c.next_sibling()).find_map(|c| row_title_text(&c))
}

/// The search entry of a page that lists rows (the page hands it to the shared search, `search_row`): it hides the rows of `list` whose title lacks the typed text
/// (`title_matches`). The filter stays on the list, so rows added later are filtered too. Hand it first in `Built.end`.
pub fn list_search(list: &ListBox) -> gtk::SearchEntry {
    let search = gtk::SearchEntry::builder().placeholder_text(kuro_i18n::tr(kuro_i18n::MsgKey::action_search_hint)).hexpand(true).build();
    let query = Rc::new(std::cell::RefCell::new(String::new()));
    let seen = Rc::clone(&query);
    list.set_filter_func(move |row| row_title_text(row.upcast_ref()).is_none_or(|title| title_matches(&title, &seen.borrow())));
    let list = list.downgrade();
    search.connect_search_changed(move |entry| {
        *query.borrow_mut() = entry.text().to_string();
        if let Some(list) = list.upgrade() {
            list.invalidate_filter();
        }
    });
    search
}

pub fn icon_button(icon: &str, tip: &str) -> Button {
    let b = Button::from_icon_name(if icon.is_empty() { "applications-system-symbolic" } else { icon });
    b.set_tooltip_text(Some(tip));
    b.update_property(&[gtk::accessible::Property::Label(tip)]);
    b
}

/// The row metrics every boxed list shares (this row and `pref_model::widgets::row`): 12 px sides, 8 px top and
/// bottom, 12 px between the parts, at least 48 px tall.
pub const ROW_PAD_H: i32 = 12;
pub const ROW_PAD_V: i32 = 8;
pub const ROW_MIN_HEIGHT: i32 = 48;

/// Title/subtitle row with an optional trailing widget.
pub fn row(title: &str, subtitle: Option<&str>, suffix: Option<gtk::Widget>) -> gtk::ListBoxRow {
    let h = GBox::new(O::Horizontal, 12);
    h.set_margin_start(ROW_PAD_H);
    h.set_margin_end(ROW_PAD_H);
    h.set_margin_top(ROW_PAD_V);
    h.set_margin_bottom(ROW_PAD_V);
    h.set_size_request(-1, ROW_MIN_HEIGHT - 2 * ROW_PAD_V);
    let v = GBox::new(O::Vertical, 2);
    v.set_hexpand(true);
    v.set_valign(Align::Center);
    v.append(&Label::builder().label(title).xalign(0.0).wrap(true).build());
    if let Some(s) = subtitle.filter(|s| !s.is_empty()) {
        v.append(&dim(s));
    }
    h.append(&v);
    if let Some(s) = suffix {
        s.set_valign(Align::Center);
        h.append(&s);
    }
    gtk::ListBoxRow::builder().child(&h).build()
}

/// The dim "opens a page" arrow at the end of a link row.
pub fn chevron() -> gtk::Image {
    gtk::Image::builder().icon_name("go-next-symbolic").css_classes(["dim-label"]).build()
}

/// The title of a list row: one weight (bold `heading`), one ellipsize. Every row builder uses it.
pub fn row_title(text: &str) -> Label {
    Label::builder().label(text).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["heading"]).build()
}

/// The two spinner sizes of the app: inside a button or a row, and as the state of a whole page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadSize {
    Inline,
    Page,
}

/// A spinning spinner. `Inline` is 16 px, for a button or a row. `Page` is 32 px and centred, filling its parent: the
/// "loading" state of a page.
pub fn loading(size: LoadSize) -> gtk::Spinner {
    let px = match size {
        LoadSize::Inline => 16,
        LoadSize::Page => 32,
    };
    let spinner = gtk::Spinner::builder().spinning(true).width_request(px).height_request(px).build();
    if size == LoadSize::Page {
        spinner.set_halign(Align::Center);
        spinner.set_valign(Align::Center);
        spinner.set_hexpand(true);
        spinner.set_vexpand(true);
    }
    spinner
}

/// The bar of a page in selection mode, under its header: Cancel, the count, Select all. `count` is the caller's label
/// (it sets the text as the selection changes); this gives it the look.
pub fn selection_header(count: &Label, on_cancel: impl Fn() + 'static, on_select_all: impl Fn() + 'static) -> GBox {
    use kuro_i18n::{tr, MsgKey};
    let bar = GBox::new(O::Horizontal, 8);
    bar.set_margin_start(16);
    bar.set_margin_end(16);
    bar.set_margin_top(8);
    bar.set_margin_bottom(4);
    let cancel = Button::with_label(&tr(MsgKey::action_cancel));
    cancel.connect_clicked(move |_| on_cancel());
    count.set_hexpand(true);
    count.set_xalign(0.0);
    count.add_css_class("heading");
    let all = Button::with_label(&tr(MsgKey::action_select_all));
    all.connect_clicked(move |_| on_select_all());
    bar.append(&cancel);
    bar.append(count);
    bar.append(&all);
    no_sticky_focus(&bar);
    bar
}

/// The bottom bar of a page in selection mode: the bulk-action `buttons` in a `gtk::ActionBar` inside a Revealer, hidden
/// at first (`set_reveal_child(true)` while something is selected). More buttons than the window is wide scroll sideways
/// instead of widening the window (a hidden bar still counts toward the window's minimum width). [`action_bar_row`] is the
/// row the buttons sit in, for buttons added later.
pub fn action_bar(buttons: &[Button]) -> gtk::Revealer {
    let row = GBox::new(O::Horizontal, 6);
    for b in buttons {
        row.append(b);
    }
    let scroller = gtk::ScrolledWindow::builder().child(&row).hexpand(true).vscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).build();
    let bar = gtk::ActionBar::new();
    bar.pack_start(&scroller);
    no_sticky_focus(&bar);
    gtk::Revealer::builder().child(&bar).transition_type(gtk::RevealerTransitionType::SlideUp).build()
}

/// The row of buttons of an [`action_bar`].
pub fn action_bar_row(bar: &gtk::Revealer) -> GBox {
    fn find(widget: &gtk::Widget) -> Option<GBox> {
        if let Some(scroller) = widget.downcast_ref::<gtk::ScrolledWindow>() {
            let child = scroller.child()?;
            let inner = child.downcast_ref::<gtk::Viewport>().and_then(|v| v.child()).unwrap_or(child);
            return inner.downcast::<GBox>().ok();
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            if let Some(row) = find(&c) {
                return Some(row);
            }
            child = c.next_sibling();
        }
        None
    }
    bar.child().as_ref().and_then(find).expect("an action bar has its row")
}

/// An empty state in the Library's look (docs/design/main-window.md): a large dim icon over the message. A caller with
/// one matching action appends its button to the returned box.
pub fn status(icon: &str, text: &str) -> GBox {
    let b = GBox::new(O::Vertical, 12);
    b.set_valign(Align::Center);
    b.set_halign(Align::Center);
    b.set_vexpand(true);
    b.append(&gtk::Image::builder().icon_name(icon).pixel_size(64).css_classes(["dim-label"]).build());
    b.append(&Label::builder().label(text).wrap(true).justify(gtk::Justification::Center).css_classes(["kb-empty-title"]).build());
    b
}

pub fn linked(labels: &[String], active: usize) -> GBox {
    let b = GBox::new(O::Horizontal, 0);
    b.add_css_class("linked");
    let mut first: Option<gtk::ToggleButton> = None;
    for (i, l) in labels.iter().enumerate() {
        let t = gtk::ToggleButton::with_label(l);
        if let Some(f) = &first {
            t.set_group(Some(f));
        } else {
            first = Some(t.clone());
        }
        t.set_active(i == active);
        b.append(&t);
    }
    b
}

/// The one popover menu of the app: flat rows in a column (spacing 2, margin 6). `rows` builds the rows once the popover
/// exists, so each can close it (`entry_core::page_shell::plain_row`).
pub fn popover_menu(rows: impl FnOnce(&gtk::Popover) -> Vec<Button>) -> gtk::Popover {
    let popover = gtk::Popover::new();
    let column = GBox::new(O::Vertical, 2);
    margins(&column, 6);
    for row in rows(&popover) {
        column.append(&row);
    }
    popover.set_child(Some(&column));
    popover
}

/// One entry of a "more" menu. `when` hides the entry while it returns false, asked each time the menu opens.
pub struct MenuEntry {
    label: String,
    run: std::rc::Rc<dyn Fn()>,
    visible: Option<std::rc::Rc<dyn Fn() -> bool>>,
}

impl MenuEntry {
    pub fn new(label: impl Into<String>, run: impl Fn() + 'static) -> Self {
        Self { label: label.into(), run: std::rc::Rc::new(run), visible: None }
    }

    pub fn when(mut self, visible: impl Fn() -> bool + 'static) -> Self {
        self.visible = Some(std::rc::Rc::new(visible));
        self
    }
}

/// The icon of every page's "more" menu.
pub const OVERFLOW_ICON: &str = "view-more-symbolic";

/// The app's one "more" menu: a native menu (the look of the view-options menu) on a button with the one overflow icon.
/// `sections` are separated by a line. An entry that is not visible when the menu opens is hidden. `None` when there is
/// nothing to show, so a page never gets a menu that does nothing.
pub fn menu_button(sections: Vec<Vec<MenuEntry>>) -> Option<gtk::MenuButton> {
    let (model, group) = (gtk::gio::Menu::new(), gtk::gio::SimpleActionGroup::new());
    let mut conditional = Vec::new();
    let mut n = 0;
    for entries in sections.into_iter().filter(|section| !section.is_empty()) {
        let part = gtk::gio::Menu::new();
        for entry in entries {
            let name = format!("e{n}");
            n += 1;
            let action = gtk::gio::SimpleAction::new(&name, None);
            let run = entry.run;
            action.connect_activate(move |_, _| run());
            group.add_action(&action);
            let item = gtk::gio::MenuItem::new(Some(entry.label.as_str()), Some(&format!("more.{name}")));
            if let Some(visible) = entry.visible {
                // a disabled action hides its item
                item.set_attribute_value("hidden-when", Some(&"action-disabled".to_variant()));
                action.set_enabled(visible());
                conditional.push((action, visible));
            }
            part.append_item(&item);
        }
        model.append_section(None, &part);
    }
    if n == 0 {
        return None;
    }
    let button = quiet_menu(gtk::MenuButton::builder().icon_name(OVERFLOW_ICON).menu_model(&model).tooltip_text(&tr(MsgKey::player_sheets_more_title)).build());
    button.update_property(&[gtk::accessible::Property::Label("More")]);
    button.add_css_class(MORE_MENU);
    button.insert_action_group("more", Some(&group));
    if !conditional.is_empty() {
        button.connect_active_notify(move |_| {
            for (action, visible) in &conditional {
                action.set_enabled(visible());
            }
        });
    }
    Some(button)
}

/// Closes every search bar in `root` (and clears it, as closing does): a search belongs to the view it was made in, so
/// leaving a tab or switching Anime | Manga closes it (user, 2026-10-14).
pub fn close_searches(root: &gtk::Widget) {
    if let Some(bar) = root.downcast_ref::<gtk::SearchBar>() {
        bar.set_search_mode(false);
    }
    let mut child = root.first_child();
    while let Some(c) = child {
        close_searches(&c);
        child = c.next_sibling();
    }
}

/// The refresh button of a page's header (`HeaderActions::refresh`), the one every page builds: the circular arrow, with
/// `tip` saying what it loads again (the Library's updates the library, a title's page fetches its episodes).
pub fn refresh_button(tip: &str) -> gtk::Button {
    icon_button("view-refresh-symbolic", tip)
}

/// How a refresh button shows that its work runs: the arrow turns until it ends, and the button is off meanwhile, so the
/// work starts once. Every page's refresh shows it this way (`style.css`, `.kb-refreshing`).
pub fn set_refreshing(button: &gtk::Button, busy: bool) {
    if busy {
        button.add_css_class("kb-refreshing");
    } else {
        button.remove_css_class("kb-refreshing");
    }
    button.set_sensitive(!busy);
}

/// One turn of a refresh button's arrow, for a refresh that is done at once (reading a list again): it shows the click
/// was taken.
pub fn spin_once(button: &gtk::Button) {
    button.remove_css_class("kb-spin-once");
    button.add_css_class("kb-spin-once");
    let weak = button.downgrade();
    glib::timeout_add_local_once(std::time::Duration::from_millis(SPIN_MS), move || {
        if let Some(button) = weak.upgrade() {
            button.remove_css_class("kb-spin-once");
        }
    });
}

/// One turn of the refresh arrow, in milliseconds (`style.css` uses the same).
const SPIN_MS: u64 = 700;

/// The css class of a page's ⋮ menu ([`menu_button`]), so a pushed page can lead its header with it.
pub const MORE_MENU: &str = "kb-more";
/// The box at the start of a page's header: the search toggle in the phone layout, then a pushed page's ⋮ menu.
pub const LEAD: &str = "kb-lead";

/// Packs a page's lead box, with a pushed page's ⋮ menu (user, 2026-10-14), after Back. A tab page has no ⋮ menu (its
/// actions are buttons at the end). The search toggle comes in first in the phone layout ([`search_follows_width`]).
/// An empty box is hidden, so it takes no spacing. Call after Back and before anything else is packed at the start.
pub fn pack_lead(header: &gtk::HeaderBar, menu: Option<&gtk::Widget>) -> GBox {
    let lead = GBox::new(O::Horizontal, 4);
    lead.add_css_class(LEAD);
    if let Some(menu) = menu {
        lead.append(menu);
    }
    lead.set_visible(lead.first_child().is_some());
    header.pack_start(&lead);
    lead
}

/// Moves `toggle` first into `lead` when `narrow`, else first into `end`, and hides whichever box has nothing showing
/// (a ⋮ menu hidden in a wide window leaves its lead box empty too).
fn put_search(toggle: &gtk::Widget, lead: &GBox, end: &GBox, narrow: bool) {
    let target = if narrow { lead } else { end };
    if toggle.parent().as_ref() != Some(target.upcast_ref()) {
        if let Some(parent) = toggle.parent().and_downcast::<GBox>() {
            parent.remove(toggle);
        }
        // shown first: a child added to a hidden box can come out hidden itself
        target.set_visible(true);
        target.prepend(toggle);
    }
    for side in [lead, end] {
        side.set_visible(std::iter::successors(side.first_child(), |w| w.next_sibling()).any(|w| w.is_visible()));
    }
}

/// A page's search toggle: first in `lead` (the start of the header) in the phone layout, below
/// `shell::layout::WIDE_MIN_WIDTH_PX`, and first in `end` (its actions at the end) in a wider window (user, 2026-10-25).
/// Either box is hidden while empty, so it takes no spacing.
pub fn search_follows_width(header: &gtk::HeaderBar, toggle: &gtk::Widget, lead: &GBox, end: &GBox) {
    let place = {
        let (toggle, lead, end) = (toggle.clone(), lead.clone(), end.clone());
        move |narrow: bool| put_search(&toggle, &lead, &end, narrow)
    };
    place(window_width().is_some_and(|w| w < crate::shell::layout::WIDE_MIN_WIDTH_PX));
    header.add_tick_callback(move |header, _| {
        if let Some(width) = header.root().map(|root| root.width()).filter(|w| *w > 0) {
            place(width < crate::shell::layout::WIDE_MIN_WIDTH_PX);
        }
        glib::ControlFlow::Continue
    });
}

/// What a main page puts in its header, left to right: search, refresh, the page's own buttons, sort/filter, and the
/// "more" menu last. Every page builds its row here, so the order and the buttons mean the same everywhere and only the
/// contents differ.
#[derive(Default)]
pub struct HeaderActions {
    /// The page's search entry; `ui::page` turns it into the shared search toggle.
    pub search: Option<gtk::SearchEntry>,
    /// Updates what the page shows (the whole library, the way it is meant everywhere).
    pub refresh: Option<gtk::Widget>,
    /// Buttons that only this page has.
    pub extra: Vec<gtk::Widget>,
    /// Sort, filter and display options, for a page with a list to shape.
    pub options: Option<gtk::Widget>,
    /// The "more" menu: everything else the page can do, in sections. Pushed pages only: a tab page's actions are
    /// buttons (`extra`), guarded by `shell::lead`'s `no_tab_page_has_a_more_menu`.
    pub menu: Vec<Vec<MenuEntry>>,
}

impl HeaderActions {
    pub fn widgets(self) -> Vec<gtk::Widget> {
        let mut row: Vec<gtk::Widget> = Vec::new();
        row.extend(self.search.map(|s| s.upcast()));
        row.extend(self.refresh);
        row.extend(self.extra);
        row.extend(self.options);
        row.extend(menu_button(self.menu).map(|m| m.upcast()));
        row
    }
}

// ---------- spec items ----------

pub fn item_row(item: &Item) -> Option<gtk::ListBoxRow> {
    Some(match item {
        Item::Switch { title, subtitle, default } => switch_item_row(&crate::spec::shown(title), subtitle.as_deref().map(crate::spec::shown).as_deref(), default.as_bool().unwrap_or(false)),
        Item::List { title, subtitle, options, default } => {
            let labels = crate::spec::shown_all(options);
            let opts: Vec<&str> = labels.iter().map(String::as_str).collect();
            let d = gtk::DropDown::from_strings(&opts);
            let def = value_str(default);
            if let Some(i) = options.iter().position(|o| *o == def) {
                d.set_selected(i as u32);
            }
            row(&crate::spec::shown(title), subtitle.as_deref().map(crate::spec::shown).as_deref(), Some(d.upcast()))
        }
        Item::Multi { title, subtitle, options, default } => {
            let defaults: Vec<String> = default.as_array().map(|a| a.iter().map(value_str).collect()).unwrap_or_default();
            let col = GBox::new(O::Vertical, 2);
            margins(&col, 6);
            for o in options {
                col.append(&gtk::CheckButton::builder().label(crate::spec::shown(o)).active(defaults.contains(o)).build());
            }
            let pop = gtk::Popover::builder().child(&scrolled_max(&col, 360)).build();
            let mb = quiet_menu(gtk::MenuButton::builder().label(format!("{} selected", defaults.len())).popover(&pop).build());
            row(&crate::spec::shown(title), subtitle.as_deref().map(crate::spec::shown).as_deref(), Some(mb.upcast()))
        }
        Item::Slider { title, min, max, step, default, unit } => {
            let (min, max) = (min.unwrap_or(0.0), max.unwrap_or(100.0));
            let step = step.unwrap_or(1.0).max(f64::EPSILON);
            // whole-number ranges use the integer slider (T-49b-020)
            let whole = step == 1.0 && min.fract() == 0.0 && max.fract() == 0.0;
            let sc = if whole {
                crate::widgets::int_slider::IntSlider::new(min as i64, max as i64, default.as_f64().unwrap_or(min) as i64).scale
            } else {
                gtk::Scale::with_range(O::Horizontal, min, max, step)
            };
            sc.set_value(default.as_f64().unwrap_or(min));
            sc.set_width_request(220);
            sc.set_draw_value(true);
            sc.set_digits(if step < 1.0 { 2 } else { 0 });
            row(&crate::spec::shown(title), unit.as_deref(), Some(sc.upcast()))
        }
        Item::Text { title, subtitle, default } => {
            let e = gtk::Entry::builder().text(value_str(default)).width_chars(18).build();
            row(&crate::spec::shown(title), subtitle.as_deref().map(crate::spec::shown).as_deref(), Some(e.upcast()))
        }
        Item::Tristate { title } => {
            // off → include (checked) → exclude (dash), like Aniyomi's TriStateItem
            let t = crate::tri_check::tri_check(kuro_prefs::checkbox::TriState::None(()), |_| {});
            let r = row(&crate::spec::shown(title), None, Some(t.widget().clone().upcast()));
            r.set_activatable(true);
            crate::widgets::row_activate::on_activate(&r, move |_| t.activate());
            r
        }
        Item::Sort { title, options } => {
            let labels = crate::spec::shown_all(options);
            let opts: Vec<&str> = labels.iter().map(String::as_str).collect();
            let h = GBox::new(O::Horizontal, 6);
            h.append(&gtk::DropDown::from_strings(&opts));
            let dir = Button::from_icon_name("view-sort-ascending-symbolic");
            dir.set_tooltip_text(Some(&tr(MsgKey::ui_ascending_descending)));
            dir.connect_clicked(|b| {
                let up = b.icon_name().as_deref() == Some("view-sort-ascending-symbolic");
                b.set_icon_name(if up { "view-sort-descending-symbolic" } else { "view-sort-ascending-symbolic" });
            });
            h.append(&dir);
            row(&crate::spec::shown(title), None, Some(h.upcast()))
        }
        Item::Chips { title, options, default } => {
            let def = value_str(default);
            let active = options.iter().position(|o| *o == def).unwrap_or(0);
            if options.len() <= 4 {
                return Some(row(&crate::spec::shown(title), None, Some(linked(&crate::spec::shown_all(options), active).upcast())));
            }
            // long chip lists (e.g. the 19 app themes) wrap under the title instead of widening the window
            let flow = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(8).column_spacing(4).row_spacing(4).build();
            let mut first: Option<gtk::ToggleButton> = None;
            for (i, o) in options.iter().enumerate() {
                let t = gtk::ToggleButton::builder().label(crate::spec::shown(o)).active(i == active).build();
                match &first { Some(f) => t.set_group(Some(f)), None => first = Some(t.clone()) }
                flow.insert(&t, -1);
            }
            let v = GBox::new(O::Vertical, 8);
            margins(&v, 10);
            v.append(&Label::builder().label(crate::spec::shown(title)).xalign(0.0).build());
            v.append(&flow);
            gtk::ListBoxRow::builder().child(&v).activatable(false).build()
        }
        Item::Link { title, subtitle, target } => {
            let r = row(&crate::spec::shown(title), subtitle.as_deref().map(crate::spec::shown).as_deref(), Some(chevron().upcast()));
            r.set_activatable(true);
            let t = target.clone();
            crate::widgets::row_activate::on_activate(&r, move |_| open(&t));
            r
        }
        Item::Action { title, subtitle } => {
            if crate::app_update::hides_row(title) {
                return None;
            }
            let r = row(&crate::spec::shown(title), subtitle.as_deref().map(crate::spec::shown).as_deref(), None);
            r.set_activatable(true);
            let t = title.clone();
            crate::widgets::row_activate::on_activate(&r, move |r| {
                if t == "Clear WebView data" {
                    crate::settings::clear_webview_data();
                    return;
                }
                // an action whose title names a dialog/sheet/screen of the current spec opens it
                let owner = owner_screen(r.upcast_ref());
                if let Some(id) = owner {
                    if crate::app_update::on_action(&id, &t) {
                        return;
                    }
                    resolve(&id, &t);
                }
            });
            r
        }
        Item::Info { title } => {
            let h = GBox::new(O::Horizontal, 12);
            h.set_margin_start(ROW_PAD_H);
            h.set_margin_end(ROW_PAD_H);
            h.set_margin_top(12);
            h.set_margin_bottom(12);
            h.append(&gtk::Image::from_icon_name("dialog-information-symbolic"));
            let l = dim(&crate::spec::shown(title));
            l.set_hexpand(true);
            h.append(&l);
            gtk::ListBoxRow::builder().child(&h).activatable(false).build()
        }
        Item::Unknown => return None,
    })
}

/// A switch row; a click or Enter flips the switch.
fn switch_item_row(title: &str, subtitle: Option<&str>, default: bool) -> gtk::ListBoxRow {
    let s = gtk::Switch::new();
    s.set_active(default);
    let r = row(title, subtitle, Some(s.clone().upcast()));
    r.set_activatable(true);
    crate::widgets::row_activate::on_activate(&r, move |_| s.set_active(!s.is_active()));
    r
}

fn scrolled_max(child: &impl IsA<gtk::Widget>, h: i32) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder().child(child).max_content_height(h).propagate_natural_height(true).hscrollbar_policy(gtk::PolicyType::Never).build()
}

pub fn groups_into(col: &GBox, groups: &[Group]) {
    for g in groups {
        if let Some(t) = g.title.as_deref().filter(|t| !t.is_empty()) {
            col.append(&heading(&crate::spec::shown(t)));
        }
        let l = boxed_list();
        for it in &g.items {
            if let Some(r) = item_row(it) {
                l.append(&r);
            }
        }
        if l.first_child().is_some() {
            col.append(&l);
        }
    }
}

// ---------- sheets & dialogs ----------

pub fn modal(title: &gtk::Widget, content: &impl IsA<gtk::Widget>, w: i32, h: i32) -> gtk::Window {
    let header = gtk::HeaderBar::new();
    header.set_decoration_layout(Some(&decoration(false)));
    header.set_title_widget(Some(title));
    let win = gtk::Window::builder()
        .transient_for(&window())
        .modal(true)
        .default_width(w)
        .default_height(h)
        .titlebar(&header)
        .child(content)
        .build();
    // the 1px outline of `style.css`, so the dialog's edge shows over a dark window behind it
    win.add_css_class("kb-dialog");
    win
}

pub fn show_sheet(sheet: &Sheet) {
    let stack = gtk::Stack::new();
    for t in &sheet.tabs {
        let col = column();
        groups_into(&col, &t.groups);
        stack.add_titled(&scrolled(&col), None, &crate::spec::shown(&t.title));
    }
    let title: gtk::Widget = if sheet.tabs.len() > 1 {
        gtk::StackSwitcher::builder().stack(&stack).build().upcast()
    } else {
        Label::builder().label(crate::spec::shown(&sheet.title)).css_classes(["title"]).build().upcast()
    };
    modal(&title, &stack, 520, 620).present();
}

pub fn show_dialog(d: &Dialog) {
    show_dialog_then(d, || {});
}

/// How a dialog button looks and acts. `Suggested` and `Destructive` mark the main button, which Enter presses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Normal,
    Suggested,
    Destructive,
}

/// One dialog button: its label, its role and what it does. `run` returns whether the dialog closes after it (a name
/// that is taken keeps the dialog open, for example).
pub struct DialogButton {
    pub label: String,
    pub role: Role,
    pub run: Rc<dyn Fn() -> bool>,
    /// A Cancel button: closing with it counts as dismissing the dialog, like Escape or the close button.
    pub dismisses: bool,
}

impl DialogButton {
    /// A button that runs `run` and closes the dialog.
    pub fn new(label: impl Into<String>, role: Role, run: impl Fn() + 'static) -> Self {
        Self::keep_open_unless(label, role, move || {
            run();
            true
        })
    }

    /// A button that only closes the dialog.
    pub fn cancel(label: impl Into<String>) -> Self {
        Self { dismisses: true, ..Self::new(label, Role::Normal, || {}) }
    }

    /// A button that closes the dialog only when `run` returns true.
    pub fn keep_open_unless(label: impl Into<String>, role: Role, run: impl Fn() -> bool + 'static) -> Self {
        Self { label: label.into(), role, run: Rc::new(run), dismisses: false }
    }
}

/// The one dialog of the app: a title bar, `body` with the standard margins (it scrolls once it is taller than the
/// screen allows), and the buttons in a row at the bottom right. The window is as tall as its content, the main button
/// is the default (Enter), and Escape closes it. Every dialog is built here, so they all look and behave the same.
pub fn dialog(title: &str, body: &impl IsA<gtk::Widget>, width: i32, buttons: Vec<DialogButton>) -> gtk::Window {
    dialog_or_dismiss(title, body, width, buttons, || {})
}

/// [`dialog`] that runs `on_dismiss` once when it closes any way other than through a button that is not a Cancel
/// button: Escape, the close button or Cancel.
pub fn dialog_or_dismiss(title: &str, body: &impl IsA<gtk::Widget>, width: i32, buttons: Vec<DialogButton>, on_dismiss: impl Fn() + 'static) -> gtk::Window {
    let answered = Rc::new(std::cell::Cell::new(false));
    let col = GBox::new(O::Vertical, 12);
    margins(&col, 18);
    col.append(body);
    let row = GBox::new(O::Horizontal, 6);
    row.set_halign(Align::End);
    row.set_margin_top(6);
    col.append(&row);
    let win = modal(&Label::builder().label(title).css_classes(["title"]).build().upcast(), &scrolled_max(&col, 560), width, -1);
    win.set_resizable(false);
    for b in buttons {
        let button = Button::with_label(&b.label);
        match b.role {
            Role::Normal => {}
            Role::Suggested => button.add_css_class("suggested-action"),
            Role::Destructive => button.add_css_class("destructive-action"),
        }
        if b.role != Role::Normal {
            win.set_default_widget(Some(&button));
        }
        let (w, run, answered, dismisses) = (win.clone(), b.run, Rc::clone(&answered), b.dismisses);
        button.connect_clicked(move |_| {
            if run() {
                answered.set(!dismisses);
                w.close();
            }
        });
        row.append(&button);
    }
    let escape = gtk::ShortcutController::new();
    escape.add_shortcut(gtk::Shortcut::new(gtk::ShortcutTrigger::parse_string("Escape"), Some(gtk::NamedAction::new("window.close"))));
    win.add_controller(escape);
    win.connect_close_request(move |_| {
        if !answered.get() {
            on_dismiss();
        }
        glib::Propagation::Proceed
    });
    win
}

/// A dialog of text: [`dialog`] with one wrapped paragraph as its body.
pub fn message(text: &str) -> Label {
    Label::builder().label(text).wrap(true).xalign(0.0).max_width_chars(50).build()
}

/// Shows a spec dialog. Its last button runs `on_ok` and then closes the dialog; the other buttons only close it.
pub fn show_dialog_then(d: &Dialog, on_ok: impl Fn() + 'static) {
    let body = GBox::new(O::Vertical, 12);
    if let Some(t) = &d.text {
        body.append(&message(&crate::spec::shown(t)));
    }
    groups_into(&body, &d.groups);
    let labels = if d.buttons.is_empty() { vec!["OK".to_string()] } else { d.buttons.clone() };
    let n = labels.len();
    let on_ok = Rc::new(on_ok);
    let buttons = labels
        .iter()
        .enumerate()
        .map(|(i, l)| {
            if i + 1 < n {
                return DialogButton::cancel(&crate::spec::shown(l));
            }
            let lower = l.to_lowercase();
            let role = if lower.contains("delete") || lower.contains("remove") { Role::Destructive } else { Role::Suggested };
            let on_ok = Rc::clone(&on_ok);
            DialogButton::new(&crate::spec::shown(l), role, move || on_ok())
        })
        .collect();
    dialog(&crate::spec::shown(&d.title), &body, 420, buttons).present();
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

/// Route a toolbar/menu/action label of screen `id` to the matching sheet, dialog or screen.
pub fn resolve(id: &str, label: &str) -> bool {
    let s = screen(id);
    let n = norm(label);
    if n.is_empty() {
        return false;
    }
    let hit = |a: &str, b: &str| !a.is_empty() && (norm(a) == n || norm(b) == n);
    if let Some(sh) = s.sheets.iter().find(|x| hit(&x.title, &x.id) || x.tabs.iter().any(|t| norm(&t.title) == n)) {
        show_sheet(sh);
        return true;
    }
    if let Some(d) = s.dialogs.iter().find(|x| hit(&x.title, &x.id)) {
        show_dialog(d);
        return true;
    }
    if n.contains("filter") || n.contains("settings") {
        if let Some(sh) = s.sheets.first() {
            show_sheet(sh);
            return true;
        }
    }
    let target = with_spec(|sp| {
        sp.screens.values().find(|x| x.id != id && (norm(&x.title) == n || norm(&x.id) == n)).map(|x| x.id.clone())
    });
    if let Some(t) = target {
        open(&t);
        return true;
    }
    false
}

/// Toolbar buttons + overflow menu for a screen, wired through `resolve`.
pub fn toolbar(s: &Screen) -> Vec<gtk::Widget> {
    let mut out: Vec<gtk::Widget> = vec![];
    for t in &s.toolbar {
        let b = icon_button(&t.icon, &crate::spec::shown(&t.label));
        let (id, l) = (s.id.clone(), t.label.clone());
        b.connect_clicked(move |_| {
            resolve(&id, &l);
        });
        out.push(b.upcast());
    }
    let entries: Vec<MenuEntry> = s
        .menu
        .iter()
        .map(|label| {
            let (id, label) = (s.id.clone(), label.clone());
            MenuEntry::new(crate::spec::shown(&label), move || {
                resolve(&id, &label);
            })
        })
        .collect();
    out.extend(menu_button(vec![entries]).map(|menu| menu.upcast::<gtk::Widget>()));
    out
}

// Screen pages carry their id as the widget name so Action rows can find their owner.
pub fn owner_screen(w: &gtk::Widget) -> Option<String> {
    let mut cur = Some(w.clone());
    while let Some(c) = cur {
        let n = c.widget_name();
        if let Some(id) = n.strip_prefix("screen:") {
            return Some(id.to_string());
        }
        cur = c.parent();
    }
    None
}

pub fn tag(w: &impl IsA<gtk::Widget>, id: &str) {
    w.set_widget_name(&format!("screen:{id}"));
}

/// Render any spec screen: optional tab chips, groups, or empty state.
pub fn generic(s: &Screen) -> GBox {
    let col = column();
    if !s.tabs.is_empty() {
        let t = linked(&crate::spec::shown_all(&s.tabs), 0);
        t.set_halign(Align::Center);
        col.append(&t);
    }
    groups_into(&col, &s.groups);
    let content: gtk::Widget = if s.groups.is_empty() {
        let b = GBox::new(O::Vertical, 0);
        b.append(&col);
        b.append(&status("dialog-information-symbolic", &crate::spec::shown(s.empty.as_deref().unwrap_or(&s.title))));
        b.upcast()
    } else {
        scrolled(&col).upcast()
    };
    let p = page(&crate::spec::shown(&s.title), &content, toolbar(s), true);
    tag(&p, &s.id);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_leads_the_row_in_the_phone_layout_and_is_first_of_the_actions_otherwise() {
        crate::testing::with_display(|| {
            let (lead, end) = (GBox::new(O::Horizontal, 4), GBox::new(O::Horizontal, 4));
            let (toggle, refresh) = (gtk::ToggleButton::builder().visible(true).build(), Button::builder().visible(true).build());
            end.append(&refresh);
            // a pushed page's ⋮ menu, hidden in a wide window: it leaves the lead box with nothing showing
            lead.append(&gtk::MenuButton::builder().visible(false).build());
            let toggle = toggle.upcast::<gtk::Widget>();
            put_search(&toggle, &lead, &end, false);
            assert_eq!(end.first_child().as_ref(), Some(&toggle), "first of the actions in a wide window");
            assert!(!lead.is_visible(), "a lead box with nothing showing takes no spacing");
            put_search(&toggle, &lead, &end, true);
            assert_eq!(lead.first_child().as_ref(), Some(&toggle), "at the start of the row on a phone");
            assert!(lead.is_visible() && end.is_visible(), "the actions keep their other buttons");
            assert_eq!(end.first_child().as_ref(), Some(refresh.upcast_ref()));
            put_search(&toggle, &lead, &end, false);
            assert_eq!(end.first_child().as_ref(), Some(&toggle), "back at the end when the window widens");
        });
    }

    #[test]
    fn the_switch_leaves_a_header_short_of_room_and_comes_back_with_room_to_spare() {
        // in the header: it stays while the header has its spare room, and leaves just before, so the window can shrink
        assert!(switch_fits(600 + SWITCH_ROOM_PX, 600, true));
        assert!(!switch_fits(600 + SWITCH_ROOM_PX - 1, 600, true));
        // under the header: back only with the extra room, so a width at the edge does not flip it every frame
        assert!(!switch_fits(600 + SWITCH_ROOM_PX, 600, false));
        assert!(switch_fits(600 + SWITCH_ROOM_PX + SWITCH_RETURN_PX, 600, false));
    }

    /// A link a source hands over opens only when it is a web or mail link, never a file or folder of this computer.
    #[test]
    fn only_web_and_mail_links_open() {
        for ok in ["https://site.test/a", "http://site.test", "HTTPS://Site.test", "mailto:a@b.test"] {
            assert!(web_link(ok), "{ok}");
        }
        for refused in ["file:///home/u/.ssh/id_rsa", "FILE:///etc", "smb://host/share", "javascript:alert(1)", "kurobayomi://add-repo?url=x", "/home/u", ""] {
            assert!(!web_link(refused), "{refused}");
        }
    }

    #[test]
    fn over_root_replaces_the_pages_with_one_push_or_pops_when_nothing_is_pushed() {
        crate::testing::with_display(|| {
            let (stack, _) = crate::testing::nav();
            if stack.first_child().is_none() {
                stack.add_child(&gtk::Label::new(Some("root")));
            }
            pop_to_root();
            let (a, b) = (gtk::Label::new(Some("a")), gtk::Label::new(Some("b")));
            push(&a);
            over_root(|| {
                pop_to_root();
                push(&b);
            });
            assert_eq!(stack.visible_child().as_ref(), Some(b.upcast_ref::<gtk::Widget>()), "straight to the new page");
            assert_eq!(stack.transition_type(), crate::shell::transitions::spec(crate::shell::transitions::Direction::Push, crate::shell::transitions::animations_enabled()).kind);
            over_root(|| {});
            assert_eq!(page_stack().1, 0, "nothing pushed: back at the root");
        });
    }

    #[test]
    fn a_list_search_hides_rows_whose_title_lacks_the_text_ignoring_case() {
        assert!(title_matches("One Piece", "piece") && title_matches("One Piece", " ONE ") && title_matches("x", ""));
        assert!(!title_matches("One Piece", "bleach"));
        crate::testing::with_display(|| {
            let list = boxed_list();
            let rows: Vec<_> = ["One Piece", "Bleach"].map(|t| crate::browse::look::list_row(None, t, Some("Piece of cake"), None)).into();
            rows.iter().for_each(|r| list.append(r));
            let search = list_search(&list);
            let visible = || rows.iter().map(|r| r.is_child_visible()).collect::<Vec<_>>();
            let type_in = |text: &str| {
                search.set_text(text);
                search.emit_by_name::<()>("search-changed", &[]);
            };
            type_in("PIECE");
            assert_eq!(visible(), [true, false], "only the title counts, not the subtitle");
            type_in("");
            assert_eq!(visible(), [true, true]);
        });
    }

    /// Every widget at or below `root`.
    fn all(root: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
        out.push(root.clone());
        let mut child = root.first_child();
        while let Some(w) = child {
            all(&w, out);
            child = w.next_sibling();
        }
    }

    fn labels(root: &gtk::Widget) -> Vec<String> {
        let mut found = Vec::new();
        all(root, &mut found);
        found.iter().filter_map(|w| w.downcast_ref::<Label>().map(|l| l.label().to_string())).collect()
    }

    #[test]
    fn the_shared_dialog_puts_cancel_then_the_main_button_and_tells_a_dismiss_from_an_answer() {
        crate::testing::with_display(|| {
            let _nav = crate::testing::nav();
            let (ran, dismissed) = (Rc::new(std::cell::Cell::new(0)), Rc::new(std::cell::Cell::new(0)));
            let open = || {
                let (ran, dismissed) = (Rc::clone(&ran), Rc::clone(&dismissed));
                let buttons = vec![DialogButton::cancel("Cancel"), DialogButton::new("Add", Role::Suggested, move || ran.set(ran.get() + 1))];
                dialog_or_dismiss("Add category", &message("Body"), 380, buttons, move || dismissed.set(dismissed.get() + 1))
            };
            let buttons_of = |win: &gtk::Window| {
                let mut found = Vec::new();
                all(win.upcast_ref(), &mut found);
                found.into_iter().filter_map(|w| w.downcast::<Button>().ok()).filter(|b| b.label().is_some()).collect::<Vec<_>>()
            };

            // a window that was never shown has nothing to close, so each one is presented like the app does
            let win = open();
            win.present();
            assert!(!win.is_resizable() && win.is_modal());
            let buttons = buttons_of(&win);
            assert_eq!(buttons.iter().map(|b| b.label().unwrap().to_string()).collect::<Vec<_>>(), ["Cancel", "Add"]);
            assert!(buttons[1].has_css_class("suggested-action"));
            assert_eq!(win.default_widget().as_ref(), Some(buttons[1].upcast_ref::<gtk::Widget>()), "Enter presses the main button");
            buttons[1].emit_clicked();
            assert_eq!((ran.get(), dismissed.get()), (1, 0), "the answer runs and is not a dismiss");

            let win = open();
            win.present();
            buttons_of(&win)[0].emit_clicked();
            assert_eq!((ran.get(), dismissed.get()), (1, 1), "Cancel dismisses");

            let win = open();
            win.present();
            win.close();
            assert_eq!((ran.get(), dismissed.get()), (1, 2), "the close button dismisses");
        });
    }

    #[test]
    fn a_sidebar_page_has_no_title_and_a_pushed_page_has_its_title_and_back() {
        crate::testing::with_display(|| {
            let root = page("Updates", &GBox::new(O::Vertical, 0), vec![], false);
            let header = root.first_child().and_downcast::<gtk::HeaderBar>().expect("the header row");
            assert!(!labels(header.upcast_ref()).iter().any(|l| l == "Updates"), "the sidebar names the tab");
            assert!(header.title_widget().is_some(), "an empty title widget keeps GTK from showing the window title");
            let pushed = page("Appearance", &GBox::new(O::Vertical, 0), vec![], true);
            let header = pushed.first_child().expect("the header row");
            assert!(labels(&header).iter().any(|l| l == "Appearance"));
            let mut w = Vec::new();
            all(&header, &mut w);
            assert!(w.iter().any(|w| w.downcast_ref::<Button>().is_some_and(|b| b.icon_name().as_deref() == Some("go-previous-symbolic"))));
        });
    }

    #[test]
    fn a_search_entry_handed_first_becomes_a_toggle_in_the_header_and_a_bar_under_it() {
        crate::testing::with_display(|| {
            let search = gtk::SearchEntry::new();
            let refresh = icon_button("view-refresh-symbolic", "Refresh");
            let p = page("Updates", &GBox::new(O::Vertical, 0), vec![search.clone().upcast(), refresh.upcast()], false);
            let header = p.first_child().and_downcast::<gtk::HeaderBar>().expect("the header row");
            let bar = header.next_sibling().and_downcast::<gtk::SearchBar>().expect("the bar sits under the header");
            assert!(search.is_ancestor(&bar) && !search.is_ancestor(&header), "the entry is in the bar, not the header");
            let mut w = Vec::new();
            all(header.upcast_ref(), &mut w);
            let toggle = w.iter().find_map(|w| w.downcast_ref::<gtk::ToggleButton>().filter(|b| b.icon_name().as_deref() == Some("system-search-symbolic"))).expect("the search toggle");
            assert_eq!(toggle.tooltip_text().as_deref(), Some("Search"));
            assert!(!bar.is_search_mode(), "closed to start with");
            // the toggle opens the bar, and Escape in the entry (its stop-search) closes it, clearing the text
            toggle.set_active(true);
            assert!(bar.is_search_mode(), "the toggle opens the bar");
            search.set_text("one");
            search.emit_by_name::<()>("stop-search", &[]);
            assert!(!bar.is_search_mode() && !toggle.is_active(), "Escape closes the bar");
            assert_eq!(search.text(), "", "closing clears the filter text");
            // text put in from outside opens it
            search.set_text("two");
            search.emit_by_name::<()>("search-changed", &[]);
            assert!(bar.is_search_mode(), "text in the entry shows the bar");
            // a pushed page opens with the bar showing
            let pushed = page("Search settings", &GBox::new(O::Vertical, 0), vec![gtk::SearchEntry::new().upcast()], true);
            assert!(pushed.first_child().and_then(|h| h.next_sibling()).and_downcast::<gtk::SearchBar>().is_some_and(|b| b.is_search_mode()));
        });
    }
}
