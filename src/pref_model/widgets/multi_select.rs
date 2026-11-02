//! Multi-select (Kotlin `MultiSelectListPreferenceWidget`): checkboxes in entries order, OK writes the
//! selection, Cancel writes nothing. Unknown stored keys are dropped on OK (T-E29, see DECISIONS).

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Box as GBox, CheckButton, Orientation, PolicyType, ScrolledWindow};
use kuro_prefs::pref::Pref;

use super::super::bind::{self, Outcome, Veto, WriteGate};
use super::super::model::{multi_select_labels, Entry, ItemCommon};
use super::dialog::{self, Dialog};
use crate::widgets::row::{self, set_subtitle, RowSpec};
use super::CHECK_ROW_MIN;

/// The selection restricted to keys that exist in `entries`, in entries order.
pub fn keep_known(entries: &[Entry], selected: &BTreeSet<String>) -> BTreeSet<String> {
    entries
        .iter()
        .filter(|(key, _)| selected.contains(key))
        .map(|(key, _)| key.clone())
        .collect()
}

pub fn row(common: &ItemCommon, pref: Pref<BTreeSet<String>>, entries: Vec<Entry>, veto: Veto<BTreeSet<String>>) -> adw::ActionRow {
    let entries = Rc::new(entries);
    let stored = Rc::new(RefCell::new(pref.get()));
    let built = row::build(RowSpec {
        title: &common.title,
        subtitle: common.subtitle_for(&multi_select_labels(&entries, &stored.borrow())).as_deref(),
        icon: common.icon,
        trailing: None,
        on_click: None,
    });
    built.set_sensitive(common.enabled);
    built.set_activatable(common.enabled);

    let refresh: Rc<dyn Fn(&BTreeSet<String>)> = {
        let (subtitle, template, entries) = (built.clone(), common.subtitle.clone(), entries.clone());
        Rc::new(move |selected: &BTreeSet<String>| {
            let text = template
                .as_deref()
                .and_then(|t| super::super::model::format_subtitle(t, &multi_select_labels(&entries, selected)));
            set_subtitle(&subtitle, text.as_deref());
        })
    };
    let gate = Rc::new(WriteGate::default());
    let title = common.title.clone();
    {
        let (stored, refresh, entries, pref, veto, gate) =
            (stored.clone(), refresh.clone(), entries.clone(), pref.clone(), veto.clone(), gate.clone());
        crate::widgets::row_activate::on_activate(&built, move |row| {
            let Some(parent) = row.root().and_downcast::<gtk::Window>() else {
                return;
            };
            let initial = stored.borrow().clone();
            open(&parent, &title, initial, &entries, {
                let (stored, refresh, pref, veto, gate) =
                    (stored.clone(), refresh.clone(), pref.clone(), veto.clone(), gate.clone());
                Rc::new(move |selection: BTreeSet<String>, dialog: Dialog| {
                    let (stored, refresh) = (stored.clone(), refresh.clone());
                    let kept = selection.clone();
                    bind::write(gate.clone(), veto.clone(), pref.clone(), selection, move |outcome| match outcome {
                        Outcome::Written => {
                            *stored.borrow_mut() = kept.clone();
                            refresh(&kept);
                            dialog.finish();
                        }
                        Outcome::Kept => {}
                        Outcome::Failed(message) => dialog.set_error(Some(&message)),
                    });
                })
            });
        });
    }

    let weak = built.downgrade();
    bind::follow(&pref, move || weak.upgrade().is_some(), move |value: BTreeSet<String>| {
        *stored.borrow_mut() = value.clone();
        refresh(&value);
    });
    built
}

/// Opens the checkbox dialog. `on_ok` receives the chosen set and the dialog.
fn open(
    parent: &gtk::Window,
    title: &str,
    initial: BTreeSet<String>,
    entries: &[Entry],
    on_ok: Rc<dyn Fn(BTreeSet<String>, Dialog)>,
) {
    let body = GBox::new(Orientation::Vertical, 0);
    let selected: Rc<RefCell<BTreeSet<String>>> = Rc::new(RefCell::new(keep_known(entries, &initial)));
    for (key, label) in entries {
        let check = CheckButton::with_label(label);
        check.set_size_request(-1, CHECK_ROW_MIN);
        check.set_active(selected.borrow().contains(key));
        let (key, selected) = (key.clone(), selected.clone());
        check.connect_toggled(move |button| {
            let mut set = selected.borrow_mut();
            if button.is_active() {
                set.insert(key.clone());
            } else {
                set.remove(&key);
            }
        });
        body.append(&check);
    }
    let scroll = ScrolledWindow::new();
    scroll.set_hscrollbar_policy(PolicyType::Never);
    scroll.set_propagate_natural_height(true);
    scroll.set_max_content_height(420);
    scroll.set_child(Some(&body));

    let slot: Rc<RefCell<Option<Dialog>>> = Rc::new(RefCell::new(None));
    let confirm = {
        let (slot, selected) = (slot.clone(), selected.clone());
        move |_: Dialog| {
            let Some(dialog) = slot.borrow().clone() else { return };
            let chosen = selected.borrow().clone();
            on_ok(chosen, dialog);
        }
    };
    let dialog = dialog::show(parent, title, &scroll, Some(super::super::strings::ok().as_str()), confirm, || {});
    *slot.borrow_mut() = Some(dialog);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<Entry> {
        vec![("a".into(), "A".into()), ("b".into(), "B".into())]
    }

    #[test]
    fn ok_drops_unknown_stored_keys() {
        let stored: BTreeSet<String> = ["zz", "a"].iter().map(|s| s.to_string()).collect();
        let kept: Vec<String> = keep_known(&entries(), &stored).into_iter().collect();
        assert_eq!(kept, vec!["a".to_owned()]);
    }

    #[test]
    fn keep_known_returns_entries_order_regardless_of_input_order() {
        let stored: BTreeSet<String> = ["b", "a"].iter().map(|s| s.to_string()).collect();
        assert_eq!(keep_known(&entries(), &stored).into_iter().collect::<Vec<_>>(), vec!["a", "b"]);
    }
}
