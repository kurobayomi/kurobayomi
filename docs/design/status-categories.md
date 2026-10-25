# Watching, Completed and auto-track

Decided with the user on 2026-10-25. Replaces an earlier draft with fixed status categories, which was dropped as too
much: the library already holds the progress that tells what you are watching and what you finished.

## Library: Show only

The library's view options menu shows: **Downloaded, Watching (Reading for manga), Completed, Tracked**.

- **Watching** = started, with episodes or chapters left. It replaces the Unwatched / Unread check and is stored under
  that check's old key (`pref_filter_*_unread_v2`).
- **Completed** = started, with nothing left: the user finished it. It no longer looks at the series status, and it
  ignores airing: a caught-up show that gets a new episode is back under Watching on its own. Stored under the old
  completed key (`pref_filter_*_completed_v2`).
- Both on = every started title. Nothing is stored per title, so nothing goes stale.
- Works the same with or without a tracker. Categories are unchanged: no default categories, no migration.
- Code: `src/library/view/options.rs` (the checks), `src/library/load.rs` (`filters`: the two checks become the
  domain's started and unread filters; the series-status filter stays off).

## Auto-track

- One switch in Settings > Tracking: **Track new titles automatically**, off by default. It can be turned on only
  while AniList is logged in. Stored as `auto_track_library` (not an Aniyomi key, so it lives in `[extra]`).
- When on, a title that joins the library (title page or History) is searched on AniList. It is bound only when
  exactly one result has a name equal to the title after tidying (case, punctuation, a trailing Dub / Sub, "2nd
  Season" read as "Season 2"), compared against all of AniList's names (display, romaji, English, native, synonyms).
- No match, or more than one: nothing happens; the user picks on the tracking page as before. A bind shows a toast
  "Tracking *title* on AniList".
- The bind is the normal one: a show already on the user's AniList list keeps its status and score; otherwise it is
  added as Planning (or Watching when something was seen) and the progress syncs.
- AniList only; MyAnimeList is not auto-tracked. No "track my whole library" run for now.
- Code: `crates/kuro-track/src/auto_bind.rs` (match and bind), `tracker/anilist/queries.rs` and `dto.rs` (search asks
  for every name), `model/search.rs` (`alt_titles`), `src/updates/track_binding.rs` (runs after an add),
  `src/settings_ui/tracking.rs` (the switch).

## Not done, on purpose

- No status categories, no category sync, no Dropped tab, no import of the AniList list.
- Removing a title from the library always stays a user action.
