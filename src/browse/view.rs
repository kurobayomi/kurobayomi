//! Thin GTK widgets for the browse primitives. Every rule they show comes from the pure modules next to this file.

use gtk::prelude::*;
use gtk::{Align, Box as GBox, Button, Image, Label, Orientation as O};
use kuro_prefs::library::LibraryDisplayMode;
use kuro_i18n::tr;

use std::path::Path;
use std::rc::Rc;

use super::grid::{CELL_MARGIN_PX, badge_visible, cell_alpha};
use super::icons::{ICON_HEIGHT_PX, IconRef, SourceIcon, fallback_glyph, fetch_icon, http_get, icon_cache_root, icon_ref, load_icon, placeholder_rgba};
use super::listing::ListItem;
use super::primitives::EmptyAction;
use crate::theme::tokens::{SPACE_M, SPACE_S, SPACE_XS};

/// The error page of a listing: `widgets::status::error_with` with the buttons of `EmptyAction`, in the order given (icon and
/// words, so a person who does not know the icon still reads what the button does). `on_action` runs for the button pressed.
pub fn error_state(message: &str, actions: &[EmptyAction], on_action: impl Fn(EmptyAction) + Clone + 'static) -> adw::StatusPage {
    let buttons: Vec<Button> = actions
        .iter()
        .copied()
        .take(super::primitives::EMPTY_ACTION_LIMIT)
        .map(|action| {
            let content = GBox::new(O::Horizontal, SPACE_S);
            content.append(&Image::from_icon_name(action.icon()));
            content.append(&Label::new(Some(&action.label())));
            let button = Button::new();
            button.set_child(Some(&content));
            button.set_tooltip_text(Some(&action.label()));
            let callback = on_action.clone();
            button.connect_clicked(move |_| callback(action));
            button
        })
        .collect();
    crate::widgets::status::error_with(message, None, &buttons)
}

/// The icon of a row, 40 px square. A built-in glyph is tinted with the error colour for a stub or an untrusted
/// extension. The file of an installed extension shows the placeholder fill while it is read and decoded on a
/// blocking thread, then the picture; a file that cannot be read leaves the default glyph (T-41a-004). The icon of an
/// available extension is a url: the same placeholder shows while it is fetched through the image cache, and a failed
/// fetch leaves the error glyph (T-40a-027).
pub fn icon_image(icon: SourceIcon, file: Option<&str>) -> Image {
    icon_image_sized(icon, file, ICON_HEIGHT_PX)
}

/// `icon_image` at `px` square: 40 in a row, 112 on the details page.
pub fn icon_image_sized(icon: SourceIcon, file: Option<&str>, px: i32) -> Image {
    let image = match (icon.icon_name(), file) {
        (Some(name), _) => Image::from_icon_name(name),
        (None, None) => Image::from_icon_name(SourceIcon::Default.icon_name().unwrap_or("image-missing")),
        (None, Some(file)) => {
            let image = Image::new();
            let [r, g, b, a] = placeholder_rgba();
            image.set_paintable(Some(&texture_of(1, 1, &[r, g, b, a])));
            let (weak, source) = (image.downgrade(), file.to_owned());
            gtk::glib::spawn_future_local(async move {
                let url = matches!(icon_ref(&source), IconRef::Url(_));
                let decoded = gtk::gio::spawn_blocking(move || match icon_ref(&source) {
                    IconRef::Url(url) => fetch_icon(icon_cache_root().as_deref(), url, http_get),
                    IconRef::File(file) => load_icon(std::path::Path::new(file)),
                })
                .await
                .ok()
                .flatten();
                let Some(image) = weak.upgrade() else { return };
                match decoded {
                    Some(icon) => image.set_paintable(Some(&texture_of(icon.width, icon.height, &icon.rgba))),
                    None => {
                        let (glyph, error) = fallback_glyph(url);
                        image.set_icon_name(Some(glyph));
                        if error {
                            image.add_css_class("error");
                        }
                    }
                }
            });
            image
        }
    };
    image.set_pixel_size(px);
    if icon.is_error() {
        image.add_css_class("error");
    }
    image
}

fn texture_of(width: u32, height: u32, rgba: &[u8]) -> gtk::gdk::MemoryTexture {
    gtk::gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &gtk::glib::Bytes::from(rgba),
        width as usize * 4,
    )
}

