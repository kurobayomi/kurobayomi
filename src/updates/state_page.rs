//! Page states as a `gtk::Stack` (T-42a-008, T-42a-022): a spinner while loading, the empty state (icon, message, one
//! action), the error state with a retry, and the list. The child names are the ones [`PageState::child_name`] gives.
//! Widgets only; the rules are in [`crate::updates::list`] and [`crate::updates::updates_page`]. The loading, empty and
//! error children are the shared ones of `widgets::status`.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Button, Stack};

use crate::updates::list::PageState;
use crate::widgets::status;

pub struct PageStates {
    stack: Stack,
    empty: adw::StatusPage,
    /// Runs the load again; the error child is rebuilt with each message, so it keeps this.
    on_retry: Rc<dyn Fn()>,
}

impl PageStates {
    /// The stack with its four children. `list` is the page's list widget; `on_retry` runs the load again.
    pub fn new(list: &impl IsA<gtk::Widget>, on_retry: impl Fn() + 'static) -> Self {
        let stack = status::switcher();
        stack.set_vexpand(true);
        stack.add_named(&status::loading(None), Some("loading"));

        // the empty look of the Library page (docs/design/other-tabs.md); the screen sets its icon and action
        let empty = status::empty("view-list-symbolic", "", None);
        stack.add_named(&empty, Some("empty"));

        // the error child is added by `show`, with its message
        stack.add_named(list, Some("list"));
        Self { stack, empty, on_retry: Rc::new(on_retry) }
    }

    /// The icon of the empty page and its one action (none by default).
    pub fn set_empty_look(&self, icon: &str, action: Option<&Button>) {
        self.empty.set_icon_name(Some(icon));
        if let Some(action) = action {
            action.add_css_class("pill");
            self.empty.set_child(Some(action));
        }
    }

    pub fn widget(&self) -> &Stack {
        &self.stack
    }

    /// Shows the child of `state`, and sets the message the empty or error child carries.
    pub fn show(&self, state: &PageState) {
        match state {
            PageState::Empty(message) => self.empty.set_title(message),
            PageState::Error(message) => {
                if let Some(old) = self.stack.child_by_name("error") {
                    self.stack.remove(&old);
                }
                let retry = Rc::clone(&self.on_retry);
                self.stack.add_named(&status::error(message, None, Some(move || retry())), Some("error"));
            }
            PageState::Loading | PageState::List => {}
        }
        self.stack.set_visible_child_name(state.child_name());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_empty_page_has_the_library_look_and_one_action() {
        crate::testing::with_display(|| {
            let pages = PageStates::new(&gtk::Label::new(Some("rows")), || {});
            let action = Button::with_label(&kuro_i18n::tr(kuro_i18n::MsgKey::action_update_library));
            pages.set_empty_look("software-update-available-symbolic", Some(&action));
            pages.show(&PageState::Empty("No recent updates".into()));
            assert_eq!(pages.stack.visible_child_name().as_deref(), Some("empty"));
            assert_eq!(pages.empty.title(), "No recent updates");
            assert_eq!(pages.empty.icon_name().as_deref(), Some("software-update-available-symbolic"));
            assert_eq!(pages.empty.child().as_ref(), Some(action.upcast_ref::<gtk::Widget>()));
            pages.show(&PageState::Error("boom".into()));
            assert_eq!(pages.stack.visible_child_name().as_deref(), Some("error"));
            let error = pages.stack.child_by_name("error").and_downcast::<adw::StatusPage>().expect("the error page");
            assert_eq!(error.title(), "boom");
        });
    }
}
