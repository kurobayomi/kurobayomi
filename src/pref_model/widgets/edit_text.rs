//! Single-line text edit (Kotlin `EditTextPreferenceWidget`, single-line variant). OK is enabled only when
//! the text differs from the value captured at open, is not blank (unless allowed), and validates.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Box as GBox, Entry, EntryIconPosition, Label, Orientation};
use kuro_prefs::pref::Pref;

use super::super::bind::{self, Outcome, Veto, WriteGate};
use super::super::model::{format_subtitle, ItemCommon};
use super::dialog::{self, Dialog};
use crate::widgets::row::{self, set_subtitle, RowSpec};
use super::super::strings;
use crate::theme::tokens::{SPACE_S};

/// Blank means empty after trimming whitespace (E-03).
pub fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

/// The error icon replaces the clear button, and the supporting text can show (E-31).
pub fn is_error(text: &str, can_be_blank: bool, valid: bool) -> bool {
    (is_blank(text) && !can_be_blank) || !valid
}

/// OK rule from the Kotlin dialog: changed from the snapshot, not blank unless allowed, and valid (E-32, E-33).
pub fn can_confirm(text: &str, snapshot: &str, can_be_blank: bool, valid: bool) -> bool {
    text != snapshot && (!is_blank(text) || can_be_blank) && valid
}

/// Supporting text: the validator's message when the text is invalid. A blank-only error shows none (E-03).
pub fn supporting_text(valid: bool, message: Option<String>) -> Option<String> {
    if valid { None } else { message }
}

/// Edit row with a veto-gated write. The dialog keeps the snapshot taken when it opened (E-35).
pub fn row(
    common: &ItemCommon,
    pref: Pref<String>,
    dialog_subtitle: Option<String>,
    can_be_blank: bool,
    validate: Rc<dyn Fn(&str) -> bool>,
    error_message: Option<Rc<dyn Fn(&str) -> String>>,
    veto: Veto<String>,
) -> adw::ActionRow {
    let stored = Rc::new(RefCell::new(pref.get()));
    let built = row::build(RowSpec {
        title: &common.title,
        subtitle: common.subtitle_for(&stored.borrow()).as_deref(),
        icon: common.icon,
        trailing: None,
        on_click: None,
    });
    built.set_sensitive(common.enabled);
    built.set_activatable(common.enabled);

    let refresh: Rc<dyn Fn(&str)> = {
        let (subtitle, template) = (built.clone(), common.subtitle.clone());
        Rc::new(move |value: &str| {
            let text = template.as_deref().and_then(|t| format_subtitle(t, value));
            set_subtitle(&subtitle, text.as_deref());
        })
    };
    let gate = Rc::new(WriteGate::default());
    let title = common.title.clone();
    crate::widgets::row_activate::on_activate(&built, {
        let (stored, refresh, pref, veto, gate) = (stored.clone(), refresh.clone(), pref.clone(), veto.clone(), gate.clone());
        move |row| {
            let Some(parent) = row.root().and_downcast::<gtk::Window>() else {
                return;
            };
            let snapshot = stored.borrow().clone();
            let spec = EditSpec {
                title: title.clone(),
                dialog_subtitle: dialog_subtitle.clone(),
                snapshot,
                can_be_blank,
                validate: validate.clone(),
                error_message: error_message.clone(),
            };
            let (stored, refresh, pref, veto, gate) = (stored.clone(), refresh.clone(), pref.clone(), veto.clone(), gate.clone());
            open(&parent, spec, move |text: String, dialog: Dialog| {
                let (stored, refresh) = (stored.clone(), refresh.clone());
                let written = text.clone();
                bind::write(gate.clone(), veto.clone(), pref.clone(), text, move |outcome| match outcome {
                    Outcome::Written => {
                        *stored.borrow_mut() = written.clone();
                        refresh(&written);
                        dialog.finish();
                    }
                    Outcome::Kept => {}
                    Outcome::Failed(message) => dialog.set_error(Some(&message)),
                });
            });
        }
    });

    let weak = built.downgrade();
    bind::follow(&pref, move || weak.upgrade().is_some(), move |value: String| {
        *stored.borrow_mut() = value.clone();
        refresh(&value);
    });
    built
}

