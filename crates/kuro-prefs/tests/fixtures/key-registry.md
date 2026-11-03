# Key registry: every preference declared in scope (generated from the Kotlin sources, then reviewed)

Source of rows: every `preferenceStore.get*(...)` declaration in the 18 `*Preferences` classes (327 rows + `extension_installer`, the hand-written preference in `app/src/main/java/eu/kanade/domain/base/ExtensionInstallerPreference.kt`, = 328 keys; the per-tracker and per-source families expand at runtime). Columns: **Key** is the literal string stored (after the `__APP_STATE_` / `__PRIVATE_` prefix, shown in Store). **Kotlin type** is the SharedPreferences primitive actually written. **Default** is the value used when the key is absent. **Rust field** is `Settings.<section>.<field>` for normal keys, `State.<section>.<field>` for app-state keys, `Secrets` for private keys. **Dec** = KEEP / KEEP-CHANGED / REPLACE / DROP. **Pri** is the implementation priority of the field.


## BasePreferences (`app/src/main/java/eu/kanade/domain/base/BasePreferences.kt`), 7 keys, Rust section `base`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `extensionInstaller()` | `extension_installer` | normal | Enum ExtensionInstaller (name string) | PACKAGEINSTALLER (LEGACY on MIUI) | `Settings.base.extension_installer` | DROP | P3 | APK installers (LEGACY, PACKAGEINSTALLER, SHIZUKU, PRIVATE) do not exist on Linux; replaced by the extension strategy. Value kept only so backups round-trip. |
| 2 | `downloadedOnly()` | `__APP_STATE_pref_downloaded_only` | app-state | Boolean | false | `State.base.downloaded_only` | KEEP | P0 | App-state, not backed up. |
| 3 | `incognitoMode()` | `__APP_STATE_incognito_mode` | app-state | Boolean | false | `State.base.incognito_mode` | KEEP | P0 | App-state, not backed up. |
| 4 | `shownOnboardingFlow()` | `__APP_STATE_onboarding_complete` | app-state | Boolean | false | `State.base.shown_onboarding_flow` | KEEP | P2 | App-state; first-run flag. |
| 5 | `displayProfile()` | `pref_display_profile_key` | normal | String | "" | `Settings.base.display_profile` | DROP | P3 | Android ICC display-profile path for the reader; GTK/GdkPixbuf has no equivalent switch. Loss: no custom colour profile for pages. |
| 6 | `hardwareBitmapThreshold()` | `pref_hardware_bitmap_threshold` | normal | Int | 2048 | `Settings.base.hardware_bitmap_threshold` | DROP | P3 | Android GPU texture limit for hardware bitmaps; GTK textures have no such user knob. |
| 7 | `alwaysDecodeLongStripWithSSIV()` | `pref_always_decode_long_strip_with_ssiv` | normal | Boolean | false | `Settings.base.always_decode_long_strip_with_ssiv` | DROP | P3 | Android SubsamplingScaleImageView workaround; Rust reader tiles tall strips itself. |

## UiPreferences (`app/src/main/java/eu/kanade/domain/ui/UiPreferences.kt`), 8 keys, Rust section `ui`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `themeMode()` | `pref_theme_mode_key` | normal | Enum ThemeMode (name string) | SYSTEM | `Settings.ui.theme_mode` | KEEP-CHANGED | P0 | LIGHT/DARK drive gtk-application-prefer-dark-theme; SYSTEM follows the desktop setting. |
| 2 | `appTheme()` | `pref_app_theme` | normal | Enum AppTheme (name string) | MONET if dynamic colour available else DEFAULT | `Settings.ui.app_theme` | KEEP-CHANGED | P1 | All enum names kept for backup round-trip; MONET maps to the desktop accent where available else DEFAULT; default on Linux is DEFAULT (no dynamic colour). Rendering owned. |
| 3 | `themeDarkAmoled()` | `pref_theme_dark_amoled_key` | normal | Boolean | false | `Settings.ui.theme_dark_amoled` | KEEP | P2 | Pure-black dark variant. |
| 4 | `relativeTime()` | `relative_time_v2` | normal | Boolean | true | `Settings.ui.relative_time` | KEEP | P1 |  |
| 5 | `dateFormat()` | `app_date_format` | normal | String | "" | `Settings.ui.date_format` | KEEP | P0 | Empty = locale SHORT date; else strftime-compatible pattern translated from java DateTimeFormatter pattern (see behavior). Empty string = locale SHORT date style. |
| 6 | `tabletUiMode()` | `tablet_ui_mode` | normal | Enum TabletUiMode (name string) | AUTOMATIC | `Settings.ui.tablet_ui_mode` | DROP | P3 | Android tablet/phone layout switch; desktop layout is responsive. Stored/restored but unused. |
| 7 | `startScreen()` | `start_screen` | normal | Enum StartScreen (name string) | ANIME | `Settings.ui.start_screen` | KEEP | P1 |  |
| 8 | `navStyle()` | `bottom_rail_nav_style` | normal | Enum NavStyle (name string) | MOVE_HISTORY_TO_MORE | `Settings.ui.nav_style` | KEEP-CHANGED | P2 | Controls which tab moves into "More" on narrow windows; wide windows use the sidebar and ignore it. |

## SourcePreferences (`app/src/main/java/eu/kanade/domain/source/service/SourcePreferences.kt`), 34 keys, Rust section `sources`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `sourceDisplayMode()` | `pref_display_mode_catalogue` | normal | Object LibraryDisplayMode (string codec) | COMPACT_GRID | `Settings.sources.source_display_mode` | KEEP | P1 |  |
| 2 | `enabledLanguages()` | `source_languages` | normal | Set<String> | {"all","en",<system language code>} | `Settings.sources.enabled_languages` | KEEP | P1 | Source language codes; "all" is the multi-language pseudo-language. |
| 3 | `showNsfwSource()` | `show_nsfw_source` | normal | Boolean | true | `Settings.sources.show_nsfw_source` | KEEP | P1 | Default true. |
| 4 | `migrationSortingMode()` | `pref_migration_sorting` | normal | Enum MigrateSortMode (name string) | ALPHABETICAL | `Settings.sources.migration_sorting_mode` | KEEP | P1 |  |
| 5 | `migrationSortingDirection()` | `pref_migration_direction` | normal | Enum MigrateSortDirection (name string) | ASCENDING | `Settings.sources.migration_sorting_direction` | KEEP | P1 |  |
| 6 | `animeExtensionRepos()` | `anime_extension_repos` | normal | Set<String> | {} | `Settings.sources.anime_extension_repos` | KEEP | P1 | Legacy: Aniyomi migrates to DB tables; Kurobayomi reads it only on backup import. |
| 7 | `mangaExtensionRepos()` | `extension_repos` | normal | Set<String> | {} | `Settings.sources.manga_extension_repos` | KEEP | P1 | Legacy: same. |
| 8 | `trustedExtensions()` | `__APP_STATE_trusted_extensions` | app-state | Set<String> | {} | `State.sources.trusted_extensions` | KEEP-CHANGED | P1 | Set of signing fingerprints on Android; extension strategy decides the Linux trust unit. |
| 9 | `globalSearchFilterState()` | `__APP_STATE_has_filters_toggle_state` | app-state | Boolean | false | `State.sources.global_search_filter_state` | KEEP | P1 |  |
| 10 | `disabledAnimeSources()` | `hidden_anime_catalogues` | normal | Set<String> | {} | `Settings.sources.disabled_anime_sources` | KEEP | P1 |  |
| 11 | `disabledMangaSources()` | `hidden_catalogues` | normal | Set<String> | {} | `Settings.sources.disabled_manga_sources` | KEEP | P1 |  |
| 12 | `incognitoAnimeExtensions()` | `incognito_anime_extensions` | normal | Set<String> | {} | `Settings.sources.incognito_anime_extensions` | KEEP | P2 |  |
| 13 | `incognitoMangaExtensions()` | `incognito_manga_extensions` | normal | Set<String> | {} | `Settings.sources.incognito_manga_extensions` | KEEP | P2 |  |
| 14 | `pinnedAnimeSources()` | `pinned_anime_catalogues` | normal | Set<String> | {} | `Settings.sources.pinned_anime_sources` | KEEP | P1 |  |
| 15 | `pinnedMangaSources()` | `pinned_catalogues` | normal | Set<String> | {} | `Settings.sources.pinned_manga_sources` | KEEP | P1 |  |
| 16 | `lastUsedAnimeSource()` | `__APP_STATE_last_anime_catalogue_source` | app-state | Long | -1 | `State.sources.last_used_anime_source` | KEEP | P1 | -1 = none. |
| 17 | `lastUsedMangaSource()` | `__APP_STATE_last_catalogue_source` | app-state | Long | -1 | `State.sources.last_used_manga_source` | KEEP | P1 | -1 = none. |
| 18 | `animeExtensionUpdatesCount()` | `animeext_updates_count` | normal | Int | 0 | `Settings.sources.anime_extension_updates_count` | KEEP | P2 | Derived counter; recompute instead of persisting is acceptable. |
| 19 | `mangaExtensionUpdatesCount()` | `ext_updates_count` | normal | Int | 0 | `Settings.sources.manga_extension_updates_count` | KEEP | P2 | Same. |
| 20 | `hideInAnimeLibraryItems()` | `browse_hide_in_anime_library_items` | normal | Boolean | false | `Settings.sources.hide_in_anime_library_items` | KEEP | P1 |  |
| 21 | `hideInMangaLibraryItems()` | `browse_hide_in_library_items` | normal | Boolean | false | `Settings.sources.hide_in_manga_library_items` | KEEP | P1 |  |
| 22 | `enableSourceBlacklist()` | `eh_enable_source_blacklist` | normal | Boolean | true | `Settings.sources.enable_source_blacklist` | KEEP | P1 |  |
| 23 | `sourcesTabCategories()` | `sources_tab_categories` | normal | Set<String> | {} | `Settings.sources.sources_tab_categories` | DROP | P3 | Commented out in source (SY leftover). |
| 24 | `sourcesTabCategoriesFilter()` | `sources_tab_categories_filter` | normal | Boolean | false | `Settings.sources.sources_tab_categories_filter` | KEEP | P1 |  |
| 25 | `sourcesTabSourcesInCategories()` | `sources_tab_source_categories` | normal | Set<String> | {} | `Settings.sources.sources_tab_sources_in_categories` | KEEP | P1 |  |
| 26 | `dataSaver()` | `data_saver` | normal | Enum DataSaver (name string) | NONE | `Settings.sources.data_saver` | KEEP | P1 |  |
| 27 | `dataSaverIgnoreJpeg()` | `ignore_jpeg` | normal | Boolean | false | `Settings.sources.data_saver_ignore_jpeg` | KEEP | P1 |  |
| 28 | `dataSaverIgnoreGif()` | `ignore_gif` | normal | Boolean | true | `Settings.sources.data_saver_ignore_gif` | KEEP | P1 |  |
| 29 | `dataSaverImageQuality()` | `data_saver_image_quality` | normal | Int | 80 | `Settings.sources.data_saver_image_quality` | KEEP | P1 |  |
| 30 | `dataSaverImageFormatJpeg()` | `data_saver_image_format_jpeg` | normal | Boolean | false | `Settings.sources.data_saver_image_format_jpeg` | KEEP | P1 |  |
| 31 | `dataSaverServer()` | `data_saver_server` | normal | String | "" | `Settings.sources.data_saver_server` | KEEP | P1 |  |
| 32 | `dataSaverColorBW()` | `data_saver_color_bw` | normal | Boolean | false | `Settings.sources.data_saver_color_bw` | KEEP | P1 |  |
| 33 | `dataSaverExcludedSources()` | `data_saver_excluded` | normal | Set<String> | {} | `Settings.sources.data_saver_excluded_sources` | KEEP | P1 |  |
| 34 | `dataSaverDownloader()` | `data_saver_downloader` | normal | Boolean | true | `Settings.sources.data_saver_downloader` | KEEP | P1 |  |

