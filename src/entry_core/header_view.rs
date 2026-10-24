//! The info header widget over `header.rs`: cover, title, author, artist, status and source. A click on a line runs
//! its search, a right click (a long press on a touch screen) opens Copy and Search for this, and a stub source shows a
//! warning icon in the error color.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Align, Box as GBox, Label, Orientation as O};
use kuro_i18n::{Arg, MsgKey, tr, trf};

use crate::entry_core::action_row::{self, Tone, tone_alpha};
use crate::entry_core::backdrop::TINT_ALPHA;
use crate::entry_core::backdrop_view;
use crate::entry_core::header::{Field, HeaderModel, SearchRequest, cover_size};
use crate::entry_core::cover_view::CoverView;
use crate::widgets::context_menu;

/// Draws a button of the action row in `tone`: the primary colour at full opacity, or the foreground at 0.38 alpha.
pub fn apply_tone(widget: &impl IsA<gtk::Widget>, tone: Tone) {
    widget.set_opacity(tone_alpha(tone));
    if tone == Tone::Primary {
        widget.add_css_class("chapter-primary");
    } else {
        widget.remove_css_class("chapter-primary");
    }
}

/// The library button of an entry page: a heart and "In library" for a favorite, an empty star and "Add to library"
/// otherwise. It is a plain button with an icon and a label as its child, because a button holds either its own
/// label or its own icon, not both. The primary colour marks a favorite, the foreground at 0.38 alpha the rest
/// (`action_row::library`), and a long press opens the category editor, for a favorite only (E-13).
#[derive(Clone)]
pub struct LibraryButton {
    pub button: gtk::Button,
    icon: gtk::Image,
    label: Label,
    content: GBox,
    favorite: Rc<Cell<bool>>,
}

impl LibraryButton {
    pub fn new() -> Self {
        let icon = gtk::Image::new();
        // the same shape as the row's other buttons (icon above a label that may shorten): a wider library label would
        // widen the whole row of equal buttons, and with it the 360 px info pane
        let label = Label::builder().ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(12).build();
        let content = GBox::new(O::Vertical, 2);
        content.set_halign(Align::Center);
        content.append(&icon);
        content.append(&label);
        let button = gtk::Button::builder().child(&content).css_classes(["flat"]).focus_on_click(false).build();
        let this = Self { button, icon, label, content, favorite: Rc::new(Cell::new(false)) };
        this.set(false);
        this
    }

    pub fn set(&self, favorite: bool) {
        self.favorite.set(favorite);
        let shown = action_row::library(favorite);
        self.label.set_label(&kuro_i18n::tr(shown.label));
        // a star pair every stock icon theme ships (Adwaita has no emblem-favorite)
        // the app's own heart pair (bundled), the same on every icon theme
        self.icon.set_icon_name(Some(if shown.filled { "kb-heart-filled-symbolic" } else { "kb-heart-outline-symbolic" }));
        apply_tone(&self.content, shown.tone);
    }

    /// Runs `on_click` on a click and `on_long_press` on a long press. The long press counts only while the entry is a
    /// favorite; then the release that follows it is not a click. Otherwise the press is a plain click.
    pub fn connect(&self, on_click: impl Fn() + 'static, on_long_press: impl Fn() + 'static) {
        let long_fired = Rc::new(Cell::new(false));
        let press = gtk::GestureClick::builder().propagation_phase(gtk::PropagationPhase::Capture).build();
        press.connect_pressed({
            let long_fired = Rc::clone(&long_fired);
            move |_, _, _, _| long_fired.set(false)
        });
        self.button.add_controller(press);
        let long = gtk::GestureLongPress::new();
        long.connect_pressed({
            let (favorite, long_fired) = (Rc::clone(&self.favorite), Rc::clone(&long_fired));
            move |_, _, _| {
                if action_row::library_long_press_opens_categories(favorite.get()) {
                    long_fired.set(true);
                    on_long_press();
                }
            }
        });
        self.button.add_controller(long);
        self.button.connect_clicked(move |_| {
            if !long_fired.replace(false) {
                on_click();
            }
        });
    }
}

/// What the header reports back to the page.
#[derive(Clone)]
pub struct Handlers {
    pub on_search: Rc<dyn Fn(SearchRequest)>,
    pub on_copy: Rc<dyn Fn(String)>,
}

