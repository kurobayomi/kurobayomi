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

## Fix pass (audit of the first Nightly, 2026-11-21)

The first build of Home (7d5e27e0) was checked against the Library, from the user's screenshots (wide and narrow window)
and the code (`library/view/card.rs`, the Library cell in `library/view/mod.rs`, `widgets/cover_grid.rs`, `home/mod.rs`).
Home drew its own cards instead of the Library's, and it shows.

### What is wrong, and why

| # | Seen | Cause in the code |
|---|---|---|
| 1 | Narrow window: the row is cut off, "See all history" is clipped | The row is a plain horizontal box: its minimum width is the sum of every card, so the page cannot get narrower than the row. Hiding covers that do not fit never helps, because the width it measures can never be smaller than the row itself. |
| 2 | A long title widens its card and the cover's frame | The title label asks for its whole text width; the cover frame stretches to the card. |
| 3 | The title sits right of the line under it | The title is a flat button, with a button's padding. |
| 4 | No community score, no unread count | Home builds its own card with only a kind tag. The Library card shows the unread count (top start) and the score with a star, then Local or the source language (top end). |
| 5 | The cover size setting is ignored after start, and the display mode not at all | Home reads the cover size once per card and never follows a change; the Library follows `cover_size` live and draws the title by `display_mode` (on the cover, under it, or none). |
| 6 | No progress bar for "25 min left" | The bar uses the faint `osd` style, inside a frame that is wider than the cover (2). |
| 7 | The top bar is empty | Home's header has only the window buttons. |
| 8 | Nothing shows while the rows load | No loading state: the page is blank until the four queries answer. |
| 9 | Home rebuilds every few seconds during playback, even when it is not on screen | The History query follows the `items` table, which the player writes as it saves the position; each change rebuilds every card and reads the database again on the GTK thread. |
| 10 | Covers cannot be reached with the keyboard | Clicks are gesture handlers on plain boxes; nothing is focusable or activatable. |

### The fix

1. **The Library's card, not a copy.** Home builds each cover with `library::view::card::build` (made shareable), from a
   `LibraryItem` per title: library titles from the Library's own read (`library::load::snapshot`, so the unread,
   download, Local and language badges and the score are exactly the Library's); a title only in the history gets the
   same row read from its entry, with its unread count. The badge switches are the Library's (`load::badges`).
2. **The Library's settings, followed live:** cover size (bound like the Library binds it), display mode (title on the
   cover, under it, or none; in List mode Home uses the comfortable grid, since a list of one row each is not a list),
   and the continue button rule.
3. **Home's own line under the card**, kept short: "Episode 1 · 25 min left", "Chapter 1199 · page 9", or the oldest new
   unit for New for you, with the count of new units as the unread badge. The anime progress bar is the theme's normal
   bar at the foot of the cover.
4. **Responsive like the Library:** each row is a flow box whose minimum width is one card, cells of the Library's
   column width and margins (`cover_grid`), and as many covers as fit the row's own width (the Library's column rule,
   shared from `cover_grid`, so both count alike). No sideways scrolling, nothing clipped.
5. **Header:** "Home" as the page's title at the start, global search, and Refresh, which starts the library update of
   both kinds and turns while it runs (the Updates page's button, shared).
6. **Loading:** the page's spinner until the first answers (the shared state switcher, as every page has).
7. **Quiet while hidden:** changes mark Home stale; it redraws when it is on screen, and on screen only when what it
   shows changed (the titles, units, progress or badges of the shown covers), not on every write.
8. **Keyboard:** covers are flow box children; Enter and a click both open, as in the Library.

### Checked before it is committed

- Tests: the shared column rule (Home and Library agree at every width), the item built for a history-only title, the
  display modes, the stale-while-hidden rule, the header actions.
- On screen, in a local build, with screenshots: a wide window, a narrow window (two covers, nothing clipped), each
  display mode, a cover size change, and the welcome view on an empty profile.
