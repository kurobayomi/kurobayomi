# Unified database design (step 3a)

Source: `docs/design/unify-map.md` sections 1, 1b, 3, step 3, section 5 (Data, Backup), plus a read of
`crates/kuro-db`, `crates/kuro-data`, `crates/kuro-backup`, `crates/kuro-downloads`, `crates/kuro-track` and the
`src/` wiring on 2026-10-12. Read only; nothing was built or run. Steps 1 and 2 are done.

Decisions this design makes (the rest follows from them):

1. One file `<data_dir>/kurobayomi.db`, `user_version` 1, `application_id` 0x4B555232 ("KUR2"). No migration from the
   old files. Old data comes in only through backup import.
2. `kind` is TEXT `'manga' | 'anime'` (= `Kind::as_str()`), same as `track_retry_queue` already stores it.
3. Item flags: the column holds the anime SUPERSET layout for both kinds. Manga callers keep the manga layout in Rust
   until step 4; the translation sits in two SQL expressions inside the kuro-db manga statements (section 3.4).
4. Step 3 rewrites the per-kind statement modules in place against the new tables (kind pinned by a literal) and keeps
   their public names, row structs and `from_row` mappers. One statement module per table with `kind` as a parameter,
   and `Entry/Item` structs, land in step 4 together with the callers that need them. Reason: ~35 call sites outside
   kuro-data (backup, domain, src) use `kuro_db::statements_manga/anime` and the per-kind row structs; converting them
   now doubles the work and the conversion code is deleted in step 4 anyway. This is the one place this design
   narrows the map's wording ("one statement module per table"); say so if you want the bigger step 3 instead.
5. Categories keep per-kind ids (composite key `(kind, _id)`), because the Aniyomi preference sets and every caller
   treat id 0 as "Default" per kind. Entries, items and tracks have globally unique ids.

---

## 1. Schema `kurobayomi.db` v1

File: `crates/kuro-db/sql/unified/schema.sql` (tables, indexes, triggers, seed rows) and
`crates/kuro-db/sql/unified/views.sql` (three views). `BASELINE_VERSION = 1`; `SCHEMA_VERSION = 3`: migration 2
(`sql/unified/migrate_2.sql`, the `community_scores` cache) and migration 3 (`sql/unified/migrate_3.sql`,
`items.page_count`: a chapter's page count, written by the reader with the last page read; NULL for episodes and for a
chapter not read since, and no reading state, so it bumps no `version`). Pragmas as today (`open_connection`: WAL, foreign_keys ON, busy_timeout, `kuro_lower`).

