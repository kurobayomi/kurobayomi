//! Single-choice list (Kotlin `ListPreferenceWidget`) and its basic variant. The row opens a radio
//! dialog. Choosing a different entry runs the veto and closes; choosing the current entry does nothing.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Box as GBox, CheckButton, Orientation, PolicyType, ScrolledWindow};
use kuro_prefs::pref::Pref;

use super::super::bind::{self, Outcome, Veto, WriteGate};
use super::super::model::{Entry, ItemCommon};
use super::dialog::{self, Dialog};
use crate::widgets::row::{self, set_subtitle, RowSpec};
use super::CHECK_ROW_MIN;

/// Called with the chosen key and the dialog, and a revert that re-selects the current key.
type Revert = Rc<dyn Fn()>;
type OnWritten = Rc<dyn Fn(&str)>;
pub type Commit = Rc<dyn Fn(String, Dialog, Revert, OnWritten)>;

/// Visible label of `key`, or the raw key when it is not in `entries` (E-23).
pub fn label_for(entries: &[Entry], key: &str) -> String {
    entries
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, label)| label.clone())
        .unwrap_or_else(|| key.to_owned())
}

/// A bound single-choice row. The store is written only after the veto accepts.
pub fn row(common: &ItemCommon, pref: Pref<String>, entries: Vec<Entry>, veto: Veto<String>) -> adw::ActionRow {
    let gate = Rc::new(WriteGate::default());
    let commit: Commit = {
        let pref = pref.clone();
        Rc::new(move |key: String, dialog: Dialog, revert: Revert, on_written: OnWritten| {
            let (gate, veto, pref) = (gate.clone(), veto.clone(), pref.clone());
            let written_key = key.clone();
            bind::write(gate, veto, pref, key, move |outcome| match outcome {
                Outcome::Written => {
                    on_written(&written_key);
                    dialog.finish();
                }
                Outcome::Kept => revert(),
                Outcome::Failed(message) => {
                    dialog.set_error(Some(&message));
                    revert();
                }
            });
        })
    };
    let (built, current, entries) = build(common, pref.get(), entries, commit);
    let weak = built.downgrade();
    let refresh = refresher(&built, common, &current, entries);
    bind::follow(&pref, move || weak.upgrade().is_some(), move |value: String| refresh(&value));
    built
}

/// A single-choice row whose value the caller owns. Choosing an entry only calls `on_change`.
pub fn basic_row(common: &ItemCommon, value: String, entries: Vec<Entry>, on_change: Veto<String>) -> adw::ActionRow {
    let gate = Rc::new(WriteGate::default());
    let commit: Commit = Rc::new(move |key: String, dialog: Dialog, revert: Revert, on_written: OnWritten| {
        let gate = gate.clone();
        let ticket = gate.begin();
        let veto = on_change.clone();
        gtk::glib::spawn_future_local(async move {
            let decision = bind::decide(veto(key.clone()).await, gate.is_latest(ticket));
            match decision {
                Ok(true) => {
                    on_written(&key);
                    dialog.finish();
                }
                Ok(false) => revert(),
                Err(message) => {
                    dialog.set_error(Some(&message));
                    revert();
                }
            }
        });
    });
    build(common, value, entries, commit).0
}

/// Builds the row and its dialog wiring. Returns the row, the shared current key and the entries.
fn build(common: &ItemCommon, initial: String, entries: Vec<Entry>, commit: Commit) -> (adw::ActionRow, Rc<RefCell<String>>, Rc<Vec<Entry>>) {
    let current = Rc::new(RefCell::new(initial.clone()));
    let subtitle_text = common.subtitle_for(&label_for(&entries, &initial));
    let built = row::build(RowSpec {
        title: &common.title,
        subtitle: subtitle_text.as_deref(),
        icon: common.icon,
        trailing: None,
        on_click: None,
    });
    built.set_activatable(common.enabled);
    built.set_sensitive(common.enabled);

    let entries = Rc::new(entries);
    let title = common.title.clone();
    let subtitle_template = common.subtitle.clone();
    let on_written: OnWritten = {
        let (current, subtitle, entries) = (current.clone(), built.clone(), entries.clone());
        Rc::new(move |key: &str| {
            *current.borrow_mut() = key.to_owned();
            let text = subtitle_template
                .as_deref()
                .and_then(|t| super::super::model::format_subtitle(t, &label_for(&entries, key)));
            set_subtitle(&subtitle, text.as_deref());
        })
    };
    crate::widgets::row_activate::on_activate(&built, {
        let (current, entries, commit, on_written) = (current.clone(), entries.clone(), commit.clone(), on_written.clone());
        move |row| {
            let Some(parent) = row.root().and_downcast::<gtk::Window>() else {
                return;
            };
            choose(&parent, &title, current.borrow().clone(), &entries, commit.clone(), on_written.clone());
        }
    });
    (built, current, entries)
}

