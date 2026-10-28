//! Reader chrome: the initial load error card (T-43a-024) and the top bar (T-43a-040).
//!
//! The pure helpers hold the text and the source-dependent overflow rule, so they are tested without GTK.
//! The widget builders only assemble them.

use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use kuro_prefs::reader::ReadingMode;

use super::model::LoadError;
use super::strings;
use crate::widgets::page_header;

/// The card text for an initial load failure. A blank message shows the generic text, so no card is empty. The
/// downloaded-only text is shown as is, since it is not a page-load failure (T-43a-074).
pub fn error_card_message(error: &LoadError) -> String {
    if error.message == strings::chapter_not_downloaded() {
        return error.message.clone();
    }
    let message = if error.message.trim().is_empty() { strings::unknown_error() } else { error.message.as_str().to_string() };
    strings::failed_to_load_pages(&message)
}

/// The action group the reader page installs, so the menu's items name their actions as `reader.<name>`.
pub const ACTION_GROUP: &str = "reader";
const ACTION_MODE: &str = "mode";
const ACTION_CROP: &str = "crop";
const ACTION_SETTINGS: &str = "settings";
const ACTION_OPEN_BROWSER: &str = "open-browser";
const ACTION_SHARE: &str = "share";
const ACTION_SET_COVER: &str = "set-cover";

/// The five modes the menu offers, with the string each radio item carries as its action target. Default is not a
/// choice: a Default manga reads in the preference's mode, which is one of these.
pub const MODES: [(&str, ReadingMode); 5] = [
    ("ltr", ReadingMode::LeftToRight),
    ("rtl", ReadingMode::RightToLeft),
    ("vertical", ReadingMode::Vertical),
    ("webtoon", ReadingMode::Webtoon),
    ("webtoon-gaps", ReadingMode::ContinuousVertical),
];

/// The label of a mode, in the menu, the settings chips and the toast.
pub fn mode_label(mode: ReadingMode) -> String {
    match mode {
        ReadingMode::Default => strings::label_default(),
        ReadingMode::LeftToRight => strings::left_to_right_viewer(),
        ReadingMode::RightToLeft => strings::right_to_left_viewer(),
        ReadingMode::Vertical => strings::vertical_viewer(),
        ReadingMode::Webtoon => strings::webtoon_viewer(),
        ReadingMode::ContinuousVertical => strings::vertical_plus_viewer(),
    }
}

/// The action target of a mode; Default has none, so no radio item shows as chosen.
fn mode_target(mode: ReadingMode) -> &'static str {
    MODES.iter().find(|(_, m)| *m == mode).map_or("", |(target, _)| target)
}

fn mode_from_target(target: &str) -> Option<ReadingMode> {
    MODES.iter().find(|(t, _)| *t == target).map(|(_, mode)| *mode)
}

fn detailed(action: &str) -> String {
    format!("{ACTION_GROUP}.{action}")
}

/// The model of the top bar's menu: Reading mode (five radio items), Crop borders and Reader settings, then Open in
/// browser and Share for an HTTP source, or Set as cover (the page on screen) for any other source (a local manga).
pub fn overflow_menu(is_http_source: bool) -> gio::Menu {
    let menu = gio::Menu::new();
    let modes = gio::Menu::new();
    for (target, mode) in MODES {
        let item = gio::MenuItem::new(Some(&mode_label(mode)), None);
        item.set_action_and_target_value(Some(&detailed(ACTION_MODE)), Some(&target.to_variant()));
        modes.append_item(&item);
    }
    menu.append_section(Some(&strings::pref_category_reading_mode()), &modes);
    let reader = gio::Menu::new();
    reader.append(Some(&strings::pref_crop_borders()), Some(&detailed(ACTION_CROP)));
    reader.append(Some(&strings::menu_reader_settings()), Some(&detailed(ACTION_SETTINGS)));
    menu.append_section(None, &reader);
    let page = gio::Menu::new();
    if is_http_source {
        page.append(Some(&strings::action_open_in_browser()), Some(&detailed(ACTION_OPEN_BROWSER)));
        page.append(Some(&strings::action_share()), Some(&detailed(ACTION_SHARE)));
    } else {
        page.append(Some(&strings::set_as_cover()), Some(&detailed(ACTION_SET_COVER)));
    }
    menu.append_section(None, &page);
    menu
}