/// Everything the edit dialog needs, captured when it opens.
struct EditSpec {
    title: String,
    dialog_subtitle: Option<String>,
    snapshot: String,
    can_be_blank: bool,
    validate: Rc<dyn Fn(&str) -> bool>,
    error_message: Option<Rc<dyn Fn(&str) -> String>>,
}

fn open(parent: &gtk::Window, spec: EditSpec, on_ok: impl Fn(String, Dialog) + 'static) {
    let body = GBox::new(Orientation::Vertical, SPACE_S);
    if let Some(sub) = &spec.dialog_subtitle {
        let label = Label::new(Some(sub));
        label.set_wrap(true);
        label.set_xalign(0.0);
        body.append(&label);
    }
    let entry = Entry::new();
    entry.set_text(&spec.snapshot);
    body.append(&entry);
    let support = Label::new(None);
    support.add_css_class("pref-error");
    support.set_wrap(true);
    support.set_xalign(0.0);
    support.set_visible(false);
    body.append(&support);

    let slot: Rc<RefCell<Option<Dialog>>> = Rc::new(RefCell::new(None));
    let spec = Rc::new(spec);
    let on_ok = Rc::new(on_ok);

    let refresh = {
        let (entry, support, slot, spec) = (entry.clone(), support.clone(), slot.clone(), spec.clone());
        Rc::new(move || {
            let text = entry.text().to_string();
            let valid = (spec.validate)(&text);
            let error = is_error(&text, spec.can_be_blank, valid);
            entry.set_secondary_icon_name(Some(if error { "dialog-error-symbolic" } else { "edit-clear-symbolic" }));
            let message = supporting_text(valid, spec.error_message.as_ref().map(|m| m(&text)));
            support.set_text(message.as_deref().unwrap_or(""));
            support.set_visible(message.is_some());
            if let Some(dialog) = slot.borrow().as_ref() {
                dialog.set_confirm_enabled(can_confirm(&text, &spec.snapshot, spec.can_be_blank, valid));
            }
        })
    };
    entry.connect_changed({
        let refresh = refresh.clone();
        move |_| refresh()
    });
    entry.connect_icon_press({
        let (spec, entry) = (spec.clone(), entry.clone());
        move |_, position| {
            if position != EntryIconPosition::Secondary {
                return;
            }
            let text = entry.text().to_string();
            let valid = (spec.validate)(&text);
            // The clear button only exists while the text is valid (E-31).
            if !is_error(&text, spec.can_be_blank, valid) {
                entry.set_text("");
            }
        }
    });
    entry.connect_activate({
        let (slot, on_ok, entry) = (slot.clone(), on_ok.clone(), entry.clone());
        move |_| {
            let Some(dialog) = slot.borrow().clone() else { return };
            if dialog.confirm.as_ref().is_some_and(|c| c.is_sensitive()) {
                on_ok(entry.text().to_string(), dialog);
            }
        }
    });

    let confirm = {
        let (entry, on_ok) = (entry.clone(), on_ok.clone());
        move |dialog: Dialog| on_ok(entry.text().to_string(), dialog)
    };
    let dialog = dialog::show(parent, &spec.title, &body, Some(strings::ok().as_str()), confirm, || {});
    *slot.borrow_mut() = Some(dialog);
    refresh();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_is_whitespace_only() {
        assert!(is_blank(""));
        assert!(is_blank("   "));
        assert!(!is_blank(" a "));
    }

    #[test]
    fn invalid_or_blank_not_allowed_shows_the_error_state() {
        assert!(is_error("", false, true));
        assert!(!is_error("", true, true));
        assert!(is_error("x", true, false));
    }

    #[test]
    fn ok_needs_a_change_a_value_and_validation() {
        assert!(!can_confirm("same", "same", true, true));
        assert!(!can_confirm("   ", "old", false, true));
        assert!(can_confirm("", "old", true, true));
        assert!(!can_confirm("new", "old", true, false));
        assert!(can_confirm("new", "old", false, true));
    }

    #[test]
    fn blank_only_error_has_no_supporting_text() {
        assert_eq!(supporting_text(true, Some("bad".into())), None);
        assert_eq!(supporting_text(false, Some("bad".into())).as_deref(), Some("bad"));
        assert_eq!(supporting_text(false, None), None);
    }
}
