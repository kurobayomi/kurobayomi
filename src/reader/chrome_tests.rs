//! Headless integration tests for the reader chrome (T-43a-085): the navigator with its RTL mirror, the bottom bar with its
//! crop toggle, and the top bar with the bookmark label and its menu. They build real widgets under [`crate::testing::with_display`], find the buttons by walking the widget tree, and skip when
//! there is no display. CI runs them under a virtual display (`xvfb-run`) so they do not skip there.

use kuro_i18n::{MsgKey, tr};
use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use kuro_prefs::reader::ReadingMode;

use super::chrome::{Choice, MODES, MenuHandlers, MenuState, TopBar, TopBarActions, build_top_bar, menu_actions, mode_label, overflow_menu, sync_menu_state};
use super::navigator::{Navigator, NavigatorState};

/// Every button under `root`, in tree order.
fn buttons_under(root: &gtk::Widget) -> Vec<gtk::Button> {
    let mut found = Vec::new();
    collect(root, &mut found);
    found
}

fn collect(widget: &gtk::Widget, found: &mut Vec<gtk::Button>) {
    if let Some(button) = widget.downcast_ref::<gtk::Button>() {
        found.push(button.clone());
    }
    let mut child = widget.first_child();
    while let Some(next) = child {
        collect(&next, found);
        child = next.next_sibling();
    }
}

/// The index of the button whose tooltip is `text`, in tree order.
fn position_of(buttons: &[gtk::Button], text: &str) -> Option<usize> {
    buttons.iter().position(|b| b.tooltip_text().as_deref() == Some(text))
}

fn navigator_state(rtl: bool, previous_enabled: bool, next_enabled: bool) -> NavigatorState {
    NavigatorState { total: 10, current_page: 3, beside: None, previous_enabled, next_enabled, rtl, loading: false }
}

#[test]
fn the_navigator_puts_next_on_the_left_in_rtl_and_disables_the_missing_chapter() {
    crate::testing::with_display(|| {
        let navigator = Navigator::new();
        navigator.update(&navigator_state(true, true, false), (800, 600));
        let buttons = buttons_under(navigator.root.upcast_ref());
        let next = position_of(&buttons, &tr(MsgKey::action_next_chapter)).expect("a next button");
        let previous = position_of(&buttons, &tr(MsgKey::action_previous_chapter)).expect("a previous button");
        assert!(next < previous, "in RTL the next chapter button is on the left");
        assert!(!buttons[next].is_sensitive(), "no next chapter, so the button is disabled");
        assert!(buttons[previous].is_sensitive());
    });
}

#[test]
fn the_navigator_keeps_previous_on_the_left_in_ltr() {
    crate::testing::with_display(|| {
        let navigator = Navigator::new();
        navigator.update(&navigator_state(false, true, true), (800, 600));
        let buttons = buttons_under(navigator.root.upcast_ref());
        let next = position_of(&buttons, &tr(MsgKey::action_next_chapter)).expect("a next button");
        let previous = position_of(&buttons, &tr(MsgKey::action_previous_chapter)).expect("a previous button");
        assert!(previous < next);
        assert!(buttons[next].is_sensitive() && buttons[previous].is_sensitive());
    });
}

#[test]
fn the_navigator_is_the_whole_bottom_bar_and_has_no_settings_buttons() {
    crate::testing::with_display(|| {
        let navigator = Navigator::new();
        navigator.update(&navigator_state(false, true, true), (800, 600));
        let buttons = buttons_under(navigator.root.upcast_ref());
        assert_eq!(buttons.len(), 2, "only the previous and next chapter buttons");
        for gone in [tr(MsgKey::viewer), tr(MsgKey::pref_crop_borders), tr(MsgKey::action_settings)] {
            assert!(position_of(&buttons, &gone).is_none());
        }
    });
}

#[test]
fn the_top_bar_labels_the_bookmark_by_its_state_and_has_a_named_back_button() {
    crate::testing::with_display(|| {
        let bar = TopBar { manga_title: "Manga".into(), chapter_name: "Chapter 1".into(), bookmarked: true, is_http_source: false };
        let actions = TopBarActions { on_back: Box::new(|| {}), on_bookmark: Box::new(|| {}), on_chapters: Box::new(|| {}) };
        let (header, title) = build_top_bar(&bar, actions);
        // the player's format: a plain two-line title that takes no clicks
        assert_eq!((title.title.label().as_str(), title.subtitle().as_str()), ("Manga", "Chapter 1"), "the manga over the chapter");
        assert_eq!(title.root.observe_controllers().n_items(), 0, "the title is not clickable");
        let buttons = buttons_under(header.upcast_ref());
        assert!(position_of(&buttons, &tr(MsgKey::action_remove_bookmark)).is_some(), "a bookmarked chapter offers to unbookmark");
        assert!(position_of(&buttons, "Back").is_some(), "the back button is named");
        assert!(position_of(&buttons, &super::chrome::chapters_tooltip()).is_some(), "the chapter list has its button");
        let menu = all_of::<gtk::MenuButton>(header.upcast_ref()).pop().expect("the menu button");
        assert!(menu.tooltip_text().is_none(), "no tooltip to cover the open menu");
    });
}

