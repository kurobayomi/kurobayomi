//! Entries of both kinds (docs/design/unified-model.md section 1.3): one `Entry`, one `EntryUpdate`, one library and
//! season row, and one `EntryRepo` over `kuro_db::tables`. The kind is a field of the entry or a parameter of the
//! call. `item_flags` holds the unified layout (`kuro_db::flags::item`); the legacy `Manga` wrapper translates.

use std::collections::BTreeSet;
use std::sync::Arc;

use kuro_db::codecs::{GenreList, UpdateStrategy};
use kuro_db::handle::Db;
use kuro_db::codec::{FetchType, Memo};
use kuro_db::tables::excluded;
use kuro_db::subscribe::Subscription;
use kuro_db::tables::entries::{self as t, FlagColumn};
use kuro_db::tables::{self, DeletableRow, EntryPatch, EntryRow, LibraryRow, SeasonRow};
use kuro_platform::kind::Kind;
use rusqlite::Connection;
use serde_json::{Map, Value};

use crate::time::{self, Clock};
use crate::{
    DataResult, ENTRY_DEBOUNCE, ID_CHUNK, LIBRARY_DEBOUNCE, LIBRARY_RETRIES, LIBRARY_RETRY_DELAY, LibraryState, LookupOrInsert,
    none_if_not_found, retrying,
};

/// Tables of one entry row.
const ENTRY_TABLE: &[&str] = &["entries"];
const ITEM_TABLE: &[&str] = &["items"];
const EXCLUDED_TABLE: &[&str] = &["excluded_scanlators"];
/// Tables a `libraryView` row or a season row reads.
const LIBRARY_TABLES: &[&str] = &["entries", "items", "entry_categories", "excluded_scanlators", "history"];

/// One `entries` row. The anime-only fields are `None` for manga (the table's two `CHECK`s).
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// `-1` until stored.
    pub id: i64,
    pub kind: Kind,
    pub source: i64,
    pub url: String,
    pub title: String,
    pub artist: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub genre: Option<Vec<String>>,
    pub status: i64,
    pub thumbnail_url: Option<String>,
    pub favorite: bool,
    pub last_update: i64,
    pub next_update: i64,
    pub fetch_interval: i32,
    pub date_added: i64,
    /// Kind-native bits (reader modes, intro and airing), not translated.
    pub viewer_flags: i64,
    /// The unified layout of `kuro_db::flags::item`.
    pub item_flags: i64,
    pub cover_last_modified: i64,
    pub update_strategy: UpdateStrategy,
    pub initialized: bool,
    pub last_modified_at: i64,
    pub favorite_modified_at: Option<i64>,
    pub version: i64,
    pub fetch_type: Option<FetchType>,
    pub parent_id: Option<i64>,
    pub season_flags: Option<i64>,
    pub season_number: Option<f64>,
    pub season_source_order: Option<i64>,
    pub background_url: Option<String>,
    pub background_last_modified: Option<i64>,
    pub memo: Option<Memo>,
}

impl Entry {
    /// `Manga.create()` / `Anime.create()`: id -1, source -1, version 0, `ALWAYS_UPDATE`. Anime also gets
    /// `Episodes`, season flags 0, season number -1.0, order 0, background 0 and an empty memo; manga keeps them `None`.
    pub fn create(kind: Kind) -> Self {
        let anime = kind.is_anime();
        Self {
            id: -1,
            kind,
            source: -1,
            url: String::new(),
            title: String::new(),
            artist: None,
            author: None,
            description: None,
            genre: None,
            status: 0,
            thumbnail_url: None,
            favorite: false,
            last_update: 0,
            next_update: 0,
            fetch_interval: 0,
            date_added: 0,
            viewer_flags: 0,
            item_flags: 0,
            cover_last_modified: 0,
            update_strategy: UpdateStrategy::AlwaysUpdate,
            initialized: false,
            last_modified_at: 0,
            favorite_modified_at: None,
            version: 0,
            fetch_type: anime.then_some(FetchType::Episodes),
            parent_id: None,
            season_flags: anime.then_some(0),
            season_number: anime.then_some(-1.0),
            season_source_order: anime.then_some(0),
            background_url: None,
            background_last_modified: anime.then_some(0),
            memo: anime.then(Memo::default),
        }
    }

