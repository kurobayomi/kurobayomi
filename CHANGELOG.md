# Changelog

All notable changes to Kurobayomi. The format follows [Keep a Changelog](https://keepachangelog.com/), and the versions
follow [Semantic Versioning](https://semver.org/). A release is cut from the section of its version (`scripts/release-notes.sh`
reads it), so the section must exist and have notes before its tag is pushed (see `docs/RELEASING.md`).

## [Unreleased]

### Added

- Downloads lists every downloaded episode and chapter under the queue, each with a button that opens its folder, and
  the header has a button that opens the downloads folder.

### Changed

- Windows: a downloaded update says it is ready ("install it now, or it installs when you close the app") instead of
  installed, and its button is Install now instead of Restart, as installing takes a few seconds there. Install now
  shows the setup's progress window until the new version starts; a plain quit still installs without a window.
- Every page's search button is the first of the buttons on the right of its top bar, instead of at the start of the
  bar (or beside the app name over the sidebar). In the phone layout it sits at the start of the bar.
- A source's view options match the Library's: View (Grid or List), Grid style (Compact, Comfortable, Cover only) and
  Cover size. The look is remembered for every source, separately from the Library's.
- The ⋮ menu beside the app name is gone: Library, Updates, History and Downloads show its actions as buttons in the
  page header (link a folder, open the Local folder, Upcoming, Clear history, Pause or Resume and Clear queue, the last
  two only while something is queued). Update category is F5 or a category tab's right-click menu.

## [0.2.2] - 2026-10-25

### Fixed

- Windows: an update left the old version in place, after Restart and after a plain quit. GLib's session bus helper
  (`gdbus.exe`) outlives the app and keeps some of its files open, and the silent setup gave up when it could not close
  it; the setup now closes whatever still uses the app's files. An installed 0.2.0 or 0.2.1 gets this from the 0.2.2
  setup it downloads, so it updates without a reinstall.

## [0.2.1] - 2026-10-25

### Added

- A Cover size slider in the Library's view menu sets how big the covers of the Library and Browse grids are, from 100
  to 320 px wide; the covers resize while it moves and the number of columns follows the window.
- Reader settings per manga: the reader's own settings (reading mode, background, scale type, zoom start, crop borders,
  split wide pages) and its crop button now change only the open manga. Settings > Reader holds the defaults that every
  manga follows until a value is picked for it. A reading mode picked in the reader is kept in the manga, where Aniyomi
  backups keep it too.

### Changed

- On a wide window the info pane of a title page (and its cover) grows with the window, at 30% of the width, instead of
  staying 360 px wide beside an empty page.
- The blurred cover behind a title's header shows clearly, fading into the window background only at the bottom.

### Fixed

- Windows: sources behind a site's browser check (Cloudflare) work; they answered with the check before, because the
  hidden pages that pass it had no Windows engine. They now run through WebView2.

## [0.2.0] - 2026-10-24

### Added

- Windows 10 and 11 (64-bit): `Kurobayomi-Setup-x86_64.exe` installs Kurobayomi for the current user, without an
  administrator prompt, with a Start menu entry and an uninstaller. The installed copy updates itself like the AppImage:
  it downloads the new setup and runs it when the app quits, or at once from the Restart button. The stable installer
  takes `kurobayomi://` links (tracker sign-in comes back through one) and opens `.tachibk` backups.
- A Windows nightly, `Kurobayomi-Nightly-Setup-x86_64.exe` on the nightly release. It installs beside the stable app with
  its own name and data, and leaves `kurobayomi://` links and `.tachibk` files to the stable app.
- The player's HW and HW+ decoding also use Direct3D 11 where mpv offers it.
- The AppImage and the Windows installer come with ffmpeg and ffprobe, so anime downloads and local video previews work
  without installing ffmpeg. The app uses the one next to it first, then one on `PATH`.
- Help opens Kurobayomi's own help on the website: an FAQ (general, library, updates, browse, downloads, player, reader,
  storage) and guides (troubleshooting, backups, tracking, categories, local folders, source migration, keyboard
  shortcuts). The other help buttons (Upcoming, migration, update errors, the large-update warning) open its pages
  instead of Aniyomi's.

### Changed

- Data and storage lists the same storage roots wherever it is opened: the mounted disks and the one that holds the app's
  data (on Windows, the fixed and removable drives).
- Anime downloads take the player's Default quality, else the source's best video, as playing does.
- Keyboard shortcuts left the side panel and the menu; F1 or ? still show them on a title page, and the help lists
  them all.
- The large download and update warnings and the update-failure notice say Kurobayomi and Click, not Aniyomi and Tap.

### Fixed