## TrackPreferences (`app/src/main/java/eu/kanade/domain/track/service/TrackPreferences.kt`), 9 keys, Rust section `tracking`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `trackUsername(tracker: Tracker)` | `__PRIVATE_pref_mangasync_username_${tracker.id}` | private | String | "" | `Secrets.tracking.track_username(tracker)` | REPLACE | P1 | Private; keyring. |
| 2 | `trackPassword(tracker: Tracker)` | `__PRIVATE_pref_mangasync_password_${tracker.id}` | private | String | "" | `Secrets.tracking.track_password(tracker)` | REPLACE | P1 | Private key kept for backup parsing; value stored in keyring (SecretStore), never in config.toml. |
| 3 | `trackAuthExpired(tracker: Tracker)` | `__PRIVATE_pref_tracker_auth_expired_${tracker.id}` | private | Boolean | false | `Secrets.tracking.track_auth_expired(tracker)` | KEEP | P1 | Private flag; stored in state.toml. |
| 4 | `trackToken(tracker: Tracker)` | `__PRIVATE_track_token_${tracker.id}` | private | String | "" | `Secrets.tracking.track_token(tracker)` | REPLACE | P1 | Private; keyring. |
| 5 | `anilistScoreType()` | `anilist_score_type` | normal | String | "POINT_10" | `Settings.tracking.anilist_score_type` | KEEP | P1 | POINT_100,POINT_10,POINT_10_DECIMAL,POINT_5,POINT_3. |
| 6 | `autoUpdateTrack()` | `pref_auto_update_manga_sync_key` | normal | Boolean | true | `Settings.tracking.auto_update_track` | KEEP | P1 |  |
| 7 | `trackOnAddingToLibrary()` | `track_on_adding_to_library` | normal | Boolean | true | `Settings.tracking.track_on_adding_to_library` | KEEP | P1 |  |
| 8 | `showNextEpisodeAiringTime()` | `show_next_episode_airing_time` | normal | Boolean | true | `Settings.tracking.show_next_episode_airing_time` | KEEP | P1 |  |
| 9 | `autoUpdateTrackOnMarkRead()` | `pref_auto_update_manga_on_mark_read` | normal | Enum AutoTrackState (name string) | ALWAYS | `Settings.tracking.auto_update_track_on_mark_read` | KEEP | P1 |  |

