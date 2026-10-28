//! Headless integration tests for the reader chrome (T-43a-085): the navigator with its RTL mirror, the bottom bar with its
//! crop toggle, and the top bar with the bookmark label and its menu. They build real widgets under [`crate::testing::with_display`], find the buttons by walking the widget tree, and skip when
//! there is no display. CI runs them under a virtual display (`xvfb-run`) so they do not skip there.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use kuro_prefs::reader::ReadingMode;

use super::chrome::{MODES, MenuHandlers, TopBar, TopBarActions, build_top_bar, menu_actions, mode_label, overflow_menu, sync_menu_state};
use super::navigator::{Navigator, NavigatorState};
use super::strings;

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
    NavigatorState { total: 10, current_page: 3, previous_enabled, next_enabled, rtl, loading: false }
}

#[test]
fn the_navigator_puts_next_on_the_left_in_rtl_and_disables_the_missing_chapter() {
    crate::testing::with_display(|| {
        let navigator = Navigator::new();
        navigator.update(&navigator_state(true, true, false), (800, 600));
        let buttons = buttons_under(navigator.root.upcast_ref());
        let next = position_of(&buttons, &strings::action_next_chapter()).expect("a next button");
        let previous = position_of(&buttons, &strings::action_previous_chapter()).expect("a previous button");
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
        let next = position_of(&buttons, &strings::action_next_chapter()).expect("a next button");
        let previous = position_of(&buttons, &strings::action_previous_chapter()).expect("a previous button");
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
        for gone in [strings::viewer(), strings::pref_crop_borders(), strings::action_settings()] {
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
        assert!(position_of(&buttons, &strings::action_remove_bookmark()).is_some(), "a bookmarked chapter offers to unbookmark");
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
            let text = |name: &str| menu.item_attribute_value(i, name, None).and_then(|v| v.get::<String>());
            (text("label").unwrap_or_default(), text("action"), text("target"))
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

#[test]
fn the_menu_model_has_five_radio_modes_then_crop_and_settings_then_the_source_entries() {
    let web = sections_of(true);
    assert_eq!(web.len(), 3);
    assert_eq!(web[0].0.as_deref(), Some(strings::pref_category_reading_mode()).as_deref());
    let modes: Vec<_> = MODES
        .iter()
        .map(|(target, mode)| (mode_label(*mode).to_owned(), Some("reader.mode".to_owned()), Some((*target).to_owned())))
        .collect();
    assert_eq!(web[0].1, modes, "five radio items on one action, each with its mode as the target");
    assert_eq!(modes.len(), 5);
    assert_eq!(modes[0].0, "Left to right");
    assert_eq!(web[1].1, [item(&strings::pref_crop_borders(), "reader.crop"), item(&strings::menu_reader_settings(), "reader.settings")]);
    assert_eq!(web[2].1, [item(&strings::action_open_in_browser(), "reader.open-browser"), item(&strings::action_share(), "reader.share")]);
    assert_eq!(sections_of(false)[2].1, [item(&strings::set_as_cover(), "reader.set-cover")], "a local manga sets the cover instead");
}

#[test]
fn choosing_a_radio_mode_applies_it_and_the_state_is_the_current_mode() {
    let chosen: Rc<RefCell<Vec<ReadingMode>>> = Rc::default();
    let crops = Rc::new(std::cell::Cell::new(0));
    let (sink, crop_sink) = (Rc::clone(&chosen), Rc::clone(&crops));
    let noop: Rc<dyn Fn()> = Rc::new(|| {});
    let handlers = MenuHandlers {
        on_mode: Rc::new(move |mode| sink.borrow_mut().push(mode)),
        on_crop: Rc::new(move || crop_sink.set(crop_sink.get() + 1)),
        on_settings: Rc::clone(&noop),
        on_open_browser: Rc::clone(&noop),
        on_share: Rc::clone(&noop),
        on_set_cover: noop,
    };
    let group = menu_actions(ReadingMode::RightToLeft, true, handlers);
    let state = |name: &str| group.action_state(name).expect("a stateful action");
    assert_eq!(state("mode").str(), Some("rtl"), "the state is the current mode");
    assert_eq!(state("crop").get::<bool>(), Some(true));

    group.change_action_state("mode", &"webtoon".to_variant());
    assert_eq!(*chosen.borrow(), [ReadingMode::Webtoon], "the choice applies at once");
    assert_eq!(state("mode").str(), Some("webtoon"));
    group.change_action_state("mode", &"nonsense".to_variant());
    assert_eq!(chosen.borrow().len(), 1, "an unknown target does nothing");

    group.change_action_state("crop", &false.to_variant());
    assert_eq!((crops.get(), state("crop").get::<bool>()), (1, Some(false)));

    sync_menu_state(&group, ReadingMode::LeftToRight, true);
    assert_eq!((state("mode").str(), state("crop").get::<bool>()), (Some("ltr"), Some(true)));
    assert_eq!((chosen.borrow().len(), crops.get()), (1, 1), "syncing the states runs no handler");
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
