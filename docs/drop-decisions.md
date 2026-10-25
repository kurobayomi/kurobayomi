# Dropped Android build steps

Decisions recorded for area 54. Each item names the task, what was dropped, and what the user loses. Checked by
`scripts/check-android-leftovers.sh`.

| Task | Dropped | Replaced by | User impact |
|---|---|---|---|
| T-54a-014 | Upstream funding links file | nothing; the repository has no funding file | none for the app; the upstream maintainer's sponsor links are not shown here. A maintainer who wants links adds their own and notes it in CONTRIBUTING.md |
| T-54a-022 | Android integer version number | `[workspace.package] version` in `Cargo.toml` is the only version | none; Flatpak updates are ordered by the metainfo `<release>` list and the branch |
| T-54a-024 | Generated ahead-of-time compilation profile (37,790 lines) and its generator and readme | startup is measured by `scripts/bench-startup.sh` | none; a native binary has no JIT warm-up for a profile to replace |
| T-54a-025 | The macrobenchmark module | `scripts/bench-startup.sh`, `docs/benchmarking.md` | none |
| T-54a-026 | ProGuard and consumer rule files (14 files) | none; nothing in the Rust build reads a keep list | none |
| T-54a-030 | Variant filter block that referred to flavors that do not exist | none | none; the code never ran |
| T-54a-036 | Precompiled build plugins directory | workspace lints, `cargo xtask lint` | none |
| T-54a-043 | Glance home-screen widget | see below | the Android home-screen widget is gone |
| T-54a-060 | Android-only CI steps | see the table below | none |

## Extension keep list is obsolete

The extension API keep list (the source-api consumer rule file and lines 25 to 31 of the app rule file) existed so the
Android shrinker would not remove classes that extension APKs load by name. Extensions in Kurobayomi are not loaded
by class name (area 20, extension strategy redesign), and there is no shrinker. The list is obsolete. This note is the
record for it, because the area 20 decision log sits in the read-only plans.

## Home-screen widget

No `kb-widget` crate exists in v1. A desktop widget was added later as an optional extra: the two Plasma applets in
`dist/plasmoid/` show recent updates from the same data source (`kurobayomi --updates-json`; see `PACKAGING.md`).
A GNOME extension is still a possible later feature with the same data source.

## CI steps (p1-topic-ci-and-release.md, section 1)

No `setup-java` or `setup-gradle` action remains in any workflow.

| # | Step | Status | Replacement or note |
|---|---|---|---|
| 1 | Clone | kept | `actions/checkout` with `fetch-depth: 0` so the commit count is right |
| 2 | Dependency review | replaced | `cargo deny check` |
| 3 | Set up JDK 17 | dropped | Rust toolchain from `rust-toolchain.toml` |
| 4 | Set up Gradle | replaced | Rust build cache |
| 5 | Formatting check | replaced | `cargo xtask lint` |
| 6 | Assemble release | replaced | `cargo build --profile dist --locked` (`--features updater` for the tarball only) |
| 7 | Unit tests | replaced | `cargo test --workspace --locked` (`scripts/test-workspace.sh`) |
| 8 | Upload APK | replaced | upload the tarball and the Flatpak bundle |
| 9 | Upload R8 mapping | dropped | no R8 mapping file to share; the release carries split debug info (no user impact in practice) |
| 10 | Get tag name | kept | plus a check that the tag equals `v` and the `Cargo.toml` version |
| 11 | Sign APK (third-party signing action) | dropped | GPG-signed `SHA256SUMS` (T-54a-056) |
| 12 | Clean up artifacts and hash | replaced | `cargo xtask dist` writes `SHA256SUMS` |
| 13 | Create release | kept | same release action, draft release, body from the changelog |

## Android platform pieces not carried over (areas 26, 35 and 53)

Each row is checked by `src/dropped/mod.rs`: the test compares the row word for word and scans the code of `src/` for the dropped names.
The drop register of the legacy import (T-11a-029) is `src/dropped/register.rs`.
T-11a-029 is PARTIAL: the table, `lookup()`, `report()` and their tests exist, but nothing calls them yet, because the T-11a-010 normaliser and the import dialog do not exist in `src/` or `crates/`. When the normaliser lands it must skip keys through `dropped::register::lookup()` and the import dialog must list `report()`. The MIUI installer is covered only through the `extension_installer` row.

| Task | Dropped | Replaced by | User impact |
|---|---|---|---|
| T-26a-120 | foreground service declaration of the library update job | the resident process or the systemd timer | nothing beyond background running when the app is closed in mode A without the portal |
| T-26a-121 | notification channels and the deprecated channel cleanup | freedesktop urgency per kind and one notification switch (T-26a-036) | per-channel system settings |
| T-26a-122 | Wi-Fi pre-check for Android versions below 28 | the NetworkManager constraint (T-26a-027) in every case | none |
| T-26a-123 | FileProvider URIs for shared files | plain file:// URIs from gio (T-26a-096) | none |
| T-26a-124 | Injekt dependency lookup | explicit PassDeps construction at startup | none |
| T-26a-125 | WorkManager migrations | validation of the stored interval: only 0, 12, 24, 48, 72 and 168 hours, other values snap to the nearest choice | none |
| T-26a-126 | decoding the launcher icon bitmap for notifications | the freedesktop app icon name | none |
| T-35a-012 | channel creation and the nine deprecated channel ids | the notification switches of T-35a-004 | per-channel system settings |
| T-35a-025 | the unused ACTION_OPEN_ENTRY constant | entry opening of T-35a-015 | none |
| T-35a-026 | group summary dismissal of the notification receiver | none; desktop notices have no group summary | none visible; a single notice cannot be left behind |
| T-35a-027 | Samsung S Pen remote actions | the five actions as keyboard keys (T-35a-028) | S Pen remote gestures |
| T-35a-029 | searchable.xml, the Android global search hint and search provider | the in-app global search entry; the GNOME search provider is an optional extra of T-53a-066, not part of v1 | system-wide search for library titles on Android |
| T-35a-030 | FileProvider paths (provider_paths.xml) | plain file paths; no content URIs are created | none |
| T-35a-031 | network_security_config.xml (the Android cleartext and certificate-authority policy) | the reqwest client: rustls with the system certificate verifier, so the system and user-installed CAs are trusted, and http:// URLs are not refused | none |
| T-35a-056 | APK install and in-place AppImage self-replacement | the update notice offers Open folder and the user replaces the file | the app cannot replace itself |
| T-35a-059 | foreground service and the data sync service type | none; a desktop process is not suspended | none |
| T-35a-060 | the OsTooOld result of the update check | none; the check has no operating system version gate | none |
| T-35a-061 | the preview build channel with commit-count tags | one stable channel | preview builds through the in-app updater |
| T-35a-063 | the runtime notification permission check | the toast fallback of T-35a-011 and T-35a-002 when the notification service is missing | none |
| T-35a-064 | accent colour tint on notifications | the desktop theme tint | Android-style tinted notifications |
| T-35a-065 | Android sound override for the backup-complete notice | none; the category keeps high priority and the desktop picks the sound | the backup-complete notice may play the desktop sound |
| T-53a-036 | extension install services and activities | extension install of area 20 | installing APK extensions, and the vendor-skin timing workaround (defect E-29) is not carried |
| T-53a-038 | package install, delete and query permissions and the privileged installer provider; the extension_installer preference is not ported | none; no package query or install permission exists on Linux | installing extensions through the privileged helper app |
| T-53a-046 | the deep-link trampoline activities | route_request receives the search request directly | none; the forwarding was internal |
| T-53a-052 | the ten unreferenced player animations | none; the player has no transition, as in Aniyomi | none |
| T-53a-056 | the unused preference switch layout | gtk::Switch rows | none |
| T-53a-059 | the chapter download menu | the row menu of the download queue (T-53a-060) | none |
| T-53a-061 | the old bottom navigation menu | the HomeScreen tab set of area 36 | none |
| T-53a-062 | the md_* keep rule | none; no md_* layout exists | none |
| T-53a-064 | FileProvider and the path whitelist | files are opened through Opener | none visible |
| T-53a-065 | S Pen remote actions | keys Right, Left, Page Up, Page Down, N and P in the reader and player; Escape and Alt+Left go back | S Pen remote gestures |
| T-53a-067 | the empty module manifests and the macrobenchmark module | one Cargo crate per module, none needs a manifest | none |
| T-53a-068 | the home-screen widget receivers and provider files | the Updates tab (area 42) and desktop notices (area 35); the optional Plasma applets read the same data | the Android home-screen widget |

