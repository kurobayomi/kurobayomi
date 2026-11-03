//! Context-menu item selection. The UI builds the `gtk::PopoverMenu` from the result, in
//! the order the items were declared.

/// The items that show, in their declared order. Each entry is `(item, hidden)`, where `hidden` is what the menu's
/// init step decided (for example, "move to top" on the first row).
pub fn visible_items<T>(items: impl IntoIterator<Item = (T, bool)>) -> Vec<T> {
    items
        .into_iter()
        .filter_map(|(item, hidden)| (!hidden).then_some(item))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_items_are_dropped_and_order_is_kept() {
        let items = vec![
            ("move_to_top", true),
            ("move_up", false),
            ("delete", false),
            ("move_to_bottom", true),
        ];
        assert_eq!(visible_items(items), vec!["move_up", "delete"]);
    }

    #[test]
    fn all_hidden_gives_an_empty_menu() {
        assert!(visible_items(vec![(1, true), (2, true)]).is_empty());
    }
}