## LibraryPreferences (`domain/src/main/java/tachiyomi/domain/library/service/LibraryPreferences.kt`), 87 keys, Rust section `library`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `displayMode()` | `pref_display_mode_library` | normal | Object LibraryDisplayMode (string codec) | COMPACT_GRID | `Settings.library.display_mode` | KEEP | P0 |  |
| 2 | `mangaSortingMode()` | `library_sorting_mode` | normal | Object MangaLibrarySort (string codec) | "ALPHABETICAL,ASCENDING" | `Settings.library.manga_sorting_mode` | KEEP | P0 |  |
| 3 | `animeSortingMode()` | `animelib_sorting_mode` | normal | Object AnimeLibrarySort (string codec) | "ALPHABETICAL,ASCENDING" | `Settings.library.anime_sorting_mode` | KEEP | P0 |  |
| 4 | `lastUpdatedTimestamp()` | `__APP_STATE_library_update_last_timestamp` | app-state | Long | 0L | `State.library.last_updated_timestamp` | KEEP | P1 |  |
| 5 | `autoUpdateInterval()` | `pref_library_update_interval_key` | normal | Int | 0 | `Settings.library.auto_update_interval` | KEEP | P1 | Hours. UI choices 0 (never),12,24,48,72,168 (weekly). Migrations rewrite 1,2 -> 3 and a legacy quick value to 12. |
| 6 | `autoUpdateDeviceRestrictions()` | `library_update_restriction` | normal | Set<String> | setOf( DEVICE_ONLY_ON_WIFI, ) | `Settings.library.auto_update_device_restrictions` | KEEP | P1 |  |
| 7 | `autoUpdateItemRestrictions()` | `library_update_manga_restriction` | normal | Set<String> | {"manga_fully_read","manga_ongoing","manga_started","manga_outside_release_period"} (ENTRY_HAS_UNVIEWED, ENTRY_NON_COMPLETED, ENTRY_NON_VIEWED, ENTRY_OUTSIDE_RELEASE_PERIOD) | `Settings.library.auto_update_item_restrictions` | KEEP | P1 |  |
| 8 | `autoUpdateMetadata()` | `auto_update_metadata` | normal | Boolean | false | `Settings.library.auto_update_metadata` | KEEP | P1 |  |
| 9 | `showContinueViewingButton()` | `display_continue_reading_button` | normal | Boolean | false | `Settings.library.show_continue_viewing_button` | KEEP | P0 |  |
| 10 | `categoryTabs()` | `display_category_tabs` | normal | Boolean | true | `Settings.library.category_tabs` | KEEP | P0 |  |
| 11 | `categoryNumberOfItems()` | `display_number_of_items` | normal | Boolean | false | `Settings.library.category_number_of_items` | KEEP | P0 |  |
| 12 | `categorizedDisplaySettings()` | `categorized_display` | normal | Boolean | false | `Settings.library.categorized_display_settings` | KEEP | P0 |  |
| 13 | `hideHiddenCategoriesSettings()` | `hidden_categories` | normal | Boolean | false | `Settings.library.hide_hidden_categories_settings` | KEEP | P0 |  |
| 14 | `filterIntervalCustom()` | `pref_filter_library_interval_custom` | normal | Enum None (name string) | DISABLED | `Settings.library.filter_interval_custom` | KEEP | P0 |  |
| 15 | `downloadBadge()` | `display_download_badge` | normal | Boolean | false | `Settings.library.download_badge` | KEEP | P0 |  |
| 16 | `unreadBadge()` | `display_unread_badge` | normal | Boolean | true | `Settings.library.unread_badge` | KEEP | P0 |  |
| 17 | `localBadge()` | `display_local_badge` | normal | Boolean | true | `Settings.library.local_badge` | KEEP | P0 |  |
| 18 | `languageBadge()` | `display_language_badge` | normal | Boolean | false | `Settings.library.language_badge` | KEEP | P0 |  |
| 19 | `newShowUpdatesCount()` | `library_show_updates_count` | normal | Boolean | true | `Settings.library.new_show_updates_count` | KEEP | P1 |  |
| 20 | `autoClearItemCache()` | `auto_clear_chapter_cache` | normal | Boolean | false | `Settings.library.auto_clear_item_cache` | KEEP | P1 |  |
| 21 | `randomAnimeSortSeed()` | `library_random_anime_sort_seed` | normal | Int | 0 | `Settings.library.random_anime_sort_seed` | KEEP | P1 |  |
| 22 | `randomMangaSortSeed()` | `library_random_manga_sort_seed` | normal | Int | 0 | `Settings.library.random_manga_sort_seed` | KEEP | P1 |  |
| 23 | `animePortraitColumns()` | `pref_animelib_columns_portrait_key` | normal | Int | 0 | `Settings.library.anime_portrait_columns` | KEEP | P0 |  |
| 24 | `mangaPortraitColumns()` | `pref_library_columns_portrait_key` | normal | Int | 0 | `Settings.library.manga_portrait_columns` | KEEP | P0 |  |
| 25 | `animeLandscapeColumns()` | `pref_animelib_columns_landscape_key` | normal | Int | 0 | `Settings.library.anime_landscape_columns` | KEEP | P0 |  |
| 26 | `mangaLandscapeColumns()` | `pref_library_columns_landscape_key` | normal | Int | 0 | `Settings.library.manga_landscape_columns` | KEEP | P0 |  |
| 27 | `filterDownloadedAnime()` | `pref_filter_animelib_downloaded_v2` | normal | Enum None (name string) | DISABLED | `Settings.library.filter_downloaded_anime` | KEEP | P0 |  |
| 28 | `filterDownloadedManga()` | `pref_filter_library_downloaded_v2` | normal | Enum None (name string) | DISABLED | `Settings.library.filter_downloaded_manga` | KEEP | P0 |  |
| 29 | `filterUnseen()` | `pref_filter_animelib_unread_v2` | normal | Enum None (name string) | DISABLED | `Settings.library.filter_unseen` | KEEP | P0 | Anime "unseen" filter; key says unread (pref_filter_animelib_unread_v2). |
| 30 | `filterUnread()` | `pref_filter_library_unread_v2` | normal | Enum None (name string) | DISABLED | `Settings.library.filter_unread` | KEEP | P0 |  |
| 31 | `filterStartedAnime()` | `pref_filter_animelib_started_v2` | normal | Enum None (name string) | DISABLED | `Settings.library.filter_started_anime` | KEEP | P0 |  |
| 32 | `filterStartedManga()` | `pref_filter_library_started_v2` | normal | Enum None (name string) | DISABLED | `Settings.library.filter_started_manga` | KEEP | P0 |  |
| 33 | `filterBookmarkedAnime()` | `pref_filter_animelib_bookmarked_v2` | normal | Enum None (name string) | DISABLED | `Settings.library.filter_bookmarked_anime` | KEEP | P0 |  |
| 34 | `filterBookmarkedManga()` | `pref_filter_library_bookmarked_v2` | normal | Enum None (name string) | DISABLED | `Settings.library.filter_bookmarked_manga` | KEEP | P0 |  |
| 35 | `filterCompletedAnime()` | `pref_filter_animelib_completed_v2` | normal | Enum None (name string) | DISABLED | `Settings.library.filter_completed_anime` | KEEP | P0 |  |
| 36 | `filterCompletedManga()` | `pref_filter_library_completed_v2` | normal | Enum None (name string) | DISABLED | `Settings.library.filter_completed_manga` | KEEP | P0 |  |
| 37 | `filterTrackedAnime(id: Int)` | `pref_filter_animelib_tracked_${id}_v2` | normal | Enum None (name string) | DISABLED | `Settings.library.filter_tracked_anime(id)` | KEEP | P1 |  |
| 38 | `filterTrackedManga(id: Int)` | `pref_filter_library_tracked_${id}_v2` | normal | Enum None (name string) | DISABLED | `Settings.library.filter_tracked_manga(id)` | KEEP | P1 |  |
| 39 | `newMangaUpdatesCount()` | `library_unread_updates_count` | normal | Int | 0 | `Settings.library.new_manga_updates_count` | KEEP | P1 |  |
| 40 | `newAnimeUpdatesCount()` | `library_unseen_updates_count` | normal | Int | 0 | `Settings.library.new_anime_updates_count` | KEEP | P1 |  |
| 41 | `defaultAnimeCategory()` | `default_anime_category` | normal | Int | -1 | `Settings.library.default_anime_category` | KEEP | P0 | -1 = ask, 0 = default category, else category id. |
| 42 | `defaultMangaCategory()` | `default_category` | normal | Int | -1 | `Settings.library.default_manga_category` | KEEP | P0 | -1 = ask, 0 = default category, else category id. Remapped by category name on restore. |
| 43 | `lastUsedAnimeCategory()` | `__APP_STATE_last_used_anime_category` | app-state | Int | 0 | `State.library.last_used_anime_category` | KEEP | P0 | Category id of last viewed tab. |
| 44 | `lastUsedMangaCategory()` | `__APP_STATE_last_used_category` | app-state | Int | 0 | `State.library.last_used_manga_category` | KEEP | P0 | Category id of last viewed tab. |
| 45 | `animeUpdateCategories()` | `animelib_update_categories` | normal | Set<String> | {} | `Settings.library.anime_update_categories` | KEEP | P1 |  |
| 46 | `mangaUpdateCategories()` | `library_update_categories` | normal | Set<String> | {} | `Settings.library.manga_update_categories` | KEEP | P1 |  |
| 47 | `animeUpdateCategoriesExclude()` | `animelib_update_categories_exclude` | normal | Set<String> | {} | `Settings.library.anime_update_categories_exclude` | KEEP | P1 |  |
| 48 | `mangaUpdateCategoriesExclude()` | `library_update_categories_exclude` | normal | Set<String> | {} | `Settings.library.manga_update_categories_exclude` | KEEP | P1 |  |
| 49 | `filterEpisodeBySeen()` | `default_episode_filter_by_seen` | normal | Long | 0 | `Settings.library.filter_episode_by_seen` | KEEP | P0 |  |
| 50 | `filterChapterByRead()` | `default_chapter_filter_by_read` | normal | Long | 0 | `Settings.library.filter_chapter_by_read` | KEEP | P0 |  |
| 51 | `filterEpisodeByDownloaded()` | `default_episode_filter_by_downloaded` | normal | Long | 0 | `Settings.library.filter_episode_by_downloaded` | KEEP | P0 |  |
| 52 | `filterChapterByDownloaded()` | `default_chapter_filter_by_downloaded` | normal | Long | 0 | `Settings.library.filter_chapter_by_downloaded` | KEEP | P0 |  |
| 53 | `filterEpisodeByBookmarked()` | `default_episode_filter_by_bookmarked` | normal | Long | 0 | `Settings.library.filter_episode_by_bookmarked` | KEEP | P0 |  |
| 54 | `filterChapterByBookmarked()` | `default_chapter_filter_by_bookmarked` | normal | Long | 0 | `Settings.library.filter_chapter_by_bookmarked` | KEEP | P0 |  |
| 55 | `filterEpisodeByFillermarked()` | `default_episode_filter_by_fillermarked` | normal | Long | 0 | `Settings.library.filter_episode_by_fillermarked` | KEEP | P0 |  |
| 56 | `sortEpisodeBySourceOrNumber()` | `default_episode_sort_by_source_or_number` | normal | Long | 0 | `Settings.library.sort_episode_by_source_or_number` | KEEP | P0 |  |
| 57 | `sortChapterBySourceOrNumber()` | `default_chapter_sort_by_source_or_number` | normal | Long | 0 | `Settings.library.sort_chapter_by_source_or_number` | KEEP | P0 |  |
| 58 | `displayEpisodeByNameOrNumber()` | `default_chapter_display_by_name_or_number` | normal | Long | 0 | `Settings.library.display_episode_by_name_or_number` | KEEP | P0 | QUIRK: shares the key with displayChapterByNameOrNumber (episode default writes to the chapter key). Both enums use 0x0 name / 0x100000 number so values are compatible. |
| 59 | `displayChapterByNameOrNumber()` | `default_chapter_display_by_name_or_number` | normal | Long | 0 | `Settings.library.display_chapter_by_name_or_number` | KEEP | P0 | QUIRK: shared key (see displayEpisodeByNameOrNumber). |
| 60 | `sortEpisodeByAscendingOrDescending()` | `default_chapter_sort_by_ascending_or_descending` | normal | Long | 0 | `Settings.library.sort_episode_by_ascending_or_descending` | KEEP | P0 | QUIRK: shares key with sortChapterByAscendingOrDescending; DESC=0x0, ASC=0x1 in both. |
| 61 | `sortChapterByAscendingOrDescending()` | `default_chapter_sort_by_ascending_or_descending` | normal | Long | 0 | `Settings.library.sort_chapter_by_ascending_or_descending` | KEEP | P0 | QUIRK: shared key (see sortEpisodeByAscendingOrDescending). |
| 62 | `showEpisodeThumbnailPreviews()` | `default_episode_show_thumbnail_previews` | normal | Long | 0 | `Settings.library.show_episode_thumbnail_previews` | KEEP | P0 |  |
| 63 | `showEpisodeSummaries()` | `default_episode_show_summaries` | normal | Long | 0 | `Settings.library.show_episode_summaries` | KEEP | P0 |  |
| 64 | `filterSeasonByDownload()` | `default_season_filter_by_downloaded` | normal | Long | 0 | `Settings.library.filter_season_by_download` | KEEP | P2 | BUG in setSeasonSettingsDefault: written from seasonUnseenFilterRaw (wrong mask). |
| 65 | `filterSeasonByUnseen()` | `default_season_filter_by_unseen` | normal | Long | 0 | `Settings.library.filter_season_by_unseen` | KEEP | P2 |  |
| 66 | `filterSeasonByStarted()` | `default_season_filter_by_started` | normal | Long | 0 | `Settings.library.filter_season_by_started` | KEEP | P2 |  |
| 67 | `filterSeasonByCompleted()` | `default_season_filter_by_completed` | normal | Long | 0 | `Settings.library.filter_season_by_completed` | KEEP | P2 |  |
| 68 | `filterSeasonByBookmarked()` | `default_season_filter_by_bookmarked` | normal | Long | 0 | `Settings.library.filter_season_by_bookmarked` | KEEP | P2 |  |
| 69 | `filterSeasonByFillermarked()` | `default_season_filter_by_fillermarked` | normal | Long | 0 | `Settings.library.filter_season_by_fillermarked` | KEEP | P2 |  |
| 70 | `sortSeasonBySourceOrNumber()` | `default_season_sort_by_source_or_number` | normal | Long | 0 | `Settings.library.sort_season_by_source_or_number` | KEEP | P2 |  |
| 71 | `sortSeasonByAscendingOrDescending()` | `default_season_sort_by_ascending_or_descending` | normal | Long | 0 | `Settings.library.sort_season_by_ascending_or_descending` | KEEP | P2 |  |
| 72 | `seasonDisplayGridMode()` | `default_season_grid_display_mode` | normal | Long | 0 | `Settings.library.season_display_grid_mode` | KEEP | P2 |  |
| 73 | `seasonDisplayGridSize()` | `default_season_grid_display_size` | normal | Int | 0 | `Settings.library.season_display_grid_size` | KEEP | P2 | 0 = auto, UI range 0..10. |
| 74 | `seasonDownloadOverlay()` | `default_season_download_overlay` | normal | Boolean | false | `Settings.library.season_download_overlay` | KEEP | P2 |  |
| 75 | `seasonUnseenOverlay()` | `default_season_unseen_overlay` | normal | Boolean | true | `Settings.library.season_unseen_overlay` | KEEP | P2 |  |
| 76 | `seasonLocalOverlay()` | `default_season_local_overlay` | normal | Boolean | true | `Settings.library.season_local_overlay` | KEEP | P2 |  |
| 77 | `seasonLangOverlay()` | `default_season_lang_overlay` | normal | Boolean | false | `Settings.library.season_lang_overlay` | KEEP | P2 |  |
| 78 | `seasonContinueOverlay()` | `default_season_continue_overlay` | normal | Boolean | true | `Settings.library.season_continue_overlay` | KEEP | P2 |  |
| 79 | `seasonDisplayMode()` | `default_season_display_mode` | normal | Long | 0 | `Settings.library.season_display_mode` | KEEP | P2 |  |
| 80 | `updateSeasonOnRefresh()` | `pref_update_season_on_refresh` | normal | Boolean | false | `Settings.library.update_season_on_refresh` | KEEP | P2 |  |
| 81 | `updateSeasonOnLibraryUpdate()` | `pref_update_season_on_library_update` | normal | Boolean | false | `Settings.library.update_season_on_library_update` | KEEP | P2 |  |
| 82 | `swipeEpisodeStartAction()` | `pref_episode_swipe_end_action` | normal | Enum EpisodeSwipeAction (name string) | ToggleSeen | `Settings.library.swipe_episode_start_action` | KEEP | P1 | QUIRK: Kotlin function "Start" reads key pref_episode_swipe_end_action (names swapped vs keys). Keep key strings exactly. |
| 83 | `swipeEpisodeEndAction()` | `pref_episode_swipe_start_action` | normal | Enum EpisodeSwipeAction (name string) | ToggleBookmark | `Settings.library.swipe_episode_end_action` | KEEP | P1 | QUIRK: reads key pref_episode_swipe_start_action. |
| 84 | `swipeChapterStartAction()` | `pref_chapter_swipe_end_action` | normal | Enum ChapterSwipeAction (name string) | ToggleRead | `Settings.library.swipe_chapter_start_action` | KEEP | P1 | QUIRK: Kotlin "Start" reads key pref_chapter_swipe_end_action. |
| 85 | `swipeChapterEndAction()` | `pref_chapter_swipe_start_action` | normal | Enum ChapterSwipeAction (name string) | ToggleBookmark | `Settings.library.swipe_chapter_end_action` | KEEP | P1 | QUIRK: reads key pref_chapter_swipe_start_action. |
| 86 | `markDuplicateReadChapterAsRead()` | `mark_duplicate_read_chapter_read` | normal | Set<String> | {} | `Settings.library.mark_duplicate_read_chapter_as_read` | KEEP | P2 |  |
| 87 | `markDuplicateSeenEpisodeAsSeen()` | `mark_duplicate_seen_episode_seen` | normal | Set<String> | {} | `Settings.library.mark_duplicate_seen_episode_as_seen` | KEEP | P2 |  |

