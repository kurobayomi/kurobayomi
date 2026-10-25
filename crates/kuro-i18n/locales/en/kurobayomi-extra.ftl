# source: kurobayomi
restore_checking_backup = Checking backup file…

# source: kurobayomi
restore_close_keep_running = Keep running

# source: kurobayomi
restore_close_cancel_and_close = Cancel restore and close

# source: kurobayomi
restore_progress_anime_categories = Anime categories

# source: kurobayomi
restore_progress_manga_categories = Manga categories

# source: kurobayomi
backup_extensions_info = Extension packages are saved as Kurobayomi extension files. Android APK extensions are replaced by them and are not included.

# source: kurobayomi
ext_not_runnable_on_linux = Not runnable on Linux

# source: kurobayomi
ext_unsigned_package = Unsigned package

# source: kurobayomi
ext_store_not_runnable_hidden =
    { $count ->
        [one] 1 entry not runnable on Linux
       *[other] { NUMBER($count, useGrouping: "false") } entries not runnable on Linux
    }

# source: kurobayomi
ext_load_problems = Load problems

# source: kurobayomi
settings_save_failed = Could not save settings. Your changes are kept and saved again on the next change.

# source: kurobayomi
pref_download_only_over_wifi = Unmetered connection only

# source: kurobayomi
db_error_too_new = This library was created by a newer version of Kurobayomi (database version { NUMBER($arg1, useGrouping: "false") }, this version supports up to { NUMBER($arg2, useGrouping: "false") }). Update Kurobayomi to open it.

# source: kurobayomi
db_error_legacy_file = This is an Aniyomi library database. Import it to keep your library.

# source: kurobayomi
db_error_corrupt = The library database is damaged. Restore it from the newest backup, or start with an empty library.

# source: kurobayomi
db_restore_from_backup = Restore from backup

# source: kurobayomi
db_import_report = Imported { NUMBER($arg1, useGrouping: "false") } entries. { NUMBER($arg2, useGrouping: "false") } could not be imported.

# source: kurobayomi
db_pre_migration_backup = A backup of your library was saved before the upgrade at { $arg1 }.

# source: kurobayomi
cloudflare_check_title = Cloudflare check

# source: kurobayomi
cloudflare_check_hint = Complete the check in this window; it closes by itself when it succeeds.

# source: kurobayomi
cookie_import_title = Import Cloudflare cookie

# source: kurobayomi
cookie_import_hint = Paste the value of the cf_clearance cookie from your browser and the browser's user agent.

# source: kurobayomi
pref_library_summary = Categories, global update

# source: kurobayomi
pref_player_internal_summary = Progress, controls

# source: kurobayomi
pref_security_summary = App lock

# source: kurobayomi
pref_advanced_summary = Dump crash logs

# source: kurobayomi
pref_reset_viewer_flags_summary = Resets reading mode of all series

# source: kurobayomi
add_source_repository = Add a source repository

# source: kurobayomi
restore_from_backup = Restore from a backup

# source: kurobayomi
library_empty_search_hint = Your library is empty. Search for something in Browse to add it.

# source: kurobayomi
welcome_title = Welcome to Kurobayomi

# source: kurobayomi
welcome_intro = Add a source repository to start watching anime and reading manga.

# source: kurobayomi
welcome_guide = Getting started guide

# source: kurobayomi
action_select = Select

# source: kurobayomi
episode_time_left = { $arg1 } left

# source: kurobayomi
action_delete_download = Delete download

# source: kurobayomi
action_mark_previous_as_seen = Mark previous as watched

# source: kurobayomi
label_extension_repos = Repositories

# source: kurobayomi
left_to_right_viewer = Paged, left to right

# source: kurobayomi
right_to_left_viewer = Paged, right to left

# source: kurobayomi
vertical_viewer = Paged, vertical

# source: kurobayomi
source_status_active = Working

# source: kurobayomi
source_status_blocked = Temporarily blocked by the site

# source: kurobayomi
source_status_broken = Broken: waiting for a fix

