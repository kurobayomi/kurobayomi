//! The related-anime section of an anime page (T-39a-030, T-39a-031). Thin: the packing and the item style come from
//! `related_pack`; this only lays them out. Small groups share a three-slot grid row, multi groups scroll sideways.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Align as GAlign, Box as GBox, Button, Grid, Label, Orientation as O, ScrolledWindow};
use kuro_data::anime_relation::AnimeRelationGroup;
use kuro_prefs::library::LibraryDisplayMode;

use super::related_pack::{self as pack, Align, MAX_ITEMS_PER_ROW, RELATED_ITEM_WIDTH};
use crate::theme::tokens::{SPACE_L, SPACE_S, SPACE_XS};

/// One related item: its cover, and its title when the mode shows one.
fn item(entry: &kuro_data::entry::Entry, mode: LibraryDisplayMode, on_open: &Rc<dyn Fn(i64)>) -> Button {
    let (title, favorite, id) = (entry.title.as_str(), entry.favorite, entry.id);
    let style = pack::item_style(mode, favorite);
    let content = GBox::new(O::Vertical, SPACE_XS);
    let cover = crate::browse::look::cover(title, Some(&crate::browse::look::entry_art(entry)), RELATED_ITEM_WIDTH, RELATED_ITEM_WIDTH * 3 / 2);
    cover.set_opacity(style.cover_alpha);
    content.append(&cover);
    if style.show_title {
        let mut label = Label::builder().label(title).xalign(0.0).wrap(true).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(12);
        if let Some(lines) = style.title_max_lines {
            label = label.lines(lines as i32);
        }
        content.append(&label.build());
    }
    if style.in_library_badge {
        content.append(&gtk::Image::from_icon_name("object-select-symbolic"));
    }
    let button = Button::builder().child(&content).css_classes(["flat"]).build();
    let on_open = Rc::clone(on_open);
    button.connect_clicked(move |_| on_open(id));
    button
}

/// The related section, or `None` when there are no groups (`RelatedAnimeRows` returns nothing for an empty list).
pub fn section(groups: &[AnimeRelationGroup], mode: LibraryDisplayMode, on_open: Rc<dyn Fn(i64)>) -> Option<GBox> {
    if groups.is_empty() {
        return None;
    }
    let sizes: Vec<usize> = groups.iter().map(|g| g.anime.len()).collect();
    let (small, multi) = pack::partition(&sizes);
    let root = GBox::new(O::Vertical, SPACE_S);
    root.set_margin_bottom(SPACE_L);

    let small_sizes: Vec<usize> = small.iter().map(|&i| sizes[i]).collect();
    for row in pack::pack_rows(&small_sizes, MAX_ITEMS_PER_ROW) {
        let grid = Grid::builder().column_homogeneous(true).column_spacing(SPACE_XS).margin_start(SPACE_S).margin_end(SPACE_S).build();
        let row_sizes: Vec<usize> = row.iter().map(|&k| small_sizes[k]).collect();
        let starts = pack::start_slots(&row_sizes);
        for (k, &slot) in row.iter().enumerate() {
            let group = &groups[small[slot]];
            let (start, size) = (starts[k], row_sizes[k]);
            let column = GBox::new(O::Vertical, SPACE_XS);
            column.append(&heading(&group.name));
            let items = GBox::new(O::Horizontal, SPACE_XS);
            for entry in &group.anime {
                items.append(&item(entry, mode, &on_open));
            }
            column.append(&items);
            column.set_halign(match pack::align(start, size) {
                Align::Start => GAlign::Start,
                Align::Center => GAlign::Center,
                Align::End => GAlign::End,
            });
            grid.attach(&column, start as i32, 0, size as i32, 1);
        }
        root.append(&grid);
    }

    for index in multi {
        let group = &groups[index];
        let items = GBox::new(O::Horizontal, SPACE_XS);
        items.set_margin_start(SPACE_S);
        items.set_margin_end(SPACE_S);
        for entry in &group.anime {
            items.append(&item(entry, mode, &on_open));
        }
        let scroller = ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Automatic).vscrollbar_policy(gtk::PolicyType::Never).child(&items).build();
        root.append(&heading_with_margin(&group.name));
        root.append(&scroller);
    }
    Some(root)
}

fn heading(text: &str) -> Label {
    Label::builder().label(text).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["heading"]).build()
}

fn heading_with_margin(text: &str) -> Label {
    let label = heading(text);
    label.set_margin_start(SPACE_S);
    label.set_margin_end(SPACE_S);
    label.set_margin_top(SPACE_S);
    label
}
