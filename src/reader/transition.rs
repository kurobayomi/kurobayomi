//! Chapter transition card (`ChapterTransition.kt`). T-43a-022.
//!
//! [`content`] is the pure model: which header and chapter go in each block, the fallback text for a missing
//! side, and the gap warning. [`build`] turns it into GTK widgets. Names are cut to 5 lines and scanlators to 2
//! by the labels; the card is at most 460 px wide and keeps its natural width below that.

use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use gtk::pango;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use kuro_data::item::Item;
use kuro_domain::items::missing::item_gap;


use super::model::{ChapterState, ChapterTransition};
use super::strings;

/// One chapter in a block: its header, name, scanlator and download state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChapterLine {
    pub header: String,
    pub name: String,
    pub scanlator: Option<String>,
    pub downloaded: bool,
}

/// A block of the card: a chapter, or the "no chapter" notice for a side that does not exist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Slot {
    Chapter(ChapterLine),
    NoChapter(String),
}

/// The whole card. `gap` is the missing-chapter count between the two chapters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Content {
    pub top: Slot,
    pub bottom: Slot,
    pub gap: i32,
}

impl Content {
    /// The warning between the two blocks: only when the bottom block shows a chapter and the gap is positive.
    pub fn gap_warning(&self) -> Option<String> {
        match self.bottom {
            Slot::Chapter(_) if self.gap > 0 => Some(strings::missing_chapters_warning(self.gap)),
            _ => None,
        }
    }
}

fn line(header: String, chapter: &Item, downloaded: bool) -> ChapterLine {
    ChapterLine {
        header,
        name: chapter.name.clone(),
        scanlator: chapter.scanlator.clone(),
        downloaded,
    }
}

/// `ChapterTransition` (`:49-85`). `from_downloaded` and `to_downloaded` are the download lookups for each side.
pub fn content(transition: &ChapterTransition, from_downloaded: bool, to_downloaded: bool) -> Content {
    let from = &transition.from().chapter;
    let to = transition.to().map(|c| &c.chapter);
    match transition {
        ChapterTransition::Prev { .. } => Content {
            top: match to {
                Some(to) => Slot::Chapter(line(strings::transition_previous(), to, to_downloaded)),
                None => Slot::NoChapter(strings::transition_no_previous()),
            },
            bottom: Slot::Chapter(line(strings::transition_current(), from, from_downloaded)),
            // calculateChapterGap(currChapter, goingToChapter)
            gap: item_gap(Some(from), to),
        },
        ChapterTransition::Next { .. } => Content {
            top: Slot::Chapter(line(strings::transition_finished(), from, from_downloaded)),
            bottom: match to {
                Some(to) => Slot::Chapter(line(strings::transition_next(), to, to_downloaded)),
                None => Slot::NoChapter(strings::transition_no_next()),
            },
            // calculateChapterGap(goingToChapter, currChapter)
            gap: item_gap(to, Some(from)),
        },
    }
}

/// The widest the card is laid out (460 dp in the Kotlin card).
pub const CARD_MAX_WIDTH: i32 = 460;

/// The width the card takes in `available`: its natural width, never more than [`CARD_MAX_WIDTH`], and never more than
/// there is room for. Returns the width and the offset that centres it.
pub fn card_extent(available: i32) -> (i32, i32) {
    let width = available.min(CARD_MAX_WIDTH);
    (width, (available - width) / 2)
}

mod max_width {
    use super::*;

    #[derive(Default)]
    pub struct MaxWidth;

    #[glib::object_subclass]
    impl ObjectSubclass for MaxWidth {
        const NAME: &'static str = "KuroMaxWidthLayout";
        type Type = super::MaxWidthLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for MaxWidth {}