    /// `Episodes` for manga.
    pub fn fetch_type(&self) -> FetchType {
        self.fetch_type.unwrap_or(FetchType::Episodes)
    }

    /// 0 for manga.
    pub fn season_flags(&self) -> i64 {
        self.season_flags.unwrap_or(0)
    }

    /// -1.0 for manga.
    pub fn season_number(&self) -> f64 {
        self.season_number.unwrap_or(-1.0)
    }

    /// 0 for manga.
    pub fn season_source_order(&self) -> i64 {
        self.season_source_order.unwrap_or(0)
    }

    /// 0 for manga.
    pub fn background_last_modified(&self) -> i64 {
        self.background_last_modified.unwrap_or(0)
    }

    /// Empty for manga.
    pub fn memo(&self) -> Map<String, Value> {
        self.memo.as_ref().map(|m| m.0.clone()).unwrap_or_default()
    }
}

impl From<EntryRow> for Entry {
    fn from(r: EntryRow) -> Self {
        Self {
            id: r.id,
            kind: r.kind,
            source: r.source,
            url: r.url,
            title: r.title,
            artist: r.artist,
            author: r.author,
            description: r.description,
            genre: r.genre.map(|g| g.0),
            status: r.status,
            thumbnail_url: r.thumbnail_url,
            favorite: r.favorite,
            last_update: r.last_update.unwrap_or(0),
            next_update: r.next_update.unwrap_or(0),
            fetch_interval: r.calculate_interval as i32,
            date_added: r.date_added,
            viewer_flags: r.viewer,
            item_flags: r.item_flags,
            cover_last_modified: r.cover_last_modified,
            update_strategy: r.update_strategy,
            initialized: r.initialized,
            last_modified_at: r.last_modified_at,
            favorite_modified_at: r.favorite_modified_at,
            version: r.version,
            fetch_type: r.fetch_type,
            parent_id: r.parent_id,
            season_flags: r.season_flags,
            season_number: r.season_number,
            season_source_order: r.season_source_order,
            background_url: r.background_url,
            background_last_modified: r.background_last_modified,
            memo: r.memo,
        }
    }
}

/// The insert row. The statement binds neither `id`, `last_modified_at` nor `favorite_modified_at`.
impl From<&Entry> for EntryRow {
    fn from(e: &Entry) -> Self {
        Self {
            id: e.id,
            kind: e.kind,
            source: e.source,
            url: e.url.clone(),
            title: e.title.clone(),
            artist: e.artist.clone(),
            author: e.author.clone(),
            description: e.description.clone(),
            genre: e.genre.clone().map(GenreList),
            status: e.status,
            thumbnail_url: e.thumbnail_url.clone(),
            favorite: e.favorite,
            last_update: Some(e.last_update),
            next_update: Some(e.next_update),
            initialized: e.initialized,
            viewer: e.viewer_flags,
            item_flags: e.item_flags,
            cover_last_modified: e.cover_last_modified,
            date_added: e.date_added,
            update_strategy: e.update_strategy,
            calculate_interval: i64::from(e.fetch_interval),
            last_modified_at: 0,
            favorite_modified_at: None,
            version: e.version,
            fetch_type: e.fetch_type,
            parent_id: e.parent_id,
            season_flags: e.season_flags,
            season_number: e.season_number,
            season_source_order: e.season_source_order,
            background_url: e.background_url.clone(),
            background_last_modified: e.background_last_modified,
            memo: e.memo.clone(),
        }
    }
}

