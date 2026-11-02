//! The download queue page (lane D): one kind's queue as a list, then what is on disk (the download index), one row per
//! title (`on_disk`): its title, count and size on disk, a click opening its page and a button opening its folder, then
//! its episodes or chapters, each played or read with a click and deleted with its button. The page header has a Pause/Resume button and a Clear
//! queue button (both hidden while the queue is empty) and one that opens the kind's downloads folder. A queued row has
//! the shared download button (a click cancels, or retries a failed row). The commands go to the one download service (`Downloads`), and the rows come from its snapshot. A unit whose source is not installed is an
//! ordinary row that reads "Waiting for source". A row's right-click menu offers Cancel, Retry (a failed row), Move to
//! top and Move to bottom. The list is rebuilt once a second, only when its rows or their states changed; the buttons
//! follow the progress live (`downloads::live`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gtk::glib::{self, prelude::*};
use adw::prelude::*;
use kuro_platform::kind::Kind;
use kuro_downloads::DownloadState;
use kuro_downloads::index::Tree;
use kuro_downloads::queue_model::{QueueItem, QueueModel};
use kuro_downloads::service::Downloads;
use kuro_downloads::state::RunState;
use kuro_i18n::{MsgKey, tr};

use crate::browse::look;
use crate::downloads::{live, on_disk};
use crate::widgets::context_menu::{self, Item};
use crate::widgets::download_button::{DownloadButton, Request};
use crate::widgets::row::{self, RowSpec};

/// How often the page checks the queue.
const REFRESH: Duration = Duration::from_secs(1);

/// The queue page of one kind, its title search and its header buttons (Pause or Resume, Clear queue, the downloads
/// folder), or `None` when
/// the app runs no download service (the Download tab then shows its empty state).
pub fn page(kind: Kind) -> Option<(gtk::Widget, gtk::SearchEntry, Vec<gtk::Widget>)> {
    let downloads = live::downloads()?;
    let column = crate::ui::column();
    let list = crate::ui::boxed_list();
    // the app's empty state (icon, line), as the library and history show theirs
    let empty = crate::widgets::status::empty("folder-download-symbolic", &tr(MsgKey::ui_no_downloads_are_queued), None);
    column.append(&list);
    column.append(&empty);
    let search = crate::ui::list_search(&list);

    let (run, clear) = header_buttons(&downloads, kind);
    let page = Page { kind, downloads: downloads.clone(), list, empty, run: run.clone(), clear: clear.clone(), shown: Rc::new(RefCell::new(None)), buttons: Rc::default() };

    page.draw();
    live::follow(&column, {
        let buttons = Rc::clone(&page.buttons);
        move |change| {
            if let Some(button) = buttons.borrow().get(&(change.kind, change.id)) {
                button.set(change.state, change.percent);
            }
        }
    });
    let weak = column.downgrade();
    let refresh = page.clone();
    glib::timeout_add_local(REFRESH, move || {
        if weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        refresh.draw();
        glib::ControlFlow::Continue
    });
    let folder = crate::ui::icon_button("document-open-symbolic", &tr(MsgKey::ui_open_downloads_folder));
    let root = downloads.index().root(kind).to_path_buf();
    folder.connect_clicked(move |_| open_folder(&root));
    Some((column.upcast(), search, vec![run.upcast(), clear.upcast(), folder.upcast()]))
}

/// The state the page draws: the service and the widgets that change.
#[derive(Clone)]
struct Page {
    kind: Kind,
    downloads: Downloads,
    list: gtk::ListBox,
    empty: adw::StatusPage,
    /// The header's Pause/Resume and Clear queue buttons.
    run: gtk::Button,
    clear: gtk::Button,
    /// The key of the rows last drawn. The list is rebuilt only when it changes.
    shown: Rc<RefCell<Option<String>>>,
    /// The download button of each row drawn, by kind and item id, so the live progress reaches it.
    buttons: Rc<RefCell<HashMap<(Kind, i64), DownloadButton>>>,
}

impl Page {
    fn draw(&self) {
        let model = lane_model(&self.downloads, self.kind);
        let items = model.flat();
        // the run state changes without the rows changing, so the buttons follow it on every tick
        self.run.set_visible(!items.is_empty());
        self.clear.set_visible(!items.is_empty());
        show_run(&self.run, running(&self.downloads, self.kind));
        // the index counts its changes, so a download that lands or a folder deleted on disk redraws the list
        let changes = *self.downloads.index().changes().borrow();
        let key = format!("{}#{changes}", items.iter().map(row_key).collect::<Vec<_>>().join("|"));
        if self.shown.borrow().as_deref() == Some(key.as_str()) {
            return;
        }
        *self.shown.borrow_mut() = Some(key);
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        self.buttons.borrow_mut().clear();
        let index = self.downloads.index();
        let done = on_disk(self.kind, &index.tree(self.kind), index.root(self.kind));
        let empty = items.is_empty() && done.is_empty();
        self.empty.set_visible(empty);
        self.list.set_visible(!empty);
        for item in items {
            self.list.append(&self.row(item));
        }
        // ponytail: every title and unit as widgets in a ListBox; a GtkListView if libraries of thousands make the page slow
        for title in done {
            self.list.append(&title_row(self.kind, title));
        }
    }