## Phone-only features removed (desktop)

A desktop window has no touch screen, no sensors, no task switcher and no battery-saver service, so these phone
features are removed, not hidden. The preference keys are listed in `DROPPED_KEYS` in `crates/kuro-prefs/src/registry.rs`
(the registry has no row for them), the text names in `DROPPED_NAMES` in `crates/kuro-i18n/src/import.rs` (the importer
leaves them out of every catalog). A key that an old `config.toml` or an Aniyomi backup still carries is ignored: the
config loader does not read a field it does not know, and the restore skips the key, counts it and reports no error.
Kept on purpose: page turning by swipe or drag in the reader viewers, the horizontal seek gesture and the double taps of
the player, the keyboard shortcuts, the volume slider and mouse-wheel volume, the remembered volume, the update and backup
scheduling constraints (unmetered network, power, battery level), notifications and the torrent server.

| Task | Dropped | Replaced by | User impact |
|---|---|---|---|
| T-desk-001 | screen orientation lock of the reader and the player: `pref_default_orientation_type_key`, `pref_default_player_orientation_type_key`, the per-series orientation bits of the reader flags, and the migration steps that renumbered the old values (rows 12, 13 and 42 of the migration table are DROP) | none; a window is resized by the window manager | none; the stored values are ignored |
| T-desk-002 | battery optimisation rows and the "don't kill my app" note (no preference existed; the texts `pref_disable_battery_optimization`, `battery_optimization_disabled` and their neighbours are gone) | none | none; the backup battery threshold and the update constraint `battery_not_low` stay |
| T-desk-003 | secure screen: `secure_screen_v2` and its older spelling `secure_screen`, with the conversion between them | none; a window cannot be hidden from capture or from the window list | none; screenshots of the app are not blocked, as before |
| T-desk-004 | Shizuku and the extension installer choice: `extension_installer` (already unregistered), the installer texts | the single kuroext install path | none |
| T-desk-005 | swipe actions of the chapter and episode rows: `pref_chapter_swipe_start_action`, `pref_chapter_swipe_end_action`, `pref_episode_swipe_start_action`, `pref_episode_swipe_end_action`, the enums `ChapterSwipeAction` and `EpisodeSwipeAction`, the swipe threshold and the swipe labels (quirk Q22, the crossed keys, goes with them) | the row context menu | row actions are one right click away |
| T-desk-006 | pull to refresh and haptics (no preference existed) | none | none |
| T-desk-007 | brightness and volume gestures of the player: `pref_gesture_volume_brightness`, `pref_swap_volume_and_brightness`, `pref_remember_brightness`, `player_brightness_value` (brightness had no other control) | keyboard shortcuts, the volume slider and mouse-wheel volume | no touch drags for volume or brightness; `pref_remember_volume` and `player_volume_value` stay |
| T-desk-008 | tablet layout mode: `tablet_ui_mode` | none | none; a stored value is ignored |
| T-desk-009 | the Monet app theme (dynamic colours of the phone wallpaper): the `MONET` value of `pref_app_theme` | the GTK theme default and the other palettes | a stored `MONET` reads as the default theme |

## Settings diet, first pass (diet-prefs lane, 2026-10-11)

Source: `docs/design/settings-diet.md`. The registry CUT rows that still have a reader in a file outside the diet-prefs lane cannot be removed without changing a public item another lane uses. They stay registered until that lane removes its readers; then remove the typed field, the registry row, the `spec/settings.json` twin if any, and add the key to `DROPPED_KEYS`.

| Task | Change | Replaced by | User impact |
|---|---|---|---|
| T-diet-001 | `cutout_short` (display cutout): registry row and typed field removed, added to `DROPPED_KEYS` (kept in the Android-only round-trip list as dropped) | fixed `true`, no reader | none; no cutout on desktop |
| T-diet-002 | `library_update_restriction` default: was `wifi`, now no device restriction (`registry.rs`, `library.rs`, the section-5 fixture) | no restriction | a wired PC updates without Wi-Fi; the key stays, its UI row stays until the library lane drops it |
| T-diet-003 | `pref_download_only_over_wifi_key` default: was `true`, now `false` (`registry.rs`, `kuro-domain/src/download/prefs.rs`) | off by default | the registry and the kuro-domain copy now agree; the `kuro-downloads` copy still defaults to `true` and gates the download job (see Open) |

