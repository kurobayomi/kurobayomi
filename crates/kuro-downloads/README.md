# kuro-downloads

Manga download queue, per-source scheduler, page download, integrity check and the shared download index
(Aniyomi areas 23 to 25). No GTK and no OS-specific code: free disk space, network state, the process hold and
notification delivery are injected by the GUI crate.

## Layout

| Module | Role |
|---|---|
| `manager` | `DownloadManager` facade and `ManagerParams` |
| `downloader` | actor: owns the queue, runs the scheduler, gates on connectivity |
| `queue` | pure state machine with the status transitions |
| `scheduler` | `pick_active`: at most 5 sources, one chapter each, ERROR skipped |
| `pipeline` | one chapter: precheck, tmp folder, page list, fetch, integrity, finalise |
| `images` | page file names, extension resolution, resume lookup, integrity count |
| `index` | source, manga and chapter tree: scan, hourly renew, debounced snapshot |
| `layout` | folder names and lookups (never creates folders, except `manga_dir`); the startup sweep of stray `_tmp` folders and `.cbz.part` files (T-23a-044) |
| `queue_model` | pure queue model of the Download page: sections, stable sort and reverse, moves, series moves, removals (T-25a-020, T-25a-031) |
| `sanitize` | `build_valid_filename` port and the Windows reserved-name rule |
| `store` | SQLite queue table, written whole in one transaction; `DownloadManager::restore` takes the rows read back at startup and `ManagerParams::restored` keeps the staging folders of restored chapters through the startup sweep (T-23a-044) |
| `notify` | pure event-to-notice mapping and the `NotificationSink` trait |
| `events` | `DownloadEvent` (broadcast, capacity 64) |
| `throttle` | `SpeedLimiter`: one shared byte budget for page downloads (T-23a-015) |
| `retry` | shared 4-attempt schedule, 2 s, 4 s, 8 s, with cancellable waits (T-24a-015) |
| `anime::naming` | `fat_safe`, episode, legacy and source folder names (T-24a-004, T-24a-005) |
| `anime::layout` | anime folder lookups, creation, episode and source rename, free-space precheck (T-24a-006 to T-24a-008, T-24a-017) |
| `anime::progress` | `ProgressCell` (0..=100, publishes on change) and the ffmpeg percent rule (T-24a-002) |
| `anime::store` | SQLite `download_queue` (anime rows): add, remove, clear, restore (T-24a-009) |
| `anime::enqueue` | enqueue filter and order, queue-size warning (T-24a-012, T-24a-013) |
| `anime::resolve` | `HosterResolver` seam and `resolve_video` (T-24a-014) |
| `anime::manager` | queue actor, command channel, per-source scheduler with 3 sources, delete cascade (T-24a-010, T-24a-011, T-24a-034) |
| `anime::ffmpeg` | remux argv builder, ffprobe duration, progress parser, child driver with cancel (T-24a-018 to T-24a-021) |
| `anime::local_server` | `LocalHttpServer` seam, `localhost:1` URL rules, session that stops the server on drop (T-24a-022) |
| `anime::notify` | progress (1 Hz), paused, warning and error notices with an error digest (T-24a-026 to T-24a-029) |
| `anime::lifecycle` | `RunnerState` (stopped, running, paused) and the window-close rule (T-24a-030) |
| `anime::network` | Wi-Fi-only verdict, 2 s debounce, auto-resume gate, sysfs fallback probe (T-24a-031) |
| `anime::pending` | remove-after-watched SQLite store, deduplicated, installed-source flush (T-24a-032) |
| `anime::delete_rules` | bookmark and excluded-category filter for deletes (T-24a-033) |
| `anime::cache` | downloaded-episode index: scan, hourly renewal, debounced atomic snapshot, sizes (T-24a-035 to T-24a-037) |
| `anime::offline` | offline playback file choice and `file://` video (T-24a-039) |
| `anime::torrent` | torrent links: `is_torrent`, the `link=`/`index=` to magnet conversion, `check_torrent` (`TorrentUnavailable` without the `torrent-server` feature) (T-24a-023) |
| `anime::handoff` | external downloader: location copy with its label (T-24a-025), aria2 `addUri` request and a plain HTTP JSON-RPC post (T-24a-024); routed from `EpisodeEngine::run` through `ExternalHandoff` |
| `anime::downloader` | `EpisodeEngine`, the `EpisodeRunner` for one episode: staging, resume, retry, success check, finalise with collision handling; startup `sweep_staging` (T-24a-016, T-24a-049, T-24a-062, T-24a-066) |
| `engine` | `QueueEngine` adapters for the two kinds (start, stop, pause_and_stop, clear, reorder, remove) (T-25a-017) |
| `download_prefs` | typed download preferences: 23 accessors returning `Pref<T>` on the area 02 store (T-25a-001) |
| `pending` | manga remove-after-read list: `download_pending_delete` store (manga rows), take-all transaction, flush plan (T-23a-028) |
| `watch` | `notify` watcher on the downloads root and a 2 s debouncer that invalidates the index (T-23a-045) |
| `local` | local manga source counts (directories and archives) and recursive size (T-23a-042) |
| `redact` | log redaction: URL query, fragment and userinfo removed, header values replaced by `<redacted>` (T-24a-060) |
| `comicinfo` | `ComicInfo.xml` writer: element order, number and status text, `Web` value (T-23a-021) |
| `settings` | download preference keys and defaults (`PREFERENCE_KEYS`, data section 3), speed limit KiB/s to bytes/s (T-24a-054, T-25a-002, T-25a-003, T-25a-005) |