    fn row(&self, item: &QueueItem) -> adw::ActionRow {
        let (downloads, kind, id) = (self.downloads.clone(), item.kind, item.item_id);
        // the menu of a queued or running row cancels it (Pause and Resume it runs itself), and a click on a failed row starts its queue again
        let button = DownloadButton::new(kind, move |request| match request {
            Request::Cancel => live::cancel(kind, id),
            Request::Start => downloads.start(kind),
            Request::Delete => {}
        });
        let (state, percent) = live::queued(&self.downloads.snapshot(), kind, id).unwrap_or((item.state, 0));
        button.set(state, percent);
        let line = look::list_row(None, &format!("{} · {}", item.title, item.item_name), Some(&status_line(item)), Some(button.widget.clone().upcast()));
        let (on_menu, drawn) = (self.downloads.clone(), item.state);
        context_menu::attach(&line, move |_, _| {
            // the state now, not the one the row was drawn with
            let failed = live::queued(&on_menu.snapshot(), kind, id).map_or(drawn, |(state, _)| state) == DownloadState::Error;
            row_menu(&on_menu, kind, id, failed)
        });
        self.buttons.borrow_mut().insert((kind, id), button);
        line
    }
}

/// The right-click menu of a queued row: Cancel, Retry when it failed, Move to top and Move to bottom.
fn row_menu(downloads: &Downloads, kind: Kind, id: i64, failed: bool) -> Vec<Item> {
    let mut items = vec![Item::new(tr(MsgKey::action_cancel), 0, move || live::cancel(kind, id))];
    if failed {
        // as the row's button does on a failed row: the queue starts again
        let downloads = downloads.clone();
        items.push(Item::new(tr(MsgKey::action_retry), 0, move || downloads.start(kind)));
    }
    for (label, top) in [(MsgKey::action_move_to_top, true), (MsgKey::action_move_to_bottom, false)] {
        let downloads = downloads.clone();
        items.push(Item::new(tr(label), 1, move || move_item(&downloads, kind, id, top)));
    }
    items
}

/// The rows of one kind's lane, in queue order.
fn lane_model(downloads: &Downloads, kind: Kind) -> QueueModel {
    QueueModel::from_units(downloads.snapshot().of(kind))
}

/// Moves one item to the top or the bottom of its source's section (`QueueModel::move_to_top`) and sends the new order.
fn move_item(downloads: &Downloads, kind: Kind, id: i64, top: bool) {
    let mut model = lane_model(downloads, kind);
    let moved = if top { model.move_to_top(id) } else { model.move_to_bottom(id) };
    if moved {
        downloads.reorder(kind, model.flat().iter().map(|item| item.item_id).collect());
    }
}

/// Whether `kind`'s lane is running.
fn running(downloads: &Downloads, kind: Kind) -> bool {
    downloads.snapshot().run(kind) == RunState::Running
}

/// The header buttons: one that pauses the lane while it runs and resumes it otherwise, and Clear queue. The page
/// hides both while nothing is queued.
fn header_buttons(downloads: &Downloads, kind: Kind) -> (gtk::Button, gtk::Button) {
    let run = crate::ui::icon_button("media-playback-start-symbolic", &tr(MsgKey::ui_resume_downloads));
    let toggle = downloads.clone();
    run.connect_clicked(move |button| {
        if running(&toggle, kind) { toggle.pause(kind) } else { toggle.start(kind) }
        show_run(button, running(&toggle, kind));
    });
    let clear = crate::ui::icon_button("edit-clear-all-symbolic", &tr(MsgKey::ui_clear_queue));
    let downloads = downloads.clone();
    clear.connect_clicked(move |_| downloads.clear(kind));
    (run, clear)
}

/// Shows Pause on the run button while the lane runs, Resume otherwise.
fn show_run(button: &gtk::Button, running: bool) {
    let (icon, tip) = if running { ("media-playback-pause-symbolic", MsgKey::ui_pause_downloads) } else { ("media-playback-start-symbolic", MsgKey::ui_resume_downloads) };
    if button.icon_name().as_deref() != Some(icon) {
        button.set_icon_name(icon);
        button.set_tooltip_text(Some(&tr(tip)));
        button.update_property(&[gtk::accessible::Property::Label(&tr(tip))]);
    }
}