/// The saved PNG icon of a repo source: 40 px square and cover-fitted, with the letter tile's rounded corners.
pub fn repo_picture(path: &Path) -> gtk::Image {
    // an Image at a fixed pixel size: a Picture would ask for the file's own size (192 px) and stretch the row
    let image = gtk::Image::from_file(path);
    image.set_pixel_size(ICON_HEIGHT_PX);
    image.add_css_class("kb-row-cover");
    image.set_overflow(gtk::Overflow::Hidden);
    image.set_halign(Align::Start);
    image.set_valign(Align::Center);
    image
}

#[cfg(test)]
/// A source row: a 40 px icon, the name on one line with ellipsis, the language when shown, "Not installed" in the
/// error colour for a stub, and an action slot.
pub fn source_row(
    name: &str,
    language: Option<&str>,
    icon: SourceIcon,
    icon_file: Option<&str>,
    stub: bool,
    action: Option<&gtk::Widget>,
) -> adw::ActionRow {
    source_row_with(name, language, icon, icon_file, None, stub, false, action)
}

/// [`source_row`] for the Sources lists: `picture` is the saved icon of a repo source, shown in place of the letter tile;
/// `failing` adds the "Not working right now" line of a source that failed several times in a row
/// (`errors::is_failing`).
#[allow(clippy::too_many_arguments)]
pub fn source_row_with(
    name: &str,
    language: Option<&str>,
    icon: SourceIcon,
    icon_file: Option<&str>,
    picture: Option<&Path>,
    stub: bool,
    failing: bool,
    action: Option<&gtk::Widget>,
) -> adw::ActionRow {
    // the shared row of `look`; the notes of a stub or a failing source go in red before the action
    let cover: gtk::Widget = if let Some(path) = picture {
        repo_picture(path).upcast()
    } else if icon == SourceIcon::Letter && icon_file.is_none() {
        super::look::row_cover(name, None, ICON_HEIGHT_PX, ICON_HEIGHT_PX)
    } else {
        icon_image(icon, icon_file).upcast()
    };
    let notes = [stub.then(|| tr(kuro_i18n::MsgKey::not_installed)), (failing && !stub).then(super::errors::not_working_badge)];
    let trailing = GBox::new(O::Horizontal, SPACE_S);
    for note in notes.into_iter().flatten() {
        trailing.append(&Label::builder().label(note).css_classes(["error"]).build());
    }
    if let Some(action) = action {
        trailing.append(action);
    }
    let has_trailing = trailing.first_child().is_some();
    super::look::list_row(Some(cover), name, language, has_trailing.then(|| trailing.upcast()))
}

/// Builds the cover of a listed entry at a size, with the app's one cover widget (`look::cover`).
pub type CoverLoader = Rc<dyn Fn(&ListItem, i32, i32) -> gtk::Widget>;

/// The cover builder of a listing of `kind`: a web, `file://` or bare-path cover goes through the shared cover widget
/// (image service, load only while on screen); no cover shows the title's initials.
///
/// `kind` is the kind of the listed entries: the picture is cached under the same key the entry page and the library
/// use for it, so a cover seen in Browse is not downloaded again when the entry opens.
pub fn file_cover_loader(kind: kuro_platform::kind::Kind) -> CoverLoader {
    Rc::new(move |item, width, height| {
        // a bare path (a local title's file) is the same picture as its `file://` address
        let url = item.cover.as_deref().map(|url| if url.starts_with('/') { gtk::gio::File::for_path(url).uri().to_string() } else { url.to_owned() });
        let art = url.map(|url| remote_art(&url, item.source_id, kind));
        super::look::cover(&item.title, art.as_ref(), width, height)
    })
}

/// A listed entry's remote cover. The entry is not a library entry yet (entry id 0, not a favourite), so the image goes to
/// the shared image cache only and no cover file is written; the source id picks the source's image headers. The key is
/// `<kind>;<url>;0`, the key of a stored entry whose cover was never refreshed, so the entry page finds the same file.
pub(super) fn remote_art(url: &str, source_id: i64, kind: kuro_platform::kind::Kind) -> kuro_images::art::ArtRef {
    use kuro_images::art::{ArtKind, ArtRef};
    ArtRef { owner: kind, entry_id: 0, source_id, kind: ArtKind::Cover, url: Some(url.to_owned()), favorite: false, last_modified: 0 }
}

