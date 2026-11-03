//! Selection engine of the Updates lists (behaviour 2.5, T-42a-006). The selected flags live on the rows; this keeps
//! only the two bounds (`positions[0..1]`), so a stale id set cannot exist (EC-42-U-04). The bounds are recomputed
//! with [`Selection::sync`] whenever the list is replaced (EC-42-U-05, `feed::replace_rows`). Indices are positions in the row list; headers are not rows.

use std::ops::Range;

/// A row that can be selected. The engine reads and writes the flag and never sees the rest of the row.
pub trait Selectable {
    fn id(&self) -> i64;
    fn selected(&self) -> bool;
    fn set_selected(&mut self, selected: bool);
}

/// The bounds of a selection: the first and last selected index, or `-1` when unset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    bounds: [isize; 2],
}

impl Default for Selection {
    fn default() -> Self {
        Self { bounds: [-1, -1] }
    }
}

impl Selection {
    #[cfg(test)]
    pub fn bounds(&self) -> [isize; 2] {
        self.bounds
    }

    /// Recomputes the bounds from the selected flags (the list was replaced).
    pub fn sync<T: Selectable>(&mut self, items: &[T]) {
        self.bounds = bounds_of(items);
    }

    /// `toggleSelection`: returns the indices whose flag changed. An id that is not in the list is a no-op.
    pub fn toggle<T: Selectable>(
        &mut self,
        items: &mut [T],
        id: i64,
        selected: bool,
        user_initiated: bool,
        from_long_press: bool,
    ) -> Vec<usize> {
        let Some(index) = items.iter().position(|x| x.id() == id) else {
            return Vec::new();
        };
        if items[index].selected() == selected {
            return Vec::new();
        }
        let first_selection = !items.iter().any(|x| x.selected());
        items[index].set_selected(selected);
        let mut changed = vec![index];
        let at = index as isize;
        if selected && user_initiated && from_long_press {
            let range = if first_selection {
                self.bounds = [at, at];
                Range { start: index, end: index }
            } else if at < self.bounds[0] {
                let range = Range { start: index + 1, end: (self.bounds[0] as usize).min(items.len()) };
                self.bounds[0] = at;
                range
            } else if at > self.bounds[1] {
                let range = Range { start: (self.bounds[1] + 1) as usize, end: index };
                self.bounds[1] = at;
                range
            } else {
                Range { start: index, end: index }
            };
            for i in range {
                if !items[i].selected() {
                    items[i].set_selected(true);
                    changed.push(i);
                }
            }
        } else if user_initiated && !from_long_press {
            if !selected {
                if at == self.bounds[0] {
                    self.bounds[0] = first_index(items).map_or(-1, |i| i as isize);
                } else if at == self.bounds[1] {
                    self.bounds[1] = last_index(items).map_or(-1, |i| i as isize);
                }
            } else if at < self.bounds[0] {
                self.bounds[0] = at;
            } else if at > self.bounds[1] {
                self.bounds[1] = at;
            }
        }
        changed
    }

    /// `toggleAllSelection`: every row gets the flag, and the bounds reset to `-1`.
    pub fn toggle_all<T: Selectable>(&mut self, items: &mut [T], selected: bool) {
        for item in items.iter_mut() {
            item.set_selected(selected);
        }
        self.bounds = [-1, -1];
    }

    /// `invertSelection`: every flag flips, and the bounds reset to `-1`.
    pub fn invert<T: Selectable>(&mut self, items: &mut [T]) {
        for item in items.iter_mut() {
            let flipped = !item.selected();
            item.set_selected(flipped);
        }
        self.bounds = [-1, -1];
    }
}

/// The selection mode is on while any row is selected.
pub fn is_selection_mode<T: Selectable>(items: &[T]) -> bool {
    items.iter().any(|x| x.selected())
}

fn bounds_of<T: Selectable>(items: &[T]) -> [isize; 2] {
    [first_index(items).map_or(-1, |i| i as isize), last_index(items).map_or(-1, |i| i as isize)]
}

fn first_index<T: Selectable>(items: &[T]) -> Option<usize> {
    items.iter().position(|x| x.selected())
}

