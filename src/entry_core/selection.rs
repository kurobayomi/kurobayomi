//! Chapter selection engine and the keyboard / pointer gesture map.
//! Upstream: `MangaScreenModel.kt:971-1057` (toggle, select all, invert).
//!
//! Selection is keyed by chapter id over the raw list. Anchors are chapter ids, resolved against the
//! current view on every gesture, so a filter change does not move them to another row.

use std::collections::BTreeSet;

/// How a toggle was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// A plain tap, a Ctrl-click or Space.
    Tap,
    /// A long press, a right click, a Shift-click or Shift+Space. Selecting it extends a range.
    LongPress,
}

/// The selection set and the two range anchors (first and last selected row, as chapter ids).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    ids: BTreeSet<i64>,
    anchors: [Option<i64>; 2],
}

/// What a click on a row did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowClick {
    /// Nothing was selected and the click was plain: open the row's item.
    Open(i64),
    /// The click changed the selection.
    Toggled,
}

impl Selection {
    pub fn contains(&self, id: i64) -> bool {
        self.ids.contains(&id)
    }

    /// Whether selection mode is on.
    pub fn is_active(&self) -> bool {
        !self.ids.is_empty()
    }

    /// How many ids of `list` are selected.
    pub fn count_in(&self, list: &[i64]) -> usize {
        list.iter().filter(|id| self.ids.contains(id)).count()
    }

    /// Back or Escape: the selection is cleared first and the key is consumed (`true`). With nothing selected it
    /// returns `false` and the caller navigates up. A selection hidden by a filter still counts (E-04).
    pub fn back(&mut self) -> bool {
        let active = self.is_active();
        self.clear();
        active
    }

    /// A click or a long press on a row of `view`. A plain click with nothing selected opens the row; anything else
    /// toggles it: Shift-click and a long press (or right click) select it and the range up to it, Ctrl-click and a
    /// plain click while selecting flip just that row (also when nothing is selected yet).
    pub fn press(&mut self, view: &[i64], id: i64, mods: Mods, long_press_or_right: bool) -> RowClick {
        let command = click_command(mods, long_press_or_right);
        if command == Command::Tap && !mods.ctrl && !self.is_active() {
            return RowClick::Open(id);
        }
        self.toggle_row(view, id, command);
        RowClick::Toggled
    }

    /// Flips one row of `view` for a `Tap` or a `LongPress` command (the keyboard's Space does this on the focused
    /// row). A tap flips it; a long press selects it and the range up to it.
    pub fn toggle_row(&mut self, view: &[i64], id: i64, command: Command) {
        let selected = !self.contains(id);
        let pick = if command == Command::LongPress { Pick::LongPress } else { Pick::Tap };
        self.toggle(view, id, selected, pick);
    }