    impl LayoutManagerImpl for MaxWidth {
        fn measure(&self, widget: &gtk::Widget, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(child) = widget.first_child() else { return (0, 0, -1, -1) };
            if orientation == gtk::Orientation::Horizontal {
                let (min, natural, _, _) = child.measure(orientation, for_size);
                return (min.min(CARD_MAX_WIDTH), natural.min(CARD_MAX_WIDTH), -1, -1);
            }
            // the height the child needs at the width it will get
            let width = if for_size < 0 { child.measure(gtk::Orientation::Horizontal, -1).1 } else { for_size };
            let (min, natural, _, _) = child.measure(orientation, width.min(CARD_MAX_WIDTH));
            (min, natural, -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, height: i32, baseline: i32) {
            if let Some(child) = widget.first_child() {
                let (card, x) = card_extent(width);
                child.allocate(card, height, baseline, Some(gtk::gsk::Transform::new().translate(&gtk::graphene::Point::new(x as f32, 0.0))));
            }
        }
    }
}

glib::wrapper! {
    /// Gives its only child its natural width up to [`CARD_MAX_WIDTH`], centred (GTK has no max-width).
    pub struct MaxWidthLayout(ObjectSubclass<max_width::MaxWidth>) @extends gtk::LayoutManager;
}

impl MaxWidthLayout {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

/// The GTK card: at most 460 px wide, and as wide as its text asks below that.
pub fn build(content: &Content) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 24);
    column.append(&slot_widget(&content.top));
    if let Some(warning) = content.gap_warning() {
        column.append(&notice(&warning, "dialog-warning-symbolic", "error"));
    }
    column.append(&slot_widget(&content.bottom));
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.set_halign(gtk::Align::Center);
    card.set_layout_manager(Some(MaxWidthLayout::new()));
    card.append(&column);
    card
}

fn slot_widget(slot: &Slot) -> gtk::Widget {
    match slot {
        Slot::Chapter(line) => chapter_text(line).upcast(),
        Slot::NoChapter(text) => notice(text, "dialog-information-symbolic", "accent").upcast(),
    }
}

fn chapter_text(line: &ChapterLine) -> gtk::Box {
    let block = gtk::Box::new(gtk::Orientation::Vertical, 4);
    block.set_halign(gtk::Align::Center);
    let header = gtk::Label::new(Some(&line.header));
    header.add_css_class("title-4");
    block.append(&header);

    let name_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    name_row.set_halign(gtk::Align::Center);
    if line.downloaded {
        let check = gtk::Image::from_icon_name("emblem-ok-symbolic");
        check.set_tooltip_text(Some(&strings::label_downloaded()));
        check.update_property(&[gtk::accessible::Property::Label(&strings::label_downloaded())]);
        name_row.append(&check);
    }
    let name = gtk::Label::new(Some(&line.name));
    name.set_wrap(true);
    name.set_lines(5);
    name.set_ellipsize(pango::EllipsizeMode::End);
    name.set_justify(gtk::Justification::Center);
    name.add_css_class("title-2");
    name_row.append(&name);
    block.append(&name_row);

    if let Some(scanlator) = crate::entry_core::text::shown_scanlator(line.scanlator.as_deref()) {
        let by = gtk::Label::new(Some(scanlator));
        by.set_wrap(true);
        by.set_lines(2);
        by.set_ellipsize(pango::EllipsizeMode::End);
        by.add_css_class("dim-label");
        block.append(&by);
    }
    block
}

fn notice(text: &str, icon: &str, css: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.set_halign(gtk::Align::Center);
    row.set_margin_start(16);
    row.set_margin_end(16);
    row.set_margin_top(12);
    row.set_margin_bottom(12);
    row.add_css_class("card");
    let image = gtk::Image::from_icon_name(icon);
    image.add_css_class(css);
    row.append(&image);
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_max_width_chars(40);
    row.append(&label);
    row
}

/// The spacing of a transition card in a viewer (B§11.4). GTK margins and padding are logical pixels, so the
/// toolkit scales them with the display's scale factor and no conversion is needed here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metrics {
    pub side: i32,
    pub top: i32,
    pub bottom: i32,
    /// The margin above and below the pages status line (webtoon only).
    pub status_margin: i32,
}

/// Pager: 64 px on each side, centred vertically.
pub const PAGER_METRICS: Metrics = Metrics { side: 64, top: 0, bottom: 0, status_margin: 0 };
/// Webtoon: 128 px above and below, 32 px on each side, 16 px around the status line.
pub const WEBTOON_METRICS: Metrics = Metrics { side: 32, top: 128, bottom: 128, status_margin: 16 };

/// The destination's downloaded flag (B§11.2). A local manga is always downloaded and needs no lookup. Otherwise
/// the lookup decides for the destination chapter, and a missing destination is not downloaded.
pub fn going_to_downloaded(manga_is_local: bool, to: Option<&Item>, lookup: impl FnOnce(&Item) -> bool) -> bool {
    manga_is_local || to.is_some_and(lookup)
}

/// The status line under a destination card (B§11.3): nothing while waiting or once loaded, the spinner text while
/// loading, and the error text otherwise.
pub fn status_text(state: &ChapterState) -> Option<String> {
    match state {
        ChapterState::Wait | ChapterState::Loaded(_) => None,
        ChapterState::Loading => Some(strings::transition_pages_loading().to_owned()),
        ChapterState::Error(e) => Some(strings::failed_to_load_pages(&e.message)),
    }
}

/// Runs the download lookup for the destination off the GTK thread, then calls `done` on the main loop with the
/// flag. Only the chapter and the lookup cross the thread; the GTK side keeps its own state.
pub fn going_to_downloaded_off_thread(
    manga_is_local: bool,
    to: Option<Item>,
    lookup: Arc<dyn Fn(&Item) -> bool + Send + Sync>,
    done: impl FnOnce(bool) + 'static,
) {
    crate::background::run(move || going_to_downloaded(manga_is_local, to.as_ref(), |c| lookup(c)), done);
}

