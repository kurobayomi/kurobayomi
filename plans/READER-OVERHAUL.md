# Manga reader overhaul

Audit of `src/reader` (about 24,000 lines) on 2026-11-02, and the plan that follows from it. Phases 1 to 3 are built; phase 4 is left.

## Why

The reader works, but it was built as its own island. It re-implements pieces the shared image crate (`kuro-images`)
already has, and one of those copies cannot read a format a source sends. On the desktop it also misses the reading
features people expect from a manga reader: the right reading mode for the content, two pages side by side on a wide
window, and dragging a webtoon with the mouse.

## What the audit found

### Duplicated instead of shared

`kuro-images` says in its own notes that one image service serves the covers, the full-cover viewer and the reader,
so all three share the fetch and decode slots and one decoded-image cache. The reader never joined it.

| Reader copy | Shared version | Problem |
|---|---|---|
| `page_decode.rs` (`image::load_from_memory`) | `kuro_images::decode` (raster, JPEG XL, HEIF, AVIF, animation) | The reader cannot decode JPEG XL. Kagane asks for JPEG XL first, so its pages can fail with "The image couldn't be loaded". |
| `crop_borders.rs` | `kuro_images::decode::borders` | Two border crops to keep in step. |
| `cache.rs` (chapter disk cache, 100 MiB) | `kuro_images::chapter_cache` (same folder, same rules) | Two implementations of one cache; the shared one has no user at all. |
| `decoded_lru.rs` (a fixed 512 MiB in the reader) | `kuro_images::memory_cache` (64 MiB) | Two memory budgets that do not know about each other; 512 MiB is heavy on an 8 GB machine. |
| `loader/http.rs` (one worker thread, one page at a time) | the image service's fetch (8 slots, priority lane) | Pages download one after another, and the reader's queue is separate from the rest of the app's. |
| `strings.rs` (642 lines of one-line wrappers around `tr`) | `tr(MsgKey::...)` as everywhere else | A second way to get a text; 79 of the wrappers are unused. |

### Not best practice in the code

- `src/reader/mod.rs` turns dead-code warnings off for the whole module (`#![allow(dead_code, unused_imports)]`) with a
  note that parts "attach later". They are attached; the allow hides about 100 unused items.
- The module overview says trackers, downloads and the source registry are not connected. They are; the notes mislead.
- Display colour management (`page_profile`, ICC) exists but the build never enables it and its setting was cut.
- Three files are far past the "no god files" rule: `screen.rs` (2,318 lines), `viewers/engine.rs` (2,239),
  `page_view.rs` (1,519).

### Missing or odd for readers

1. **Every title opens left to right in single pages.** Japanese manga reads right to left; webtoons and manhwa read as
   a vertical strip. Nothing picks a mode from the content; only a per-title choice the reader remembers.
2. **No two pages side by side.** On a wide desktop window "fit screen" shows one narrow page between black bars. Only
   the opposite exists (splitting one wide spread into two pages).
3. **Mouse drag does not scroll a webtoon at normal size.** The drag code only pans when zoomed in, and GTK's own drag
   scrolling reacts to touch only. The page-by-page modes do take mouse drags.
4. **Page errors never say why.** Offline, blocked by the site, or a damaged file all show "The image couldn't be loaded".

### Already right (keep)

Background decoding with a page pyramid for very tall images, the error card with Retry and Open in browser, position
saved on every page, the live Incognito check, the next chapter fetched ahead, F11 and Escape handling, the shared top
bar, and the native settings window.

## The plan

Each phase is one commit, built and tested at its end (touched modules only), in this order so the features in phase 3
are built once, on the shared pieces.

### Phase 1: clean up (no behaviour change)

- Remove the blanket allow in `src/reader/mod.rs`, then delete what it was hiding (the unused wrappers, functions and
  constants).
- Replace `strings.rs` with direct `tr(MsgKey::...)` calls, like the rest of the app, and delete the file.
- Rewrite the stale notes in the module overview to say what is connected now.
- Decide the ICC code: enable the `icc` feature with its setting, or delete it. Recommended: delete (the setting was cut).

### Phase 2: one image pipeline (done 2026-11-02)

- Pages decode with `kuro_images::decode` (gains JPEG XL, HEIF, AVIF and the EXIF orientation; animation stays) and
  borders are cropped by it; `page_decode`'s own decoder and `crop_borders.rs` are gone. The tiling stays the reader's.
- One chapter cache: the reader's `DiskCache` (synchronous, on the app's cache-size setting, also used by Updates)
  stays, and the unused async copy in `kuro-images` (`chapter_cache`, with the unused `loader::start`) is deleted.
- The decoded-page budget is a sixteenth of the machine's memory, between 128 and 512 MiB, instead of a fixed 512 MiB.
  It stays a cache of its own: it holds tile pyramids and frames, the image service holds covers.
- Page fetches stay one at a time in the reader's priority queue. Mihon's loader is sequential too, and sources are
  written for it (their own rate limits); parallel downloads would risk being throttled or banned for little gain.

### Phase 3: reading features

- **Reading mode from the content** (done): the default reading mode Adaptive (the default since 2026-11-02) opens a
  title whose tags say Webtoon, Manhwa, Manhua or Long Strip as the vertical strip, and everything else as pages in the
  "Adaptive page direction" (right to left by default, as in Mihon; left to right on request). A fixed default mode
  ignores the tags. A choice the user made for a title always wins.
- **Two pages side by side** (done) in the left-to-right and right-to-left modes when the window is wider than tall:
  pages pair in reading order from each chapter's start, a wide page (an existing spread) and the first page stand
  alone, and the pair follows the reading direction with both halves meeting in the middle. The setting "Page layout"
  (Two pages when wide, the default; Two pages; One page) is per title like the other Paged settings, and sits in the
  reader's menu under Reading mode (greyed out in the scrolling modes). While pairs
  show, wide pages are not split, and each half zooms on its own.
- **Drag to scroll a webtoon with the mouse** at any zoom, with the same momentum the page modes have.
- **Page errors say why**: no connection, the site refused the image (with the HTTP status), or the image could not be
  read; Retry and Open in browser stay.

### Phase 4: split the big files

Split by concern so each file is one part of the reader: `screen.rs` into loading, chapter navigation, chrome and
sheets; `viewers/engine.rs` into the pager and the webtoon engine with their shared drag and wheel input;
`page_view.rs` into the page widget and its gestures. Mechanical, after the behaviour changes, so the moves are pure
moves.

## Checks

- Each phase: the touched modules' tests, both CI scripts, and a manual read in the dev build with real libadwaita.
- Manual list before the nightly: a Kagane chapter (JPEG XL pages), a Japanese manga (right to left, two pages on a wide
  window), a webtoon (vertical, mouse drag), a local archive, a page that fails (offline), and the next-chapter keys.

## Decisions (2026-11-02)

- Two pages side by side: Automatic by window shape is the default, with Always and Never as the other choices.
- Reading mode from the content: the Adaptive default reading mode (2026-11-02), vertical strip only (tags), pages in
  the adaptive direction (right to left, the common manga default and Mihon's) otherwise; a mode the user picked for a
  title always wins, and a fixed default mode is used as it is.