/// Partial update: `None` keeps the stored value, so a nullable column cannot be cleared through this type. An
/// anime-only field set on a manga entry fails the table's `CHECK` (`DataError::Constraint` or a SQLite constraint).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntryUpdate {
    pub id: i64,
    pub source: Option<i64>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub genre: Option<Vec<String>>,
    pub status: Option<i64>,
    pub thumbnail_url: Option<String>,
    pub favorite: Option<bool>,
    pub last_update: Option<i64>,
    pub next_update: Option<i64>,
    pub fetch_interval: Option<i32>,
    pub date_added: Option<i64>,
    pub viewer_flags: Option<i64>,
    pub item_flags: Option<i64>,
    pub cover_last_modified: Option<i64>,
    pub update_strategy: Option<UpdateStrategy>,
    pub initialized: Option<bool>,
    pub version: Option<i64>,
    pub fetch_type: Option<FetchType>,
    pub parent_id: Option<i64>,
    pub season_flags: Option<i64>,
    pub season_number: Option<f64>,
    pub season_source_order: Option<i64>,
    pub background_url: Option<String>,
    pub background_last_modified: Option<i64>,
    pub memo: Option<Memo>,
}

impl EntryUpdate {
    pub fn new(id: i64) -> Self {
        Self { id, ..Default::default() }
    }
}

impl From<&EntryUpdate> for EntryPatch {
    fn from(u: &EntryUpdate) -> Self {
        Self {
            id: u.id,
            source: u.source,
            url: u.url.clone(),
            title: u.title.clone(),
            artist: u.artist.clone(),
            author: u.author.clone(),
            description: u.description.clone(),
            genre: u.genre.clone().map(GenreList),
            status: u.status,
            thumbnail_url: u.thumbnail_url.clone(),
            favorite: u.favorite,
            last_update: u.last_update,
            next_update: u.next_update,
            initialized: u.initialized,
            viewer: u.viewer_flags,
            item_flags: u.item_flags,
            cover_last_modified: u.cover_last_modified,
            date_added: u.date_added,
            update_strategy: u.update_strategy,
            calculate_interval: u.fetch_interval.map(i64::from),
            version: u.version,
            fetch_type: u.fetch_type,
            parent_id: u.parent_id,
            season_flags: u.season_flags,
            season_number: u.season_number,
            season_source_order: u.season_source_order,
            background_url: u.background_url.clone(),
            background_last_modified: u.background_last_modified,
            memo: u.memo.clone(),
        }
    }
}

/// One `libraryView` row: the entry, one category it is in (0 when none) and the item aggregates.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryEntry {
    pub entry: Entry,
    pub category: i64,
    pub total: i64,
    pub done: i64,
    pub bookmarks: i64,
    pub fillermarks: i64,
    pub latest_upload: i64,
    pub item_fetched_at: i64,
    pub last_activity: i64,
}

impl LibraryEntry {
    pub fn id(&self) -> i64 {
        self.entry.id
    }

    pub fn pending(&self) -> i64 {
        self.total - self.done
    }

    pub fn has_bookmarks(&self) -> bool {
        self.bookmarks > 0
    }

    pub fn has_started(&self) -> bool {
        self.done > 0
    }
}

impl From<LibraryRow> for LibraryEntry {
    fn from(r: LibraryRow) -> Self {
        Self {
            entry: r.entry.into(),
            category: r.category,
            total: r.total,
            done: r.done,
            bookmarks: r.bookmarks,
            fillermarks: r.fillermarks,
            latest_upload: r.latest_upload,
            item_fetched_at: r.item_fetched_at,
            last_activity: r.last_activity,
        }
    }
}

/// A child of a `Seasons` parent with its item aggregates (no category).
#[derive(Clone, Debug, PartialEq)]
pub struct SeasonEntry {
    pub entry: Entry,
    pub total: i64,
    pub done: i64,
    pub bookmarks: i64,
    pub fillermarks: i64,
    pub latest_upload: i64,
    pub fetched_at: i64,
    pub last_activity: i64,
}

