//! The lists the player's menu offers: quality and server (`quality`), speed (`speed`), chapters (`chapters`); the
//! audio and subtitle tracks are in `menu.rs`. Each list is a column of [`Choice`]s with the current one marked; the
//! player's one menu (the header's ⋮ and the video's right-click, `widgets::context_menu`) shows each as a submenu
//! through [`items`]. Main thread only.

pub(super) mod chapters;
pub(super) mod quality;
pub(super) mod speed;

use crate::widgets::context_menu::Item;

/// One entry of a list.
pub struct Choice {
    pub title: String,
    pub subtitle: Option<String>,
    /// The entry that carries the check mark.
    pub current: bool,
    /// An entry that is not enabled is shown greyed out and does nothing (a server that failed to load).
    pub enabled: bool,
    /// Runs when the entry is chosen.
    pub pick: Box<dyn Fn()>,
}

/// The entries of a submenu for `choices`: the current one checked, a subtitle after the title ("Opening · 1:30").
pub fn items(choices: Vec<Choice>) -> Vec<Item> {
    choices
        .into_iter()
        .map(|choice| {
            let label = match &choice.subtitle {
                Some(subtitle) => format!("{} · {subtitle}", choice.title),
                None => choice.title,
            };
            let item = Item::new(label, 0, choice.pick).checked(choice.current);
            if choice.enabled { item } else { item.disabled() }
        })
        .collect()
}