    #[cfg(test)]
    /// Selected chapter ids in ascending order.
    pub fn ids(&self) -> impl Iterator<Item = i64> + '_ {
        self.ids.iter().copied()
    }

    #[cfg(test)]
    /// Drops selected ids that no longer exist (a refresh removed the chapter). Keeps the rest.
    pub fn retain_existing(&mut self, existing: &[i64]) {
        self.ids.retain(|id| existing.contains(id));
        self.anchors = [
            self.anchors[0].filter(|a| existing.contains(a)),
            self.anchors[1].filter(|a| existing.contains(a)),
        ];
    }

    /// Sets every raw chapter id to `on`, and resets the anchors. Acts on the raw list, not the view (E-SEL-4).
    pub fn set_all(&mut self, raw_ids: &[i64], on: bool) {
        for &id in raw_ids {
            if on {
                self.ids.insert(id);
            } else {
                self.ids.remove(&id);
            }
        }
        self.anchors = [None, None];
    }

    /// Flips every raw chapter id, and resets the anchors.
    pub fn invert(&mut self, raw_ids: &[i64]) {
        for &id in raw_ids {
            if !self.ids.remove(&id) {
                self.ids.insert(id);
            }
        }
        self.anchors = [None, None];
    }

    /// Clears the selection and leaves selection mode. Mark, bookmark and download actions call this (E-SEL-5).
    pub fn clear(&mut self) {
        self.ids.clear();
        self.anchors = [None, None];
    }

    /// Toggles `id` in the view `view` (chapter ids in view order). Returns true when the selection changed.
    /// `selected` is the requested state. A request that matches the current state changes nothing.
    pub fn toggle(&mut self, view: &[i64], id: i64, selected: bool, pick: Pick) -> bool {
        let Some(idx) = view.iter().position(|&v| v == id).map(|i| i as i64) else {
            return false;
        };
        if self.ids.contains(&id) == selected {
            return false;
        }
        let first_selection = !view.iter().any(|v| self.ids.contains(v));
        if selected {
            self.ids.insert(id);
        } else {
            self.ids.remove(&id);
        }

        let mut p0 = self.anchor_index(view, 0);
        let mut p1 = self.anchor_index(view, 1);
        match pick {
            Pick::LongPress if selected => {
                if first_selection {
                    p0 = idx;
                    p1 = idx;
                } else {
                    let range = if idx < p0 {
                        let r = (idx + 1)..p0;
                        p0 = idx;
                        r
                    } else if idx > p1 {
                        let r = (p1 + 1)..idx;
                        p1 = idx;
                        r
                    } else {
                        0..0
                    };
                    for i in range {
                        self.ids.insert(view[i as usize]);
                    }
                }
            }
            Pick::Tap => {
                if !selected {
                    if idx == p0 {
                        p0 = self.first_selected_index(view);
                    } else if idx == p1 {
                        p1 = self.last_selected_index(view);
                    }
                } else if p0 < 0 && p1 < 0 {
                    // ponytail: upstream sets only the second anchor when none exists (E-SEL-3). Both are set here.
                    p0 = idx;
                    p1 = idx;
                } else if idx < p0 {
                    p0 = idx;
                } else if idx > p1 {
                    p1 = idx;
                }
            }
            _ => {}
        }
        self.anchors = [self.id_at(view, p0), self.id_at(view, p1)];
        true
    }

    fn anchor_index(&self, view: &[i64], slot: usize) -> i64 {
        self.anchors[slot]
            .and_then(|a| view.iter().position(|&v| v == a))
            .map_or(-1, |i| i as i64)
    }

    fn id_at(&self, view: &[i64], index: i64) -> Option<i64> {
        usize::try_from(index)
            .ok()
            .and_then(|i| view.get(i))
            .copied()
    }

    fn first_selected_index(&self, view: &[i64]) -> i64 {
        view.iter()
            .position(|v| self.ids.contains(v))
            .map_or(-1, |i| i as i64)
    }

    fn last_selected_index(&self, view: &[i64]) -> i64 {
        view.iter()
            .rposition(|v| self.ids.contains(v))
            .map_or(-1, |i| i as i64)
    }
}

/// A user command from the keyboard or pointer, before it is applied to the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Tap,
    LongPress,
}

/// Modifier keys held during a key press or click.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
}