## DownloadPreferences (`domain/src/main/java/tachiyomi/domain/download/service/DownloadPreferences.kt`), 23 keys, Rust section `downloads`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `downloadOnlyOverWifi()` | `pref_download_only_over_wifi_key` | normal | Boolean | true | `Settings.downloads.download_only_over_wifi` | KEEP-CHANGED | P1 | "Wi-Fi" becomes "not a metered connection" via NetworkManager/GIO NetworkMonitor. |
| 2 | `useExternalDownloader()` | `use_external_downloader` | normal | Boolean | false | `Settings.downloads.use_external_downloader` | REPLACE | P3 | Android external downloader apps (ADM, 1DM) replaced by an optional command line (e.g. aria2c); key kept. |
| 3 | `externalDownloaderSelection()` | `external_downloader_selection` | normal | String | "" | `Settings.downloads.external_downloader_selection` | REPLACE | P3 | Package name on Android; command string on Linux. |
| 4 | `saveChaptersAsCBZ()` | `save_chapter_as_cbz` | normal | Boolean | true | `Settings.downloads.save_chapters_as_cbz` | KEEP | P1 |  |
| 5 | `splitTallImages()` | `split_tall_images` | normal | Boolean | true | `Settings.downloads.split_tall_images` | KEEP | P1 |  |
| 6 | `autoDownloadWhileReading()` | `auto_download_while_reading` | normal | Int | 0 | `Settings.downloads.auto_download_while_reading` | KEEP | P1 | UI choices 0 (disabled),2,3,5,10 next unread chapters. |
| 7 | `autoDownloadWhileWatching()` | `auto_download_while_watching` | normal | Int | 0 | `Settings.downloads.auto_download_while_watching` | KEEP | P1 | UI choices 0 (disabled),2,3,5,10 next unseen episodes. |
| 8 | `removeAfterReadSlots()` | `remove_after_read_slots` | normal | Int | -1 | `Settings.downloads.remove_after_read_slots` | KEEP | P1 | -1 disabled; 0 last read; 1..4 second..fifth to last (delete chapterList[currentPosition - slots], ReaderViewModel.kt:514). |
| 9 | `removeAfterMarkedAsRead()` | `pref_remove_after_marked_as_read_key` | normal | Boolean | false | `Settings.downloads.remove_after_marked_as_read` | KEEP | P1 |  |
| 10 | `removeBookmarkedChapters()` | `pref_remove_bookmarked` | normal | Boolean | false | `Settings.downloads.remove_bookmarked_chapters` | KEEP | P1 |  |
| 11 | `downloadFillermarkedItems()` | `pref_download_fillermarked` | normal | Boolean | false | `Settings.downloads.download_fillermarked_items` | KEEP | P1 |  |
| 12 | `removeExcludeCategories()` | `remove_exclude_categories` | normal | Set<String> | {} | `Settings.downloads.remove_exclude_categories` | KEEP | P1 |  |
| 13 | `removeExcludeAnimeCategories()` | `remove_exclude_anime_categories` | normal | Set<String> | {} | `Settings.downloads.remove_exclude_anime_categories` | KEEP | P1 |  |
| 14 | `downloadNewChapters()` | `download_new` | normal | Boolean | false | `Settings.downloads.download_new_chapters` | KEEP | P1 |  |
| 15 | `downloadNewEpisodes()` | `download_new_episode` | normal | Boolean | false | `Settings.downloads.download_new_episodes` | KEEP | P1 |  |
| 16 | `downloadNewChapterCategories()` | `download_new_categories` | normal | Set<String> | {} | `Settings.downloads.download_new_chapter_categories` | KEEP | P1 |  |
| 17 | `downloadNewEpisodeCategories()` | `download_new_anime_categories` | normal | Set<String> | {} | `Settings.downloads.download_new_episode_categories` | KEEP | P1 |  |
| 18 | `downloadNewChapterCategoriesExclude()` | `download_new_categories_exclude` | normal | Set<String> | {} | `Settings.downloads.download_new_chapter_categories_exclude` | KEEP | P1 |  |
| 19 | `downloadNewEpisodeCategoriesExclude()` | `download_new_anime_categories_exclude` | normal | Set<String> | {} | `Settings.downloads.download_new_episode_categories_exclude` | KEEP | P1 |  |
| 20 | `numberOfDownloads()` | `download_slots` | normal | Int | 3 | `Settings.downloads.number_of_downloads` | KEEP | P1 | Parallel downloads (UI 1..5). |
| 21 | `downloadSpeedLimit()` | `download_speed_limit` | normal | Int | 0 | `Settings.downloads.download_speed_limit` | KEEP | P1 | KiB/s (MangaDownloader.kt:525 multiplies by 1024); 0 = off. |
| 22 | `downloadNewUnreadChaptersOnly()` | `download_new_unread_chapters_only` | normal | Boolean | false | `Settings.downloads.download_new_unread_chapters_only` | KEEP | P1 |  |
| 23 | `downloadNewUnseenEpisodesOnly()` | `download_new_unread_episodes_only` | normal | Boolean | false | `Settings.downloads.download_new_unseen_episodes_only` | KEEP | P1 |  |