# source: kurobayomi
browse_check_sources = Check for new and updated sources

# source: kurobayomi
downloads_check_files = Check the downloaded files again

# source: kurobayomi
reader_unknown_err = Unknown err

# source: kurobayomi
reader_chapter_not_downloaded = The chapter is not downloaded. Turn off downloaded-only mode or download it first.

# source: kurobayomi
reader_action_show_in_file_manager = Show in file manager

# source: kurobayomi
reader_custom_brightness_unavailable = Custom brightness is not available on this display

# source: kurobayomi
reader_menu_reader_settings = Reader settings…

# source: kurobayomi
reader_cutout_dropped_note = Show content in cutout area is not used on Linux: there is no display cutout.

# source: kurobayomi
reader_volume_keys_media_caveat = The desktop can take the volume keys before the reader gets them. Page Up, Page Down and Space always turn pages.

# source: kurobayomi
reader_pref_restore_after_crash = Restore the chapter and page after a crash

# source: kurobayomi
reader_requested_chapter_not_found = Requested chapter of id { NUMBER($arg1, useGrouping: "false") } not found in chapter list

# source: kurobayomi
browse_errors_offline = You're offline. Check your connection and try again.

# source: kurobayomi
browse_errors_generic = Something went wrong. Try again in a moment.

# source: kurobayomi
browse_errors_not_working_badge = Not working right now

# source: kurobayomi
browse_global_search_empty_hint = Search a source by title

# source: kurobayomi
browse_loading_sources = Loading sources…

# source: kurobayomi
entry_list_options_tooltip = List options

# source: kurobayomi
library_exclude_hint = Use -term to exclude

# source: kurobayomi
misc_interval_first_run_note = The first automatic update runs as soon as the conditions are met after you turn this on.

# source: kurobayomi
repos_adding_repo = Adding repository…

# source: kurobayomi
repos_could_not_add_repo = Could not add the repo

# source: kurobayomi
reader_still_loading = The page list is still loading. Try again in a moment.

# source: kurobayomi
crash_no_details = No crash details available

# source: kurobayomi
crash_subtitle = Kurobayomi ran into an unexpected error. Share the crash logs when you report the problem.

# source: kurobayomi
about_build_time_title = Build time

# source: kurobayomi
about_model_title = Device model

# source: kurobayomi
about_menu_title = Show in applications menu

# source: kurobayomi
about_startup_check_title = Check for updates at startup

# source: kurobayomi
about_startup_check_subtitle = A newer version is downloaded in the background and runs after a restart

# source: kurobayomi
settings_auth_unlock_button = Unlock

# source: kurobayomi
settings_auth_wrong_passphrase = Wrong passphrase

# source: kurobayomi
settings_browse_source_languages = Source languages

# source: kurobayomi
browse_nsfw_refused = Authentication is required to change this setting

# source: kurobayomi
settings_chooser_unavailable = No file chooser available

# source: kurobayomi
settings_library_csv_failed = Could not export library

# source: kurobayomi
settings_maintenance_copied_text = Crash logs copied to the clipboard

# source: kurobayomi
settings_maintenance_failed_text = Failed to get logs

# source: kurobayomi
settings_notifications_new_items_title = Notify about new chapters and episodes

# source: kurobayomi
player_default_volume = Default volume

# source: kurobayomi
settings_storage_info = Holds downloads (Anime, Manga), backups and local folders. A new location is used for new downloads after a restart; files already there stay where they are.

# source: kurobayomi
settings_tracking_not_ready = Tracker services are not available yet

# source: kurobayomi
settings_tracking_waiting = Waiting for the browser login…

# source: kurobayomi
settings_usage_unavailable = Storage size unavailable

# source: kurobayomi
shell_incognito_label = Incognito

# source: kurobayomi
storage_storage_scanning = Scanning downloads…

# source: kurobayomi
storage_storage_empty_category = No downloads in this category

# source: kurobayomi
storage_storage_source_missing = Source not installed; files not deleted