/// Every widget of type `T` under `root`, in tree order.
fn all_of<T: IsA<gtk::Widget>>(root: &gtk::Widget) -> Vec<T> {
    let mut found = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(widget) = stack.pop() {
        if let Some(it) = widget.downcast_ref::<T>() {
            found.push(it.clone());
        }
        let mut children = Vec::new();
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            children.push(next);
        }
        stack.extend(children.into_iter().rev());
    }
    found
}

/// The labelled items of a menu model section: label, action, target.
fn items_of(menu: &gtk::gio::MenuModel) -> Vec<(String, Option<String>, Option<String>)> {
    (0..menu.n_items())
        .map(|i| {
            let value = |name: &str| menu.item_attribute_value(i, name, None);
            let text = |name: &str| value(name).and_then(|v| v.get::<String>());
            // a radio target is the mode's name or a stored code
            let target = value("target").and_then(|v| v.get::<String>().or_else(|| v.get::<i32>().map(|n| n.to_string())));
            (text("label").unwrap_or_default(), text("action"), target)
        })
        .collect()
}

/// The three sections of the top bar menu, each with its label and items.
fn sections_of(is_http_source: bool) -> Vec<(Option<String>, Vec<(String, Option<String>, Option<String>)>)> {
    let menu = overflow_menu(is_http_source);
    (0..menu.n_items())
        .map(|i| {
            let label = menu.item_attribute_value(i, "label", None).and_then(|v| v.get::<String>());
            let section = menu.item_link(i, "section").expect("a section");
            (label, items_of(&section))
        })
        .collect()
}

fn item(label: &str, action: &str) -> (String, Option<String>, Option<String>) {
    (label.to_owned(), Some(action.to_owned()), None)
}

/// The submenus of a section: each one's label and its items.
fn submenus_of(section: &gtk::gio::MenuModel) -> Vec<(String, Vec<(String, Option<String>, Option<String>)>)> {
    (0..section.n_items())
        .filter_map(|i| {
            let sub = section.item_link(i, "submenu")?;
            let label = section.item_attribute_value(i, "label", None).and_then(|v| v.get::<String>()).unwrap_or_default();
            Some((label, items_of(&sub)))
        })
        .collect()
}

fn radios(action: &str, options: &[(i32, String)]) -> Vec<(String, Option<String>, Option<String>)> {
    options.iter().map(|(code, label)| (label.clone(), Some(format!("reader.{action}")), Some(code.to_string()))).collect()
}

#[test]
fn the_menu_holds_every_reader_setting_and_no_browser_or_share_entries() {
    let web = sections_of(true);
    assert_eq!(web.len(), 3, "reading mode, page layout, then the page settings");
    assert_eq!(web[0].0.as_deref(), Some(tr(MsgKey::pref_category_reading_mode)).as_deref());
    let modes: Vec<_> = MODES
        .iter()
        .map(|(target, mode)| (mode_label(*mode).to_owned(), Some("reader.mode".to_owned()), Some((*target).to_owned())))
        .collect();
    assert_eq!(web[0].1, modes, "five radio items on one action, each with its mode as the target");
    assert_eq!(web[1].0.as_deref(), Some("Page layout"));
    assert_eq!(web[1].1, radios("pages", &Choice::PageLayout.options()));
    assert_eq!(web[1].1.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(), ["Two pages when wide", "Two pages", "One page"]);
    let page = overflow_menu(true).item_link(2, "section").unwrap();
    let checks: Vec<_> = items_of(&page).into_iter().filter(|(_, action, _)| action.is_some()).collect();
    assert_eq!(checks, [item(&tr(MsgKey::pref_crop_borders), "reader.crop"), item(&tr(MsgKey::pref_dual_page_split), "reader.split")]);
    let subs = submenus_of(&page);
    assert_eq!(
        subs,
        [
            (tr(MsgKey::pref_reader_theme), radios("background", &Choice::Background.options())),
            (tr(MsgKey::pref_image_scale_type), radios("scale", &Choice::ScaleType.options())),
            (tr(MsgKey::pref_zoom_start), radios("zoom-start", &Choice::ZoomStart.options())),
        ]
    );
    let all: Vec<String> = web.iter().flat_map(|(_, items)| items.iter().map(|i| i.0.clone())).chain(subs.iter().map(|s| s.0.clone())).collect();
    for gone in [tr(MsgKey::action_open_in_browser), tr(MsgKey::action_share), tr(MsgKey::reader_menu_reader_settings)] {
        assert!(!all.contains(&gone), "{gone} is not in the menu");
    }
    let local = sections_of(false);
    assert_eq!(local.len(), 4);
    assert_eq!(local[3].1, [item(&tr(MsgKey::set_as_cover), "reader.set-cover")], "a local manga can set the page as its cover");
}

