//! The download queue page (lane D): one kind's queue as a list, then every downloaded episode or chapter on disk (the
//! download index), each with a button that opens its folder. The page header has a Pause/Resume button and a Clear
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
use gtk::prelude::*;
use kuro_platform::kind::Kind;
use kuro_downloads::DownloadState;
use kuro_downloads::index::Tree;
use kuro_downloads::queue_model::{QueueItem, QueueModel};
use kuro_downloads::service::Downloads;
use kuro_downloads::state::RunState;
use kuro_i18n::{MsgKey, tr};

use crate::browse::look;
use crate::downloads::live;
use crate::widgets::context_menu::{self, Item};
use crate::widgets::download_button::{DownloadButton, Request};

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
    let (empty, _) = look::empty_state("folder-download-symbolic", &tr(MsgKey::ui_no_downloads_are_queued), &[]);
    empty.set_vexpand(true);
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
    empty: gtk::Box,
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
        let done = downloaded(&index.tree(self.kind), index.root(self.kind));
        let empty = items.is_empty() && done.is_empty();
        self.empty.set_visible(empty);
        self.list.set_visible(!empty);
        for item in items {
            self.list.append(&self.row(item));
        }
        // ponytail: one row per unit in a ListBox; a GtkListView if libraries of thousands make the page slow
        for (label, folder) in done {
            let open = crate::ui::icon_button("document-open-symbolic", &tr(MsgKey::ui_open_containing_folder));
            open.add_css_class("flat");
            open.set_valign(gtk::Align::Center);
            open.connect_clicked(move |_| open_folder(&folder));
            let line = look::list_row(None, &label, Some("Downloaded"), Some(open.upcast()));
            line.set_activatable(false);
            self.list.append(&line);
        }
    }

    fn row(&self, item: &QueueItem) -> gtk::ListBoxRow {
        let (downloads, kind, id) = (self.downloads.clone(), item.kind, item.item_id);
        // a click on a queued or running row cancels it, and on a failed row starts its queue again
        let button = DownloadButton::new(move |request| match request {
            Request::Cancel => live::cancel(kind, id),
            Request::Start => downloads.start(kind),
            Request::Delete => {}
        });
        let (state, percent) = live::queued(&self.downloads.snapshot(), kind, id).unwrap_or((item.state, 0));
        button.set(state, percent);
        let line = look::list_row(None, &format!("{} · {}", item.title, item.item_name), Some(&status_line(item)), Some(button.widget.clone().upcast()));
        line.set_activatable(false);
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

/// Every downloaded unit in `tree` as "title · unit" with the folder that holds it, by source, title and unit name.
fn downloaded(tree: &Tree, root: &Path) -> Vec<(String, PathBuf)> {
    tree.sources
        .values()
        .flat_map(|source| source.entries.values())
        .flat_map(|entry| {
            let folder = root.join(&entry.source_dir).join(&entry.dir_name);
            entry.units.iter().map(move |unit| (format!("{} · {unit}", entry.dir_name), folder.clone()))
        })
        .collect()
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
    fn every_downloaded_unit_is_a_row_with_its_title_folder() {
        use kuro_downloads::index::{EntryDir, SourceDir};
        let entry = EntryDir { source_dir: "Source (EN)".into(), dir_name: "Berserk".into(), units: ["Chapter 1".to_owned(), "Chapter 2".to_owned()].into() };
        let mut tree = Tree::default();
        tree.sources.insert(7, SourceDir { dir_name: "Source (EN)".into(), entries: [("Berserk".to_owned(), entry)].into() });
        let folder = Path::new("/d/Source (EN)/Berserk").to_path_buf();
        assert_eq!(downloaded(&tree, Path::new("/d")), [("Berserk · Chapter 1".to_owned(), folder.clone()), ("Berserk · Chapter 2".to_owned(), folder)]);
        assert!(downloaded(&Tree::default(), Path::new("/d")).is_empty());
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