/// A label that searches on click, with the app's right-click menu (a long press on a touch screen) for the fields
/// that have text: Copy, and Search for this across every source for the title, the author and the artist.
fn field_label(model: &HeaderModel, field: Field, text: &str, css: &[&str], xalign: f32, h: &Handlers) -> Label {
    let label = Label::builder().label(text).xalign(xalign).wrap(true).css_classes(css.to_vec()).build();
    if let Some(request) = model.search(field) {
        label.set_cursor_from_name(Some("pointer"));
        let click = gtk::GestureClick::builder().button(1).build();
        let on_search = Rc::clone(&h.on_search);
        click.connect_released(move |_, _, _, _| on_search(request.clone()));
        label.add_controller(click);
    }
    if let Some(copy) = model.copy(field) {
        let (global, h) = (model.search(field).filter(|r| r.global), h.clone());
        context_menu::attach(&label, move |_, _| field_menu(&copy, global.as_ref(), &h));
    }
    label
}

/// The right-click menu of a header line: Copy, then the global search when the line has one.
fn field_menu(copy: &str, global: Option<&SearchRequest>, h: &Handlers) -> Vec<context_menu::Item> {
    let (on_copy, text) = (Rc::clone(&h.on_copy), copy.to_owned());
    let mut items = vec![context_menu::Item::new(tr(MsgKey::action_copy_to_clipboard), 0, move || on_copy(text.clone()))];
    if let Some(request) = global {
        let (on_search, request) = (Rc::clone(&h.on_search), request.clone());
        let label = trf(MsgKey::action_global_search_query, &[("arg1", Arg::Str(&request.query))]);
        items.push(context_menu::Item::new(label, 0, move || on_search(request.clone())));
    }
    items
}

fn icon_row(icon: &str, content: &impl IsA<gtk::Widget>, centered: bool) -> GBox {
    let row = GBox::new(O::Horizontal, 6);
    row.set_halign(if centered { Align::Center } else { Align::Start });
    row.append(&gtk::Image::from_icon_name(icon));
    row.append(content);
    row
}

/// The theme colour behind the header: `TINT_ALPHA` at the top, fading out downward.
fn fade() -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_can_target(false);
    area.set_draw_func(|area, cr, w, h| {
        let color = area.color();
        let (r, g, b) = (f64::from(color.red()), f64::from(color.green()), f64::from(color.blue()));
        let gradient = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, f64::from(h));
        gradient.add_color_stop_rgba(0.0, r, g, b, TINT_ALPHA);
        gradient.add_color_stop_rgba(1.0, r, g, b, 0.0);
        let _ = cr.set_source(&gradient);
        cr.rectangle(0.0, 0.0, f64::from(w), f64::from(h));
        let _ = cr.fill();
    });
    area
}

/// Below this column width the title takes the smaller `title-4` style.
pub const NARROW_TITLE_BELOW: i32 = 480;
/// The title wraps to at most this many lines, then ends with an ellipsis.
pub const TITLE_MAX_LINES: i32 = 3;