/// What the menu's actions do. Run on the GTK thread.
pub struct MenuHandlers {
    /// A radio item was chosen: apply the mode to this manga at once.
    pub on_mode: Rc<dyn Fn(ReadingMode)>,
    /// The Crop borders switch was flipped.
    pub on_crop: Rc<dyn Fn()>,
    pub on_settings: Rc<dyn Fn()>,
    pub on_open_browser: Rc<dyn Fn()>,
    pub on_share: Rc<dyn Fn()>,
    pub on_set_cover: Rc<dyn Fn()>,
}

/// The actions the menu items name, with the mode and crop states as given. The page installs the group under
/// [`ACTION_GROUP`]; [`sync_menu_state`] keeps the two states current.
pub fn menu_actions(mode: ReadingMode, crop: bool, handlers: MenuHandlers) -> gio::SimpleActionGroup {
    let MenuHandlers { on_mode, on_crop, on_settings, on_open_browser, on_share, on_set_cover } = handlers;
    let group = gio::SimpleActionGroup::new();

    let mode_action = gio::SimpleAction::new_stateful(ACTION_MODE, Some(glib::VariantTy::STRING), &mode_target(mode).to_variant());
    mode_action.connect_change_state(move |action, value| {
        let Some(value) = value else { return };
        let Some(mode) = value.str().and_then(mode_from_target) else { return };
        action.set_state(value);
        on_mode(mode);
    });
    group.add_action(&mode_action);

    let crop_action = gio::SimpleAction::new_stateful(ACTION_CROP, None, &crop.to_variant());
    crop_action.connect_change_state(move |action, value| {
        let Some(value) = value else { return };
        action.set_state(value);
        on_crop();
    });
    group.add_action(&crop_action);

    for (name, run) in [
        (ACTION_SETTINGS, on_settings),
        (ACTION_OPEN_BROWSER, on_open_browser),
        (ACTION_SHARE, on_share),
        (ACTION_SET_COVER, on_set_cover),
    ] {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| run());
        group.add_action(&action);
    }
    group
}

/// Writes the mode and crop states without running their handlers.
pub fn sync_menu_state(group: &gio::SimpleActionGroup, mode: ReadingMode, crop: bool) {
    let set = |name: &str, state: glib::Variant| {
        if let Some(action) = group.lookup_action(name).and_then(|a| a.downcast::<gio::SimpleAction>().ok()) {
            action.set_state(&state);
        }
    };
    set(ACTION_MODE, mode_target(mode).to_variant());
    set(ACTION_CROP, crop.to_variant());
}

/// The web address of a chapter for the overflow entries: only an absolute http or https url has one. A relative
/// source url needs the source's base url, which the reader does not have.
pub fn web_url(chapter_url: &str) -> Option<String> {
    let url = chapter_url.trim();
    (url.starts_with("https://") || url.starts_with("http://")).then(|| url.to_owned())
}

/// The bookmark action's label for the chapter's state.
pub fn bookmark_label(bookmarked: bool) -> String {
    if bookmarked { strings::action_remove_bookmark() } else { strings::action_bookmark() }
}

/// The chapter list button's tooltip, with its key.
pub fn chapters_tooltip() -> String {
    format!("{} (L)", strings::chapters())
}

/// The error card: the message, a Retry button and a Close button. The reader does not close by itself.
pub fn build_error_card(error: &LoadError, on_retry: impl Fn() + 'static, on_close: impl Fn() + 'static) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Center);
    card.set_margin_start(16);
    card.set_margin_end(16);
    let label = gtk::Label::new(Some(&error_card_message(error)));
    label.set_wrap(true);
    label.set_max_width_chars(48);
    card.append(&label);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::Center);
    let retry = gtk::Button::with_label(&strings::retry());
    retry.connect_clicked(move |_| on_retry());
    let close = gtk::Button::with_label(&strings::close());
    close.connect_clicked(move |_| on_close());
    row.append(&retry);
    row.append(&close);
    card.append(&row);
    card
}

