# Settings diet: keep or cut every preference

Written 2026-10-11 for the "as simple as possible" pass. Read-only analysis: no build, no code changed. Sources read: `crates/kuro-prefs/src/registry.rs` (317 rows), the section structs in `crates/kuro-prefs/src`, `src/settings_ui/*`, `src/pref_model/*`, `spec/settings.json`, and every reader of each key in `src/` and `crates/`.

**Status (user decision, 2026-10-11: "remove data saver and detach"):** Data Saver and Detach are CUT. Their code, settings rows, tests and strings are removed. Their stored keys are in `DROPPED_KEYS`, so an old `config.toml` or an Aniyomi backup still loads (see `docs/drop-decisions.md`). The counts and tables below are the analysis from before that cut and stay as the record.

Rule used for every row: a setting is KEEP (a visible control) or CUT (code, row, tests and stored key removed, a hard-coded default put in its place). No hidden settings. The one exception is the player's own mpv.conf / input.conf files, which need no UI. KEEP-LATER (torrent server, WebView and Cloudflare window, the other trackers) is left untouched and not counted.

## 1. Counts

| | Registry rows | Share |
|---|---|---|
| Total rows in `registry.rs` | **317** (315 distinct keys; 2 keys are registered twice) | |
| KEEP | **60** | of which 13 are state keys with no settings row |
| CUT | **248** | |
| KEEP-LATER | **9** | 6 torrent + 3 tracker-secret families |
| DEAD (no code reads the stored value at all) | **9** | 8 CUT, 1 KEEP-LATER (`pref_torrserver_shownotice`) |
| UNWIRED (a library crate has a reader, the live app never feeds it) | **100** | 9 are KEEP or KEEP-LATER and need wiring, the rest are CUT |
| Keys outside the registry that code reads or writes | **11** | listed in section 7; 3 KEEP, 7 CUT, 1 KEEP-LATER |

Read the DEAD and UNWIRED rows together: changing any of those 109 settings today changes nothing the user can see. Why: `src/player_view/mod.rs` starts the player from `PlayerPrefs::default()` and only copies the remembered decoder, the volume and the window choice from the store; `src/updates/download_host.rs` says the app has no live download manager; no source returns a `DataSaver`. The app has also been reading `pref_hardware_bitmap_threshold`, a key `DROPPED_KEYS` says is gone (section 7).

Flags used in the last column: DEAD, UNWIRED, READ-BY-APP (some app code really reads it), STATE (not a setting, app memory), FAMILY (one row per tracker id), SHARED-KEY (two registry rows, one key), OUTSIDE-REGISTRY. For every CUT row the import column says what the Aniyomi backup restore must do.

## 2. Proposed final pages (8)

Key-backed visible settings: **38**. Add the action rows that are not preferences (Edit categories x2, Create backup, Restore backup, Clear cache) and the About rows (5) and the total is about **48**, under the 60 target on purpose; section 5 lists what to bring back first if you want to get nearer 60.

| Page | Keys it keeps (visible) | State keys it needs | Rows the user sees |
|---|---|---|---|
| 1. Appearance | `pref_theme_mode_key`, `start_screen`, `app_language` | - | Theme (System, Light, Dark), Start screen, App language. |
| 2. Library | `default_anime_category`, `default_category`, `pref_library_update_interval_key`, `muted_notification_channels` | `__APP_STATE_library_update_last_timestamp`, `library_unread_updates_count`, `library_unseen_updates_count`, `__APP_STATE_last_used_anime_category`, `__APP_STATE_last_used_category` | Edit anime categories, Edit manga categories (actions), Default anime category, Default manga category, Automatic updates (interval), one switch Notify about new chapters and episodes. The library view menu (not a page) keeps: display mode, sort, filters Downloaded, Unread/Unseen, Completed, Tracked. |
| 2b. Library view menu (not a settings page) | `pref_display_mode_library`, `library_sorting_mode`, `animelib_sorting_mode`, `pref_filter_animelib_downloaded_v2`, `pref_filter_library_downloaded_v2`, `pref_filter_animelib_unread_v2`, `pref_filter_library_unread_v2`, `pref_filter_animelib_completed_v2`, `pref_filter_library_completed_v2`, `pref_filter_animelib_tracked_{id}_v2`, `pref_filter_library_tracked_{id}_v2` | - | Sort, Filter and Display tabs of the library; they stay where they are. |
| 3. Reader | `pref_default_reading_mode_key`, `pref_reader_theme_key`, `pref_image_scale_type_key`, `pref_zoom_start_key`, `crop_borders`, `pref_dual_page_split` | - | Default reading mode, Background color, Scale type, Zoom start position, Crop borders, Split wide pages. The reader window keeps the same six. |
| 4. Player | `pref_subtitle_lang`, `pref_audio_lang`, `player_volume_value`, `pref_auto_play_enabled` | - | Preferred subtitle languages, Preferred audio languages, Default volume, Autoplay next episode. The hoster and quality pickers stay in the player (no key). The mpv.conf and input.conf FILES in the mpv-config folder are the one advanced item, no UI. The Torrent sub-page stays as it is (KEEP-LATER keys, not counted). |
| 5. Downloads and storage | `__APP_STATE_storage_dir`, `download_slots`, `download_new`, `download_new_episode`, `remove_after_read_slots` | - | Download location, Maximum downloads, Download new chapters, Download new episodes, Delete after reading/watching, plus the Clear chapter and episode cache action. |
| 6. Sources and tracking | `source_languages`, `show_nsfw_source`, `anime_extension_repos`, `extension_repos`, `__PRIVATE_track_token_{id}`, `pref_auto_update_manga_sync_key` | `hidden_anime_catalogues`, `hidden_catalogues`, `pinned_anime_catalogues`, `pinned_catalogues`, `__APP_STATE_trusted_extensions`, `anime_ext_last_check`, `anilist_score_type` | Source languages, Show NSFW sources, Anime extension repos, Manga extension repos, AniList (login), Update progress after reading/watching. Revoke trusted extensions moves here as an action. |
| 7. Backup and restore | `backup_interval` | `__APP_STATE_last_auto_backup_timestamp` | Automatic backup frequency, Create backup, Restore backup (both actions, no include/exclude dialogs). |
| 8. About | - | - | Version and Check for updates, What is new, Open source licenses, Share crash logs, Debug info. No keys. |
| Outside the 8 pages (no settings row) | `__APP_STATE_pref_downloaded_only`, `__APP_STATE_incognito_mode`, `__APP_STATE_onboarding_complete` | - | Downloaded only is the Library filter and Incognito a check in the app menu (the More menu is gone); the onboarding flag is first-run state. |

Pages removed: Security and privacy, Advanced, Data Saver, Player > Internal player, Gestures, Decoder, Subtitles, Audio, Custom buttons, Script editor, Code editor, Advanced (mpv), and the separate Tracking, Browse and Data and storage pages (merged above). KEEP-LATER keys that stay as they are: `pref_torrserver_enable`, `pref_torrserver_shownotice`, `pref_torrserver_port`, `pref_torrserver_tackers`, `pref_torrserver_proxymode`, `pref_torrserver_proxyurl`, `__PRIVATE_pref_mangasync_username_{id}`, `__PRIVATE_pref_mangasync_password_{id}`, `__PRIVATE_pref_tracker_auth_expired_{id}`.

## 3. The 10 biggest wins

1. **Player: 92 rows, 5 kept.** 76 of the cuts are rows or keys the live player never reads (UNWIRED or DEAD). Eight of the nine player sub-pages disappear (Internal player, Gestures, Decoder, Subtitles, Audio, Custom buttons, Script editor, Advanced); Torrent stays for later.
2. **Reader: 53 rows, 6 kept.** E-ink flash (4 rows), colour filters (7), volume keys (2), tap-zone layouts and inversions (4), the four rotate-to-fit and the long-strip twins of every paged switch all go.
3. **Entry defaults: 29 `default_*` keys, 0 kept.** They only live behind the "Set as default" buttons of the entry sheets. The per-entry flags stay in the database; new entries get fixed defaults.
4. **Downloads: 23 rows, 4 kept (plus the storage location from Data and storage).** The app has no download manager yet, so most of the 19 cuts remove code nobody calls, including the Android external-downloader hand-off.
5. **Appearance: 22 palettes, pure black, navigation style, date format, relative time all gone.** Theme mode (System, Light, Dark) plus Start screen plus Language stay.
6. **Data Saver: 9 keys and 9 rows in Advanced.** Cut (2026-10-11). Nothing in the app built a `DataSaver` from them.
7. **Security page and Advanced page deleted (9 keys, about 30 rows).** App lock, notification hiding, DoH, user agent, verbose logging, display profile, bitmap threshold.
8. **Library page: 80 rows, 23 kept.** Smart update rules, update scopes (4 category sets), per-category sort, hidden-category and season switches, badges and tab micro toggles, Started/Bookmarked filters, Random sort, the portrait column keys.
9. **Tracking: one login (AniList) and one switch.** Ten other tracker rows leave the UI; their code and the token, username, password and expired families stay (KEEP-LATER).
10. **Desktop traps removed.** `library_update_restriction` defaults to wifi, and the runner test shows that blocks a wired machine; `download_only_over_wifi` defaults to true. Both go, with the other Android battery and data-plan rows.

## 4. How a cut is done (so "keep accepting on import" stays true)

- Delete the `Pref` field in its section struct (`crates/kuro-prefs/src/{base,ui,source,library,reader,player,player_advanced,tracking,network,security,backup}.rs`), the `registry.rs` row and the settings row (the typed row AND its twin in `spec/settings.json`, because `merge.rs` keeps spec rows that no typed row replaces), plus the tests that pin the key. Put a named const with the hard-coded default next to the code that read it.
- Add the key to `DROPPED_KEYS` in `registry.rs`. That list is the existing mechanism: restore skips those keys, so an Aniyomi backup or an old `config.toml` still loads. Every CUT row below that says "keep accepting on import" needs exactly that one line.
- Tracker secret families (`__PRIVATE_*_{id}`) are kept by KEEP-LATER; their keyring storage is separate from these registry names.
- `pref_mpv_conf` and `pref_mpv_input` are CUT as keys; on import, write their text to the real `mpv.conf` / `input.conf` in the mpv-config folder only if that file is missing.
- Two pinning tests will need a new count: `registry_has_the_309_plan_rows_in_order` in `registry.rs` (it reads `plans/02-preferences-system/p1-topic-key-registry.md`) and the per-section `FIELDS.len()` asserts such as the one in `source.rs`. The download keys exist twice (`crates/kuro-domain/src/download/prefs.rs` and `crates/kuro-downloads/src/download_prefs.rs`); cut both.

## 5. Decisions that need you (features, not micro-options) and what to restore first

Cut here, but they are whole features. Say no and I keep them:

- App lock (Security and privacy page, 4 keys). A working passphrase lock exists (`src/applock_host.rs`).
- External player (`pref_always_use_external_player`, read by 4 screens today). Only the always-on switch and the Android picker go; the one-off "play in external player" action stays.
- Mini player / PiP (4 keys): the feature stays on, the switches go.
- Data Saver (Bandwidth Hero proxy): feature and 9 keys. Cut by the user on 2026-10-11 (see the decisions table at the end).
- Random sort of the library and per-extension incognito.
- Player pages Custom buttons, Script editor, Code editor (no registry keys; stored in the database and the backup). Principle 4 says decide these one by one with you. The mpv.conf file covers the power-user need.
- Library list CSV export, Reindex downloads, Clear Manga/Anime database, Refresh library covers, Reset per-series reader settings (actions, section 8).

Cut by "when in doubt", first to restore if you want to approach 60: `pref_keep_screen_on_key`, `reader_navigation_mode_pager` (tap zones), `pref_enable_ani_skip` with `pref_enable_auto_skip_ani_skip` (AniSkip), `pref_player_speed`, `pref_skip_length_preference`, `library_update_manga_restriction` (Smart update), the Started and Bookmarked filters, `categorized_display` (per-category sort), `auto_download_while_reading` and `auto_download_while_watching`, `save_chapter_as_cbz`, `pref_always_use_external_player`.

Not a registry key but you asked for it: **player quality and server choice**. Aniyomi has no stored preference for it; the hoster and quality pickers live in the player. Nothing to cut. A "preferred quality" setting would be new work.

## 6. Tables by settings page

### Appearance (settings_appearance, App language)

8 keys: 5 CUT, 3 KEEP.

| Key | Where the user sees it | What it does | Verdict | If CUT: hard-code | Reason | Flags and import |
|---|---|---|---|---|---|---|
| `pref_theme_mode_key` | Appearance > Theme > Theme mode | Chooses System, Light or Dark colours. | **KEEP** |  | The one look setting a new user expects; System follows the desktop. | - |
| `pref_app_theme` | Appearance > Theme > App theme (strip of palette cards) | Picks one of the 22 colour palettes. | **CUT** | DEFAULT | The GTK theme and the system accent already own colours; Light, Dark and System are enough. | import: keep accepting on import |
| `pref_theme_dark_amoled_key` | Appearance > Theme > Pure black dark mode | Black instead of dark grey in dark mode. | **CUT** | false | A palette variant; goes away with the palettes. | import: keep accepting on import |
| `relative_time_v2` | Appearance > Display > Relative timestamps | Shows Today instead of a date in lists. | **CUT** | true | Micro option; the default is the right one. | import: keep accepting on import |
| `app_date_format` | Appearance > Display > Date format | Picks a date pattern for lists. | **CUT** | "" (locale short date) | The system locale already formats dates. | import: keep accepting on import |
| `start_screen` | Appearance > Display > Start screen | Tab that opens at launch (Anime, Manga, Updates, History, Browse). | **KEEP** |  | A manga-only or anime-only user wants it in the first 10 minutes. | - |
| `bottom_rail_nav_style` | Appearance > Display > Navigation style | Chooses which tab moves into the More tab. | **CUT** | MOVE_HISTORY_TO_MORE | Phone bottom-bar idea; a desktop sidebar has room for every tab. | import: keep accepting on import |
| `app_language` | Appearance > Display > App language (page settings_app_language) | Language of the app, empty means follow the system. NOT in the registry (stored by src/pref_model/language.rs). | **KEEP** |  | Always keep; add it to the registry so import and tests see it. | OUTSIDE-REGISTRY |

### Library page (settings_library) and the library window sheets

81 keys: 57 CUT, 24 KEEP.