## BackupPreferences (`domain/src/main/java/tachiyomi/domain/backup/service/BackupPreferences.kt`), 3 keys, Rust section `backup`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `backupInterval()` | `backup_interval` | normal | Int | 12 | `Settings.backup.backup_interval` | KEEP | P1 | Hours. UI choices 0 (off),6,12,24,48,168. Migration EnableAutoBackupMigration sets 0 -> 12. |
| 2 | `lastAutoBackupTimestamp()` | `__APP_STATE_last_auto_backup_timestamp` | app-state | Long | 0L | `State.backup.last_auto_backup_timestamp` | KEEP | P1 |  |
| 3 | `backupFlags()` | `backup_flags` | normal | Set<String> | {"1","2","4","8"} (FLAG_CATEGORIES, FLAG_CHAPTERS, FLAG_HISTORY, FLAG_TRACK) | `Settings.backup.backup_flags` | DROP | P3 | Never read in Aniyomi (dead); backup options use BackupOptions instead. |

## StoragePreferences (`domain/src/main/java/tachiyomi/domain/storage/service/StoragePreferences.kt`), 1 keys, Rust section `storage`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `baseStorageDirectory()` | `__APP_STATE_storage_dir` | app-state | String | file URI of <external storage>/Aniyomi (Android) | `State.storage.base_storage_directory` | KEEP-CHANGED | P0 | App-state. Linux default is the platform data directory (Paths trait), stored as a plain path not a content URI. |

## SecurityPreferences (`core/common/src/main/java/eu/kanade/tachiyomi/core/security/SecurityPreferences.kt`), 5 keys, Rust section `security`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `useAuthenticator()` | `use_biometric_lock` | normal | Boolean | false | `Settings.security.use_authenticator` | REPLACE | P2 | Android biometric prompt replaced by an app-lock passphrase dialog; key kept. |
| 2 | `lockAppAfter()` | `lock_app_after` | normal | Int | 0 | `Settings.security.lock_app_after` | KEEP | P2 | Minutes: 0 always, 1, 2, 5, 10, -1 never. Minutes. 0 always, 1, 2, 5, 10, -1 never. |
| 3 | `secureScreen()` | `secure_screen_v2` | normal | Enum SecureScreenMode (name string) | INCOGNITO | `Settings.security.secure_screen` | DROP | P3 | FLAG_SECURE has no Wayland/X11 equivalent for GTK apps. Loss: screenshots of the app are not blocked. |
| 4 | `hideNotificationContent()` | `hide_notification_content` | normal | Boolean | false | `Settings.security.hide_notification_content` | KEEP-CHANGED | P2 | Applies to desktop notifications via Notifier trait. |
| 5 | `lastAppClosed()` | `__APP_STATE_last_app_closed` | app-state | Long | 0 | `State.security.last_app_closed` | KEEP | P2 | App-state. |

## NetworkPreferences (`core/common/src/main/java/eu/kanade/tachiyomi/network/NetworkPreferences.kt`), 3 keys, Rust section `network`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `verboseLogging()` | `verbose_logging` | normal | Boolean | false (constructor arg; true only in debug builds) | `Settings.network.verbose_logging` | KEEP-CHANGED | P2 | Maps to tracing level debug/trace for HTTP. |
| 2 | `dohProvider()` | `doh_provider` | normal | Int | -1 | `Settings.network.doh_provider` | KEEP | P2 | Values -1 none, 1..13 providers (see data-and-config). -1 off; 1 Cloudflare,2 Google,3 AdGuard,4 Quad9,5 AliDNS,6 DNSPod,7 360,8 Quad101,9 Mullvad,10 ControlD,11 Njalla,12 Shecan,13 LibreDNS. |
| 3 | `defaultUserAgent()` | `default_user_agent` | normal | String | "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:136.0) Gecko/20100101 Firefox/136.0" | `Settings.network.default_user_agent` | KEEP | P0 | Sent by the shared HTTP client. |

## TorrentPreferences (`core/common/src/main/java/aniyomi/core/common/torrent/TorrentPreferences.kt`), 6 keys, Rust section `torrent`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `torrServerEnable()` | `pref_torrserver_enable` | normal | Boolean | false | `Settings.torrent.torr_server_enable` | KEEP-CHANGED | P2 | TorrServer runs as a spawned child process. |
| 2 | `torrServerShownNotice()` | `pref_torrserver_shownotice` | normal | Boolean | false | `Settings.torrent.torr_server_shown_notice` | KEEP | P2 |  |
| 3 | `torrServerPort()` | `pref_torrserver_port` | normal | String | "8090" | `Settings.torrent.torr_server_port` | KEEP | P2 | String, default "8090". |
| 4 | `torrServerTrackers()` | `pref_torrserver_tackers` | normal | String | 26 tracker URLs joined by "\n" (spaces removed) | `Settings.torrent.torr_server_trackers` | KEEP | P2 | Newline-separated announce URLs. |
| 5 | `torrServerProxyMode()` | `pref_torrserver_proxymode` | normal | Enum ProxyMode (name string) | None | `Settings.torrent.torr_server_proxy_mode` | KEEP | P2 | QUIRK: ProxyMode.None.value == "tracker" (same as Tracker). |
| 6 | `torrServerProxyUrl()` | `pref_torrserver_proxyurl` | normal | String | "" | `Settings.torrent.torr_server_proxy_url` | KEEP | P2 |  |

