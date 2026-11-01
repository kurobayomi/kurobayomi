//! GTK's own selection state for a grid or a list whose selection the page owns (a click, a long press or a drag selects
//! through the page's logic, not through GTK). The page tells the mirror which positions it has selected; the model then
//! marks those items `:selected`, and libadwaita draws its selected fill on them. A change GTK makes on its own, such as a
//! plain click selecting the clicked item, is put back to what the page selected.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;

pub struct SelectionMirror {
    model: gtk::MultiSelection,
    wanted: RefCell<Vec<u32>>,
    applying: Cell<bool>,
}

impl SelectionMirror {
    /// The mirror of `model`, which starts with nothing selected.
    pub fn new(model: &gtk::MultiSelection) -> Rc<Self> {
        let mirror = Rc::new(Self { model: model.clone(), wanted: RefCell::new(Vec::new()), applying: Cell::new(false) });
        let weak = Rc::downgrade(&mirror);
        model.connect_selection_changed(move |_, _, _| {
            if let Some(mirror) = weak.upgrade().filter(|mirror| !mirror.applying.get()) {
                mirror.put_back();
            }
        });
        mirror
    }

    /// Selects exactly the items at `positions` of the model, and remembers them.
    pub fn apply(&self, positions: Vec<u32>) {
        *self.wanted.borrow_mut() = positions;
        self.put_back();
    }

    fn put_back(&self) {
        let chosen = gtk::Bitset::new_empty();
        for position in self.wanted.borrow().iter() {
            chosen.add(*position);
        }
        self.applying.set(true);
        self.model.set_selection(&chosen, &gtk::Bitset::new_range(0, self.model.n_items()));
        self.applying.set(false);
    }
}

/// The same for a `gtk::ListBox` whose rows the page has just built: `selected` are the rows it has selected. The box goes
/// to multiple selection so libadwaita draws `row:selected` on them, and a change the box makes on its own (a click) is
/// put back to `selected`.
pub fn mirror_list_box(list: &gtk::ListBox, selected: Vec<gtk::ListBoxRow>) {
    list.set_selection_mode(gtk::SelectionMode::Multiple);
    let apply = move |list: &gtk::ListBox| {
        list.unselect_all();
        for row in &selected {
            list.select_row(Some(row));
        }
    };
    apply(list);
    // putting the selection back changes it again: that change is ignored
    let busy = Cell::new(false);
    list.connect_selected_rows_changed(move |list| {
        if !busy.replace(true) {
            apply(list);
            busy.set(false);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_wanted_positions_are_selected_and_a_change_of_gtk_is_put_back() {
        crate::testing::with_display(|| {
            let store = gtk::gio::ListStore::new::<gtk::StringObject>();
            for name in ["a", "b", "c", "d"] {
                store.append(&gtk::StringObject::new(name));
            }
            let model = gtk::MultiSelection::new(Some(store));
            let mirror = SelectionMirror::new(&model);
            mirror.apply(vec![1, 3]);
            let selected = || (0..4).filter(|i| model.is_selected(*i)).collect::<Vec<u32>>();
            assert_eq!(selected(), [1, 3]);
            model.select_item(0, true);
            assert_eq!(selected(), [1, 3], "a click of GTK's own does not change what the page selected");
            mirror.apply(Vec::new());
            assert!(selected().is_empty());
        });
    }
}
