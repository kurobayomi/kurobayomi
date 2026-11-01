//! The expandable description: three lines with a 24 px scrim at the bottom while collapsed. The caret button and the
//! text both toggle it, and the state lasts for the page session.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Align, Box as GBox, Orientation as O};

use crate::entry_core::header::{GenreChip, GenreLayout, genre_layout};
use crate::entry_core::header_view::Handlers;
use crate::entry_core::markdown::to_markup;
use crate::entry_core::text::{COLLAPSED_LINES, Description, Expansion, SCRIM_PX};

/// The scrim: the window background fading in over the last collapsed line.
fn scrim(host: &gtk::Widget) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_height(SCRIM_PX);
    area.set_valign(Align::End);
    area.set_can_target(false);
    let host = host.clone();
    area.set_draw_func(move |_, cr, w, h| {
        #[allow(deprecated)]
        let bg = host.style_context().lookup_color("theme_bg_color").unwrap_or_else(|| gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 1.0));
        let (r, g, b) = (f64::from(bg.red()), f64::from(bg.green()), f64::from(bg.blue()));
        let fade = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, f64::from(h));
        fade.add_color_stop_rgba(0.0, r, g, b, 0.0);
        fade.add_color_stop_rgba(1.0, r, g, b, 1.0);
        let _ = cr.set_source(&fade);
        cr.rectangle(0.0, 0.0, f64::from(w), f64::from(h));
        let _ = cr.fill();
    });
    area
}

/// One genre chip: a button whose popover offers Search (the genre in the entry's own source) and Copy to clipboard.
fn genre_chip(chip: &GenreChip, h: &Handlers) -> gtk::MenuButton {
    let search = gtk::Button::builder().label(kuro_i18n::tr(kuro_i18n::MsgKey::action_search)).css_classes(["flat"]).build();
    let copy = gtk::Button::builder().label(kuro_i18n::tr(kuro_i18n::MsgKey::action_copy_to_clipboard)).css_classes(["flat"]).build();
    let menu = GBox::new(O::Vertical, 2);
    menu.append(&search);
    menu.append(&copy);
    let popover = gtk::Popover::builder().child(&menu).build();
    let button = crate::ui::quiet_menu(gtk::MenuButton::builder().label(&chip.label).popover(&popover).css_classes(["pill"]).build());
    let (on_search, request) = (Rc::clone(&h.on_search), chip.search.clone());
    search.connect_clicked({
        let popover = popover.clone();
        move |_| {
            popover.popdown();
            on_search(request.clone());
        }
    });
    let (on_copy, text) = (Rc::clone(&h.on_copy), chip.copy.clone());
    copy.connect_clicked(move |_| {
        popover.popdown();
        on_copy(text.clone());
    });
    button
}

/// The two layouts of the chips, both built once: a horizontal row that scrolls instead of widening the page, and a
/// wrapping block. `show` picks one from the expansion state (`header::genre_layout`).
struct Genres {
    row: gtk::ScrolledWindow,
    wrap: gtk::FlowBox,
    count: usize,
}

impl Genres {
    fn new(chips: &[GenreChip], h: &Handlers) -> Self {
        let strip = GBox::new(O::Horizontal, 8);
        let wrap = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).homogeneous(false).row_spacing(8).column_spacing(8).build();
        for chip in chips {
            strip.append(&genre_chip(chip, h));
            wrap.insert(&genre_chip(chip, h), -1);
        }
        let row = gtk::ScrolledWindow::builder()
            .child(&strip)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_width(false)
            .build();
        Self { row, wrap, count: chips.len() }
    }

    fn show(&self, expanded: bool) {
        let layout = genre_layout(self.count, expanded);
        self.row.set_visible(layout == GenreLayout::Row);
        self.wrap.set_visible(layout == GenreLayout::Wrap);
    }
}