## Decisions

- **Android machinery and state (T-23a-038, T-23a-039, T-24a-003).** Dropped; the reasons are in the `lib.rs` doc comment.
  `AnimeDownloadPart` had no references, so no range-part type exists.
- **Android-only drops (T-24a-047, T-24a-048, T-24a-063, T-25a-034).** No `ensureSuccessfulAnimeDownload` port: it was never
  called in Kotlin, and the success check is `is_download_successful` (T-24a-049). No WorkManager, foreground info or
  notification channel: the runner (`anime::lifecycle`) and the notifier (`anime::notify`) replace them. No MediaScanner
  call: the per-episode `.nomedia` replaces it. The downloads tab's animated icon, Compose dark-theme alpha and
  `HorizontalPager` are GUI items, not in this crate: the static icon is dropped, the alpha comes from GTK theme CSS, and the
  pager is meant to become `gtk::Notebook`. This crate keeps the pager order (Anime, then Manga) and the scroll collapse rule
  in `page.rs` (`FabState::on_scroll`). **Still open (T-25a-034 bullets 3 and 4):** no GTK code calls `on_scroll` from a
  `gtk::EventControllerScroll`, and the downloads page in the root `src/screens.rs` still uses `gtk::Stack` plus
  `gtk::StackSwitcher`, not `gtk::Notebook`. This crate has no GTK dependency, so the wiring belongs to the app page (T-25a-023).
- **Per-episode `.nomedia` (T-24a-056).** Written into each finished episode folder by `finalise`. The root-level
  `.nomedia` stays dropped (DM-29, D-DL-10).
- **Anime notifications (T-24a-028, T-24a-029, T-24a-064, T-24a-065).** The progress pause button is `Action::Stop`
  (label "Stop"); manga keeps `Action::Pause` ("Pause"). `Action::OpenHelp(url)` is offered only for a known help link
  (`known_help_url`). The help button label "Help" is a decision: the Kotlin text was not checked. The queue-size warning
  is also sent as a `DownloadEvent::Warning` with its help link, so the notifier can show it.
- **Stopped queue toast (T-24a-069).** `EnqueueOutcome::continue_toast` is set when items join a non-empty queue while the
  runner is stopped. The GUI shows `download_queued_press_continue`.
