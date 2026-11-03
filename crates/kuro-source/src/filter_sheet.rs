//! The filter sheet without its widgets (`SourceFilterMangaDialog.kt`; T-15a-039).
//! The UI draws the rows from [`FilterSheet::items`] and writes each change through the edit methods. An edit names its
//! filter by a path: the index at each level, down through groups. Every state rule lives here, so the GTK layer only
//! maps widgets to these calls. The sheet owns its own copy of the list: a running search keeps its own clone (RC-06).

use std::collections::HashSet;

use crate::filter::{Filter, SortSelection, TriStateState};
use crate::filter_list::FilterList;

/// Why an edit did not apply. The sheet state is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SheetError {
    #[error("no filter at that path")]
    NoSuchFilter,
    #[error("the filter at that path is not of the expected kind")]
    WrongKind,
    #[error("index {0} is outside the values")]
    IndexOutOfRange(usize),
}

/// One row of the sheet. `path` is the address that edits take.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetItem {
    pub path: Vec<usize>,
    pub view: ItemView,
}

/// What one row shows.
#[derive(Debug, Clone, PartialEq)]
pub enum ItemView {
    Heading {
        name: String,
    },
    Divider,
    CheckBox {
        label: String,
        checked: bool,
    },
    TriState {
        label: String,
        state: TriStateState,
    },
    Text {
        label: String,
        value: String,
    },
    /// `selected` is `None` when the stored index is outside `options` (EC-FI-01): the row shows no value.
    Select {
        label: String,
        options: Vec<String>,
        selected: Option<usize>,
    },
    Sort {
        label: String,
        options: Vec<String>,
        selection: Option<SortSelection>,
        expanded: bool,
    },
    Group {
        label: String,
        expanded: bool,
        children: Vec<SheetItem>,
    },
}

/// The arrow of one sort row: `Some(true)` descending, `Some(false)` ascending, `None` for an unselected row
/// (`sortDescending` of `SourceFilterMangaDialog.kt`).
pub fn sort_descending(selection: Option<SortSelection>, index: usize) -> Option<bool> {
    let sel = selection?;
    (usize::try_from(sel.index).ok() == Some(index)).then_some(!sel.ascending)
}

/// The filters of one browse screen, with the expansion state of its groups and sort boxes.
#[derive(Debug, Clone, Default)]
pub struct FilterSheet {
    filters: FilterList,
    /// Paths of the groups and sort boxes that are open. Both start closed (`CollapsibleBox` starts collapsed).
    expanded: HashSet<Vec<usize>>,
}

impl FilterSheet {
    pub fn new(filters: FilterList) -> Self {
        Self { filters, expanded: HashSet::new() }
    }

    pub fn filters(&self) -> &FilterList {
        &self.filters
    }

    pub fn into_filters(self) -> FilterList {
        self.filters
    }

    /// The rows in source order. Children of a group are always listed; the UI draws them only when `expanded`.
    pub fn items(&self) -> Vec<SheetItem> {
        build(&self.filters.0, &[], &self.expanded)
    }

    /// Opens or closes the group or sort box at `path`.
    pub fn toggle_expanded(&mut self, path: &[usize]) -> Result<(), SheetError> {
        let filter = find(&self.filters.0, path).ok_or(SheetError::NoSuchFilter)?;
        if !matches!(filter, Filter::Group { .. } | Filter::Sort { .. }) {
            return Err(SheetError::WrongKind);
        }
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_vec());
        }
        Ok(())
    }

    /// Flips a checkbox.
    pub fn toggle_check(&mut self, path: &[usize]) -> Result<(), SheetError> {
        match at_mut(&mut self.filters.0, path)? {
            Filter::CheckBox { state, .. } => {
                *state = !*state;
                Ok(())
            }
            _ => Err(SheetError::WrongKind),
        }
    }

    /// Steps a tri-state `Ignore`, `Include`, `Exclude`, `Ignore` (`0 -> 1 -> 2 -> 0`).
    pub fn cycle_tri(&mut self, path: &[usize]) -> Result<(), SheetError> {
        match at_mut(&mut self.filters.0, path)? {
            Filter::TriState { state, .. } => {
                *state = state.next();
                Ok(())
            }
            _ => Err(SheetError::WrongKind),
        }
    }

    /// Writes the text of a text filter. Called on every change, as the Kotlin sheet does (EC-FI-06).
    pub fn set_text(&mut self, path: &[usize], value: String) -> Result<(), SheetError> {
        match at_mut(&mut self.filters.0, path)? {
            Filter::Text { state, .. } => {
                *state = value;
                Ok(())
            }
            _ => Err(SheetError::WrongKind),
        }
    }

    /// Chooses option `index` of a select filter.
    pub fn set_select(&mut self, path: &[usize], index: usize) -> Result<(), SheetError> {
        match at_mut(&mut self.filters.0, path)? {
            Filter::Select { values, state, .. } => {
                if index >= values.len() {
                    return Err(SheetError::IndexOutOfRange(index));
                }
                *state = i32::try_from(index).map_err(|_| SheetError::IndexOutOfRange(index))?;
                Ok(())
            }
            _ => Err(SheetError::WrongKind),
        }
    }

    /// A tap on sort row `index` (EC-FI-03..05): the same row flips the direction, another row takes the selection and
    /// keeps the direction, and with no selection the tapped row is ascending.
    pub fn tap_sort(&mut self, path: &[usize], index: usize) -> Result<(), SheetError> {
        match at_mut(&mut self.filters.0, path)? {
            Filter::Sort { values, state, .. } => {
                if index >= values.len() {
                    return Err(SheetError::IndexOutOfRange(index));
                }
                let index = i32::try_from(index).map_err(|_| SheetError::IndexOutOfRange(index))?;
                *state = Some(SortSelection::tap(*state, index));
                Ok(())
            }
            _ => Err(SheetError::WrongKind),
        }
    }

    /// Reset: every filter object is replaced by `fresh`. Nothing is searched here (EC-FI-10).
    pub fn reset(&mut self, fresh: FilterList) {
        self.filters = fresh;
        self.expanded.clear();
    }
}