/// The line under a destination card for its state: a spinner with the loading text, or the error with a Retry
/// button that asks for the chapter again. Nothing for a waiting or loaded chapter.
pub fn status_box(state: &ChapterState, on_retry: Rc<dyn Fn()>) -> Option<gtk::Box> {
    let text = status_text(state)?;
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    column.set_halign(gtk::Align::Center);
    column.set_margin_top(8);
    column.set_margin_bottom(8);
    if matches!(state, ChapterState::Loading) {
        let spinner = gtk::Spinner::new();
        spinner.start();
        column.append(&spinner);
    }
    let label = gtk::Label::new(Some(&text));
    label.set_wrap(true);
    label.set_justify(gtk::Justification::Center);
    column.append(&label);
    if matches!(state, ChapterState::Error(_)) {
        let retry = gtk::Button::with_label(&strings::retry());
        retry.set_halign(gtk::Align::Center);
        retry.connect_clicked(move |_| on_retry());
        column.append(&retry);
    }
    Some(column)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::model::ReaderChapter;
    use std::rc::Rc;

    fn chapter(id: i64, number: f64, name: &str, scanlator: Option<&str>) -> Rc<ReaderChapter> {
        let mut c = Item::create(kuro_platform::kind::Kind::Manga);
        c.id = id;
        c.url = format!("/c/{id}");
        c.name = name.into();
        c.number = number;
        c.scanlator = scanlator.map(String::from);
        ReaderChapter::new(c)
    }

    #[test]
    fn prev_shows_previous_over_the_target_and_current_under() {
        let from = chapter(2, 2.0, "Two", Some("S"));
        let to = chapter(1, 1.0, "One", None);
        let t = ChapterTransition::Prev { from, to: Some(to) };
        let c = content(&t, false, true);
        assert_eq!(c.top, Slot::Chapter(ChapterLine { header: "Previous:".to_string(), name: "One".into(), scanlator: None, downloaded: true }));
        assert_eq!(c.bottom, Slot::Chapter(ChapterLine { header: "Current:".to_string(), name: "Two".into(), scanlator: Some("S".into()), downloaded: false }));
        assert_eq!(c.gap, 0);
        assert_eq!(c.gap_warning(), None);
    }

    #[test]
    fn next_shows_finished_over_the_current_and_next_under_the_target() {
        let from = chapter(1, 1.0, "One", None);
        let to = chapter(2, 2.0, "Two", None);
        let t = ChapterTransition::Next { from, to: Some(to) };
        let c = content(&t, true, false);
        assert!(matches!(&c.top, Slot::Chapter(l) if l.header == "Finished:" && l.name == "One" && l.downloaded));
        assert!(matches!(&c.bottom, Slot::Chapter(l) if l.header == "Next:" && l.name == "Two"));
    }

    #[test]
    fn a_missing_target_shows_the_no_chapter_text_in_its_slot() {
        let from = chapter(2, 2.0, "Two", None);
        let prev = ChapterTransition::Prev { from: Rc::clone(&from), to: None };
        assert_eq!(content(&prev, false, false).top, Slot::NoChapter("There's no previous chapter".to_string()));
        let next = ChapterTransition::Next { from, to: None };
        assert_eq!(content(&next, false, false).bottom, Slot::NoChapter("There's no next chapter".to_string()));
    }

    #[test]
    fn gap_warning_only_when_the_gap_is_positive_and_the_bottom_shows_a_chapter() {
        let from = chapter(1, 5.0, "Five", None);
        let to = chapter(2, 9.0, "Nine", None);
        let next = ChapterTransition::Next { from: Rc::clone(&from), to: Some(Rc::clone(&to)) };
        let c = content(&next, false, false);
        assert_eq!(c.gap, 3);
        assert_eq!(c.gap_warning().as_deref(), Some("Skipping 3 chapters, either the source is missing them or they have been filtered out"));

        let close = chapter(3, 5.5, "Five and a half", None);
        let no_gap = ChapterTransition::Next { from, to: Some(close) };
        assert_eq!(content(&no_gap, false, false).gap_warning(), None);

        let unknown = chapter(4, -1.0, "?", None);
        let unknown_next = ChapterTransition::Next { from: unknown, to: Some(to) };
        assert_eq!(content(&unknown_next, false, false).gap, 0, "unknown numbers never warn");
    }

    #[test]
    fn the_card_takes_its_available_width_up_to_460_and_is_centred_beyond_it() {
        assert_eq!(card_extent(300), (300, 0), "a narrow window gives the card all of it");
        assert_eq!(card_extent(460), (460, 0));
        assert_eq!(card_extent(1000), (460, 270));
    }

    fn natural_width(card: &gtk::Box) -> i32 {
        card.measure(gtk::Orientation::Horizontal, -1).1
    }

    #[test]
    fn a_long_name_is_capped_at_460_and_a_short_one_keeps_its_natural_width() {
        crate::testing::with_display(|| {
            let transition = |name: &str| ChapterTransition::Next { from: chapter(1, 1.0, name, None), to: None };
            let long = build(&content(&transition(&"a very long chapter name ".repeat(30)), false, false));
            assert_eq!(natural_width(&long), CARD_MAX_WIDTH);
            let short = build(&content(&transition("One"), false, false));
            let width = natural_width(&short);
            assert!(width < CARD_MAX_WIDTH, "a short card is not stretched to 460 (got {width})");
            assert!(width > 0);
        });
    }
}