/// The titles on disk, matched to the library (`on_disk`). A read of the library that fails lists the folders as they are.
fn on_disk(kind: Kind, tree: &Tree, root: &Path) -> Vec<on_disk::Title> {
    let db = crate::data::app().db.clone();
    let (entries, items) = (kuro_data::entry::EntryRepo::new(db.clone()), kuro_data::item::ItemRepo::new(db));
    on_disk::titles(kind, tree, root, |source| entries.by_source(kind, source).unwrap_or_default(), |entry| items.by_entry_id(entry, false).unwrap_or_default())
}

/// One title on disk: a heading row (title, count and size; a click opens the title's page, a button its folder), then a
/// row per episode or chapter (a click plays or reads it, a button deletes it), in a list of their own. One row of the
/// page's list, so a search by the title keeps the whole group.
fn title_row(kind: Kind, title: on_disk::Title) -> gtk::ListBoxRow {
    let count = kuro_i18n::tr_plural(crate::entry_core::wording::count(kind), title.units.len() as i64, &[]);
    let folder = crate::ui::icon_button("document-open-symbolic", &tr(MsgKey::ui_open_containing_folder));
    folder.add_css_class("flat");
    let path = title.folder.clone();
    folder.connect_clicked(move |_| open_folder(&path));
    let entry_id = title.entry_id;
    let on_click: Option<Rc<dyn Fn()>> = entry_id.map(|id| Rc::new(move || crate::screens::open_entry(kind, id)) as Rc<dyn Fn()>);
    let heading = row::build(RowSpec { title: &title.label, subtitle: Some(&count), icon: None, trailing: Some(folder.upcast_ref()), on_click });
    if entry_id.is_none() {
        heading.set_tooltip_text(Some(&tr(MsgKey::ui_download_not_in_library)));
    }
    show_size(&heading, count, title.folder.clone());
    // a list of its own inside the page's card: no second frame
    let lines = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
    lines.append(&heading);
    for unit in title.units {
        lines.append(&unit_line(kind, entry_id, unit));
    }
    gtk::ListBoxRow::builder().child(&lines).activatable(false).build()
}

/// One episode or chapter: a click plays or reads it, the button deletes it. A unit the library no longer knows only
/// shows its name.
fn unit_line(kind: Kind, entry_id: Option<i64>, unit: on_disk::Unit) -> adw::ActionRow {
    let (Some(entry_id), Some(item_id)) = (entry_id, unit.item_id) else {
        let line = row::plain(&unit.label, None, None);
        line.set_tooltip_text(Some(&tr(MsgKey::ui_download_not_in_library)));
        return line;
    };
    let delete = crate::ui::icon_button("user-trash-symbolic", &tr(MsgKey::action_delete));
    delete.add_css_class("flat");
    delete.connect_clicked(move |button| {
        // the index changes once the folder is gone, and the page redraws without the line
        button.set_sensitive(false);
        let backend = crate::updates::download_host::current();
        crate::updates::download_host::spawn_detached(move || backend.delete(kind, entry_id, &[item_id]));
    });
    let on_click: Rc<dyn Fn()> = Rc::new(move || crate::screens::open_unit(kind, entry_id, item_id));
    row::build(RowSpec { title: &unit.label, subtitle: None, icon: None, trailing: Some(delete.upcast_ref()), on_click: Some(on_click) })
}

/// Adds the size of `folder` on disk to the count in the subtitle of `heading`, once it is known (it is summed off the GTK
/// thread).
fn show_size(heading: &adw::ActionRow, count: String, folder: PathBuf) {
    let heading = heading.downgrade();
    glib::spawn_future_local(async move {
        let Ok(bytes) = crate::background::blocking(move || kuro_core::storage::dir_size(&folder).unwrap_or(0)).await else { return };
        if let Some(heading) = heading.upgrade() {
            row::set_subtitle(&heading, Some(&format!("{count} · {}", glib::format_size(bytes))));
        }
    });
}

/// Opens `folder` in the file manager; a failure is a toast.
fn open_folder(folder: &Path) {
    crate::ui::open_local(folder, |error| crate::toast::notify(&error));
}

/// What one row shows, as a key: a change of any of these rebuilds the list.
fn row_key(item: &QueueItem) -> String {
    format!("{:?}:{}:{}:{}:{}", item.kind, item.item_id, item.state as u8, item.progress, item.waiting)
}

