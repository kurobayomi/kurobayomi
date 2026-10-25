//! Bulk actions on a selection: mark read or viewed (T-37a-027), change categories (T-37a-030) and the continue
//! button's visibility (T-37a-033). The rules are pure; each write goes through the storage layer that owns it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use kuro_data::entry::{Entry, EntryRepo};
use kuro_i18n::{MsgKey, tr};
use kuro_data::item::{Item, ItemRepo};
use kuro_db::handle::Db;
use kuro_domain::category::bulk;
use kuro_domain::category::service::CategoryService;
use kuro_domain::error::DomainError;
use kuro_domain::history::next_item::next_items;
use kuro_domain::items::status::{ItemDownloads, SetDoneResult, SetDoneStatus, items_to_change};
use kuro_platform::kind::Kind;
use kuro_domain::library::model::LibraryRow;
use kuro_domain::library::search::EmptyPage;

use crate::misc::diet_defaults as diet;
use crate::updates::download_actions::UnitTarget;
use crate::updates::download_exec;
use crate::updates::download_host::DownloadBackend;
use crate::updates::download_state::{self, DiskKey};

/// The category ids an entry keeps after a change. `all_links` must hold every link, hidden ones included, so
/// a hidden category the user cannot see in the dialog is not dropped (E-07).
pub fn next_membership(all_links: &[i64], include: &[i64], exclude: &[i64]) -> Vec<i64> {
    bulk::apply(all_links, include, exclude)
}

/// Applies a dialog result to every selected entry. Returns how many entries failed to save; the others still
/// change. Each entry is one write, which is one transaction in the repository.
pub async fn set_membership(
    categories: &CategoryService,
    entries: &[i64],
    include: &[i64],
    exclude: &[i64],
) -> Result<usize, DomainError> {
    let mut failed = 0;
    for entry in entries {
        let links: Vec<i64> = categories.repo().by_entry(*entry, false)?.iter().map(|c| c.id).collect();
        let next = next_membership(&links, include, exclude);
        if let Err(e) = categories.set_entry_categories(*entry, &next).await {
            tracing::error!(error = %e, entry, "saving categories failed");
            failed += 1;
        }
    }
    Ok(failed)
}

/// "Delete after marked read": the one place the setting is read. Library's mark, the notice's "Mark as read", Updates
/// and the title page all pass it to `SetDoneStatus::run` (decision 7). The downloads go only once the download host is
/// installed (step 5).
pub fn remove_after_marked_read() -> bool {
    diet::DOWNLOAD_REMOVE_AFTER_MARKED_READ
}

/// The download port of `SetDoneStatus` over the app's download backend: the units that became done are checked on disk
/// and the downloaded ones are deleted, grouped per entry (E-U33).
pub struct BackendDownloads(pub Arc<dyn DownloadBackend>);

impl ItemDownloads for BackendDownloads {
    fn delete(&self, e: &Entry, items: &[Item]) {
        let targets: Vec<UnitTarget> = items
            .iter()
            .map(|item| {
                let key = DiskKey {
                    source_id: e.source,
                    entry_title: e.title.clone(),
                    unit_name: item.name.clone(),
                    scanlator: item.scanlator.clone(),
                };
                UnitTarget { entry_id: e.id, unit_id: item.id, state: download_state::derive(None, self.0.on_disk(e.kind, &key)).state }
            })
            .collect();
        download_exec::remove_read_downloads(&*self.0, e.kind, true, &targets);
    }
}

fn set_done(db: &Arc<Db>, backend: Arc<dyn DownloadBackend>) -> SetDoneStatus {
    SetDoneStatus::new(ItemRepo::new(db.clone()), EntryRepo::new(db.clone()), Arc::new(BackendDownloads(backend)))
}

