//! The database writer (T-43a-088). One thread owns the write path. Commands arrive on a channel, are coalesced
//! per chapter before the write, and run through the repositories. Nothing here runs on the GTK thread.
//!
//! Progress and bookmark writes use the item repository. Viewer flags use the entry repository's
//! `set_viewer_bits`, which merges under the mask inside one transaction.
//!
//! Reader preferences do not go through this actor: `Pref::set` updates the preference store's memory at once and
//! the store's own flusher thread saves it, so the GTK thread never waits on a file (see `settings_window`).

use kuro_i18n::{MsgKey, tr};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use kuro_data::entry::EntryRepo;
use kuro_data::history::HistoryRepo;
use kuro_data::item::{ItemRepo, ItemUpdate};
use kuro_db::Db;

use super::restore::{self, SavedSession};
use super::session::ReaderStore;

/// One write the reader asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Progress { chapter_id: i64, last_page_read: i64, read: bool },
    Bookmark { chapter_id: i64, bookmark: bool },
    ViewerBits { manga_id: i64, value: i64, mask: i64 },
    /// Marks a chapter read without touching its last page (the duplicate rule, T-43a-031).
    MarkRead { chapter_id: i64 },
    /// Adds `duration` ms to the chapter's read time and sets its last read time (T-43a-033).
    History { chapter_id: i64, read_at: i64, duration: i64 },
    /// The crash-restore record, replaced in its file (T-43a-039).
    Restore { file: PathBuf, session: SavedSession },
}

/// How long [`ReaderStore::flush`] waits for the writer before it gives up and goes on.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// What travels on the writer's channel: a write, or a barrier that is acknowledged once every write sent
/// before it has run.
enum Msg {
    Write(Command),
    /// A write whose outcome the caller waits for (a bookmark, which reverts when it fails).
    Checked(Command, Sender<Result<(), String>>),
    Flush(Sender<()>),
}

/// Coalesces one batch: per chapter only the last progress and the last bookmark are written. Viewer-bit writes,
/// read marks and history adds are kept in order, since each one merges into or adds to the previous value.
pub fn coalesce(batch: Vec<Command>) -> Vec<Command> {
    let mut progress: BTreeMap<i64, Command> = BTreeMap::new();
    let mut bookmark: BTreeMap<i64, Command> = BTreeMap::new();
    let mut bits = Vec::new();
    let mut restore: Option<Command> = None;
    for command in batch {
        match &command {
            Command::Progress { chapter_id, .. } => {
                progress.insert(*chapter_id, command);
            }
            Command::Bookmark { chapter_id, .. } => {
                bookmark.insert(*chapter_id, command);
            }
            Command::ViewerBits { .. } | Command::MarkRead { .. } | Command::History { .. } => bits.push(command),
            // only the last record of a batch matters
            Command::Restore { .. } => restore = Some(command),
        }
    }
    bits.into_iter().chain(progress.into_values()).chain(bookmark.into_values()).chain(restore).collect()
}

/// The writer's sending half, held by the reader session.
#[derive(Clone)]
pub struct WriterHandle {
    tx: Sender<Msg>,
    /// Where the crash-restore record goes. Without it `save_restore` does nothing.
    restore_file: Option<PathBuf>,
}

impl WriterHandle {
    /// Starts the writer thread over `db`. It runs until every handle is dropped.
    pub fn spawn(db: Arc<Db>) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("kuro-reader-writer".into())
            .spawn(move || run(db, rx))
            .expect("the reader writer thread starts");
        Self { tx, restore_file: None }
    }

    /// Keeps the crash-restore record in the platform data directory `data_dir` (T-43a-039).
    pub fn with_restore_in(mut self, data_dir: &std::path::Path) -> Self {
        self.restore_file = Some(restore::path_in(data_dir));
        self
    }
}

impl ReaderStore for WriterHandle {
    fn save_progress(&self, chapter_id: i64, last_page_read: i64, read: bool) {
        self.send(Command::Progress { chapter_id, last_page_read, read });
    }

    /// Waits for the writer's answer, so a failed bookmark write can revert the flag (T-43a-035). A writer that does
    /// not answer within the flush timeout counts as a failure.
    fn set_bookmark(&self, chapter_id: i64, bookmark: bool) -> Result<(), String> {
        let (reply, answer) = mpsc::channel();
        if self.tx.send(Msg::Checked(Command::Bookmark { chapter_id, bookmark }, reply)).is_err() {
            return Err(tr(MsgKey::unknown_error).to_owned());
        }
        answer.recv_timeout(FLUSH_TIMEOUT).unwrap_or_else(|_| Err(tr(MsgKey::unknown_error).to_owned()))
    }