/// The status text beside a row while it downloads: pages done of the list for a manga (its progress is a sum of page
/// percents, so it is not a percent), the percent for an episode, else "Downloading…". Otherwise the state; a unit whose
/// source is not installed waits for it.
fn status_line(item: &QueueItem) -> String {
    if item.waiting {
        return tr(MsgKey::ui_waiting_for_source).to_owned();
    }
    match item.state {
        DownloadState::Downloading => match (item.kind, item.pages_total) {
            (Kind::Manga, Some(total)) => format!("{} of {total} pages", item.pages_done),
            (Kind::Anime, _) if item.progress > 0 => format!("{}%", item.progress),
            _ => "Downloading…".to_owned(),
        },
        DownloadState::Queue => "Queued".to_owned(),
        DownloadState::Error => "Failed".to_owned(),
        DownloadState::Downloaded => "Downloaded".to_owned(),
        DownloadState::NotDownloaded => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use kuro_platform::kind::Kind;

    use super::*;

    fn item(kind: Kind, state: DownloadState, progress: i64, pages: Option<(u32, u32)>) -> QueueItem {
        QueueItem {
            kind,
            item_id: 1,
            parent_id: 1,
            source_id: 1,
            source_name: "Source".into(),
            title: "Title".into(),
            item_name: "Name".into(),
            date_upload: 0,
            number: 1.0,
            state,
            pages_done: pages.map_or(0, |(done, _)| done),
            pages_total: pages.map(|(_, total)| total),
            progress,
            waiting: false,
        }
    }

    #[test]
    fn a_title_row_has_its_heading_then_a_line_per_unit_and_only_known_units_can_be_deleted() {
        crate::testing::with_display(|| {
            let units = vec![on_disk::Unit { item_id: Some(10), label: "Episode 1".into() }, on_disk::Unit { item_id: None, label: "Episode 9 (stray)".into() }];
            let row = title_row(Kind::Anime, on_disk::Title { entry_id: Some(1), label: "Bleach".into(), folder: "/d/Src/Bleach".into(), units });
            let mut all = Vec::new();
            let mut stack = vec![row.clone().upcast::<gtk::Widget>()];
            while let Some(w) = stack.pop() {
                stack.extend(std::iter::successors(w.first_child(), |c| c.next_sibling()));
                all.push(w);
            }
            let rows: Vec<&adw::ActionRow> = all.iter().filter_map(|w| w.downcast_ref::<adw::ActionRow>()).collect();
            let texts: Vec<String> = rows.iter().flat_map(|r| [r.title().to_string(), r.subtitle().map(|s| s.to_string()).unwrap_or_default()]).collect();
            for want in ["Bleach", "2 episodes", "Episode 1", "Episode 9 (stray)"] {
                assert!(texts.iter().any(|l| l == want), "{want} in {texts:?}");
            }
            assert!(!texts.iter().any(|l| l == "Downloaded"), "no per-row English word");
            let trash = all.iter().filter_map(|w| w.downcast_ref::<gtk::Button>()).filter(|b| b.tooltip_text().as_deref() == Some(tr(MsgKey::action_delete).as_str())).count();
            assert_eq!(trash, 1, "the unit the library knows has a delete button, the stray one none");
            let stray = rows.iter().find(|r| r.title() == "Episode 9 (stray)").expect("the stray line");
            assert!(!stray.is_activatable(), "a unit the library does not know cannot be opened");
            let known = rows.iter().find(|r| r.title() == "Episode 1").expect("the known line");
            assert!(known.is_activatable());
        });
    }

    #[test]
    fn a_row_shows_its_progress_while_it_downloads_and_its_state_otherwise() {
        assert_eq!(status_line(&item(Kind::Anime, DownloadState::Downloading, 40, None)), "40%");
        assert_eq!(status_line(&item(Kind::Anime, DownloadState::Downloading, 0, None)), "Downloading…");
        assert_eq!(status_line(&item(Kind::Manga, DownloadState::Downloading, 250, Some((2, 5)))), "2 of 5 pages", "a manga row counts pages, not the page percent sum");
        assert_eq!(status_line(&item(Kind::Manga, DownloadState::Downloading, 0, None)), "Downloading…");
        assert_eq!(status_line(&item(Kind::Manga, DownloadState::Queue, 0, None)), "Queued");
        assert_eq!(status_line(&item(Kind::Anime, DownloadState::Error, 0, None)), "Failed");
        assert_eq!(status_line(&item(Kind::Manga, DownloadState::Downloaded, 0, None)), "Downloaded");
        let waiting = QueueItem { waiting: true, ..item(Kind::Manga, DownloadState::Queue, 0, None) };
        assert_eq!(status_line(&waiting), "Waiting for source");
    }
}