/// The notice's "Mark as read" (T-26a-093): marks the items of one entry named by `urls` read and, when
/// `remove_downloads` is on, deletes the downloads of the items that changed. Urls the entry does not hold are ignored.
/// Returns how many items changed.
pub async fn mark_notified_read(
    db: &Arc<Db>,
    backend: Arc<dyn DownloadBackend>,
    entry_id: i64,
    urls: &[String],
    remove_downloads: bool,
) -> Result<usize, String> {
    let all = ItemRepo::new(db.clone()).by_entry_id(entry_id, false).map_err(|e| e.to_string())?;
    let named: Vec<Item> = all.into_iter().filter(|i| urls.contains(&i.url)).collect();
    let changed = items_to_change(true, &named).len();
    match set_done(db, backend).run(true, named, remove_downloads).await {
        SetDoneResult::Success => Ok(changed),
        SetDoneResult::NoItems => Ok(0),
        SetDoneResult::InternalError(error) => Err(error),
    }
}

/// The ids of the chapters or episodes of an entry whose url is in `urls`. Empty when the entry is gone.
pub fn item_ids_by_url(db: &Arc<Db>, entry_id: i64, urls: &[String]) -> Vec<i64> {
    ItemRepo::new(db.clone())
        .by_entry_id(entry_id, false)
        .map(|rows| rows.into_iter().filter(|i| urls.contains(&i.url)).map(|i| i.id).collect())
        .unwrap_or_default()
}

/// The notice's "Download" (T-26a-093): queues the named items with autostart on, so the queue starts at once. An entry
/// that is gone, or items that match nothing, queue nothing and say nothing. Returns how many items were named.
pub fn download_notified(
    db: &Arc<Db>,
    backend: &dyn DownloadBackend,
    kind: Kind,
    entry_id: i64,
    urls: &[String],
) -> usize {
    let ids = item_ids_by_url(db, entry_id, urls);
    if ids.is_empty() {
        return 0;
    }
    backend.enqueue(kind, entry_id, &ids);
    backend.start_queue(kind);
    ids.len()
}

/// The selection bar's Download: for each selected entry the unread units in reading order that are neither queued nor on
/// disk (`plan_download`), queued together, and then the lane starts once. Returns how many units were queued.
pub fn download_selection(db: &Arc<Db>, backend: &dyn DownloadBackend, kind: Kind, entry_ids: &[i64], amount: DownloadAmount) -> usize {
    let mut queued = 0;
    for &entry_id in entry_ids {
        let Ok(entry) = EntryRepo::new(Arc::clone(db)).by_id(entry_id) else { continue };
        let units = ItemRepo::new(Arc::clone(db)).by_entry_id(entry_id, false).unwrap_or_default();
        let pending: Vec<i64> = next_items(db, kind, entry_id, None, true).into_iter().map(|item| item.id).collect();
        let skip = |id: i64| {
            backend.queue_entry(kind, id).is_some()
                || units.iter().find(|unit| unit.id == id).is_some_and(|unit| {
                    let key = DiskKey { source_id: entry.source, entry_title: entry.title.clone(), unit_name: unit.name.clone(), scanlator: unit.scanlator.clone() };
                    backend.on_disk(kind, &key)
                })
        };
        let ids = plan_download(&pending, amount, skip);
        if !ids.is_empty() && backend.enqueue(kind, entry_id, &ids) {
            queued += ids.len();
        }
    }
    if queued > 0 {
        backend.start_queue(kind);
    }
    queued
}

/// Marks every item of the selected entries done or undone and returns how many entries were written. A failed entry
/// is logged and the rest still change (T-37a-027). `remove_downloads` is [`remove_after_marked_read`].
pub async fn mark_many(db: &Arc<Db>, backend: Arc<dyn DownloadBackend>, ids: &[i64], done: bool, remove_downloads: bool) -> usize {
    let status = set_done(db, backend);
    let mut written = 0;
    for id in ids {
        // read before the write: the items it changes are what the trackers hear about
        let changed = if done { ItemRepo::new(db.clone()).by_entry_id(*id, false).map(|items| items_to_change(true, &items)).unwrap_or_default() } else { Vec::new() };
        match status.run_entry(*id, done, remove_downloads).await {
            SetDoneResult::Success | SetDoneResult::NoItems => {
                written += 1;
                if let Some(kind) = changed.first().map(|item| item.kind) {
                    crate::entry_core::tracking::progress::marked_items(kind, &changed);
                }
            }
            SetDoneResult::InternalError(error) => tracing::error!(%error, entry = *id, "marking the entry failed"),
        }
    }
    written
}

