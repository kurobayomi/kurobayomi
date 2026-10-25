//! One library card (docs/design/main-window.md): a 2:3 cover carrying worded badges, the title under it (on it in the
//! compact grid, none in the cover-only grid), and the List mode row. What shows is decided by `layout::card_model`;
//! this file only places widgets, and the click handling lives in the view.

use kuro_i18n::{MsgKey, tr};
use kuro_platform::kind::Kind;
use gtk::prelude::*;
use gtk::{Align, Orientation as O};
use kuro_domain::library::page::LibraryItem;
use kuro_images::art::ArtRef;
use kuro_prefs::library::LibraryDisplayMode;

use crate::library::layout::{self, CardModel, TitlePlace, SPACE_XS, SPACE_S};

/// The widgets of one card. `continue_button` is present when the card shows one.
pub struct Card {
    pub root: gtk::Widget,
    pub continue_button: Option<gtk::Button>,
}

/// What a card needs besides the item: the cover art (any entry with a stored thumbnail), its kind,
/// and whether its unread count counts seasons (T-37a-048).
pub struct Extras<'a> {
    pub art: Option<&'a ArtRef>,
    pub kind: Kind,
    pub season_parent: bool,
    /// The width of a grid card: the column width of the stored cover size (`columns::column_width`).
    pub width: i32,
}

/// The cover of a card: the shared cover (`look::cover`), `width` x `height`.
fn cover(model: &CardModel, art: Option<&ArtRef>, width: i32, height: i32) -> gtk::Widget {
    crate::browse::look::cover(&model.title, art, width, height)
}

fn badge(text: &str, tooltip: Option<&str>, accent: bool) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("kb-badge");
    if accent {
        label.add_css_class("kb-badge-unread");
    }
    label.set_tooltip_text(tooltip);
    label
}

/// Unread and downloaded, top start.
fn count_badges(model: &CardModel) -> gtk::Box {
    let group = gtk::Box::new(O::Horizontal, SPACE_XS);
    if let Some(text) = &model.unread {
        group.append(&badge(text, model.unread_tooltip.or(Some(&tr(MsgKey::ui_not_yet_watched_or_read))), true));
    }
    if let Some(text) = &model.downloads {
        group.append(&badge(text, Some("Downloaded"), false));
    }
    group
}

/// Local (icon and word) or the source language, top end.
fn origin_badge(model: &CardModel) -> gtk::Box {
    let group = gtk::Box::new(O::Horizontal, SPACE_XS);
    if model.local {
        group.add_css_class("kb-badge");
        group.set_tooltip_text(Some(&tr(MsgKey::ui_from_a_local_folder)));
        group.append(&gtk::Image::from_icon_name("folder-symbolic"));
        group.append(&gtk::Label::new(Some(&layout::local_badge())));
    } else if let Some(text) = &model.language {
        group.append(&badge(text, Some(&tr(MsgKey::ui_source_language)), false));
    }
    group
}

fn continue_button(mode: LibraryDisplayMode) -> gtk::Button {
    let size = layout::continue_size(mode);
    let button = gtk::Button::from_icon_name("media-playback-start-symbolic");
    button.set_css_classes(&["circular", "osd"]);
    button.set_size_request(size, size);
    button.set_tooltip_text(Some(&tr(MsgKey::action_resume)));
    button
}

fn title_label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class(class);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_lines(2);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_xalign(0.0);
    label
}

/// Builds the grid card of `item`. `selected` marks it; `show_continue` adds the continue button.
pub fn build(item: &LibraryItem, mode: LibraryDisplayMode, selected: bool, show_continue: bool, extras: &Extras) -> Card {
    let model = layout::card_model(item, mode, extras.kind, extras.season_parent);
    let width = extras.width;
    let overlay = gtk::Overlay::new();
    overlay.add_css_class("kb-card-cover");
    overlay.set_overflow(gtk::Overflow::Hidden);
    overlay.set_child(Some(&cover(&model, extras.art, width, layout::cover_height(width))));

    let counts = count_badges(&model);
    counts.set_halign(Align::Start);
    counts.set_valign(Align::Start);
    ui_inset(&counts);
    overlay.add_overlay(&counts);
    let origin = origin_badge(&model);
    origin.set_halign(Align::End);
    origin.set_valign(Align::Start);
    ui_inset(&origin);
    overlay.add_overlay(&origin);
    if model.title_place == TitlePlace::OnCover {
        let title = title_label(&model.title, "kb-card-title-scrim");
        title.set_valign(Align::End);
        overlay.add_overlay(&title);
    }
    let continue_button = show_continue.then(|| {
        let button = continue_button(mode);
        button.set_halign(Align::End);
        button.set_valign(Align::End);
        ui_inset(&button);
        overlay.add_overlay(&button);
        button
    });

    let root = gtk::Box::new(O::Vertical, SPACE_S);
    root.set_size_request(width, -1);
    root.set_halign(Align::Center);
    if selected {
        root.add_css_class("kb-card-selected");
    }
    root.append(&overlay);
    if model.title_place == TitlePlace::Below {
        root.append(&title_label(&model.title, "kb-card-title"));
    }
    Card { root: root.upcast(), continue_button }
}

