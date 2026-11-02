//! The season grid of a `Seasons` entry (T-39b-064, T-39a-032 to T-39a-034): one tile per child season in a wrapping grid
//! whose minimum column count follows the entry's grid size setting. The tile rules (title, badges, continue button) and
//! the column rule come from `season_tile`; this widget only draws them. The widget is a `gtk::FlowBox`, so a handful of
//! seasons needs no virtualised list. Upstream: `AnimeScreen.kt` season cells, `AnimeSeasonListItem.kt`.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Align, Box as GBox, Label, Orientation as O};
use kuro_domain::entries::season_display::SeasonDisplayMode;

use super::season_tile::{self, SeasonItem, Tile, edge_badges, list_badges};
use crate::theme::tokens::{SPACE_L, SPACE_M, SPACE_S, SPACE_XS};

/// What decides how the grid draws and how many columns it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub mode: SeasonDisplayMode,
    /// The season titles are "Season {n}" (`SEASON_DISPLAY_MODE_NUMBER`), else the source titles.
    pub number_mode: bool,
    /// The page has a continue callback, so a tile may show the continue button.
    pub has_continue: bool,
    pub columns: u32,
    /// The grid size setting: in list mode the number of visible rows (0 is automatic).
    pub size: i64,
    /// A set size is also the maximum per line; an automatic one is only a minimum.
    pub fixed: bool,
    pub width: i32,
    /// The height of the area the grid fills, for the list-mode row height (T-39a-068).
    pub height: i32,
}

/// The height of one list-mode row: the shared library rule (`library::layout::list_row_height`). Size 0 is the fixed 76 px
/// row, and a size of n shows n rows in the area, never below 48 px, so the size slider changes the visible rows.
pub fn list_row_px(container_height: i32, size: i64) -> i32 {
    crate::library::layout::list_row_height(f64::from(container_height.max(0)), size.clamp(0, season_tile::MAX_GRID_SIZE) as usize).round() as i32
}

impl Layout {
    /// The columns of a grid of this width and size setting (list mode is one column, see `season_tile`).
    pub fn columns_for(width: i32, size: i64, mode: SeasonDisplayMode) -> u32 {
        season_tile::grid_columns(width, size, mode == SeasonDisplayMode::List)
    }
}

pub struct SeasonGrid {
    pub widget: gtk::FlowBox,
    on_open: Rc<dyn Fn(i64)>,
    on_continue: Option<Rc<dyn Fn(i64)>>,
}

impl SeasonGrid {
    pub fn new(on_open: Rc<dyn Fn(i64)>, on_continue: Option<Rc<dyn Fn(i64)>>) -> Self {
        let widget = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .homogeneous(true)
            .row_spacing(SPACE_S.unsigned_abs())
            .column_spacing(SPACE_S.unsigned_abs())
            .margin_start(SPACE_L)
            .margin_end(SPACE_L)
            .margin_bottom(SPACE_S)
            .visible(false)
            .build();
        Self { widget, on_open, on_continue }
    }