# source: kurobayomi
storage_delete_source_missing = Source not installed, downloads were not deleted

# source: kurobayomi
history_row_open_details = Open details

# source: kurobayomi
history_row_remove_from_history = Remove from history

# source: kurobayomi
library_update_could_not_start = Could not start library update

# source: kurobayomi
ui_use_for_all_titles = Use for all titles

# source: kurobayomi
ui_download_all = Download all

# source: kurobayomi
ui_connect_anilist = Connect AniList

# source: kurobayomi
ui_tracking_keeps_your_progress_on_anilist = Tracking keeps your progress on AniList.

# source: kurobayomi
ui_stop_tracking = Stop tracking

# source: kurobayomi
ui_from_a_local_folder = From a local folder

# source: kurobayomi
ui_checking_the_downloaded_files_again = Checking the downloaded files again

# source: kurobayomi
ui_compact = Compact

# source: kurobayomi
ui_restart = Restart

# source: kurobayomi
ui_links = Links

# source: kurobayomi
ui_failed_to_reindex_downloads = Failed to reindex downloads

# source: kurobayomi
ui_search_instead = Search instead

# source: kurobayomi
ui_restart_kurobayomi_to_use_the_new_version = Restart Kurobayomi to use the new version

# source: kurobayomi
ui_could_not_load_whats_new = Could not load what's new

# source: kurobayomi
ui_new = New

# source: kurobayomi
ui_update_waiting_open_about = An update is waiting: open Settings, About, What's new

# source: kurobayomi
settings_log_out_of_tracker = Log out of this tracker?

# source: kurobayomi
repos_page_repos_empty = No repositories yet. Add a link someone gave you to get sources.

# source: kurobayomi
repos_page_add_title = Add a repository

# source: kurobayomi
repos_page_add_intro = Paste the repository link.

# source: kurobayomi
repos_page_input_label = Repository link

# source: kurobayomi
library_load_failed = Could not load the library

# source: kurobayomi
browse_migrate_tracks_always = Tracking data is always transferred.

# source: kurobayomi
browse_open_website = Open website

# source: kurobayomi
browse_search_options = Search options

# source: kurobayomi
storage_showing_latest_500 = Showing latest 500

# source: kurobayomi
ui_quit = Quit

# source: kurobayomi
ui_ascending_descending = Ascending / descending

# source: kurobayomi
ui_chapter_unavailable = Chapter unavailable

# source: kurobayomi
ui_not_an_image = Not an image

# source: kurobayomi
ui_page_list_is_empty = Page list is empty

# source: kurobayomi
ui_reading_manga = Reading manga

# source: kurobayomi
ui_the_document_is_larger_than_16_mib = The document is larger than 16 MiB

# source: kurobayomi
ui_added_1_source = Added 1 source

# source: kurobayomi
ui_the_repo_could_not_be_reached = The repo could not be reached

# source: kurobayomi
ui_a_source_does_not_match_its_checksum_in_the = A source does not match its checksum in the repo index

# source: kurobayomi
ui_the_repo_index_is_not_valid = This is not a valid Kurobayomi repository. Kurobayomi only supports its own repositories, not Mihon, Tachiyomi or Aniyomi ones.

# source: kurobayomi
ui_not_yet_watched_or_read = Not yet watched or read

# source: kurobayomi
ui_source_language = Source language

# source: kurobayomi
ui_cover_only = Cover only

# source: kurobayomi
ui_reverse_order = Reverse order

# source: kurobayomi
ui_show_only = Show only

# source: kurobayomi
ui_sort_by = Sort by

# source: kurobayomi
ui_grid_style = Grid style

# source: kurobayomi
ui_view_options = View options

# source: kurobayomi
ui_mark_all_as_read = Mark all as read

# source: kurobayomi
ui_download_unread = Download unread

# source: kurobayomi
ui_mark_all_as_watched = Mark all as watched

# source: kurobayomi
ui_download_unwatched = Download unwatched

