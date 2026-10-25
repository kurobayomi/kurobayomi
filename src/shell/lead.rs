//! The open tab's search toggle in the sidebar's header (user, 2026-10-14). Every tab page leads its own header with it
//! (`ui::pack_lead`), which is where it stays in a narrow window. While the sidebar shows a tab page, it moves into the
//! sidebar's header before the app name. Switching tabs, pushing a page or narrowing the window puts it back first. A
//! tab page has no ⋮ menu: its actions are buttons at the end of its own header (user, 2026-10-25).

use std::cell::RefCell;

use gtk::prelude::*;
use gtk::Box as GBox;

thread_local! {
    /// The place in the sidebar's header before the app name.
    static HOST: GBox = GBox::new(gtk::Orientation::Horizontal, 0);
    /// The slot now in the sidebar's header, with the lead box it came from.
    static AWAY: RefCell<Option<(gtk::Widget, GBox)>> = const { RefCell::new(None) };
}

/// The sidebar header's place for the search (before the app name).
pub fn host() -> GBox {
    HOST.with(GBox::clone)
}

/// The lead box of a tab page: in the header that starts the page.
fn lead_of(page: &gtk::Widget) -> Option<GBox> {
    fn find(widget: &gtk::Widget) -> Option<GBox> {
        if widget.has_css_class(crate::ui::LEAD) {
            return widget.clone().downcast().ok();
        }
        let mut child = widget.first_child();
        while let Some(c) = child {
            if let Some(found) = find(&c) {
                return Some(found);
            }
            child = c.next_sibling();
        }
        None
    }
    let header = page.first_child().filter(|w| w.is::<gtk::HeaderBar>())?;
    find(&header)
}

/// Puts the search slot of `page` (the visible tab page, when one is on screen) in the sidebar's header when `wide`,
/// and one there before back into its own page.
pub fn place(page: Option<&gtk::Widget>, wide: bool) {
    if let Some((slot, home)) = AWAY.with(|away| away.borrow_mut().take()) {
        if let Some(parent) = slot.parent().and_downcast::<GBox>() {
            parent.remove(&slot);
        }
        home.prepend(&slot);
        // the page being left still fades out on screen: it must not flash its slots back in its own header
        home.set_visible(!wide);
    }
    let Some(lead) = page.and_then(lead_of) else { return };
    // an empty slot box still takes its spacing in the header, so while the sidebar holds the slot every page's box is
    // hidden, the page on screen and the one fading out alike: the Anime | Manga switch then keeps its place across a
    // change of tab
    lead.set_visible(!wide);
    if !wide {
        return;
    }
    let mut child = lead.first_child();
    while let Some(slot) = child {
        child = slot.next_sibling();
        if slot.has_css_class(crate::ui::LEAD_SEARCH) {
            lead.remove(&slot);
            host().append(&slot);
            AWAY.with(|away| *away.borrow_mut() = Some((slot, lead.clone())));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_goes_to_the_sidebar_when_wide_and_comes_back() {
        crate::testing::with_display(|| {
            let page = GBox::new(gtk::Orientation::Vertical, 0);
            let header = gtk::HeaderBar::new();
            page.append(&header);
            let search = gtk::ToggleButton::new();
            crate::ui::pack_lead(&header, Some(search.upcast_ref()), None);
            let lead = lead_of(page.upcast_ref()).expect("the lead box");
            place(Some(page.upcast_ref()), true);
            assert!(search.is_ancestor(&host()), "in the sidebar's header");
            assert!(lead.first_child().is_none());
            place(Some(page.upcast_ref()), false);
            assert!(search.is_ancestor(&lead), "back home");
            place(Some(page.upcast_ref()), true);
            let other = GBox::new(gtk::Orientation::Vertical, 0);
            place(Some(other.upcast_ref()), true);
            assert!(search.is_ancestor(&lead) && !lead.is_visible(), "a page left while wide keeps its slots hidden");
            place(Some(page.upcast_ref()), true);
            assert!(!lead.is_visible(), "the page on screen has its empty box hidden too, so no page leaves a gap");
            place(Some(page.upcast_ref()), false);
            assert!(lead.is_visible(), "shown again once it is the page on screen");
            place(None, true);
            assert!(search.is_ancestor(&lead), "no page on screen: nothing moves");
        });
    }

    /// A tab page has no ⋮ menu (user, 2026-10-25): its actions are buttons in its header. Only pages pushed over the
    /// tabs fill `HeaderActions::menu`: a title's page and the Upcoming page's guide.
    #[test]
    fn no_tab_page_has_a_more_menu() {
        const PUSHED: [&str; 2] = ["src/entry_core/details/route.rs", "src/updates/upcoming/page.rs"];
        fn files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("the source folder").flatten() {
                let path = entry.path();
                if path.is_dir() { files(&path, out) } else if path.extension().is_some_and(|e| e == "rs") { out.push(path) }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut all = Vec::new();
        files(&root.join("src"), &mut all);
        let mut filled = Vec::new();
        for path in all {
            let name = path.strip_prefix(root).expect("under the crate").to_string_lossy().replace('\\', "/");
            // the pushed pages, the struct itself and this test
            if PUSHED.contains(&name.as_str()) || name == "src/ui.rs" || name == "src/shell/lead.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a source file");
            for (at, _) in text.match_indices("HeaderActions {") {
                // the struct literal, up to its closing brace
                let mut depth = 0;
                let body: String = text[at..]
                    .chars()
                    .take_while(|c| {
                        match c {
                            '{' => depth += 1,
                            '}' => depth -= 1,
                            _ => {}
                        }
                        !(depth == 0 && *c == '}')
                    })
                    .collect();
                if body.split(|c: char| !c.is_alphanumeric() && c != '_').any(|word| word == "menu") {
                    filled.push(name.clone());
                }
            }
            if text.contains(".menu.push") {
                filled.push(name);
            }
        }
        assert!(filled.is_empty(), "a tab page fills HeaderActions::menu: {filled:?}");
    }
}