/// The toast after a bulk write when some entries failed (T2-BULK-02); `None` when every write succeeded.
pub fn partial_failure_toast(ok: usize, total: usize) -> Option<String> {
    (ok < total).then(|| format!("{ok} of {total} updated"))
}

/// Whether the bulk download action shows: it is hidden when any selected entry is local (T2-BULK-03).
pub fn download_visible<'a>(mut selected: impl Iterator<Item = &'a LibraryRow>) -> bool {
    !selected.any(LibraryRow::is_local)
}

pub use crate::entry_core::download_menu::DownloadAmount;
use crate::entry_core::download_menu::pick;

/// The item ids to queue for one entry. `unseen` is its next-items list in reading order. Items that are already
/// queued or downloaded (`skip`) are dropped first and the amount is taken after, so "Next 5" picks the next five
/// that are not on disk, which may lie further along the list (E-36, T2-BULK-01, T2-BULK-04).
pub fn plan_download(unseen: &[i64], amount: DownloadAmount, skip: impl Fn(i64) -> bool) -> Vec<i64> {
    pick(unseen.iter().copied().filter(|id| !skip(*id)), amount)
}

/// One entry of a cover's right-click menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverAction {
    Open,
    MarkAllViewed,
    DownloadUnviewed,
    ChangeCategories,
    Remove,
}

/// A cover's right-click menu for `kind`, in order, as (action, label, section). A local entry has nothing to
/// download, as in the selection bar (T2-BULK-03).
pub fn cover_menu(kind: Kind, local: bool) -> Vec<(CoverAction, String, u8)> {
    let (mark, download) = match kind {
        Kind::Manga => (tr(MsgKey::ui_mark_all_as_read), tr(MsgKey::ui_download_unread)),
        Kind::Anime => (tr(MsgKey::ui_mark_all_as_watched), tr(MsgKey::ui_download_unwatched)),
    };
    let mut menu = vec![(CoverAction::Open, "Open".to_owned(), 0), (CoverAction::MarkAllViewed, mark.to_owned(), 1)];
    if !local {
        menu.push((CoverAction::DownloadUnviewed, download.to_owned(), 1));
    }
    menu.push((CoverAction::ChangeCategories, tr(MsgKey::ui_change_categories).to_owned(), 2));
    menu.push((CoverAction::Remove, kuro_i18n::tr(kuro_i18n::MsgKey::remove_from_library), 3));
    menu
}

/// The two checkboxes of the remove dialog: take the entries out of the library, delete their downloads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeleteChoice {
    pub from_library: bool,
    pub downloads: bool,
}

impl DeleteChoice {
    /// OK needs at least one box checked (E-37).
    pub fn confirmable(self) -> bool {
        self.from_library || self.downloads
    }
}

/// What a confirmed remove dialog does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeletePlan {
    /// Entries whose favorite flag goes off.
    pub unfavorite: Vec<i64>,
    /// Entries whose downloads go through the download manager.
    pub delete_downloads: Vec<i64>,
}

#[cfg(test)]
/// The "Downloaded" checkbox is not offered when the selection holds a local entry (E-28, T2-DEL-04).
pub fn downloads_checkbox_offered<'a>(mut selected: impl Iterator<Item = &'a LibraryRow>) -> bool {
    !selected.any(LibraryRow::is_local)
}

/// Plans a confirmed remove for `entries` as `(id, is_local)` pairs. The two choices are independent: downloads can go
/// while the entry stays in the library, and the other way round. A local entry's folder is user data and is never
/// deleted (E-28, T2-DEL-01, T2-DEL-02, T2-DEL-04).
pub fn delete_plan(entries: &[(i64, bool)], choice: DeleteChoice) -> DeletePlan {
    let mut seen = std::collections::BTreeSet::new();
    let unique: Vec<(i64, bool)> = entries.iter().copied().filter(|(id, _)| seen.insert(*id)).collect();
    DeletePlan {
        unfavorite: if choice.from_library { unique.iter().map(|(id, _)| *id).collect() } else { Vec::new() },
        delete_downloads: if choice.downloads { unique.iter().filter(|(_, local)| !local).map(|(id, _)| *id).collect() } else { Vec::new() },
    }
}