| Key | Where the user sees it | What it does | Verdict | If CUT: hard-code | Reason | Flags and import |
|---|---|---|---|---|---|---|
| `pref_display_mode_library` | Library > Display sheet > mode chips (compact grid, comfortable grid, cover only, list) | How library cards are drawn. | **KEEP** |  | Library display basic. | - |
| `library_cover_size` | Library > view options menu > Cover size (a slider) | Column width of the Library and Browse cover grids, 100 to 320 px (160 by default); the covers resize while the slider moves and the column count follows the window. Kurobayomi key, added 2026-10-24. | **KEEP** |  | Users asked to control cover size; the cut column keys only left gaps because covers have a fixed width. | - |
| `library_sorting_mode` | Library > Sort sheet (manga library) | Sort field and direction of the manga library. | **KEEP** |  | Library display basic. | - |
| `animelib_sorting_mode` | Library > Sort sheet (anime library) | Sort field and direction of the anime library. | **KEEP** |  | Library display basic (same control as the manga one). | - |
| `__APP_STATE_library_update_last_timestamp` | Nowhere (state) | When the last library update ran; drives the schedule and the Updated-ago text. | **KEEP** |  | State the update schedule needs. | STATE |
| `pref_library_update_interval_key` | Library > Global update > Automatic updates | How often the library refreshes (off, 6, 12, 24, 48 hours, weekly). | **KEEP** |  | Always keep: update interval. | - |
| `library_update_restriction` | Library > Global update > Automatic updates device restrictions | Update only on Wi-Fi, while charging, or on an unmetered network. | **CUT** | {} (no restriction) | Phone battery and data idea; today's default wifi blocks a wired desktop (see src/misc/runner.rs wifi_only test). | import: keep accepting on import |
| `library_update_manga_restriction` | Library > Global update > Smart update | Skips entries that are fully read, not started, completed or outside their predicted release window. | **CUT** | the four default skip rules | Micro option; the default behaviour is what almost everyone wants. | import: keep accepting on import |
| `auto_update_metadata` | Library > Global update > Automatically refresh metadata | Also refreshes covers and details during a library update. | **CUT** | false | Micro option; a manual refresh on the entry page covers it. | import: keep accepting on import |
| `display_continue_reading_button` | Library > Display sheet > Overlay > Continue button | Shows a continue button on each library card. | **CUT** | false | Micro per-card toggle. | import: keep accepting on import |
| `display_category_tabs` | Library > Display sheet > Tabs > Show tabs | Shows category tabs above the grid. | **CUT** | true | With one category there are no tabs anyway; hiding them is a micro option. | import: keep accepting on import |
| `display_number_of_items` | Library > Display sheet > Tabs > Show number of items | Shows a count next to each category tab. | **CUT** | false | Micro option. | import: keep accepting on import |
| `categorized_display` | Library > Categories > Per-category settings for sort (key categorized_display) | Lets each category keep its own sort and display. | **CUT** | false | Power-user option; one library-wide sort is simpler. | import: keep accepting on import |
| `hidden_categories` | Library > Categories > Hide hidden categories from categories and storage screen (key hidden_categories) | Hides hidden categories from two screens. | **CUT** | false | Micro option. | import: keep accepting on import |
| `pref_filter_library_interval_custom` | Library > Filter sheet (interval chip, if shown) | Filters by update interval (custom). | **CUT** | DISABLED | Micro filter nobody asks for. | import: keep accepting on import |
| `display_download_badge` | Library > Display sheet > Overlay > Downloaded | Badge with the number of downloaded items. | **CUT** (round 5) | false | Its control went with the Display sheet; a setting is visible or cut. | import: keep accepting on import |
| `display_unread_badge` | Library > Display sheet > Overlay > Unread / Unseen | Badge with the number of unread chapters or unseen episodes. | **CUT** (round 5) | true | Its control went with the Display sheet; the badge always shows. | import: keep accepting on import |
| `display_local_badge` | Library > Display sheet > Overlay > Local | Badge on local-source entries. | **CUT** | true | Micro option; the default is fine. | import: keep accepting on import |
| `display_language_badge` | Library > Display sheet > Overlay > Language | Badge with the source language. | **CUT** | false | Micro option. | import: keep accepting on import |
| `library_show_updates_count` | Library > Global update > Show unseen/unread count on Updates icon | Shows a count on the Updates tab. | **CUT** | true | Micro option; the count is cheap and useful. | import: keep accepting on import |
| `library_random_anime_sort_seed` | Nowhere (state of the Random sort) | Seed of the Random sort of the anime library. | **CUT** | 0 | Drop the Random sort entry (micro); then no seed is needed. | import: keep accepting on import |
| `library_random_manga_sort_seed` | Nowhere (state of the Random sort) | Seed of the Random sort of the manga library. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `pref_animelib_columns_portrait_key` | Library > Display sheet > Items per row (narrow window, anime) | Grid columns when the window is under 900 px wide. | **CUT** | 0 (auto) | One slider that silently switches key with window width is a trap; keep the wide-window key only. | import: keep accepting on import |
| `pref_library_columns_portrait_key` | Library > Display sheet > Items per row (narrow window, manga) | Grid columns when the window is under 900 px wide. | **CUT** | 0 (auto) | Same as above. | import: keep accepting on import |
| `pref_animelib_columns_landscape_key` | Library > Display sheet > Items per row (anime) | Grid columns of the anime library (window 900 px or wider). | **CUT** (round 5) | 0 (auto) | Its control went with the Display sheet; the grid fits the window. | import: keep accepting on import |
| `pref_library_columns_landscape_key` | Library > Display sheet > Items per row (manga) | Grid columns of the manga library. | **CUT** (round 5) | 0 (auto) | Same for manga. | import: keep accepting on import |
| `pref_filter_animelib_downloaded_v2` | Library > Filter sheet > Downloaded (anime) | Tri-state filter: only downloaded entries. | **KEEP** |  | Library filter basic. | - |
| `pref_filter_library_downloaded_v2` | Library > Filter sheet > Downloaded (manga) | Same filter for manga. | **KEEP** |  | Same control for manga. | - |
| `pref_filter_animelib_unread_v2` | Library > Filter sheet > Unseen (anime) | Tri-state filter: entries with unseen episodes. | **KEEP** |  | Library filter basic. | - |
| `pref_filter_library_unread_v2` | Library > Filter sheet > Unread (manga) | Same filter for manga. | **KEEP** |  | Same control for manga. | - |
| `pref_filter_animelib_started_v2` | Library > Filter sheet > Started (anime) | Tri-state filter: entries already started. | **CUT** | DISABLED | Micro filter; Unread covers the same need. | import: keep accepting on import |
| `pref_filter_library_started_v2` | Library > Filter sheet > Started (manga) | Same filter for manga. | **CUT** | DISABLED | Same as above. | import: keep accepting on import |
| `pref_filter_animelib_bookmarked_v2` | Library > Filter sheet > Bookmarked (anime) | Tri-state filter: entries with a bookmarked item. | **CUT** | DISABLED | Micro filter. | import: keep accepting on import |
| `pref_filter_library_bookmarked_v2` | Library > Filter sheet > Bookmarked (manga) | Same filter for manga. | **CUT** | DISABLED | Same as above. | import: keep accepting on import |
| `pref_filter_animelib_completed_v2` | Library > Filter sheet > Completed (anime) | Tri-state filter: completed entries. | **KEEP** |  | Library filter basic; finished shows are the common filter. | - |
| `pref_filter_library_completed_v2` | Library > Filter sheet > Completed (manga) | Same filter for manga. | **KEEP** |  | Same control for manga. | - |
| `pref_filter_animelib_tracked_{id}_v2` | Library > Filter sheet > Tracked (anime, one row per logged-in tracker) | Tri-state filter: entries tracked in that tracker. | **KEEP** |  | Stays while AniList can log in; the sheet shows one Tracked chip. Id family kept for the other trackers (KEEP-LATER). | FAMILY |
| `pref_filter_library_tracked_{id}_v2` | Library > Filter sheet > Tracked (manga) | Same filter for manga. | **KEEP** |  | Same as above. | FAMILY |
| `library_unread_updates_count` | Nowhere (state) | Count of new manga updates since the Updates tab was opened. | **KEEP** |  | State behind the Updates badge. | STATE |
| `library_unseen_updates_count` | Nowhere (state) | Count of new anime updates since the Updates tab was opened. | **KEEP** |  | State behind the Updates badge. | STATE |
| `default_anime_category` | Library > Categories > Default anime category | Category a new anime lands in (or Always ask). | **KEEP** |  | Always keep: categories. | - |
| `default_category` | Library > Categories > Default manga category | Category a new manga lands in (or Always ask). | **KEEP** |  | Always keep: categories. | - |
| `__APP_STATE_last_used_anime_category` | Nowhere (state) | Category tab last open in the anime library. | **KEEP** |  | State that makes the library reopen where you left. | STATE |
| `__APP_STATE_last_used_category` | Nowhere (state) | Category tab last open in the manga library. | **KEEP** |  | Same as above. | STATE |
| `animelib_update_categories` | Library > Global update > Anime Categories | Anime categories included in the library update. | **CUT** | {} (all categories) | Update scope micro option; all categories is the simple rule. | import: keep accepting on import |
| `library_update_categories` | Library > Global update > Manga Categories | Manga categories included in the library update. | **CUT** | {} (all categories) | Same as above. | import: keep accepting on import |
| `animelib_update_categories_exclude` | Library > Global update > Anime Categories (excluded) | Anime categories left out of the update. | **CUT** | {} | Same as above. | import: keep accepting on import |
| `library_update_categories_exclude` | Library > Global update > Manga Categories (excluded) | Manga categories left out of the update. | **CUT** | {} | Same as above. | import: keep accepting on import |
| `pref_update_season_on_refresh` | Library > Season > Update seasons with episodes on refresh | Also refreshes seasons when one entry is refreshed. | **CUT** | false | Micro option of one source type. | import: keep accepting on import |
| `pref_update_season_on_library_update` | Library > Season > Update seasons with episodes on library update | Also refreshes seasons during a library update. | **CUT** | false | Micro option of one source type. | import: keep accepting on import |
| `mark_duplicate_read_chapter_read` | Library > Chapter behavior > Mark duplicate read chapter as read | When a chapter is read, marks same-numbered chapters read. | **CUT** | {} (off) | Micro option; off by default. | import: keep accepting on import |
| `mark_duplicate_seen_episode_seen` | Library > Episode behavior > Mark duplicate seen episode as seen | When an episode is seen, marks same-numbered episodes seen. | **CUT** | {} (off) | Micro option; off by default. | import: keep accepting on import |
| `default_episode_filter_by_seen` | Entry page > Filter sheet > Set as default (episodes, unseen) | Default per-entry filter for new anime entries. | **CUT** | 0 | Hard-code the default; the per-entry sheet and its stored flags stay. | import: keep accepting on import |
| `default_chapter_filter_by_read` | Entry page > Filter sheet > Set as default (chapters, unread) | Default per-entry filter for new manga entries. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `default_episode_filter_by_downloaded` | Entry page > Filter sheet > Set as default | Default downloaded filter, episodes. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `default_chapter_filter_by_downloaded` | Entry page > Filter sheet > Set as default | Default downloaded filter, chapters. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `default_episode_filter_by_bookmarked` | Entry page > Filter sheet > Set as default | Default bookmarked filter, episodes. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `default_chapter_filter_by_bookmarked` | Entry page > Filter sheet > Set as default | Default bookmarked filter, chapters. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `default_episode_filter_by_fillermarked` | Entry page > Filter sheet > Set as default | Default fillermarked filter, episodes. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `default_episode_sort_by_source_or_number` | Entry page > Sort sheet > Set as default | Default sort field, episodes. | **CUT** | 0 (by source) | Same as above. | import: keep accepting on import |
| `default_chapter_sort_by_source_or_number` | Entry page > Sort sheet > Set as default | Default sort field, chapters. | **CUT** | 0 (by source) | Same as above. | import: keep accepting on import |
| `default_chapter_display_by_name_or_number` | Entry page > Display sheet > Set as default (episode AND chapter field, registered twice) | Default title style: name or number. | **CUT** | 0 | Same as above. | SHARED-KEY; import: keep accepting on import |
| `default_chapter_sort_by_ascending_or_descending` | Entry page > Sort sheet > Set as default (episode AND chapter field, registered twice) | Default direction of the list. | **CUT** | 0 (descending as the app draws it today) | Same as above. | SHARED-KEY; import: keep accepting on import |
| `default_episode_show_thumbnail_previews` | Entry page > Display sheet > Set as default | Default for thumbnails in the episode list. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `default_episode_show_summaries` | Entry page > Display sheet > Set as default | Default for episode summaries. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `default_season_filter_by_downloaded` | Anime entry page (seasons) > Filter sheet > Set as default | Default downloaded filter for season lists. | **CUT** | 0 | Season list tuning; hard-code. | import: keep accepting on import |
| `default_season_filter_by_unseen` | Anime entry page (seasons) > Filter sheet | Default unseen filter for season lists. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `default_season_filter_by_started` | Anime entry page (seasons) > Filter sheet | Default started filter for season lists. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `default_season_filter_by_completed` | Anime entry page (seasons) > Filter sheet | Default completed filter for season lists. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `default_season_filter_by_bookmarked` | Anime entry page (seasons) > Filter sheet | Default bookmarked filter for season lists. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `default_season_filter_by_fillermarked` | Anime entry page (seasons) > Filter sheet | Default fillermarked filter for season lists. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `default_season_sort_by_source_or_number` | Anime entry page (seasons) > Sort sheet | Default sort field for season lists. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `default_season_sort_by_ascending_or_descending` | Anime entry page (seasons) > Sort sheet | Default direction of season lists. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `default_season_grid_display_mode` | Anime entry page (seasons) > Display sheet | Default grid style of seasons. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `default_season_grid_display_size` | Anime entry page (seasons) > Display sheet | Default grid size of seasons. | **CUT** | 0 | Same as above. | import: keep accepting on import |
| `default_season_download_overlay` | Anime entry page (seasons) > Display sheet > Overlay | Download badge on season tiles. | **CUT** | false | Same as above. | import: keep accepting on import |
| `default_season_unseen_overlay` | Anime entry page (seasons) > Display sheet > Overlay | Unseen badge on season tiles. | **CUT** | true | Same as above. | import: keep accepting on import |
| `default_season_local_overlay` | Anime entry page (seasons) > Display sheet > Overlay | Local badge on season tiles. | **CUT** | true | Same as above. | import: keep accepting on import |
| `default_season_lang_overlay` | Anime entry page (seasons) > Display sheet > Overlay | Language badge on season tiles. | **CUT** | false | Same as above. | import: keep accepting on import |
| `default_season_continue_overlay` | Anime entry page (seasons) > Display sheet > Overlay | Continue button on season tiles. | **CUT** | true | Same as above. | import: keep accepting on import |
| `default_season_display_mode` | Anime entry page (seasons) > Display sheet | Default season list style. | **CUT** | 0 | Same as above. | import: keep accepting on import |

### Reader (settings_reader; the reader window has its own Settings sheet over the same keys)

53 keys: 47 CUT, 6 KEEP.

