//! Sidebar geometry (T-49b-017): the numbers `shell::sidebar` applies. The sidebar always shows icon, label and
//! badge, with the list at the top; there is no collapsed or icon-only state (docs/design/main-window.md).

/// Sidebar width in px.
pub const WIDTH: i32 = 200;
/// Space above and below the item list in px.
pub const PAD_V: i32 = 8;
/// Gap between items in px.
pub const ITEM_SPACING: i32 = 4;
/// Padding inside one row in px.
pub const ROW_PAD: i32 = 8;

/// One row of a `navigation-sidebar` list (the app's panel and the settings pages' list): `content` padded by
/// [`ROW_PAD`], the row spaced from its neighbours by [`ITEM_SPACING`]. Every such list builds its rows here, so two
/// side lists have the same row pitch.
pub fn row(content: &impl gtk::prelude::IsA<gtk::Widget>) -> gtk::ListBoxRow {
    use gtk::prelude::*;
    crate::ui::margins(content, ROW_PAD);
    let row = gtk::ListBoxRow::builder().child(content).build();
    row.set_margin_top(ITEM_SPACING / 2);
    row.set_margin_bottom(ITEM_SPACING / 2);
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::prelude::*;

    #[test]
    fn geometry_constants() {
        assert_eq!((WIDTH, PAD_V, ITEM_SPACING, ROW_PAD), (200, 8, 4, 8));
    }

    fn descendants(root: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
        let mut child = root.first_child();
        while let Some(w) = child {
            descendants(&w, out);
            child = w.next_sibling();
            out.push(w);
        }
    }

    #[test]
    fn the_sidebar_rows_always_show_their_labels_at_the_top() {
        crate::testing::with_display(|| {
            let sidebar = crate::shell::sidebar::build(&gtk::Stack::new());
            let mut all = vec![];
            descendants(sidebar.upcast_ref(), &mut all);
            let titles: Vec<String> = crate::ui::with_spec(|s| s.nav.iter().map(|n| n.title.clone()).collect());
            assert!(!titles.is_empty());
            for title in titles {
                let label = all.iter().filter_map(|w| w.downcast_ref::<gtk::Label>()).find(|l| l.label() == title.as_str()).unwrap_or_else(|| panic!("a label for {title}"));
                assert!(label.is_visible(), "{title} is visible");
            }
            let list = all.iter().find_map(|w| w.downcast_ref::<gtk::ListBox>().cloned()).expect("the nav list");
            assert_eq!(list.valign(), gtk::Align::Start, "the nav list sits at the top");
            // the app menu's button is the only toggle (inside the menu button)
            assert!(all.iter().all(|w| !w.is::<gtk::ToggleButton>() || w.parent().is_some_and(|p| p.is::<gtk::MenuButton>())), "no collapse toggle");
            assert_eq!(sidebar.width_request(), WIDTH);
        });
    }
}