```sql
-- ===== entries: manga and anime in one table ==================================================
CREATE TABLE entries(
    _id                      INTEGER NOT NULL PRIMARY KEY,
    kind                     TEXT    NOT NULL CHECK (kind IN ('manga', 'anime')),
    source                   INTEGER NOT NULL,
    url                      TEXT    NOT NULL,
    title                    TEXT    NOT NULL,
    artist                   TEXT,
    author                   TEXT,
    description              TEXT,
    genre                    TEXT,
    status                   INTEGER NOT NULL,
    thumbnail_url            TEXT,
    favorite                 INTEGER NOT NULL,
    last_update              INTEGER,
    next_update              INTEGER,
    initialized              INTEGER NOT NULL,
    viewer                   INTEGER NOT NULL,   -- kind-native bits (reader modes / intro+airing), see 3.4
    item_flags               INTEGER NOT NULL,   -- anime superset layout for both kinds, see 3.4
    cover_last_modified      INTEGER NOT NULL,
    date_added               INTEGER NOT NULL,
    update_strategy          INTEGER NOT NULL DEFAULT 0,
    calculate_interval       INTEGER NOT NULL DEFAULT 0,
    last_modified_at         INTEGER NOT NULL DEFAULT 0,
    favorite_modified_at     INTEGER,
    version                  INTEGER NOT NULL DEFAULT 0,
    -- anime only: NULL for manga (checked below)
    fetch_type               INTEGER,            -- 1 = has episodes, 0 = container of seasons
    parent_id                INTEGER,            -- no foreign key, as before (orphans are handled in code)
    season_flags             INTEGER,
    season_number            REAL,
    season_source_order      INTEGER,
    background_url           TEXT,
    background_last_modified INTEGER,
    memo                     BLOB,               -- JSON object bytes
    UNIQUE (kind, source, url),
    UNIQUE (_id, kind),                          -- parent key for the composite foreign keys below
    CHECK (kind = 'anime' OR (fetch_type IS NULL AND parent_id IS NULL AND season_flags IS NULL
                              AND season_number IS NULL AND season_source_order IS NULL
                              AND background_url IS NULL AND background_last_modified IS NULL AND memo IS NULL)),
    CHECK (kind = 'manga' OR (fetch_type IS NOT NULL AND season_flags IS NOT NULL AND season_number IS NOT NULL
                              AND season_source_order IS NOT NULL AND background_last_modified IS NOT NULL
                              AND memo IS NOT NULL))
);
CREATE INDEX entries_favorite_index ON entries(kind) WHERE favorite = 1;
CREATE INDEX entries_parent_index   ON entries(parent_id) WHERE parent_id IS NOT NULL;

CREATE TRIGGER update_last_favorite_at_entries
AFTER UPDATE OF favorite ON entries
BEGIN
    UPDATE entries SET favorite_modified_at = strftime('%s', 'now') WHERE _id = new._id;
END;
CREATE TRIGGER update_last_modified_at_entries
AFTER UPDATE ON entries
FOR EACH ROW
BEGIN
    UPDATE entries SET last_modified_at = strftime('%s', 'now') WHERE _id = new._id;
END;
CREATE TRIGGER update_entry_version AFTER UPDATE ON entries
BEGIN
    UPDATE entries SET version = version + 1
    WHERE _id = new._id AND (new.url != old.url OR new.description != old.description OR new.favorite != old.favorite);
END;

-- ===== items: chapters and episodes ============================================================
CREATE TABLE items(
    _id              INTEGER NOT NULL PRIMARY KEY,
    entry_id         INTEGER NOT NULL REFERENCES entries(_id) ON DELETE CASCADE,
    url              TEXT    NOT NULL,
    name             TEXT    NOT NULL,
    scanlator        TEXT,
    done             INTEGER NOT NULL,           -- chapters.read / episodes.seen
    bookmark         INTEGER NOT NULL,
    progress         INTEGER NOT NULL,           -- chapters.last_page_read / episodes.last_second_seen
    total_seconds    INTEGER,                    -- episodes only (NULL for chapters)
    number           REAL    NOT NULL,           -- chapter_number / episode_number
    source_order     INTEGER NOT NULL,
    date_fetch       INTEGER NOT NULL,
    date_upload      INTEGER NOT NULL,
    last_modified_at INTEGER NOT NULL DEFAULT 0,
    version          INTEGER NOT NULL DEFAULT 0,
    fillermark       INTEGER NOT NULL DEFAULT 0, -- episodes only (0 for chapters)
    summary          TEXT,                       -- episodes only
    preview_url      TEXT,                       -- episodes only
    memo             BLOB                        -- episodes only
);
CREATE INDEX items_entry_index  ON items(entry_id);
CREATE INDEX items_undone_index ON items(entry_id, done) WHERE done = 0;
CREATE INDEX items_url_index    ON items(url);   -- episodes did not have it; chapters got it in migration 34

CREATE TRIGGER update_last_modified_at_items
AFTER UPDATE ON items
FOR EACH ROW
BEGIN
    UPDATE items SET last_modified_at = strftime('%s', 'now') WHERE _id = new._id;
END;
CREATE TRIGGER update_item_and_entry_version AFTER UPDATE ON items
WHEN new.done != old.done OR new.bookmark != old.bookmark OR new.progress != old.progress
BEGIN
    UPDATE items   SET version = version + 1 WHERE _id = new._id;
    UPDATE entries SET version = version + 1 WHERE _id = new.entry_id;
END;

CREATE TABLE excluded_scanlators(                -- manga-only today; entry ids are unique so no kind column
    entry_id  INTEGER NOT NULL REFERENCES entries(_id) ON DELETE CASCADE,
    scanlator TEXT    NOT NULL
);
CREATE INDEX excluded_scanlators_entry_index ON excluded_scanlators(entry_id);

-- ===== history =================================================================================
CREATE TABLE history(
    _id         INTEGER NOT NULL PRIMARY KEY,
    item_id     INTEGER NOT NULL UNIQUE REFERENCES items(_id) ON DELETE CASCADE,
    last_at     INTEGER NOT NULL DEFAULT 0,      -- ms; 0 = reset (anime last_seen NULL reads as 0)
    duration_ms INTEGER NOT NULL DEFAULT 0       -- chapters.time_read; anime writes 0 until step 4
);

-- ===== categories (ids are per kind) ===========================================================
CREATE TABLE categories(
    kind   TEXT    NOT NULL CHECK (kind IN ('manga', 'anime')),
    _id    INTEGER NOT NULL,
    name   TEXT    NOT NULL,
    sort   INTEGER NOT NULL,
    flags  INTEGER NOT NULL,
    hidden INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (kind, _id)
);
INSERT INTO categories(kind, _id, name, sort, flags) VALUES ('manga', 0, '', -1, 64), ('anime', 0, '', -1, 64);
CREATE TRIGGER system_category_delete_trigger BEFORE DELETE ON categories
BEGIN SELECT CASE WHEN old._id <= 0 THEN RAISE(ABORT, 'System category can''t be deleted') END; END;

CREATE TABLE entry_categories(                   -- NOT "WITHOUT ROWID": SQLite's update hook skips those tables
    entry_id    INTEGER NOT NULL,
    kind        TEXT    NOT NULL,
    category_id INTEGER NOT NULL,
    PRIMARY KEY (entry_id, category_id),
    FOREIGN KEY (entry_id, kind)    REFERENCES entries(_id, kind)    ON DELETE CASCADE,
    FOREIGN KEY (kind, category_id) REFERENCES categories(kind, _id) ON DELETE CASCADE
);
CREATE INDEX entry_categories_category_index ON entry_categories(kind, category_id);
CREATE TRIGGER insert_entry_category_update_version AFTER INSERT ON entry_categories
BEGIN
    UPDATE entries SET version = version + 1 WHERE _id = new.entry_id;
END;

-- ===== tracks (manga_sync + anime_sync) ========================================================
CREATE TABLE tracks(
    _id         INTEGER NOT NULL PRIMARY KEY,
    entry_id    INTEGER NOT NULL REFERENCES entries(_id) ON DELETE CASCADE,
    tracker_id  INTEGER NOT NULL,                -- sync_id
    remote_id   INTEGER NOT NULL,
    library_id  INTEGER,
    title       TEXT    NOT NULL,
    last_item   REAL    NOT NULL,                -- last_chapter_read / last_episode_seen
    total_items INTEGER NOT NULL,                -- total_chapters / total_episodes
    status      INTEGER NOT NULL,
    score       REAL    NOT NULL,
    remote_url  TEXT    NOT NULL,
    start_date  INTEGER NOT NULL,
    finish_date INTEGER NOT NULL,
    private     INTEGER NOT NULL DEFAULT 0,
    UNIQUE (entry_id, tracker_id) ON CONFLICT REPLACE
);

-- ===== sources and relations ====================================================================
CREATE TABLE stub_sources(                       -- sources + animesources; id 0 (local) exists in both kinds
    kind TEXT    NOT NULL CHECK (kind IN ('manga', 'anime')),
    id   INTEGER NOT NULL,
    lang TEXT    NOT NULL,
    name TEXT    NOT NULL,
    PRIMARY KEY (kind, id)
);

CREATE TABLE anime_relations(
    entry_id      INTEGER NOT NULL REFERENCES entries(_id) ON DELETE CASCADE,
    related_id    INTEGER NOT NULL REFERENCES entries(_id) ON DELETE CASCADE,
    name          TEXT    NOT NULL,
    sort_order    INTEGER NOT NULL,
    last_fetched_at INTEGER NOT NULL,
    PRIMARY KEY (entry_id, related_id, name)
);
CREATE INDEX anime_relations_related_index ON anime_relations(related_id);

-- ===== downloads and tracker retry (today created lazily by their own stores) ==================
CREATE TABLE download_queue(
    unit_id     INTEGER NOT NULL PRIMARY KEY REFERENCES items(_id) ON DELETE CASCADE,
    kind        TEXT    NOT NULL,
    entry_id    INTEGER NOT NULL,
    source      INTEGER NOT NULL,
    position    INTEGER NOT NULL,
    status      INTEGER NOT NULL DEFAULT 1,
    enqueued_at INTEGER NOT NULL,
    FOREIGN KEY (entry_id, kind) REFERENCES entries(_id, kind) ON DELETE CASCADE
);
CREATE INDEX download_queue_position_index ON download_queue(position);
CREATE INDEX download_queue_source_index   ON download_queue(kind, source, position);

CREATE TABLE download_pending_delete(            -- no foreign keys: the item may be gone when the flush runs
    unit_id     INTEGER NOT NULL PRIMARY KEY,
    kind        TEXT    NOT NULL CHECK (kind IN ('manga', 'anime')),
    entry_id    INTEGER NOT NULL,
    entry_url   TEXT    NOT NULL,
    entry_title TEXT    NOT NULL,
    source      INTEGER NOT NULL,
    unit_url    TEXT    NOT NULL,
    unit_name   TEXT    NOT NULL,
    scanlator   TEXT,
    done        INTEGER NOT NULL DEFAULT 0,      -- anime pending rows carried seen/bookmark; manga wrote 0
    bookmark    INTEGER NOT NULL DEFAULT 0,
    seq         INTEGER NOT NULL,                -- insertion order inside an entry (manga used enqueued_at, id)
    enqueued_at INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX download_pending_delete_entry_index ON download_pending_delete(entry_id, seq);

CREATE TABLE download_pending_corrupt(           -- anime_download_pending_corrupt, E-P04: rows that could not be read
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    source_rowid INTEGER NOT NULL,
    reason       TEXT    NOT NULL,
    recorded_at  INTEGER NOT NULL
);

CREATE TABLE track_retry_queue(                  -- verbatim from kuro-track/src/queue/store.rs
    kind       TEXT    NOT NULL CHECK (kind IN ('anime', 'manga')),
    entry_id   INTEGER NOT NULL,
    tracker_id INTEGER NOT NULL,
    progress   REAL    NOT NULL,
    queued_at  INTEGER NOT NULL,
    attempts   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (kind, entry_id, tracker_id)
);
```