#[test]
fn choosing_in_the_menu_stores_it_and_the_states_follow_the_open_manga() {
    let chosen: Rc<RefCell<Vec<ReadingMode>>> = Rc::default();
    let crops = Rc::new(std::cell::Cell::new(0));
    let picks: Rc<RefCell<Vec<(Choice, i32)>>> = Rc::default();
    let splits: Rc<RefCell<Vec<bool>>> = Rc::default();
    let (sink, crop_sink, pick_sink, split_sink) = (Rc::clone(&chosen), Rc::clone(&crops), Rc::clone(&picks), Rc::clone(&splits));
    let handlers = MenuHandlers {
        on_mode: Rc::new(move |mode| sink.borrow_mut().push(mode)),
        on_crop: Rc::new(move || crop_sink.set(crop_sink.get() + 1)),
        on_split: Rc::new(move |on| split_sink.borrow_mut().push(on)),
        on_choice: Rc::new(move |choice, code| pick_sink.borrow_mut().push((choice, code))),
        on_set_cover: Rc::new(|| {}),
    };
    let group = menu_actions(MenuState { mode: ReadingMode::RightToLeft, crop: true, split: false, choices: [0, 1, 1, 1] }, handlers);
    let state = |name: &str| group.action_state(name).expect("a stateful action");
    assert_eq!(state("mode").str(), Some("rtl"), "the state is the current mode");
    assert_eq!((state("crop").get::<bool>(), state("split").get::<bool>()), (Some(true), Some(false)));
    assert_eq!((state("pages").get::<i32>(), state("background").get::<i32>()), (Some(0), Some(1)));

    group.change_action_state("mode", &"webtoon".to_variant());
    assert_eq!(*chosen.borrow(), [ReadingMode::Webtoon], "the choice applies at once");
    group.change_action_state("mode", &"nonsense".to_variant());
    assert_eq!(chosen.borrow().len(), 1, "an unknown target does nothing");
    group.change_action_state("crop", &false.to_variant());
    assert_eq!((crops.get(), state("crop").get::<bool>()), (1, Some(false)));
    group.change_action_state("split", &true.to_variant());
    group.change_action_state("pages", &2.to_variant());
    group.change_action_state("background", &3.to_variant());
    group.change_action_state("scale", &99.to_variant());
    assert_eq!(*splits.borrow(), [true]);
    assert_eq!(*picks.borrow(), [(Choice::PageLayout, 2), (Choice::Background, 3)], "a code the choice does not offer does nothing");

    sync_menu_state(&group, MenuState { mode: ReadingMode::LeftToRight, crop: true, split: true, choices: [1, 2, 3, 4] });
    assert_eq!((state("mode").str(), state("pages").get::<i32>(), state("zoom-start").get::<i32>()), (Some("ltr"), Some(1), Some(4)));
    assert_eq!((chosen.borrow().len(), crops.get(), picks.borrow().len(), splits.borrow().len()), (1, 1, 2, 1), "syncing runs no handler");
    assert!(["pages", "split", "scale", "zoom-start", "background"].iter().all(|a| group.is_action_enabled(a)));
    sync_menu_state(&group, MenuState { mode: ReadingMode::Vertical, crop: true, split: true, choices: [1, 2, 3, 4] });
    assert!(!group.is_action_enabled("pages") && group.is_action_enabled("scale"), "vertical pages turn one at a time, never side by side");
    sync_menu_state(&group, MenuState { mode: ReadingMode::Webtoon, crop: true, split: true, choices: [1, 2, 3, 4] });
    for greyed in ["pages", "split", "scale", "zoom-start"] {
        assert!(!group.is_action_enabled(greyed), "{greyed} does nothing in the strip");
    }
    assert!(group.is_action_enabled("background") && group.is_action_enabled("crop"));
}

#[test]
fn the_top_bar_menu_is_a_native_popover_menu_and_the_title_has_a_position_line() {
    crate::testing::with_display(|| {
        let bar = TopBar { manga_title: "M".into(), chapter_name: "C".into(), bookmarked: false, is_http_source: true };
        let actions = TopBarActions { on_back: Box::new(|| {}), on_bookmark: Box::new(|| {}), on_chapters: Box::new(|| {}) };
        let (header, title) = build_top_bar(&bar, actions);
        let menu = all_of::<gtk::MenuButton>(header.upcast_ref()).pop().expect("the menu button");
        let popover = menu.popover().expect("the popover");
        assert!(popover.downcast_ref::<gtk::PopoverMenu>().is_some_and(|p| p.menu_model().is_some()), "a menu model, not hand-made buttons");
        assert_eq!(title.subtitle(), "C", "the chapter alone before a page is shown");
        title.set_subtitle(Some("Page 1 of 23"));
        assert_eq!(title.subtitle(), "C · Page 1 of 23");
    });
}
