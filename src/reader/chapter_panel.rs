//! The chapter list of the reader, and the episode list of the player: a panel that slides in from the right over the
//! pages or the video, so a far chapter or episode is one click away instead of a trip back to the entry page. The top
//! bar's list button opens it (and L in the reader, E in the player); a click on a row, or Enter in its search box, opens
//! that one. The matching rules are pure functions, tested without GTK.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use super::strings;

/// One chapter of the reader's list, as the panel shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The chapter's position in the reader's list, what a pick hands back.
    pub index: usize,
    pub name: String,
    /// The chapter number, `-1` when the source gave none.
    pub number: f64,
    pub scanlator: Option<String>,
    pub read: bool,
    pub bookmarked: bool,
    pub downloaded: bool,
    pub current: bool,
}

/// A chapter number as typed: `57` for a whole number, `57.5` otherwise. `None` for an unknown number.
pub fn number_text(number: f64) -> Option<String> {
    if number < 0.0 {
        None
    } else if number.fract() == 0.0 {
        Some(format!("{}", number as i64))
    } else {
        Some(number.to_string())
    }
}

/// Whether `row` matches the search text: every row for a blank query, else a number that starts with it or a name
/// (or scanlator) that contains it, ignoring case.
pub fn matches(row: &Row, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    number_text(row.number).is_some_and(|n| n.starts_with(&query))
        || row.name.to_lowercase().contains(&query)
        || row.scanlator.as_deref().is_some_and(|s| s.to_lowercase().contains(&query))
}

/// The chapter Enter opens: the one whose number is exactly the query, else the first match in the order shown.
pub fn enter_target(rows: &[Row], query: &str) -> Option<usize> {
    let query = query.trim();
    rows.iter()
        .find(|r| number_text(r.number).as_deref() == Some(query))
        .or_else(|| rows.iter().find(|r| matches(r, query)))
        .map(|r| r.index)
}

/// The rows in the order shown: the newest chapter first, as on the manga page. The reader's list is ascending.
pub fn shown_order(mut rows: Vec<Row>) -> Vec<Row> {
    rows.reverse();
    rows
}

pub struct ChapterPanel {
    revealer: gtk::Revealer,
    /// "Chapters" or "Episodes"; the title adds the count.
    heading: String,
    title: gtk::Label,
    search: gtk::SearchEntry,
    scroller: gtk::ScrolledWindow,
    list: gtk::ListBox,
    /// The rows shown, in the list box's order.
    rows: RefCell<Vec<Row>>,
    on_pick: RefCell<Option<Rc<dyn Fn(usize)>>>,
    on_closed: RefCell<Option<Rc<dyn Fn()>>>,
}

impl ChapterPanel {
    /// A panel titled `heading` ("Chapters", "Episodes") with `placeholder` in its search box.
    pub fn new(heading: String, placeholder: String) -> Rc<Self> {
        let title = gtk::Label::builder().xalign(0.0).hexpand(true).css_classes(["heading"]).build();
        let close = gtk::Button::builder().icon_name("window-close-symbolic").css_classes(["flat"]).tooltip_text(strings::close()).build();
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.append(&title);
        head.append(&close);
        let search = gtk::SearchEntry::builder().placeholder_text(placeholder).build();
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("navigation-sidebar");
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();
        let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
        column.add_css_class("background");
        column.add_css_class("reader-chapter-panel");
        column.set_width_request(320);
        let top = gtk::Box::new(gtk::Orientation::Vertical, 8);
        top.set_margin_top(8);
        top.set_margin_start(12);
        top.set_margin_end(8);
        top.append(&head);
        top.append(&search);
        column.append(&top);
        column.append(&scroller);
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideLeft)
            .transition_duration(200)
            .halign(gtk::Align::End)
            .valign(gtk::Align::Fill)
            .child(&column)
            .build();
        // the panel takes no room while it is shut, so a click on the pages behind it still turns them
        revealer.set_visible(false);

