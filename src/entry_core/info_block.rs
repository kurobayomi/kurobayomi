//! The info block of an entry page, one widget for the manga and the anime page: the header (cover, title, author,
//! status, source), the action row, and the description with its genre chips, in that order. The anime page adds its
//! related rows under it. The route places the block: in the left pane of the two-pane layout, above the list otherwise.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Box as GBox, Orientation as O};

use crate::entry_core::action_row_view::ActionRow;
use crate::entry_core::description_view;
use crate::entry_core::header::{GenreChip, HeaderModel};
use crate::entry_core::header_view::{self, Handlers};
use crate::entry_core::text::{Description, Expansion};
use crate::theme::tokens::{SPACE_M};

pub struct InfoBlock {
    pub root: GBox,
    header: GBox,
    pub action_row: ActionRow,
    description: GBox,
    /// The header on screen: its model, the layout and the column it was built for.
    shown_header: RefCell<Option<(HeaderModel, bool, (i32, bool))>>,
    /// The description on screen with its genre chips.
    shown_description: RefCell<Option<(Description, Vec<GenreChip>)>>,
}

impl InfoBlock {
    pub fn new(action_row: ActionRow) -> Self {
        let header = GBox::new(O::Vertical, 0);
        let description = GBox::new(O::Vertical, 0);
        let root = GBox::new(O::Vertical, SPACE_M);
        root.append(&header);
        root.append(&action_row.root);
        root.append(&description);
        Self { root, header, action_row, description, shown_header: RefCell::default(), shown_description: RefCell::default() }
    }

    /// Draws the header for `model` in the `large` layout, `column` px wide. It is built again only when the model
    /// changes (`HeaderModel::looks_like`, which ignores the library flag), or the layout or the column does: the cover
    /// is sized for its column.
    pub fn set_header(&self, model: &HeaderModel, large: bool, column: i32, handlers: &Handlers) {
        // the cover size and the title style, not the width, decide a rebuild: a resize rebuilds the header only when one of
        // them changes
        let cover = crate::entry_core::header::cover_size(large, column - 32).0;
        let at = (cover, !large && column < header_view::NARROW_TITLE_BELOW);
        let same = self.shown_header.borrow().as_ref().is_some_and(|(shown, was_large, shown_at)| shown.looks_like(model) && *was_large == large && *shown_at == at);
        if same {
            return;
        }
        clear(&self.header);
        self.header.append(&header_view::build(model, large, column, handlers));
        *self.shown_header.borrow_mut() = Some((model.clone(), large, at));
    }

    /// Draws the description and its genre chips. They are built again only when either changes.
    pub fn set_description(&self, description: &Description, genres: &[GenreChip], expansion: &Rc<Cell<Expansion>>, handlers: &Handlers) {
        let key = (description.clone(), genres.to_vec());
        if self.shown_description.borrow().as_ref() == Some(&key) {
            return;
        }
        clear(&self.description);
        self.description.append(&description_view::build(&key.0, Rc::clone(expansion), Some((&key.1, handlers))));
        *self.shown_description.borrow_mut() = Some(key);
    }
}

fn clear(slot: &GBox) {
    while let Some(child) = slot.first_child() {
        slot.remove(&child);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::entry_core::header::SearchRequest;
    use crate::entry_core::text::description;

    /// One display test: the header is built again only when its model, its layout or its cover size changes, and the
    /// description only when its text or its genre chips change.
    #[test]
    fn the_header_and_the_description_are_built_again_only_when_they_change() {
        crate::testing::with_display(|| {
            let handlers = Handlers {
                on_search: Rc::new(|_: SearchRequest| {}),
                on_copy: Rc::new(|_: String| {}),
            };
            let info = InfoBlock::new(ActionRow::new(Rc::new(|| {}), None, None));
            let expansion = Rc::new(Cell::new(Expansion::new(false, false)));
            let model = HeaderModel::from_parts("Title", Some("Author"), None, ("document-open-recent-symbolic", "Ongoing".into()), "Source", false);

            info.set_header(&model, false, 400, &handlers);
            let header = info.header.first_child();
            info.set_header(&model, false, 400, &handlers);
            assert_eq!(info.header.first_child(), header, "the same header keeps its widgets");
            info.set_header(&model, false, 600, &handlers);
            assert_ne!(info.header.first_child(), header, "a column wide enough for another cover size draws the header again");
            let large = info.header.first_child();
            info.set_header(&model, true, 300, &handlers);
            assert_ne!(info.header.first_child(), large, "a new layout draws the header again");

            let text = description(Some("A story."));
            info.set_description(&text, &[], &expansion, &handlers);
            let shown = info.description.first_child();
            info.set_description(&text, &[], &expansion, &handlers);
            assert_eq!(info.description.first_child(), shown, "the same description keeps its widgets");
            info.set_description(&description(Some("Another story.")), &[], &expansion, &handlers);
            assert_ne!(info.description.first_child(), shown, "new text draws the description again");
        });
    }
}