`download_queue` is keyed by `unit_id` alone: item ids are unique across kinds, so `(kind, unit_id)` adds nothing. `kind`
is a column, and the composite foreign key to `entries(_id, kind)` makes a wrong kind impossible. `track_retry_queue`
keeps its `(kind, entry_id, tracker_id)` key and gets no foreign key (entries may be deleted while a retry is parked,
as today).

`custom_buttons`, `extension_store`, `extension_repos`: not created (see 1.2).

### 1.1 Views (`views.sql`): three, all with `kind`

```sql
CREATE VIEW historyView AS
SELECT h._id AS id, e.kind, e._id AS entryId, i._id AS itemId, e.title, e.thumbnail_url AS thumbnailUrl,
       e.source, e.favorite, e.cover_last_modified, i.number AS itemNumber,
       h.last_at AS lastAt, h.duration_ms AS durationMs,
       m.last_at AS maxLastAt, m.item_id AS maxLastAtItemId
FROM entries e
JOIN items   i ON i.entry_id = e._id
JOIN history h ON h.item_id  = i._id
JOIN (SELECT i2.entry_id, i2._id AS item_id, MAX(h2.last_at) AS last_at       -- SQLite bare-column-with-MAX: the row of the max
      FROM items i2 JOIN history h2 ON h2.item_id = i2._id GROUP BY i2.entry_id) m
  ON m.entry_id = e._id;

CREATE VIEW updatesView AS
SELECT e.kind, e._id AS entryId, e.title AS entryTitle, i._id AS itemId, i.name AS itemName, i.scanlator,
       i.done, i.bookmark, i.fillermark, i.progress, i.total_seconds AS totalSeconds,
       e.source, e.favorite, e.thumbnail_url AS thumbnailUrl, e.cover_last_modified AS coverLastModified,
       i.date_upload AS dateUpload, i.date_fetch AS datefetch
FROM entries e JOIN items i ON i.entry_id = e._id
WHERE e.favorite = 1 AND i.date_fetch > e.date_added;      -- no ORDER BY inside the view; statements order

CREATE VIEW libraryView AS
WITH scope(entry_id) AS (                         -- the favourites prefilter (migration 34) for BOTH kinds
    SELECT _id FROM entries WHERE favorite = 1
    UNION
    SELECT _id FROM entries WHERE parent_id IN (SELECT _id FROM entries WHERE favorite = 1)
),
item_stats AS (
    SELECT i.entry_id,
           count(*)                        AS total,
           sum(i.done)                     AS doneCount,
           coalesce(max(i.date_upload), 0) AS latestUpload,
           coalesce(max(i.date_fetch), 0)  AS fetchedAt,
           sum(i.bookmark)                 AS bookmarkCount,
           sum(i.fillermark)               AS fillermarkCount,
           coalesce(max(h.last_at), 0)     AS lastActivity
    FROM items i
    LEFT JOIN excluded_scanlators x ON x.entry_id = i.entry_id AND x.scanlator = i.scanlator
    LEFT JOIN history h             ON h.item_id = i._id
    WHERE x.scanlator IS NULL AND i.entry_id IN (SELECT entry_id FROM scope)
    GROUP BY i.entry_id
),
season_stats AS (                                 -- anime containers (fetch_type = 0) roll up their children
    SELECT s.parent_id,
           count(*) AS child_count,
           sum(CASE WHEN s.fetch_type = 1 AND coalesce(t.total, 0) = coalesce(t.doneCount, 0) THEN 1 ELSE 0 END) AS fully_done,
           max(coalesce(t.latestUpload, 0))  AS latestUpload,
           max(coalesce(t.fetchedAt, 0))     AS fetchedAt,
           max(coalesce(t.lastActivity, 0))  AS lastActivity,
           sum(coalesce(t.bookmarkCount, 0)) AS bookmarkCount,
           sum(coalesce(t.fillermarkCount, 0)) AS fillermarkCount
    FROM entries s LEFT JOIN item_stats t ON t.entry_id = s._id
    WHERE s.parent_id IN (SELECT _id FROM entries WHERE favorite = 1)
    GROUP BY s.parent_id
)
SELECT e.*,
       CASE WHEN e.fetch_type = 0 THEN coalesce(ss.child_count, 0)     ELSE coalesce(st.total, 0)           END AS totalCount,
       CASE WHEN e.fetch_type = 0 THEN coalesce(ss.fully_done, 0)      ELSE coalesce(st.doneCount, 0)       END AS doneCount,
       CASE WHEN e.fetch_type = 0 THEN coalesce(ss.latestUpload, 0)    ELSE coalesce(st.latestUpload, 0)    END AS latestUpload,
       CASE WHEN e.fetch_type = 0 THEN coalesce(ss.fetchedAt, 0)       ELSE coalesce(st.fetchedAt, 0)       END AS itemFetchedAt,
       CASE WHEN e.fetch_type = 0 THEN coalesce(ss.lastActivity, 0)    ELSE coalesce(st.lastActivity, 0)    END AS lastActivity,
       CASE WHEN e.fetch_type = 0 THEN coalesce(ss.bookmarkCount, 0)   ELSE coalesce(st.bookmarkCount, 0)   END AS bookmarkCount,
       CASE WHEN e.fetch_type = 0 THEN coalesce(ss.fillermarkCount, 0) ELSE coalesce(st.fillermarkCount, 0) END AS fillermarkCount,
       coalesce(ec.category_id, 0) AS category
FROM entries e
LEFT JOIN item_stats st       ON st.entry_id  = e._id
LEFT JOIN season_stats ss     ON ss.parent_id = e._id
LEFT JOIN entry_categories ec ON ec.entry_id  = e._id
WHERE e.favorite = 1;
```

`animeseasonsView` becomes a statement, not a view: `statements_anime::seasons_of(c, parent_id)` uses the same two CTEs
with the scope replaced by `(children of :parent) UNION (their children)`, because a season list is also shown for a
parent that is NOT a favourite (a browsed title), which the favourites prefilter would zero out. A test asserts the two
texts give equal aggregates for a favourite parent. `animedeletableView` becomes the inline statement
`SELECT _id, source, fetch_type FROM entries WHERE kind='anime' AND favorite=0 AND parent_id IS NULL`.
`episodestatsView`, `animehistorystatsView`, `animeseasonstatsView` are gone (CTEs above).

### 1.2 What is dropped, and why

| Dropped | Reason (checked in code) |
|---|---|
| `mangas.is_syncing`, `chapters.is_syncing`, `animes.is_syncing`, `episodes.is_syncing` and every `is_syncing = 0` in a version trigger | Nothing ever sets it to 1 (only the restore tests do). The two heals (`reset_is_syncing`, `heal_stuck_sync_flags`) go too. `MangaRow.is_syncing`/`AnimeRow.is_syncing` stay as always-false/0 fields until step 4 removes the structs. |
| `mangas_url_index`, `animes_url_index`, `animes_fetch_type` index | No statement filters entries by url alone or by fetch_type; `UNIQUE(kind, source, url)` serves the real lookup. |
| `custom_buttons` table and its seeded "skip intro" row | No reader or writer in the app: the creator emits an empty list, the restore never writes it, the player has its own intro logic. |
| `extension_store` | Read only by `ext_host::store_rows` for a legacy list nothing fills (repos are prefs links). `ExtensionStoreRepo` and `ext_host.rs::store_rows` go; step 11 finishes the rest. |
| `extension_repos` | No live writer (only tests); `MangaRepoKeys` is never constructed; the restore of the section is already a no-op. The creator's `manga_extension_repos()` returns an empty list (the repo links travel in the prefs section). `kuro_data::extension_repo` and `kuro_db::statements_manga::extension_repos` are deleted. |
| views `episodestatsView`, `animehistorystatsView`, `animeseasonstatsView`, `animeseasonsView`, `animedeletableView`, `animelibView`, `animehistoryView`, `animeupdatesView`, per-kind `libraryView/historyView/updatesView` | Replaced by the three views and two statements above. |
| ids of the old rows | Not carried; new ids are assigned on restore (entries/items/tracks) or per kind (categories, starting at 1 after the system row). |