- **Reorder no-op (T-24a-052, T-25a-028).** `normalize_order` returns `None` when the result equals the current order, and
  the engine returns before any pause or persist. A drop at the same position in `drag_within_group` is `None` too.
- **Rename failures (T-24a-071).** A cross-device rename is `DownloadError::CrossDevice` (no copy). Any failed folder rename
  also emits a `DownloadEvent::Warning`, not only the `Err` reply. A missing episode folder stays a logged no-op (Kotlin).
- **Torrent path (T-24a-023).** `TORRENT_SERVER_HOST` defaults to `http://127.0.0.1:8090`. With the feature, a torrent link
  fails with an internal error because the TorrServer stream client is not in this crate yet. The engine calls `check_torrent`
  after the hoster resolve.
- **Local anime count (T-24a-038).** `local::anime_episode_count` counts the supported files directly in the anime folder,
  matched case-sensitively. `AnimeIndex::episode_count_anime` routes `LOCAL_SOURCE_ID` to it, reading the folder set by
  `AnimeIndex::with_local_anime_root` (the argument is the anime url, as in Kotlin). Without a local root the count is 0.
- **External downloader (T-24a-024, T-24a-025).** `EpisodeEngine::with_external(ExternalHandoff)` enables it. Per episode,
  after the hoster resolve and the torrent check, `use_external_downloader` on and a non-empty
  `external_downloader_selection` (the aria2 endpoint, `http://host:port/jsonrpc`) send `aria2.addUri` with the video URL,
  its headers and the final episode folder as `dir`; the folder path is copied through the injected `Clipboard`
  (a failed copy is only logged), no staging folder is made and nothing is finalised. aria2 writes into the final
  folder, not `<name>_tmp`, so the startup sweep of `_tmp` folders cannot remove its files. A refused or failed post is
  an error and the item goes to ERROR. The episode then counts as downloaded in the queue (it leaves the queue), as
  the engine returned `Ok`.
- **Drag across sections (T-25a-028, E-34).** Decision: refused, as in Kotlin. `AnimeDownloadAdapter.shouldMove`
  returns true only when both rows have the same header, so a cross-section drop never reaches `onItemReleased`.
  E-34 assumed the opposite ("cross-section drops are allowed"); the Kotlin behaviour wins by the
  port rule.
- **Hidden notification content (T-23a-043, T-24a-028).** `NotifierSink` passes each notice through as built. The text
  rules (`effect_for`, `progress_notice`) already hide names under `hide_notification_content` ("Downloading (x/y)" or
  "Downloading…", empty body), so the platform's blanket replacement is not used on download notices.
- **Logs (T-24a-060).** ffmpeg stderr is read; a line that says error, warning or fatal is logged at error level with its
  URLs redacted. Headers are logged only by name.

- **New-item filter (T-25a-004) has one owner: `kuro-domain::download::filter`** (`should_download_new`, `filter_new`),
  which is also where `remove_after_read_index` lives. The copy that was here had no caller and is gone. Nothing
  enqueues new items automatically yet: the library-update flow (area 26) must call `filter_new` with the stored
  preferences and then `AnimeQueue::enqueue` or `DownloadManager::enqueue`. Until that area lands the flags have no
  effect on its own, which is the deferral.

- **Anime episode engine (T-24a-016) lives here.** Tasks T-24a-049, T-24a-062 and T-24a-066 depend on it, so
  `anime::downloader` implements the episode steps of behaviour 5.2 to 5.9 with the existing ffmpeg driver. Source
  headers are not passed to it: a resolved `Video` carries its own headers (`Video.headers`).
- **Finished folder name collision (E-F02, E-F03).** A new download whose plain folder name is held by another non-empty
  folder gets `<name> (<episode id>)`. Every lookup tries the plain, legacy and suffixed names. Finalise onto a
  non-empty folder is `EpisodeExists` and the staging folder is kept.