/// Builds the header. The small layout puts the cover beside the text, the large layout puts a wider cover above
/// centered text. `column_width` is the width of the column that holds the header. The header sits inside 16 px side
/// margins, over the backdrop: the theme colour fading out and the cover blurred behind it (T-38a-014).
pub fn build(model: &HeaderModel, large: bool, column_width: i32, h: &Handlers) -> gtk::Overlay {
    // the cover sits inside the header's 16 px side margins
    let (w, ht) = cover_size(large, column_width - 32);
    let cover_view = CoverView::new(w, ht);
    if let Some(art) = &model.cover {
        cover_view.load(art.clone());
    }
    // the cover is a plain picture: a click on it does nothing
    let cover = cover_view.frame.clone();
    cover.set_valign(Align::Start);

    let xalign = if large { 0.5 } else { 0.0 };
    let info = GBox::new(O::Vertical, 4);
    info.set_hexpand(true);
    // at the top of the cover: centred, a text taller than the cover spilled over the action row below
    info.set_valign(Align::Center);
    // a narrow column takes a smaller title, and a long one stops at three lines
    let title_style = if !large && column_width < NARROW_TITLE_BELOW { "title-4" } else { "title-2" };
    let title = field_label(model, Field::Title, &model.title, &[title_style], xalign, h);
    title.set_lines(TITLE_MAX_LINES);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    info.append(&title);
    let author = field_label(model, Field::Author, &model.author, &[], xalign, h);
    info.append(&icon_row("avatar-default-symbolic", &author, large));
    if let Some(artist) = &model.artist {
        let artist = field_label(model, Field::Artist, artist, &[], xalign, h);
        info.append(&icon_row("document-edit-symbolic", &artist, large));
    }
    info.append(&icon_row(model.status_icon, &Label::builder().label(&model.status).css_classes(["dim-label"]).build(), large));
    let source_row = GBox::new(O::Horizontal, 6);
    source_row.set_halign(if large { Align::Center } else { Align::Start });
    source_row.append(&gtk::Image::from_icon_name("network-server-symbolic"));
    source_row.append(&field_label(model, Field::Source, &model.source, &["dim-label"], xalign, h));
    if model.stub {
        // the stock `error` class colors the icon like an error label
        source_row.append(&gtk::Image::builder().icon_name("dialog-warning-symbolic").css_classes(["error"]).build());
    }
    info.append(&source_row);

    let root = GBox::new(if large { O::Vertical } else { O::Horizontal }, 16);
    root.set_margin_start(16);
    root.set_margin_end(16);
    root.set_margin_top(16);
    root.set_margin_bottom(12);
    if large {
        cover.set_halign(Align::Center);
    }
    root.append(&cover);
    root.append(&info);
    // the backdrop fills what the header measures; only the root is measured, so the header is as tall as its content
    let layered = gtk::Overlay::new();
    layered.set_child(Some(&fade()));
    layered.add_overlay(&backdrop_view::build(model.cover.as_ref()));
    layered.add_overlay(&root);
    layered.set_measure_overlay(&root, true);
    layered
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// One display test: the button draws a favorite in the primary colour at full opacity and anything else at 0.38,
    /// a long press runs only for a favorite and then swallows the click that follows, and a plain click always runs.
    #[test]
    fn library_button_tone_and_long_press() {
        crate::testing::with_display(|| {
            let button = LibraryButton::new();
            let content = button.button.child().expect("the content box");
            assert!((content.opacity() - 0.38).abs() < 0.01, "not in the library: dimmed (GTK keeps opacity in 8 bits)");
            assert!(!content.has_css_class("chapter-primary"));
            let clicks = Rc::new(Cell::new(0));
            let longs = Rc::new(Cell::new(0));
            button.connect(
                {
                    let clicks = Rc::clone(&clicks);
                    move || clicks.set(clicks.get() + 1)
                },
                {
                    let longs = Rc::clone(&longs);
                    move || longs.set(longs.get() + 1)
                },
            );
            let long_press = || {
                let controllers = button.button.observe_controllers();
                let gesture = (0..controllers.n_items())
                    .filter_map(|i| controllers.item(i).and_downcast::<gtk::GestureLongPress>())
                    .next()
                    .expect("the long press gesture");
                gesture.emit_by_name::<()>("pressed", &[&0.0f64, &0.0f64]);
            };
            long_press();
            assert_eq!(longs.get(), 0, "no long press for an entry outside the library (E-13)");
            button.set(true);
            assert_eq!(content.opacity(), 1.0, "a favorite: full opacity");
            assert!(content.has_css_class("chapter-primary"), "and the primary colour");
            long_press();
            assert_eq!(longs.get(), 1);
            button.button.emit_clicked();
            assert_eq!(clicks.get(), 0, "the release after a long press is not a click");
            button.button.emit_clicked();
            assert_eq!(clicks.get(), 1, "a plain click runs");
        });
    }

    #[test]
    fn a_header_line_menu_copies_and_searches_everywhere_for_the_title_and_the_author_only() {
        let copied: Rc<std::cell::RefCell<Vec<String>>> = Rc::default();
        let searched: Rc<std::cell::RefCell<Vec<SearchRequest>>> = Rc::default();
        let h = Handlers {
            on_search: Rc::new({
                let searched = Rc::clone(&searched);
                move |r: SearchRequest| searched.borrow_mut().push(r)
            }),
            on_copy: Rc::new({
                let copied = Rc::clone(&copied);
                move |t: String| copied.borrow_mut().push(t)
            }),
        };
        let request = SearchRequest { query: "Berserk".into(), global: true, genre: false };
        let items = field_menu("Berserk", Some(&request), &h);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["Copy to clipboard", "Search for \"Berserk\" globally"]);
        for item in &items {
            (item.run)();
        }
        assert_eq!(copied.borrow().as_slice(), ["Berserk".to_owned()]);
        assert_eq!(searched.borrow().as_slice(), [request]);
        assert_eq!(field_menu("Source", None, &h).len(), 1, "the source line only copies");
    }
}