/// Re-renders the subtitle from a new stored key (watch callback, no write).
fn refresher(
    built: &adw::ActionRow,
    common: &ItemCommon,
    current: &Rc<RefCell<String>>,
    entries: Rc<Vec<Entry>>,
) -> impl Fn(&str) + 'static {
    let (subtitle, template, current) = (built.clone(), common.subtitle.clone(), current.clone());
    move |key: &str| {
        *current.borrow_mut() = key.to_owned();
        let text = template
            .as_deref()
            .and_then(|t| super::super::model::format_subtitle(t, &label_for(&entries, key)));
        set_subtitle(&subtitle, text.as_deref());
    }
}

/// The list of a dialog is at most 60% of the window's height (ui-parity 3.1). Before the window has a size the old
/// fixed 420 px applies.
pub fn max_list_height(window_height: i32) -> i32 {
    if window_height <= 0 { 420 } else { window_height * 6 / 10 }
}

/// Which dividers a scrolling list shows: one above when content is hidden above, one below when content is hidden
/// below (`value` is the scroll offset, `page` the visible height, `upper` the content height).
pub fn scroll_dividers(value: f64, page: f64, upper: f64) -> (bool, bool) {
    (value > 0.5, value + page < upper - 0.5)
}

/// `scroll` between two separators that show only while the list can scroll that way.
fn with_scroll_dividers(scroll: &ScrolledWindow) -> GBox {
    let (top, bottom) = (gtk::Separator::new(Orientation::Horizontal), gtk::Separator::new(Orientation::Horizontal));
    let update = {
        let (top, bottom, scroll) = (top.clone(), bottom.clone(), scroll.clone());
        move || {
            let adjustment = scroll.vadjustment();
            let (above, below) = scroll_dividers(adjustment.value(), adjustment.page_size(), adjustment.upper());
            top.set_opacity(if above { 1.0 } else { 0.0 });
            bottom.set_opacity(if below { 1.0 } else { 0.0 });
        }
    };
    update();
    let adjustment = scroll.vadjustment();
    adjustment.connect_value_changed({
        let update = update.clone();
        move |_| update()
    });
    adjustment.connect_changed(move |_| update());
    let column = GBox::new(Orientation::Vertical, 0);
    column.append(&top);
    column.append(scroll);
    column.append(&bottom);
    column
}

/// Opens the radio dialog. Cancel only; an empty list shows no choices.
fn choose(parent: &gtk::Window, title: &str, current: String, entries: &[Entry], commit: Commit, on_written: OnWritten) {
    let body = GBox::new(Orientation::Vertical, 0);
    let radios: Rc<RefCell<Vec<(String, CheckButton)>>> = Rc::new(RefCell::new(Vec::new()));
    let slot: Rc<RefCell<Option<Dialog>>> = Rc::new(RefCell::new(None));
    let mut group: Option<CheckButton> = None;
    for (key, label) in entries {
        let radio = CheckButton::with_label(label);
        radio.set_size_request(-1, CHECK_ROW_MIN);
        match &group {
            Some(first) => radio.set_group(Some(first)),
            None => group = Some(radio.clone()),
        }
        radio.set_active(*key == current);
        body.append(&radio);
        radios.borrow_mut().push((key.clone(), radio));
    }
    for (key, radio) in radios.borrow().iter() {
        let (key, current, slot, commit, on_written, radios) =
            (key.clone(), current.clone(), slot.clone(), commit.clone(), on_written.clone(), radios.clone());
        radio.connect_toggled(move |button| {
            if !button.is_active() || key == current {
                return;
            }
            let Some(dialog) = slot.borrow().clone() else {
                return;
            };
            let revert: Revert = {
                let (radios, current) = (radios.clone(), current.clone());
                Rc::new(move || {
                    if let Some((_, original)) = radios.borrow().iter().find(|(k, _)| *k == current) {
                        original.set_active(true);
                    }
                })
            };
            commit(key.clone(), dialog, revert, on_written.clone());
        });
    }

    let scroll = ScrolledWindow::new();
    scroll.set_hscrollbar_policy(PolicyType::Never);
    scroll.set_propagate_natural_height(true);
    scroll.set_max_content_height(max_list_height(parent.height()));
    scroll.set_child(Some(&body));
    let framed = with_scroll_dividers(&scroll);
    let dialog = dialog::show(parent, title, &framed, None, |_| {}, || {});
    *slot.borrow_mut() = Some(dialog);
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-47ba-046: at most 60% of the window, and a divider only on the side with hidden content.
    #[test]
    fn the_list_is_capped_at_sixty_percent_of_the_window() {
        assert_eq!(max_list_height(1000), 600);
        assert_eq!(max_list_height(780), 468);
        assert_eq!(max_list_height(0), 420, "before the window has a size");
    }

    #[test]
    fn dividers_show_only_on_the_side_that_can_scroll() {
        assert_eq!(scroll_dividers(0.0, 300.0, 300.0), (false, false), "a list that fits has none");
        assert_eq!(scroll_dividers(0.0, 300.0, 800.0), (false, true));
        assert_eq!(scroll_dividers(200.0, 300.0, 800.0), (true, true));
        assert_eq!(scroll_dividers(500.0, 300.0, 800.0), (true, false));
    }
}