fn find<'a>(list: &'a [Filter], path: &[usize]) -> Option<&'a Filter> {
    let (first, rest) = path.split_first()?;
    let filter = list.get(*first)?;
    if rest.is_empty() {
        return Some(filter);
    }
    match filter {
        Filter::Group { state, .. } => find(state, rest),
        _ => None,
    }
}

fn at_mut<'a>(list: &'a mut [Filter], path: &[usize]) -> Result<&'a mut Filter, SheetError> {
    let (first, rest) = path.split_first().ok_or(SheetError::NoSuchFilter)?;
    let filter = list.get_mut(*first).ok_or(SheetError::NoSuchFilter)?;
    if rest.is_empty() {
        return Ok(filter);
    }
    match filter {
        Filter::Group { state, .. } => at_mut(state.as_mut_slice(), rest),
        _ => Err(SheetError::NoSuchFilter),
    }
}

fn build(list: &[Filter], prefix: &[usize], expanded: &HashSet<Vec<usize>>) -> Vec<SheetItem> {
    list.iter()
        .enumerate()
        .map(|(i, filter)| {
            let mut path = prefix.to_vec();
            path.push(i);
            let open = expanded.contains(&path);
            let view = match filter {
                Filter::Header { name } => ItemView::Heading { name: name.clone() },
                Filter::Separator { .. } => ItemView::Divider,
                Filter::CheckBox { name, state } => ItemView::CheckBox { label: name.clone(), checked: *state },
                Filter::TriState { name, state } => ItemView::TriState { label: name.clone(), state: *state },
                Filter::Text { name, state } => ItemView::Text { label: name.clone(), value: state.clone() },
                Filter::Select { name, values, state } => ItemView::Select {
                    label: name.clone(),
                    options: values.clone(),
                    selected: usize::try_from(*state).ok().filter(|i| *i < values.len()),
                },
                Filter::Sort { name, values, state } => {
                    ItemView::Sort { label: name.clone(), options: values.clone(), selection: *state, expanded: open }
                }
                Filter::Group { name, state } => {
                    ItemView::Group { label: name.clone(), expanded: open, children: build(state, &path, expanded) }
                }
            };
            SheetItem { path, view }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> Vec<String> {
        ["Popularity", "Latest", "Title"].map(str::to_owned).to_vec()
    }

    /// A list shaped like a real source: a heading, a text box, a select, a sort, and a group with a tri-state and a
    /// nested group.
    fn sample() -> FilterList {
        FilterList(vec![
            Filter::header("Search"),
            Filter::text("Query"),
            Filter::separator("-"),
            Filter::select_at("Status", vec!["Any".into(), "Ongoing".into()], 1),
            Filter::sort("Order", values()),
            Filter::group(
                "Genres",
                vec![Filter::tri_state("Action"), Filter::group("Deep", vec![Filter::check_box("Nested")])],
            ),
        ])
    }

    #[test]
    fn the_text_filter_named_query_is_a_row_with_its_label() {
        let items = FilterSheet::new(sample()).items();
        assert_eq!(items[1].path, vec![1]);
        assert_eq!(items[1].view, ItemView::Text { label: "Query".into(), value: String::new() });
        assert_eq!(items[0].view, ItemView::Heading { name: "Search".into() });
        assert_eq!(items[2].view, ItemView::Divider);
    }

    #[test]
    fn text_is_written_on_every_change() {
        let mut sheet = FilterSheet::new(sample());
        sheet.set_text(&[1], "a".into()).expect("text");
        sheet.set_text(&[1], "ab".into()).expect("text");
        assert_eq!(sheet.filters()[1], Filter::Text { name: "Query".into(), state: "ab".into() });
    }

    #[test]
    fn a_select_index_outside_the_values_shows_no_value_and_does_not_crash() {
        let list = FilterList(vec![Filter::Select { name: "Pick".into(), values: values(), state: 5 }]);
        let items = FilterSheet::new(list.clone()).items();
        assert_eq!(items[0].view, ItemView::Select { label: "Pick".into(), options: values(), selected: None });
        assert_eq!(list[0].select_value(), None);
    }

    #[test]
    fn set_select_writes_the_chosen_index_and_rejects_an_index_past_the_values() {
        let mut sheet = FilterSheet::new(sample());
        sheet.set_select(&[3], 2).expect_err("only two values");
        sheet.set_select(&[3], 0).expect("valid index");
        assert_eq!(sheet.filters()[3].select_value(), Some("Any"));
        assert_eq!(sheet.set_select(&[3], 9), Err(SheetError::IndexOutOfRange(9)));
    }

    #[test]
    fn tri_state_cycles_zero_one_two_zero() {
        let mut sheet = FilterSheet::new(sample());
        let states: Vec<TriStateState> = (0..4)
            .map(|_| {
                sheet.cycle_tri(&[5, 0]).expect("tri");
                match &sheet.filters()[5] {
                    Filter::Group { state, .. } => match state[0] {
                        Filter::TriState { state, .. } => state,
                        _ => panic!("expected tri-state"),
                    },
                    _ => panic!("expected group"),
                }
            })
            .collect();
        assert_eq!(
            states,
            vec![TriStateState::Include, TriStateState::Exclude, TriStateState::Ignore, TriStateState::Include]
        );
    }

    #[test]
    fn check_box_toggles_and_nested_groups_are_written_at_depth() {
        let mut sheet = FilterSheet::new(sample());
        sheet.toggle_check(&[5, 1, 0]).expect("nested check");
        match &sheet.filters()[5] {
            Filter::Group { state, .. } => match &state[1] {
                Filter::Group { state, .. } => {
                    assert_eq!(state[0], Filter::CheckBox { name: "Nested".into(), state: true })
                }
                _ => panic!("expected nested group"),
            },
            _ => panic!("expected group"),
        }
        assert_eq!(sheet.toggle_check(&[1]), Err(SheetError::WrongKind));
        assert_eq!(sheet.toggle_check(&[9]), Err(SheetError::NoSuchFilter));
    }

    #[test]
    fn sort_taps_follow_the_three_rules() {
        let mut sheet = FilterSheet::new(sample());
        sheet.tap_sort(&[4], 2).expect("sort");
        assert_eq!(
            sheet.filters()[4],
            Filter::Sort {
                name: "Order".into(),
                values: values(),
                state: Some(SortSelection { index: 2, ascending: true })
            }
        );
        sheet.tap_sort(&[4], 2).expect("sort");
        sheet.tap_sort(&[4], 0).expect("sort");
        assert_eq!(
            sheet.filters()[4],
            Filter::Sort {
                name: "Order".into(),
                values: values(),
                state: Some(SortSelection { index: 0, ascending: false })
            }
        );
    }

    #[test]
    fn the_sort_arrow_is_only_on_the_selected_row() {
        let sel = Some(SortSelection { index: 1, ascending: true });
        assert_eq!(sort_descending(sel, 1), Some(false));
        assert_eq!(sort_descending(sel, 0), None);
        assert_eq!(sort_descending(None, 1), None);
        assert_eq!(sort_descending(Some(SortSelection { index: 1, ascending: false }), 1), Some(true));
    }

    #[test]
    fn groups_and_sort_boxes_start_closed_and_open_on_toggle() {
        let mut sheet = FilterSheet::new(sample());
        let closed = sheet.items();
        assert!(matches!(closed[5].view, ItemView::Group { expanded: false, .. }));
        sheet.toggle_expanded(&[5]).expect("group");
        sheet.toggle_expanded(&[4]).expect("sort");
        let open = sheet.items();
        assert!(matches!(open[5].view, ItemView::Group { expanded: true, .. }));
        assert!(matches!(open[4].view, ItemView::Sort { expanded: true, .. }));
        assert_eq!(sheet.toggle_expanded(&[1]), Err(SheetError::WrongKind));
    }

    #[test]
    fn nested_rows_carry_their_own_paths() {
        let items = FilterSheet::new(sample()).items();
        let ItemView::Group { children, .. } = &items[5].view else { panic!("expected group") };
        assert_eq!(children[0].path, vec![5, 0]);
        let ItemView::Group { children: deep, .. } = &children[1].view else { panic!("expected nested group") };
        assert_eq!(deep[0].path, vec![5, 1, 0]);
    }

    #[test]
    fn reset_replaces_every_filter_and_closes_the_boxes() {
        let mut sheet = FilterSheet::new(sample());
        sheet.set_text(&[1], "x".into()).expect("text");
        sheet.toggle_expanded(&[5]).expect("group");
        sheet.reset(sample());
        assert_eq!(sheet.filters()[1], Filter::text("Query"));
        assert!(matches!(sheet.items()[5].view, ItemView::Group { expanded: false, .. }));
    }
}
