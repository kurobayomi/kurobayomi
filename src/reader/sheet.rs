//! The reader's bottom sheet (T-043a-079): the page actions. A revealer over a scrim inside the reader's overlay; Escape
//! and a click outside the sheet dismiss it. The reader settings are in the top bar's menu (`chrome`). The confirm
//! prompt uses the shared dialog. No libadwaita.

use kuro_i18n::{MsgKey, tr};
use crate::ui::DialogOpen as _;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use crate::ui::{DialogButton, Role};

/// A bottom sheet over `base`. Only one content is shown at a time; `show` replaces it.
pub struct Sheet {
    overlay: gtk::Overlay,
    scrim: gtk::Box,
    revealer: gtk::Revealer,
    container: gtk::Box,
    open: Cell<bool>,
    on_dismissed: RefCell<Option<Rc<dyn Fn()>>>,
}

impl Sheet {
    pub fn new(base: &impl IsA<gtk::Widget>) -> Rc<Self> {
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(base.as_ref()));
        let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scrim.add_css_class("reader-scrim");
        scrim.set_hexpand(true);
        scrim.set_vexpand(true);
        scrim.set_visible(false);
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.set_margin_top(16);
        container.set_margin_bottom(16);
        container.set_margin_start(16);
        container.set_margin_end(16);
        container.add_css_class("background");
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideUp);
        revealer.set_transition_duration(200);
        revealer.set_valign(gtk::Align::End);
        revealer.set_halign(gtk::Align::Fill);
        revealer.set_child(Some(&container));
        overlay.add_overlay(&scrim);
        overlay.add_overlay(&revealer);

        let sheet = Rc::new(Self {
            overlay: overlay.clone(),
            scrim: scrim.clone(),
            revealer,
            container,
            open: Cell::new(false),
            on_dismissed: RefCell::new(None),
        });
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&sheet);
        click.connect_pressed(move |_, _, _, _| {
            if let Some(sheet) = weak.upgrade() {
                sheet.dismiss();
            }
        });
        scrim.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&sheet);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key != gtk::gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            match weak.upgrade() {
                Some(sheet) if sheet.is_open() => {
                    sheet.dismiss();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        overlay.add_controller(keys);
        sheet
    }

    /// The widget to put in the reader in place of the base.
    pub fn root(&self) -> &gtk::Overlay {
        &self.overlay
    }

    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Runs when the sheet is dismissed by Escape, a click outside, or [`Sheet::dismiss`].
    pub fn connect_dismissed(&self, f: impl Fn() + 'static) {
        *self.on_dismissed.borrow_mut() = Some(Rc::new(f));
    }

    /// Shows `content` in the sheet, replacing the previous content.
    pub fn show(&self, content: &impl IsA<gtk::Widget>) {
        while let Some(child) = self.container.first_child() {
            self.container.remove(&child);
        }
        self.container.append(content.as_ref());
        self.scrim.set_visible(true);
        self.revealer.set_reveal_child(true);
        self.open.set(true);
        // the focus goes into the sheet, so Enter and the arrows reach it and Escape is not left to the bottom bar
        let container = self.container.clone();
        glib::idle_add_local_once(move || {
            if let Some(first) = first_focusable(container.upcast_ref()) {
                first.grab_focus();
            }
        });
    }

    /// Hides the sheet and runs the dismissed callback once. A sheet that is not open does nothing.
    pub fn dismiss(&self) {
        if !self.open.replace(false) {
            return;
        }
        self.revealer.set_reveal_child(false);
        self.scrim.set_visible(false);
        let callback = self.on_dismissed.borrow().clone();
        if let Some(callback) = callback {
            callback();
        }
    }
}

/// The first widget under `root`, in tree order, that can take the focus.
fn first_focusable(root: &gtk::Widget) -> Option<gtk::Widget> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if widget.is_visible() && widget.is_sensitive() && widget.is_focusable() && widget.can_focus() {
            return Some(widget);
        }
        if let Some(found) = first_focusable(&widget) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

/// Asks a yes or no question with the shared dialog. `on_ok` runs only for OK. Cancel and outside dismissal do
/// nothing.
pub fn confirm(parent: &impl IsA<gtk::Window>, message: &str, on_ok: impl FnOnce() + 'static) {
    let on_ok = std::cell::Cell::new(Some(on_ok));
    let ok = DialogButton::new(tr(MsgKey::action_ok), Role::Suggested, move || {
        if let Some(on_ok) = on_ok.take() {
            on_ok();
        }
    });
    crate::ui::alert(message, "", vec![DialogButton::cancel(tr(MsgKey::action_cancel)), ok], || {}).open_over(parent.as_ref());
}
