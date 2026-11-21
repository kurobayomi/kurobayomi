# Home: plan (accepted by the user, 2026-11-21)

Accepted as written, with the four defaults below. Nothing here is built yet. It changes the "Coming back" part of
`docs/design/app-flow.md`, which is updated in step 4.

## Why

Coming back is spread over three screens today: Library (badges), Updates (what appeared) and History (what you were in
the middle of). A returning user wants two answers, "where was I" and "what is new for me", and has to visit two screens,
each split by kind, to get them. Home puts both on the page the app opens on, for anime and manga together.

Design principle 2 (one way to do each thing) decides the shape: Home **replaces** Updates and History in the sidebar
instead of joining them. Their full screens stay, one click away, behind "See all".

## What Home shows

The page has the usual top bar (window buttons, draggable; top bar rule in `CLAUDE.md`), then two rows of covers.

### Continue

- What the user is in the middle of, anime and manga together, most recent first: the newest rows of the History table
  of both kinds, one per title.
- A title counts only while something is left: a finished last episode or last chapter drops it from the row.
- Each card shows the cover, a small kind tag (Anime or Manga), the title and one line: "Episode 7 · 14 min left" or
  "Chapter 12 · page 8". Anime also gets a progress bar. A chapter's page count is not stored, so manga shows the page
  only (no "of 22", no bar).
- A click resumes exactly as a History row does (same code: `updates::history_screen`'s resume, with
  `updates::history_data::ordered_units` and `updates::history::next_units_from`), straight into the player or the
  reader. A click on the title opens the title's page.
- The footer link "See all history" opens the History screen.

### New for you

- New episodes and chapters of **library** titles only, newest first, one card per title: the rows of the Updates feed
  of both kinds, grouped by title. A source's general "Latest" stays in Browse, where it is now.
- Each card shows the cover, the kind tag, a count pill ("+2"), the title and one line: the oldest new unit's name.
- A click opens that oldest new unit, so nothing is skipped; the title opens the title's page.
- The footer link "See all updates" opens the Updates screen.

### Row size

One row each, as many cards as fit the window width (the Library's cover size), no sideways scrolling. Everything else
is behind "See all".

### Empty states

- Both rows empty (a new user): one line and one button, "Find something to watch or read", which opens Browse.
- One row empty: that row is left out; no empty box.

## The window around it

- Sidebar: **Home, Library, Browse, Downloads**, then Settings at the bottom. Updates and History leave the sidebar and
  open from Home's "See all" links (each with its back arrow to Home).
- The app opens on Home. The **Start screen** setting (`kuro_prefs::ui::StartScreen`, Settings → Appearance) is cut,
  under design principle 1: with Home first, it has no reason left. A stored value is ignored, not an error (design
  principle 3: old backups keep restoring).
- The Anime/Manga switch is not on Home: both kinds are mixed, told apart by the kind tag. Library, Browse, Updates and
  History keep their switch.
- Badges: the count of new units that the Updates item showed in the sidebar moves to the Home item.

## How it is built

- A new screen module `src/home/` (its own folder, so a lane can own it), registered in the sidebar like the others
  (`shell::sidebar`, `shell::tabs`: a new tab id `home`).
- It reads the same queries the History and Updates screens read, through their existing data modules, for both kinds,
  and merges them by time. No new table, no new stored setting.
- It follows changes the same way those screens do (the table-change signal with the 50 ms coalesce), so a chapter read
  elsewhere updates Continue at once.
- The cards reuse the Library's cover widget (`look::cover` and the shared cover cache), so a cover seen anywhere is not
  downloaded again.
- Strings: new keys in both `kurobayomi-extra.ftl` and `kurobayomi.ftl`, with `MSG_COUNT` in `crates/kuro-i18n/src/tests.rs`
  updated; removed keys (the Start screen row) leave both files too.

## Tests

- Merging: rows of both kinds come out in time order; one card per title; a finished title is left out.
- Continue click: the same unit the History screen would resume (shared code, one test on the shared function).
- New for you: units of one title are grouped and counted; a title outside the library never appears.
- Empty states: both empty shows the Browse button; one empty row is left out.
- Sidebar guard: Home is first, Updates and History are not sidebar items, and both screens are still reachable.
- Header guard: Home has a HeaderBar with the window buttons (the existing guard test covers new pages).

## Steps

1. Home screen with both rows, reading the existing data (no sidebar change yet; reachable as a new first item).
2. Sidebar change: Updates and History move behind "See all"; the badge moves to Home.
3. Cut the Start screen setting and its strings.
4. Update `docs/design/app-flow.md` ("Coming back") and the getting-started guide.

## Decisions (user, 2026-11-21: "accept the defaults")

1. "New for you" shows library titles only; each source's newest titles stay in Browse.
2. Updates and History leave the sidebar and open from Home's "See all" links.
3. One row each, fitted to the window, no sideways scrolling.
4. The Start screen setting is cut.
