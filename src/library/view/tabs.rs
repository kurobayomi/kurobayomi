//! The category tab strip (T-37a-016): one toggle button per visible page in the shared top row of a page
//! (`ui::pill_row`), in a horizontal scroller for when there are more categories than fit.

use std::rc::Rc;

use gtk::prelude::*;
use kuro_i18n::{MsgKey, tr};

use crate::widgets::context_menu::{self, Item};

#[derive(Clone)]
pub struct TabStrip {
    pub scroller: gtk::ScrolledWindow,
    row: gtk::Box,
}

impl TabStrip {
    pub fn new() -> Self {
        // the shared top row of a page (`ui::pill_row`), as the download queue's Resume and Clear; the tabs start where
        // the cards start
        let row = crate::ui::pill_row();
        row.set_margin_start(crate::library::layout::CARD_EDGE);
        row.set_margin_end(12);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .child(&row)
            .build();
        Self { scroller, row }
    }

    /// Rebuilds the strip with `labels`, `active` pressed. `on_select` gets the index of a tab the user picks, and
    /// `on_update` the index of a tab whose right-click menu's "Update category" was chosen; the menu is there only with
    /// more than one category (with one, Refresh updates the same titles). The buttons are created before their handlers
    /// are connected, so setting the active one does not fire it.
    pub fn set(&self, labels: &[String], active: usize, on_select: Rc<dyn Fn(usize)>, on_update: Rc<dyn Fn(usize)>) {
        while let Some(child) = self.row.first_child() {
            self.row.remove(&child);
        }
        for (index, button) in crate::ui::choice_pills(&self.row, labels, active).into_iter().enumerate() {
            let on_select = on_select.clone();
            button.connect_toggled(move |b| {
                if b.is_active() {
                    on_select(index);
                }
            });
            if labels.len() > 1 {
                let on_update = on_update.clone();
                context_menu::attach(&button, move |_, _| {
                    let on_update = on_update.clone();
                    vec![Item::new(tr(MsgKey::action_update_category), 0, move || on_update(index))]
                });
            }
        }
        self.scroller.set_visible(!labels.is_empty());
    }
}
