//! Change notification. The writer's hooks collect the tables a write touched (triggers and foreign
//! key cascades included), and the write publishes them once its commit has returned. The set is
//! carried by table name; `Tables` names the tables of the unified file as bits.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rusqlite::functions::FunctionFlags;
use rusqlite::hooks::Action;
use rusqlite::Connection;

use crate::error::DbResult;

/// Names of the tables one committed write touched, in name order.
pub type Changed = Arc<BTreeSet<String>>;

/// The tables of the unified file, in the order of the [`Tables`] bits. Exported as `kuro_db::DB_TABLES`.
pub const TABLES: &[&str] = &[
    "entries",
    "items",
    "history",
    "categories",
    "entry_categories",
    "excluded_scanlators",
    "tracks",
    "stub_sources",
    "anime_relations",
    "download_queue",
    "download_pending_delete",
    "download_pending_corrupt",
    "track_retry_queue",
];

/// A set of tables, one bit per table in [`TABLES`] order.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Tables(pub u16);

impl Tables {
    pub const NONE: Tables = Tables(0);
    pub const ENTRIES: Tables = Tables(1);
    pub const ITEMS: Tables = Tables(2);
    pub const HISTORY: Tables = Tables(4);
    pub const CATEGORIES: Tables = Tables(8);
    pub const ENTRY_CATEGORIES: Tables = Tables(16);
    pub const EXCLUDED_SCANLATORS: Tables = Tables(32);
    pub const TRACKS: Tables = Tables(64);
    pub const STUB_SOURCES: Tables = Tables(128);
    pub const ANIME_RELATIONS: Tables = Tables(256);
    pub const DOWNLOAD_QUEUE: Tables = Tables(512);
    pub const DOWNLOAD_PENDING_DELETE: Tables = Tables(1024);
    pub const DOWNLOAD_PENDING_CORRUPT: Tables = Tables(2048);
    pub const TRACK_RETRY_QUEUE: Tables = Tables(4096);

    /// The table names of this set, in bit order.
    pub fn names(self) -> Vec<&'static str> {
        TABLES
            .iter()
            .enumerate()
            .filter(|(bit, _)| self.0 & (1 << bit) != 0)
            .map(|(_, name)| *name)
            .collect()
    }

    /// The bit of a table name; `NONE` for any other name.
    pub fn from_name(name: &str) -> Tables {
        TABLES.iter().position(|t| *t == name).map_or(Tables::NONE, |bit| Tables(1 << bit))
    }

    /// The set of the tables named in `changed`.
    pub fn of_changed(changed: &BTreeSet<String>) -> Tables {
        changed.iter().fold(Tables::NONE, |acc, name| acc | Tables::from_name(name))
    }

    pub const fn intersects(self, other: Tables) -> bool {
        self.0 & other.0 != 0
    }
}

impl std::ops::BitOr for Tables {
    type Output = Tables;
    fn bitor(self, rhs: Tables) -> Tables {
        Tables(self.0 | rhs.0)
    }
}

const fn of(parts: &[Tables]) -> Tables {
    let mut bits = 0;
    let mut i = 0;
    while i < parts.len() {
        bits |= parts[i].0;
        i += 1;
    }
    Tables(bits)
}

// Dependency sets of the live queries. A query subscribes to the tables it reads.
pub const ENTRY_DEPS: Tables = Tables::ENTRIES;
pub const LIBRARY_DEPS: Tables =
    of(&[Tables::ENTRIES, Tables::ITEMS, Tables::EXCLUDED_SCANLATORS, Tables::HISTORY, Tables::ENTRY_CATEGORIES]);
pub const CATEGORIES_DEPS: Tables = Tables::CATEGORIES;
pub const CATEGORIES_OF_ENTRY_DEPS: Tables = of(&[Tables::CATEGORIES, Tables::ENTRY_CATEGORIES]);
pub const ITEMS_OF_ENTRY_DEPS: Tables = of(&[Tables::ITEMS, Tables::EXCLUDED_SCANLATORS]);
pub const SCANLATORS_DEPS: Tables = Tables::ITEMS;
pub const HISTORY_DEPS: Tables = of(&[Tables::ENTRIES, Tables::ITEMS, Tables::HISTORY]);
pub const EXCLUDED_SCANLATORS_DEPS: Tables = Tables::EXCLUDED_SCANLATORS;
pub const TRACKS_DEPS: Tables = Tables::TRACKS;
pub const UPDATES_DEPS: Tables = of(&[Tables::ENTRIES, Tables::ITEMS]);
pub const STUB_SOURCES_DEPS: Tables = Tables::STUB_SOURCES;
pub const SOURCE_COUNTS_DEPS: Tables = Tables::ENTRIES;