| Key | Where the user sees it | What it does | Verdict | If CUT: hard-code | Reason | Flags and import |
|---|---|---|---|---|---|---|
| `pref_enable_transitions_key` | Reader > Animate page transitions | Slides pages instead of cutting. | **CUT** | true | Micro option; the default is right. | import: keep accepting on import |
| `pref_reader_flash` | Reader > E-Ink > Flash on page change | Flashes the screen on page change to reduce ghosting. | **CUT** | false | E-ink screens only; no desktop display needs it. | import: keep accepting on import |
| `pref_reader_flash_duration` | Reader > E-Ink > Flash duration | Length of that flash. | **CUT** | 100 | E-ink only. | import: keep accepting on import |
| `pref_reader_flash_interval` | Reader > E-Ink > Flash every | Pages between flashes. | **CUT** | 1 | E-ink only. | import: keep accepting on import |
| `pref_reader_flash_mode` | Reader > E-Ink > Flash with | Colour of the flash. | **CUT** | BLACK | E-ink only. | import: keep accepting on import |
| `pref_double_tap_anim_speed` | Reader > Double tap animation speed | Speed of the double-click zoom animation. | **CUT** | 500 | Micro option. | import: keep accepting on import |
| `pref_show_page_number_key` | Reader > Display > Show page number | Shows the page counter. | **CUT** | true | Micro option; the counter is always useful. | import: keep accepting on import |
| `pref_show_reading_mode` | Reader > Show reading mode | Briefly shows the reading mode when the reader opens. | **CUT** | true | Micro option. | import: keep accepting on import |
| `fullscreen` | Reader > Display > Fullscreen | Opens the reader fullscreen. | **CUT** | false | F11 does it in one key; the desktop default is already false. | import: keep accepting on import |
| `cutout_short` | Nowhere (no row; message says not used on Linux) | Draw into the phone display cut-out. | **CUT** | true | Phone notch, Android-only. | DEAD; import: keep accepting on import |
| `pref_keep_screen_on_key` | Reader > Display > Keep screen on | Stops the screen from sleeping while reading. | **CUT** | true | Hard-code on; desktop idle inhibit during reading is the right behaviour. | import: keep accepting on import |
| `pref_default_reading_mode_key` | Reader > Default reading mode | Reading direction and style: paged left to right, right to left, vertical, long strip, long strip with gaps. | **KEEP** |  | Always keep: reading direction. | - |
| `pref_enable_double_tap_zoom_webtoon` | Reader > Long strip > Double tap to zoom | Double click zooms in a long strip. | **CUT** | true | Micro per-mode option. | import: keep accepting on import |
| `pref_image_scale_type_key` | Reader > Paged > Scale type | Fit screen, stretch, fit width, fit height, original size, smart fit. | **KEEP** |  | Always keep: fit. | - |
| `pref_zoom_start_key` | Reader > Paged > Zoom start position | Where a zoomed page starts (automatic, left, right, center). | **KEEP** |  | Always keep: reader zoom. | - |
| `pref_reader_theme_key` | Reader > Display > Background color | Black, gray, white or automatic page background. | **KEEP** |  | Core reading comfort setting. | - |
| `always_show_chapter_transition` | Reader > Reading > Always show chapter transition | Shows the between-chapters card even when the next chapter is read. | **CUT** | true | Micro option. | import: keep accepting on import |
| `pref_preserve_reading_position` | Reader > Reading > Preserve reading position on read chapters | Reopens a read chapter where you stopped. | **CUT** | false | Micro option. | import: keep accepting on import |
| `crop_borders` | Reader > Paged > Crop borders | Trims white or black margins from pages. | **KEEP** |  | Frequently wanted for scans; one switch is enough (long strip follows it). | - |
| `navigate_pan` | Reader > Paged > Pan wide images | Tap zones pan a zoomed wide image before turning the page. | **CUT** | true | Micro option. | import: keep accepting on import |
| `landscape_zoom` | Reader > Paged > Automatically zoom into wide images | Zooms wide pages to fill the height. | **CUT** | true | Micro option; scale type covers it. | import: keep accepting on import |
| `crop_borders_webtoon` | Reader > Long strip > Crop borders | Crop borders for long strip. | **CUT** | false | Per-mode duplicate; one crop switch is enough. | import: keep accepting on import |
| `webtoon_side_padding` | Reader > Long strip > Side padding | Margins left and right in long strip, 0 to 25 percent. | **CUT** | 0 | Micro option; a wide window already centres pages. | import: keep accepting on import |
| `reader_hide_threshold` | Reader > Long strip > Sensitivity for hiding menu on scroll | How far you scroll before the menu hides. | **CUT** | LOW | Micro per-mode tweak. | import: keep accepting on import |
| `create_folder_per_manga` | Reader > Actions > Save pages into separate folders | Page saves go in a folder named after the entry. | **CUT** | false | Micro option. | import: keep accepting on import |
| `skip_read` | Reader > Reading > Skip chapters marked read | Next chapter jumps over read ones. | **CUT** | false | Micro option. | import: keep accepting on import |
| `skip_filtered` | Reader > Reading > Skip filtered chapters | Next chapter jumps over filtered-out ones. | **CUT** | true | Micro option. | import: keep accepting on import |
| `skip_dupe` | Reader > Reading > Skip duplicate chapters | Next chapter jumps over same-numbered duplicates. | **CUT** | false | Micro option. | import: keep accepting on import |
| `webtoon_disable_zoom_out` | Reader > Long strip > Disable zoom out | Stops zooming below fit width. | **CUT** | false | Micro per-mode option. | import: keep accepting on import |
| `pref_dual_page_split` | Reader > Paged > Split wide pages | Cuts a double-page spread into two pages. | **KEEP** |  | Real manga need; one switch, off by default. | - |
| `pref_dual_page_invert` | Reader > Paged > Invert split page placement | Swaps the order of the two halves. | **CUT** | false | Micro option; the reading direction already orders the halves. | import: keep accepting on import |
| `pref_dual_page_split_webtoon` | Reader > Long strip > Split wide pages | Same split for long strip. | **CUT** | false | Per-mode duplicate. | import: keep accepting on import |
| `pref_dual_page_invert_webtoon` | Reader > Long strip > Invert split page placement | Swaps halves in long strip. | **CUT** | false | Per-mode micro option. | import: keep accepting on import |
| `pref_dual_page_rotate` | Reader > Paged > Rotate wide pages to fit | Rotates a wide page to fill a portrait screen. | **CUT** | false | Phone orientation idea; split is the desktop answer. | import: keep accepting on import |
| `pref_dual_page_rotate_invert` | Reader > Paged > Flip orientation of rotated wide pages | Rotation direction. | **CUT** | false | Goes with rotate. | import: keep accepting on import |
| `pref_dual_page_rotate_webtoon` | Reader > Long strip > Rotate wide pages to fit | Rotation in long strip. | **CUT** | false | Phone orientation idea. | import: keep accepting on import |
| `pref_dual_page_rotate_invert_webtoon` | Reader > Long strip > Flip orientation of rotated wide pages | Rotation direction in long strip. | **CUT** | false | Goes with rotate. | import: keep accepting on import |
| `pref_custom_brightness_key` | Reader window > Settings > Custom filter > Custom brightness | Dims the page below the system brightness. | **CUT** | false | Colour filter; the monitor and the desktop night light handle it. | import: keep accepting on import |
| `custom_brightness_value` | Reader window > Settings > Custom filter | Brightness amount. | **CUT** | 0 | Colour filter. | import: keep accepting on import |
| `pref_color_filter_key` | Reader window > Settings > Custom filter > Custom color filter | Tints the page with a colour. | **CUT** | false | Colour filter. | import: keep accepting on import |
| `color_filter_value` | Reader window > Settings > Custom filter | Colour of that tint (ARGB). | **CUT** | 0 | Colour filter. | import: keep accepting on import |
| `color_filter_mode` | Reader window > Settings > Custom filter > blend mode | Blend mode of the tint. | **CUT** | 0 | Colour filter. | import: keep accepting on import |
| `pref_grayscale` | Reader window > Settings > Custom filter > Grayscale | Draws pages in grey. | **CUT** | false | Colour filter. | import: keep accepting on import |
| `pref_inverted_colors` | Reader window > Settings > Custom filter > Invert colors | Inverts page colours. | **CUT** | false | Colour filter. | import: keep accepting on import |
| `reader_long_tap` | Reader > Actions > Show actions on long tap | Long press opens the page actions menu. | **CUT** | true | Touch gesture; a right click does it on desktop. | import: keep accepting on import |
| `reader_volume_keys` | Reader > Navigation > Volume keys | Volume keys turn pages. | **CUT** | false | Android-only; the app note itself says the desktop takes these keys. | import: keep accepting on import |
| `reader_volume_keys_inverted` | Reader > Navigation > Invert volume keys | Swaps volume key direction. | **CUT** | false | Goes with volume keys. | import: keep accepting on import |
| `reader_navigation_mode_pager` | Reader > Paged > Tap zones | Layout of the click zones in paged mode. | **CUT** | 0 (default layout) | Per-mode micro tweak; the default layout follows the reading direction. | import: keep accepting on import |
| `reader_navigation_mode_webtoon` | Reader > Long strip > Tap zones | Layout of the click zones in long strip. | **CUT** | 0 (default layout) | Per-mode micro tweak. | import: keep accepting on import |
| `reader_tapping_inverted` | Reader > Paged > Invert tap zones | Mirrors the click zones in paged mode. | **CUT** | NONE | Micro option; right to left mode already mirrors them. | import: keep accepting on import |
| `reader_tapping_inverted_webtoon` | Reader > Long strip > Invert tap zones | Mirrors the click zones in long strip. | **CUT** | NONE | Micro option. | import: keep accepting on import |
| `reader_navigation_overlay_new_user` | Nowhere | First-run overlay of the tap zones. | **CUT** | true | Nothing reads it today. | DEAD; import: keep accepting on import |
| `reader_navigation_overlay_on_start` | Reader > Show tap zones overlay | Briefly draws the click zones when the reader opens. | **CUT** | false | Micro option; goes with the tap zone tweaks. | import: keep accepting on import |

### Player (settings_player and its nine rows). The live player reads almost none of these (see the top section)

92 keys: 81 CUT, 5 KEEP, 6 KEEP-LATER.

