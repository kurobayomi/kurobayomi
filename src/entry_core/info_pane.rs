//! The two-pane body of the details layout, shared by the manga and the anime page: the info pane (a vertical scroller
//! that is exactly as wide as the route sets it, `constants::left_pane_width`, whatever its content asks for), a separator, and the list. The pane
//! starts hidden; the narrow layout shows the info block above the list instead, and the route puts it there.

use gtk::prelude::*;
use gtk::{Box as GBox, Orientation as O};

use crate::entry_core::constants::LEFT_PANE_WIDTH;

/// The two-pane body: `root` is the row of the pane, the separator and the list. `left` is the content of the pane,
/// where the info block goes in the two-pane layout.
pub struct TwoPane {
    pub root: GBox,
    pub left: GBox,
    pane: gtk::ScrolledWindow,
    separator: gtk::Separator,
}

impl TwoPane {
    /// The body with `right` (the list) on the right side.
    pub fn new(right: &gtk::Widget) -> Self {
        let left = GBox::new(O::Vertical, 0);
        let pane = build(&left);
        let separator = gtk::Separator::new(O::Vertical);
        separator.set_visible(false);
        right.set_hexpand(true);
        let root = GBox::new(O::Horizontal, 0);
        root.set_vexpand(true);
        root.append(&pane);
        root.append(&separator);
        root.append(right);
        Self { root, left, pane, separator }
    }

    /// Shows the pane and its separator for the two-pane layout, hides them for the one-column layout.
    pub fn set_large(&self, large: bool) {
        self.pane.set_visible(large);
        self.separator.set_visible(large);
    }

    /// The width of the pane. Changing it returns true.
    pub fn set_width(&self, width: i32) -> bool {
        if self.pane.width_request() == width {
            return false;
        }
        // the minimum is lifted first, so it is never above the maximum while the two change
        self.pane.set_min_content_width(-1);
        self.pane.set_max_content_width(width);
        self.pane.set_min_content_width(width);
        self.pane.set_width_request(width);
        true
    }

    pub fn width(&self) -> i32 {
        self.pane.width_request()
    }
}

/// The pane around `child`. The horizontal policy is External, not Never: with Never the widest child (a long button
/// label, a chip row) would set the pane's width and the pane would change size when its content changes.
fn build(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .child(child)
        .hscrollbar_policy(gtk::PolicyType::External)
        .width_request(LEFT_PANE_WIDTH)
        .min_content_width(LEFT_PANE_WIDTH)
        .max_content_width(LEFT_PANE_WIDTH)
        .hexpand(false)
        .vexpand(true)
        .visible(false)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pane_keeps_its_width_whatever_the_content_asks_for() {
        crate::testing::with_display(|| {
            let wide = gtk::Label::new(Some(&"a long label that needs more than the pane ".repeat(4)));
            let pane = build(&wide);
            assert_eq!(pane.hscrollbar_policy(), gtk::PolicyType::External);
            assert!(pane.measure(gtk::Orientation::Horizontal, -1).0 <= LEFT_PANE_WIDTH, "the content does not widen it");
        });
    }

    #[test]
    fn the_pane_takes_the_width_it_is_given() {
        crate::testing::with_display(|| {
            let panes = TwoPane::new(gtk::Label::new(None).upcast_ref());
            assert_eq!(panes.width(), LEFT_PANE_WIDTH);
            assert!(panes.set_width(480));
            assert!(!panes.set_width(480), "the same width is no change");
            assert_eq!((panes.width(), panes.pane.min_content_width(), panes.pane.max_content_width()), (480, 480, 480));
            assert!(panes.set_width(LEFT_PANE_WIDTH));
            assert_eq!(panes.pane.max_content_width(), LEFT_PANE_WIDTH);
        });
    }
}