/// What the top bar shows and does. Callbacks are plain closures, run on the GTK thread.
pub struct TopBar {
    pub manga_title: String,
    pub chapter_name: String,
    pub bookmarked: bool,
    pub is_http_source: bool,
}

/// The top bar actions. The overflow menu's own actions are [`menu_actions`].
pub struct TopBarActions {
    pub on_back: Box<dyn Fn()>,
    pub on_bookmark: Box<dyn Fn()>,
    /// Opens or closes the chapter list.
    pub on_chapters: Box<dyn Fn()>,
}

/// The top bar before a chapter is known (the page list is loading, or the load failed): Back and the window buttons,
/// with no title. The reader always has a bar, so the window can be dragged while it loads.
pub fn build_bare_top_bar(on_back: impl Fn() + 'static) -> gtk::HeaderBar {
    page_header::header(&gtk::Box::new(gtk::Orientation::Horizontal, 0), on_back)
}

/// The top bar: the themed header the player also uses (docs/design/player-reader.md), in the same format: Back on the
/// left, the manga over the chapter and its page position centred (a plain title, as the player's), and
/// the bookmark and the menu on the right. The bar is never hidden. The menu's actions come from the page's
/// [`ACTION_GROUP`]; the returned title takes the page position.
pub fn build_top_bar(bar: &TopBar, actions: TopBarActions) -> (gtk::HeaderBar, page_header::TwoLineTitle) {
    let TopBarActions { on_back, on_bookmark, on_chapters } = actions;
    let (header, title) = page_header::media_header(&page_header::MediaTitle::new(&bar.manga_title, &bar.chapter_name), on_back);

    let bookmark = gtk::ToggleButton::builder()
        .icon_name(if bar.bookmarked { "bookmark-filled-symbolic" } else { "bookmark-new-symbolic" })
        .active(bar.bookmarked)
        .tooltip_text(bookmark_label(bar.bookmarked))
        .build();
    bookmark.connect_clicked(move |_| on_bookmark());
    bookmark.update_property(&[gtk::accessible::Property::Label(&bookmark_label(bar.bookmarked))]);

    let popover = gtk::PopoverMenu::from_model(Some(&overflow_menu(bar.is_http_source)));
    // no tooltip: it would sit on top of the open menu; the accessible label below still names the button
    let menu = gtk::MenuButton::builder().icon_name("view-more-symbolic").popover(&popover).build();
    menu.update_property(&[gtk::accessible::Property::Label(&strings::overflow_menu())]);

    let chapters = gtk::Button::builder().icon_name("view-list-symbolic").tooltip_text(chapters_tooltip()).build();
    chapters.connect_clicked(move |_| on_chapters());

    // pack_end puts the first widget packed at the far right: the menu, then the bookmark, then the chapter list
    header.pack_end(&menu);
    header.pack_end(&bookmark);
    header.pack_end(&chapters);
    // the buttons packed here came after page_header::header made its pass, so they need their own
    crate::ui::no_sticky_focus(&header);
    (header, title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_card_message_uses_the_load_error_and_never_a_blank_text() {
        assert_eq!(error_card_message(&LoadError::new("No pages found")), "Failed to load pages: No pages found");
        assert_eq!(error_card_message(&LoadError::new("  ")), "Failed to load pages: Unknown error");
    }

    #[test]
    fn the_downloaded_only_card_has_no_failure_prefix() {
        let error = LoadError::new(crate::reader::session::list_error_message(&crate::reader::chapters::ListError::CurrentChapterMissing));
        assert_eq!(
            error_card_message(&error),
            "The chapter is not downloaded. Turn off downloaded-only mode or download it first."
        );
    }

    #[test]
    fn only_an_absolute_chapter_url_has_a_web_address() {
        assert_eq!(web_url(" https://example.org/c/1 ").as_deref(), Some("https://example.org/c/1"));
        assert_eq!(web_url("/c/1"), None);
        assert_eq!(web_url(""), None);
    }

    #[test]
    fn bookmark_label_follows_the_state() {
        assert_eq!(bookmark_label(false), "Bookmark chapter");
        assert_eq!(bookmark_label(true), "Unbookmark chapter");
    }
}