/// What an update request covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateScope {
    Library,
    Category,
}

// Decision (T-37a-062, supersedes T-26a-113): the library's toasts go through the shared layer (`crate::toast::notify_for`)
// for `crate::library::TOAST_TIME` (4 s, U-1.8).

/// The toast an update request shows: "Updating library" or "Updating category", or the already-running notice
/// when an update is in progress (T2-UPD-01, T2-UPD-02).
pub fn update_toast(scope: UpdateScope, already_running: bool) -> String {
    use kuro_i18n::{MsgKey, tr};
    tr(match (already_running, scope) {
        (true, _) => MsgKey::update_already_running,
        (false, UpdateScope::Library) => MsgKey::updating_library,
        (false, UpdateScope::Category) => MsgKey::updating_category,
    })
}

/// What the empty library offers (T-37a-053), shown only when it has no entries, no search and no filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyLibraryAction {
    /// No source repository is set up yet: the welcome view (`view::welcome`) leads with adding one.
    AddRepository,
    /// A repository exists: the library is simply empty, so the button leads to Browse.
    Browse,
}

/// The action on the empty library page, `None` on any other empty page (a search, a filter or an empty category).
pub fn empty_library_action(page: EmptyPage, has_repositories: bool) -> Option<EmptyLibraryAction> {
    (page == EmptyPage::LibraryEmpty).then_some(if has_repositories { EmptyLibraryAction::Browse } else { EmptyLibraryAction::AddRepository })
}

/// The toast when the continue button finds nothing to open (E-25).
pub fn no_next_item() -> String {
    tr(MsgKey::no_next_chapter)
}

/// The item the continue button opens: the first pending chapter or episode of the entry's own list, after the
/// entry's filters and sort (the last pending one when sorted descending). `is_downloaded` takes the item id.
pub fn next_item(
    db: &Arc<Db>,
    row: &LibraryRow,
    downloaded_only: bool,
    is_downloaded: impl Fn(i64) -> bool,
) -> Result<Option<i64>, DomainError> {
    // the one next-unread rule, shared with the player's "resume next" (`crate::selection`)
    let entry = EntryRepo::new(db.clone()).by_id(row.id)?;
    let list = ItemRepo::new(db.clone()).by_entry_id(row.id, true)?;
    Ok(crate::selection::next_pending(&entry, list, downloaded_only, |i| is_downloaded(i.id))?.map(|i| i.id))
}

/// Minimum time between two continue clicks on the same entry (R-11).
pub const CONTINUE_DEBOUNCE: Duration = Duration::from_millis(300);

/// Whether a continue click on an entry goes through, given the time of its previous click.
pub fn continue_click_allowed(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|t| now.duration_since(t) >= CONTINUE_DEBOUNCE)
}