Open, outside this lane: `kuro-downloads/src/download_prefs.rs`, `kuro-downloads/src/settings.rs` and `kuro-downloads/src/downloader.rs` still default `pref_download_only_over_wifi_key` to `true`; the remaining `true` defaults are `crates/kuro-downloads/src/download_prefs.rs:50`, `crates/kuro-downloads/src/settings.rs:11` and `62`, `crates/kuro-downloads/src/downloader.rs:88` and `102`, and `src/settings_ui/downloads.rs:160`. `src/misc/runner.rs` does not read this key; the only consumer is the engine's `DownloadSettings::default()` (`downloader.rs:88`), which the downloads lane must wire to the registry value, so the trap is not gone until then. The two download prefs structs (`kuro-domain::download::prefs::DownloadPrefs` and `kuro-downloads::download_prefs::DownloadPrefs`) are both live: the domain copy is read by `src/entry_core`, `src/reader`, `src/settings_ui/downloads.rs`; the downloads copy by `src/misc`, `src/updates`. Unifying them needs both lanes.

Kept for now, per the user's list (not cut in this pass, the cut rows still exist): app lock (`use_biometric_lock`, `lock_app_after`), external player (`always_use_external_player` is not in this list; `external_player_preference`), mini player (`pref_enable_pip`, `pref_pip_episode_toasts`, `pref_pip_on_exit`, `pip_replace_with_previous`), Random sort (`library_random_anime_sort_seed`, `library_random_manga_sort_seed`), Custom buttons, Script editor and Code editor (`mpv_scripts`, `pref_mpv_conf`, `pref_mpv_input`).

Note: `pref_mpv_conf` and `pref_mpv_input` are kept for now with their Script and Code editor pages, so the diet's import rule (write the text to `mpv.conf` / `input.conf` when the file is missing) is not implemented yet.

### Blocked: CUT keys with a reader outside this lane (first readers listed)