# source: kurobayomi
ui_change_categories = Change categories

# source: kurobayomi
ui_search_anilist = Search AniList

# source: kurobayomi
ui_not_tracked_yet = Not tracked yet

# source: kurobayomi
ui_open_on_anilist = Open on AniList

# source: kurobayomi
ui_not_set = Not set

# source: kurobayomi
ui_the_start_date_can_t_be_in_the_future_or_aft = The start date can't be in the future or after the finish date.

# source: kurobayomi
ui_the_finish_date_can_t_be_in_the_future_or_be = The finish date can't be in the future or before the start date.

# source: kurobayomi
ui_update_tracker = Update tracker

# source: kurobayomi
ui_up_down = Up / Down

# source: kurobayomi
ui_move_focus_in_the_chapter_list = Move focus in the chapter list

# source: kurobayomi
ui_open_the_focused_chapter = Open the focused chapter

# source: kurobayomi
ui_toggle_the_focused_chapter = Toggle the focused chapter

# source: kurobayomi
ui_select_a_range_from_the_anchor = Select a range from the anchor

# source: kurobayomi
ui_toggle_one_chapter = Toggle one chapter

# source: kurobayomi
ui_invert_the_selection = Invert the selection

# source: kurobayomi
ui_clear_the_selection = Clear the selection

# source: kurobayomi
ui_open_the_filter_sheet = Open the filter sheet

# source: kurobayomi
ui_open_the_webview = Open the WebView

# source: kurobayomi
ui_f1_or = F1 or ?

# source: kurobayomi
ui_show_this_help = Show this help

# source: kurobayomi
ui_keyboard_shortcuts = Keyboard shortcuts

# source: kurobayomi
ui_this_episode_is_no_longer_in_the_library = This episode is no longer in the library

# source: kurobayomi
ui_this_anime_is_no_longer_in_the_library = This anime is no longer in the library

# source: kurobayomi
ui_something_went_wrong_while_reading_this_entr = Something went wrong while reading this entry

# source: kurobayomi
ui_use_as_default_for_all_titles = Use as default for all titles

# source: kurobayomi
ui_source_order = Source order

# source: kurobayomi
ui_upload_date = Upload date

# source: kurobayomi
ui_every_title_in_your_library_takes_these_list = Every title in your library takes these list options, and titles you open later start with them.

# source: kurobayomi
ui_this_source = This source

# source: kurobayomi
ui_subtitle_language = Subtitle language

# source: kurobayomi
ui_audio_language = Audio language

# source: kurobayomi
ui_autoplay_next_episode = Autoplay next episode

# source: kurobayomi
ui_theme_mode = Theme mode

# source: kurobayomi
ui_no_passphrase_is_set = No passphrase is set

# source: kurobayomi
ui_hide_the_bars_automatically = Hide the bars automatically

# source: kurobayomi
ui_while_reading_or_watching_the_top_bar_and_th = While reading or watching, the top bar and the controls get out of the way

# source: kurobayomi
ui_type_to_search_every_setting = Type to search every setting

# source: kurobayomi
ui_your_appimage_manager_already_lists_it_there = Your AppImage manager already lists it there

# source: kurobayomi
ui_with_its_icon_like_an_installed_app = With its icon, like an installed app

# source: kurobayomi
ui_report_an_issue = Report an issue

# source: kurobayomi
ui_source_not_installed = Source not installed

# source: kurobayomi
ui_missing_repository_address = Missing repository address

# source: kurobayomi
ui_anime_library = Anime library

# source: kurobayomi
ui_downloads_running = Downloads running

# source: kurobayomi
ui_no_results = No results

# source: kurobayomi
ui_couldn_t_open_this_link = Couldn't open this link

# source: kurobayomi
ui_add_local_manga_folder = Link a manga folder

# source: kurobayomi
ui_add_local_anime_folder = Link an anime folder

# source: kurobayomi
ui_quality_and_server = Quality and server

# source: kurobayomi
ui_could_not_load = Could not load

