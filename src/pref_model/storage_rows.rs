//! The storage usage rows of Data and storage (T-22a-035): one row per storage root with its path, a usage bar and the
//! caption "Available: X / Total: Y". The roots come from `kuro_platform::storage::roots`, the sizes from GIO, and the
//! sizes are formatted by `glib::format_size`, since `kuro-backup` has no GTK.

use std::path::Path;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use kuro_backup::storage_info::{StorageRow, storage_row_with};

/// `(total, available)` bytes of the file system at `path`, or `None` when GIO cannot tell.
fn sizes(path: &Path) -> Option<(u64, u64)> {
    let info = gio::File::for_path(path)
        .query_filesystem_info("filesystem::size,filesystem::free", None::<&gio::Cancellable>)
        .ok()?;
    Some((info.attribute_uint64("filesystem::size"), info.attribute_uint64("filesystem::free")))
}

/// One row per mounted storage root.
pub fn rows() -> Vec<StorageRow> {
    let data_dir = kuro_platform::system_paths::SystemPaths::system().map(|p| kuro_platform::paths::Paths::data_dir(&p)).unwrap_or_default();
    kuro_platform::storage::roots(&data_dir)
        .into_iter()
        .map(|root| {
            let (total, available) = sizes(&root).unwrap_or((0, 0));
            storage_row_with(&root, total, available, &|bytes| glib::format_size(bytes).to_string())
        })
        .collect()
}

fn row_widget(row: &StorageRow) -> gtk::ListBoxRow {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    crate::ui::margins(&column, 12);
    column.append(&gtk::Label::builder().label(row.path.display().to_string()).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).css_classes(["heading"]).build());
    if let Some(used) = row.used {
        column.append(&gtk::ProgressBar::builder().fraction(used).build());
    }
    column.append(&crate::ui::dim(&row.caption));
    gtk::ListBoxRow::builder().child(&column).activatable(false).selectable(false).build()
}

fn find_label(widget: &gtk::Widget, starts_with: &str) -> Option<gtk::Label> {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        if label.text().starts_with(starts_with) {
            return Some(label.clone());
        }
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(found) = find_label(&c, starts_with) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

/// Puts the real rows where the screen's one-line "Available: … / Total: …" row stands. Returns false when the page has
/// no such row, which leaves it as it is.
pub fn replace_usage_row(page: &gtk::Widget, rows: &[StorageRow]) -> bool {
    let Some(label) = find_label(page, "Available:") else { return false };
    let (Some(row), Some(list)) = (label.ancestor(gtk::ListBoxRow::static_type()), label.ancestor(gtk::ListBox::static_type())) else {
        return false;
    };
    let (Some(row), Some(list)) = (row.downcast::<gtk::ListBoxRow>().ok(), list.downcast::<gtk::ListBox>().ok()) else { return false };
    let at = row.index();
    for (offset, storage) in rows.iter().enumerate() {
        list.insert(&row_widget(storage), at + 1 + i32::try_from(offset).unwrap_or(0));
    }
    list.remove(&row);
    true
}