    fn mark_read(&self, chapter_id: i64) {
        self.send(Command::MarkRead { chapter_id });
    }

    fn upsert_history(&self, chapter_id: i64, read_at: i64, duration: i64) {
        // the one writer of reading history: Incognito keeps it empty whichever screen asked
        if crate::incognito::is_on() {
            return;
        }
        self.send(Command::History { chapter_id, read_at, duration });
    }

    fn set_viewer_flags(&self, manga_id: i64, value: i64, mask: i64) {
        self.send(Command::ViewerBits { manga_id, value, mask });
    }

    fn save_restore(&self, session: SavedSession) {
        if let Some(file) = &self.restore_file {
            self.send(Command::Restore { file: file.clone(), session });
        }
    }

    /// Waits until the writer has run everything sent before this call (T-43a-025). Any later write that goes
    /// through this writer, such as the history upsert of T-43a-033, is covered the same way.
    fn flush(&self) {
        let (ack, done) = mpsc::channel();
        if self.tx.send(Msg::Flush(ack)).is_err() || done.recv_timeout(FLUSH_TIMEOUT).is_err() {
            tracing::warn!("reader writer did not confirm a flush");
        }
    }
}

impl WriterHandle {
    fn send(&self, command: Command) {
        if self.tx.send(Msg::Write(command)).is_err() {
            tracing::warn!("reader writer is gone; a progress write was dropped");
        }
    }
}

/// Splits one batch of messages into its writes, its checked writes and its flush acknowledgements.
type Split = (Vec<Command>, Vec<(Command, Sender<Result<(), String>>)>, Vec<Sender<()>>);

fn split(batch: Vec<Msg>) -> Split {
    let (mut writes, mut checked, mut acks) = (Vec::new(), Vec::new(), Vec::new());
    for msg in batch {
        match msg {
            Msg::Write(command) => writes.push(command),
            Msg::Checked(command, reply) => checked.push((command, reply)),
            Msg::Flush(ack) => acks.push(ack),
        }
    }
    (writes, checked, acks)
}

/// Runs one write through its repository.
fn execute(items: &ItemRepo, histories: &HistoryRepo, entries: &EntryRepo, command: &Command) -> Result<(), String> {
    let result = match command {
        Command::Progress { chapter_id, last_page_read, read } => {
            let mut update = ItemUpdate::new(*chapter_id);
            update.done = Some(*read);
            update.progress = Some(*last_page_read);
            items.update(&update).map_err(|e| e.to_string())
        }
        Command::Bookmark { chapter_id, bookmark } => {
            let mut update = ItemUpdate::new(*chapter_id);
            update.bookmark = Some(*bookmark);
            items.update(&update).map_err(|e| e.to_string())
        }
        Command::MarkRead { chapter_id } => {
            let mut update = ItemUpdate::new(*chapter_id);
            update.done = Some(true);
            items.update(&update).map_err(|e| e.to_string())
        }
        Command::History { chapter_id, read_at, duration } => {
            histories.upsert(*chapter_id, *read_at, *duration).map_err(|e| e.to_string())
        }
        Command::ViewerBits { manga_id, value, mask } => {
            entries.set_viewer_bits(*manga_id, *value, *mask).map(|_| ()).map_err(|e| e.to_string())
        }
        Command::Restore { file, session } => restore::save(file, session).map_err(|e| e.to_string()),
    };
    if let Err(error) = &result {
        tracing::warn!(error = %error, ?command, "reader write failed");
    }
    result
}