# source: kurobayomi
ui_playback_speed = Playback speed

# source: kurobayomi
ui_playing_video = Playing video

# source: kurobayomi
ui_quality_loading = Quality: loading…

# source: kurobayomi
ui_no_downloads_are_queued = No downloads are queued.

# source: kurobayomi
ui_pause_downloads = Pause downloads

# source: kurobayomi
ui_resume_downloads = Resume downloads

# source: kurobayomi
ui_clear_queue = Clear queue

# source: kurobayomi
ui_waiting_for_source = Waiting for source

# source: kurobayomi
ui_select_count_from_items_i_join_entries_e_on = SELECT count(*) FROM items i JOIN entries e ON e._id = i.entry_id WHERE e.kind = ?1

# source: kurobayomi
ui_this_can_take_a_while_the_app_stays_usable_b = This can take a while. The app stays usable, but do not close it until it is done.

# source: kurobayomi
ui_compact_the_database_to_free_the_space = Compact the database to free the space?

# source: kurobayomi
ui_github_would_not_carry_on_the_download = GitHub would not carry on the download

# source: kurobayomi
ui_github_did_not_answer_in_time = GitHub did not answer in time

# source: kurobayomi
ui_sha256sums_does_not_list_the_appimage = SHA256SUMS does not list the AppImage

# source: kurobayomi
ui_checking_for_updates = Checking for updates…

# source: kurobayomi
ui_up_to_date = Up to date

# source: kurobayomi
ui_nightly_update_installed = Nightly update installed

# source: kurobayomi
ui_update_installed = Update installed

# source: kurobayomi
ui_no_changes_since_your_last_version = No changes since your last version.

# source: kurobayomi
ui_comfortable = Comfortable

# source: kurobayomi
repos_added =
    { $count ->
        [one] Added 1 source
       *[other] Added { NUMBER($count, useGrouping: "false") } sources
    }

# source: kurobayomi
spec_anime_details = Anime details

# source: kurobayomi
spec_anime_title = Anime title

# source: kurobayomi
spec_auto_play = Auto-play

# source: kurobayomi
spec_automatic_and_manual_backups_maintenance = Automatic and manual backups, maintenance

# source: kurobayomi
spec_backup_file_schema = Backup file schema

# source: kurobayomi
spec_clear = Clear

# source: kurobayomi
spec_downloads_and_storage = Downloads and storage

# source: kurobayomi
spec_kurobayomi_version_licence_and_links = Kurobayomi, version, licence and links

# source: kurobayomi
spec_languages_default_volume_autoplay = Languages, default volume, autoplay

# source: kurobayomi
spec_library_name = Library name

# source: kurobayomi
spec_library_settings = Library settings

# source: kurobayomi
spec_manga_details = Manga details

# source: kurobayomi
spec_maximum_downloads_auto_download_cleanup = Maximum downloads, auto-download, cleanup

# source: kurobayomi
spec_migrate_search = Migrate search

# source: kurobayomi
spec_remove_anilist_tracking = Remove AniList tracking?

# source: kurobayomi
spec_result = Result

# source: kurobayomi
spec_screenshot = Screenshot

# source: kurobayomi
spec_source_name = Source name

# source: kurobayomi
spec_source_search = Source search

# source: kurobayomi
spec_sources_and_tracking = Sources and tracking

# source: kurobayomi
spec_sources_repositories_anilist_cookies = Sources, repositories, AniList, cookies

# source: kurobayomi
spec_tag = Tag

# source: kurobayomi
spec_theme_and_start_screen = Theme and start screen

# source: kurobayomi
spec_track_search = Track search

# source: kurobayomi
spec_welcome = Welcome!

# source: kurobayomi
spec_worker_info = Worker info

# source: kurobayomi
ui_maintenance = Maintenance

# source: kurobayomi
update_github_answered = GitHub answered { $status }

# source: kurobayomi
update_could_not_reach_github = could not reach GitHub

# source: kurobayomi
update_download_broke_off = the download from GitHub broke off