/// A cover: the shared cover widget in the card's rounded frame. Only this widget carries the favourite alpha, so the
/// title and the badge next to it stay at full strength (B§2.7).
pub(super) fn cover_widget(item: &ListItem, width: i32, height: i32, load: &CoverLoader) -> gtk::Overlay {
    let overlay = gtk::Overlay::new();
    overlay.add_css_class("kb-card-cover");
    overlay.set_overflow(gtk::Overflow::Hidden);
    overlay.set_child(Some(&load(item, width, height)));
    overlay.set_opacity(cell_alpha(item.favorite));
    overlay
}

/// The badge of an entry already in the library, in words.
fn in_library_badge() -> Label {
    let badge = super::look::badge(&tr(kuro_i18n::MsgKey::in_library), true);
    badge.set_halign(Align::Start);
    badge.set_valign(Align::Start);
    badge.set_margin_start(SPACE_XS);
    badge.set_margin_top(SPACE_XS);
    badge
}

fn title_label(text: &str, lines: i32) -> Label {
    let label = Label::new(Some(text));
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_lines(lines);
    label.set_wrap(true);
    // the label never asks for more than one character's width, so a long title wraps to the cover's width and never
    // widens its card (a lone card in a global-search row is otherwise given its title's full width)
    label.set_max_width_chars(1);
    label.set_xalign(0.0);
    label
}

/// A grid cell for the comfortable, compact or cover-only mode. A `None` slot is a placeholder and draws nothing.
/// The cell carries half the gap on every side; the container (see `listing_view`) carries the rest of the padding. The
/// cover is `cover_width` px wide and 2:3.
pub fn grid_cell(item: Option<&ListItem>, mode: LibraryDisplayMode, load: &CoverLoader, cover_width: i32) -> GBox {
    grid_cell_lines(item, mode, load, cover_width, 2)
}

/// `grid_cell` with `title_lines` lines of title: the listing shows two, a global-search card three (T-40a-011).
pub fn grid_cell_lines(
    item: Option<&ListItem>,
    mode: LibraryDisplayMode,
    load: &CoverLoader,
    cover_width: i32,
    title_lines: i32,
) -> GBox {
    let cell = GBox::new(O::Vertical, SPACE_XS);
    for margin in [GBox::set_margin_start, GBox::set_margin_end, GBox::set_margin_top, GBox::set_margin_bottom] {
        margin(&cell, CELL_MARGIN_PX);
    }
    // the cell is exactly as wide as its cover, whatever the title says: the grid's own measuring never decides the layout
    // (the Library card does the same)
    cell.set_size_request(cover_width, -1);
    cell.set_halign(Align::Center);
    let Some(item) = item else { return cell };
    let stage = gtk::Overlay::new();
    stage.set_child(Some(&cover_widget(item, cover_width, cover_width * 3 / 2, load)));
    if badge_visible(item.favorite) {
        stage.add_overlay(&in_library_badge());
    }
    match mode {
        LibraryDisplayMode::CompactGrid => {
            let title = title_label(&item.title, title_lines);
            title.add_css_class("kb-card-title-scrim");
            title.set_valign(Align::End);
            title.set_halign(Align::Fill);
            stage.add_overlay(&title);
            cell.append(&stage);
        }
        LibraryDisplayMode::ComfortableGrid | LibraryDisplayMode::List => {
            cell.append(&stage);
            let title = title_label(&item.title, title_lines);
            title.add_css_class("kb-card-title");
            cell.append(&title);
        }
        LibraryDisplayMode::CoverOnlyGrid => cell.append(&stage),
    }
    cell
}

/// The cell of the list-mode listing, a cell of a `gtk::ListView` (the view's `rich-list` class gives it its insets): a
/// small cover, the title on one line, and the In library badge at the end for favourites.
pub fn list_cell(item: Option<&ListItem>, load: &CoverLoader) -> GBox {
    let line = GBox::new(O::Horizontal, SPACE_M);
    let Some(item) = item else { return line };
    let size = super::look::SMALL;
    let cover = cover_widget(item, size.width, size.height, load);
    cover.set_valign(Align::Center);
    line.append(&cover);
    let title = crate::ui::row_title(&item.title);
    title.set_hexpand(true);
    title.set_valign(Align::Center);
    line.append(&title);
    if badge_visible(item.favorite) {
        let badge = super::look::badge(&tr(kuro_i18n::MsgKey::in_library), true);
        line.append(&badge);
    }
    line
}

