-- Version 3: how many pages a chapter has, written by the reader with the last page read, so Home can show how far
-- through a chapter one is (pages read / page count), as it does for an episode (seconds / length). NULL: not known
-- yet (never opened in the reader since this version) and for episodes. Not user state: it does not bump `version`.
ALTER TABLE items ADD COLUMN page_count INTEGER;