| Key | Where the user sees it | What it does | Verdict | If CUT: hard-code | Reason | Flags and import |
|---|---|---|---|---|---|---|
| `pref_preserve_watching_position` | Player > Internal player > Preserve watch position on seen episodes | Reopens a seen episode where you stopped instead of at the start. | **CUT** | false | Micro option; src/player_view/store.rs already hard-codes false. | UNWIRED; import: keep accepting on import |
| `pref_progress_preference` | Player > Internal player > At what point to mark the episode as seen | Fraction watched (70 to 100 percent) that counts as seen. | **CUT** | 0.85 | Micro option; src/player_view/store.rs already uses the 0.85 constant. | UNWIRED; import: keep accepting on import |
| `pref_allow_gestures_in_panels` | Player > Internal player > Controls > Allow gesture in panels | Gestures still work while a panel is open. | **CUT** | false | Touch gesture option. | UNWIRED; import: keep accepting on import |
| `pref_show_loading` | Player > Internal player > Controls > Show loading circle | Spinner while loading. | **CUT** | true | Micro option. | UNWIRED; import: keep accepting on import |
| `pref_show_current_chapter` | Player > Internal player > Controls > Show current chapter indicator | Shows the video chapter name. | **CUT** | true | Micro option. | UNWIRED; import: keep accepting on import |
| `pref_remember_volume` | Nowhere (no row; plan: Player > Default volume) | Remember the volume between episodes. | **CUT** | true | Always remember; one Default volume value is the setting. | READ-BY-APP (src/player_view/mod.rs); import: keep accepting on import |
| `player_volume_value` | Player > Default volume (new row; bound to the stored volume) | The volume the player starts at; updated when you change volume. | **KEEP** |  | Always keep: default volume. | READ-BY-APP (src/player_view/mod.rs) |
| `pref_show_failed_hosters` | Player > Internal player > Hosters > Display failed hosters | Lists servers that failed to load. | **CUT** | false | Micro option; the server picker stays. | UNWIRED; import: keep accepting on import |
| `pref_show_empty_hosters` | Player > Internal player > Hosters > Display empty hosters | Lists servers with no videos. | **CUT** | false | Micro option. | UNWIRED; import: keep accepting on import |
| `player_fullscreen` | Player > Internal player > Display > Fullscreen | Opens the player fullscreen. | **CUT** | false | F11 / double click; desktop default already false. | UNWIRED; import: keep accepting on import |
| `player_hide_controls` | Player > Internal player > Display > Hide player controls when opening | Starts with controls hidden. | **CUT** | false | Micro option. | UNWIRED; import: keep accepting on import |
| `pref_display_vol_as_per` | Player > Internal player > Display > Display volume as percentage | Shows volume as percent. | **CUT** | true | Micro option; nothing reads it. | DEAD; import: keep accepting on import |
| `pref_show_system_status_bar` | Nowhere (row was dropped) | Shows the phone status bar in the player. | **CUT** | false | Android-only. | DEAD; import: keep accepting on import |
| `pref_reduce_motion` | Player > Internal player > Display > Reduce player animation | Fewer animations. | **CUT** | false | GTK already has an animations switch (gtk-enable-animations). | UNWIRED; import: keep accepting on import |
| `pref_player_time_to_disappear` | Player > Internal player > Display > Hide player controls time | Milliseconds before controls hide. | **CUT** | 4000 | Micro option. | UNWIRED; import: keep accepting on import |
| `pref_panel_opacity` | Player > Internal player > Display > Panel opacity | Opacity of player panels. | **CUT** | 60 | Micro option. | UNWIRED; import: keep accepting on import |
| `pref_enable_skip_intro` | Player > Internal player > Intro skip > Enable intro skip | Shows the skip intro button. | **CUT** | true | Always on is the right default. | UNWIRED; import: keep accepting on import |
| `pref_enable_auto_skip_ani_skip` | Player > Internal player > Intro skip > Enable auto skip | Skips intros without a click. | **CUT** | false | Micro option. | UNWIRED; import: keep accepting on import |
| `pref_enable_netflixStyle_aniskip` | Player > Internal player > Intro skip > Enable Netflix style | Netflix-style skip button. | **CUT** | false | Micro option; nothing reads it. | DEAD; import: keep accepting on import |
| `pref_waiting_time_aniskip` | Player > Internal player > Intro skip > Button timeout | Seconds the skip button stays (5 to 10). | **CUT** | 5 | Micro option; nothing reads it. | DEAD; import: keep accepting on import |
| `pref_enable_ani_skip` | Player > Internal player > Intro skip > Enable AniSkip | Fetches intro times from the AniSkip service. | **CUT** | false | Needs a MAL or AniList track and a network call; borderline, see the borderline list. | UNWIRED; import: keep accepting on import |
| `pref_disabled_ani_skip_chapters` | Player > Internal player > Intro skip > Disable AniSkip if video already contains chapters | Skips AniSkip when the file has chapters. | **CUT** | true | Goes with AniSkip. | UNWIRED; import: keep accepting on import |
| `pref_enable_pip` | Player > Internal player > Mini player > Enable mini player | Allows the small floating player. | **CUT** | true | Row only shows on some displays; keep the feature on, drop the switch. | UNWIRED; import: keep accepting on import |
| `pref_pip_episode_toasts` | Player > Internal player > Mini player > episode change toasts | Toast when the episode changes in the mini player. | **CUT** | true | Micro option. | UNWIRED; import: keep accepting on import |
| `pref_pip_on_exit` | Player > Internal player > Mini player > Open mini player on exit | Mini player opens when you leave. | **CUT** | false | Micro option. | UNWIRED; import: keep accepting on import |
| `pip_replace_with_previous` | Player > Internal player > Mini player > Switch next to previous | Mini player buttons order. | **CUT** | false | Micro option. | UNWIRED; import: keep accepting on import |
| `pref_always_use_external_player` | Player > Internal player > External player > Always use external player | Opens episodes in another player program. | **CUT** | false | Android intent idea; the Play in external player menu action stays, the always-on switch goes. CONFIRM: it is read by 4 screens today. | READ-BY-APP (library/mount, updates, entry route); import: keep accepting on import |
| `external_player_preference` | Player > Internal player > External player > External player preference | Which external player. | **CUT** | "" | Android package picker; nothing in the app reads it. | UNWIRED; import: keep accepting on import |
| `pref_player_speed` | In the player (speed control) | Playback speed to start at. | **CUT** | 1.0 | Per-session control; no setting row. | UNWIRED; import: keep accepting on import |
| `default_speed_presets` | In the player (speed sheet) | Speed values offered in the sheet. | **CUT** | the 12 default presets | Micro option. | UNWIRED; import: keep accepting on import |
| `invert_duration` | In the player (time label tap) | Shows remaining time instead of total. | **CUT** | false | Micro option. | UNWIRED; import: keep accepting on import |
| `pref_player_aspect_state` | In the player (aspect cycle button) | Last chosen aspect mode. | **CUT** | Fit | Per-session; always start at Fit. | UNWIRED; import: keep accepting on import |
| `pref_auto_play_enabled` | Player > Autoplay next episode (new switch; today only in the player menu) | Plays the next episode automatically. | **KEEP** |  | A big comfort setting for series watching. | UNWIRED |
| `pref_gesture_horizontal_seek` | Player > Gestures > Seeking > Enable Horizontal Seek Gesture | Swipe left or right to seek. | **CUT** | true | Touch gesture. | UNWIRED; import: keep accepting on import |
| `pref_show_seekbar` | Player > Gestures > Seeking > Show seekbar when seeking | Shows the bar while seeking. | **CUT** | false | Micro option. | UNWIRED; import: keep accepting on import |
| `pref_default_intro_length` | Player > Gestures > Seeking > Default skip intro length | Seconds the manual skip jumps (85). | **CUT** | 85 | Micro option. Read by src/entry_core/anime/route.rs today. | READ-BY-APP (entry route); import: keep accepting on import |
| `pref_skip_length_preference` | Player > Gestures > Seeking > Double tap to skip length | Seconds a double tap skips (3 to 30). | **CUT** | 10 | Micro option; arrow keys cover seeking. | UNWIRED; import: keep accepting on import |
| `pref_player_smooth_seek` | Player > Gestures > Seeking > Enable precise seeking | Exact instead of keyframe seeking. | **CUT** | false | Micro option. | UNWIRED; import: keep accepting on import |
| `pref_left_double_tap` | Player > Gestures > Double tap (left) | Action of a left double tap. | **CUT** | Seek | Touch gesture. | UNWIRED; import: keep accepting on import |
| `pref_center_double_tap` | Player > Gestures > Double tap (center) | Action of a center double tap. | **CUT** | PlayPause | Touch gesture. | UNWIRED; import: keep accepting on import |
| `pref_right_double_tap` | Player > Gestures > Double tap (right) | Action of a right double tap. | **CUT** | Seek | Touch gesture. | UNWIRED; import: keep accepting on import |
| `pref_media_previous` | Player > Gestures > Media controls > Previous | Action of the Previous media key. | **CUT** | Switch | mpv input.conf and the desktop media keys already bind these. | UNWIRED; import: keep accepting on import |
| `pref_media_playpause` | Player > Gestures > Media controls > Play/Pause | Action of the Play/Pause media key. | **CUT** | PlayPause | Same as above. | UNWIRED; import: keep accepting on import |
| `pref_media_next` | Player > Gestures > Media controls > Next | Action of the Next media key. | **CUT** | Switch | Same as above. | UNWIRED; import: keep accepting on import |
| `pref_try_hwdec` | Player > Decoder > Try hardware decoding | Lets mpv use the GPU decoder. | **CUT** | true | Right default; mpv.conf can override (hwdec=no). | UNWIRED; import: keep accepting on import |
| `pref_gpu_next` | Player > Decoder > Enable gpu-next | Uses the gpu-next renderer. | **CUT** | false | mpv.conf can set it. | UNWIRED; import: keep accepting on import |
| `pref_video_debanding` | Player > Decoder > Debanding | Removes colour banding. | **CUT** | None | mpv.conf can set it. | UNWIRED; import: keep accepting on import |
| `use_yuv420p` | Player > Decoder > Use YUV420P pixel format | Forces one pixel format. | **CUT** | true | mpv.conf can set it. | UNWIRED; import: keep accepting on import |
| `pref_player_filter_brightness` | Nowhere (no row) | Video brightness filter. | **CUT** | 0 | Colour filter. | UNWIRED; import: keep accepting on import |
| `pref_player_filter_saturation` | Nowhere (no row) | Video saturation filter. | **CUT** | 0 | Colour filter. | UNWIRED; import: keep accepting on import |
| `pref_player_filter_contrast` | Nowhere (no row) | Video contrast filter. | **CUT** | 0 | Colour filter. | UNWIRED; import: keep accepting on import |
| `pref_player_filter_gamma` | Nowhere (no row) | Video gamma filter. | **CUT** | 0 | Colour filter. | UNWIRED; import: keep accepting on import |
| `pref_player_filter_hue` | Nowhere (no row) | Video hue filter. | **CUT** | 0 | Colour filter. | UNWIRED; import: keep accepting on import |
| `pref_audio_lang` | Player > Audio > Preferred languages | Audio languages picked first, comma separated. | **KEEP** |  | Always keep: audio language. | UNWIRED |
| `pref_audio_pitch_correction` | Player > Audio > Enable audio pitch correction | Keeps pitch at other speeds. | **CUT** | true | Right default. | UNWIRED; import: keep accepting on import |
| `pref_audio_config` | Player > Audio > Audio channels | Auto, mono, stereo, reverse stereo. | **CUT** | AutoSafe | mpv.conf can set it. | UNWIRED; import: keep accepting on import |
| `pref_audio_volume_boost_cap` | Player > Audio > Volume boost cap | How far volume may go past 100 percent. | **CUT** | 30 | Micro option. | UNWIRED; import: keep accepting on import |
| `pref_audio_delay` | Nowhere (no row) | Audio delay in ms. | **CUT** | 0 | Per-session control. | UNWIRED; import: keep accepting on import |
| `pref_subtitle_lang` | Player > Subtitles > Preferred languages | Subtitle languages picked first, comma separated. | **KEEP** |  | Always keep: subtitle language. | UNWIRED |
| `pref_subtitle_whitelist` | Player > Subtitles > Whitelist | Words a wanted subtitle track must contain. | **CUT** | "" | Micro option. | UNWIRED; import: keep accepting on import |
| `pref_subtitle_blacklist` | Player > Subtitles > Blacklist | Words that rule out a subtitle track. | **CUT** | "" | Micro option. | UNWIRED; import: keep accepting on import |
| `pref_screenshot_subtitles` | Nowhere (no row) | Include subtitles in screenshots. | **CUT** | false | Nothing reads it. | DEAD; import: keep accepting on import |
| `pref_subtitle_font` | Nowhere (no row; subtitle look panel is not built) | Subtitle font. | **CUT** | Sans Serif | Subtitle look micro tweak; mpv defaults and mpv.conf (sub-font) cover it. | UNWIRED; import: keep accepting on import |
| `pref_subtitles_font_size` | Nowhere (no row) | Subtitle font size. | **CUT** | 55 | Same as above. | UNWIRED; import: keep accepting on import |
| `pref_sub_scale` | Nowhere (no row) | Subtitle scale. | **CUT** | 1.0 | Same as above. | UNWIRED; import: keep accepting on import |
| `pref_sub_border_size` | Nowhere (no row) | Subtitle outline size. | **CUT** | 3 | Same as above. | UNWIRED; import: keep accepting on import |
| `pref_bold_subtitles` | Nowhere (no row) | Bold subtitles. | **CUT** | false | Same as above. | UNWIRED; import: keep accepting on import |
| `pref_italic_subtitles` | Nowhere (no row) | Italic subtitles. | **CUT** | false | Same as above. | UNWIRED; import: keep accepting on import |
| `pref_text_color_subtitles` | Nowhere (no row) | Subtitle text colour. | **CUT** | -1 (white) | Colour tweak. | UNWIRED; import: keep accepting on import |
| `pref_border_color_subtitles` | Nowhere (no row) | Subtitle outline colour. | **CUT** | -16777216 (black) | Colour tweak. | UNWIRED; import: keep accepting on import |
| `pref_border_style_subtitles` | Nowhere (no row) | Outline and shadow style. | **CUT** | OutlineAndShadow | Same as above. | UNWIRED; import: keep accepting on import |
| `sub_shadow_offset` | Nowhere (no row) | Subtitle shadow offset. | **CUT** | 0 | Same as above. | UNWIRED; import: keep accepting on import |
| `pref_background_color_subtitles` | Nowhere (no row) | Subtitle background colour. | **CUT** | 0 (none) | Colour tweak. | UNWIRED; import: keep accepting on import |
| `pref_sub_justify` | Nowhere (no row) | Subtitle text alignment. | **CUT** | Auto | Same as above. | UNWIRED; import: keep accepting on import |
| `pref_sub_pos` | Nowhere (no row) | Subtitle vertical position. | **CUT** | 100 | Same as above. | UNWIRED; import: keep accepting on import |
| `pref_override_subtitles_ass` | Nowhere (no row) | Override styles of ASS subtitles. | **CUT** | false | Same as above. | UNWIRED; import: keep accepting on import |
| `pref_subtitles_delay` | Nowhere (no row; in-player delay card) | Subtitle delay in ms. | **CUT** | 0 | Per-session adjustment, not a setting. | UNWIRED; import: keep accepting on import |
| `pref_subtitles_speed` | Nowhere (no row) | Subtitle speed factor. | **CUT** | 1.0 | Per-session adjustment. | UNWIRED; import: keep accepting on import |
| `pref_subtitles_secondary_delay` | Nowhere (no row) | Second subtitle track delay. | **CUT** | 0 | Per-session adjustment. | UNWIRED; import: keep accepting on import |
| `mpv_scripts` | Player > Advanced > Use user MPV files | Loads scripts, fonts and shaders from the mpv-config folder. | **CUT** | true | Always load the folder; the mpv config folder IS the one advanced item and needs no UI. | UNWIRED; import: keep accepting on import |
| `pref_mpv_conf` | Player > Advanced > Edit MPV configuration file | Text of mpv.conf kept as a setting. | **CUT** | "" (use the real mpv.conf file) | The real file in the mpv-config folder replaces the stored copy. On import write the text to mpv.conf only if the file is missing. | UNWIRED; import: keep accepting on import |
| `pref_mpv_input` | Player > Advanced > Edit MPV input file | Text of input.conf kept as a setting. | **CUT** | "" (use the real input.conf file) | Same as above. | UNWIRED; import: keep accepting on import |
| `pref_player_statistics_page` | In the player (stats overlay page) | Which mpv stats page shows. | **CUT** | 0 | Per-session; nothing in the app feeds it. | UNWIRED; import: keep accepting on import |
| `pref_remember_decoder` | Nowhere (no row) | Opt-in: remember the decoder chosen last time. Kurobayomi-only. | **CUT** | false | Micro option; hwdec choice goes to mpv.conf. Read by src/player_view/mod.rs today, so remove that code too. | READ-BY-APP; import: n/a (Kurobayomi-only key, not in Aniyomi backups) |
| `player_decoder` | Nowhere (state of the above) | The remembered decoder. Kurobayomi-only. | **CUT** | "" | Goes with remember decoder. | READ-BY-APP; import: n/a (Kurobayomi-only key, not in Aniyomi backups) |
| `player_open_detached` | In the player (detach button) | Removed with the Detach button (user, 2026-10-11). | **CUT** |  | Detach feature cut. | DROPPED (in `DROPPED_KEYS`; import skips it) |
| `pref_torrserver_enable` | Player > Torrent > Enable TorrServer | Turns the embedded torrent server on. | **KEEP-LATER** |  | Torrent server is being kept for later. | READ-BY-APP (entry route) |
| `pref_torrserver_shownotice` | Nowhere | Remembers that the server notice was shown. | **KEEP-LATER** |  | Torrent server kept for later. | DEAD |
| `pref_torrserver_port` | Player > Torrent > TorrServer port | Port of the torrent server. | **KEEP-LATER** |  | Torrent server kept for later. | UNWIRED |
| `pref_torrserver_tackers` | Player > Torrent > Torrent trackers | Announce URL list (26 defaults). | **KEEP-LATER** |  | Torrent server kept for later. | UNWIRED |
| `pref_torrserver_proxymode` | Player > Torrent > Proxy mode | None, tracker, peer or full proxy. | **KEEP-LATER** |  | Torrent server kept for later. | UNWIRED |
| `pref_torrserver_proxyurl` | Player > Torrent > Proxy url | The proxy address. | **KEEP-LATER** |  | Torrent server kept for later. | UNWIRED |

### Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these

23 keys: 19 CUT, 4 KEEP.

| Key | Where the user sees it | What it does | Verdict | If CUT: hard-code | Reason | Flags and import |
|---|---|---|---|---|---|---|
| `pref_download_only_over_wifi_key` | Downloads > Only on Wi-Fi | Downloads only on an unmetered network. | **CUT** | false | Phone data-plan idea; desktop cannot tell and the default true can block wired users. | UNWIRED; import: keep accepting on import |
| `use_external_downloader` | Downloads > External downloader > Always use external downloader for anime | Hands anime downloads to another app (Android intent). | **CUT** | false | Android-only hand-off. Read by src/misc/runner.rs and the anime downloader today, remove both. | READ-BY-APP; import: keep accepting on import |
| `external_downloader_selection` | Downloads > External downloader > Downloader app preference | Which downloader app. | **CUT** | "" | Android-only. | UNWIRED; import: keep accepting on import |
| `save_chapter_as_cbz` | Downloads > Save as CBZ archive | Saves a chapter as one .cbz file instead of a folder. | **CUT** | true | Micro option; one archive per chapter is the better default. | UNWIRED; import: keep accepting on import |
| `split_tall_images` | Downloads > Split tall images | Cuts very tall images into parts. | **CUT** | true | Micro option. | UNWIRED; import: keep accepting on import |
| `auto_download_while_reading` | Downloads > Download ahead > Auto download while reading | Downloads the next chapters while you read. | **CUT** | 0 (off) | Micro option; add back later if wanted. | UNWIRED; import: keep accepting on import |
| `auto_download_while_watching` | Downloads > Download ahead > Auto download while watching | Downloads the next episodes while you watch. | **CUT** | 0 (off) | Micro option. | UNWIRED; import: keep accepting on import |
| `remove_after_read_slots` | Downloads > Delete chapters/episodes > After reading/watching automatically delete | Deletes a download some items after you finish it. | **KEEP** |  | Basic download option that keeps the disk tidy. | UNWIRED |
| `pref_remove_after_marked_as_read_key` | Downloads > Delete chapters/episodes > After manually marked as read/watched | Also deletes when you mark by hand. | **CUT** | false | Micro option. | UNWIRED; import: keep accepting on import |
| `pref_remove_bookmarked` | Downloads > Delete chapters/episodes > Allow deleting bookmarked | Auto delete may remove bookmarked items. | **CUT** | false | Micro option. | UNWIRED; import: keep accepting on import |
| `pref_download_fillermarked` | Downloads > Delete chapters/episodes > Allow downloading of fillermarked episodes | Auto download takes filler episodes too. | **CUT** | false | Micro option. | UNWIRED; import: keep accepting on import |
| `remove_exclude_categories` | Downloads > Delete chapters/episodes > Excluded manga categories | Manga categories never auto-deleted. | **CUT** | {} | Micro option. | UNWIRED; import: keep accepting on import |
| `remove_exclude_anime_categories` | Downloads > Delete chapters/episodes > Excluded anime categories | Anime categories never auto-deleted. | **CUT** | {} | Micro option. | UNWIRED; import: keep accepting on import |
| `download_new` | Downloads > Auto-download > Download new chapters | Downloads new chapters found by a library update. | **KEEP** |  | Basic download option. | READ-BY-APP (src/misc/runner.rs) |
| `download_new_episode` | Downloads > Auto-download > Download new episodes | Downloads new episodes found by a library update. | **KEEP** |  | Basic download option. | READ-BY-APP (src/misc/runner.rs) |
| `download_new_categories` | Downloads > Auto-download > Manga Categories (included) | Manga categories whose new chapters download. | **CUT** | {} (all) | Scope micro option. | READ-BY-APP (runner); import: keep accepting on import |
| `download_new_anime_categories` | Downloads > Auto-download > Anime Categories (included) | Anime categories whose new episodes download. | **CUT** | {} (all) | Scope micro option. | READ-BY-APP (runner); import: keep accepting on import |
| `download_new_categories_exclude` | Downloads > Auto-download > Manga Categories (excluded) | Manga categories left out. | **CUT** | {} | Scope micro option. | READ-BY-APP (runner); import: keep accepting on import |
| `download_new_anime_categories_exclude` | Downloads > Auto-download > Anime Categories (excluded) | Anime categories left out. | **CUT** | {} | Scope micro option. | READ-BY-APP (runner); import: keep accepting on import |
| `download_slots` | Downloads > Maximum downloads | How many downloads run at once (1 to 5). | **KEEP** |  | Basic download option. | UNWIRED |
| `download_speed_limit` | Downloads > Chapter download speed limit | Speed cap in KB/s for chapter downloads. | **CUT** | 0 (no limit) | Micro option. | UNWIRED; import: keep accepting on import |
| `download_new_unread_chapters_only` | Nowhere (no row) | Auto download only if earlier chapters are read. | **CUT** | false | Micro option; no row shows it. | READ-BY-APP (runner); import: keep accepting on import |
| `download_new_unread_episodes_only` | Nowhere (no row) | Auto download only if earlier episodes are seen. | **CUT** | false | Micro option; no row shows it. | READ-BY-APP (runner); import: keep accepting on import |

### Browse (settings_browse and the Sources tab)

31 keys: 21 CUT, 10 KEEP.