Every other old column survives: `version`, `last_modified_at`, `favorite_modified_at` are read by the backup creator and
by the restore merge (newer `version` wins); `library_id`, `private` are in the wire format and the trackers;
`calculate_interval`, `update_strategy`, `next_update`, `last_update` drive the library update and Upcoming.

---

## 2. Mapping, old to new

This is the definition the rewritten statements and the backup restore implement. It is NOT a data migration: no old
file is read.

### 2.1 Tables

| Old (manga.db) | Old (anime.db) | New |
|---|---|---|
| `mangas` | `animes` | `entries` (`kind`) |
| `chapters` | `episodes` | `items` |
| `history` | `animehistory` | `history` |
| `categories` | `categories` | `categories` (`kind`, per-kind `_id`) |
| `mangas_categories` | `animes_categories` | `entry_categories` (`kind`) |
| `manga_sync` | `anime_sync` | `tracks` |
| `sources` | `animesources` | `stub_sources` (`kind`) |
| `excluded_scanlators` | (none) | `excluded_scanlators` |
| (none) | `anime_relations` | `anime_relations` |
| `manga_download_queue` (own store) | `anime_download_queue` (own store) | `download_queue` |
| `manga_pending_delete` (own store) | `anime_download_pending_delete`, `anime_download_pending_corrupt` | `download_pending_delete`, `download_pending_corrupt` |
| (kuro-track, anime.db) `track_retry_queue` | same table | `track_retry_queue` |
| `extension_repos` | `extension_store`, `custom_buttons` | dropped |

### 2.2 Columns

| Old | New | Note |
|---|---|---|
| `mangas._id` / `animes._id` | `entries._id` | unique across kinds |
| (implicit) | `entries.kind` | `'manga'` / `'anime'` |
| `source, url, artist, author, description, genre, title, status, thumbnail_url, favorite, last_update, next_update, initialized, viewer, cover_last_modified, date_added, update_strategy, calculate_interval, last_modified_at, favorite_modified_at, version` | same names | |
| `mangas.chapter_flags` | `entries.item_flags` | translated, see 3.4 |
| `animes.episode_flags` | `entries.item_flags` | identity |
| `animes.fetch_type, parent_id, season_flags, season_number, season_source_order, background_url, background_last_modified, memo` | same names | NULL for manga |
| `*.is_syncing` | dropped | |
| `chapters._id` / `episodes._id` | `items._id` | unique across kinds |
| `chapters.manga_id` / `episodes.anime_id` | `items.entry_id` | |
| `chapters.read` / `episodes.seen` | `items.done` | |
| `chapters.last_page_read` / `episodes.last_second_seen` | `items.progress` | |
| `episodes.total_seconds` | `items.total_seconds` | NULL for chapters |
| `chapters.chapter_number` / `episodes.episode_number` | `items.number` | |
| `url, name, scanlator, bookmark, source_order, date_fetch, date_upload, last_modified_at, version` | same names | |
| `episodes.summary, preview_url, fillermark, memo` | same names | chapters: NULL / NULL / 0 / NULL |
| `history.chapter_id` / `animehistory.episode_id` | `history.item_id` | |
| `history.last_read` / `animehistory.last_seen` | `history.last_at` | anime NULL becomes 0 |
| `history.time_read` | `history.duration_ms` | anime: 0 |
| `categories._id, name, sort, flags, hidden` | same names plus `kind` | system row `(kind, 0)` per kind; manga seed flags 0 then "fresh => 64" (Q28) becomes seed 64 in the baseline |
| `mangas_categories.manga_id` / `animes_categories.anime_id` | `entry_categories.entry_id` | `_id` column dropped (nothing reads it); duplicates now ignored (`INSERT OR IGNORE`) |
| `manga_sync.manga_id` / `anime_sync.anime_id` | `tracks.entry_id` | |
| `sync_id` | `tracks.tracker_id` | |
| `last_chapter_read` / `last_episode_seen` | `tracks.last_item` | |
| `total_chapters` / `total_episodes` | `tracks.total_items` | |
| `remote_id, library_id, title, status, score, remote_url, start_date, finish_date, private` | same names | |
| `sources._id` / `animesources._id` | `stub_sources.id` (+`kind`) | |
| `anime_relations.anime_id, related_anime_id` | `entry_id, related_id` | |
| `manga_download_queue.chapter_id, manga_id, source_id, position, status, enqueued_at` | `download_queue.unit_id, entry_id, source, position, status, enqueued_at` | |
| `anime_download_queue.episode_id, anime_id, ord, changed_at` | `download_queue.unit_id, entry_id, position, enqueued_at` | `source` filled from `entries.source` by the store; `changed_at` not kept |
| `manga_pending_delete.chapter_id, manga_id, manga_title, manga_url, source_id, chapter_name, chapter_url, scanlator, enqueued_at` | `download_pending_delete.unit_id, entry_id, entry_title, entry_url, source, unit_name, unit_url, scanlator, enqueued_at` | `seq` = per-entry counter |
| `anime_download_pending_delete.episode_id, anime_id, episode_url, episode_name, scanlator, seen, bookmark, anime_url, anime_title, anime_source, seq` | `unit_id, entry_id, unit_url, unit_name, scanlator, done, bookmark, entry_url, entry_title, source, seq` | |

### 2.3 Views

| Old | New |
|---|---|
| `libraryView` columns `totalCount, readCount, latestUpload, chapterFetchedAt, lastRead, bookmarkCount, category` | `libraryView` `totalCount, doneCount, latestUpload, itemFetchedAt, lastActivity, bookmarkCount, fillermarkCount, category`; the manga facade selects `doneCount AS readCount, itemFetchedAt AS chapterFetchedAt, lastActivity AS lastRead` |
| `animelibView` `seenCount, episodeFetchedAt, lastSeen, fillermarkCount` | same view; facade aliases `doneCount AS seenCount, itemFetchedAt AS episodeFetchedAt, lastActivity AS lastSeen` |
| `historyView` / `animehistoryView` (`mangaId/animeId, chapterId/episodeId, readAt/seenAt, readDuration`) | `historyView` (`entryId, itemId, lastAt, durationMs`, `kind`) |
| `updatesView` / `animeupdatesView` | `updatesView` (`kind`) |

---

## 3. Keys and invariants

