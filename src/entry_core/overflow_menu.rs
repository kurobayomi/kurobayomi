//! The page actions of the manga page and the anime page, one set for both (T-39a-075, T-39a-057): header buttons, once
//! the overflow menu ("⋮"; user, 2026-10-25). Upstream:
//! `EntryToolbar.kt` (the order: change intro length, edit categories, Migrate, Share, Open in browser, Settings; Refresh
//! is the header button, not a menu item) and the screens' conditions: a favorite for the category item, Migrate and the
//! intro length; episodes for the intro length; an HTTP source for Share and Open in browser; a configurable source for
//! Settings. The model is pure; the menu entries are built by the page, and the address copy and the browser launch are
//! the shared helpers the two pages call.

use gtk::prelude::*;
use kuro_i18n::{Arg, MsgKey, tr, trf};

/// Whether Open in browser may launch this address: an `http` or `https` one only.
pub fn can_launch(url: &str) -> bool {
    let lower = url.trim_start().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// The overflow entries, in the order they are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// The skip-intro length of an anime's episodes.
    ChangeIntro,
    EditCategories,
    Migrate,
    Share,
    OpenInBrowser,
    Settings,
}

/// What the menu reads from the entry and its source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub favorite: bool,
    /// The entry has an intro length to change: an anime that lists episodes, not seasons.
    pub has_intro_length: bool,
    pub http_source: bool,
    pub configurable_source: bool,
}

/// The items the menu shows for `entry`; none for a plain entry.
pub fn items(entry: Entry) -> Vec<Item> {
    let mut items = Vec::new();
    if entry.favorite && entry.has_intro_length {
        items.push(Item::ChangeIntro);
    }
    if entry.favorite {
        items.push(Item::EditCategories);
        items.push(Item::Migrate);
    }
    if entry.http_source {
        items.push(Item::Share);
        items.push(Item::OpenInBrowser);
    }
    if entry.configurable_source {
        items.push(Item::Settings);
    }
    items
}

/// The row label of an item.
pub fn label(item: Item) -> String {
    tr(match item {
        Item::ChangeIntro => MsgKey::action_change_intro_length,
        Item::EditCategories => MsgKey::action_edit_categories,
        Item::Migrate => MsgKey::action_migrate,
        Item::Share => MsgKey::action_share,
        Item::OpenInBrowser => MsgKey::action_open_in_browser,
        Item::Settings => MsgKey::action_settings,
    })
}

/// The header button icon of an item.
pub fn icon(item: Item) -> &'static str {
    match item {
        Item::ChangeIntro => "media-skip-forward-symbolic",
        Item::EditCategories => "tag-symbolic",
        Item::Migrate => "kb-swap-symbolic",
        Item::Share => "kb-share-symbolic",
        Item::OpenInBrowser => "external-link-symbolic",
        Item::Settings => "emblem-system-symbolic",
    }
}

/// Share: copies the entry's address to the clipboard and returns the notice to show ("Copied to clipboard: …"), or
/// `None` when there is no display.
pub fn share(url: &str) -> Option<String> {
    let display = gtk::gdk::Display::default()?;
    display.clipboard().set_text(url);
    Some(trf(MsgKey::copied_to_clipboard, &[("arg1", Arg::Str(url))]))
}

/// Open in browser: the entry's address in the desktop's browser. Only a web address launches; a failed launch is a
/// toast, never a panic.
pub fn open_in_browser(url: &str) {
    if can_launch(url) {
        crate::ui::open_link(url, Some(crate::ui::window().upcast_ref()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(favorite: bool, has_intro_length: bool, http_source: bool, configurable_source: bool) -> Entry {
        Entry { favorite, has_intro_length, http_source, configurable_source }
    }

    #[test]
    fn a_plain_entry_offers_nothing() {
        assert_eq!(items(entry(false, true, false, false)), vec![]);
        assert_eq!(items(entry(false, false, false, false)), vec![]);
    }

    #[test]
    fn a_favorite_with_an_intro_length_offers_the_library_items_in_android_order() {
        assert_eq!(
            items(entry(true, true, true, true)),
            vec![Item::ChangeIntro, Item::EditCategories, Item::Migrate, Item::Share, Item::OpenInBrowser, Item::Settings],
        );
    }

    #[test]
    fn a_manga_and_a_season_entry_never_offer_the_intro_length() {
        assert_eq!(items(entry(true, false, false, false)), vec![Item::EditCategories, Item::Migrate]);
        assert_eq!(
            items(entry(true, false, true, true)),
            vec![Item::EditCategories, Item::Migrate, Item::Share, Item::OpenInBrowser, Item::Settings],
        );
    }

    #[test]
    fn settings_shows_only_for_a_configurable_source_and_share_and_open_only_for_http() {
        assert_eq!(items(entry(false, true, false, true)), vec![Item::Settings]);
        assert_eq!(items(entry(false, true, true, false)), vec![Item::Share, Item::OpenInBrowser]);
    }

    #[test]
    fn only_web_addresses_launch() {
        assert!(can_launch("https://a.example/x") && can_launch("HTTP://a.example"));
        assert!(!can_launch("/relative/path") && !can_launch("file:///x") && !can_launch(""));
    }
}