| Key | Where the user sees it | What it does | Verdict | If CUT: hard-code | Reason | Flags and import |
|---|---|---|---|---|---|---|
| `pref_display_mode_catalogue` | Browse > source results (grid or list toggle) | Grid or list look of a source's results. | **CUT** | COMPACT_GRID | Micro option. | import: keep accepting on import |
| `source_languages` | Browse > Sources > language filter | Languages whose sources and extensions show. | **KEEP** |  | Without it a new user drowns in 40 languages. | - |
| `show_nsfw_source` | Browse > NSFW (18+) sources > Show in sources and extensions lists | Shows or hides 18+ sources. | **KEEP** |  | Content-safety switch, not a micro option. | - |
| `pref_migration_sorting` | Browse > Migrate sources list | Sort field of the migrate list. | **CUT** | ALPHABETICAL | Micro option. | import: keep accepting on import |
| `pref_migration_direction` | Browse > Migrate sources list | Sort direction of the migrate list. | **CUT** | ASCENDING | Micro option. | import: keep accepting on import |
| `anime_extension_repos` | Browse > Sources > Anime extension repos | The anime extension repository addresses. | **KEEP** |  | Always keep: source repository management. | - |
| `extension_repos` | Browse > Sources > Manga extension repos | The manga extension repository addresses. | **KEEP** |  | Always keep: source repository management. | - |
| `__APP_STATE_trusted_extensions` | Nowhere (state; Advanced > Revoke trusted unknown extensions moves to Browse) | Extensions the user trusted once. | **KEEP** |  | Security state of extension loading. | STATE |
| `__APP_STATE_has_filters_toggle_state` | Browse > global search (Pinned only toggle) | Remembers the pinned-only toggle of global search. | **CUT** | false | Micro state. | import: app state, not in normal backups; skip it if present |
| `hidden_anime_catalogues` | Aniyomi backups only | Aniyomi's disabled anime sources. | **KEEP** |  | Read once after a restore: every known source minus these is turned on, then the key is cleared (2026-10-23). Kurobayomi stores the sources that are on in `enabled_sources`. | import: restored, then converted |
| `hidden_catalogues` | Aniyomi backups only | Aniyomi's disabled manga sources. | **KEEP** |  | As above. | import: restored, then converted |
| `incognito_anime_extensions` | Browse > extension details > Incognito | Anime extensions that never write history. | **CUT** | {} | Per-extension micro option; global incognito covers it. | import: keep accepting on import |
| `incognito_manga_extensions` | Browse > extension details > Incognito | Manga extensions that never write history (the extension details page writes the unregistered key incognito_extensions instead). | **CUT** | {} | Same as above. | import: keep accepting on import |
| `pinned_anime_catalogues` | Browse > Sources > pin a source (anime) | Anime sources pinned to the top. | **CUT** | {} | 2026-10-23: one on/off switch per source, no pinning. | import: in `DROPPED_KEYS`, skipped |
| `pinned_catalogues` | Browse > Sources > pin a source (manga) | Manga sources pinned to the top. | **CUT** | {} | As above. | import: in `DROPPED_KEYS`, skipped |
| `__APP_STATE_last_anime_catalogue_source` | Browse > Sources > Last used group (anime) | Id of the last anime source opened. | **CUT** | -1 | Micro convenience; pinned sources do the job. | import: app state, not in normal backups; skip it if present |
| `__APP_STATE_last_catalogue_source` | Browse > Sources > Last used group (manga) | Id of the last manga source opened. | **CUT** | -1 | Same as above. | import: app state, not in normal backups; skip it if present |
| `ext_updates_count` | Browse tab badge | Cached number of manga extension updates. | **CUT** | 0 | Derive in memory like the anime count (registry note T-19a-064). | import: keep accepting on import |
| `show_not_runnable` | Nowhere (no row; Kurobayomi-only) | Lists extensions that cannot run on Linux. | **CUT** | false | Hidden toggle without a row; extensions that cannot run have no use. | import: n/a (Kurobayomi-only key, not in Aniyomi backups) |
| `anime_ext_last_check` | Nowhere (state; Kurobayomi-only) | When extensions were last checked for updates. | **KEEP** |  | State that throttles the update check. | STATE |
| `browse_hide_in_anime_library_items` | Browse > Sources > Hide anime entries already in library | Hides library entries in source results. | **CUT** | false | Micro option; the in-library badge already tells. | import: keep accepting on import |
| `browse_hide_in_library_items` | Browse > Sources > Hide manga entries already in library | Hides library entries in source results. | **CUT** | false | Micro option. | import: keep accepting on import |
| `data_saver` | Advanced > Data Saver > Data Saver | Routes images through the Bandwidth Hero proxy (none or one service). | **CUT** | NONE | Phone data-plan idea; users of a desktop connection do not use it. | UNWIRED (loader and downloader take a DataSaver the app never builds from the stored mode); import: skipped on import (in `DROPPED_KEYS`) |
| `ignore_jpeg` | Advanced > Data Saver > Ignore Jpeg Images | Skip proxy for jpeg. | **CUT** | false | Part of Data Saver. | UNWIRED (kuro-downloads has a DataSaver; the app never builds it from this key); import: skipped on import (in `DROPPED_KEYS`) |
| `ignore_gif` | Advanced > Data Saver > Ignore Gif Animations | Skip proxy for gif. | **CUT** | true | Part of Data Saver. | UNWIRED (kuro-downloads has a DataSaver; the app never builds it from this key); import: skipped on import (in `DROPPED_KEYS`) |
| `data_saver_image_quality` | Advanced > Data Saver > Image Quality | Proxy image quality percent. | **CUT** | 80 | Part of Data Saver. | UNWIRED (kuro-downloads has a DataSaver; the app never builds it from this key); import: skipped on import (in `DROPPED_KEYS`) |
| `data_saver_image_format_jpeg` | Advanced > Data Saver > Compress to Jpeg | Proxy returns jpeg. | **CUT** | false | Part of Data Saver. | UNWIRED (kuro-downloads has a DataSaver; the app never builds it from this key); import: skipped on import (in `DROPPED_KEYS`) |
| `data_saver_server` | Advanced > Data Saver > Bandwidth Hero Proxy Server | Address of the proxy. | **CUT** | "" | Part of Data Saver. | UNWIRED (kuro-downloads has a DataSaver; the app never builds it from this key); import: skipped on import (in `DROPPED_KEYS`) |
| `data_saver_color_bw` | Advanced > Data Saver > Convert to Black And White | Proxy returns grey images. | **CUT** | false | Part of Data Saver. | UNWIRED (kuro-downloads has a DataSaver; the app never builds it from this key); import: skipped on import (in `DROPPED_KEYS`) |
| `data_saver_excluded` | Browse > source menu > exclude from data saver | Sources the proxy skips. | **CUT** | {} | Part of Data Saver. | READ-BY-APP (kuro-domain source app); import: skipped on import (in `DROPPED_KEYS`) |
| `data_saver_downloader` | Advanced > Data Saver > Use data saver in the downloader | Applies the proxy to downloads. | **CUT** | true | Part of Data Saver. | UNWIRED (kuro-downloads has a DataSaver; the app never builds it from this key); import: skipped on import (in `DROPPED_KEYS`) |

### Tracking (settings_tracking)

9 keys: 3 CUT, 3 KEEP, 3 KEEP-LATER.

| Key | Where the user sees it | What it does | Verdict | If CUT: hard-code | Reason | Flags and import |
|---|---|---|---|---|---|---|
| `__PRIVATE_pref_mangasync_username_{id}` | Tracking > Trackers > login dialog (username, trackers that use a password) | Tracker username (Kitsu, MangaUpdates and similar). | **KEEP-LATER** |  | Other trackers' code is kept for later; AniList uses a token only. Keep accepting on import. | FAMILY; NOT-READ-AS-PREF (secrets live in the keyring) |
| `__PRIVATE_pref_mangasync_password_{id}` | Tracking > Trackers > login dialog (password) | Tracker password. | **KEEP-LATER** |  | Same as above. | FAMILY; NOT-READ-AS-PREF |
| `__PRIVATE_pref_tracker_auth_expired_{id}` | Nowhere (state per tracker) | Marks a login as expired. | **KEEP-LATER** |  | Other trackers kept for later. | FAMILY; NOT-READ-AS-PREF |
| `__PRIVATE_track_token_{id}` | Tracking > Trackers > AniList (login) | Tracker access token; the AniList one is the login. | **KEEP** |  | Always keep: AniList login (other ids KEEP-LATER). | FAMILY; NOT-READ-AS-PREF |
| `anilist_score_type` | Nowhere (set from the AniList profile at login) | How AniList scores are written (10 point, 100 point, stars...). | **KEEP** |  | Needed to write a score; not a user choice. | STATE |
| `pref_auto_update_manga_sync_key` | Tracking > Update progress after reading/watching | Sends progress to the tracker when you finish an item. | **KEEP** |  | The reason to log in at all. | - |
| `track_on_adding_to_library` | Tracking > Open track menu on adding to library | Opens the track dialog when you add an entry. | **CUT** | true | Micro option; make sure the dialog only appears when a tracker is logged in. | import: keep accepting on import |
| `show_next_episode_airing_time` | Tracking > Show next episode's airing time | Shows the next air date on anime pages. | **CUT** | true | Micro option. | import: keep accepting on import |
| `pref_auto_update_manga_on_mark_read` | Tracking > Update progress when marked as read/watched | Always, ask or never when you mark by hand. | **CUT** | ALWAYS | Micro option; one switch (above) is enough. | import: keep accepting on import |

### Data and storage (settings_data_storage, settings_create_backup)

5 keys: 2 CUT, 3 KEEP.

| Key | Where the user sees it | What it does | Verdict | If CUT: hard-code | Reason | Flags and import |
|---|---|---|---|---|---|---|
| `backup_interval` | Data and storage > Backup and restore > Automatic backup frequency | How often an automatic backup runs. | **KEEP** |  | Always keep: backup. | - |
| `__APP_STATE_last_auto_backup_timestamp` | Data and storage > note under the backup rows | When the last automatic backup ran. | **KEEP** |  | State; shown as Last automatically backed up. | STATE |
| `backup_flags` | Data and storage > Create backup > what to include | Remembered choices of the create-backup dialog (library, chapters, tracking, history, categories...). | **CUT** | all four flags (everything) | One button, one full backup; fewer ways to restore half a library. | import: keep accepting on import |
| `__APP_STATE_storage_dir` | Data and storage > Storage location (moves to Downloads > Download location) | Base folder for downloads, backups, local source and mpv config. | **KEEP** |  | Always keep: download location. | - |
| `auto_clear_chapter_cache` | Data and storage > Storage usage > Clear episode/chapter cache on app launch | Empties the reading cache at each start. | **CUT** | false | Micro option; the Clear cache button stays. | READ-BY-APP (src/main.rs); import: keep accepting on import |

### Security and privacy (settings_security)

4 keys: 4 CUT.

