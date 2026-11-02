//! Reader chrome: the initial load error card (T-43a-024) and the top bar (T-43a-040).
//!
//! The pure helpers hold the text and the source-dependent overflow rule, so they are tested without GTK.
//! The widget builders only assemble them.

use kuro_i18n::{Arg, MsgKey, tr, trf};
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use kuro_prefs::reader::ReadingMode;

use super::model::LoadError;
use crate::widgets::page_header;

/// The card text for an initial load failure. A blank message shows the generic text, so no card is empty. The
/// downloaded-only text is shown as is, since it is not a page-load failure (T-43a-074).
pub fn error_card_message(error: &LoadError) -> String {
    if error.message == tr(MsgKey::reader_chapter_not_downloaded) {
        return error.message.clone();
    }
    let message = if error.message.trim().is_empty() { tr(MsgKey::unknown_error) } else { error.message.as_str().to_string() };
    trf(MsgKey::transition_pages_error, &[("arg1", Arg::Str(&message))])
}

/// The action group the reader page installs, so the menu's items name their actions as `reader.<name>`.
pub const ACTION_GROUP: &str = "reader";
const ACTION_MODE: &str = "mode";
const ACTION_CROP: &str = "crop";
const ACTION_SPLIT: &str = "split";
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
        ReadingMode::Default => tr(MsgKey::label_default),
        ReadingMode::LeftToRight => tr(MsgKey::left_to_right_viewer),
        ReadingMode::RightToLeft => tr(MsgKey::right_to_left_viewer),
        ReadingMode::Vertical => tr(MsgKey::vertical_viewer),
        ReadingMode::Webtoon => tr(MsgKey::webtoon_viewer),
        ReadingMode::ContinuousVertical => tr(MsgKey::vertical_plus_viewer),
    }
}

/// The action target of a mode; Default has none, so no radio item shows as chosen.
fn mode_target(mode: ReadingMode) -> &'static str {
    MODES.iter().find(|(_, m)| *m == mode).map_or("", |(target, _)| target)
}

fn mode_from_target(target: &str) -> Option<ReadingMode> {
    MODES.iter().find(|(t, _)| *t == target).map(|(_, mode)| *mode)
}

/// The page layout codes (`pref_two_pages`): two pages when the window is wide (the default), always two, always one.
/// They apply in the left-to-right and right-to-left modes only.
pub const PAGE_LAYOUTS: [i32; 3] = [0, 1, 2];

/// The label of a page layout, in the menu and on the settings page.
pub fn page_layout_label(code: i32) -> String {
    tr(match code {
        1 => MsgKey::two_pages_always,
        2 => MsgKey::two_pages_never,
        _ => MsgKey::two_pages_automatic,
    })
}

/// Whether `mode` shows one page at a time (left to right, right to left, vertical), not a scrolling strip.
fn page_by_page(mode: ReadingMode) -> bool {
    mode.viewer_type() == Ok(kuro_prefs::reader::ViewerType::Pager)
}

/// Whether the page layout applies in `mode`: the page-by-page modes that turn sideways.
pub fn page_layout_applies(mode: ReadingMode) -> bool {
    matches!(mode, ReadingMode::LeftToRight | ReadingMode::RightToLeft)
}

/// The reader settings the menu offers as one choice of several, each a radio group on an action of its own whose
/// state is the stored code. The labels and codes are the ones the Settings page shows (`settings_ui::reader::page`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// `pref_two_pages`, a section of its own under the reading mode.
    PageLayout,
    /// `pref_reader_theme_key`.
    Background,
    /// `pref_image_scale_type_key`.
    ScaleType,
    /// `pref_zoom_start_key`.
    ZoomStart,
}

impl Choice {
    pub const ALL: [Choice; 4] = [Choice::PageLayout, Choice::Background, Choice::ScaleType, Choice::ZoomStart];

    fn action(self) -> &'static str {
        match self {
            Choice::PageLayout => "pages",
            Choice::Background => "background",
            Choice::ScaleType => "scale",
            Choice::ZoomStart => "zoom-start",
        }
    }

    fn title(self) -> String {
        tr(match self {
            Choice::PageLayout => MsgKey::pref_two_pages,
            Choice::Background => MsgKey::pref_reader_theme,
            Choice::ScaleType => MsgKey::pref_image_scale_type,
            Choice::ZoomStart => MsgKey::pref_zoom_start,
        })
    }

    /// The codes and labels, in menu order.
    pub fn options(self) -> Vec<(i32, String)> {
        use crate::settings_ui::reader::page as rows;
        match self {
            Choice::PageLayout => PAGE_LAYOUTS.iter().map(|&code| (code, page_layout_label(code))).collect(),
            Choice::Background => rows::backgrounds().to_vec(),
            Choice::ScaleType => rows::scale_types().to_vec(),
            Choice::ZoomStart => rows::zoom_starts().to_vec(),
        }
    }

    /// Whether the choice does anything in `mode`: the page layout in the sideways modes, the scale and the zoom start
    /// in the page-by-page modes (the strip fits the width), the background everywhere.
    pub fn applies(self, mode: ReadingMode) -> bool {
        match self {
            Choice::PageLayout => page_layout_applies(mode),
            Choice::Background => true,
            Choice::ScaleType | Choice::ZoomStart => page_by_page(mode),
        }
    }
}