impl SeasonEntry {
    pub fn is_done(&self) -> bool {
        self.total == self.done
    }

    pub fn pending(&self) -> i64 {
        self.total - self.done
    }

    /// Category -1 and the fetch date carried over.
    pub fn to_library_entry(&self) -> LibraryEntry {
        LibraryEntry {
            entry: self.entry.clone(),
            category: -1,
            total: self.total,
            done: self.done,
            bookmarks: self.bookmarks,
            fillermarks: self.fillermarks,
            latest_upload: self.latest_upload,
            item_fetched_at: self.fetched_at,
            last_activity: self.last_activity,
        }
    }
}

impl From<SeasonRow> for SeasonEntry {
    fn from(r: SeasonRow) -> Self {
        Self {
            entry: r.entry.into(),
            total: r.total,
            done: r.done,
            bookmarks: r.bookmarks,
            fillermarks: r.fillermarks,
            latest_upload: r.latest_upload,
            fetched_at: r.fetched_at,
            last_activity: r.last_activity,
        }
    }
}

/// A non-favourite top-level entry: a candidate of the clear-database screen. `fetch_type` is `Episodes` for manga.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeletableEntry {
    pub entry_id: i64,
    pub source_id: i64,
    pub kind: Kind,
    pub fetch_type: FetchType,
}

impl From<DeletableRow> for DeletableEntry {
    fn from(r: DeletableRow) -> Self {
        Self { entry_id: r.entry_id, source_id: r.source_id, kind: r.kind, fetch_type: r.fetch_type }
    }
}

fn entries(rows: Vec<EntryRow>) -> Vec<Entry> {
    rows.into_iter().map(Entry::from).collect()
}

/// The non-blank scanlators of a list (`GetAvailableScanlators` drops blank names).
fn available(scanlators: Vec<String>) -> BTreeSet<String> {
    scanlators.into_iter().filter(|s| !s.trim().is_empty()).collect()
}

fn deletable(c: &Connection, kind: Kind) -> rusqlite::Result<Vec<DeletableEntry>> {
    Ok(t::deletable_parents(c)?.into_iter().filter(|r| r.kind == kind).map(Into::into).collect())
}

/// The non-library entries of `kind` as `(source, entry id)` rows: `removable` holds each deletable parent and the ids
/// its clear removes, `orphaned` the favourite children that lose their parent. The children are searched only under
/// `Seasons` parents, as the clear screen does.
pub(crate) fn non_library_tree(c: &Connection, kind: Kind) -> rusqlite::Result<(Vec<(i64, i64)>, Vec<(i64, i64)>)> {
    let mut removable = Vec::new();
    let mut orphaned = Vec::new();
    for parent in deletable(c, kind)? {
        let (ids, gone) = if parent.fetch_type == FetchType::Seasons {
            t::clearable_tree(c, parent.entry_id)?
        } else {
            (vec![parent.entry_id], Vec::new())
        };
        removable.extend(ids.into_iter().map(|id| (parent.source_id, id)));
        orphaned.extend(gone.into_iter().map(|id| (parent.source_id, id)));
    }
    Ok((removable, orphaned))
}

/// Typed access to `entries` for both kinds. Shares one `Db` with the other repositories of the database. Every
/// subscription wakes on a write of either kind; the subscription layer publishes only a changed result.
#[derive(Clone)]
pub struct EntryRepo {
    db: Arc<Db>,
}