/// Whether the continue button shows on an entry. It depends on the raw unseen count, not the unread badge, so
/// turning the badge off does not hide the button (E-02, the fix in T-37a-033).
pub fn continue_visible(button_on: bool, row: &LibraryRow) -> bool {
    button_on && row.pending() > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cover_menu_lists_its_actions_and_skips_download_for_a_local_entry() {
        use CoverAction::{ChangeCategories, DownloadUnviewed, MarkAllViewed, Open, Remove};
        let actions = |kind, local| cover_menu(kind, local).into_iter().map(|(a, _, _)| a).collect::<Vec<_>>();
        assert_eq!(actions(Kind::Manga, false), [Open, MarkAllViewed, DownloadUnviewed, ChangeCategories, Remove]);
        assert_eq!(actions(Kind::Anime, true), [Open, MarkAllViewed, ChangeCategories, Remove]);
        assert_eq!(cover_menu(Kind::Anime, false)[1].1, "Mark all as watched");
        assert_eq!(cover_menu(Kind::Manga, false)[4].1, "Remove from library");
    }
    use std::collections::BTreeSet;

    #[test]
    fn t2_cont_03_an_excluded_scanlator_is_never_the_next_chapter() {
        // The only unread chapter belongs to an excluded scanlator, so there is nothing to continue with.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("test.db");
        let writer = kuro_db::schema_unified::open_file(&path).expect("open manga db");
        let db = Arc::new(Db::new(&path, writer).expect("manga db handle"));
        let manga_id = EntryRepo::new(db.clone())
            .insert(&Entry { favorite: true, source: 7, url: "/m".into(), title: "t".into(), ..Entry::create(Kind::Manga) })
            .expect("insert manga");
        ItemRepo::new(db.clone())
            .add_all(&[
                Item { entry_id: manga_id, url: "/1".into(), name: "1".into(), number: 1.0, done: true, ..Item::create(Kind::Manga) },
                Item {
                    entry_id: manga_id,
                    url: "/2".into(),
                    name: "2".into(),
                    number: 2.0,
                    scanlator: Some("bad".into()),
                    ..Item::create(Kind::Manga)
                },
            ])
            .expect("add chapters");
        EntryRepo::new(db.clone())
            .set_excluded_scanlators(manga_id, &BTreeSet::from(["bad".to_string()]))
            .expect("exclude scanlator");
        let entry = LibraryRow { id: manga_id, ..row(2, 1) };
        assert_eq!(next_item(&db, &entry, false, |_| false).expect("next item"), None);
    }

    fn row(total: i64, done: i64) -> LibraryRow {
        LibraryRow {
            id: 1,
            source: 7,
            title: "t".into(),
            category: 0,
            total,
            done,
            bookmarks: 0,
            fillermarks: 0,
            latest_upload: 0,
            item_fetched_at: 0,
            last_activity: 0,
            last_update: 0,
            date_added: 0,
            status: 0,
            fetch_interval: 0,
            viewer_flags: 0,
            always_update: true,
            next_update: 0,
        }
    }

    fn db() -> (tempfile::TempDir, Arc<Db>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("test.db");
        let writer = kuro_db::schema_unified::open_file(&path).expect("open manga db");
        (dir, Arc::new(Db::new(&path, writer).expect("manga db handle")))
    }

    #[test]
    fn membership_is_computed_from_every_link_so_hidden_categories_survive() {
        // category 9 is hidden: the dialog never shows it, but the entry keeps it
        let next = next_membership(&[1, 9], &[3], &[1]);
        assert_eq!(next, vec![3, 9]);
        assert_eq!(next_membership(&[1, 2], &[], &[]), vec![1, 2], "a mixed row left alone changes nothing");
        assert_eq!(next_membership(&[2], &[0], &[]), vec![2], "Default is never added");
    }

    fn manga_with_chapters(db: &Arc<Db>, flags: i64, chapters: &[(i64, bool, bool)]) -> LibraryRow {
        let id = EntryRepo::new(db.clone())
            .insert(&Entry {
                favorite: true,
                source: 7,
                url: "m".into(),
                title: "m".into(),
                item_flags: flags,
                ..Entry::create(Kind::Manga)
            })
            .expect("insert");
        db.write(|tx| {
            for (n, read, bookmark) in chapters {
                tx.execute(
                    "INSERT INTO items(entry_id, url, name, scanlator, done, bookmark, progress, number, source_order, date_fetch, date_upload) \
                     VALUES (?1, ?2, 'c', NULL, ?3, ?4, 0, ?5, ?5, 0, 0)",
                    (id, format!("c{n}"), read, bookmark, *n as f64),
                )?;
            }
            Ok(())
        })
        .expect("chapters");
        LibraryRow { id, ..row(0, 0) }
    }

    #[test]
    fn t2_bulk_02_partial_failures_report_n_of_m_and_full_success_stays_quiet() {
        assert_eq!(partial_failure_toast(2, 3).as_deref(), Some("2 of 3 updated"));
        assert_eq!(partial_failure_toast(3, 3), None);
    }

    #[tokio::test]
    async fn mark_many_counts_the_entries_written() {
        use crate::updates::download_host::fake::Fake;
        let (_dir, db) = db();
        let row = manga_with_chapters(&db, 0, &[(1, false, false)]);
        let backend = Arc::new(Fake::installed());
        assert_eq!(mark_many(&db, backend, &[row.id, row.id], true, false).await, 2);
    }

    #[tokio::test]
    async fn marking_done_changes_only_undone_items_and_is_idempotent_for_both_kinds() {
        use crate::updates::download_host::fake::Fake;
        let (_dir, db) = db();
        let id = EntryRepo::new(db.clone())
            .insert(&Entry { favorite: true, source: 7, url: "m".into(), title: "m".into(), ..Entry::create(Kind::Manga) })
            .expect("insert");
        db.write(|tx| {
            for (n, read) in [(1, true), (2, false), (3, false)] {
                tx.execute(
                    "INSERT INTO items(entry_id, url, name, scanlator, done, bookmark, progress, number, source_order, date_fetch, date_upload) \
                     VALUES (?1, ?2, 'c', NULL, ?3, 0, 0, ?4, ?4, 0, 0)",
                    (id, format!("c{n}"), read, n as f64),
                )?;
            }
            Ok(())
        })
        .expect("chapters");
        let status = set_done(&db, Arc::new(Fake::installed()));
        assert_eq!(status.run_entry(id, true, false).await, SetDoneResult::Success);
        assert_eq!(status.run_entry(id, true, false).await, SetDoneResult::NoItems);
        assert_eq!(status.run_entry(id, false, false).await, SetDoneResult::Success);
        assert_eq!(ItemRepo::new(db.clone()).by_entry_id(id, false).expect("rows").iter().filter(|i| i.done).count(), 0);
    }

    fn notified_manga(db: &Arc<Db>) -> (i64, Vec<(String, i64)>) {
        let id = EntryRepo::new(db.clone())
            .insert(&Entry { favorite: true, source: 7, url: "m".into(), title: "m".into(), ..Entry::create(Kind::Manga) })
            .expect("insert");
        db.write(|tx| {
            for n in 1..=3 {
                tx.execute(
                    "INSERT INTO items(entry_id, url, name, scanlator, done, bookmark, progress, number, source_order, date_fetch, date_upload) \
                     VALUES (?1, ?2, ?3, NULL, 0, 0, 0, ?4, ?4, 0, 0)",
                    (id, format!("u{n}"), format!("c{n}"), n as f64),
                )?;
            }
            Ok(())
        })
        .expect("chapters");
        let ids = ItemRepo::new(db.clone()).by_entry_id(id, false).expect("rows").into_iter().map(|c| (c.url, c.id)).collect();
        (id, ids)
    }

    fn id_of(ids: &[(String, i64)], url: &str) -> i64 {
        ids.iter().find(|(u, _)| u == url).map(|(_, id)| *id).expect("chapter id")
    }

    #[tokio::test]
    async fn t_26a_093_mark_read_from_a_notice_removes_the_downloads_of_the_changed_items_when_the_option_is_on() {
        use crate::updates::download_host::fake::{Call, Fake};
        let (_dir, db) = db();
        let (id, ids) = notified_manga(&db);
        let backend = Arc::new(Fake::installed());
        backend.disk.lock().unwrap().extend([1, 2]);
        let urls = ["u1".to_owned(), "u2".to_owned(), "u3".to_owned()];
        // option off: the items are marked, nothing is deleted
        assert_eq!(mark_notified_read(&db, backend.clone(), id, &urls[..1], false).await.expect("mark"), 1);
        assert!(backend.calls().is_empty(), "remove-after-read is off");
        // option on: only the changed, downloaded item goes (c1 was already read, c3 is not on disk)
        assert_eq!(mark_notified_read(&db, backend.clone(), id, &urls, true).await.expect("mark"), 2);
        assert_eq!(backend.calls(), vec![Call::Delete { entry_id: id, unit_ids: vec![id_of(&ids, "u2")] }]);
        // nothing changes the second time, so nothing is deleted again
        assert_eq!(mark_notified_read(&db, backend.clone(), id, &urls, true).await.expect("mark"), 0);
        assert_eq!(backend.calls().len(), 1);
    }

    #[test]
    fn t_26a_093_the_download_button_queues_the_named_items_with_autostart_and_a_missing_entry_is_a_no_op() {
        use crate::updates::download_host::fake::{Call, Fake};
        let (_dir, db) = db();
        let (id, ids) = notified_manga(&db);
        let backend = Fake::installed();
        let urls = ["u2".to_owned(), "u3".to_owned(), "gone".to_owned()];
        assert_eq!(download_notified(&db, &backend, Kind::Manga, id, &urls), 2);
        assert_eq!(
            backend.calls(),
            vec![
                Call::Enqueue { entry_id: id, unit_ids: vec![id_of(&ids, "u2"), id_of(&ids, "u3")] },
                Call::StartQueue,
            ],
            "autostart: the queue starts right after the enqueue"
        );
        let other = Fake::installed();
        assert_eq!(download_notified(&db, &other, Kind::Manga, 9_999, &urls), 0);
        assert!(other.calls().is_empty(), "a missing entry is a silent no-op");
    }

    #[test]
    fn the_selection_bar_download_queues_the_next_unread_items_that_are_not_queued_or_on_disk_and_starts_once() {
        use crate::updates::download_host::fake::{Call, Fake};
        use crate::updates::download_state::DownloadState;
        let (_dir, db) = db();
        let (id, ids) = notified_manga(&db);
        let backend = Fake::installed();
        assert_eq!(download_selection(&db, &backend, Kind::Manga, &[id], DownloadAmount::Next(2)), 2);
        assert_eq!(
            backend.calls(),
            // the fixture's source order is 1..=3, so the oldest chapter (the first to read) is the last one the source lists
            vec![Call::Enqueue { entry_id: id, unit_ids: vec![id_of(&ids, "u3"), id_of(&ids, "u2")] }, Call::StartQueue],
            "reading order, then one start"
        );
        let other = Fake::installed();
        other.disk.lock().unwrap().push(3);
        other.queue(id_of(&ids, "u2"), DownloadState::Queue, 0);
        assert_eq!(download_selection(&db, &other, Kind::Manga, &[id, 9_999], DownloadAmount::AllUnread), 1);
        assert_eq!(other.calls()[0], Call::Enqueue { entry_id: id, unit_ids: vec![id_of(&ids, "u1")] });
        let none = Fake::installed();
        assert_eq!(download_selection(&db, &none, Kind::Manga, &[9_999], DownloadAmount::AllUnread), 0);
        assert!(none.calls().is_empty(), "nothing to queue, nothing started");
    }

    #[test]
    fn t2_bulk_03_the_download_action_hides_when_any_selected_entry_is_local() {
        let mut local = row(3, 0);
        local.source = 0;
        assert!(download_visible([row(3, 0), row(3, 1)].iter()));
        assert!(!download_visible([row(3, 0), local].iter()));
        assert!(download_visible([].iter()));
    }

    #[test]
    fn t2_upd_01_global_and_category_update_messages() {
        assert_eq!(update_toast(UpdateScope::Library, false), "Updating library");
        assert_eq!(update_toast(UpdateScope::Category, false), "Updating category");
    }

    #[test]
    fn t_37a_053_an_empty_library_offers_a_repository_first_and_browse_once_one_exists() {
        use kuro_domain::library::search::empty_page;
        let library_empty = empty_page(false, false, true);
        assert_eq!(empty_library_action(library_empty, false), Some(EmptyLibraryAction::AddRepository));
        assert_eq!(empty_library_action(library_empty, true), Some(EmptyLibraryAction::Browse));
    }

    #[test]
    fn t_37a_053_the_empty_library_action_never_shows_for_a_search_a_filter_or_an_empty_category() {
        use kuro_domain::library::search::empty_page;
        assert_eq!(empty_library_action(empty_page(true, false, true), false), None, "a search leaves no room for it");
        assert_eq!(empty_library_action(empty_page(false, true, true), true), None, "an active filter does not");
        assert_eq!(empty_library_action(empty_page(false, false, false), false), None, "an empty category is not an empty library");
    }

    #[test]
    fn t2_upd_02_a_running_update_says_so() {
        assert_eq!(update_toast(UpdateScope::Library, true), "An update is already running");
        assert_eq!(update_toast(UpdateScope::Category, true), "An update is already running");
    }
}
