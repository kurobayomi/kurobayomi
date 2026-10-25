//! The GTK pieces the manga and the anime details page build the same way: the loading and missing states, the item
//! header row over the list, the Start or Resume button, the Back button in the header bar, the plain menu rows of
//! the popovers, the scroll-driven label of the button and fade of the header, and moving the info block between layouts.

use std::cell::Cell;

use gtk::prelude::*;
use gtk::{Align, Box as GBox, Button, Label, Orientation as O};
use kuro_i18n::{MsgKey, tr};

use crate::entry_core::details::layout::fab_expanded;
use crate::ui;

/// The stack of a details page with its `loading` child (a spinner) and its `missing` child (not found, with Back). The
/// page adds its `ready` child.
pub fn state_stack() -> gtk::Stack {
    let stack = gtk::Stack::new();
    let spinner = ui::loading(ui::LoadSize::Page);
    let loading = GBox::new(O::Vertical, 12);
    loading.set_halign(Align::Center);
    loading.set_valign(Align::Center);
    loading.append(&spinner);
    loading.append(&Label::new(Some(&tr(MsgKey::loading))));
    stack.add_named(&loading, Some("loading"));
    let missing_page = ui::status("dialog-error-symbolic", &tr(MsgKey::no_results_found));
    let back_missing = Button::with_label(&tr(MsgKey::action_webview_back));
    back_missing.set_halign(Align::Center);
    back_missing.connect_clicked(|_| ui::pop());
    missing_page.append(&back_missing);
    stack.add_named(&missing_page, Some("missing"));
    stack
}

/// The item header over the list (`ItemHeader`): the count and the missing warning. It is a plain row: the list
/// options live in the one menu button of the top bar (`list_options`). `row` has the width and inset of the list's cards,
/// so on a wide window the count stays over the first card instead of at the window's edge.
pub struct ItemHeaderRow {
    pub row: gtk::Widget,
    pub count: Label,
    pub missing: Label,
    /// "Download all", at the right end of the row; the page shows it for an entry that can queue downloads.
    pub download_all: Button,
}

pub fn item_header_row() -> ItemHeaderRow {
    let count = Label::builder().xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["heading"]).build();
    let missing = Label::builder().xalign(1.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["dim-label", "caption"]).build();
    let row = GBox::new(O::Horizontal, 8);
    row.set_margin_top(6);
    row.set_margin_bottom(6);
    // the inset of a card (`list_view`), so the count lines up with the first row
    row.set_margin_start(16);
    row.set_margin_end(16);
    let download_all = Button::builder().label(&tr(MsgKey::ui_download_all)).css_classes(["flat"]).valign(gtk::Align::Center).visible(false).build();
    row.append(&count);
    row.append(&missing);
    row.append(&download_all);
    ItemHeaderRow { row: crate::clamp::clamp(&row, crate::entry_core::constants::LIST_MAX_WIDTH).upcast(), count, missing, download_all }
}

/// The Start or Resume button: the play icon and its label. The page places it over the list.
pub fn fab_button() -> (Button, Label) {
    let label = Label::new(None);
    let content = GBox::new(O::Horizontal, 8);
    content.append(&gtk::Image::from_icon_name("media-playback-start-symbolic"));
    content.append(&label);
    let button = Button::builder().child(&content).css_classes(["suggested-action", "pill"]).build();
    button.set_focus_on_click(false);
    (button, label)
}

/// The header bar and title label of a page built with `ui::page`, and a Back button packed at its start. Back clears a
/// selection before it pops, so the page wires it, not `ui::page`.
pub fn header_with_back(page: &GBox) -> (Option<gtk::HeaderBar>, Option<Label>, Button) {
    let header = page.first_child().and_downcast::<gtk::HeaderBar>();
    let title = header.as_ref().and_then(|h| h.title_widget()).and_downcast::<Label>();
    let back = Button::from_icon_name("go-previous-symbolic");
    back.set_tooltip_text(Some(&tr(MsgKey::action_webview_back)));
    back.set_focus_on_click(false);
    if let Some(header) = &header {
        // Back first, then the page's search and ⋮ menu (`ui::pack_lead`), as on every pushed page
        let lead = std::iter::successors(header.first_child(), |w| w.first_child().or_else(|| w.next_sibling()))
            .find(|w| w.has_css_class(ui::LEAD))
            .and_downcast::<GBox>();
        if let Some(lead) = &lead {
            detach(lead);
        }
        header.pack_start(&back);
        if let Some(lead) = &lead {
            header.pack_start(lead);
        }
        ui::no_sticky_focus(header);
    }
    (header, title, back)
}