impl EntryRepo {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }

    // ----- reads -----

    /// `NotFound` when no row has `id`.
    pub fn by_id(&self, id: i64) -> DataResult<Entry> {
        Ok(self.db.read(|c| t::by_id(c, id))?.into())
    }

    /// The first row (lowest `_id`) with this url and source of the kind.
    pub fn by_url_and_source(&self, kind: Kind, url: &str, source: i64) -> DataResult<Option<Entry>> {
        Ok(self.db.read(|c| Ok(t::by_url_and_source(c, kind, url, source)?))?.map(Entry::from))
    }

    pub fn favorites(&self, kind: Kind) -> DataResult<Vec<Entry>> {
        Ok(entries(self.db.read(|c| Ok(t::favorites(c, kind)?))?))
    }

    pub fn favorites_by_source(&self, kind: Kind, source: i64) -> DataResult<Vec<Entry>> {
        Ok(entries(self.db.read(|c| Ok(t::favorites_by_source(c, kind, source)?))?))
    }

    /// Every entry of a source, in the library or not.
    pub fn by_source(&self, kind: Kind, source: i64) -> DataResult<Vec<Entry>> {
        Ok(entries(self.db.read(|c| Ok(t::by_source(c, kind, source)?))?))
    }

    /// Non-favourites with a done item or some progress.
    pub fn watched_not_in_library(&self, kind: Kind) -> DataResult<Vec<Entry>> {
        Ok(entries(self.db.read(|c| Ok(t::watched_not_in_library(c, kind)?))?))
    }

    /// One row per (favourite entry, category link); an entry with no link appears once with category 0.
    pub fn library(&self, kind: Kind) -> DataResult<Vec<LibraryEntry>> {
        let rows = self.db.read(|c| Ok(tables::library::library(c, kind)?))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// The other library entries of the kind whose title equals `title`, both lowercased (the stored side through
    /// `kuro_lower`). Non-favourites never match.
    pub fn duplicates(&self, kind: Kind, id: i64, title: &str) -> DataResult<Vec<Entry>> {
        Ok(entries(self.db.read(|c| Ok(t::duplicates(c, kind, id, title)?))?))
    }

    /// Favourites with `next_update` at or after local midnight of `now_ms`'s day, soonest first. The statuses are
    /// `kind.upcoming_statuses()`.
    pub fn upcoming(&self, kind: Kind, now_ms: i64) -> DataResult<Vec<Entry>> {
        let start_of_day = time::start_of_local_day_millis(now_ms);
        Ok(entries(self.db.read(|c| Ok(t::upcoming(c, kind, start_of_day, kind.upcoming_statuses())?))?))
    }

    /// `(source, url)` of every entry of the kind, library or not.
    pub fn all_source_and_url(&self, kind: Kind) -> DataResult<Vec<(i64, String)>> {
        Ok(self.db.read(|c| Ok(t::all_source_and_url(c, kind)?))?)
    }

    /// `(source, favourite count)` for every source with a favourite, ordered by source.
    pub fn favorite_counts_by_source(&self, kind: Kind) -> DataResult<Vec<(i64, i64)>> {
        Ok(self.db.read(|c| Ok(t::favorite_counts(c, kind)?))?)
    }

    /// `(source, non-library count)` for every source with a non-favourite, ordered by source.
    pub fn non_library_counts_by_source(&self, kind: Kind) -> DataResult<Vec<(i64, i64)>> {
        Ok(self.db.read(|c| Ok(t::non_library_counts(c, kind)?))?)
    }

    /// Season rows of one `Seasons` parent, non-favourite children included.
    pub fn seasons_by_parent(&self, parent_id: i64) -> DataResult<Vec<SeasonEntry>> {
        let rows = self.db.read(|c| Ok(t::seasons_of(c, parent_id)?))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Every row whose `parent_id` is `parent_id`, favourite or not.
    pub fn children_by_parent(&self, parent_id: i64) -> DataResult<Vec<Entry>> {
        Ok(entries(self.db.read(|c| Ok(t::children_of(c, parent_id)?))?))
    }

    /// Non-favourite top-level entries of the kind, the candidates of the clear-database screen.
    pub fn deletable_parents(&self, kind: Kind) -> DataResult<Vec<DeletableEntry>> {
        Ok(self.db.read(|c| Ok(deletable(c, kind)?))?)
    }

    pub fn excluded_scanlators(&self, id: i64) -> DataResult<BTreeSet<String>> {
        self.db.read(|c| Ok(excluded::by_entry_id(c, id)?.into_iter().collect()))
    }

    /// The distinct, non-blank scanlators of an entry's items.
    pub fn available_scanlators(&self, id: i64) -> DataResult<BTreeSet<String>> {
        self.db.read(|c| Ok(available(tables::items::scanlators(c, id)?)))
    }

    // ----- live queries -----

    /// The row with `id`, published again after each change. `None` once the row is deleted.
    pub fn subscribe_by_id(&self, id: i64) -> Subscription<Option<Entry>> {
        self.db.subscribe(ENTRY_TABLE, ENTRY_DEBOUNCE, move |c| none_if_not_found(t::by_id(c, id).map(Entry::from)))
    }

    pub fn subscribe_by_url_and_source(&self, kind: Kind, url: &str, source: i64) -> Subscription<Option<Entry>> {
        let url = url.to_owned();
        self.db.subscribe(ENTRY_TABLE, ENTRY_DEBOUNCE, move |c| Ok(t::by_url_and_source(c, kind, &url, source)?.map(Entry::from)))
    }

    pub fn subscribe_favorites_by_source(&self, kind: Kind, source: i64) -> Subscription<Vec<Entry>> {
        self.db.subscribe(ENTRY_TABLE, ENTRY_DEBOUNCE, move |c| {
            Ok(t::favorites_by_source(c, kind, source)?.into_iter().map(Entry::from).collect::<Vec<_>>())
        })
    }

    /// The `library()` rows, published again after a change to any table they read. A transient lock is retried (see
    /// `retrying`); after the last retry the error text is published instead of the rows.
    pub fn subscribe_library(&self, kind: Kind) -> Subscription<LibraryState<LibraryEntry>> {
        self.db.subscribe(
            LIBRARY_TABLES,
            LIBRARY_DEBOUNCE,
            retrying(LIBRARY_RETRIES, LIBRARY_RETRY_DELAY, move |c| {
                Ok(tables::library::library(c, kind)?.into_iter().map(LibraryEntry::from).collect::<Vec<_>>())
            }),
        )
    }

    pub fn subscribe_seasons_by_parent(&self, parent_id: i64) -> Subscription<Vec<SeasonEntry>> {
        self.db.subscribe(LIBRARY_TABLES, LIBRARY_DEBOUNCE, move |c| {
            Ok(t::seasons_of(c, parent_id)?.into_iter().map(SeasonEntry::from).collect::<Vec<_>>())
        })
    }

    pub fn subscribe_upcoming(&self, kind: Kind) -> Subscription<Vec<Entry>> {
        self.subscribe_upcoming_with_clock(kind, Arc::new(time::SystemClock))
    }

    /// Favourites due from local midnight of the day `clock` reads, soonest first, published again after a change and
    /// at each local midnight. The cut-off is recomputed on every run.
    pub fn subscribe_upcoming_with_clock(&self, kind: Kind, clock: Arc<dyn Clock>) -> Subscription<Vec<Entry>> {
        let timer = Arc::clone(&clock);
        self.db.subscribe_timed(
            ENTRY_TABLE,
            ENTRY_DEBOUNCE,
            move || time::until_next_local_midnight(timer.now_millis()),
            move |c| {
                let start_of_day = time::start_of_local_day_millis(clock.now_millis());
                Ok(t::upcoming(c, kind, start_of_day, kind.upcoming_statuses())?.into_iter().map(Entry::from).collect::<Vec<_>>())
            },
        )
    }

    pub fn subscribe_favorite_counts(&self, kind: Kind) -> Subscription<Vec<(i64, i64)>> {
        self.db.subscribe(ENTRY_TABLE, ENTRY_DEBOUNCE, move |c| Ok(t::favorite_counts(c, kind)?))
    }

    pub fn subscribe_non_library_counts(&self, kind: Kind) -> Subscription<Vec<(i64, i64)>> {
        self.db.subscribe(ENTRY_TABLE, ENTRY_DEBOUNCE, move |c| Ok(t::non_library_counts(c, kind)?))
    }

    /// `deletable_parents`, published again after a change to an entry, its items or its history.
    pub fn subscribe_deletable_parents(&self, kind: Kind) -> Subscription<Vec<DeletableEntry>> {
        self.db.subscribe(LIBRARY_TABLES, LIBRARY_DEBOUNCE, move |c| Ok(deletable(c, kind)?))
    }

    /// `(source, entry id)` rows of the non-library entries and of the orphaned children (`non_library_tree`).
    pub fn subscribe_non_library_tree(&self, kind: Kind) -> Subscription<(Vec<(i64, i64)>, Vec<(i64, i64)>)> {
        self.db.subscribe(LIBRARY_TABLES, LIBRARY_DEBOUNCE, move |c| Ok(non_library_tree(c, kind)?))
    }

    pub fn subscribe_excluded_scanlators(&self, id: i64) -> Subscription<BTreeSet<String>> {
        self.db.subscribe(EXCLUDED_TABLE, ENTRY_DEBOUNCE, move |c| Ok(excluded::by_entry_id(c, id)?.into_iter().collect()))
    }

    /// The available scanlators, published again after each change of the items.
    pub fn subscribe_available_scanlators(&self, id: i64) -> Subscription<BTreeSet<String>> {
        self.db.subscribe(ITEM_TABLE, ENTRY_DEBOUNCE, move |c| Ok(available(tables::items::scanlators(c, id)?)))
    }

    // ----- writes -----

    /// Inserts `e` and returns the new id. A manga entry must carry `None` in every anime-only field.
    pub fn insert(&self, e: &Entry) -> DataResult<i64> {
        let row = EntryRow::from(e);
        self.db.write(|tx| Ok(t::insert(tx, &row)?))
    }

    /// Looks up `(kind, url, source)` and inserts `e` when no row has it, in one writer transaction. Callers that race
    /// on the same key leave one row and get the same id.
    pub fn lookup_or_insert(&self, e: &Entry) -> DataResult<LookupOrInsert<Entry>> {
        let row = EntryRow::from(e);
        self.db.write(|tx| match t::by_url_and_source(tx, e.kind, &e.url, e.source)? {
            Some(found) => Ok(LookupOrInsert::Found(found.into())),
            None => Ok(LookupOrInsert::Inserted(t::insert(tx, &row)?)),
        })
    }

    /// Coalesce update of one row. A missing id is not an error.
    pub fn update(&self, u: &EntryUpdate) -> DataResult<()> {
        let patch = EntryPatch::from(u);
        self.db.write(|tx| Ok(t::update(tx, &patch).map(drop)?))
    }

    /// Every update in one transaction: one failure rolls back all of them.
    pub fn update_all(&self, updates: &[EntryUpdate]) -> DataResult<()> {
        self.db.write(|tx| {
            for u in updates {
                t::update(tx, &EntryPatch::from(u))?;
            }
            Ok(())
        })
    }

    /// Replaces the category links of one entry in one transaction. A repeated id is linked once; an id the kind does
    /// not have rolls the whole call back.
    pub fn set_categories(&self, id: i64, category_ids: &[i64]) -> DataResult<()> {
        self.db.write(|tx| Ok(t::set_categories(tx, id, category_ids)?))
    }

    /// Makes the excluded set of an entry equal to `wanted`, inserts and deletes in one transaction.
    pub fn set_excluded_scanlators(&self, id: i64, wanted: &BTreeSet<String>) -> DataResult<()> {
        self.db.write(|tx| Ok(excluded::set_excluded(tx, id, wanted)?))
    }

    /// Writes `value` under `mask` into the viewer flags, in one statement. `Ok(false)` for a missing entry.
    pub fn set_viewer_bits(&self, id: i64, value: i64, mask: i64) -> DataResult<bool> {
        self.db.write(|tx| Ok(t::set_viewer_bits(tx, id, value, mask)? > 0))
    }

    pub fn reset_viewer_flags(&self, kind: Kind) -> DataResult<()> {
        self.db.write(|tx| Ok(t::reset_viewer(tx, kind).map(drop)?))
    }

    /// Sets the `mask` bits of `item_flags` to those of `flag` in one UPDATE (a flag write bumps `version`). For a
    /// manga entry the mask is cut to the manga groups. Returns whether a row changed.
    pub fn set_item_flags_masked(&self, id: i64, flag: i64, mask: i64) -> DataResult<bool> {
        self.set_flags_masked(FlagColumn::ItemFlags, id, flag, mask)
    }

    /// Reads `item_flags`, applies `change` and stores the result, in one writer transaction.
    pub fn update_item_flags(&self, id: i64, change: impl FnOnce(i64) -> i64) -> DataResult<bool> {
        self.update_flags(FlagColumn::ItemFlags, id, change)
    }

    /// As [`Self::set_item_flags_masked`] for `season_flags` (anime entries only).
    pub fn set_season_flags_masked(&self, id: i64, flag: i64, mask: i64) -> DataResult<bool> {
        self.set_flags_masked(FlagColumn::SeasonFlags, id, flag, mask)
    }

    /// As [`Self::update_item_flags`] for `season_flags`.
    pub fn update_season_flags(&self, id: i64, change: impl FnOnce(i64) -> i64) -> DataResult<bool> {
        self.update_flags(FlagColumn::SeasonFlags, id, change)
    }

    /// Clears `parent_id` on the entries named by `ids` (`parent_id` has no foreign key, so a deleted parent leaves it).
    /// Chunks of [`ID_CHUNK`], one transaction.
    pub fn remove_parent_ids(&self, ids: &[i64]) -> DataResult<()> {
        self.db.write(|tx| {
            for chunk in ids.chunks(ID_CHUNK) {
                t::remove_parent_ids(tx, chunk)?;
            }
            Ok(())
        })
    }

    /// Clear database: deletes the non-favourite entries of `source_ids`, orphans the favourite seasons whose parent
    /// went, then deletes the kind's reset history, in one transaction. Returns the history rows removed. An empty
    /// list changes nothing.
    pub fn clear_non_library(&self, kind: Kind, source_ids: &[i64]) -> DataResult<usize> {
        self.db.write(|tx| Ok(t::clear_non_library(tx, kind, source_ids)?))
    }

    /// The clear screen's tree path: deletes the non-favourites in `ids`, orphans the `orphaned` ones that were not
    /// deleted, then deletes the reset history of the deleted entries' kinds.
    pub fn clear_non_library_ids(&self, ids: &[i64], orphaned: &[i64]) -> DataResult<()> {
        self.db.write(|tx| Ok(t::clear_ids(tx, ids, orphaned)?))
    }

    fn set_flags_masked(&self, col: FlagColumn, id: i64, flag: i64, mask: i64) -> DataResult<bool> {
        self.db.write(|tx| Ok(t::set_flags_masked(tx, col, id, flag, mask)? > 0))
    }

    fn update_flags(&self, col: FlagColumn, id: i64, change: impl FnOnce(i64) -> i64) -> DataResult<bool> {
        self.db.write(|tx| match t::read_flags(tx, col, id)? {
            Some(word) => Ok(t::write_flags(tx, col, id, change(word))? > 0),
            None => Ok(false),
        })
    }
}