fn run(db: Arc<Db>, rx: Receiver<Msg>) {
    let items = ItemRepo::new(Arc::clone(&db));
    let histories = HistoryRepo::new(Arc::clone(&db));
    let entries = EntryRepo::new(db);
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        while let Ok(next) = rx.try_recv() {
            batch.push(next);
        }
        let (writes, checked, acks) = split(batch);
        for command in coalesce(writes) {
            let _ = execute(&items, &histories, &entries, &command);
        }
        for (command, reply) in checked {
            let _ = reply.send(execute(&items, &histories, &entries, &command));
        }
        for ack in acks {
            let _ = ack.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_platform::kind::Kind;

    #[test]
    fn progress_is_coalesced_per_chapter_to_the_last_write() {
        let out = coalesce(vec![
            Command::Progress { chapter_id: 1, last_page_read: 0, read: false },
            Command::Progress { chapter_id: 2, last_page_read: 4, read: false },
            Command::Progress { chapter_id: 1, last_page_read: 3, read: true },
        ]);
        assert_eq!(
            out,
            vec![
                Command::Progress { chapter_id: 1, last_page_read: 3, read: true },
                Command::Progress { chapter_id: 2, last_page_read: 4, read: false },
            ]
        );
    }

    #[test]
    fn a_batch_splits_into_its_writes_and_its_flush_acknowledgements() {
        let (ack, _done) = mpsc::channel();
        let (reply, _answer) = mpsc::channel();
        let (writes, checked, acks) = split(vec![
            Msg::Write(Command::Bookmark { chapter_id: 1, bookmark: true }),
            Msg::Flush(ack),
            Msg::Checked(Command::Bookmark { chapter_id: 3, bookmark: false }, reply),
            Msg::Write(Command::Bookmark { chapter_id: 2, bookmark: true }),
        ]);
        assert_eq!(writes.len(), 2);
        assert_eq!(checked.len(), 1);
        assert_eq!(acks.len(), 1);
    }

    #[test]
    fn history_adds_and_read_marks_are_never_coalesced() {
        let out = coalesce(vec![
            Command::History { chapter_id: 1, read_at: 10, duration: 5 },
            Command::MarkRead { chapter_id: 2 },
            Command::History { chapter_id: 1, read_at: 20, duration: 7 },
        ]);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], Command::History { chapter_id: 1, read_at: 10, duration: 5 });
        assert_eq!(out[2], Command::History { chapter_id: 1, read_at: 20, duration: 7 });
    }

    #[test]
    fn flush_returns_only_after_the_earlier_writes_are_in_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = kuro_db::schema_unified::db_path(dir.path());
        let writer = kuro_db::schema_unified::open_file(&path).unwrap();
        let db = Arc::new(Db::new(&path, writer).unwrap());
        let manga_id = EntryRepo::new(Arc::clone(&db)).insert(&kuro_data::entry::Entry::create(Kind::Manga)).unwrap();
        let mut chapter = kuro_data::item::Item::create(Kind::Manga);
        chapter.entry_id = manga_id;
        chapter.url = "/c/1".into();
        let chapter_id = ItemRepo::new(Arc::clone(&db)).add_all(&[chapter]).unwrap()[0].id;

        let handle = WriterHandle::spawn(Arc::clone(&db));
        handle.save_progress(chapter_id, 4, true);
        handle.flush();
        let stored = ItemRepo::new(db).by_id(chapter_id).unwrap().unwrap();
        assert_eq!((stored.progress, stored.done), (4, true));
    }

    #[test]
    fn only_the_last_restore_record_of_a_batch_is_written() {
        let file = PathBuf::from("/data/reader_session.json");
        let at = |page_index| SavedSession { manga_id: 1, chapter_id: 2, page_index, saved_at: 5 };
        let out = coalesce(vec![
            Command::Restore { file: file.clone(), session: at(1) },
            Command::Progress { chapter_id: 2, last_page_read: 1, read: false },
            Command::Restore { file: file.clone(), session: at(4) },
        ]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1], Command::Restore { file, session: at(4) });
    }

    #[test]
    fn the_handle_writes_the_restore_record_to_its_file_and_a_plain_handle_writes_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = kuro_db::schema_unified::db_path(dir.path());
        let writer = kuro_db::schema_unified::open_file(&path).unwrap();
        let db = Arc::new(Db::new(&path, writer).unwrap());
        let record = SavedSession { manga_id: 1, chapter_id: 2, page_index: 3, saved_at: 4 };

        let plain = WriterHandle::spawn(Arc::clone(&db));
        plain.save_restore(record);
        plain.flush();
        assert_eq!(restore::load(&restore::path_in(dir.path())), None);

        let keeping = WriterHandle::spawn(db).with_restore_in(dir.path());
        keeping.save_restore(record);
        keeping.flush();
        assert_eq!(restore::load(&restore::path_in(dir.path())), Some(record));
    }

    #[test]
    fn viewer_bit_writes_keep_their_order_and_run_before_progress() {
        let out = coalesce(vec![
            Command::ViewerBits { manga_id: 9, value: 2, mask: 0x7 },
            Command::Progress { chapter_id: 1, last_page_read: 1, read: false },
            Command::ViewerBits { manga_id: 9, value: 0x10, mask: 0x38 },
        ]);
        assert_eq!(out[0], Command::ViewerBits { manga_id: 9, value: 2, mask: 0x7 });
        assert_eq!(out[1], Command::ViewerBits { manga_id: 9, value: 0x10, mask: 0x38 });
        assert_eq!(out.len(), 3);
    }
}