/// Builds the description and, under it, the genre chips: one row while collapsed, a wrapping block once expanded,
/// nothing without chips. The caret and the text toggle both. `expansion` is shared with the page so the state outlives a
/// rebuild of the widget.
pub fn build(description: &Description, expansion: Rc<Cell<Expansion>>, genres: Option<(&[GenreChip], &Handlers)>) -> GBox {
    let genres = genres.map(|(chips, handlers)| Genres::new(chips, handlers));
    let label = gtk::Label::builder().xalign(0.0).wrap(true).build();
    label.connect_activate_link(|label, uri| {
        crate::ui::open_link(uri, label.root().and_downcast_ref::<gtk::Window>());
        gtk::glib::Propagation::Stop
    });
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&label));
    let fade = scrim(label.upcast_ref());
    overlay.add_overlay(&fade);
    let caret = gtk::Button::builder().css_classes(["flat"]).halign(Align::Center).build();

    let chips = genres.as_ref().map(|g| (g.row.clone(), g.wrap.clone()));
    let genres = genres.map(Rc::new);
    let render = {
        let (label, fade, caret) = (label.clone(), fade.clone(), caret.clone());
        let (expanded, shrunk) = (to_markup(&description.expanded), to_markup(&description.shrunk));
        move |state: Expansion| {
            if state.is_expanded() {
                label.set_markup(&expanded);
                label.set_lines(-1);
                label.set_ellipsize(gtk::pango::EllipsizeMode::None);
                caret.set_icon_name("go-up-symbolic");
            } else {
                label.set_markup(&shrunk);
                label.set_lines(COLLAPSED_LINES);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                caret.set_icon_name("go-down-symbolic");
            }
            fade.set_visible(state.scrim());
            if let Some(genres) = &genres {
                genres.show(state.is_expanded());
            }
        }
    };
    render(expansion.get());
    let toggle = {
        let expansion = Rc::clone(&expansion);
        move || {
            let mut state = expansion.get();
            state.toggle();
            expansion.set(state);
            render(state);
        }
    };
    let on_text = toggle.clone();
    let tap = gtk::GestureClick::new();
    // a click on a link opens the link and leaves the text as it is
    let link = label.clone();
    tap.connect_released(move |_, _, _, _| {
        if link.current_uri().is_none() {
            on_text();
        }
    });
    overlay.add_controller(tap);
    caret.connect_clicked(move |_| toggle());

    // the text never touches the pane's edges, on the manga and the anime page alike
    let root = GBox::new(O::Vertical, 0);
    root.set_margin_start(16);
    root.set_margin_end(16);
    root.append(&overlay);
    root.append(&caret);
    if let Some((row, wrap)) = chips {
        for w in [row.upcast_ref::<gtk::Widget>(), wrap.upcast_ref()] {
            w.set_margin_bottom(8);
            root.append(w);
        }
    }
    root
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;
    use crate::entry_core::header::{SearchRequest, genre_chips};
    use crate::entry_core::text::description;

    fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut out = vec![root.clone()];
        let mut child = root.first_child();
        while let Some(w) = child {
            out.extend(descendants(&w));
            child = w.next_sibling();
        }
        out
    }

    /// One display test: the chips are one scrolling row while the description is collapsed and a wrapping block once
    /// it is expanded, a chip's Search runs the genre in the entry's own source, Copy copies it, and no chips draw no row.
    #[test]
    fn genre_chip_row() {
        crate::testing::with_display(|| {
            let searched: Rc<RefCell<Vec<SearchRequest>>> = Rc::default();
            let copied: Rc<RefCell<Vec<String>>> = Rc::default();
            let handlers = Handlers {
                on_search: Rc::new({
                    let searched = Rc::clone(&searched);
                    move |r| searched.borrow_mut().push(r)
                }),
                on_copy: Rc::new({
                    let copied = Rc::clone(&copied);
                    move |t| copied.borrow_mut().push(t)
                }),
            };
            let text = description(Some("A long story."));
            let chips = genre_chips(Some(&["Action".to_owned(), "Drama".to_owned()]));
            let expansion = Rc::new(Cell::new(Expansion::new(false, false)));
            let root = build(&text, Rc::clone(&expansion), Some((&chips, &handlers)));
            let all = descendants(root.upcast_ref());
            let row = all.iter().find_map(|w| w.clone().downcast::<gtk::ScrolledWindow>().ok()).expect("the collapsed row");
            let wrap = all.iter().find_map(|w| w.clone().downcast::<gtk::FlowBox>().ok()).expect("the wrapping block");
            assert!(row.is_visible() && !wrap.is_visible(), "collapsed: one row");
            let caret = all.iter().find_map(|w| w.clone().downcast::<gtk::Button>().ok().filter(|b| b.icon_name().is_some())).expect("the caret");
            caret.emit_clicked();
            assert!(!row.is_visible() && wrap.is_visible(), "expanded: wrapping");
            // the first chip of the row: its popover has Search and Copy to clipboard
            let chip = descendants(row.upcast_ref()).into_iter().find_map(|w| w.downcast::<gtk::MenuButton>().ok()).expect("a chip");
            assert_eq!(chip.label().as_deref(), Some("Action"));
            let menu = chip.popover().expect("the chip menu");
            let buttons: Vec<gtk::Button> = descendants(menu.upcast_ref()).into_iter().filter_map(|w| w.downcast::<gtk::Button>().ok()).collect();
            let labelled = |text: &str| buttons.iter().find(|b| b.label().as_deref() == Some(text)).cloned();
            labelled("Search").expect("Search").emit_clicked();
            labelled("Copy to clipboard").expect("Copy to clipboard").emit_clicked();
            assert_eq!(*searched.borrow(), vec![SearchRequest { query: "Action".into(), global: false, genre: true }]);
            assert_eq!(*copied.borrow(), vec!["Action".to_owned()]);
            let plain = build(&text, expansion, None);
            assert!(descendants(plain.upcast_ref()).iter().all(|w| w.downcast_ref::<gtk::FlowBox>().is_none()), "no chips, no row");
        });
    }
}