1. **Source ids collide across kinds** (local = 0 for both; `generate_id` can repeat). Every source-keyed row carries
   `kind`: `entries UNIQUE(kind, source, url)`, `stub_sources PRIMARY KEY(kind, id)`, `download_queue.kind` +
   `source`, `download_pending_delete.kind`. All statements that filter by source add `AND kind = ?`: the favourite
   counts per source, non-library counts, `get_favorite_by_source_id`, clear-non-library-by-sources, the duplicate
   check, upcoming. (Hidden/pinned source sets are prefs and already per kind.)
2. **Entry, item and track ids are unique across kinds** (single rowid tables), so `TrackChanged { entry_id }` stops
   being ambiguous in step 8. Per-kind statements still pin the kind (`WHERE _id = ? AND kind = 'manga'`) so a wrong
   id from the other kind returns "not found", never a foreign row.
3. **Category ids are per kind.** `(kind, _id)` is the key; the new id is `max(_id)+1` of that kind (as before, no
   AUTOINCREMENT): `INSERT INTO categories(kind,_id,name,sort,flags) SELECT :kind, coalesce(max(_id),0)+1, ... FROM
   categories WHERE kind = :kind RETURNING _id` (`RETURNING` replaces `last_insert_rowid()`, which is the hidden rowid
   here). `entry_categories` has composite foreign keys on both sides, so a membership across kinds fails at the
   database. System categories are `(manga,0)` and `(anime,0)`, undeletable by trigger, flags seeded 64.
4. **Flag words.** `entries.item_flags` is the anime superset layout for both kinds:

   | Field | manga `chapter_flags` | unified (= anime `episode_flags`) |
   |---|---|---|
   | sort dir 0x1, unread mask 0x6, downloaded mask 0x18, bookmarked mask 0x60 | same bits | same bits |
   | fillermarked (0x80/0x100) | none | anime only |
   | sorting source/number/upload/alphabet | 0x000 / 0x100 / 0x200 / 0x300 (mask 0x300) | 0x000 / 0x200 / 0x400 / 0x600 (mask 0x600) |
   | previews 0x800, summaries 0x1000 | none | anime only |
   | display 0x100000 | same | same |

   Manga to unified: `tr(x) = (x & ~0x300) | ((x & 0x300) << 1)`. Unified to manga: `un(x) = (x & ~0x600) | ((x & 0x600) >> 1)`.
   Manga rows never carry 0x80/0x100/0x800/0x1000, so the round trip is exact. Rust has both
   (`kuro_db::flags::manga_item_flags_to_unified/from_unified`); the manga statements use the equivalent SQL
   expressions: SELECT lists alias `un(item_flags) AS chapter_flags`; INSERT/UPDATE bind `tr(:chapter_flags)`; the masked
   writers (`kuro_data::flags_write`, `src/entry_core/store.rs:33`) become
   `UPDATE entries SET item_flags = (item_flags & ~tr(?2)) | (tr(?1) & tr(?2)) WHERE _id = ?3 AND kind = 'manga'`
   (translating both flag and mask keeps every field exact). Anime and all callers above the statements see no change.
   Step 4 deletes the translation and moves it into the backup edge when manga callers switch to the unified
   constants. `entries.viewer` is NOT translated: it holds the kind's own bits (reader mode/orientation vs
   intro/airing masks), written by masked writes that already know the kind. `categories.flags` (library sort) has the
   same layout for both kinds (anime adds AIRING 0x30); no translation.
5. **Uniqueness is new.** `UNIQUE(kind, source, url)` did not exist (anime test
   `duplicate_source_and_url_resolve_to_the_first_row` documents the old tolerance). A blind second insert now fails
   with `DbError::Constraint`. Production inserts go through `lookup_or_insert`/restore `get_by_url_and_source`, so
   they are fine; test fixtures that insert the same `(source, url)` twice must change (found by running the suite).
6. **Kind checks:** the two `CHECK`s on `entries` make anime-only columns NULL for manga and NOT NULL for anime, so a
   facade that forgets a column fails loudly. `download_queue`'s `(entry_id, kind)` foreign key does the same for kind.
7. **Notifications.** Live queries subscribe by table name (`Db::subscribe(&["entries", ...])`). With one file a manga
   write also wakes anime subscribers; every subscription already publishes only when the result changed, and the
   debounce stays, so this costs a re-query, not a UI redraw. The table lists in kuro-data (`ANIME_LIBRARY_TABLES`,
   `MANGA_HISTORY_TABLES`, ...) are rewritten to the new names; `notify::Tables`/`MANGA_TABLES` become one
   `TABLES` list of these 13 names, `Tables` bits renumbered (only kuro-db uses them):
   `entries, items, history, categories, entry_categories, excluded_scanlators, tracks, stub_sources, anime_relations,
   download_queue, download_pending_delete, download_pending_corrupt, track_retry_queue`.
8. **Search.** Both kinds' history search is `kuro_lower(title) LIKE '%' || kuro_lower(:q) || '%' ESCAPE '\'` with
   `kind = :kind` (fixes the manga ASCII-only drift, section 5 Data row 1).

---

## 4. Open path

New API in `kuro-db` (file `crates/kuro-db/src/schema_unified.rs`, replaces `schema_manga.rs` + `schema_anime.rs`
opening code; the anime types `FetchType/Millis/Memo/flags` stay in `schema_anime.rs` until step 4):

```rust
pub const DB_FILE: &str = "kurobayomi.db";
pub const APPLICATION_ID: i64 = 0x4B55_5232;
pub const SCHEMA_VERSION: i64 = 1;
pub const UNIFIED_SCHEMA: SchemaBundle = SchemaBundle { baseline_sql: concat!(schema.sql, views.sql), baseline_version: 1,
    migrations: &[], application_id: APPLICATION_ID, tables: TABLES, refuse_foreign: |f| DbError::ForeignFile { found: f } };
pub fn db_path(data_dir: &Path) -> PathBuf;
pub fn open_file(path: &Path) -> DbResult<Connection>;      // open_and_migrate(path, &UNIFIED_SCHEMA)? then heal(&c)
pub fn heal(c: &Connection) -> DbResult<()>;                // INSERT OR IGNORE the two system categories (the one heal)
pub fn open_in_memory() -> DbResult<Connection>;            // tests; replaces schema_anime::open_in_memory
pub fn legacy_files(data_dir: &Path) -> Vec<PathBuf>;       // manga.db / anime.db that exist and are non-empty
impl Db { pub fn open(paths: &dyn Paths) -> DbResult<Arc<Db>>; }   // was Db::open(kind, paths)
```

- `open_and_migrate`, the `.dirty` marker, `quick_check`/`integrity_check`, quarantine (`*.corrupt-<UTC>`),
  `restore_newest_backup` and `migrate::run` are reused unchanged; they are bundle-generic. With no migrations the
  `*.pre-vN` backup never fires until migration 2 exists.