## ReaderPreferences (`app/src/main/java/eu/kanade/tachiyomi/ui/reader/setting/ReaderPreferences.kt`), 54 keys, Rust section `reader`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `pageTransitions()` | `pref_enable_transitions_key` | normal | Boolean | true | `Settings.reader.page_transitions` | KEEP | P0 |  |
| 2 | `flashOnPageChange()` | `pref_reader_flash` | normal | Boolean | false | `Settings.reader.flash_on_page_change` | KEEP | P2 |  |
| 3 | `flashDurationMillis()` | `pref_reader_flash_duration` | normal | Int | 100 | `Settings.reader.flash_duration_millis` | KEEP | P2 | Milliseconds, UI step 100; default 100. |
| 4 | `flashPageInterval()` | `pref_reader_flash_interval` | normal | Int | 1 | `Settings.reader.flash_page_interval` | KEEP | P2 |  |
| 5 | `flashColor()` | `pref_reader_flash_mode` | normal | Enum FlashColor (name string) | BLACK | `Settings.reader.flash_color` | KEEP | P2 |  |
| 6 | `doubleTapAnimSpeed()` | `pref_double_tap_anim_speed` | normal | Int | 500 | `Settings.reader.double_tap_anim_speed` | KEEP | P2 |  |
| 7 | `showPageNumber()` | `pref_show_page_number_key` | normal | Boolean | true | `Settings.reader.show_page_number` | KEEP | P0 |  |
| 8 | `showReadingMode()` | `pref_show_reading_mode` | normal | Boolean | true | `Settings.reader.show_reading_mode` | KEEP | P0 |  |
| 9 | `fullscreen()` | `fullscreen` | normal | Boolean | true | `Settings.reader.fullscreen` | KEEP-CHANGED | P1 | Window fullscreen while reading. |
| 10 | `cutoutShort()` | `cutout_short` | normal | Boolean | true | `Settings.reader.cutout_short` | DROP | P3 | Display cutout; none on desktop. |
| 11 | `keepScreenOn()` | `pref_keep_screen_on_key` | normal | Boolean | true | `Settings.reader.keep_screen_on` | REPLACE | P1 | Screen wake lock replaced by IdleInhibitor trait. |
| 12 | `defaultReadingMode()` | `pref_default_reading_mode_key` | normal | Int | 2 | `Settings.reader.default_reading_mode` | KEEP | P0 | ReadingMode.flagValue: 0 DEFAULT,1 LTR,2 RTL,3 VERTICAL,4 WEBTOON,5 CONTINUOUS_VERTICAL. |
| 13 | `defaultOrientationType()` | `pref_default_orientation_type_key` | normal | Int | 8 (0x08) | `Settings.reader.default_orientation_type` | DROP | P3 | Screen orientation; stored for backups. ReaderOrientation.flagValue: 0 DEFAULT,0x08 FREE,0x10 PORTRAIT,0x18 LANDSCAPE,0x20 LOCKED_PORTRAIT,0x28 LOCKED_LANDSCAPE,0x30 REVERSE_PORTRAIT. |
| 14 | `webtoonDoubleTapZoomEnabled()` | `pref_enable_double_tap_zoom_webtoon` | normal | Boolean | true | `Settings.reader.webtoon_double_tap_zoom_enabled` | KEEP | P0 |  |
| 15 | `imageScaleType()` | `pref_image_scale_type_key` | normal | Int | 1 | `Settings.reader.image_scale_type` | KEEP | P0 | UI stores index+1: 1 fit screen,2 stretch,3 fit width,4 fit height,5 original size,6 smart fit. |
| 16 | `zoomStart()` | `pref_zoom_start_key` | normal | Int | 1 | `Settings.reader.zoom_start` | KEEP | P0 | UI stores index+1: 1 automatic,2 left,3 right,4 center. |
| 17 | `readerTheme()` | `pref_reader_theme_key` | normal | Int | 1 | `Settings.reader.reader_theme` | KEEP | P0 | 0 white,1 black,2 gray,3 automatic. |
| 18 | `alwaysShowChapterTransition()` | `always_show_chapter_transition` | normal | Boolean | true | `Settings.reader.always_show_chapter_transition` | KEEP | P0 |  |
| 19 | `preserveReadingPosition()` | `pref_preserve_reading_position` | normal | Boolean | false | `Settings.reader.preserve_reading_position` | KEEP | P0 |  |
| 20 | `cropBorders()` | `crop_borders` | normal | Boolean | false | `Settings.reader.crop_borders` | KEEP | P0 |  |
| 21 | `navigateToPan()` | `navigate_pan` | normal | Boolean | true | `Settings.reader.navigate_to_pan` | KEEP | P0 |  |
| 22 | `landscapeZoom()` | `landscape_zoom` | normal | Boolean | true | `Settings.reader.landscape_zoom` | KEEP | P0 |  |
| 23 | `cropBordersWebtoon()` | `crop_borders_webtoon` | normal | Boolean | false | `Settings.reader.crop_borders_webtoon` | KEEP | P0 |  |
| 24 | `webtoonSidePadding()` | `webtoon_side_padding` | normal | Int | 0 | `Settings.reader.webtoon_side_padding` | KEEP | P0 | 0..25 (percent per side). |
| 25 | `readerHideThreshold()` | `reader_hide_threshold` | normal | Enum ReaderHideThreshold (name string) | LOW | `Settings.reader.reader_hide_threshold` | KEEP | P0 |  |
| 26 | `folderPerManga()` | `create_folder_per_manga` | normal | Boolean | false | `Settings.reader.folder_per_manga` | KEEP | P2 |  |
| 27 | `skipRead()` | `skip_read` | normal | Boolean | false | `Settings.reader.skip_read` | KEEP | P1 |  |
| 28 | `skipFiltered()` | `skip_filtered` | normal | Boolean | true | `Settings.reader.skip_filtered` | KEEP | P1 |  |
| 29 | `skipDupe()` | `skip_dupe` | normal | Boolean | false | `Settings.reader.skip_dupe` | KEEP | P1 |  |
| 30 | `webtoonDisableZoomOut()` | `webtoon_disable_zoom_out` | normal | Boolean | false | `Settings.reader.webtoon_disable_zoom_out` | KEEP | P2 |  |
| 31 | `dualPageSplitPaged()` | `pref_dual_page_split` | normal | Boolean | false | `Settings.reader.dual_page_split_paged` | KEEP | P2 |  |
| 32 | `dualPageInvertPaged()` | `pref_dual_page_invert` | normal | Boolean | false | `Settings.reader.dual_page_invert_paged` | KEEP | P2 |  |
| 33 | `dualPageSplitWebtoon()` | `pref_dual_page_split_webtoon` | normal | Boolean | false | `Settings.reader.dual_page_split_webtoon` | KEEP | P2 |  |
| 34 | `dualPageInvertWebtoon()` | `pref_dual_page_invert_webtoon` | normal | Boolean | false | `Settings.reader.dual_page_invert_webtoon` | KEEP | P2 |  |
| 35 | `dualPageRotateToFit()` | `pref_dual_page_rotate` | normal | Boolean | false | `Settings.reader.dual_page_rotate_to_fit` | KEEP | P2 |  |
| 36 | `dualPageRotateToFitInvert()` | `pref_dual_page_rotate_invert` | normal | Boolean | false | `Settings.reader.dual_page_rotate_to_fit_invert` | KEEP | P2 |  |
| 37 | `dualPageRotateToFitWebtoon()` | `pref_dual_page_rotate_webtoon` | normal | Boolean | false | `Settings.reader.dual_page_rotate_to_fit_webtoon` | KEEP | P2 |  |
| 38 | `dualPageRotateToFitInvertWebtoon()` | `pref_dual_page_rotate_invert_webtoon` | normal | Boolean | false | `Settings.reader.dual_page_rotate_to_fit_invert_webtoon` | KEEP | P2 |  |
| 39 | `customBrightness()` | `pref_custom_brightness_key` | normal | Boolean | false | `Settings.reader.custom_brightness` | KEEP | P1 |  |
| 40 | `customBrightnessValue()` | `custom_brightness_value` | normal | Int | 0 | `Settings.reader.custom_brightness_value` | KEEP | P1 | Range -75..100. -75..-1 dark overlay (black, alpha = value), 0 = system brightness / no overlay, 1..100 set brightness. |
| 41 | `colorFilter()` | `pref_color_filter_key` | normal | Boolean | false | `Settings.reader.color_filter` | KEEP | P1 |  |
| 42 | `colorFilterValue()` | `color_filter_value` | normal | Int | 0 | `Settings.reader.color_filter_value` | KEEP | P1 | ARGB int (alpha<<24 | r<<16 | g<<8 | b); UI sliders R,G,B,A each 0..255. |
| 43 | `colorFilterMode()` | `color_filter_mode` | normal | Int | 0 | `Settings.reader.color_filter_mode` | KEEP | P1 | Index into ColorFilterMode: 0 SrcOver(default),1 Modulate(multiply),2 Screen,3 Overlay,4 Lighten,5 Darken. |
| 44 | `grayscale()` | `pref_grayscale` | normal | Boolean | false | `Settings.reader.grayscale` | KEEP | P1 |  |
| 45 | `invertedColors()` | `pref_inverted_colors` | normal | Boolean | false | `Settings.reader.inverted_colors` | KEEP | P1 |  |
| 46 | `readWithLongTap()` | `reader_long_tap` | normal | Boolean | true | `Settings.reader.read_with_long_tap` | KEEP-CHANGED | P2 | Right-click / long press opens page actions. |
| 47 | `readWithVolumeKeys()` | `reader_volume_keys` | normal | Boolean | false | `Settings.reader.read_with_volume_keys` | DROP | P3 | Hardware volume keys; keyboard/mouse bindings replace them. |
| 48 | `readWithVolumeKeysInverted()` | `reader_volume_keys_inverted` | normal | Boolean | false | `Settings.reader.read_with_volume_keys_inverted` | DROP | P3 | Same. |
| 49 | `navigationModePager()` | `reader_navigation_mode_pager` | normal | Int | 0 | `Settings.reader.navigation_mode_pager` | KEEP | P0 | Index of TapZones: 0 default,1 L,2 kindlish,3 edge,4 right-and-left,5 disabled. |
| 50 | `navigationModeWebtoon()` | `reader_navigation_mode_webtoon` | normal | Int | 0 | `Settings.reader.navigation_mode_webtoon` | KEEP | P0 | Same index list as pager. |
| 51 | `pagerNavInverted()` | `reader_tapping_inverted` | normal | Enum TappingInvertMode (name string) | NONE | `Settings.reader.pager_nav_inverted` | KEEP | P0 |  |
| 52 | `webtoonNavInverted()` | `reader_tapping_inverted_webtoon` | normal | Enum TappingInvertMode (name string) | NONE | `Settings.reader.webtoon_nav_inverted` | KEEP | P0 |  |
| 53 | `showNavigationOverlayNewUser()` | `reader_navigation_overlay_new_user` | normal | Boolean | true | `Settings.reader.show_navigation_overlay_new_user` | KEEP | P2 | App-style first-run hint. |
| 54 | `showNavigationOverlayOnStart()` | `reader_navigation_overlay_on_start` | normal | Boolean | false | `Settings.reader.show_navigation_overlay_on_start` | KEEP | P0 |  |

