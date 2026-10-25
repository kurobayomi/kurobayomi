# The app flow (target behavior, decided by the user on 2026-10-11)

This is how a normal user uses Kurobayomi. Every screen, setting and feature must serve this flow; what does not serve it
is cut (see `CLAUDE.md`, design principles).

No limit on how many sources a repository lists (user, 2026-10-23: the old cap of 10 per kind is gone). There is no source Install step: adding a repository
link makes its sources available at once; each source that is on has its small file fetched and checked
when it is turned on and refreshed by the daily update check. A source that is off is not on the computer (2026-10-23).

## First run, once

1. Open the app and see an empty state with one clear button: "Add a source repository".
2. Paste a link. A list of the repository's sources opens, each with a switch; the ones turned on appear in Browse at once
   (grouped by language, with a language filter); there is no Install button. Sources start off (user, 2026-10-23), so a
   repository with many sources does not fill Browse; when the daily check finds new sources, a notice opens the same list.
3. Optional and skippable: log in to AniList.

## Finding something to watch or read

1. Browse: pick a source and see Popular and Latest as a cover grid.
2. Search: one search box across all sources. This is global search, and it is the main way in.
3. Details page: cover, description, the episode or chapter list, and a big Start or Resume button.
4. Add to library: one tap, with an optional category.

## Watching an anime

1. Click Start. A spinner shows while it loads.
2. The video plays inside the window, with sound and the user's subtitle language chosen automatically.
3. The player needs only a few controls: pause, seek, volume, fullscreen, previous and next episode, a quality and server menu,
   a subtitle and audio menu, and speed.
4. At the end it moves to the next episode if the user wants, and marks the episode seen. The position is remembered.

## Reading a manga

1. Click Start and the reader opens in the window, with the themed top bar always visible (the same header as the player).
2. The user flips pages with arrow keys, space or the mouse wheel, in a default direction that feels natural. Right-to-left is an
   option, and the reader remembers it per manga.
3. The last page flows into the next chapter. Progress is saved.

## Coming back

- Library: shows what has new chapters or episodes, with a badge count.
- Updates: shows what appeared since last time.
- History: shows what you were in the middle of, with Continue.

## Tracking

- AniList only for now (the other trackers stay switched off). The user logs in once, optionally.
- Watching and reading progress is sent to AniList automatically; the details page can set status and score.
- Optional auto-track (off by default): a title added to the library is bound on AniList when the match is certain
  (`docs/design/status-categories.md`).

## Rarely

- Downloads: the user taps download on a few items to read or watch offline.
- Backup: automatic backup plus restore. Aniyomi backup import is key for switching.

That is the whole app. Next step: decide which settings survive (`docs/design/settings-diet.md`).
