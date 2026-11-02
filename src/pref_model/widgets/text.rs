//! Text row (Kotlin `TextPreferenceWidget`): static title and optional subtitle, activatable only with a handler.

use std::rc::Rc;

use super::super::model::ItemCommon;
use crate::widgets::row::{self, RowSpec};

pub fn build(common: &ItemCommon, on_click: Option<Rc<dyn Fn()>>, trailing: Option<&gtk::Widget>) -> adw::ActionRow {
    row::build(RowSpec {
        title: &common.title,
        subtitle: common.subtitle.as_deref(),
        icon: common.icon,
        trailing,
        on_click,
    })
}