- **Network resume (E-N01).** `Command::Network` feeds `NetworkGate`. A run stopped by the network starts again when
  the network returns, unless the user paused or cleared the queue in between.
- **Pending deletes (E-P04).** A row that cannot be decoded moves to `anime_download_pending_corrupt`. A group whose
  delete fails goes back into the pending table.
- **`download_slots` (E-12, E-PR01, T-25a-003).** Decision: hidden. The key stays in `PREFERENCE_KEYS` (default 1) so a
  backup restores, but the engine ignores it, as Kotlin does, and `settings::DOWNLOAD_SLOTS_HIDDEN` is asserted by a
  test. The manga queue runs `scheduler::MAX_ACTIVE_SOURCES` sources at once. There is no replacement setting.
- **`download_speed_limit` (E-PR02).** Only the conversion is here (`speed_limit_bytes_per_sec`, KiB/s times 1024,
  0 is unlimited). No limiter is applied to anime downloads: ffmpeg copies streams and has no limiter here.
- **Preference keys (T-24a-054).** The Kotlin key names are used unchanged, so backups restore. The `downloads.*`
  registry paths in `downloader::SETTING_KEYS` are a separate list for the registry.
- **Test fixtures (T-24a-061).** The edge-cases section 5 trees are checked in under `tests/fixtures/dl/` (`tree_new`,
  `tree_legacy`, `tree_tmp`, `tree_case`). `testing::dl_fixture` copies one into a temp folder, so tests never change
  the checked-in files. Trees that a test shapes itself are still built in temp directories.
- **Startup ffmpeg check (T-24a-067).** `anime::manager::spawn` runs the check once (`ffmpeg::startup_notice`) and the
  handle keeps the result: `AnimeQueue::ffmpeg_notice()` is `Some` with the text "ffmpeg is not installed" and
  `FFMPEG_HELP_URL` when `ffmpeg` or `ffprobe` is missing. Showing it as a Settings row is the GUI crate's part. A
  missing binary during a run is `FfmpegMissing` and is not retried.
- **Cancel during the probe (T-24a-058).** `remux_once` races ffprobe against the cancel token (the dropped child is
  killed), and `run_remux` returns `Cancelled` before the spawn when the token is already cancelled, without touching
  the partial file: a newer run of the same episode may own it.
- **Filename limit (T-24a-072).** Only the file component (`<name>.mkv`, `<name>.tmp`) is cut to 200 UTF-8 bytes. Folder
  names keep the 240-unit rule of `fat_safe`.

- **`ntfs-names` (default on), D-DL-01.** A reserved device name gets `_` appended before its extension
  (`CON` becomes `CON_`, `LPT1.txt` becomes `LPT1_.txt`), so folders copied to a Windows disk stay valid. Titles
  without a reserved name produce the Kotlin output byte for byte.
- **Queue table DDL** lives in the unified database (`kuro-db/sql/unified/downloads.sql`, exported as
  `schema_unified::DOWNLOAD_DDL`). The stores only read and write it; `open_in_memory` applies it for tests.
- **Index snapshot** is `KIDX\x01` followed by JSON, in `download-index-v1.bin`. bincode is not in the local
  registry cache, and JSON keeps the format inspectable. A file that fails to parse, or was written for another
  root, is deleted.
- **Settings.** `downloader::SETTING_KEYS` lists the keys this crate reads with their defaults. The Rust settings
  schema (area 02) registers them. `download_slots` is not among them: it is stored under its Kotlin key and hidden.
- **Tall split.** The optimal part height is injected (`DownloadSettings::optimal_image_height`, default 4096). The GUI
  sets it from GDK.
- **Anime queue.** Shares `DownloadState`, `scheduler::pick_active` and the retry schedule with the manga side. The
  runner and the hoster resolver are traits, implemented by the engine and the player areas.
- **Free space** is injected as a `fn(&Path) -> Option<u64>`. The GUI passes
  `kuro_platform::storage::free_bytes`.