/// Pointer map. A long press or right click selects and starts a range. Shift-click extends a range,
/// Ctrl-click toggles. A plain click returns `Tap`, and the caller opens the chapter when not in selection mode.
pub fn click_command(mods: Mods, long_press_or_right: bool) -> Command {
    if long_press_or_right || mods.shift {
        Command::LongPress
    } else {
        Command::Tap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> Vec<i64> {
        vec![10, 11, 12, 13, 14]
    }

    #[test]
    fn long_press_range_forward() {
        let v = view();
        let mut s = Selection::default();
        s.toggle(&v, 11, true, Pick::LongPress);
        s.toggle(&v, 13, true, Pick::LongPress);
        assert_eq!(s.ids().collect::<Vec<_>>(), vec![11, 12, 13]);
    }

    #[test]
    fn long_press_range_backward() {
        let v = view();
        let mut s = Selection::default();
        s.toggle(&v, 13, true, Pick::LongPress);
        s.toggle(&v, 11, true, Pick::LongPress);
        assert_eq!(s.ids().collect::<Vec<_>>(), vec![11, 12, 13]);
    }

    #[test]
    fn long_press_inside_range_only_self() {
        let v = view();
        let mut s = Selection::default();
        s.toggle(&v, 11, true, Pick::LongPress);
        s.toggle(&v, 13, true, Pick::LongPress);
        s.toggle(&v, 12, false, Pick::Tap);
        // 12 is deselected. A long press on it inside the range selects only 12.
        s.toggle(&v, 12, true, Pick::LongPress);
        assert_eq!(s.ids().collect::<Vec<_>>(), vec![11, 12, 13]);
        let mut t = Selection::default();
        t.toggle(&v, 11, true, Pick::LongPress);
        t.toggle(&v, 14, true, Pick::LongPress);
        t.toggle(&v, 12, false, Pick::LongPress);
        t.toggle(&v, 12, true, Pick::LongPress);
        assert!(t.contains(12));
    }

    #[test]
    fn invert_flips_raw() {
        let raw = [1, 2, 3];
        let mut s = Selection::default();
        s.toggle(&[1, 2, 3], 2, true, Pick::Tap);
        s.invert(&raw);
        assert_eq!(s.ids().collect::<Vec<_>>(), vec![1, 3]);
    }

    #[test]
    fn anchors_survive_filter_change() {
        // Anchors are chapter ids. After a filter hides row 11, the range is measured from 12 in the new view.
        let full = view();
        let mut s = Selection::default();
        s.toggle(&full, 12, true, Pick::LongPress);
        let filtered = vec![10, 12, 13, 14];
        assert!(s.toggle(&filtered, 14, true, Pick::LongPress));
        assert_eq!(s.ids().collect::<Vec<_>>(), vec![12, 13, 14]);
    }

    #[test]
    fn retain_existing_drops_removed_chapters() {
        let mut s = Selection::default();
        s.set_all(&[1, 2, 3], true);
        s.retain_existing(&[1, 3]);
        assert_eq!(s.ids().collect::<Vec<_>>(), vec![1, 3]);
    }
    fn ctrl() -> Mods {
        Mods { ctrl: true, ..Mods::default() }
    }

    fn shift() -> Mods {
        Mods { shift: true, ..Mods::default() }
    }

    #[test]
    fn a_plain_click_opens_when_nothing_is_selected_and_toggles_while_selecting() {
        let v = view();
        let mut s = Selection::default();
        assert_eq!(s.press(&v, 10, Mods::default(), false), RowClick::Open(10));
        assert_eq!(s.press(&v, 10, Mods::default(), true), RowClick::Toggled, "a long press selects");
        assert_eq!(s.press(&v, 12, Mods::default(), false), RowClick::Toggled);
        assert!(s.contains(12));
        assert_eq!(s.press(&v, 10, Mods::default(), false), RowClick::Toggled);
        assert!(!s.contains(10));
    }

    #[test]
    fn shift_click_selects_the_range_from_the_anchor_in_both_directions() {
        let v = view();
        let mut s = Selection::default();
        s.press(&v, 12, Mods::default(), true);
        s.press(&v, 14, shift(), false);
        assert_eq!(s.ids().collect::<Vec<_>>(), vec![12, 13, 14]);
        let mut s = Selection::default();
        s.press(&v, 13, ctrl(), false);
        s.press(&v, 10, shift(), false);
        assert_eq!(s.ids().collect::<Vec<_>>(), vec![10, 11, 12, 13], "a ctrl-click is an anchor too");
    }

    #[test]
    fn ctrl_click_flips_one_row_even_with_nothing_selected() {
        let v = view();
        let mut s = Selection::default();
        assert_eq!(s.press(&v, 11, ctrl(), false), RowClick::Toggled);
        assert_eq!(s.press(&v, 13, ctrl(), false), RowClick::Toggled);
        assert_eq!(s.ids().collect::<Vec<_>>(), vec![11, 13], "no range between them");
        s.press(&v, 11, ctrl(), false);
        assert_eq!(s.ids().collect::<Vec<_>>(), vec![13]);
    }

    #[test]
    fn back_clears_the_selection_first_and_navigates_only_when_nothing_is_selected() {
        let v = view();
        let mut s = Selection::default();
        assert!(!s.back());
        s.press(&v, 10, Mods::default(), true);
        assert!(s.back());
        assert!(!s.is_active());
        assert!(!s.back());
    }

    #[test]
    fn count_in_counts_the_selected_ids_of_a_list_and_a_filtered_out_one_still_counts_as_active() {
        let v = view();
        let mut s = Selection::default();
        s.press(&v, 10, Mods::default(), true);
        s.press(&v, 12, Mods::default(), false);
        assert_eq!(s.count_in(&[10, 11, 12]), 2);
        assert_eq!(s.count_in(&[11]), 0);
        assert!(s.is_active());
    }

    #[test]
    fn key_taps_on_the_focused_row_toggle_it_with_no_open() {
        let v = view();
        let mut s = Selection::default();
        s.toggle_row(&v, 11, Command::Tap);
        s.toggle_row(&v, 13, Command::LongPress);
        assert_eq!(s.ids().collect::<Vec<_>>(), vec![11, 12, 13]);
    }
}
