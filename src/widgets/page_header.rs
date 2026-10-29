//! The header of an embedded page, shared by the reader and the player (docs/design/player-reader.md): a themed
//! `HeaderBar` with Back on the left, the title centred, and the page's own buttons on the right. The title text and
//! the label are pure or thin helpers; [`header`] only assembles them.
//!
//! The player and the reader both build their header with [`header`], so the two stay identical; the player adds its
//! quality and tracks menus on the right.

use kuro_i18n::{Arg, MsgKey, tr, trf};
use gtk::prelude::*;

/// What the reader's and the player's bar title: the series (manga or anime) on top, the item (chapter or episode)
/// under it, as the panel stacks "Kurobayomi" over "nightly".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaTitle {
    pub series: String,
    pub item: String,
}

impl MediaTitle {
    pub fn new(series: &str, item: &str) -> Self {
        Self { series: series.to_owned(), item: item.trim().to_owned() }
    }
}

/// A title label in the header: one line, ellipsized, with the `title` style.
pub fn title_label(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).css_classes(["title"]).ellipsize(gtk::pango::EllipsizeMode::End).width_chars(24).max_width_chars(100).build()
}

/// A two-line header title: the series on top and a small dim line under it with the item and, after a dot, a detail
/// (the page position in the reader). The second line keeps its height while it is empty, so the title does not jump.
#[derive(Clone)]
pub struct TwoLineTitle {
    pub root: gtk::Box,
    /// The series line.
    pub title: gtk::Label,
    subtitle: gtk::Label,
    item: std::rc::Rc<std::cell::RefCell<String>>,
    detail: std::rc::Rc<std::cell::RefCell<Option<String>>>,
}

impl TwoLineTitle {
    /// Shows `title` (the player's next episode), keeping the detail.
    pub fn set_title(&self, title: &MediaTitle) {
        self.title.set_label(&title.series);
        *self.item.borrow_mut() = title.item.clone();
        self.show_second_line();
    }

    /// Shows `text` after the item on the second line (`Chapter 3 · Page 1 of 23`), or the item alone for `None`.
    pub fn set_subtitle(&self, text: Option<&str>) {
        *self.detail.borrow_mut() = text.map(str::to_owned);
        self.show_second_line();
    }

    fn show_second_line(&self) {
        let item = self.item.borrow();
        let detail = self.detail.borrow();
        let parts: Vec<&str> = [Some(item.as_str()), detail.as_deref()].into_iter().flatten().filter(|p| !p.is_empty()).collect();
        self.subtitle.set_label(&parts.join(" · "));
    }

    #[cfg(test)]
    pub fn subtitle(&self) -> String {
        self.subtitle.label().to_string()
    }
}

/// A centred two-line title for [`header`]: the series as [`title_label`], the item under it.
pub fn two_line_title(title: &MediaTitle) -> TwoLineTitle {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.set_halign(gtk::Align::Center);
    root.set_valign(gtk::Align::Center);
    let label = title_label(&title.series);
    root.append(&label);
    let subtitle = gtk::Label::builder().css_classes(["subtitle", "dim-label"]).ellipsize(gtk::pango::EllipsizeMode::End).build();
    root.append(&subtitle);
    let two_lines = TwoLineTitle { root, title: label, subtitle, item: Default::default(), detail: Default::default() };
    two_lines.set_title(title);
    two_lines
}

/// `Page 3 of 12`, or `None` when there is no page on screen (a page or total not above zero, or a transition card).
pub fn page_position(page: i32, total: i32) -> Option<String> {
    (page > 0 && total > 0).then(|| trf(MsgKey::page_header_page_of, &[("page", Arg::Str(&page.to_string())), ("total", Arg::Str(&total.to_string()))]))
}

/// The header with Back on the left (`on_back`) and `title` in the centre. The right side is left to the caller: the
/// first widget it packs with `pack_end` sits at the far right.
pub fn header(title: &impl IsA<gtk::Widget>, on_back: impl Fn() + 'static) -> gtk::HeaderBar {
    let header = crate::ui::header_bar();
    crate::ui::window_buttons(&header);
    header.set_title_widget(Some(title.upcast_ref()));
    let back = crate::ui::icon_button("go-previous-symbolic", &tr(MsgKey::action_webview_back));
    back.connect_clicked(move |_| on_back());
    header.pack_start(&back);
    crate::ui::no_sticky_focus(&header);
    header
}

/// The top bar of the reader and the player, one format for both: Back, the two-line title (the series over the item)
/// centred, and nothing clickable in it. The caller packs its own buttons on the right and may add a detail to the
/// second line (the reader's page position).
pub fn media_header(title: &MediaTitle, on_back: impl Fn() + 'static) -> (gtk::HeaderBar, TwoLineTitle) {
    let two_lines = two_line_title(title);
    (header(&two_lines.root, on_back), two_lines)
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn the_position_needs_a_page_and_a_total_above_zero() {
        assert_eq!(page_position(1, 23).as_deref(), Some("Page 1 of 23"));
        assert_eq!(page_position(-1, 23), None, "a transition card has no page");
        assert_eq!(page_position(0, 23), None);
        assert_eq!(page_position(1, 0), None);
    }

    #[test]
    fn the_series_sits_over_the_item_and_its_detail() {
        crate::testing::with_display(|| {
            let title = two_line_title(&MediaTitle::new("Manga", "Chapter 3"));
            assert_eq!((title.title.label().as_str(), title.subtitle().as_str()), ("Manga", "Chapter 3"));
            title.set_subtitle(Some("Page 1 of 23"));
            assert_eq!(title.subtitle(), "Chapter 3 · Page 1 of 23");
            title.set_subtitle(None);
            assert_eq!(title.subtitle(), "Chapter 3");
            title.set_title(&MediaTitle::new("Anime", ""));
            assert_eq!((title.title.label().as_str(), title.subtitle().as_str()), ("Anime", ""), "no item, no dot");
            title.set_subtitle(Some("Page 2 of 5"));
            assert_eq!(title.subtitle(), "Page 2 of 5");
        });
    }

    #[test]
    fn a_blank_item_is_empty() {
        assert_eq!(MediaTitle::new("Manga", "  ").item, "");
    }
}
