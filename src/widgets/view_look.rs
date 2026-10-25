//! The look sections of a view-options menu, shared by the Library and a source's page: View (Grid / List), Grid style
//! (Compact / Comfortable / Cover only) and the Cover size slider. The owner puts its own sections before [`sections`],
//! gives the slider its pref with [`install`], turns a pick into the mode to store with [`picked`], and shows what is
//! stored with [`sync`]. Each owner keeps its own display-mode key; the cover size is one key for every grid.

use kuro_i18n::{MsgKey, tr};
use kuro_prefs::library::LibraryDisplayMode;
use kuro_prefs::pref::Pref;

use super::view_options::{Item, Pick, Section, ViewOptions};

/// The radio group of View (Grid / List) and of Grid style.
const MODE: &str = "mode";
const STYLE: &str = "style";

/// The custom slot of the cover size slider.
const COVER_SIZE: &str = "cover_size";

/// The three look sections, in this order.
pub fn sections() -> Vec<Section> {
    vec![
        Section::new(tr(MsgKey::ui_view), vec![Item::radio(MODE, "grid", tr(MsgKey::ui_grid)), Item::radio(MODE, "list", tr(MsgKey::action_display_list))]),
        Section::new(tr(MsgKey::ui_grid_style), grid_styles().into_iter().map(|(token, label, _)| Item::radio(STYLE, token, label)).collect()),
        Section::new(tr(MsgKey::pref_cover_size), vec![Item::custom(COVER_SIZE)]),
    ]
}

/// Puts the cover size slider in the menu, writing `pref`.
pub fn install(menu: &ViewOptions, pref: &Pref<i32>) {
    let pref = pref.clone();
    menu.set_custom(COVER_SIZE, move || cover_size_slider(&pref));
}

/// The display mode a look pick asks for, from the `current` one; `None` for any other pick.
pub fn picked(pick: &Pick, current: LibraryDisplayMode) -> Option<LibraryDisplayMode> {
    match pick {
        Pick::Radio(MODE, target) => Some(mode(target == "list", current)),
        Pick::Radio(STYLE, token) => style_mode(token),
        _ => None,
    }
}

/// Marks the View and Grid style radios of `mode`.
pub fn sync(menu: &ViewOptions, mode: LibraryDisplayMode) {
    menu.set_radio(MODE, mode_target(mode));
    menu.set_radio(STYLE, style_target(mode));
}

/// The display mode the Grid / List radios write. Grid keeps a stored grid style and turns List into the default grid.
fn mode(list: bool, current: LibraryDisplayMode) -> LibraryDisplayMode {
    match (list, current) {
        (true, _) => LibraryDisplayMode::List,
        (false, LibraryDisplayMode::List) => LibraryDisplayMode::DEFAULT,
        (false, grid) => grid,
    }
}

fn mode_target(mode: LibraryDisplayMode) -> &'static str {
    if mode == LibraryDisplayMode::List { "list" } else { "grid" }
}

/// The three grid styles of the Grid style radios: the title on the cover, under it, or no title.
fn grid_styles() -> [(&'static str, String, LibraryDisplayMode); 3] {
    [
        ("compact", tr(MsgKey::ui_compact), LibraryDisplayMode::CompactGrid),
        ("comfortable", tr(MsgKey::ui_comfortable), LibraryDisplayMode::ComfortableGrid),
        ("cover", tr(MsgKey::ui_cover_only), LibraryDisplayMode::CoverOnlyGrid),
    ]
}

/// The Grid style radio of `mode`; none in the list view.
fn style_target(mode: LibraryDisplayMode) -> &'static str {
    grid_styles().into_iter().find(|(_, _, m)| *m == mode).map_or("", |(token, _, _)| token)
}

/// The grid a Grid style radio picks (from the list view too).
fn style_mode(token: &str) -> Option<LibraryDisplayMode> {
    grid_styles().into_iter().find(|(t, _, _)| *t == token).map(|(_, _, m)| m)
}

/// The cover size slider: the column width of the Library and Browse grids. A move is stored at once, and the grids
/// follow the stored value, so the covers resize while it moves.
fn cover_size_slider(pref: &Pref<i32>) -> gtk::Widget {
    use gtk::prelude::*;
    use kuro_domain::library::columns::{COVER_WIDTH_MAX_PX, COVER_WIDTH_MIN_PX, COVER_WIDTH_STEP_PX, column_width, snap_cover_width};
    let scale = gtk::Scale::with_range(
        gtk::Orientation::Horizontal,
        f64::from(COVER_WIDTH_MIN_PX),
        f64::from(COVER_WIDTH_MAX_PX),
        f64::from(COVER_WIDTH_STEP_PX),
    );
    scale.set_value(f64::from(column_width(pref.get())));
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.set_size_request(200, -1);
    scale.update_property(&[gtk::accessible::Property::Label(&tr(MsgKey::pref_cover_size))]);
    let pref = pref.clone();
    scale.connect_value_changed(move |scale| {
        let width = snap_cover_width(scale.value());
        if width != pref.get()
            && let Err(e) = pref.set(width)
        {
            tracing::warn!(error = %e, "the cover size could not be saved");
        }
    });
    scale.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_grid_style_picks_its_grid_and_the_list_view_marks_none() {
        for (token, _, mode) in grid_styles() {
            assert_eq!(style_mode(token), Some(mode));
            assert_eq!(style_target(mode), token);
        }
        assert_eq!(style_target(LibraryDisplayMode::List), "", "no style is marked in the list view");
        assert_eq!(style_mode("list"), None);
    }

    #[test]
    fn grid_keeps_a_grid_style_and_list_is_list() {
        assert_eq!(mode(true, LibraryDisplayMode::ComfortableGrid), LibraryDisplayMode::List);
        assert_eq!(mode(false, LibraryDisplayMode::List), LibraryDisplayMode::DEFAULT);
        assert_eq!(mode(false, LibraryDisplayMode::CoverOnlyGrid), LibraryDisplayMode::CoverOnlyGrid);
        assert_eq!(mode_target(LibraryDisplayMode::CoverOnlyGrid), "grid");
        assert_eq!(mode_target(LibraryDisplayMode::List), "list");
    }

    #[test]
    fn only_look_picks_give_a_mode_and_the_labels_are_translated() {
        let list = LibraryDisplayMode::List;
        assert_eq!(picked(&Pick::Radio(MODE, "grid".into()), list), Some(LibraryDisplayMode::DEFAULT));
        assert_eq!(picked(&Pick::Radio(STYLE, "cover".into()), list), Some(LibraryDisplayMode::CoverOnlyGrid));
        assert_eq!(picked(&Pick::Radio("sort", "grid".into()), list), None);
        assert_eq!(picked(&Pick::Toggle(MODE, true), list), None);
        let titles: Vec<_> = sections().into_iter().map(|s| s.title.unwrap_or_default()).collect();
        assert_eq!(titles, ["View", "Grid style", "Cover size"]);
    }
}
