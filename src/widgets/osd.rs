//! The floating control bar of the player and the reader: one rounded capsule over the video or the pages, GTK's own
//! on-screen-display look (`osd` + `toolbar`, so its buttons are flat and light on the dark background), kept 12 px off
//! the window's edges. One builder, so both bars look the same.

use gtk::prelude::*;

/// How far the bar keeps from the sides and the bottom of the window, in px.
pub const MARGIN: i32 = 12;

/// An empty bar: the caller appends its controls in order.
pub fn bar() -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bar.add_css_class("osd");
    bar.add_css_class("toolbar");
    bar.set_margin_start(MARGIN);
    bar.set_margin_end(MARGIN);
    bar.set_margin_bottom(MARGIN);
    bar.set_valign(gtk::Align::End);
    bar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_is_the_native_on_screen_display_toolbar_off_the_edges() {
        crate::testing::with_display(|| {
            let bar = bar();
            assert!(bar.has_css_class("osd") && bar.has_css_class("toolbar"));
            assert_eq!((bar.margin_start(), bar.margin_end(), bar.margin_bottom()), (MARGIN, MARGIN, MARGIN));
        });
    }
}