        let panel = Rc::new(Self {
            revealer,
            heading,
            title,
            search: search.clone(),
            scroller,
            list: list.clone(),
            rows: RefCell::new(Vec::new()),
            on_pick: RefCell::new(None),
            on_closed: RefCell::new(None),
        });
        let weak = Rc::downgrade(&panel);
        close.connect_clicked(move |_| {
            if let Some(panel) = weak.upgrade() {
                panel.close();
            }
        });
        let weak = Rc::downgrade(&panel);
        search.connect_search_changed(move |entry| {
            if let Some(panel) = weak.upgrade() {
                panel.filter(&entry.text());
            }
        });
        let weak = Rc::downgrade(&panel);
        search.connect_activate(move |entry| {
            let Some(panel) = weak.upgrade() else { return };
            let target = enter_target(&panel.rows.borrow(), &entry.text());
            if let Some(index) = target {
                panel.pick(index);
            }
        });
        // Escape in the search box closes the panel
        let weak = Rc::downgrade(&panel);
        search.connect_stop_search(move |_| {
            if let Some(panel) = weak.upgrade() {
                panel.close();
            }
        });
        let weak = Rc::downgrade(&panel);
        list.connect_row_activated(move |_, row| {
            let Some(panel) = weak.upgrade() else { return };
            let index = usize::try_from(row.index()).ok().and_then(|i| panel.rows.borrow().get(i).map(|r| r.index));
            if let Some(index) = index {
                panel.pick(index);
            }
        });
        panel
    }

    /// The widget to put over the pages, aligned to their right edge.
    pub fn root(&self) -> &gtk::Revealer {
        &self.revealer
    }

    pub fn is_open(&self) -> bool {
        self.revealer.reveals_child()
    }

    /// Whether the keyboard focus is in the panel (its search box or a row), so the keys belong to it.
    pub fn has_focus(&self) -> bool {
        self.revealer.root().and_then(|root| root.focus()).is_some_and(|focus| focus.is_ancestor(&self.revealer))
    }

    /// Called with the reader-list index of the chapter picked. The panel closes first.
    pub fn connect_pick(&self, f: impl Fn(usize) + 'static) {
        *self.on_pick.borrow_mut() = Some(Rc::new(f));
    }

    /// Called after the panel closed, so the reader takes the keys back.
    pub fn connect_closed(&self, f: impl Fn() + 'static) {
        *self.on_closed.borrow_mut() = Some(Rc::new(f));
    }

    /// Opens the panel on `rows` (the reader's list, in its own order) with the search box empty and focused, scrolled to
    /// the chapter being read.
    pub fn open(&self, rows: Vec<Row>) {
        self.search.set_text("");
        self.set_rows(rows);
        self.revealer.set_visible(true);
        self.revealer.set_reveal_child(true);
        self.search.grab_focus();
        self.scroll_to_current();
    }

    /// Shows `rows` again in an open panel (the chapter being read changed), keeping the search text.
    pub fn refresh(&self, rows: Vec<Row>) {
        self.set_rows(rows);
        self.filter(&self.search.text());
    }

    pub fn close(&self) {
        if !self.is_open() {
            return;
        }
        self.revealer.set_reveal_child(false);
        self.revealer.set_visible(false);
        let closed = self.on_closed.borrow().clone();
        if let Some(closed) = closed {
            closed();
        }
    }

    pub fn toggle(&self, rows: impl FnOnce() -> Vec<Row>) {
        if self.is_open() { self.close() } else { self.open(rows()) }
    }

    fn pick(&self, index: usize) {
        self.close();
        let pick = self.on_pick.borrow().clone();
        if let Some(pick) = pick {
            pick(index);
        }
    }

    fn set_rows(&self, rows: Vec<Row>) {
        let rows = shown_order(rows);
        self.title.set_text(&format!("{} · {}", self.heading, rows.len()));
        self.list.remove_all();
        for row in &rows {
            self.list.append(&build_row(row));
        }
        *self.rows.borrow_mut() = rows;
    }

    fn filter(&self, query: &str) {
        let rows = self.rows.borrow();
        let mut child = self.list.first_child();
        for row in rows.iter() {
            let Some(widget) = child else { break };
            widget.set_visible(matches(row, query));
            child = widget.next_sibling();
        }
    }

    /// Scrolls so the chapter being read sits in the middle, once the list has its size.
    fn scroll_to_current(&self) {
        let Some(position) = self.rows.borrow().iter().position(|r| r.current) else { return };
        let Some(row) = self.list.row_at_index(position as i32) else { return };
        let adjustment = self.scroller.vadjustment();
        let list = self.list.clone();
        self.list.add_tick_callback(move |_, _| {
            if row.height() == 0 {
                return glib::ControlFlow::Continue;
            }
            if let Some(point) = row.compute_point(&list, &gtk::graphene::Point::new(0.0, 0.0)) {
                let middle = f64::from(point.y()) - (adjustment.page_size() - f64::from(row.height())) / 2.0;
                adjustment.set_value(middle.clamp(adjustment.lower(), adjustment.upper() - adjustment.page_size()));
            }
            glib::ControlFlow::Break
        });
    }
}

