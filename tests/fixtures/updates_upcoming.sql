-- Fixture of the Updates feed edge cases E-U1 to E-U4, loaded by
-- src/updates/store.rs into an empty manga database. The feed window cut-off the tests use is 1_700_000_000_000.
-- M1 is a favourite added at 1_700_000_000_000; M2 is not a favourite.
INSERT INTO entries(_id, kind, source, url, title, status, favorite, initialized, viewer, item_flags, cover_last_modified, date_added)
VALUES (1, 'manga', 1, 'm1', 'M1', 1, 1, 0, 0, 0, 0, 1700000000000),
       (2, 'manga', 1, 'm2', 'M2', 1, 0, 0, 0, 0, 0, 1700000000000);

-- C1: the one update (fetched after M1 was added, uploaded after the cut-off).
-- C2: upload date unknown (0), so not an update (E-U1).
-- C3: fetched before M1 was added, so a backfill (E-U2).
-- C4: a recent chapter of M2, which is not a favourite (E-U3).
INSERT INTO items(_id, entry_id, url, name, scanlator, done, bookmark, progress, number, source_order, date_fetch, date_upload)
VALUES (1, 1, 'c1', 'C1', NULL, 0, 0, 0, 1, 0, 1760000000000, 1759000000000),
       (2, 1, 'c2', 'C2', NULL, 0, 0, 0, 2, 1, 1760000000000, 0),
       (3, 1, 'c3', 'C3', NULL, 0, 0, 0, 3, 2, 1600000000000, 1759000000000),
       (4, 2, 'c4', 'C4', NULL, 0, 0, 0, 1, 0, 1760000000000, 1759000000000);