| Key | First readers outside the lane |
|---|---|
| `always_show_chapter_transition` | src/settings_ui/reader/page.rs, src/reader/settings_window.rs |
| `animelib_update_categories` | src/misc/runner.rs, src/updates/stats_data.rs |
| `animelib_update_categories_exclude` | src/misc/runner.rs, src/settings_ui/library_update.rs |
| `app_date_format` | crates/kuro-source-scrape/tests/check.rs, crates/kuro-source-scrape/tests/recipe.rs |
| `__APP_STATE_has_filters_toggle_state` | src/browse/prefs.rs, src/browse/search_screen.rs |
| `__APP_STATE_last_anime_catalogue_source` | src/browse/mount.rs, src/browse/anime/browse_screen.rs |
| `__APP_STATE_last_app_check` | src/dropped/register.rs, src/app_update.rs |
| `__APP_STATE_last_app_closed` | src/dropped/register.rs, src/applock_host.rs |
| `__APP_STATE_last_catalogue_source` | src/browse/mount.rs, src/browse/pages.rs |
| `__APP_STATE_onboarding_complete` | src/shell/onboarding.rs, src/pref_model/onboarding.rs (first-run wizard, removed 2026-10-12) |
| `auto_clear_chapter_cache` | src/main.rs, src/settings_ui/route.rs |
| `auto_update_metadata` | src/misc/runner.rs, src/library_update/pass.rs |
| `backup_flags` | crates/kuro-source/tests/dropped_android_surface.rs |
| `bottom_rail_nav_style` | src/screens.rs, src/main.rs |
| `browse_hide_in_anime_library_items` | src/browse/anime/prefs.rs, src/settings_ui/browse.rs |
| `browse_hide_in_library_items` | src/settings_ui/browse.rs, src/browse/pages.rs |
| `categorized_display` | src/library/t2/sort.rs, src/library/view/mod.rs |
| `check_app_updates` | src/app_update.rs |
| `color_filter_mode` | src/reader/overlay.rs, src/reader/sync.rs |
| `color_filter_value` | src/reader/overlay.rs, src/reader/sync.rs |
| `create_folder_per_manga` | src/reader/screen.rs, src/settings_ui/reader/page.rs |
| `crop_borders_webtoon` | src/reader/screen.rs, src/reader/page_view.rs |
| `custom_brightness_value` | src/reader/overlay.rs, src/reader/sync.rs |
| `default_episode_show_summaries` | src/entry_core/anime/settings_view.rs |
| `default_season_continue_overlay` | src/entry_core/anime/season_tile.rs |
| `default_season_display_mode` | src/entry_core/anime/route.rs |
| `default_season_grid_display_mode` | src/entry_core/anime/route.rs |
| `default_season_grid_display_size` | src/entry_core/anime/route.rs |
| `default_season_lang_overlay` | src/entry_core/anime/season_tile.rs |
| `default_season_local_overlay` | src/entry_core/anime/season_tile.rs |
| `default_season_unseen_overlay` | src/entry_core/anime/season_tile.rs |
| `default_user_agent` | crates/kuro-net/tests/extension_surface.rs, crates/kuro-net/src/client.rs |
| `display_category_tabs` | src/library/layout.rs, src/library/view/mod.rs |
| `display_continue_reading_button` | src/library/display_tab.rs, src/library/view/mod.rs |
| `display_language_badge` | src/entry_core/anime/season_tile.rs, src/library/layout.rs |
| `display_local_badge` | src/library/display_tab.rs, src/library/load.rs |
| `display_number_of_items` | src/library/display_tab.rs, src/library/view/mod.rs |
| `doh_provider` | crates/kuro-net/src/client.rs, crates/kuro-net/src/doh.rs |
| `ext_updates_count` | src/misc/badge.rs, crates/kuro-extensions/src/notice.rs |
| `fullscreen` | src/player_view/tests.rs, src/player_view/mod.rs |
| `hidden_categories` | src/updates/mount.rs, src/updates/prefs.rs |
| `hide_notification_content` | crates/kuro-downloads/README.md, src/misc/runner.rs |
| `incognito_anime_extensions` | src/browse/mount.rs, src/browse/anime/browse_screen.rs |
| `incognito_manga_extensions` | src/browse/mount.rs, src/browse/browse_screen.rs |
| `invert_duration` | crates/kuro-player/src/player_prefs.rs, crates/kuro-player/src/view.rs |
| `landscape_zoom` | src/settings_ui/reader/page.rs, src/reader/page_view.rs |
| `library_show_updates_count` | src/misc/badge.rs, src/settings_ui/library_update.rs |
| `library_update_categories` | src/misc/runner.rs, src/updates/stats_data.rs |
| `library_update_categories_exclude` | src/misc/runner.rs, src/updates/stats_data.rs |
| `library_update_manga_restriction` | src/misc/runner.rs, src/library/load.rs |
| `library_update_restriction` | src/misc/runner.rs, src/settings_ui/library_update.rs |
| `mark_duplicate_read_chapter_read` | src/misc/runner.rs, src/entry_core/backend.rs |
| `mark_duplicate_seen_episode_seen` | src/misc/runner.rs, src/settings_ui/library.rs |
| `navigate_pan` | src/settings_ui/merge.rs, src/settings_ui/reader/page.rs |
| `player_decoder` | src/player_view/mod.rs, crates/kuro-player/src/options.rs |
| `player_fullscreen` | crates/kuro-player/src/player_prefs.rs, src/settings_ui/player/controls.rs |
| `player_hide_controls` | src/settings_ui/player/controls.rs, crates/kuro-player/src/seekbar.rs |
| `pref_allow_gestures_in_panels` | src/settings_ui/player/controls.rs |
| `pref_always_use_external_player` | src/updates/history_screen.rs, src/updates/updates_screen.rs |
| `pref_animelib_columns_portrait_key` | src/browse/anime/prefs.rs, src/library/display_tab.rs |
| `pref_app_theme` | src/theme/apply.rs, src/theme/mod.rs |
| `pref_auto_update_manga_on_mark_read` | src/settings_ui/tracking.rs |
| `pref_center_double_tap` | src/settings_ui/player/seeking.rs |
| `pref_color_filter_key` | src/reader/settings_window.rs, src/reader/overlay.rs |
| `pref_custom_brightness_key` | src/reader/overlay.rs, src/reader/settings_window.rs |
| `pref_default_intro_length` | src/entry_core/anime/route.rs, src/settings_ui/route.rs |
| `pref_disabled_ani_skip_chapters` | src/settings_ui/player/controls.rs |
| `pref_display_profile_key` | crates/kuro-images/src/fetch.rs, crates/kuro-images/src/decode/mod.rs |
| `pref_display_vol_as_per` | src/settings_ui/player/controls.rs |
| `pref_double_tap_anim_speed` | src/reader/viewers/page_opts.rs, src/reader/viewers/settings.rs |
| `pref_dual_page_invert` | src/settings_ui/reader/page.rs, src/reader/settings_window.rs |
| `pref_dual_page_invert_webtoon` | src/settings_ui/reader/page.rs, src/reader/chrome_tests.rs |
| `pref_dual_page_rotate` | src/settings_ui/rows.rs, src/settings_ui/reader/rules.rs |
| `pref_dual_page_rotate_invert` | src/settings_ui/reader/page.rs, src/reader/viewers/settings.rs |
| `pref_dual_page_rotate_invert_webtoon` | src/settings_ui/reader/page.rs, src/reader/settings_window.rs |
| `pref_dual_page_rotate_webtoon` | src/settings_ui/reader/rules.rs, src/settings_ui/reader/page.rs |
| `pref_dual_page_split_webtoon` | src/reader/settings_window.rs, src/reader/chrome_tests.rs |
| `pref_enable_ani_skip` | src/settings_ui/route.rs, src/settings_ui/player/controls.rs |
| `pref_enable_auto_skip_ani_skip` | src/settings_ui/player/controls.rs |
| `pref_enable_double_tap_zoom_webtoon` | src/settings_ui/reader/page.rs, src/reader/viewers/settings.rs |
| `pref_enable_netflixStyle_aniskip` | src/settings_ui/player/controls.rs |
| `pref_enable_skip_intro` | src/settings_ui/route.rs, src/settings_ui/player/controls.rs |
| `pref_enable_transitions_key` | src/reader/viewers/engine.rs, src/reader/viewers/settings.rs |
| `pref_filter_animelib_bookmarked_v2` | src/library/filter_tab.rs, src/library/load.rs |
| `pref_filter_animelib_started_v2` | src/library/filter_tab.rs, src/library/load.rs |
| `pref_filter_library_bookmarked_v2` | src/library/filter_tab.rs, src/library/load.rs |
| `pref_filter_library_interval_custom` | src/library/load.rs |
| `pref_filter_library_started_v2` | src/library/filter_tab.rs, src/library/load.rs |
| `pref_gesture_horizontal_seek` | src/settings_ui/player/seeking.rs, crates/kuro-player/src/gesture_prefs.rs |
| `pref_gpu_next` | crates/kuro-player/src/options.rs, src/settings_ui/player/decoder.rs |
| `pref_grayscale` | src/reader/screen.rs, src/reader/overlay.rs |
| `pref_inverted_colors` | src/reader/overlay.rs, src/reader/settings_window.rs |
| `pref_keep_screen_on_key` | src/settings_ui/reader/page.rs, src/reader/screen.rs |
| `pref_left_double_tap` | src/settings_ui/player/seeking.rs |
| `pref_library_columns_portrait_key` | src/browse/anime/prefs.rs, src/library/display_tab.rs |
| `pref_media_next` | src/settings_ui/player/seeking.rs |
| `pref_media_playpause` | src/settings_ui/player/seeking.rs |
| `pref_media_previous` | src/settings_ui/player/seeking.rs |
| `pref_migration_direction` | src/browse/prefs.rs, src/browse/migrate_list.rs |
| `pref_migration_sorting` | src/browse/migrate_list.rs, src/browse/prefs.rs |
| `pref_panel_opacity` | src/settings_ui/player/controls.rs |
| `pref_player_smooth_seek` | src/settings_ui/player/seeking.rs |
| `pref_player_speed` | crates/kuro-player/src/options.rs, crates/kuro-player/src/strings.rs |
| `pref_player_time_to_disappear` | src/settings_ui/player/controls.rs |
| `pref_preserve_reading_position` | src/settings_ui/reader/page.rs, src/reader/screen.rs |
| `pref_preserve_watching_position` | src/settings_ui/player/controls.rs |
| `pref_progress_preference` | src/settings_ui/player/controls.rs |
| `pref_reader_flash` | src/reader/screen.rs, src/reader/settings_window.rs |
| `pref_reader_flash_duration` | src/settings_ui/reader/page.rs, src/reader/overlay.rs |
| `pref_reader_flash_interval` | src/reader/overlay.rs, src/settings_ui/reader/page.rs |
| `pref_reader_flash_mode` | src/settings_ui/reader/page.rs, src/reader/overlay.rs |
| `pref_reduce_motion` | src/settings_ui/player/controls.rs, crates/kuro-player/src/anim.rs |
| `pref_remember_decoder` | src/player_view/mod.rs, crates/kuro-player/src/options.rs |
| `pref_remember_volume` | src/player_view/mod.rs |
| `pref_right_double_tap` | src/settings_ui/player/seeking.rs |
| `pref_show_current_chapter` | src/settings_ui/player/controls.rs, crates/kuro-player/src/view.rs |
| `pref_show_empty_hosters` | src/settings_ui/player/controls.rs |
| `pref_show_failed_hosters` | src/settings_ui/player/controls.rs |
| `pref_show_loading` | src/settings_ui/player/controls.rs |
| `pref_show_page_number_key` | src/shell/reader_root.rs, src/settings_ui/reader/page.rs |
| `pref_show_reading_mode` | src/settings_ui/reader/page.rs, src/reader/screen.rs |
| `pref_show_seekbar` | src/settings_ui/player/seeking.rs, crates/kuro-player/src/model.rs |
| `pref_skip_length_preference` | src/settings_ui/player/seeking.rs |
| `pref_theme_dark_amoled_key` | src/shell/onboarding.rs, src/settings_ui/appearance.rs |
| `pref_try_hwdec` | src/settings_ui/player/decoder.rs |
| `pref_update_season_on_library_update` | src/misc/runner.rs, src/settings_ui/library.rs |
| `pref_update_season_on_refresh` | src/settings_ui/library.rs, src/entry_core/anime/remote.rs |
| `pref_video_debanding` | src/settings_ui/rows.rs, src/settings_ui/player/decoder.rs |
| `pref_waiting_time_aniskip` | src/settings_ui/player/controls.rs |
| `reader_hide_threshold` | src/settings_ui/reader/page.rs, src/reader/viewers/settings.rs |
| `reader_long_tap` | src/settings_ui/reader/page.rs, src/reader/settings_window.rs |
| `reader_navigation_mode_pager` | src/reader/viewers/settings.rs, src/reader/settings_window.rs |
| `reader_navigation_mode_webtoon` | src/settings_ui/reader/rules.rs, src/settings_ui/reader/page.rs |
| `reader_navigation_overlay_new_user` | src/reader/screen.rs |
| `reader_navigation_overlay_on_start` | src/settings_ui/reader/page.rs, src/reader/nav_overlay.rs |
| `reader_tapping_inverted` | src/settings.rs, src/settings_ui/reader/page.rs |
| `reader_tapping_inverted_webtoon` | src/settings_ui/reader/page.rs, src/reader/settings_window.rs |
| `reader_volume_keys` | src/settings_ui/reader/rules.rs, src/settings_ui/reader/page.rs |
| `reader_volume_keys_inverted` | src/reader/viewers/settings.rs, src/reader/settings_window.rs |
| `relative_time_v2` | src/updates/history_screen.rs, src/updates/updates_screen.rs |
| `show_next_episode_airing_time` | src/settings_ui/tracking.rs, crates/kuro-track/src/prefs.rs |
| `show_not_runnable` | src/ext_host.rs, crates/kuro-extensions/src/anime/browse.rs |
| `skip_dupe` | src/settings_ui/reader/page.rs, src/reader/chapters.rs |
| `skip_filtered` | src/settings_ui/reader/page.rs, src/reader/screen.rs |
| `skip_read` | src/settings_ui/reader/page.rs, src/reader/screen.rs |
| `track_on_adding_to_library` | src/entry_core/route.rs, src/entry_core/tracking/summary.rs |
| `use_yuv420p` | crates/kuro-player/src/player_prefs.rs, crates/kuro-player/src/options.rs |
| `verbose_logging` | src/util/logging.rs, crates/kuro-player/src/options.rs |
| `webtoon_disable_zoom_out` | src/settings_ui/reader/page.rs, src/reader/settings_window.rs |
| `webtoon_side_padding` | src/settings_ui/reader/page.rs, src/reader/viewers/engine.rs |

