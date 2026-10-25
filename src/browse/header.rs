//! The page actions of the source header bar (T-40a-002): buttons in a wide window, a ⋮ menu in the phone layout. Its view options are the look sections shared with the
//! Library (`widgets::view_look`).

use kuro_i18n::{MsgKey, tr};

/// Overflow entries of the source header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverflowItem {
    /// The local source's help page. Left out while no Kurobayomi help page exists.
    Help,
    /// Any other source: open its site in the web view.
    OpenInWebView,
    /// Only for sources that have settings.
    Settings,
}

/// Overflow of a source header: help for the local source, web view for others, and settings when configurable.
pub fn overflow_items(is_local: bool, configurable: bool) -> Vec<OverflowItem> {
    overflow_items_with(is_local, configurable, super::nav::help_url().is_some())
}

/// `has_help` is false while there is no help page: the local source then has no Help entry.
fn overflow_items_with(is_local: bool, configurable: bool, has_help: bool) -> Vec<OverflowItem> {
    let mut items = match (is_local, has_help) {
        (true, true) => vec![OverflowItem::Help],
        (true, false) => vec![],
        (false, _) => vec![OverflowItem::OpenInWebView],
    };
    if configurable {
        items.push(OverflowItem::Settings);
    }
    items
}

/// The header button icon of an overflow entry.
pub fn overflow_icon(item: OverflowItem) -> &'static str {
    match item {
        OverflowItem::Help => "help-browser-symbolic",
        OverflowItem::OpenInWebView => "web-browser-symbolic",
        OverflowItem::Settings => "emblem-system-symbolic",
    }
}

/// The label of an overflow entry.
pub fn overflow_label(item: OverflowItem) -> String {
    tr(match item {
        OverflowItem::Help => MsgKey::label_help,
        OverflowItem::OpenInWebView => MsgKey::action_open_in_web_view,
        OverflowItem::Settings => MsgKey::label_settings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_overflow_entry_has_a_label() {
        for item in [OverflowItem::Help, OverflowItem::OpenInWebView, OverflowItem::Settings] {
            assert!(!overflow_label(item).is_empty());
        }
    }

    #[test]
    fn overflow_has_help_for_local_and_web_view_otherwise_plus_settings_when_configurable() {
        assert_eq!(overflow_items_with(true, false, true), vec![OverflowItem::Help]);
        assert_eq!(overflow_items_with(false, false, true), vec![OverflowItem::OpenInWebView]);
        assert_eq!(overflow_items_with(false, true, true), vec![OverflowItem::OpenInWebView, OverflowItem::Settings]);
        // no help page yet: the local source has no Help entry, every other list is unchanged
        assert!(overflow_items_with(true, false, false).is_empty());
        assert_eq!(overflow_items_with(true, true, false), vec![OverflowItem::Settings]);
        assert!(!overflow_items(true, false).contains(&OverflowItem::Help));
    }
}
