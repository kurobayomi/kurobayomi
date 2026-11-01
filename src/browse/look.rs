//! The look the Updates, History and Browse tabs share with the Library page (docs/design/other-tabs.md), beside the
//! shared column and heading of `ui`: worded badges and the covers. The loading, empty and error views are `widgets::status`.
//! Stock GTK widgets and theme colours only.

use adw::prelude::*;
use gtk::{Align, Orientation as O};

use kuro_images::art::{ArtKind, ArtRef};
use kuro_platform::kind::Kind;

use crate::library::layout::{self, CoverKind};

/// A badge that says what it counts (`3 updates`, `Installed`). `accent` takes the theme's selected colour.
pub fn badge(text: &str, accent: bool) -> gtk::Label {
    let label = gtk::Label::builder().label(text).valign(Align::Center).css_classes(["kb-badge"]).build();
    if accent {
        label.add_css_class("kb-badge-unread");
    }
    label
}

/// The cover sizes of a list row, width x height (2:3 like the cards): [`SMALL`] in the Updates feed and the Browse
/// lists, [`MEDIUM`] in History and Upcoming, [`BOOK`] for a row that leads with its cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoverSize {
    pub width: i32,
    pub height: i32,
}

pub const SMALL: CoverSize = CoverSize { width: 40, height: 56 };
pub const MEDIUM: CoverSize = CoverSize { width: 53, height: 80 };

/// One `ArtRef` for the cover of an entry: the one place the struct is filled in by name.
/// The cover reference of a stored entry, for a cover drawn outside the library and browse lists (seasons, related titles).
pub fn entry_art(entry: &kuro_data::entry::Entry) -> ArtRef {
    cover_art(entry.kind, entry.id, entry.source, entry.thumbnail_url.clone(), entry.favorite, entry.cover_last_modified)
}

pub fn cover_art(owner: Kind, entry_id: i64, source_id: i64, url: Option<String>, favorite: bool, last_modified: i64) -> ArtRef {
    ArtRef { owner, entry_id, source_id, kind: ArtKind::Cover, url, favorite, last_modified }
}

/// A list row: `cover` (from [`row_cover`] at a [`CoverSize`]) as the prefix, the title, an optional dim subtitle, and an
/// optional `trailing` widget at the end. `widgets::row`'s row, so every list of entries has the same insets.
pub fn list_row(cover: Option<gtk::Widget>, title: &str, subtitle: Option<&str>, trailing: Option<gtk::Widget>) -> adw::ActionRow {
    let row = crate::widgets::row::plain(title, subtitle, trailing.as_ref());
    if let Some(cover) = cover {
        cover.set_valign(Align::Center);
        row.add_prefix(&cover);
    }
    row
}

/// The cover of a list row: [`cover`] with the small rounded corners and initials of a row.
pub fn row_cover(title: &str, art: Option<&ArtRef>, width: i32, height: i32) -> gtk::Widget {
    let cover = cover(title, art, width, height);
    cover.add_css_class("kb-row-cover");
    cover.set_overflow(gtk::Overflow::Hidden);
    cover.set_valign(Align::Center);
    cover
}

/// A cover: the stored image when it can be drawn, else the title's initials on a tile, as the library cards do.
/// Always `width` x `height`.
pub fn cover(title: &str, art: Option<&ArtRef>, width: i32, height: i32) -> gtk::Widget {
    let remote = crate::entry_core::cover_view::service().is_some();
    let cover: gtk::Widget = match layout::cover_kind(art.and_then(|a| a.url.as_deref()), remote) {
        CoverKind::File(url) => crate::local_flow::cover::cover_widget(Some(&url), width, height).upcast(),
        CoverKind::Remote(_) => {
            let view = crate::entry_core::cover_view::CoverView::new(width, height);
            if let Some(art) = art {
                view.load(art.clone());
            }
            view.frame.upcast()
        }
        CoverKind::Initials => {
            let text = gtk::Label::builder().label(layout::initials(title)).vexpand(true).valign(Align::Center).css_classes(["kb-cover-initials-text"]).build();
            let tile = gtk::Box::new(O::Vertical, 0);
            tile.add_css_class("kb-cover-initials");
            tile.set_size_request(width, height);
            tile.append(&text);
            tile.upcast()
        }
    };
    // a fixed size: never grow with the room around it (the initials' vexpand would otherwise bubble up to the page)
    cover.set_halign(Align::Start);
    cover.set_valign(Align::Start);
    cover.set_hexpand(false);
    cover.set_vexpand(false);
    cover
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_without_a_stored_cover_shows_the_initials() {
        crate::testing::with_display(|| {
            let cover = row_cover("Solo Leveling", None, 40, 56);
            assert!(cover.has_css_class("kb-cover-initials") && cover.has_css_class("kb-row-cover"));
            let text = cover.first_child().and_downcast::<gtk::Label>().expect("the initials");
            assert_eq!(text.label(), "SL");
        });
    }
}