fn detailed(action: &str) -> String {
    format!("{ACTION_GROUP}.{action}")
}

fn radio_items(choice: Choice) -> gio::Menu {
    let items = gio::Menu::new();
    for (code, label) in choice.options() {
        let item = gio::MenuItem::new(Some(&label), None);
        item.set_action_and_target_value(Some(&detailed(choice.action())), Some(&code.to_variant()));
        items.append_item(&item);
    }
    items
}

/// The model of the top bar's menu, every reader setting in it (user, 2026-11-02: no separate settings window): Reading
/// mode (five radio items), Page layout (three), then Crop borders and Split wide pages, and Background color, Scale
/// type and Zoom start position as submenus. A local manga adds Set as cover (the page on screen).
pub fn overflow_menu(is_http_source: bool) -> gio::Menu {
    let menu = gio::Menu::new();
    let modes = gio::Menu::new();
    for (target, mode) in MODES {
        let item = gio::MenuItem::new(Some(&mode_label(mode)), None);
        item.set_action_and_target_value(Some(&detailed(ACTION_MODE)), Some(&target.to_variant()));
        modes.append_item(&item);
    }
    menu.append_section(Some(&tr(MsgKey::pref_category_reading_mode)), &modes);
    menu.append_section(Some(&Choice::PageLayout.title()), &radio_items(Choice::PageLayout));
    let page = gio::Menu::new();
    page.append(Some(&tr(MsgKey::pref_crop_borders)), Some(&detailed(ACTION_CROP)));
    page.append(Some(&tr(MsgKey::pref_dual_page_split)), Some(&detailed(ACTION_SPLIT)));
    for choice in [Choice::Background, Choice::ScaleType, Choice::ZoomStart] {
        page.append_submenu(Some(&choice.title()), &radio_items(choice));
    }
    menu.append_section(None, &page);
    if !is_http_source {
        let local = gio::Menu::new();
        local.append(Some(&tr(MsgKey::set_as_cover)), Some(&detailed(ACTION_SET_COVER)));
        menu.append_section(None, &local);
    }
    menu
}

/// What the menu's actions do. Run on the GTK thread.
pub struct MenuHandlers {
    /// A radio item was chosen: apply the mode to this manga at once.
    pub on_mode: Rc<dyn Fn(ReadingMode)>,
    /// The Crop borders switch was flipped.
    pub on_crop: Rc<dyn Fn()>,
    /// Split wide pages was switched to the value given: store it for this manga.
    pub on_split: Rc<dyn Fn(bool)>,
    /// A choice was made (its stored code): store it for this manga. The reader follows the stored values at once.
    pub on_choice: Rc<dyn Fn(Choice, i32)>,
    pub on_set_cover: Rc<dyn Fn()>,
}

/// What the menu's radio and check items show: the open manga's values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuState {
    pub mode: ReadingMode,
    pub crop: bool,
    pub split: bool,
    /// The code of each [`Choice`], in [`Choice::ALL`] order.
    pub choices: [i32; 4],
}

impl MenuState {
    fn code(&self, choice: Choice) -> i32 {
        self.choices[Choice::ALL.iter().position(|c| *c == choice).unwrap_or(0)]
    }
}