fn last_index<T: Selectable>(items: &[T]) -> Option<usize> {
    items.iter().rposition(|x| x.selected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Row {
        id: i64,
        selected: bool,
    }

    impl Selectable for Row {
        fn id(&self) -> i64 {
            self.id
        }
        fn selected(&self) -> bool {
            self.selected
        }
        fn set_selected(&mut self, selected: bool) {
            self.selected = selected;
        }
    }

    fn rows(n: i64) -> Vec<Row> {
        (0..n).map(|id| Row { id, selected: false }).collect()
    }

    fn flags(items: &[Row]) -> Vec<bool> {
        items.iter().map(|r| r.selected).collect()
    }

    #[test]
    fn selection_long_press_fills_gap_outward() {
        let mut items = rows(8);
        let mut sel = Selection::default();
        sel.toggle(&mut items, 2, true, true, true);
        assert_eq!(sel.bounds(), [2, 2]);
        sel.toggle(&mut items, 6, true, true, true);
        assert_eq!(flags(&items), [false, false, true, true, true, true, true, false]);
        assert_eq!(sel.bounds(), [2, 6]);
    }

    #[test]
    fn selection_long_press_upward() {
        let mut items = rows(8);
        let mut sel = Selection::default();
        sel.toggle(&mut items, 6, true, true, true);
        sel.toggle(&mut items, 2, true, true, true);
        assert_eq!(flags(&items), [false, false, true, true, true, true, true, false]);
        assert_eq!(sel.bounds(), [2, 6]);
    }

    #[test]
    fn selection_tap_deselect_bound_recomputes() {
        let mut items = rows(6);
        let mut sel = Selection::default();
        sel.toggle(&mut items, 1, true, true, true);
        sel.toggle(&mut items, 4, true, true, true);
        assert_eq!(sel.bounds(), [1, 4]);
        sel.toggle(&mut items, 1, false, true, false);
        assert_eq!(sel.bounds(), [2, 4]);
        sel.toggle(&mut items, 4, false, true, false);
        assert_eq!(sel.bounds(), [2, 3]);
    }

    #[test]
    fn selection_tap_outside_bounds_extends_them() {
        let mut items = rows(6);
        let mut sel = Selection::default();
        sel.toggle(&mut items, 2, true, true, true);
        sel.toggle(&mut items, 5, true, true, false);
        assert_eq!(sel.bounds(), [2, 5]);
        assert_eq!(flags(&items), [false, false, true, false, false, true]);
    }

    #[test]
    fn selection_stale_bounds_after_shrink() {
        let mut items = rows(8);
        let mut sel = Selection::default();
        for id in 3..=5 {
            sel.toggle(&mut items, id, true, true, true);
        }
        // the list is replaced by one without the selected rows
        let mut shrunk: Vec<Row> = (0..4).map(|id| Row { id: id + 100, selected: false }).collect();
        sel.sync(&shrunk);
        assert_eq!(sel.bounds(), [-1, -1]);
        sel.toggle(&mut shrunk, 100, true, true, true);
        assert_eq!(flags(&shrunk), [true, false, false, false]);
    }

    #[test]
    fn selection_sync_reads_bounds_from_flags() {
        let mut items = rows(5);
        items[1].selected = true;
        items[3].selected = true;
        let mut sel = Selection::default();
        sel.sync(&items);
        assert_eq!(sel.bounds(), [1, 3]);
    }

    #[test]
    fn selection_unknown_id_and_same_state_are_no_ops() {
        let mut items = rows(3);
        let mut sel = Selection::default();
        assert!(sel.toggle(&mut items, 99, true, true, true).is_empty());
        assert!(sel.toggle(&mut items, 0, false, true, false).is_empty());
        assert_eq!(sel.bounds(), [-1, -1]);
    }

    #[test]
    fn selection_toggle_all_and_invert_reset_bounds() {
        let mut items = rows(4);
        let mut sel = Selection::default();
        sel.toggle(&mut items, 1, true, true, true);
        sel.toggle_all(&mut items, true);
        assert_eq!(sel.bounds(), [-1, -1]);
        assert!(flags(&items).iter().all(|f| *f));
        sel.invert(&mut items);
        assert_eq!(sel.bounds(), [-1, -1]);
        assert!(flags(&items).iter().all(|f| !*f));
        assert!(!is_selection_mode(&items));
    }

    #[test]
    fn selection_range_skips_rows_already_selected() {
        let mut items = rows(5);
        let mut sel = Selection::default();
        sel.toggle(&mut items, 0, true, true, true);
        sel.toggle(&mut items, 2, true, true, true);
        let changed = sel.toggle(&mut items, 4, true, true, true);
        assert_eq!(changed, vec![4, 3]);
        assert!(flags(&items).iter().all(|f| *f));
    }

    #[test]
    fn vector_3_1_long_press_and_tap_sequence() {
        // rows with ids 1..8 at index id - 1; here ids are the indices 0..7
        let mut items = rows(8);
        let mut sel = Selection::default();
        sel.toggle(&mut items, 2, true, true, true);
        assert_eq!((flags(&items), sel.bounds()), (vec![false, false, true, false, false, false, false, false], [2, 2]));
        sel.toggle(&mut items, 5, true, true, true);
        assert_eq!((flags(&items), sel.bounds()), (vec![false, false, true, true, true, true, false, false], [2, 5]));
        sel.toggle(&mut items, 0, true, true, true);
        assert_eq!((flags(&items), sel.bounds()), (vec![true, true, true, true, true, true, false, false], [0, 5]));
        sel.toggle(&mut items, 0, false, true, false);
        assert_eq!((flags(&items), sel.bounds()), (vec![false, true, true, true, true, true, false, false], [1, 5]));
        sel.toggle(&mut items, 7, true, true, false);
        assert_eq!((flags(&items), sel.bounds()), (vec![false, true, true, true, true, true, false, true], [1, 7]));
        sel.toggle(&mut items, 7, false, true, false);
        assert_eq!((flags(&items), sel.bounds()), (vec![false, true, true, true, true, true, false, false], [1, 5]));
    }

    #[test]
    fn vector_3_2_quirk_tap_then_long_press_keeps_lower_anchor_unset() {
        let mut items = rows(8);
        let mut sel = Selection::default();
        sel.toggle(&mut items, 4, true, true, false);
        assert_eq!(sel.bounds(), [-1, 4]);
        sel.toggle(&mut items, 6, true, true, true);
        assert_eq!(flags(&items), [false, false, false, false, true, true, true, false]);
        assert_eq!(sel.bounds(), [-1, 6]);
    }

    #[test]
    fn quirk_long_press_with_stale_anchors() {
        let mut items = rows(8);
        let mut sel = Selection::default();
        sel.toggle(&mut items, 2, true, false, false);
        assert_eq!(sel.bounds(), [-1, -1], "a programmatic select leaves the anchors alone");
        sel.toggle(&mut items, 5, true, true, true);
        assert_eq!(flags(&items), [true, true, true, true, true, true, false, false]);
        assert_eq!(sel.bounds(), [-1, 5]);
    }
}
