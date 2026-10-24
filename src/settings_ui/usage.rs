//! Storage usage of Data and storage (tasks T-47b-037, T-47b-038 and T-47b-039): the storage roots
//! (`kuro_platform::storage::roots`), the usage state of each root (bar, caption, or "Storage size unavailable") and the
//! SI size formatter. Sizes come from GIO, asked on a worker thread so a hung mount cannot block the page. The rows replace the one "Available: ... / Total: ..." row of the spec page.
use kuro_i18n::{Arg, MsgKey, tr, trf};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gtk::gio;
use gtk::prelude::*;

/// How long a size query may take before its root shows no bar (T-47b-038).
pub const SIZE_TIMEOUT: Duration = Duration::from_secs(2);

/// The caption of a root whose size is unknown.
pub fn unavailable() -> String {
    tr(MsgKey::settings_usage_unavailable)
}

/// Total and available bytes of a file system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Space {
    pub total: u64,
    pub available: u64,
}

/// The size of the file system at `path`, or `None` when the query fails or takes longer than [`SIZE_TIMEOUT`].
pub fn space_of(path: &Path) -> Option<Space> {
    let path = path.to_path_buf();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(query(&path));
    });
    receiver.recv_timeout(SIZE_TIMEOUT).ok().flatten()
}

fn query(path: &Path) -> Option<Space> {
    let info = gio::File::for_path(path)
        .query_filesystem_info("filesystem::size,filesystem::free", None::<&gio::Cancellable>)
        .ok()?;
    Some(Space { total: info.attribute_uint64("filesystem::size"), available: info.attribute_uint64("filesystem::free") })
}

/// What a root's row shows: a bar at `fraction` with its caption, or no bar and [`unavailable()`].
#[derive(Clone, Debug, PartialEq)]
pub enum Usage {
    Bar { fraction: f64, caption: String },
    Unavailable,
}

/// `(total - available) / total` clamped to 0..1, with the caption `Available: <x> / Total: <y>`. A total that is not
/// positive, or no answer, is unavailable.
pub fn usage(space: Option<Space>) -> Usage {
    match space {
        Some(Space { total, available }) if total > 0 => {
            let fraction = ((total as f64 - available as f64) / total as f64).clamp(0.0, 1.0);
            Usage::Bar { fraction, caption: trf(MsgKey::available_disk_space_info, &[("arg1", Arg::Str(&si_size(available))), ("arg2", Arg::Str(&si_size(total)))]) }
        }
        _ => Usage::Unavailable,
    }
}

/// An SI size: `0 B`, `1.5 kB`, `1.0 MB`, `12 GB`. A value below ten shows one decimal, a larger one none; a value that
/// would round to 1000 moves to the next unit.
pub fn si_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut unit = 1;
    let mut value = bytes as f64 / 1000.0;
    loop {
        let text = if value < 10.0 { format!("{value:.1}") } else { format!("{value:.0}") };
        if text.parse::<f64>().unwrap_or(0.0) >= 1000.0 && unit + 1 < UNITS.len() {
            unit += 1;
            value /= 1000.0;
            continue;
        }
        return format!("{text} {}", UNITS[unit]);
    }
}

/// Every storage root with its usage, from the live mount table and the data directory of this app.
pub fn live_rows() -> Vec<(PathBuf, Usage)> {
    let data_dir = kuro_platform::system_paths::SystemPaths::system().map(|p| kuro_platform::paths::Paths::data_dir(&p)).unwrap_or_default();
    kuro_platform::storage::roots(&data_dir)
        .into_iter()
        .map(|root| {
            let usage = usage(space_of(&root));
            (root, usage)
        })
        .collect()
}

fn root_widget(root: &Path, usage: &Usage) -> gtk::ListBoxRow {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    column.set_margin_top(12);
    column.set_margin_bottom(12);
    column.set_margin_start(16);
    column.set_margin_end(16);
    column.append(
        &gtk::Label::builder()
            .label(root.display().to_string())
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .css_classes(["heading"])
            .build(),
    );
    match usage {
        Usage::Bar { fraction, caption } => {
            let bar = gtk::ProgressBar::builder().fraction(*fraction).build();
            bar.set_size_request(-1, 12);
            column.append(&bar);
            column.append(&dim(caption));
        }
        Usage::Unavailable => column.append(&dim(&unavailable())),
    }
    gtk::ListBoxRow::builder().child(&column).activatable(false).selectable(false).build()
}

fn dim(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).css_classes(["dim-label"]).build()
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

/// Puts one row per root where the spec's "Available: ... / Total: ..." row stands. Returns false when the page has no
/// such row, which leaves the page as it is.
pub fn replace_usage_row(page: &gtk::Widget, rows: &[(PathBuf, Usage)]) -> bool {
    let Some(label) = find_label(page, "Available:") else { return false };
    let (Some(row), Some(list)) = (label.ancestor(gtk::ListBoxRow::static_type()), label.ancestor(gtk::ListBox::static_type())) else {
        return false;
    };
    let (Some(row), Some(list)) = (row.downcast::<gtk::ListBoxRow>().ok(), list.downcast::<gtk::ListBox>().ok()) else {
        return false;
    };
    let at = row.index();
    for (offset, (root, usage)) in rows.iter().enumerate() {
        list.insert(&root_widget(root, usage), at + 1 + i32::try_from(offset).unwrap_or(0));
    }
    list.remove(&row);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_si_formatter_matches_the_edge_table() {
        assert_eq!(si_size(0), "0 B");
        assert_eq!(si_size(1500), "1.5 kB");
        assert_eq!(si_size(999_999), "1.0 MB");
        assert_eq!(si_size(12_300_000_000), "12 GB");
        assert_eq!(si_size(512), "512 B");
        assert_eq!(si_size(64_000_000_000), "64 GB");
    }

    #[test]
    fn the_bar_fraction_is_used_over_total_and_clamped() {
        let full_half = usage(Some(Space { total: 64_000_000_000, available: 32_000_000_000 }));
        assert_eq!(full_half, Usage::Bar { fraction: 0.5, caption: "Available: 32 GB / Total: 64 GB".to_owned() });
        let over = usage(Some(Space { total: 100, available: 200 }));
        assert!(matches!(over, Usage::Bar { fraction, .. } if fraction == 0.0));
        let under = usage(Some(Space { total: 100, available: 0 }));
        assert!(matches!(under, Usage::Bar { fraction, .. } if fraction == 1.0));
    }

    #[test]
    fn a_zero_total_or_no_answer_is_unavailable_with_no_bar() {
        assert_eq!(usage(Some(Space { total: 0, available: 0 })), Usage::Unavailable);
        assert_eq!(usage(None), Usage::Unavailable);
    }

    #[test]
    fn a_size_query_for_the_running_directory_answers_in_time() {
        let dir = tempfile::tempdir().unwrap();
        let space = space_of(dir.path()).expect("the temp directory has a file system");
        assert!(space.total >= space.available);
    }
}