/// The actions the menu items name, with the states given. The page installs the group under [`ACTION_GROUP`];
/// [`sync_menu_state`] keeps the states current.
pub fn menu_actions(state: MenuState, handlers: MenuHandlers) -> gio::SimpleActionGroup {
    let MenuHandlers { on_mode, on_crop, on_split, on_choice, on_set_cover } = handlers;
    let group = gio::SimpleActionGroup::new();

    let mode_action = gio::SimpleAction::new_stateful(ACTION_MODE, Some(glib::VariantTy::STRING), &mode_target(state.mode).to_variant());
    mode_action.connect_change_state(move |action, value| {
        let Some(value) = value else { return };
        let Some(mode) = value.str().and_then(mode_from_target) else { return };
        action.set_state(value);
        on_mode(mode);
    });
    group.add_action(&mode_action);

    let crop_action = gio::SimpleAction::new_stateful(ACTION_CROP, None, &state.crop.to_variant());
    crop_action.connect_change_state(move |action, value| {
        let Some(value) = value else { return };
        action.set_state(value);
        on_crop();
    });
    group.add_action(&crop_action);

    let split_action = gio::SimpleAction::new_stateful(ACTION_SPLIT, None, &state.split.to_variant());
    split_action.connect_change_state(move |action, value| {
        let Some(on) = value.and_then(glib::Variant::get::<bool>) else { return };
        action.set_state(&on.to_variant());
        on_split(on);
    });
    group.add_action(&split_action);

    for choice in Choice::ALL {
        let action = gio::SimpleAction::new_stateful(choice.action(), Some(glib::VariantTy::INT32), &state.code(choice).to_variant());
        let on_choice = Rc::clone(&on_choice);
        action.connect_change_state(move |action, value| {
            let Some(code) = value.and_then(glib::Variant::get::<i32>) else { return };
            if !choice.options().iter().any(|(c, _)| *c == code) {
                return;
            }
            action.set_state(&code.to_variant());
            on_choice(choice, code);
        });
        group.add_action(&action);
    }

    let cover = gio::SimpleAction::new(ACTION_SET_COVER, None);
    cover.connect_activate(move |_, _| on_set_cover());
    group.add_action(&cover);
    sync_enabled(&group, state.mode);
    group
}

/// Writes the states without running their handlers.
pub fn sync_menu_state(group: &gio::SimpleActionGroup, state: MenuState) {
    let set = |name: &str, value: glib::Variant| {
        if let Some(action) = simple_action(group, name) {
            action.set_state(&value);
        }
    };
    set(ACTION_MODE, mode_target(state.mode).to_variant());
    set(ACTION_CROP, state.crop.to_variant());
    set(ACTION_SPLIT, state.split.to_variant());
    for choice in Choice::ALL {
        set(choice.action(), state.code(choice).to_variant());
    }
    sync_enabled(group, state.mode);
}

/// Greys out what does nothing in `mode` (the page-by-page settings while the strip scrolls).
fn sync_enabled(group: &gio::SimpleActionGroup, mode: ReadingMode) {
    for choice in Choice::ALL {
        if let Some(action) = simple_action(group, choice.action()) {
            action.set_enabled(choice.applies(mode));
        }
    }
    if let Some(split) = simple_action(group, ACTION_SPLIT) {
        split.set_enabled(page_by_page(mode));
    }
}

fn simple_action(group: &gio::SimpleActionGroup, name: &str) -> Option<gio::SimpleAction> {
    group.lookup_action(name).and_then(|a| a.downcast::<gio::SimpleAction>().ok())
}

/// The bookmark action's label for the chapter's state.
pub fn bookmark_label(bookmarked: bool) -> String {
    if bookmarked { tr(MsgKey::action_remove_bookmark) } else { tr(MsgKey::action_bookmark) }
}

/// The chapter list button's tooltip, with its key.
pub fn chapters_tooltip() -> String {
    format!("{} (L)", tr(MsgKey::chapters))
}

/// The error page: the message, a Retry button and a Close button (the shared `widgets::status` page). The reader does
/// not close by itself.
pub fn build_error_card(error: &LoadError, on_retry: impl Fn() + 'static, on_close: impl Fn() + 'static) -> adw::StatusPage {
    let retry = gtk::Button::with_label(&tr(MsgKey::action_retry));
    retry.connect_clicked(move |_| on_retry());
    let close = gtk::Button::with_label(&tr(MsgKey::action_close));
    close.connect_clicked(move |_| on_close());
    crate::widgets::status::error_with(&error_card_message(error), None, &[retry, close])
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
pub fn build_bare_top_bar(on_back: impl Fn() + 'static) -> adw::HeaderBar {
    page_header::header(&gtk::Box::new(gtk::Orientation::Horizontal, 0), on_back)
}

/// The top bar: the themed header the player also uses (docs/design/player-reader.md), in the same format: Back on the
/// left, the manga over the chapter and its page position centred (a plain title, as the player's), and
/// the bookmark and the menu on the right. The bar is never hidden. The menu's actions come from the page's
/// [`ACTION_GROUP`]; the returned title takes the page position.
pub fn build_top_bar(bar: &TopBar, actions: TopBarActions) -> (adw::HeaderBar, page_header::TwoLineTitle) {
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
    menu.update_property(&[gtk::accessible::Property::Label(&tr(MsgKey::player_sheets_more_title))]);

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
    fn bookmark_label_follows_the_state() {
        assert_eq!(bookmark_label(false), "Bookmark chapter");
        assert_eq!(bookmark_label(true), "Unbookmark chapter");
    }
}