/// A row: the chapter name over its scanlator, then a bookmark and a downloaded mark. A read chapter is dimmed, and the
/// chapter being read is highlighted.
fn build_row(row: &Row) -> gtk::ListBoxRow {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    line.set_margin_top(4);
    line.set_margin_bottom(4);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    let name = gtk::Label::builder().label(&row.name).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
    text.append(&name);
    if let Some(scanlator) = &row.scanlator {
        let label = gtk::Label::builder().label(scanlator).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
        label.add_css_class("caption");
        label.add_css_class("dim-label");
        text.append(&label);
    }
    line.append(&text);
    if row.bookmarked {
        let icon = gtk::Image::from_icon_name("bookmark-filled-symbolic");
        icon.update_property(&[gtk::accessible::Property::Label(&strings::action_filter_bookmarked())]);
        line.append(&icon);
    }
    if row.downloaded {
        let icon = gtk::Image::from_icon_name("folder-download-symbolic");
        icon.add_css_class("dim-label");
        icon.update_property(&[gtk::accessible::Property::Label(&strings::label_downloaded())]);
        line.append(&icon);
    }
    if row.read && !row.current {
        text.add_css_class("dim-label");
    }
    let widget = gtk::ListBoxRow::builder().child(&line).build();
    if row.current {
        widget.add_css_class("reader-chapter-current");
    }
    widget
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(index: usize, number: f64, name: &str) -> Row {
        Row {
            index,
            name: name.to_owned(),
            number,
            scanlator: None,
            read: false,
            bookmarked: false,
            downloaded: false,
            current: false,
        }
    }

    #[test]
    fn a_whole_number_has_no_decimals_and_an_unknown_one_has_no_text() {
        assert_eq!(number_text(57.0).as_deref(), Some("57"));
        assert_eq!(number_text(57.5).as_deref(), Some("57.5"));
        assert_eq!(number_text(-1.0), None);
    }

    #[test]
    fn the_search_matches_a_number_prefix_a_name_or_a_scanlator() {
        let mut r = row(0, 57.0, "Ch. 57: The Rematch");
        r.scanlator = Some("Lunar Scans".into());
        assert!(matches(&r, ""));
        assert!(matches(&r, "5"));
        assert!(matches(&r, " 57 "));
        assert!(!matches(&r, "7"));
        assert!(matches(&r, "rematch"));
        assert!(matches(&r, "lunar"));
        assert!(!matches(&r, "58"));
    }

    #[test]
    fn enter_opens_the_exact_number_before_the_first_match() {
        let rows = shown_order(vec![row(0, 5.0, "Chapter 5"), row(1, 50.0, "Chapter 50"), row(2, 57.0, "Chapter 57")]);
        assert_eq!(enter_target(&rows, "5"), Some(0), "the exact number wins over 57 and 50 shown above it");
        assert_eq!(enter_target(&rows, "57"), Some(2));
        assert_eq!(enter_target(&rows, "chapter"), Some(2), "else the first match shown, the newest");
        assert_eq!(enter_target(&rows, "99"), None);
    }

    #[test]
    fn the_newest_chapter_is_shown_first() {
        let shown = shown_order(vec![row(0, 1.0, "a"), row(1, 2.0, "b")]);
        assert_eq!(shown.iter().map(|r| r.index).collect::<Vec<_>>(), vec![1, 0]);
    }

    #[test]
    fn opening_lists_every_chapter_and_a_pick_hands_back_its_index() {
        crate::testing::with_display(|| {
            let panel = ChapterPanel::new(strings::chapters(), strings::chapter_list_search());
            let picked = Rc::new(std::cell::Cell::new(None));
            let seen = Rc::clone(&picked);
            panel.connect_pick(move |index| seen.set(Some(index)));
            let mut rows: Vec<Row> = (0..120).map(|i| row(i, (i + 1) as f64, &format!("Chapter {}", i + 1))).collect();
            rows[100].current = true;
            panel.open(rows);
            assert!(panel.is_open());
            let mut count = 0;
            let mut child = panel.list.first_child();
            while let Some(widget) = child {
                count += 1;
                child = widget.next_sibling();
            }
            assert_eq!(count, 120);
            panel.search.set_text("57");
            panel.search.emit_activate();
            assert_eq!(picked.get(), Some(56), "chapter 57 is index 56 of the reader's list");
            assert!(!panel.is_open(), "a pick closes the panel");
        });
    }
}