/// Makes `row` (a [`super::look::list_row`] or a source row) a row of a list of pages: activating it (a click, Enter or
/// Space, through `widgets::row_activate`) runs `on_primary`; a right-click, the Menu key or Shift+F10 (and a long press
/// on a touch screen) opens the menu of the items `menu` returns (`widgets::context_menu`). The menu gestures never
/// activate the row.
pub fn list_row(row: &adw::ActionRow, on_primary: impl Fn() + 'static, menu: impl Fn() -> Vec<crate::widgets::context_menu::Item> + 'static) -> gtk::ListBoxRow {
    row.set_activatable(true);
    crate::widgets::row_activate::on_activate(row, move |_| on_primary());
    crate::widgets::context_menu::attach(row, move |_, _| menu());
    row.clone().upcast()
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::Cell;

    use super::*;

    /// The icon or tile in the prefix of a source row: the first widget below it that wears the row cover class or is
    /// a plain icon.
    fn icon_of(row: &adw::ActionRow) -> gtk::Widget {
        fn find(w: &gtk::Widget) -> Option<gtk::Widget> {
            if w.has_css_class("kb-row-cover") || w.is::<Image>() {
                return Some(w.clone());
            }
            std::iter::successors(w.first_child(), |c| c.next_sibling()).find_map(|c| find(&c))
        }
        find(row.upcast_ref()).expect("the icon slot")
    }

    #[test]
    fn a_source_without_an_icon_shows_a_letter_tile_and_one_with_its_own_icon_keeps_it() {
        crate::testing::with_display(|| {
            let letter = source_row("MangaDex", None, SourceIcon::Letter, None, false, None);
            let tile = icon_of(&letter);
            assert!(tile.has_css_class("kb-cover-initials") && tile.has_css_class("kb-row-cover"));
            assert_eq!(tile.first_child().and_downcast::<Label>().expect("the letter").label(), "M");
            let plain = source_row("Local", None, SourceIcon::Local, None, false, None);
            assert!(icon_of(&plain).is::<Image>());
        });
    }

    #[test]
    fn a_repo_source_with_a_saved_icon_shows_that_picture_in_place_of_the_letter_tile() {
        crate::testing::with_display(|| {
            let dir = tempfile::tempdir().expect("dir");
            let file = dir.path().join("icon.png");
            image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255])).save(&file).expect("a png");
            let row = source_row_with("Repo", None, SourceIcon::Letter, None, Some(&file), false, false, None);
            let picture = icon_of(&row).downcast::<gtk::Image>().expect("the saved icon");
            assert!(picture.has_css_class("kb-row-cover"), "the letter tile's rounded corners");
            assert_eq!(picture.pixel_size(), ICON_HEIGHT_PX);
        });
    }

    #[test]
    fn a_long_title_never_makes_a_card_wider_than_its_cover() {
        crate::testing::with_display(|| {
            let item = ListItem { title: "Omniscient Reader's Viewpoint, the long edition".into(), ..ListItem::default() };
            let cell = grid_cell_lines(Some(&item), LibraryDisplayMode::ComfortableGrid, &loader(), 96, 3);
            let (_, natural, _, _) = cell.measure(O::Horizontal, -1);
            assert!(natural <= 96 + 2 * CELL_MARGIN_PX, "the card asks for {natural} px");
        });
    }

    fn favorite(title: &str) -> ListItem {
        ListItem { title: title.into(), favorite: true, ..ListItem::default() }
    }

    fn loader() -> CoverLoader {
        Rc::new(|_, _, _| GBox::new(O::Vertical, 0).upcast())
    }

    fn a_placeholder_slot_renders_nothing() {
        for mode in LibraryDisplayMode::ALL {
            assert!(grid_cell(None, mode, &loader(), 136).first_child().is_none(), "{mode:?}");
        }
        assert!(list_cell(None, &loader()).first_child().is_none());
    }

    fn a_favourite_dims_only_its_cover_and_not_the_cell_or_the_title() {
        let cell = grid_cell(Some(&favorite("Berserk")), LibraryDisplayMode::ComfortableGrid, &loader(), 136);
        assert_eq!(cell.opacity(), 1.0, "the cell itself is not dimmed");
        let stage = cell.first_child().and_downcast::<gtk::Overlay>().expect("stage");
        let cover = stage.child().expect("cover");
        assert!((cover.opacity() - 0.34).abs() < 0.005, "GTK keeps opacity in 8 bits: {}", cover.opacity());
        let title = stage.next_sibling().and_downcast::<Label>().expect("title under the cover");
        assert_eq!(title.opacity(), 1.0, "the title keeps full strength");
    }

    fn a_cell_has_half_a_gap_of_margin_on_every_side() {
        let cell = grid_cell(None, LibraryDisplayMode::CompactGrid, &loader(), 136);
        assert_eq!((cell.margin_start(), cell.margin_end(), cell.margin_top(), cell.margin_bottom()), (2, 2, 2, 2));
    }

    fn cover_only_has_no_title_and_comfortable_has_one_under_the_cover() {
        let item = ListItem { title: "A".into(), ..ListItem::default() };
        let only = grid_cell(Some(&item), LibraryDisplayMode::CoverOnlyGrid, &loader(), 136);
        assert!(only.first_child().and_then(|c| c.next_sibling()).is_none());
        let comfortable = grid_cell(Some(&item), LibraryDisplayMode::ComfortableGrid, &loader(), 136);
        assert!(comfortable.last_child().is_some_and(|c| c.is::<Label>()));
    }

    fn the_cover_loader_is_called_with_the_item_and_the_cell_size() {
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        let load: CoverLoader = Rc::new(move |item: &ListItem, width, height| {
            sink.borrow_mut().push((item.cover.clone(), item.source_id, width, height));
            GBox::new(O::Vertical, 0).upcast()
        });
        let item = ListItem { title: "A".into(), source_id: 41, cover: Some("/covers/a.png".into()), ..ListItem::default() };
        let _ = grid_cell(Some(&item), LibraryDisplayMode::CompactGrid, &load, 136);
        let _ = list_cell(Some(&item), &load);
        let small = super::super::look::SMALL;
        let cover = Some("/covers/a.png".to_owned());
        assert_eq!(*seen.borrow(), [(cover.clone(), 41, 136, 204), (cover, 41, small.width, small.height)]);
    }

    fn a_listed_cover_is_the_shared_cover_widget() {
        let load = file_cover_loader(kuro_platform::kind::Kind::Manga);
        let item = ListItem { title: "Solo Leveling".into(), ..ListItem::default() };
        let cover = load(&item, 40, 56);
        assert!(cover.has_css_class("kb-cover-initials"), "no cover is the title's initials, as everywhere else");
    }

    fn a_row_click_activates_and_a_right_click_or_the_menu_key_asks_for_the_menu() {
        let (clicks, menus) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let (c, m) = (Rc::clone(&clicks), Rc::clone(&menus));
        let row = list_row(&crate::widgets::row::plain("Row", None, None), move || c.set(c.get() + 1), move || {
            m.set(m.get() + 1);
            Vec::new()
        });
        let list = gtk::ListBox::new();
        list.append(&row);
        crate::widgets::row_activate::click_row(&row);
        assert_eq!(clicks.get(), 1, "a click activates the row once");
        row.emit_by_name::<()>("activate", &[]);
        assert_eq!(clicks.get(), 2, "so does Enter or Space");
        let all = row.observe_controllers();
        for controller in (0..all.n_items()).filter_map(|i| all.item(i)) {
            if let Some(click) = controller.downcast_ref::<gtk::GestureClick>().filter(|c| c.button() == gtk::gdk::BUTTON_SECONDARY) {
                click.emit_by_name::<()>("pressed", &[&1_i32, &0.0_f64, &0.0_f64]);
            } else if let Some(keys) = controller.downcast_ref::<gtk::EventControllerKey>() {
                let none = gtk::gdk::ModifierType::empty();
                let _ = keys.emit_by_name::<bool>("key-pressed", &[&gtk::gdk::Key::Menu, &0_u32, &none]);
            }
        }
        assert_eq!(clicks.get(), 2, "the menu gestures never activate the row");
        assert_eq!(menus.get(), 2, "the right click and the Menu key ask for the menu");
    }

    /// Run by `smoke::browse_widgets_smoke`: every widget check of this lane runs in that one test, on the shared
    /// GTK thread of `crate::testing`.
    pub(crate) fn smoke() {
        a_placeholder_slot_renders_nothing();
        a_favourite_dims_only_its_cover_and_not_the_cell_or_the_title();
        a_cell_has_half_a_gap_of_margin_on_every_side();
        cover_only_has_no_title_and_comfortable_has_one_under_the_cover();
        the_cover_loader_is_called_with_the_item_and_the_cell_size();
        a_listed_cover_is_the_shared_cover_widget();
        a_row_click_activates_and_a_right_click_or_the_menu_key_asks_for_the_menu();
    }
}