- `DbError` loses `LegacyFile`, `NeedsLegacyImport`, `TooOldForImport`; gains `ForeignFile { found }` ("not a
  Kurobayomi database"). `TooNew` stays. `DbKindExt`, `Kind::path/file_name/schema_version`, `ANIME_SCHEMA`,
  `MANGA_SCHEMA`, `ANIME_APPLICATION_ID` and the two `refuse_foreign` fns are deleted.
- `src/data.rs::open_blocking`: one path, one `open_or_restore(&path, kuro_db::schema_unified::open_file)`, one
  `Db::new(&path, writer)`; `Opened`/`App` hold one `Arc<Db>` (`App.db`); `App::categories(kind)` stays (one
  `CategoryService` per kind over the same `Arc<Db>`, step 4 merges them). `main.rs` shutdown closes one handle.
  `src/backup_run.rs::open_headless` and `restore_into` open once; `src/cli.rs:84,172`, `src/settings.rs:300` use
  `db_path`.
- **A profile with the old two files.** The new file does not exist, so the open creates an empty v1 database. The
  old `manga.db`, `anime.db` and their `-wal`, `-shm`, `.dirty`, `*.pre-vN`, `*.corrupt-*` siblings are never opened,
  renamed, moved or deleted: byte-identical beside the new file (a test hashes them). Right after creating the new
  file `data.rs` calls `legacy_files(data_dir)` and, if any, logs ONCE
  (`tracing::warn!(files = ?, "old per-kind databases found; left untouched and not read; use Restore to bring the data over")`).
  "Once" holds because the check runs only when `kurobayomi.db` was absent (`existed = metadata(path).len() > 0`
  before the open); later launches see it present. No toast and no new i18n key in 3a (a user-visible hint is a
  follow-up). The old downloads queue rows and the unfinished pending deletes live in the old files and are not carried;
  downloaded files on disk are untouched (the disk index finds them).
- Stale per-kind category-id preferences (default category, update/download category sets) point at ids that do not
  exist in the fresh database; the existing startup cleanup (`startup_cleanup` with `known_ids` per kind in
  `data.rs::install`) already drops unknown ids. Unchanged.
- A kuro-db file from a newer app: `TooNew`, refused untouched (existing behaviour, test kept).

---

## 5. Transition plan (compiles at every commit)

### 5.1 The seam

`kuro-db`'s public Rust API stays source-compatible for every crate above it, except the open functions. Concretely,
lane A keeps: the `statements_manga::{mangas, chapters, categories, mangas_categories, history, views, manga_sync,
excluded_scanlators, sources}` modules and `statements_anime::*` functions with the same names and signatures, the row
structs (`MangaRow`, `ChapterRow`, `LibraryRow`, `HistoryViewRow`, `UpdateViewRow`, `TrackRow`, `StubSourceRow`,
`AnimeRow`, `EpisodeRow`, `AnimeCategory`, `AnimeTrack`, `LibraryAnimeRow`, `SeasonAnimeRow`, `AnimeUpdateRow`,
`HistoryEntry`, `StubSource`, patches) and their by-name `from_row`s (the SELECT lists alias unified columns to the old
names, so no mapper changes), `kuro_db::flags::*`, `codec`, `schema_anime::{FetchType, Millis, Memo, flags}`,
`Db::{new, read, write, subscribe, ...}`. Only the SQL text inside changes. The open entry points keep working through
shims until lane E (names kept, bodies call `open_file`):

| Shim (deleted in lane E) | Becomes |
|---|---|
| `schema_manga::open(path)`, `schema_anime::open(path)` | `schema_unified::open_file(path)` |
| `schema_manga::db_path`, `schema_anime::db_path` | `schema_unified::db_path` (same name `kurobayomi.db`) |
| `schema_anime::open_in_memory()` | `schema_unified::open_in_memory()` |
| `schema_manga::{BASELINE_SQL, SCHEMA_VERSION, ...}` | unified constants |

Because the shims return the unified schema at whatever path they are given, every existing test that opens "a manga
db" and "an anime db" in two temp files still compiles and runs (two independent unified files, each used for one kind).
The one place that must not use two files over one path is the real app: `src/data.rs`, `backup_run.rs` (lane D)
open one file once, in the same merge train as lane A.

Compile-green holds at every commit (API above is unchanged). Test-green holds at the end of the train (A to E): between
A and its consumers the raw SQL strings in kuro-data/downloads/domain/src (not compile-checked) still name the old
tables, so land A to D as one merge train on one branch and run the full suite at D and E. Each lane runs its own
crate's tests before it is merged. Rollback is section 7.

### 5.2 Lanes (disjoint file ownership), in order

**Lane A: kuro-db (blocks the rest; one agent).** Owns `crates/kuro-db/**` and `crates/kuro-testkit/src/db.rs`.
- Add `sql/unified/schema.sql`, `sql/unified/views.sql`, `src/schema_unified.rs`; rewrite `schema.rs` (`SchemaBundle` stays,
  baseline consts replaced), `open.rs` (tests use `UNIFIED_SCHEMA`), `runtime.rs` (`Db::open(paths)`, delete
  `DbKindExt`), `notify.rs` (`TABLES`, bits, `*_DEPS`), `error.rs`, `lib.rs`, `maintenance.rs` (unchanged),
  `testdb.rs`.
- Rewrite SQL in `statements_manga.rs` (all modules, plus fixtures) and `statements_anime.rs` (all functions): table
  and column renames per 2.2, `kind` literal, the flag translation (3.4), explicit SELECT column lists with aliases (no
  `SELECT *` over `entries`), `INSERT OR IGNORE` for memberships, `RETURNING _id` for categories, per-kind category
  and stub-source statements, the new `libraryView` aliases, `seasons_of` as a statement, `kuro_lower` in both history
  searches, favourites prefilter via the view.
- Delete: `schema_manga.rs` (move `test_db()` to `testdb.rs`), the opening/`heal` half of `schema_anime.rs`
  (keep the type and flag modules), `views_anime.rs`, `conformance.rs` Aniyomi parts, `sql/schema_v33.sql`,
  `sql/migrations/34.sql`, `sql/anime/**`, `tests/fixtures/aniyomi_fresh_v33.sql`, `tests/fixtures/schema_v33.expected.txt`,
  `tests/golden/anime_schema_v139.txt`, `tests/library_view_34.rs`; extension statements (`extension_repos`,
  `extension_store*`, `custom_buttons*`).
- `testkit/src/db.rs`: `TestDb::handle(dir)` (one unified file); `manga_handle`/`anime_handle` stay as aliases of it
  with their old file names until lane E; `TestDb::anime()` uses `open_in_memory()`; its schema-golden test becomes the
  unified inventory test.
- Compile seam out: the API listed in 5.1 plus `schema_unified::{open_file, open_in_memory, db_path, legacy_files}`,
  `Db::open(paths)`, `kuro_db::DB_TABLES: &[&str]` (names for `Db::subscribe`).

**Lane B: kuro-data (after A; parallel with C and D).** Owns `crates/kuro-data/src/**`. Same pub API and types.
- `manga.rs`, `chapter.rs`, `episode.rs`, `anime.rs`, `category.rs`, `history.rs`, `track.rs`, `source.rs`,
  `anime_relation.rs`: subscription table lists to the new names (`MANGA_TABLE=&["entries"]`, `CHAPTER_TABLES=&["items",
  "excluded_scanlators"]`, `ANIME_LIBRARY_TABLES=&["entries","items","history","entry_categories"]`, history
  `&["history","items","entries"]`, `MANGA_SYNC/ANIME_SYNC=&["tracks"]`, sources `&["stub_sources"]`, category
  `&["categories","entry_categories"]`, relations `&["anime_relations","entries"]`).
- Inline SQL: `anime.rs:35-47` (library/seasons/deletable/children/parent reset/delete/category link consts),
  `flags_write.rs` (six consts, with `tr/un` for manga), `chapter.rs:119`, `history.rs:108,240,258`,
  `track.rs:179,188` (the two coalesced updates), `category.rs:108,135`, `anime_relation.rs:18`, test raw SQL in
  `tests.rs` (17 sites) and the small ones in `chapter.rs/track.rs/category.rs/anime_relation.rs/episode.rs` tests.
- Delete `extension_repo.rs`, `extension_store.rs` and their `lib.rs` lines.
- Seam out: unchanged pub API. (`kuro_data::extension_repo::ExtensionRepo` leaves: lane C changes its three users.)

**Lane C: downloads, track, backup (after A; parallel with B and D).**
- `crates/kuro-downloads/src/{store.rs, pending.rs, anime/store.rs, anime/pending.rs}` and `sql/*.sql`: the four stores
  read/write `download_queue` / `download_pending_delete` / `download_pending_corrupt` with the kind literal and
  `entry_id/unit_id` names (anime store fills `source` from `entries` in the insert: `INSERT ... SELECT`); `open(conn)`
  stops running `SCHEMA`; `open_in_memory()` applies the unified DDL for those three tables, exported by kuro-db as
  `kuro_db::schema_unified::DOWNLOAD_DDL` (same text as in the baseline; rusqlite builds with foreign keys ON, so a plain
  in-memory connection enforces them and tests must insert real entry ids). The unreadable guard and the other behaviours do not change (they are step 5).
  `testdata/` fixtures regenerate. Delete the four `sql/*.sql` files.
- `crates/kuro-track/src/queue/{store.rs, mod.rs}`: drop `CREATE` (the table exists); `kuro-track/src/{repo.rs, fakes.rs}`
  tests use `TestDb::handle`.
- `crates/kuro-backup`: `restore/driver.rs` (`Targets { db: &mut Connection, app_prefs, source_prefs, scheduler }`,
  both kinds' sections use the one connection, each entry still in its own transaction), `restore/categories.rs`
  (no change in logic; kind-pinned statements), `restore/manga.rs` and `restore/anime.rs` (raw SQL: memberships
  query, `UPDATE ... SET version`, tests), `creator/db.rs` (`Library { prefs, db, extensions_dir }`; both repos over
  one `Arc<Db>`), `creator/{meta.rs, mod.rs}` (drop `ExtensionRepo` use; `manga_extension_repos()` returns `Vec::new()`),
  `restore/extensions.rs` (unchanged), and the tests `tests/{restore_integration, restore_fixtures,
  create_restore_round_trip, end_to_end, restore_shikimori_bangumi}.rs` plus `fixtures_gen/mod.rs`. The backup wire format
  and the Aniyomi chapterFlags/episodeFlags values are unchanged on the wire: manga `chapter_flags` in `BackupManga`
  is the manga layout and the manga statements translate on write; no backup-edge translation in step 3.
- Seam out: `kuro_backup::Library { prefs, db, extensions_dir }`, `Targets { db, .. }`.

**Lane D: domain, src wiring (after A; parallel with B and C; its tests need B and C to be merged first).**
- `crates/kuro-domain`: raw SQL in tests of `history/repo.rs` (4), `updates/repo.rs` (3), `library/repo.rs` (2),
  `app/status.rs` and `app/domain.rs` (`TestDb`), `items/*` and `entries/*` tests (`kuro_db::statements_*` fixtures
  are unchanged but their `schema_*::open` calls move to `TestDb::handle`), `tests/domain_golden.rs`,
  `extension/trust.rs` (delete `MangaRepoKeys` and the `ExtensionRepoRepo` import; keep the `SigningKeys` trait if its
  tests still use it).
- `src/`: `data.rs` (one `Arc<Db>`, `legacy_files` log, `App.db`, `Opened`), `main.rs:160` (close one), `settings.rs`
  (187-193, 248-254, 300-301: counts via `kind`-filtered statements; `SELECT count(*) FROM {table}` becomes
  `FROM items JOIN entries ... WHERE kind = ?`), `cli.rs`, `backup_run.rs`, `backup_schedule.rs`, `backup_host.rs`,
  `library_update/{store.rs, sync.rs, metadata.rs}` (constructors take one `Arc<Db>`; raw SQL at store.rs:151-188),
  `misc/runner.rs` (`Runner::new(db)`, `SqliteLibraryStore::new(db)`, `DbMetadataStore::new(db)`),
  `settings_ui/{tracker_host.rs, library_export.rs}`, `player_view/{mod.rs, source.rs, skip_button.rs, controls/mod.rs,
  sheets/quality.rs}` and `screens.rs` (`app().anime_db` becomes `app().db`), `updates/{storage_data.rs:36 raw SQL, store.rs:153}`,
  `feed/query.rs:15-17` (one query on `updatesView` with `kind`), `entry_core/store.rs:33` (masked write with `tr`),
  `watch.rs:37` (`SELECT count(*) FROM categories` becomes `... WHERE _id > 0`, since there are now two system rows),
  `downloads/{mod.rs, anime_queue.rs, manga_io.rs}` (open the one file; the stand-in tables in the test at mod.rs:184
  go away), `ext_host.rs` (`start(store)` loses its `anime_db` argument and `store_rows`), `browse/migrate_live.rs`
  (statements unchanged; test fixture at 591 uses `TestDb`).
- Seam out: `App { db: Arc<Db>, manga_categories, anime_categories, .. }`; `App::db(kind)` is deleted (callers use
  `app.db`).

**Lane E: sweep (after A to D are merged; one agent, mechanical).** Replace `kuro_db::schema_manga::open(` and
`schema_anime::open(` + `Db::new` test setups in the remaining `src/**` and crate tests by `TestDb::handle(&dir)` or
`kuro_db::schema_unified::open_file`, `schema_manga::db_path` by `kuro_db::schema_unified::db_path`; delete the shims and
the `manga_handle/anime_handle` aliases; delete `kuro-db/src/tests_anime.rs` cases that moved to `tests/unified.rs`.
Then grep gate: no `schema_manga`, `manga.db`, `anime.db`, `anime_db`, `manga_db` outside the two intentional places
(`legacy_files` and the test that checks the old files are untouched). Files: the `src/**` test modules found by
`rg "schema_manga|schema_anime|manga_db|anime_db"` (about 40) plus `crates/kuro-{domain,extensions,track,backup}/tests`.

Order: A, then B + C + D in parallel worktrees (each only touches its list; none edits kuro-db), merge B, C, D, run the
whole suite, then E, run it again. After E the app builds, the full suite is green and step 3 is done; step 4 then
replaces the per-kind statements and the per-kind repos with `Entry/Item` and `EntryRepo/ItemRepo`.

---

## 6. Test plan

**Must keep passing (adapted only in setup, not in assertions):** all statement tests inside `statements_manga.rs`
and `statements_anime.rs`, `runtime.rs` (transactions, savepoints, WAL, dirty marker), `subscribe.rs`, `notify.rs`
(names updated), `open.rs` (corrupt file moved aside, quarantine, `restore_newest_backup`, newer-file refusal),
`codec`, `flags`, `episode_sanitizer`, `maintenance`; `kuro-data` `tests.rs` and per-repo tests; `kuro-backup`
`restore_integration`, `restore_fixtures`, `create_restore_round_trip`, `end_to_end`, `prefs_wire`, every restore
unit test in `restore/{manga,anime,categories,driver}.rs` (version/merge/E-25 rules); `kuro-domain` `domain_golden`
and the items/entries/history/updates/category tests; `kuro-downloads` store, pending, manager tests; `kuro-track`
queue and retry tests; `src/` library, updates, library_update, local_flow, reader writer, player store tests.
Gate for the train: `cargo build` and `cargo test --workspace` green at D and at E.

**Replaced:** `conformance.rs` Aniyomi inventory/golden tests, `tests/library_view_34.rs`, `schema.rs` bundle-refuses-
Aniyomi tests, `open.rs` Q28 / migration-34 / "latest version is 34" tests, `tests_anime.rs`
`duplicate_source_and_url_resolve_to_the_first_row` (now: the second insert fails with `Constraint`),
`source_ids_above_2_pow_63...` (kept, now over `entries`).

**New (each proves the merge), in `crates/kuro-db/tests/unified.rs` unless noted:**
1. `fresh_profile_creates_one_file`: `Db::open(TempPaths)` creates only `kurobayomi.db`; `user_version = 1`,
   `application_id = 0x4B555232`, the two system categories, no `manga.db`/`anime.db`; golden `tests/golden/unified_v1.txt`
   (object inventory + column list per table/view, the format of the old golden).
2. `old_files_are_untouched_and_logged_once`: pre-create `manga.db`, `anime.db` (junk bytes and an Aniyomi-shaped
   file) with `-wal`; open; their sha256 and mtimes are unchanged; `legacy_files` lists both; a second open of the
   existing profile reports nothing to log.
3. `local_manga_x_and_local_anime_x_both_appear`: insert manga `(source 0, url "X")` and anime `(0, "X")`;
   `libraryView` returns two rows with different `kind` and ids; a second manga `(0,"X")` fails; `MangaRepo::by_url_and_source("X",0)`
   returns the manga only, `AnimeRepo` the anime only; `stub_sources` holds `(manga,0)` and `(anime,0)`; both
   favourite-count-by-source queries count 1.
4. `categories_are_per_kind_and_links_cannot_cross`: manga category 1 and anime category 1 coexist; linking an
   anime to the manga category fails on the foreign key; deleting `(anime,0)` aborts; deleting a category drops its
   links without bumping `version`.
5. `history_search_folds_non_ascii_for_both_kinds`: manga "Éclair" found by "éclair", "ÉCLAIR"; anime "Привет" found
   by "привет"; `%` and `_` in the query match literally; a manga hit is not returned for the anime query (`kind`).
6. `progress_is_saved_for_a_chapter_and_an_episode`: update `items.progress`/`done` for one chapter and one episode,
   upsert history for both (`duration_ms` kept for the chapter), close and reopen the file: values, `libraryView`
   `doneCount`, `lastActivity`, `version` bumps (item and entry) are as expected; ids of the chapter and the episode differ.
7. `library_view_counts_match_the_old_views`: fixtures of the old `library_view_34` test (favourites, excluded
   scanlators, bookmarks) for manga plus the anime season rollup fixture (`fetch_type 0` parent, children, fully-seen
   count); `seasons_of` equals the rollup of `libraryView` for a favourite parent and works for a non-favourite one.
8. `manga_chapter_flags_round_trip_through_the_unified_layout`: every manga sorting value and masked write read back
   equal through `MangaRepo`, stored value has the superset sorting bits; anime flags untouched; `tr`/`un` property
   over all 0..0x1FFFFF manga-legal values.
9. `kuro-backup/tests/unified_restore.rs`: `restore_a_tachibk_with_both_sections_into_one_file`: a backup built by
   `fixtures_gen` with manga and anime entries, categories of both kinds with the same ids, history, tracks per kind,
   a local "X" in each; restore with `Targets { db }`; assert counts per kind, per-kind category membership, tracks on
   the right entries, no cross-kind rows, and a second restore is idempotent. A create-then-restore round trip into a
   fresh profile diffs library/history/tracks/categories (existing `create_restore_round_trip` adapted).
10. `kuro-downloads`: `download_queue_rows_cascade_with_the_item_and_pending_rows_do_not`; both stores round-trip with
    `kind`; `kuro-track`: `retry_queue_lives_in_the_unified_file`.
11. Repo-level in `kuro-data/src/tests.rs`: `manga_and_anime_repos_on_one_handle_do_not_see_each_other` (same `Arc<Db>`,
    same `(source,url)`, ids differ, `favorites()` of each kind only its own, subscriptions fire on a write of either kind
    and publish only when their own result changed).

Real flow (run on the test display as the map's step rule requires): fresh profile launches; Restore of an Aniyomi
backup with both sections; Library shows both kinds with counts; local manga "X" and local anime "X" both listed;
History search "é"; open a chapter and an episode, progress persists after restart.

---

## 7. Risks and rollback

| Risk | Mitigation |
|---|---|
| Hand-rewriting ~4,300 lines of SQL per-kind facades: a missed column or alias breaks a mapper at runtime, not compile time | by-name `from_row`s plus `CHECK`s; `conformance`-style test prepares every statement const against the new baseline and round-trips a full row per struct (copy of the old `every_statement_prepares_against_the_baseline`); lane A does not merge until its crate tests pass |
| Raw SQL strings outside kuro-db (about 45 sites: kuro-data, downloads, backup, domain, src, their tests) are invisible to the compiler | the lists in 5.2 come from a grep of `SELECT|INSERT INTO|UPDATE .* SET|DELETE FROM` and of the old table names; lane E ends with a grep gate for the old names; the train is only merged when the full suite is green |
| `UNIQUE(kind, source, url)` breaks fixtures or a production path that inserted duplicates | production inserts go through lookup-or-insert; failing tests are fixed in their lane; surface any production caller found in review (`rg "\.insert\(&"` in `src/browse`, `entry_core`, `kuro-domain/entries`) before merging D |
| Manga `item_flags` translation off by one bit | test 8 covers every legal value and the masked-write SQL; both `tr` and `un` exist in Rust and SQL and a test checks they agree |
| Wake-ups: one file means manga writes wake anime live queries | results are compared before publishing; if profiling shows churn (library screen) add per-kind tables to the subscription key in step 4 |
| `libraryView` prefilter on `parent_id` children is new for anime | test 7 compares against the unfiltered aggregate; benchmark test from `library_view_34.rs` is ported (9,000 entries, budget unchanged) |
| Restore uses a separate raw connection, so live queries are not notified by it (existing behaviour) | unchanged; the app reloads after a restore as today |
| User loses the library on upgrade (fresh schema, decided) | the old files stay; one log line; advise a backup from the old build first. Not in 3a: a one-time toast and a "restore from my last auto-backup" hint (needs an i18n key) |
| Download queue and unfinished pending deletes of the old files are dropped | by decision; files on disk are untouched and the disk index rediscovers them |
| Category ids per kind keep two id spaces | accepted: it is what Aniyomi prefs and every caller assume; `CategoryService` stays per kind until step 4 |

**Rollback point:** the last commit of step 2 (`kuro_platform::kind::Kind` everywhere). Because the new file has a
different name, an old build pointed at the same profile still opens `manga.db` and `anime.db` untouched; reverting
lanes A to E is a plain revert of the train, nothing in the old files was written. Data entered into
`kurobayomi.db` after the switch is lost on rollback (it can be saved first with Create backup, which works in the
new build). Inside the train the fallback is per lane: A alone does not change behaviour for crates above it (shims),
so A can sit on the branch while B to D are redone.
