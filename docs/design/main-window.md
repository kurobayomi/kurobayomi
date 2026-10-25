# Main window

The window shell (sidebar, page header) and the Library page. Plain GTK 4, no libadwaita: every colour comes from
the user's GTK theme (named colours `@theme_*` only, in `src/theme/style.css`), every widget is a stock GTK widget.

## Audit of the old window

| Where | What the user saw | Problem |
| --- | --- | --- |
| Sidebar header | collapse toggle + "Kurobayomi" | The toggle and the icon-only rail add a mode nobody asked for. |
| Sidebar list | Library, Updates, History, Browse, More centred vertically | Items float mid-column with empty space above; the eye looks for nav at the top. |
| Page header | title "Library" + add folder, rescan, filter, menu | "Library" repeats the sidebar; four icons with no words. |
| Under it | Anime / Manga switch | A second header row for one control. |
| Under that | big "Anime" heading, search, a second refresh | Third title; the rescan and the update-category refresh look identical. |
| Card | "6" top-left, folder icon top-right, grey image icon, title | Mostly placeholder; badges with no words and no style class the theme knows. |

## New design

### Sidebar

- Fixed width (`widgets::rail::WIDTH`, 200 px). No collapse toggle, no icon-only mode, no width follower.
- Header: the window buttons of the left side and the app name, like a GNOME split header.
- Nav list (`navigation-sidebar`) starts at the top. Each row: icon, label, then a badge pill (`kb-nav-badge`)
  for Updates (new updates) and Browse (extension updates). The pill is invisible at zero and keeps its place.
- Below 720 px the sidebar hides and the existing bottom bar (stack switcher) takes over; selection mode still
  hides the sidebar.

### Library header (one row)

`[Anime | Manga]   [search] [categories] [refresh] [link folder] [open Local folder] [view options]`

- Start: the Anime / Manga switch (a `GtkStackSwitcher` over the two library views).
- End: icon buttons, each with a tooltip: the search toggle first, as on every page (user, 2026-10-25; in the phone layout it moves to the start of the row instead), then Categories, Refresh (updates the whole library; insensitive while a pass
  runs), Link a folder…, Open the Local folder, and View options (filter, sort, display; it carries `kb-filter-on`
  while a filter is on).
- No ⋮ menu on any tab page (user, 2026-10-25): every action is a header button. Updating one category is F5 (the
  open one) and the right-click menu of a category tab (with more than one category).
- No page title: the sidebar already says Library, the switch says which kind.
- While entries are selected, search and refresh hide and the selection row (count, select all, invert, cancel)
  shows under the header; the action bar (categories, mark, download, delete) slides up at the bottom.

### Under the header

- Category tabs (when the setting is on and there is more than one category).
- A small context line (`kb-library-context`) only when it says something the switch does not: the category name
  and counts when the tabs are hidden. Never a second "Anime".
- The grid.

### Card

- 2:3 cover, 8 px corners (`kb-card-cover`), title under it in two lines (`kb-card-title`). Compact grid puts the
  title on the cover over a theme-coloured fade; cover-only shows no title; list mode is a row.
- Cover, in order: the entry's stored thumbnail (`file://` covers draw at once; remote covers load through the image
  service once the app installs one), otherwise a tile with the title's initials (`kb-cover-initials`, the theme's
  selected colour at low alpha).
- Local anime without a cover: the first time the card is shown, the folder is searched for `cover.*`; when none
  exists, `kuro_local::media::FrameExtractor` takes the middle frame of the lowest episode and writes `cover.jpg`
  in the series folder (what Aniyomi's local source does). The row's `thumbnail_url` is then stored, so it runs
  once per entry, and only one frame is taken at a time. No ffmpeg installed means the initials tile stays.
- Badges say what they count:
  - top start: `6 unwatched` / `6 unread` (`kb-badge kb-badge-unread`, selected colour), `3 downloaded`
    (`kb-badge`); a season parent's tooltip says the count is seasons.
  - top end: `Local` with a folder icon for local entries, else the source language (`EN`).
  - bottom end: the continue button (`circular osd`), when enabled.

### Empty state

A large dim icon, the message (`Your library is empty`, `No results found`, …), and the matching action: Search
globally for an empty search, the getting-started guide for an empty library.

### Spacing scale

4, 8, 12, 16, 24 px (`library::layout::SPACE_*`). Card gap 8 (12 between rows), page side margins 16, header
padding from the theme.

## Kept features

Categories and their tabs, unread / download / local / language badges, sort, filter and display settings,
selection (click with Ctrl or Shift, long press, keyboard shortcuts) with its action bar, continue button, global
search from an empty search, random entry, update library / category, local folder add and rescan, the narrow-window
bottom bar.

## Not done here

- Remote covers need the image service installed at startup (`entry_core::cover_view::install_images`); that needs
  the network client and source headers from the sources lane. The card already uses `CoverView` when the service is
  present.