# source: kurobayomi
update_downloading = Downloading { $build }: { $percent }%

# source: kurobayomi
update_paused = Paused { $build }: { $percent }%

# source: kurobayomi
update_installed_restart = { $build } is installed: restart to use it

# source: kurobayomi
update_failed = The update failed: { $error }

# source: kurobayomi
whats_new_stable_available = Kurobayomi { $version } is available

# source: kurobayomi
whats_new_nightly_installed = Nightly { $commit } is installed

# source: kurobayomi
whats_new_stable_installed = Kurobayomi { $version } is installed

# source: kurobayomi
whats_new_nightly_restart = Nightly { $commit } is installed: restart to use it

# source: kurobayomi
whats_new_stable_restart = Kurobayomi { $version } is installed: restart to use it

# source: kurobayomi
whats_new_updated_to_stable = Updated to Kurobayomi { $version }

# source: kurobayomi
whats_new_updated_to_nightly = Updated to nightly { $commit }

# source: kurobayomi
whats_new_changes = Changes

# source: kurobayomi
whats_new_more_changes =
    { $count ->
        [one] …and 1 more change.
       *[other] …and { NUMBER($count, useGrouping: "false") } more changes.
    }

# source: kurobayomi
recipe_could_not_load = Could not load: { $sources }

# source: kurobayomi
reader_page_not_in_archive = Page not found in archive: { $entry }

# source: kurobayomi
reader_page_label = Chapter { $chapter } page { $page }

# source: kurobayomi
reader_page_label_split = Chapter { $chapter } page { $page } (split)

# source: kurobayomi
airing_in_countdown = Airing in { $days }d { $hours }h { $minutes }m { $seconds }s

# source: kurobayomi
browse_couldnt_load_source = Couldn't load { $source }

# source: kurobayomi
browse_source_not_answering = { $source } is not answering. Try again in a moment.

# source: kurobayomi
browse_source_broken = { $source } isn't working right now. It may be down or need an update; the repository checks for fixes every day.

# source: kurobayomi
migration_failed_reason = Migration failed: { $reason }

# source: kurobayomi
storage_base_folder_unavailable = Base folder not available: { $path }

# source: kurobayomi
player_volume_custom = Custom ({ $percent }%)

# source: kurobayomi
updates_bulk_failed = Could not update the selection: { $message }

# source: kurobayomi
page_header_page_of = Page { $page } of { $total }

# source: kurobayomi
local_scan_failed = Could not scan the folder: { $error }

# source: kurobayomi
local_read_failed = Could not read the folder: { $error }

# source: kurobayomi
interval_every_hours = Every { $hours } hours

# source: kurobayomi
library_selected_count = { $count } selected

# source: kurobayomi
library_selected_hidden = { $count } selected ({ $hidden } hidden by the current view)

# source: kurobayomi
browse_couldnt_load_this_source = Couldn't load this source

# source: kurobayomi
ui_source_by_id = Source { $id }

# source: kurobayomi
player_quality = Quality

# source: kurobayomi
player_server = Server

# source: kurobayomi
player_speed_menu = Speed

# source: kurobayomi
player_default_quality = Default quality

# source: kurobayomi
ui_connect_myanimelist = Connect MyAnimeList

# source: kurobayomi
ui_tracking_keeps_your_progress_on_myanimelist = Tracking keeps your progress on MyAnimeList.

# source: kurobayomi
ui_search_myanimelist = Search MyAnimeList

# source: kurobayomi
ui_open_on_myanimelist = Open on MyAnimeList

# source: kurobayomi
welcome_self_update = Kurobayomi updates itself from its GitHub releases. You can turn this off in Settings, under About.

# source: kurobayomi
pref_show_adult_titles = Show adult (18+) titles

# source: kurobayomi
pref_show_adult_titles_summary = While this is off, sources that know a title's rating leave adult titles out, and extensions marked 18+ are hidden.

# source: kurobayomi
local_linked =
    { $count ->
        [one] Linked 1 title
       *[other] Linked { NUMBER($count, useGrouping: "false") } titles
    }