### Blocked: CUT keys read by another crate's key string, or by a function another lane calls

These have no field-name reader outside the lane, but the key string or a function is used elsewhere. The library entry defaults (`default_*`) are read by `kuro-domain::entries::defaults::set_*_settings_default`, which `src/entry_core` calls from the Set-as-default flows; cut them only after the entry lane removes those calls. The player keys are read by `crates/kuro-player` (`player_prefs.rs`, `aspect.rs`, `speed.rs`, `stats_page.rs`). `pref_always_decode_long_strip_with_ssiv` is read by `src/reader/viewers`. `pref_hardware_bitmap_threshold` is already in `DROPPED_KEYS` and still read by `src/settings_ui/decode.rs` and `src/reader/tiles.rs`. `incognito_extensions` is read by `src/browse/ext_details_page.rs`; the global incognito lists (`incognito_manga_extensions`, `incognito_anime_extensions`) stay, so this one is the twin to unify once that page is changed.

`anime_migrate_flags`, `auto_download_while_reading`, `auto_download_while_watching`, `default_chapter_display_by_name_or_number`, `default_chapter_filter_by_bookmarked`, `default_chapter_filter_by_downloaded`, `default_chapter_filter_by_read`, `default_chapter_sort_by_ascending_or_descending`, `default_chapter_sort_by_source_or_number`, `default_episode_filter_by_bookmarked`, `default_episode_filter_by_downloaded`, `default_episode_filter_by_fillermarked`, `default_episode_filter_by_seen`, `default_episode_show_thumbnail_previews`, `default_episode_sort_by_source_or_number`, `default_season_download_overlay`, `default_season_filter_by_bookmarked`, `default_season_filter_by_completed`, `default_season_filter_by_downloaded`, `default_season_filter_by_fillermarked`, `default_season_filter_by_started`, `default_season_filter_by_unseen`, `default_season_sort_by_ascending_or_descending`, `default_season_sort_by_source_or_number`, `default_speed_presets`, `download_new_anime_categories`, `download_new_anime_categories_exclude`, `download_new_categories`, `download_new_categories_exclude`, `download_new_unread_chapters_only`, `download_new_unread_episodes_only`, `download_speed_limit`, `external_downloader_selection`, `incognito_extensions`, `migrate_flags`, `pref_always_decode_long_strip_with_ssiv`, `pref_audio_config`, `pref_audio_delay`, `pref_audio_pitch_correction`, `pref_audio_volume_boost_cap`, `pref_background_color_subtitles`, `pref_bold_subtitles`, `pref_border_color_subtitles`, `pref_border_style_subtitles`, `pref_download_fillermarked`, `pref_hardware_bitmap_threshold`, `pref_italic_subtitles`, `pref_override_subtitles_ass`, `pref_player_aspect_state`, `pref_player_filter_brightness`, `pref_player_filter_contrast`, `pref_player_filter_gamma`, `pref_player_filter_hue`, `pref_player_filter_saturation`, `pref_player_statistics_page`, `pref_remove_after_marked_as_read_key`, `pref_remove_bookmarked`, `pref_screenshot_subtitles`, `pref_show_system_status_bar`, `pref_sub_border_size`, `pref_sub_justify`, `pref_sub_pos`, `pref_sub_scale`, `pref_subtitle_blacklist`, `pref_subtitle_font`, `pref_subtitles_delay`, `pref_subtitles_font_size`, `pref_subtitles_secondary_delay`, `pref_subtitles_speed`, `pref_subtitle_whitelist`, `pref_text_color_subtitles`, `remove_exclude_anime_categories`, `remove_exclude_categories`, `restore_after_crash`, `save_chapter_as_cbz`, `split_tall_images`, `sub_shadow_offset`, `upcoming.home_returns_today`, `upcoming.week_start`, `use_external_downloader`