## PlayerPreferences (`app/src/main/java/eu/kanade/tachiyomi/ui/player/settings/PlayerPreferences.kt`), 36 keys, Rust section `player`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `preserveWatchingPosition()` | `pref_preserve_watching_position` | normal | Boolean | false | `Settings.player.preserve_watching_position` | KEEP | P0 |  |
| 2 | `progressPreference()` | `pref_progress_preference` | normal | Float | 0.85 | `Settings.player.progress_preference` | KEEP | P0 | Fraction of duration at which an episode is marked seen. 0.85 = mark seen at 85%. |
| 3 | `defaultPlayerOrientationType()` | `pref_default_player_orientation_type_key` | normal | Enum PlayerOrientation (name string) | SensorLandscape | `Settings.player.default_player_orientation_type` | DROP | P3 | Screen orientation has no desktop meaning; stored for backups. |
| 4 | `allowGestures()` | `pref_allow_gestures_in_panels` | normal | Boolean | false | `Settings.player.allow_gestures` | KEEP | P2 |  |
| 5 | `showLoadingCircle()` | `pref_show_loading` | normal | Boolean | true | `Settings.player.show_loading_circle` | KEEP | P0 |  |
| 6 | `showCurrentChapter()` | `pref_show_current_chapter` | normal | Boolean | true | `Settings.player.show_current_chapter` | KEEP | P0 |  |
| 7 | `rememberPlayerBrightness()` | `pref_remember_brightness` | normal | Boolean | false | `Settings.player.remember_player_brightness` | KEEP-CHANGED | P2 | Brightness maps to the mpv `brightness` video property, not screen backlight. |
| 8 | `playerBrightnessValue()` | `player_brightness_value` | normal | Float | -1.0 | `Settings.player.player_brightness_value` | KEEP-CHANGED | P2 | Same. |
| 9 | `rememberPlayerVolume()` | `pref_remember_volume` | normal | Boolean | false | `Settings.player.remember_player_volume` | KEEP | P0 |  |
| 10 | `playerVolumeValue()` | `player_volume_value` | normal | Float | -1.0 | `Settings.player.player_volume_value` | KEEP | P0 |  |
| 11 | `showFailedHosters()` | `pref_show_failed_hosters` | normal | Boolean | false | `Settings.player.show_failed_hosters` | KEEP | P2 |  |
| 12 | `showEmptyHosters()` | `pref_show_empty_hosters` | normal | Boolean | false | `Settings.player.show_empty_hosters` | KEEP | P2 |  |
| 13 | `playerFullscreen()` | `player_fullscreen` | normal | Boolean | true | `Settings.player.player_fullscreen` | KEEP | P1 | Window fullscreen on open. |
| 14 | `hideControls()` | `player_hide_controls` | normal | Boolean | false | `Settings.player.hide_controls` | KEEP | P0 |  |
| 15 | `displayVolPer()` | `pref_display_vol_as_per` | normal | Boolean | true | `Settings.player.display_vol_per` | KEEP | P2 |  |
| 16 | `showSystemStatusBar()` | `pref_show_system_status_bar` | normal | Boolean | false | `Settings.player.show_system_status_bar` | DROP | P3 | Android status bar visibility; desktop has no equivalent. |
| 17 | `reduceMotion()` | `pref_reduce_motion` | normal | Boolean | false | `Settings.player.reduce_motion` | KEEP | P2 |  |
| 18 | `playerTimeToDisappear()` | `pref_player_time_to_disappear` | normal | Int | 4000 | `Settings.player.player_time_to_disappear` | KEEP | P1 | Milliseconds. |
| 19 | `panelOpacity()` | `pref_panel_opacity` | normal | Int | 60 | `Settings.player.panel_opacity` | KEEP | P2 | Percent 0..100. |
| 20 | `enableSkipIntro()` | `pref_enable_skip_intro` | normal | Boolean | true | `Settings.player.enable_skip_intro` | KEEP | P0 |  |
| 21 | `autoSkipIntro()` | `pref_enable_auto_skip_ani_skip` | normal | Boolean | false | `Settings.player.auto_skip_intro` | KEEP | P0 |  |
| 22 | `enableNetflixStyleIntroSkip()` | `pref_enable_netflixStyle_aniskip` | normal | Boolean | false | `Settings.player.enable_netflix_style_intro_skip` | KEEP | P2 |  |
| 23 | `waitingTimeIntroSkip()` | `pref_waiting_time_aniskip` | normal | Int | 5 | `Settings.player.waiting_time_intro_skip` | KEEP | P2 |  |
| 24 | `aniSkipEnabled()` | `pref_enable_ani_skip` | normal | Boolean | false | `Settings.player.ani_skip_enabled` | KEEP | P0 |  |
| 25 | `disableAniSkipOnChapters()` | `pref_disabled_ani_skip_chapters` | normal | Boolean | true | `Settings.player.disable_ani_skip_on_chapters` | KEEP | P0 |  |
| 26 | `enablePip()` | `pref_enable_pip` | normal | Boolean | true | `Settings.player.enable_pip` | REPLACE | P3 | Android PiP; replaced by always-on-top mini player or dropped; key kept. |
| 27 | `pipEpisodeToasts()` | `pref_pip_episode_toasts` | normal | Boolean | true | `Settings.player.pip_episode_toasts` | DROP | P3 | PiP only. |
| 28 | `pipOnExit()` | `pref_pip_on_exit` | normal | Boolean | false | `Settings.player.pip_on_exit` | DROP | P3 | PiP only. |
| 29 | `pipReplaceWithPrevious()` | `pip_replace_with_previous` | normal | Boolean | false | `Settings.player.pip_replace_with_previous` | DROP | P3 | PiP only. |
| 30 | `alwaysUseExternalPlayer()` | `pref_always_use_external_player` | normal | Boolean | false | `Settings.player.always_use_external_player` | KEEP | P1 |  |
| 31 | `externalPlayerPreference()` | `external_player_preference` | normal | String | "" | `Settings.player.external_player_preference` | REPLACE | P1 | Android package name; on Linux a command template (mpv, vlc) with $URL. |
| 32 | `playerSpeed()` | `pref_player_speed` | normal | Float | 1.0 | `Settings.player.player_speed` | KEEP | P0 |  |
| 33 | `speedPresets()` | `default_speed_presets` | normal | Set<String> | {"0.25","0.5","0.75","1.0","1.25","1.5","1.75","2.0","2.5","3.0","3.5","4.0"} | `Settings.player.speed_presets` | KEEP | P2 |  |
| 34 | `invertDuration()` | `invert_duration` | normal | Boolean | false | `Settings.player.invert_duration` | KEEP | P2 |  |
| 35 | `aspectState()` | `pref_player_aspect_state` | normal | Enum VideoAspect (name string) | Fit | `Settings.player.aspect_state` | KEEP | P0 |  |
| 36 | `autoplayEnabled()` | `pref_auto_play_enabled` | normal | Boolean | false | `Settings.player.autoplay_enabled` | KEEP | P1 | "Old" key still present; autoplay next episode. |

## GesturePreferences (`app/src/main/java/eu/kanade/tachiyomi/ui/player/settings/GesturePreferences.kt`), 13 keys, Rust section `player.gestures`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `gestureVolumeBrightness()` | `pref_gesture_volume_brightness` | normal | Boolean | true | `Settings.player.gestures.gesture_volume_brightness` | KEEP | P1 |  |
| 2 | `swapVolumeBrightness()` | `pref_swap_volume_and_brightness` | normal | Boolean | false | `Settings.player.gestures.swap_volume_brightness` | KEEP | P1 |  |
| 3 | `gestureHorizontalSeek()` | `pref_gesture_horizontal_seek` | normal | Boolean | true | `Settings.player.gestures.gesture_horizontal_seek` | KEEP | P1 |  |
| 4 | `showSeekBar()` | `pref_show_seekbar` | normal | Boolean | false | `Settings.player.gestures.show_seek_bar` | KEEP | P1 |  |
| 5 | `defaultIntroLength()` | `pref_default_intro_length` | normal | Int | 85 | `Settings.player.gestures.default_intro_length` | KEEP | P1 | Seconds (manual skip-intro length). |
| 6 | `skipLengthPreference()` | `pref_skip_length_preference` | normal | Int | 10 | `Settings.player.gestures.skip_length_preference` | KEEP | P1 | Seconds for seek gesture / buttons. |
| 7 | `playerSmoothSeek()` | `pref_player_smooth_seek` | normal | Boolean | false | `Settings.player.gestures.player_smooth_seek` | KEEP | P1 |  |
| 8 | `leftDoubleTapGesture()` | `pref_left_double_tap` | normal | Enum SingleActionGesture (name string) | Seek | `Settings.player.gestures.left_double_tap_gesture` | KEEP | P1 |  |
| 9 | `centerDoubleTapGesture()` | `pref_center_double_tap` | normal | Enum SingleActionGesture (name string) | PlayPause | `Settings.player.gestures.center_double_tap_gesture` | KEEP | P1 |  |
| 10 | `rightDoubleTapGesture()` | `pref_right_double_tap` | normal | Enum SingleActionGesture (name string) | Seek | `Settings.player.gestures.right_double_tap_gesture` | KEEP | P1 |  |
| 11 | `mediaPreviousGesture()` | `pref_media_previous` | normal | Enum SingleActionGesture (name string) | Switch | `Settings.player.gestures.media_previous_gesture` | KEEP | P1 |  |
| 12 | `mediaPlayPauseGesture()` | `pref_media_playpause` | normal | Enum SingleActionGesture (name string) | PlayPause | `Settings.player.gestures.media_play_pause_gesture` | KEEP | P1 |  |
| 13 | `mediaNextGesture()` | `pref_media_next` | normal | Enum SingleActionGesture (name string) | Switch | `Settings.player.gestures.media_next_gesture` | KEEP | P1 |  |

