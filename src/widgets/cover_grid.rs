//! The one geometry of a cover grid. The Library and the Browse listing both draw 2:3 covers in cells of the same width, and
//! both count their columns here, so the two screens lay out alike on every GTK version: the app decides how many columns
//! there are and how wide a cell is, and never leaves that to the grid's own measuring (which differs between GTK versions).

use kuro_domain::library::columns::column_width;

/// The margin of a cell on every side, so cells sit 4 px apart.
pub const CELL_MARGIN_PX: i32 = 2;

/// The width of a cover in a cell for the stored `cover_size`: its column width minus the cell margins.
pub fn cover_width(cover_size: i32) -> i32 {
    column_width(cover_size) - 2 * CELL_MARGIN_PX
}

/// Sets the column count of `grid` for a viewport `viewport_px` wide and the stored `cover_size`: as many columns of that
/// size as fit, at least two. Never a forced minimum: a forced count makes the grid demand width, so the window could
/// only grow.
pub fn apply_columns(grid: &gtk::GridView, viewport_px: i32, cover_size: i32) {
    let columns = (viewport_px / column_width(cover_size)).max(2);
    grid.set_min_columns(1);
    grid.set_max_columns(columns as u32);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cover_fills_its_column_less_the_cell_margins_at_every_size() {
        for size in 0..3 {
            assert_eq!(cover_width(size) + 2 * CELL_MARGIN_PX, column_width(size));
        }
    }

    #[test]
    fn the_count_follows_the_viewport_with_at_least_two_columns() {
        crate::testing::with_display(|| {
            let grid = gtk::GridView::new(None::<gtk::NoSelection>, None::<gtk::ListItemFactory>);
            apply_columns(&grid, 1000, 1);
            assert_eq!((grid.min_columns(), grid.max_columns()), (1, 6));
            apply_columns(&grid, 100, 1);
            assert_eq!(grid.max_columns(), 2);
            apply_columns(&grid, 1000, 0);
            assert_eq!(grid.max_columns(), 8, "small covers fit more columns");
            apply_columns(&grid, 1000, 2);
            assert_eq!(grid.max_columns(), 4, "large covers fit fewer");
        });
    }
}