## Settings diet cut (diet-finish lane, 2026-10-11)

Source: `docs/design/settings-diet.md`, section 6 tables ("If CUT: hard-code"). Every key below is in `DROPPED_KEYS` in `crates/kuro-prefs/src/registry.rs`, so an Aniyomi backup or an old `config.toml` that carries it still loads, and a restore skips it silently. Data Saver and the Detach feature were cut later on 2026-10-11 (see the last section); their keys are in `DROPPED_KEYS` too. Feature cuts decided by the user: app lock, external player, mini player, random sort, custom buttons, script editor, code editor and the Lua and mpv script bridge (no UI; the user's own `mpv.conf` and `input.conf` files are still honoured).

| Key | Hard-coded value (diet) | Diet page |
|---|---|---|
| `pref_app_theme` | DEFAULT | Appearance (settings_appearance, App language) |
| `pref_theme_dark_amoled_key` | false | Appearance (settings_appearance, App language) |
| `relative_time_v2` | true | Appearance (settings_appearance, App language) |
| `app_date_format` | "" (locale short date) | Appearance (settings_appearance, App language) |
| `bottom_rail_nav_style` | MOVE_HISTORY_TO_MORE | Appearance (settings_appearance, App language) |
| `library_update_restriction` | {} (no restriction) | Library page (settings_library) and the library window sheets |
| `library_update_manga_restriction` | the four default skip rules | Library page (settings_library) and the library window sheets |
| `auto_update_metadata` | false | Library page (settings_library) and the library window sheets |
| `display_continue_reading_button` | false | Library page (settings_library) and the library window sheets |
| `display_category_tabs` | true | Library page (settings_library) and the library window sheets |
| `display_number_of_items` | false | Library page (settings_library) and the library window sheets |
| `categorized_display` | false | Library page (settings_library) and the library window sheets |
| `hidden_categories` | false | Library page (settings_library) and the library window sheets |
| `pref_filter_library_interval_custom` | DISABLED | Library page (settings_library) and the library window sheets |
| `display_local_badge` | true | Library page (settings_library) and the library window sheets |
| `display_language_badge` | false | Library page (settings_library) and the library window sheets |
| `library_show_updates_count` | true | Library page (settings_library) and the library window sheets |
| `library_random_anime_sort_seed` | 0 | Library page (settings_library) and the library window sheets |
| `library_random_manga_sort_seed` | 0 | Library page (settings_library) and the library window sheets |
| `pref_animelib_columns_portrait_key` | 0 (auto) | Library page (settings_library) and the library window sheets |
| `pref_animelib_columns_landscape_key` | 0 (auto) | Library view menu (round 5: its control was gone) |
| `pref_library_columns_landscape_key` | 0 (auto) | Library view menu (round 5: its control was gone) |
| `display_download_badge` | false | Library view menu (round 5: its control was gone) |
| `display_unread_badge` | true | Library view menu (round 5: its control was gone) |
| `pinned_anime_catalogues` | {} | Source pinning (2026-10-23: one on/off switch per source, no pinning) |
| `pinned_catalogues` | {} | Source pinning (2026-10-23: one on/off switch per source, no pinning) |
| `pref_library_columns_portrait_key` | 0 (auto) | Library page (settings_library) and the library window sheets |
| `pref_filter_animelib_started_v2` | DISABLED | Library page (settings_library) and the library window sheets |
| `pref_filter_library_started_v2` | DISABLED | Library page (settings_library) and the library window sheets |
| `pref_filter_animelib_bookmarked_v2` | DISABLED | Library page (settings_library) and the library window sheets |
| `pref_filter_library_bookmarked_v2` | DISABLED | Library page (settings_library) and the library window sheets |
| `animelib_update_categories` | {} (all categories) | Library page (settings_library) and the library window sheets |
| `library_update_categories` | {} (all categories) | Library page (settings_library) and the library window sheets |
| `animelib_update_categories_exclude` | {} | Library page (settings_library) and the library window sheets |
| `library_update_categories_exclude` | {} | Library page (settings_library) and the library window sheets |
| `pref_update_season_on_refresh` | false | Library page (settings_library) and the library window sheets |
| `pref_update_season_on_library_update` | false | Library page (settings_library) and the library window sheets |
| `mark_duplicate_read_chapter_read` | {} (off) | Library page (settings_library) and the library window sheets |
| `mark_duplicate_seen_episode_seen` | {} (off) | Library page (settings_library) and the library window sheets |
| `default_episode_filter_by_seen` | 0 | Library page (settings_library) and the library window sheets |
| `default_chapter_filter_by_read` | 0 | Library page (settings_library) and the library window sheets |
| `default_episode_filter_by_downloaded` | 0 | Library page (settings_library) and the library window sheets |
| `default_chapter_filter_by_downloaded` | 0 | Library page (settings_library) and the library window sheets |
| `default_episode_filter_by_bookmarked` | 0 | Library page (settings_library) and the library window sheets |
| `default_chapter_filter_by_bookmarked` | 0 | Library page (settings_library) and the library window sheets |
| `default_episode_filter_by_fillermarked` | 0 | Library page (settings_library) and the library window sheets |
| `default_episode_sort_by_source_or_number` | 0 (by source) | Library page (settings_library) and the library window sheets |
| `default_chapter_sort_by_source_or_number` | 0 (by source) | Library page (settings_library) and the library window sheets |
| `default_chapter_display_by_name_or_number` | 0 | Library page (settings_library) and the library window sheets |
| `default_chapter_sort_by_ascending_or_descending` | 0 (descending as the app draws it today) | Library page (settings_library) and the library window sheets |
| `default_episode_show_thumbnail_previews` | 0 | Library page (settings_library) and the library window sheets |
| `default_episode_show_summaries` | 0 | Library page (settings_library) and the library window sheets |
| `default_season_filter_by_downloaded` | 0 | Library page (settings_library) and the library window sheets |
| `default_season_filter_by_unseen` | 0 | Library page (settings_library) and the library window sheets |
| `default_season_filter_by_started` | 0 | Library page (settings_library) and the library window sheets |
| `default_season_filter_by_completed` | 0 | Library page (settings_library) and the library window sheets |
| `default_season_filter_by_bookmarked` | 0 | Library page (settings_library) and the library window sheets |
| `default_season_filter_by_fillermarked` | 0 | Library page (settings_library) and the library window sheets |
| `default_season_sort_by_source_or_number` | 0 | Library page (settings_library) and the library window sheets |
| `default_season_sort_by_ascending_or_descending` | 0 | Library page (settings_library) and the library window sheets |
| `default_season_grid_display_mode` | 0 | Library page (settings_library) and the library window sheets |
| `default_season_grid_display_size` | 0 | Library page (settings_library) and the library window sheets |
| `default_season_download_overlay` | false | Library page (settings_library) and the library window sheets |
| `default_season_unseen_overlay` | true | Library page (settings_library) and the library window sheets |
| `default_season_local_overlay` | true | Library page (settings_library) and the library window sheets |
| `default_season_lang_overlay` | false | Library page (settings_library) and the library window sheets |
| `default_season_continue_overlay` | true | Library page (settings_library) and the library window sheets |
| `default_season_display_mode` | 0 | Library page (settings_library) and the library window sheets |
| `pref_enable_transitions_key` | true | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_reader_flash` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_reader_flash_duration` | 100 | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_reader_flash_interval` | 1 | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_reader_flash_mode` | BLACK | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_double_tap_anim_speed` | 500 | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_show_page_number_key` | true | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_show_reading_mode` | true | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `fullscreen` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `cutout_short` | true | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_keep_screen_on_key` | true | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_enable_double_tap_zoom_webtoon` | true | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `always_show_chapter_transition` | true | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_preserve_reading_position` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `navigate_pan` | true | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `landscape_zoom` | true | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `crop_borders_webtoon` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `webtoon_side_padding` | 0 | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `reader_hide_threshold` | LOW | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `create_folder_per_manga` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `skip_read` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `skip_filtered` | true | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `skip_dupe` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `webtoon_disable_zoom_out` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_dual_page_invert` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_dual_page_split_webtoon` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_dual_page_invert_webtoon` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_dual_page_rotate` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_dual_page_rotate_invert` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_dual_page_rotate_webtoon` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_dual_page_rotate_invert_webtoon` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_custom_brightness_key` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `custom_brightness_value` | 0 | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_color_filter_key` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `color_filter_value` | 0 | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `color_filter_mode` | 0 | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_grayscale` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_inverted_colors` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `reader_long_tap` | true | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `reader_volume_keys` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `reader_volume_keys_inverted` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `reader_navigation_mode_pager` | 0 (default layout) | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `reader_navigation_mode_webtoon` | 0 (default layout) | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `reader_tapping_inverted` | NONE | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `reader_tapping_inverted_webtoon` | NONE | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `reader_navigation_overlay_new_user` | true | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `reader_navigation_overlay_on_start` | false | Reader (settings_reader; the reader window has its own Settings sheet over the same keys) |
| `pref_preserve_watching_position` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_progress_preference` | 0.85 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_allow_gestures_in_panels` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_show_loading` | true | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_show_current_chapter` | true | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_remember_volume` | true | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_show_failed_hosters` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_show_empty_hosters` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `player_fullscreen` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `player_hide_controls` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_display_vol_as_per` | true | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_show_system_status_bar` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_reduce_motion` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_player_time_to_disappear` | 4000 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_panel_opacity` | 60 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_enable_skip_intro` | true | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_enable_auto_skip_ani_skip` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_enable_netflixStyle_aniskip` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_waiting_time_aniskip` | 5 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_enable_ani_skip` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_disabled_ani_skip_chapters` | true | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_enable_pip` | true | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_pip_episode_toasts` | true | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_pip_on_exit` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pip_replace_with_previous` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_always_use_external_player` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `external_player_preference` | "" | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_player_speed` | 1.0 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `default_speed_presets` | the 12 default presets | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `invert_duration` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_player_aspect_state` | Fit | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_gesture_horizontal_seek` | true | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_show_seekbar` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_default_intro_length` | 85 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_skip_length_preference` | 10 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_player_smooth_seek` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_left_double_tap` | Seek | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_center_double_tap` | PlayPause | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_right_double_tap` | Seek | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_media_previous` | Switch | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_media_playpause` | PlayPause | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_media_next` | Switch | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_try_hwdec` | true | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_gpu_next` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_video_debanding` | None | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `use_yuv420p` | true | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_player_filter_brightness` | 0 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_player_filter_saturation` | 0 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_player_filter_contrast` | 0 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_player_filter_gamma` | 0 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_player_filter_hue` | 0 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_audio_pitch_correction` | true | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_audio_config` | AutoSafe | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_audio_volume_boost_cap` | 30 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_audio_delay` | 0 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_subtitle_whitelist` | "" | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_subtitle_blacklist` | "" | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_screenshot_subtitles` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_subtitle_font` | Sans Serif | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_subtitles_font_size` | 55 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_sub_scale` | 1.0 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_sub_border_size` | 3 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_bold_subtitles` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_italic_subtitles` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_text_color_subtitles` | -1 (white) | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_border_color_subtitles` | -16777216 (black) | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_border_style_subtitles` | OutlineAndShadow | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `sub_shadow_offset` | 0 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_background_color_subtitles` | 0 (none) | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_sub_justify` | Auto | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_sub_pos` | 100 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_override_subtitles_ass` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_subtitles_delay` | 0 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_subtitles_speed` | 1.0 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_subtitles_secondary_delay` | 0 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `mpv_scripts` | true | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_mpv_conf` | "" (use the real mpv.conf file) | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_mpv_input` | "" (use the real input.conf file) | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_player_statistics_page` | 0 | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_remember_decoder` | false | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `player_decoder` | "" | Player (settings_player and its nine rows). The live player reads almost none of these (see the top section) |
| `pref_download_only_over_wifi_key` | false | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `use_external_downloader` | false | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `external_downloader_selection` | "" | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `save_chapter_as_cbz` | true | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `split_tall_images` | true | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `auto_download_while_reading` | 0 (off) | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `auto_download_while_watching` | 0 (off) | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `pref_remove_after_marked_as_read_key` | false | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `pref_remove_bookmarked` | false | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `pref_download_fillermarked` | false | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `remove_exclude_categories` | {} | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `remove_exclude_anime_categories` | {} | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `download_new_categories` | {} (all) | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `download_new_anime_categories` | {} (all) | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `download_new_categories_exclude` | {} | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `download_new_anime_categories_exclude` | {} | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `download_speed_limit` | 0 (no limit) | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `download_new_unread_chapters_only` | false | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `download_new_unread_episodes_only` | false | Downloads (settings_downloads). The app has no live download manager yet, so only the library-update download rule reads these |
| `pref_migration_sorting` | ALPHABETICAL | Browse (settings_browse and the Sources tab) |
| `pref_migration_direction` | ASCENDING | Browse (settings_browse and the Sources tab) |
| `__APP_STATE_has_filters_toggle_state` | false | Browse (settings_browse and the Sources tab) |
| `incognito_anime_extensions` | {} | Browse (settings_browse and the Sources tab) |
| `incognito_manga_extensions` | {} | Browse (settings_browse and the Sources tab) |
| `__APP_STATE_last_anime_catalogue_source` | -1 | Browse (settings_browse and the Sources tab) |
| `__APP_STATE_last_catalogue_source` | -1 | Browse (settings_browse and the Sources tab) |
| `ext_updates_count` | 0 | Browse (settings_browse and the Sources tab) |
| `show_not_runnable` | false | Browse (settings_browse and the Sources tab) |
| `browse_hide_in_anime_library_items` | false | Browse (settings_browse and the Sources tab) |
| `browse_hide_in_library_items` | false | Browse (settings_browse and the Sources tab) |
| `track_on_adding_to_library` | true | Tracking (settings_tracking) |
| `show_next_episode_airing_time` | true | Tracking (settings_tracking) |
| `pref_auto_update_manga_on_mark_read` | ALWAYS | Tracking (settings_tracking) |
| `backup_flags` | all four flags (everything) | Data and storage (settings_data_storage, settings_create_backup) |
| `auto_clear_chapter_cache` | false | Data and storage (settings_data_storage, settings_create_backup) |
| `use_biometric_lock` | false | Security and privacy (settings_security) |
| `lock_app_after` | 0 | Security and privacy (settings_security) |
| `hide_notification_content` | false | Security and privacy (settings_security) |
| `__APP_STATE_last_app_closed` | 0 | Security and privacy (settings_security) |
| `verbose_logging` | false (release) / true in debug builds | Advanced (settings_advanced) and network |
| `doh_provider` | -1 (off) | Advanced (settings_advanced) and network |
| `default_user_agent` | the Firefox 136 string | Advanced (settings_advanced) and network |
| `pref_display_profile_key` | "" | Advanced (settings_advanced) and network |
| `pref_always_decode_long_strip_with_ssiv` | false | Advanced (settings_advanced) and network |
| `check_app_updates` | false | Base state and notifications (no settings page) |
| `__APP_STATE_last_app_check` | 0 | Base state and notifications (no settings page) |
| `restore_after_crash` | true | Base state and notifications (no settings page) |
| `pref_hardware_bitmap_threshold` | 4096 | Base state and notifications (no settings page) |
| `migrate_flags` | everything (i32::MAX) | Base state and notifications (no settings page) |
| `anime_migrate_flags` | everything (i32::MAX) | Base state and notifications (no settings page) |
| `incognito_extensions` | {} | Base state and notifications (no settings page) |
| `upcoming.week_start` | auto (locale) | Base state and notifications (no settings page) |
| `upcoming.home_returns_today` | false | Base state and notifications (no settings page) |

## Data saver and detach removed (user, 2026-10-11: "remove data saver and detach")

| Item | What was removed | Where | Import |
|---|---|---|---|
| Data Saver (Bandwidth Hero, wsrv.nl proxy) | the proxy service in the download crate, the Advanced "Data Saver" settings group, the source exclusion flag and its toggle, the entries menu item "Exclude from data saver" | `crates/kuro-downloads` (`data_saver` module removed; page requests use the image URL as it is), `src/settings_ui/data_saver.rs`, `crates/kuro-domain/src/source` and `app/source.rs`, `spec/entries.json` | the nine `data_saver*` keys and `ignore_jpeg`, `ignore_gif`, `data_saver_*` are in `DROPPED_KEYS`, so an old `config.toml` or an Aniyomi backup still loads and restores without them |
| Detach (player and reader pop-out window) | the Detach / Attach button, the `Detachable` host (`src/widgets/detachable.rs`), the GL unrealize handoff of the video track, the `[player.window] open_detached` choice | `src/player_view`, `src/reader/screen.rs`, `src/reader/chrome.rs` | `player_open_detached` is in `DROPPED_KEYS` |

The stored keys of both features are in `DROPPED_KEYS` (`crates/kuro-prefs/src/registry.rs`): `data_saver`,
`ignore_jpeg`, `ignore_gif`, `data_saver_image_quality`, `data_saver_image_format_jpeg`, `data_saver_server`,
`data_saver_color_bw`, `data_saver_excluded`, `data_saver_downloader` and `player_open_detached`. An old `config.toml` or
an Aniyomi backup that carries them loads, and a restore skips them.

The translated strings of the removed settings (`data_saver_*`, `bandwidth_hero`, `wsrv`, `resmush` and
`bandwidth_data_saver_server`) are removed from `crates/kuro-i18n/locales/*/kurobayomi.ftl` and listed in `DROPPED_NAMES`
(`crates/kuro-i18n/src/import.rs`), so a regenerated catalog leaves them out. The moko fixture keeps them, because it is
the unchanged importer input.

## Source view restored (2026-10-25)

`pref_display_mode_catalogue` is registered again (`sources.display_mode`, default COMPACT_GRID) and left `DROPPED_KEYS`:
a source's page has the same View, Grid style and Cover size options as the Library (`src/widgets/view_look.rs`), and
its look is remembered for every source, apart from the Library's, as in Aniyomi. An Aniyomi backup's value now restores.