## DecoderPreferences (`app/src/main/java/eu/kanade/tachiyomi/ui/player/settings/DecoderPreferences.kt`), 9 keys, Rust section `player.decoder`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `tryHWDecoding()` | `pref_try_hwdec` | normal | Boolean | true | `Settings.player.decoder.try_hw_decoding` | KEEP | P0 |  |
| 2 | `gpuNext()` | `pref_gpu_next` | normal | Boolean | false | `Settings.player.decoder.gpu_next` | KEEP | P0 |  |
| 3 | `videoDebanding()` | `pref_video_debanding` | normal | Enum Debanding (name string) | None | `Settings.player.decoder.video_debanding` | KEEP | P0 |  |
| 4 | `useYUV420P()` | `use_yuv420p` | normal | Boolean | true | `Settings.player.decoder.use_yuv420p` | KEEP | P0 |  |
| 5 | `brightnessFilter()` | `pref_player_filter_brightness` | normal | Int | 0 | `Settings.player.decoder.brightness_filter` | KEEP | P0 |  |
| 6 | `saturationFilter()` | `pref_player_filter_saturation` | normal | Int | 0 | `Settings.player.decoder.saturation_filter` | KEEP | P0 |  |
| 7 | `contrastFilter()` | `pref_player_filter_contrast` | normal | Int | 0 | `Settings.player.decoder.contrast_filter` | KEEP | P0 |  |
| 8 | `gammaFilter()` | `pref_player_filter_gamma` | normal | Int | 0 | `Settings.player.decoder.gamma_filter` | KEEP | P0 |  |
| 9 | `hueFilter()` | `pref_player_filter_hue` | normal | Int | 0 | `Settings.player.decoder.hue_filter` | KEEP | P0 |  |

## AudioPreferences (`app/src/main/java/eu/kanade/tachiyomi/ui/player/settings/AudioPreferences.kt`), 5 keys, Rust section `player.audio`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `preferredAudioLanguages()` | `pref_audio_lang` | normal | String | "" | `Settings.player.audio.preferred_audio_languages` | KEEP | P1 |  |
| 2 | `enablePitchCorrection()` | `pref_audio_pitch_correction` | normal | Boolean | true | `Settings.player.audio.enable_pitch_correction` | KEEP | P1 |  |
| 3 | `audioChannels()` | `pref_audio_config` | normal | Enum AudioChannels (name string) | AutoSafe | `Settings.player.audio.audio_channels` | KEEP | P1 | QUIRK: enum Auto -> mpv "auto-safe" and AutoSafe -> mpv "auto" (names swapped vs values). Default AutoSafe => mpv audio-channels=auto. |
| 4 | `volumeBoostCap()` | `pref_audio_volume_boost_cap` | normal | Int | 30 | `Settings.player.audio.volume_boost_cap` | KEEP | P1 | mpv volume-max = 100 + value. |
| 5 | `audioDelay()` | `pref_audio_delay` | normal | Int | 0 | `Settings.player.audio.audio_delay` | KEEP | P1 |  |

## SubtitlePreferences (`app/src/main/java/eu/kanade/tachiyomi/ui/player/settings/SubtitlePreferences.kt`), 21 keys, Rust section `player.subtitles`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `preferredSubLanguages()` | `pref_subtitle_lang` | normal | String | "" | `Settings.player.subtitles.preferred_sub_languages` | KEEP | P1 |  |
| 2 | `subtitleWhitelist()` | `pref_subtitle_whitelist` | normal | String | "" | `Settings.player.subtitles.subtitle_whitelist` | KEEP | P1 |  |
| 3 | `subtitleBlacklist()` | `pref_subtitle_blacklist` | normal | String | "" | `Settings.player.subtitles.subtitle_blacklist` | KEEP | P1 |  |
| 4 | `screenshotSubtitles()` | `pref_screenshot_subtitles` | normal | Boolean | false | `Settings.player.subtitles.screenshot_subtitles` | KEEP | P2 |  |
| 5 | `subtitleFont()` | `pref_subtitle_font` | normal | String | "Sans Serif" | `Settings.player.subtitles.subtitle_font` | KEEP | P1 |  |
| 6 | `subtitleFontSize()` | `pref_subtitles_font_size` | normal | Int | 55 | `Settings.player.subtitles.subtitle_font_size` | KEEP | P1 | mpv sub-font-size. |
| 7 | `subtitleFontScale()` | `pref_sub_scale` | normal | Float | 1.0 | `Settings.player.subtitles.subtitle_font_scale` | KEEP | P1 | mpv sub-scale. |
| 8 | `subtitleBorderSize()` | `pref_sub_border_size` | normal | Int | 3 | `Settings.player.subtitles.subtitle_border_size` | KEEP | P1 |  |
| 9 | `boldSubtitles()` | `pref_bold_subtitles` | normal | Boolean | false | `Settings.player.subtitles.bold_subtitles` | KEEP | P1 |  |
| 10 | `italicSubtitles()` | `pref_italic_subtitles` | normal | Boolean | false | `Settings.player.subtitles.italic_subtitles` | KEEP | P1 |  |
| 11 | `textColorSubtitles()` | `pref_text_color_subtitles` | normal | Int | -1 (0xFFFFFFFF) | `Settings.player.subtitles.text_color_subtitles` | KEEP | P1 |  |
| 12 | `borderColorSubtitles()` | `pref_border_color_subtitles` | normal | Int | -16777216 (0xFF000000) | `Settings.player.subtitles.border_color_subtitles` | KEEP | P1 |  |
| 13 | `borderStyleSubtitles()` | `pref_border_style_subtitles` | normal | Enum SubtitlesBorderStyle (name string) | OutlineAndShadow | `Settings.player.subtitles.border_style_subtitles` | KEEP | P1 |  |
| 14 | `shadowOffsetSubtitles()` | `sub_shadow_offset` | normal | Int | 0 | `Settings.player.subtitles.shadow_offset_subtitles` | KEEP | P1 |  |
| 15 | `backgroundColorSubtitles()` | `pref_background_color_subtitles` | normal | Int | 0 (0x00000000) | `Settings.player.subtitles.background_color_subtitles` | KEEP | P1 |  |
| 16 | `subtitleJustification()` | `pref_sub_justify` | normal | Enum SubtitleJustification (name string) | Auto | `Settings.player.subtitles.subtitle_justification` | KEEP | P1 |  |
| 17 | `subtitlePos()` | `pref_sub_pos` | normal | Int | 100 | `Settings.player.subtitles.subtitle_pos` | KEEP | P1 | mpv sub-pos 0..100. |
| 18 | `overrideSubsASS()` | `pref_override_subtitles_ass` | normal | Boolean | false | `Settings.player.subtitles.override_subs_ass` | KEEP | P1 |  |
| 19 | `subtitlesDelay()` | `pref_subtitles_delay` | normal | Int | 0 | `Settings.player.subtitles.subtitles_delay` | KEEP | P1 |  |
| 20 | `subtitlesSpeed()` | `pref_subtitles_speed` | normal | Float | 1.0 | `Settings.player.subtitles.subtitles_speed` | KEEP | P1 |  |
| 21 | `subtitlesSecondaryDelay()` | `pref_subtitles_secondary_delay` | normal | Int | 0 | `Settings.player.subtitles.subtitles_secondary_delay` | KEEP | P1 |  |

## AdvancedPlayerPreferences (`app/src/main/java/eu/kanade/tachiyomi/ui/player/settings/AdvancedPlayerPreferences.kt`), 4 keys, Rust section `player.advanced`

| # | Kotlin fn | Key | Store | Kotlin type | Default | Rust field | Dec | Pri | Notes |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `mpvUserFiles()` | `mpv_scripts` | normal | Boolean | false | `Settings.player.advanced.mpv_user_files` | KEEP-CHANGED | P1 | Enables loading ~/.config/kurobayomi/mpv scripts/fonts. |
| 2 | `mpvConf()` | `pref_mpv_conf` | normal | String | "" | `Settings.player.advanced.mpv_conf` | KEEP | P1 | Text of mpv.conf. |
| 3 | `mpvInput()` | `pref_mpv_input` | normal | String | "" | `Settings.player.advanced.mpv_input` | KEEP | P1 | Text of input.conf. |
| 4 | `playerStatisticsPage()` | `pref_player_statistics_page` | normal | Int | 0 | `Settings.player.advanced.player_statistics_page` | KEEP | P2 |  |

Total rows: 328.