- A GTK theme written for a newer GTK (adw-gtk3 6 writes its stylesheet in CSS variables) no longer leaves the window
  see-through: on a GTK older than 4.16 such a theme falls back to Adwaita, or Adwaita-dark for a dark one, for that run
  (issue #5).
- Open the Local folder, and showing the crash log or the update error log, failed in the AppImage
  ("gio-launch-desktop") and on Windows; they open through the system's own opener now.

## [0.1.10] - 2026-10-24

### Changed

- The Repositories page lists each repository's sources under its card, with their switches; the card's chevron folds
  them away. The page's search finds a source by name, language, kind or description. The Sources button is gone.
- The list of a repository's sources that opens after adding it has a search field; typing anywhere in it searches.
- Two filters on the Repositories page narrow the sources to anime or manga, and to one language.
- Browse has a Repositories button before Refresh, and no more menu: its Global search did what the search button does.
- The Library's Categories is a button before Refresh instead of a menu item.

### Fixed

- Open the Local folder failed with "The application launch failed" on KDE: it opens in the folder's default program
  now.
- A link from a source (a title's page, a description) opens only when it is a web or mail link: a link to a file or
  folder of this computer, or another program's scheme, is never opened.
- The Repositories button and the folded chevron showed the missing-icon picture; both are bundled now.

## [0.1.9] - 2026-10-24

### Added

- A repository can give each source a short description; it shows under the source's name in Browse and in the list
  of a repository's sources.

### Fixed

- Senkuro (and any source that sends its own Content-Type) failed with "isn't working right now": the request went out
  with the header twice, and Senkuro's server answers that with HTTP 400. A request now carries one Content-Type.

### Removed

- Kurobayomi no longer opens `aniyomi://` and `tachiyomi://` links, the "Add to Aniyomi" and "Add to Tachiyomi" buttons
  of extension sites: those repositories hold Android extensions, which Kurobayomi cannot use. An "Add to Kurobayomi"
  button (`kurobayomi://add-repo`) still opens the add dialog with its link.

## [0.1.8] - 2026-10-23

### Fixed

- Turning a source on works again: every switch from off to on (the Sources list, Browse's Turn on, the reader's and
  the title page's offer) failed with "The source could not be downloaded" without trying, since 0.1.5.
- A source you turn on shows in Browse even when its language was not on: turning it on turns its language on too
  (a Russian source on an English system was on but never listed). Sources already on get their language at the next
  start.
- Updating from 0.1.4 or older turns on only the sources you used (titles in your library or history), not every
  source your repositories list; the others start off and are in each repository's Sources list.

## [0.1.7] - 2026-10-23

### Fixed

- Security: sources can no longer reach this computer or your home network. Anyone may publish a source, so every request a
  source makes (plug-ins, recipes, the covers and downloads it names) goes only to internet addresses: an address of
  this computer (127.0.0.1, localhost), of a home or private network (192.168.x, 10.x, ...) or a link-local one is
  refused, whether it is written in the link, a site's name points to it or a redirect leads to it.
- Security: the player takes only internet video, audio and subtitle links from an online source: a link of another kind (a
  local file, a special player address) or to a local address is refused with a message, and the HLS relay fetches
  from the internet only. The local source still plays the files of your own folders.

## [0.1.6] - 2026-10-23

### Changed

- Kurobayomi has moved to its own organization: https://github.com/kurobayomi/kurobayomi. Updates, the Help and
  Getting started links and bug reports use the new address. The old one redirects, so nothing needs doing.
- A source repository that moves takes you along: when its index names a new address and that address works, the app
  switches the saved link by itself on the next start, keeps the sources you turned on and downloads nothing again.
  Adding an old address saves the new one.
- The README no longer names any source repository; the app ships with none.

### Fixed

- Links that start with `kurobayomi://add-repo?url=` (a repository website's "Add to Kurobayomi" button) open the add
  dialog with the address filled in, like the `aniyomi://` and `tachiyomi://` links; you still confirm.

## [0.1.5] - 2026-10-23

### Added

- Every source has its own on/off switch. Right after you add a repository, a list of its sources opens so you can turn
  on the ones you want; the repository's card has a Sources button that opens the same list. When a repository adds new
  sources, a notice says so and opens the list.
- Check now (a source's right-click menu in Browse) downloads the source again and loads its first page from your own
  connection, then shows its status dot from that result: green works, orange blocked, red broken.
- Browse and search remember their last results for about a day: opening a source again, going back to it or repeating
  a search (global search too) shows them at once while the site is asked again, and the list updates in place.
- Subtitle files of streamed episodes are kept in the cache, so reopening an episode does not download them again.
- A chapter's or episode's right-click menu has Reload, which fetches its pages or subtitle files from the site again.
- One cache size limit in Settings › Data and storage covers chapter pages, covers, subtitle files and browse results;
  when it is full the least recently used files go first. Library covers and downloads are never removed.

### Changed

- A source that is off is not on your computer: it is not downloaded or updated, and Browse, global search and the
  library update leave it out. Its titles stay in the library and their downloads still open; anything that needs the
  site says "This source is off" with a Turn on button. Turning off a source with library titles asks first.
- New sources start off. After updating, every source you already had stays on.
- Restoring an Aniyomi backup turns on every source except the ones Aniyomi had disabled.
- A repository may list any number of sources (the limit of 10 of each kind is gone).
- The player lists every audio and subtitle language again, not only English, Arabic, French, Spanish and Japanese.
- Opening a title shows its saved episode or chapter list at once and always asks the site again in the background;
  new episodes and chapters appear when it answers.
- Covers are checked again every few days with a conditional request and downloaded again only when they changed.
- Local titles' covers load through the app's image cache instead of being decoded by the toolkit each time.
- Clear cache (Data and storage) empties every cache, not only chapter pages.

### Fixed

- Quitting while a slow request was still running (a cover, a source such as AnimePahe behind its browser check) no
  longer shows the crash window. This covers every source and image load, not only the plug-in requests fixed in 0.1.4.
- A title saved while a source listed the wrong picture (AnimePahe once gave an episode screenshot) no longer opens with
  that picture: the next listing corrects the stored cover, and a poster the source finds later is stored and cached
  under its real address.
- A cover seen in Browse is no longer downloaded a second time when its title opens, and a cover the site changed shows
  its new picture without a restart.

### Removed

- Pinning sources.

## [0.1.4] - 2026-10-22

### Changed

- The reading modes have Aniyomi's names: Left to right, Right to left, Vertical, Webtoon and Continuous vertical.
- Ctrl+Shift+O follows the Anime | Manga choice: on the Anime tab it adds a local anime folder.
- The reader's tap-zone overlay fades out on its own two seconds after it is shown.

### Fixed

- A zoomed page in Left to right, Right to left or Vertical no longer slides on to the next page while you pan it. A
  drag, a touchpad swipe or the wheel moves the image; a new drag or swipe started at its edge turns the page.
- Ctrl+scroll zooms: the page in the paged modes, the whole strip in Webtoon.
- Webtoon: no dark line where each page starts (issue 4).
- Switching to Webtoon no longer steps back a page.
- A local title whose folder is missing (a drive not mounted, a folder moved) keeps its chapters, episodes and progress
  until the folder is back, instead of losing them in the next library update.
- Quitting while a source was still loading (a library update, a search) showed the crash window.
- A lone global-search result no longer stretches its cover to its title's width.

## [0.1.3] - 2026-10-22

### Fixed

- Adding a link that is not a Kurobayomi repository (a Mihon, Tachiyomi or Aniyomi one, including their newer `index.pb`)
  now says so plainly, instead of "The repo index is not valid".
- The downloads index rescanned its folders every 10 seconds and logged "could not write the index snapshot": listing
  the folders counted as a change. Only real changes (files added, removed, renamed or written) count now.

## [0.1.2] - 2026-10-22

### Added

- A **Show adult (18+) titles** switch (Settings, Sources and tracking), off by default. While it is off, sources that know
  a title's rating leave adult titles out of their lists (Kagane, Weeb Central, AnimeParadise; MangaDex already does), and
  extensions marked 18+ are hidden.
- The first-run welcome screen of an AppImage says that the app updates itself, and where to turn that off.

### Changed

- The AppImage no longer downloads an update at start on a metered connection; About's Check for updates still does.

### Fixed

- The AppImage did not start on a system without PipeWire or JACK ("error while loading shared libraries:
  libpipewire-0.3.so.0"). Their libraries now travel as fallbacks, used only when the system has none.

## [0.1.1] - 2026-10-19

### Added

- MyAnimeList tracking next to AniList: log in from Settings, then track any anime or manga from its Tracking page
  (status, progress, score, start and finish dates). Progress after reading or watching goes to every tracker an entry
  is bound to.

### Fixed

- The library update at launch, the refresh buttons and the automatic update waited for nothing, so an update that
  started before the sources had loaded failed every entry with "Source not installed". They now wait for the sources.
- Library updates gave sources only the entry's address, without its title, so AnimePahe entries whose link had
  expired failed with "AnimePahe no longer lists". The stored title is now passed along.

## [0.1.0] - 2026-10-18

### Added

- Manga and anime in one desktop app (Rust and GTK 4): one library with categories, filters and sorting, an updates feed, a
  history, and a download queue.
- Sources from repositories: recipe sources and WASM plug-ins; browsing, search, filters and a global search.
- A manga reader and an anime player (mpv) that keep your progress.
- Tracking with AniList, Kitsu, MangaUpdates and Simkl.
- Local folders as sources, and restoring Aniyomi and Tachiyomi backups (`.tachibk`).
- An AppImage for Linux that carries its own libraries; it needs glibc 2.39 or newer (Ubuntu 24.04, Fedora 40, Debian 13, Arch).
- The AppImage updates itself: at start it downloads a newer release in the background, checks it, and runs it after a
  restart.