    /// Replaces the tiles and applies the layout. List mode gives one full-width row per season.
    pub fn set(&self, items: &[SeasonItem], layout: &Layout) {
        while let Some(child) = self.widget.first_child() {
            self.widget.remove(&child);
        }
        self.widget.set_visible(!items.is_empty());
        self.widget.set_min_children_per_line(layout.columns);
        let max = if layout.fixed { layout.columns } else { (2 * season_tile::MAX_GRID_SIZE as u32).max(layout.columns) };
        self.widget.set_max_children_per_line(max);
        let list = layout.mode == SeasonDisplayMode::List;
        let row_px = list_row_px(layout.height, layout.size);
        let cover_width = if list { 96 } else { ((layout.width - 32 - 8 * (layout.columns as i32 - 1)) / layout.columns.max(1) as i32).clamp(48, 220) };
        for item in items {
            let tile = season_tile::tile(item, layout.mode, layout.number_mode, layout.has_continue);
            let content = if list { self.list_row(&tile, &item.art, row_px) } else { self.grid_tile(&tile, &item.art, cover_width) };
            let button = gtk::Button::builder().child(&content).css_classes(["flat"]).build();
            let (on_open, id) = (Rc::clone(&self.on_open), item.id);
            button.connect_clicked(move |_| on_open(id));
            if let (true, Some(on_continue)) = (tile.continue_button, &self.on_continue) {
                let (on_continue, id) = (Rc::clone(on_continue), item.id);
                let overlay = gtk::Overlay::new();
                overlay.set_child(Some(&button));
                let play = gtk::Button::builder().icon_name("media-playback-start-symbolic").css_classes(["circular", "osd"]).valign(Align::End).halign(Align::End).margin_end(SPACE_S).margin_bottom(SPACE_S).build();
                play.connect_clicked(move |_| on_continue(id));
                overlay.add_overlay(&play);
                self.widget.insert(&overlay, -1);
            } else {
                self.widget.insert(&button, -1);
            }
        }
    }

    fn grid_tile(&self, tile: &Tile, art: &kuro_images::art::ArtRef, cover_width: i32) -> GBox {
        let content = GBox::new(O::Vertical, SPACE_XS);
        // downloads and unseen sit on the start edge of the cover, the language on its end edge (T-39a-033)
        let cover = gtk::Overlay::new();
        cover.set_child(Some(&crate::browse::look::cover(tile.title.as_deref().unwrap_or_default(), Some(art), cover_width, cover_width * 3 / 2)));
        let edges = edge_badges(&tile.badges);
        for (texts, align) in [(edges.start, Align::Start), (edges.end.into_iter().collect(), Align::End)] {
            if texts.is_empty() {
                continue;
            }
            let row = GBox::new(O::Horizontal, 2);
            row.set_halign(align);
            row.set_valign(Align::Start);
            row.set_margin_top(SPACE_XS);
            row.set_margin_start(SPACE_XS);
            row.set_margin_end(SPACE_XS);
            for text in texts {
                row.append(&Label::builder().label(text).css_classes(["caption", "osd"]).build());
            }
            cover.add_overlay(&row);
        }
        content.append(&cover);
        if let Some(title) = &tile.title {
            content.append(&Label::builder().label(title).max_width_chars(14).wrap(true).lines(2).ellipsize(gtk::pango::EllipsizeMode::End).build());
        }
        content
    }

    fn list_row(&self, tile: &Tile, art: &kuro_images::art::ArtRef, row_px: i32) -> GBox {
        let row = GBox::new(O::Horizontal, SPACE_M);
        row.set_height_request(row_px);
        let cover_height = (row_px - 2 * crate::entry_core::list_row::PAD_VERTICAL).max(1);
        row.append(&crate::browse::look::row_cover(tile.title.as_deref().unwrap_or_default(), Some(art), cover_height * 2 / 3, cover_height));
        let text = GBox::new(O::Vertical, SPACE_XS);
        text.set_hexpand(true);
        if let Some(title) = &tile.title {
            text.append(&Label::builder().label(title).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["heading"]).build());
        }
        let badges = list_badges(&tile.badges);
        if !badges.is_empty() {
            text.append(&Label::builder().label(badges.join("  ")).xalign(0.0).css_classes(["caption", "dim-label"]).build());
        }
        row.append(&text);
        row
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_rows_are_76_at_size_zero_and_split_the_area_otherwise() {
        assert_eq!(list_row_px(600, 0), 76);
        assert_eq!(list_row_px(600, 4), 149, "600 / 4 - 3 / 4, rounded");
        assert_eq!(list_row_px(600, 10), 60);
        assert_eq!(list_row_px(300, 10), 48, "never below 48");
        assert!(list_row_px(600, 2) > list_row_px(600, 5), "a bigger size shows more, thinner rows");
    }
}