/// What the writer's hooks collect. `open` holds the names of the write in progress; `staged` holds
/// the names of commits whose publication is still pending.
#[derive(Default)]
pub(crate) struct Pending {
    open: Mutex<BTreeSet<String>>,
    staged: Mutex<BTreeSet<String>>,
}

/// Locks, recovering from poisoning: the hooks never leave the sets half-updated.
fn lock(m: &Mutex<BTreeSet<String>>) -> MutexGuard<'_, BTreeSet<String>> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Pending {
    /// Forgets what an earlier write touched before this one starts.
    pub(crate) fn begin(&self) {
        lock(&self.open).clear();
    }

    /// The names of the commit that just returned, or `None` when it touched no table.
    pub(crate) fn publish(&self) -> Option<Changed> {
        let names = std::mem::take(&mut *lock(&self.staged));
        (!names.is_empty()).then(|| Arc::new(names))
    }
}

/// Installs the update, commit and rollback hooks on the writer. Readers never write, so they get
/// none. The hooks only touch the sets: no SQLite call happens inside them.
pub(crate) fn install(writer: &Connection) -> DbResult<Arc<Pending>> {
    let pending = Arc::new(Pending::default());
    let (on_update, on_commit, on_rollback, on_touch) =
        (Arc::clone(&pending), Arc::clone(&pending), Arc::clone(&pending), Arc::clone(&pending));
    // `TxExt::touch`: names a table in the open write although SQLite reported no row change for it.
    writer.create_scalar_function("kuro_touch", 1, FunctionFlags::SQLITE_UTF8, move |ctx| {
        let table: String = ctx.get(0)?;
        lock(&on_touch.open).insert(table);
        Ok(1_i64)
    })?;
    writer.update_hook(Some(move |_: Action, _db: &str, table: &str, _rowid: i64| {
        let mut open = lock(&on_update.open);
        if !open.contains(table) {
            open.insert(table.to_owned());
        }
    }))?;
    // Returning false lets the commit proceed. The names move to `staged` and are published by
    // `Db::write` after the commit returns, so a subscriber never re-reads before the data is visible.
    writer.commit_hook(Some(move || {
        let taken = std::mem::take(&mut *lock(&on_commit.open));
        lock(&on_commit.staged).extend(taken);
        false
    }))?;
    writer.rollback_hook(Some(move || {
        lock(&on_rollback.open).clear();
        lock(&on_rollback.staged).clear();
    }))?;
    Ok(pending)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thirteen_bits_follow_the_table_order() {
        assert_eq!(TABLES.len(), 13);
        assert_eq!(Tables::ENTRIES.0, 1);
        assert_eq!(Tables::TRACK_RETRY_QUEUE.0, 4096);
        assert_eq!(Tables::from_name("items"), Tables::ITEMS);
        assert_eq!(Tables::from_name("tracks"), Tables::TRACKS);
        assert_eq!(Tables::from_name("mangas"), Tables::NONE, "old table names are no bits");
        assert_eq!((Tables::ENTRIES | Tables::ITEMS).names(), ["entries", "items"]);
    }

    #[test]
    fn dependency_sets_are_the_plan_table() {
        assert_eq!(LIBRARY_DEPS.names(), ["entries", "items", "history", "entry_categories", "excluded_scanlators"]);
        assert_eq!(HISTORY_DEPS.names(), ["entries", "items", "history"]);
        assert_eq!(UPDATES_DEPS.names(), ["entries", "items"]);
        assert_eq!(CATEGORIES_OF_ENTRY_DEPS.names(), ["categories", "entry_categories"]);
        assert_eq!(SCANLATORS_DEPS, Tables::ITEMS);
    }

    #[test]
    fn of_changed_maps_names_and_ignores_unknown_ones() {
        let names: BTreeSet<String> = ["items", "entries", "sqlite_sequence"].map(String::from).into();
        assert_eq!(Tables::of_changed(&names), Tables::ITEMS | Tables::ENTRIES);
    }
}