# source: kurobayomi
local_link_nothing_new = Nothing new to add from that folder: its titles are already in the library, or it holds no supported files.

# source: kurobayomi
local_folders_skipped =
    { $count ->
        [one] 1 folder in Local could not be added
       *[other] { NUMBER($count, useGrouping: "false") } folders in Local could not be added
    }

# source: kurobayomi
local_folder_unavailable = The Local folder is not available: check the storage location in Settings.

# source: kurobayomi
ui_open_local_folder = Open the Local folder

# source: kurobayomi
source_switch_off = This source is off

# source: kurobayomi
source_switch_turn_on = Turn on

# source: kurobayomi
source_switch_turn_off = Turn off

# source: kurobayomi
source_switch_off_title = Turn off { $source }?

# source: kurobayomi
source_switch_off_body =
    { $count ->
        [one] 1 title in your library uses this source. It won't update while the source is off.
       *[other] { NUMBER($count, useGrouping: "false") } titles in your library use this source. They won't update while the source is off.
    }

# source: kurobayomi
source_switch_picker_title = Sources in { $repo }

# source: kurobayomi
source_switch_added = Added { $date }

# source: kurobayomi
source_switch_updated = Updated { $date }

# source: kurobayomi
source_switch_picker_intro = Turn on the sources you want. A source that is off is not downloaded or updated, and does not show in Browse or search.

# source: kurobayomi
source_switch_new_sources =
    { $count ->
        [one] 1 new source in { $repo }
       *[other] { NUMBER($count, useGrouping: "false") } new sources in { $repo }
    }

# source: kurobayomi
source_switch_choose = Choose

# source: kurobayomi
source_switch_sources = Sources

# source: kurobayomi
source_switch_all_languages = All languages

# source: kurobayomi
source_switch_check_now = Check now

# source: kurobayomi
source_switch_checking = Checking { $source }…

# source: kurobayomi
source_switch_check_works = { $source } works from your connection

# source: kurobayomi
source_switch_check_blocked = { $source } is blocked from your connection

# source: kurobayomi
source_switch_check_broken = { $source } does not work right now

# source: kurobayomi
source_switch_failed = The source could not be downloaded. Check your connection and try again.

# source: kurobayomi
pref_clear_cache = Clear cache

# source: kurobayomi
pref_cache_size = Cache size limit

# source: kurobayomi
action_reload = Reload

# source: kurobayomi
player_link_refused = This source gave a video address that is not allowed: sources may only play from the internet

# source: kurobayomi
pref_cover_size = Cover size

# source: kurobayomi
ui_install_now = Install now

# source: kurobayomi
ui_update_ready = Update ready

# source: kurobayomi
ui_nightly_update_ready = Nightly update ready

# source: kurobayomi
update_ready_install = { $build } is ready: install it now, or it installs when you close the app

# source: kurobayomi
whats_new_nightly_ready = Nightly { $commit } is ready: install it now, or it installs when you close the app

# source: kurobayomi
whats_new_stable_ready = Kurobayomi { $version } is ready: install it now, or it installs when you close the app

# source: kurobayomi
ui_view = View

# source: kurobayomi
ui_grid = Grid

# source: kurobayomi
ui_open_downloads_folder = Open the downloads folder

# source: kurobayomi
ui_open_containing_folder = Open its folder

# source: kurobayomi
track_auto_title = Track new titles automatically

# source: kurobayomi
track_auto_summary = Titles you add are tracked on your trackers when the match is certain

# source: kurobayomi
track_auto_bound = Tracking { $title } on { $trackers }

# source: kurobayomi
track_pulled_episodes = Marked { $count } episodes as watched from { $tracker }

# source: kurobayomi
track_pulled_chapters = Marked { $count } chapters as read from { $tracker }

# source: kurobayomi
ui_download_not_in_library = The library no longer has this: open its folder to see it

# source: kurobayomi
ui_browser = Browser
