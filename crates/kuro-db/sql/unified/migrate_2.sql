-- Version 2: the community score of an entry (the public score of a catalogue site, MyAnimeList through Jikan today).
-- A cache: it is fetched again when it is old, and it is never part of a backup. One row per entry at most; a row with
-- no remote_id records that the entry was looked up and not found, so it is not asked again on every run.
CREATE TABLE community_scores(
    entry_id      INTEGER NOT NULL PRIMARY KEY REFERENCES entries(_id) ON DELETE CASCADE,
    provider      TEXT    NOT NULL,                -- 'jikan'
    remote_id     INTEGER,                         -- the provider's id of the title (a MyAnimeList id); NULL: not found
    matched_title TEXT    NOT NULL DEFAULT '',     -- the title the provider gave it, shown so a wrong match is seen
    matched_by    TEXT    NOT NULL CHECK (matched_by IN ('tracker', 'title', 'user')),
    score         REAL,                            -- 0 to 10; NULL: not found, or the site has no score for it yet
    fetched_at    INTEGER NOT NULL                 -- epoch ms
);