| Key | Where the user sees it | What it does | Verdict | If CUT: hard-code | Reason | Flags and import |
|---|---|---|---|---|---|---|
| `use_biometric_lock` | Security and privacy > Require unlock | Asks for a passphrase when the app opens (Android: fingerprint). | **CUT** | false | Phone idea; the desktop login already guards the session. CONFIRM: a working passphrase lock exists in src/applock_host.rs and src/util/app_lock_auth.rs. | READ-BY-APP; import: keep accepting on import |
| `lock_app_after` | Security and privacy > Lock when idle | Minutes before the lock returns. | **CUT** | 0 | Goes with the app lock. | READ-BY-APP; import: keep accepting on import |
| `hide_notification_content` | Security and privacy > Hide notification content | Notifications show no title or cover. | **CUT** | false | Micro option; desktop notification privacy is the system's job. | READ-BY-APP; import: keep accepting on import |
| `__APP_STATE_last_app_closed` | Nowhere (state of the app lock) | When the app last closed, for the idle lock. | **CUT** | 0 | Goes with the app lock. | READ-BY-APP; import: app state, not in normal backups; skip it if present |

### Advanced (settings_advanced) and network

5 keys: 5 CUT.

| Key | Where the user sees it | What it does | Verdict | If CUT: hard-code | Reason | Flags and import |
|---|---|---|---|---|---|---|
| `verbose_logging` | Advanced > Verbose logging | More detailed logs. | **CUT** | false (release) / true in debug builds | Use RUST_LOG for debugging instead of a setting. | READ-BY-APP; import: keep accepting on import |
| `doh_provider` | Advanced > Networking > DNS over HTTPS (DoH) | Picks a DNS-over-HTTPS service. | **CUT** | -1 (off) | The system resolver or a VPN owns this on a desktop. | READ-BY-APP; import: keep accepting on import |
| `default_user_agent` | Advanced > Networking > Default user agent string | Browser identity sent by sources. | **CUT** | the Firefox 136 string | Power-user option; share the one constant with the future WebView and Cloudflare window. | READ-BY-APP; import: keep accepting on import |
| `pref_display_profile_key` | Advanced > Reader > Custom display profile | Colour profile file applied to pages. | **CUT** | "" | Micro option; the desktop colour management owns this. | READ-BY-APP (reader/page_profile.rs); import: keep accepting on import |
| `pref_always_decode_long_strip_with_ssiv` | Nowhere (switch dropped, T-44a-061) | Legacy long strip decoder. | **CUT** | false | Android decoder; the viewer ignores it. | DEAD; import: keep accepting on import |

### Base state and notifications (no settings page)

6 keys: 2 CUT, 4 KEEP.

| Key | Where the user sees it | What it does | Verdict | If CUT: hard-code | Reason | Flags and import |
|---|---|---|---|---|---|---|
| `__APP_STATE_pref_downloaded_only` | Library view menu > Show only > Downloaded | Shows only downloaded entries everywhere. | **KEEP** |  | Core offline toggle, in the Library filter. | STATE |
| `__APP_STATE_incognito_mode` | App menu (the Kurobayomi title) > Incognito mode | Stops history and progress being saved. | **KEEP** |  | Core privacy toggle, in the app menu. | STATE |
| `__APP_STATE_onboarding_complete` | First-run guide | Remembers that the first-run guide was shown. | **KEEP** |  | State of the first-run guide. | STATE |
| `check_app_updates` | Nowhere (no row; Kurobayomi-only) | Checks for a new app version in the background. | **CUT** | false | The package manager updates a Linux app; keep a manual Check for updates in About. | READ-BY-APP (src/app_update.rs); import: n/a (Kurobayomi-only key, not in Aniyomi backups) |
| `__APP_STATE_last_app_check` | Nowhere (state of the above) | When the app last checked for a new version. | **CUT** | 0 | Goes with the background check. | READ-BY-APP; import: n/a (Kurobayomi-only key, not in Aniyomi backups) |
| `muted_notification_channels` | Advanced > Manage notifications > Muted notification channels (13 channels) | Which notification channels are muted. | **KEEP** |  | Always keep notifications for new items: replace the 13-channel list with ONE switch Notify about new chapters and episodes (channel new_chapters_episodes_channel); the rest follows the system notification settings. | - |

## 7. Keys outside the registry

Found by reading the code. They are real keys the app reads or writes with no registry row, so tests and import never see them. Verdicts:

| Key | Where | What it does | Verdict | If CUT: hard-code | Reason | Flags and import |
|---|---|---|---|---|---|---|
| `app_language` | Appearance > App language (`src/pref_model/language.rs`) | Language tag, empty follows the system. | **KEEP** |  | Always keep. Add a registry row. | OUTSIDE-REGISTRY |
| `restore_after_crash` | reader, no row (`src/reader/restore.rs`) | Reopen the reader where it was after a crash (24 h window). | **CUT** | true | Micro option; always on. | OUTSIDE-REGISTRY; Kurobayomi-only, no import |
| `pref_hardware_bitmap_threshold` | Advanced > Reader > Custom hardware bitmap threshold (`src/settings_ui/decode.rs`, `src/reader/tiles.rs`) | Largest image tile in pixels. | **CUT** | 4096 | Android texture limit. `DROPPED_KEYS` already lists it as gone, yet two files still read and write it: delete both. | OUTSIDE-REGISTRY; import: keep accepting on import (already in DROPPED_KEYS) |
| `migrate_flags` | Browse > Migrate dialog (`src/browse/prefs.rs`) | What a manga migration carries over, remembered. | **CUT** | everything (i32::MAX) | Micro option. | OUTSIDE-REGISTRY; import: keep accepting on import |
| `anime_migrate_flags` | Browse > Migrate dialog (`src/browse/anime/prefs.rs`) | Same for anime. | **CUT** | everything (i32::MAX) | Micro option. | OUTSIDE-REGISTRY; Kurobayomi-only |
| `incognito_extensions` | Browse > extension details (`src/browse/ext_details_page.rs`) | Extensions that never write history. The registry has `incognito_manga_extensions` for the same job: two names for one thing. | **CUT** | {} | Per-extension micro option; global incognito covers it. | OUTSIDE-REGISTRY; import: keep accepting on import |
| `__APP_STATE_backup_enabled_at` | none (`src/backup_run.rs`) | When automatic backup was switched on. | **KEEP** |  | State of the backup schedule. | OUTSIDE-REGISTRY; STATE |
| `__APP_STATE_last_version_code` | none (`crates/kuro-prefs/src/migrate.rs`) | Version the migrations last ran for. | **KEEP** |  | State the migrations need. | OUTSIDE-REGISTRY; STATE |
| `upcoming.week_start` | Updates > Upcoming (`src/updates/prefs.rs`) | Calendar starts on Monday, Sunday or by locale. | **CUT** | auto (locale) | Micro option; the locale decides. | OUTSIDE-REGISTRY; Kurobayomi-only |
| `upcoming.home_returns_today` | Updates > Upcoming (`src/updates/prefs.rs`) | Home key jumps the calendar back to this month. | **CUT** | false | Micro option, off by default. | OUTSIDE-REGISTRY; Kurobayomi-only |
| `bangumi_episode_progress` | Tracking > Bangumi (`crates/kuro-track/src/prefs.rs`) | Bangumi progress counts episodes. | **KEEP-LATER** |  | Other tracker kept for later. | OUTSIDE-REGISTRY |

That is 11 keys: KEEP 3, CUT 7, KEEP-LATER 1. Cutting them follows the same rule as section 4; add the key to `DROPPED_KEYS` where it was ever stored by Aniyomi.

## 8. Rows that are not preference keys (actions, pages, dialogs)

| Row or page | Verdict | Note |
|---|---|---|
| Appearance > App language | KEEP | key `app_language` |
| Library > Edit anime categories, Edit manga categories | KEEP | Always keep: categories. |
| Library > Global update > Anime Categories, Manga Categories | CUT | update scope; the four sets above |
| Downloads > Chapter download speed limit | CUT | micro; `download_speed_limit` |
| Downloads > Excluded categories rows, Auto-download category rows | CUT | scope micro options |
| Browse > Anime extension repos, Manga extension repos | KEEP | repository management |
| Browse > NSFW info text | CUT | prose only |
| Advanced > Revoke trusted unknown extensions | KEEP | move to Sources and tracking |
| Tracking > MyAnimeList, Kitsu, MangaUpdates, Shikimori, Simkl, Bangumi, Komga, Kavita, Suwayomi, Jellyfin | CUT (rows only) | Their code stays (KEEP-LATER). Only AniList shows. |
| Tracking > AniList | KEEP | login |
| Data and storage > Storage location | KEEP | moves to Downloads and storage as Download location |
| Data and storage > Create backup, Restore backup | KEEP | drop the 12-row create dialog and the 8-row restore dialog: one full backup |
| Data and storage > Storage (per-entry usage list) | CUT (confirm) | No download manager yet to act on it. |
| Data and storage > Clear chapter and episode cache | KEEP | one button |
| Data and storage > Export: Library list (CSV) | CUT (confirm) | power-user export |
| Advanced > Share crash logs, Debug info | KEEP | move into About |
| Advanced > Onboarding guide, Manage notifications (opens KDE settings) | CUT | first-run guide runs once; system owns notification settings |
| Advanced > Reindex downloads, Clear Manga database, Clear Anime database, Refresh library covers, Reset per-series reader settings | CUT (confirm) | maintenance actions; the entry page refresh covers covers |
| Advanced > Clear cookies, Clear WebView data | KEEP-LATER | WebView and Cloudflare window kept for later |
| Player > Custom buttons, Script editor, Code editor | CUT (confirm) | mpv.conf and input.conf files cover it; backups still import them |
| Player > Gestures, Decoder, Subtitles (look), Audio extras, Advanced | CUT | see Player table |
| Player > Torrent | KEEP-LATER | six keys |
| About > Version, Check for updates, What is new, Open source licenses | KEEP |  |
| About > Help translate, Privacy policy | CUT | links; micro |

## 9. Things seen on the way (not part of the diet)

- `incognito_extensions` (page) and `incognito_manga_extensions` (registry) are two names for one list.
- `pref_hardware_bitmap_threshold` is in `DROPPED_KEYS` yet `decode.rs` and `tiles.rs` read it.
- `src/settings_ui/appearance.rs`, `controls.rs` and others carry `#![allow(dead_code)]`; after the cuts, remove them.
- `library_update_restriction` default `wifi` and `download_only_over_wifi` default true would block a wired desktop.

## Decisions by the user (2026-10-11) on the seven open items

| Item | Decision |
|---|---|
| App lock | CUT |
| External player | CUT for now |
| Mini player | CUT (same reasoning as Detach) |
| Random sort | CUT |
| Custom buttons, Script editor, Code editor pages and the Lua bridge | CUT (the user's own `mpv.conf` and `input.conf` files are still honored; no editor UI) |
| Maintenance actions | KEEP |
| Data Saver | CUT (user, 2026-10-11: "remove data saver and detach") |
| Detach (the player and reader pop-out window) | CUT (user, 2026-10-11: "remove data saver and detach"); the pages stay embedded in the main window |

These cuts are applied in a follow-up pass after the code-first build, because the build's settings lanes ran with all seven kept.