/// Builds the List mode row of `item` (T-37a-021): a small 2:3 cover, the title, the badges, then the continue
/// button. `row_height` comes from `layout::list_row_height`.
pub fn build_list(item: &LibraryItem, selected: bool, show_continue: bool, row_height: f64, extras: &Extras) -> Card {
    let model = layout::card_model(item, LibraryDisplayMode::List, extras.kind, extras.season_parent);
    let height = (row_height.round() as i32 - 2 * SPACE_XS).max(40);
    let overlay = gtk::Overlay::new();
    overlay.add_css_class("kb-card-cover");
    overlay.set_overflow(gtk::Overflow::Hidden);
    overlay.set_valign(Align::Center);
    overlay.set_child(Some(&cover(&model, extras.art, height * 2 / 3, height)));

    let root = gtk::Box::new(O::Horizontal, layout::SPACE_M);
    root.set_size_request(-1, row_height.round() as i32);
    root.set_margin_start(layout::SPACE_L);
    root.set_margin_end(layout::SPACE_L);
    if selected {
        root.add_css_class("kb-card-selected");
        // a selected row of the list view also takes the selected-row tint (T-49b-029)
        root.add_css_class(crate::theme::style::class::SELECTED);
    }
    root.append(&overlay);
    let title = title_label(&model.title, "kb-card-title");
    title.set_hexpand(true);
    title.set_valign(Align::Center);
    root.append(&title);
    for group in [count_badges(&model), origin_badge(&model)] {
        group.set_valign(Align::Center);
        root.append(&group);
    }
    let continue_button = show_continue.then(|| {
        let button = continue_button(LibraryDisplayMode::List);
        button.set_valign(Align::Center);
        root.append(&button);
        button
    });
    Card { root: root.upcast(), continue_button }
}

fn ui_inset(widget: &impl IsA<gtk::Widget>) {
    crate::ui::margins(widget, SPACE_S - 2);
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_domain::library::columns::COLUMN_WIDTH_PX;

    fn labels(root: &gtk::Widget, out: &mut Vec<String>) {
        let mut child = root.first_child();
        while let Some(w) = child {
            if let Some(l) = w.downcast_ref::<gtk::Label>() {
                out.push(l.label().to_string());
            }
            labels(&w, out);
            child = w.next_sibling();
        }
    }

    fn local_anime() -> LibraryItem {
        let row = kuro_domain::library::model::LibraryRow { title: "Core Concepts".into(), source: 0, ..crate::library::fixtures::row(1, 0) };
        LibraryItem { row, download_count: 0, unread_count: 6, is_local: true, source_language: String::new() }
    }

    #[test]
    fn a_card_without_a_cover_shows_initials_worded_badges_and_the_title_below() {
        crate::testing::with_display(|| {
            let extras = Extras { art: None, kind: Kind::Anime, season_parent: false, width: COLUMN_WIDTH_PX };
            let card = build(&local_anime(), LibraryDisplayMode::ComfortableGrid, false, false, &extras);
            let mut text = vec![];
            labels(&card.root, &mut text);
            assert_eq!(text, ["CC", "6", "Local", "Core Concepts"]);
            assert!(card.continue_button.is_none());
            let cover = card.root.first_child().expect("the cover");
            assert!(cover.has_css_class("kb-card-cover"));
            let tile = cover.first_child().expect("the tile");
            assert!(tile.has_css_class("kb-cover-initials"));
            assert_eq!(tile.size_request(), (COLUMN_WIDTH_PX, layout::cover_height(COLUMN_WIDTH_PX)), "2:3");
            assert_eq!(cover.next_sibling().and_downcast::<gtk::Label>().map(|l| l.label().to_string()).as_deref(), Some("Core Concepts"), "the title sits under the cover");
        });
    }

    #[test]
    fn t_49b_029_a_selected_list_row_takes_the_selected_tint_and_an_unselected_one_none() {
        crate::testing::with_display(|| {
            let extras = Extras { art: None, kind: Kind::Anime, season_parent: false, width: COLUMN_WIDTH_PX };
            let selected = build_list(&local_anime(), true, false, 76.0, &extras);
            assert!(selected.root.has_css_class("kb-selected") && selected.root.has_css_class("kb-card-selected"));
            let plain = build_list(&local_anime(), false, false, 76.0, &extras);
            assert!(!plain.root.has_css_class("kb-selected"));
        });
    }

    #[test]
    fn the_cover_only_grid_has_no_title_and_the_compact_grid_puts_it_on_the_cover() {
        crate::testing::with_display(|| {
            let extras = Extras { art: None, kind: Kind::Manga, season_parent: false, width: COLUMN_WIDTH_PX };
            let bare = build(&local_anime(), LibraryDisplayMode::CoverOnlyGrid, false, true, &extras);
            let mut text = vec![];
            labels(&bare.root, &mut text);
            assert!(!text.iter().any(|t| t == "Core Concepts"));
            let compact = build(&local_anime(), LibraryDisplayMode::CompactGrid, true, false, &extras);
            assert!(compact.root.has_css_class("kb-card-selected"));
            assert!(compact.root.first_child().and_then(|c| c.next_sibling()).is_none(), "nothing under the cover");
            let mut text = vec![];
            labels(&compact.root, &mut text);
            assert!(text.iter().any(|t| t == "Core Concepts"));
            assert!(text.iter().any(|t| t == "6"), "manga words");
        });
    }
}