/// A row of a menu with no mark: it closes the popover and runs.
pub fn plain_row(label: &str, popover: &gtk::Popover, on_click: impl Fn(&Button) + 'static) -> Button {
    let button = Button::builder().css_classes(["flat"]).build();
    button.set_child(Some(&Label::builder().label(label).xalign(0.0).build()));
    let popover = popover.clone();
    button.connect_clicked(move |b| {
        popover.popdown();
        on_click(b);
    });
    button
}

/// Takes a widget out of the box that holds it, so it can be added to another.
pub fn detach(widget: &impl IsA<gtk::Widget>) {
    if let Some(parent) = widget.parent().and_downcast::<GBox>() {
        parent.remove(widget);
    }
}

/// Whether the label of the Start button shows (`shouldExpandFAB`): at the top, after a backward scroll and at the end
/// of the list. `scroll` keeps the last value and direction between calls.
pub fn fab_label_shown(scroll: &Cell<(f64, bool)>, adj: &gtk::Adjustment) -> bool {
    let value = adj.value();
    let (last, was_backward) = scroll.get();
    let backward = if (value - last).abs() > f64::EPSILON { value < last } else { was_backward };
    scroll.set((value, backward));
    let can_forward = value + adj.page_size() < adj.upper() - 1.0;
    fab_expanded(usize::from(value > 0.0), value as i32, backward, can_forward)
}

/// Fades the title and the header bar background with the scroll in the one-column layout, where the info block
/// (`info_height` tall) is the first thing of the list. `keep_title` shows the title whatever the scroll.
pub fn fade_header(title: Option<&Label>, bar: Option<&gtk::HeaderBar>, narrow: bool, value: f64, info_height: i32, keep_title: bool) {
    let first_visible = value < f64::from(info_height);
    if let Some(label) = title {
        label.set_opacity(if keep_title || title_opaque(narrow, first_visible) { 1.0 } else { 0.0 });
    }
    if let Some(bar) = bar {
        if background_opaque(narrow, first_visible, value > 0.0) {
            bar.remove_css_class("flat");
        } else {
            bar.add_css_class("flat");
        }
    }
}

/// Widget-tree helpers of the details page tests.
#[cfg(test)]
pub(crate) mod test_util {
    use std::time::{Duration, Instant};

    use gtk::glib;
    use gtk::prelude::*;

    /// `root` and every widget below it, in tree order.
    pub fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut out = vec![root.clone()];
        let mut child = root.first_child();
        while let Some(c) = child {
            out.extend(descendants(&c));
            child = c.next_sibling();
        }
        out
    }

    /// Pumps `ctx` until `done` holds or five seconds pass.
    pub fn pump(ctx: &glib::MainContext, done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            while ctx.iteration(false) {}
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        done()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_button_label_hides_on_a_forward_scroll_and_shows_at_the_top_backward_and_at_the_end() {
        crate::testing::with_display(|| {
            let adj = gtk::Adjustment::new(0.0, 0.0, 1000.0, 1.0, 10.0, 100.0);
            let scroll = Cell::new((0.0, false));
            assert!(fab_label_shown(&scroll, &adj), "at the top");
            adj.set_value(300.0);
            assert!(!fab_label_shown(&scroll, &adj), "a forward scroll hides it");
            adj.set_value(200.0);
            assert!(fab_label_shown(&scroll, &adj), "a backward scroll shows it");
            adj.set_value(900.0);
            assert!(fab_label_shown(&scroll, &adj), "nothing left to scroll forward to");
        });
    }
}

/// The title is opaque only in the small layout, once the first item has scrolled off. The large layout is always
/// opaque.
pub fn title_opaque(small_layout: bool, first_item_visible: bool) -> bool {
    !small_layout || !first_item_visible
}

/// The background is opaque in the small layout once the first item is off screen or the list has scrolled.
pub fn background_opaque(small_layout: bool, first_item_visible: bool, scrolled: bool) -> bool {
    !small_layout || !first_item_visible || scrolled
}
