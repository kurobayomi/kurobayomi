//! The preference key registry: one row per preference declared by the Kotlin preference classes.
//!
//! Rows follow `plans/02-preferences-system/p1-topic-key-registry.md` in order; a test checks each
//! row. A key containing `{id}` is a family (per tracker or per source); the id replaces `{id}`.
//! `table` is the dotted TOML table and `field` the TOML key inside it. Family rows keep the id as
//! the TOML key, so `field` is empty. Private keys with a string value never reach a file.
//! The two keys shared by a manga and an anime Kotlin function appear twice on purpose.

use std::collections::BTreeSet;

use crate::keys::Namespace;
use crate::values::PrefValue;

/// A registry default, written as a literal so the table is `const`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lit {
    Bool(bool),
    Int(i32),
    Long(i64),
    Float(f32),
    Str(&'static str),
    Set(&'static [&'static str]),
}

impl Lit {
    /// The default as a stored value.
    pub fn value(self) -> PrefValue {
        match self {
            Lit::Bool(b) => PrefValue::Bool(b),
            Lit::Int(i) => PrefValue::Int(i),
            Lit::Long(l) => PrefValue::Long(l),
            Lit::Float(f) => PrefValue::Float(f),
            Lit::Str(s) => PrefValue::Str(s.to_owned()),
            Lit::Set(items) => PrefValue::Set(
                items
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect::<BTreeSet<_>>(),
            ),
        }
    }
}

/// The stored kind of a value. Each variant has a [`Lit`] variant of the same name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Int,
    Long,
    Float,
    Str,
    Set,
}

/// One preference key.
#[derive(Debug)]
pub struct PrefMeta {
    pub key: &'static str,
    pub ns: Namespace,
    pub kind: Kind,
    pub default: Lit,
    pub table: &'static str,
    pub field: &'static str,
}

impl PrefMeta {
    /// Whether the key is a family, with `{id}` in place of a tracker or source id.
    pub fn is_family(&self) -> bool {
        self.key.contains("{id}")
    }
}

/// Declares preference keys in one of two forms.
///
/// Registry form: rows `key, namespace, kind, default, table, field;` expand to
/// `pub const REGISTRY: &[PrefMeta]`, in row order. The kind token builds both the kind and the
/// default literal, so the two cannot disagree.
///
/// Section form: one settings struct for one TOML table, with a typed handle per key:
///
/// ```text
/// define_prefs! {
///     /// `[network]`: Aniyomi `NetworkPreferences`.
///     pub struct NetworkPrefs in "network" {
///         verbose_logging: bool = "verbose_logging" => cfg!(debug_assertions);
///         doh_provider: i32 = "doh_provider" => -1;
///     }
/// }
/// ```
///
/// The struct gets a `pub` `Pref<T>` per field, built with the declared default, and these
/// associated items: `TABLE` (the TOML table), `FIELDS` (Rust field name and key pairs),
/// `defaults()` (key and default value per field), `new(store)` and `store()`. Section tests
/// compare `FIELDS` and `defaults()` with `REGISTRY`, so a key cannot drift from its table, its
/// TOML field or its default.
macro_rules! define_prefs {
    ($($key:literal, $ns:ident, $kind:ident, $default:expr, $table:literal, $field:literal;)*) => {
        pub const REGISTRY: &[PrefMeta] = &[
            $(PrefMeta {
                key: $key,
                ns: Namespace::$ns,
                kind: Kind::$kind,
                default: Lit::$kind($default),
                table: $table,
                field: $field,
            },)*
        ];
    };
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident in $table:literal {
            $( $(#[$fmeta:meta])* $field:ident : $ty:ty = $key:literal => $default:expr; )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone)]
        $vis struct $name {
            store: $crate::store::PrefStore,
            $( $(#[$fmeta])* pub $field: $crate::pref::Pref<$ty>, )*
        }

        impl $name {
            /// The TOML table this section is stored in.
            pub const TABLE: &'static str = $table;

            /// The Rust field name and registry key of every field, in declaration order.
            pub const FIELDS: &'static [(&'static str, &'static str)] =
                &[$((stringify!($field), $key)),*];

            /// A handle for every key, each with its declared default.
            pub fn new(store: $crate::store::PrefStore) -> Self {
                Self {
                    $($field: $crate::pref::Pref::new(store.clone(), $key, $default),)*
                    store,
                }
            }

            /// The store the handles read and write.
            pub fn store(&self) -> &$crate::store::PrefStore {
                &self.store
            }

            /// The registry key and default value of every field, in declaration order.
            pub fn defaults(&self) -> Vec<(&'static str, $crate::values::PrefValue)> {
                vec![$(($key, $crate::values::PrefType::encode(&self.$field.default_value())),)*]
            }

            /// Startup repair of every field: invalid stored values go back to their defaults.
            pub fn repair(&self) -> Result<(), $crate::PrefError> {
                $(self.$field.repair()?;)*
                Ok(())
            }
        }
    };
}

pub(crate) use define_prefs;

/// Finds the registry row for a stored key. For a family key the id is returned too.
pub fn lookup(key: &str) -> Option<(&'static PrefMeta, Option<&str>)> {
    REGISTRY
        .iter()
        .find_map(|meta| match meta.key.split_once("{id}") {
            None => (meta.key == key).then_some((meta, None)),
            Some((prefix, suffix)) => {
                let id = key.strip_prefix(prefix)?.strip_suffix(suffix)?;
                (!id.is_empty()).then_some((meta, Some(id)))
            }
        })
}

/// Hidden search terms of a preference: its TOML table and its Rust field name. The settings search matches them
/// too, so "tapping" finds `reader.pager_nav_inverted`. `None` for a key the registry does not know.
pub fn search_terms(key: &str) -> Option<[&'static str; 2]> {
    let (meta, _) = lookup(key)?;
    Some([meta.table, meta.field])
}

/// Finds the registry row for a `table.field` path (`library.display_mode`,
/// `player.gestures.gesture_horizontal_seek`). The field is the TOML name after the last dot.
pub fn find_path(path: &str) -> Option<&'static PrefMeta> {
    let (table, field) = path.rsplit_once('.')?;
    REGISTRY.iter().find(|m| m.table == table && m.field == field)
}

/// Plan `pref_torrserver_tackers` default: 26 announce URLs, spaces removed, newline-joined,
/// no trailing newline (p1-data-and-config.md section 6.1).
pub const TORRSERVER_TRACKERS: &str = concat!(
    "http://nyaa.tracker.wf:7777/announce\n",
    "http://anidex.moe:6969/announce\n",
    "http://tracker.anirena.com:80/announce\n",
    "udp://tracker.uw0.xyz:6969/announce\n",
    "http://share.camoe.cn:8080/announce\n",
    "http://t.nyaatracker.com:80/announce\n",
    "udp://47.ip-51-68-199.eu:6969/announce\n",
    "udp://9.rarbg.me:2940\n",
    "udp://9.rarbg.to:2820\n",
    "udp://exodus.desync.com:6969/announce\n",
    "udp://explodie.org:6969/announce\n",
    "udp://ipv4.tracker.harry.lu:80/announce\n",
    "udp://open.stealth.si:80/announce\n",
    "udp://opentor.org:2710/announce\n",
    "udp://opentracker.i2p.rocks:6969/announce\n",
    "udp://retracker.lanta-net.ru:2710/announce\n",
    "udp://tracker.cyberia.is:6969/announce\n",
    "udp://tracker.dler.org:6969/announce\n",
    "udp://tracker.ds.is:6969/announce\n",
    "udp://tracker.internetwarriors.net:1337\n",
    "udp://tracker.openbittorrent.com:6969/announce\n",
    "udp://tracker.opentrackr.org:1337/announce\n",
    "udp://tracker.tiny-vps.com:6969/announce\n",
    "udp://tracker.torrent.eu.org:451/announce\n",
    "udp://valakas.rollo.dnsabr.com:2710/announce\n",
    "udp://www.torrent.eu.org:451/announce",
);

#[rustfmt::skip]
define_prefs! {
    "__APP_STATE_pref_downloaded_only", AppState, Bool, false, "base", "downloaded_only";
    "__APP_STATE_incognito_mode", AppState, Bool, false, "base", "incognito_mode";
    "pref_theme_mode_key", Normal, Str, "SYSTEM", "ui", "theme_mode";
    "start_screen", Normal, Str, "ANIME", "ui", "start_screen";
    "source_languages", Normal, Set, &["all", "en"], "sources", "enabled_languages";
    "show_nsfw_source", Normal, Bool, false, "sources", "show_nsfw_source";
    "anime_extension_repos", Normal, Set, &[], "sources", "anime_extension_repos";
    "extension_repos", Normal, Set, &[], "sources", "manga_extension_repos";
    "__APP_STATE_trusted_extensions", AppState, Set, &[], "sources", "trusted_extensions";
    "hidden_anime_catalogues", Normal, Set, &[], "sources", "disabled_anime_sources";
    "hidden_catalogues", Normal, Set, &[], "sources", "disabled_manga_sources";
    "__PRIVATE_pref_mangasync_username_{id}", Private, Str, "", "secrets", "track_username";
    "__PRIVATE_pref_mangasync_password_{id}", Private, Str, "", "secrets", "track_password";
    "__PRIVATE_pref_tracker_auth_expired_{id}", Private, Bool, false, "tracking.auth_expired", "";
    "__PRIVATE_track_token_{id}", Private, Str, "", "secrets", "track_token";
    "anilist_score_type", Normal, Str, "POINT_10", "tracking", "anilist_score_type";
    "pref_auto_update_manga_sync_key", Normal, Bool, true, "tracking", "auto_update_track";
    "pref_display_mode_library", Normal, Str, "COMPACT_GRID", "library", "display_mode";
    "library_sorting_mode", Normal, Str, "ALPHABETICAL,ASCENDING", "library", "manga_sorting_mode";
    "animelib_sorting_mode", Normal, Str, "ALPHABETICAL,ASCENDING", "library", "anime_sorting_mode";
    "__APP_STATE_library_update_last_timestamp", AppState, Long, 0, "library", "last_updated_timestamp";
    "pref_library_update_interval_key", Normal, Int, 0, "library", "auto_update_interval";
    "pref_filter_animelib_downloaded_v2", Normal, Str, "DISABLED", "library", "filter_downloaded_anime";
    "pref_filter_library_downloaded_v2", Normal, Str, "DISABLED", "library", "filter_downloaded_manga";
    "pref_filter_animelib_unread_v2", Normal, Str, "DISABLED", "library", "filter_unseen";
    "pref_filter_library_unread_v2", Normal, Str, "DISABLED", "library", "filter_unread";
    "pref_filter_animelib_completed_v2", Normal, Str, "DISABLED", "library", "filter_completed_anime";
    "pref_filter_library_completed_v2", Normal, Str, "DISABLED", "library", "filter_completed_manga";
    "pref_filter_animelib_tracked_{id}_v2", Normal, Str, "DISABLED", "library.filter_tracked_anime", "";
    "pref_filter_library_tracked_{id}_v2", Normal, Str, "DISABLED", "library.filter_tracked_manga", "";
    "library_unread_updates_count", Normal, Int, 0, "library", "new_manga_updates_count";
    "library_unseen_updates_count", Normal, Int, 0, "library", "new_anime_updates_count";
    "default_anime_category", Normal, Int, -1, "library", "default_anime_category";
    "default_category", Normal, Int, -1, "library", "default_manga_category";
    "__APP_STATE_last_used_anime_category", AppState, Int, 0, "library", "last_used_anime_category";
    "__APP_STATE_last_used_category", AppState, Int, 0, "library", "last_used_manga_category";
    "remove_after_read_slots", Normal, Int, -1, "downloads", "remove_after_read_slots";
    "download_new", Normal, Bool, false, "downloads", "download_new_chapters";
    "download_new_episode", Normal, Bool, false, "downloads", "download_new_episodes";
    "download_slots", Normal, Int, 3, "downloads", "number_of_downloads";
    "backup_interval", Normal, Int, 12, "backup", "backup_interval";
    "__APP_STATE_last_auto_backup_timestamp", AppState, Long, 0, "backup", "last_auto_backup_timestamp";
    "__APP_STATE_storage_dir", AppState, Str, "", "storage", "base_storage_directory";
    "pref_torrserver_enable", Normal, Bool, false, "torrent", "torr_server_enable";
    "pref_torrserver_shownotice", Normal, Bool, false, "torrent", "torr_server_shown_notice";
    "pref_torrserver_port", Normal, Str, "8090", "torrent", "torr_server_port";
    "pref_torrserver_tackers", Normal, Str, TORRSERVER_TRACKERS, "torrent", "torr_server_trackers";
    "pref_torrserver_proxymode", Normal, Str, "None", "torrent", "torr_server_proxy_mode";
    "pref_torrserver_proxyurl", Normal, Str, "", "torrent", "torr_server_proxy_url";
    "pref_default_reading_mode_key", Normal, Int, 1, "reader", "default_reading_mode";
    "pref_image_scale_type_key", Normal, Int, 1, "reader", "image_scale_type";
    "pref_zoom_start_key", Normal, Int, 1, "reader", "zoom_start";
    "pref_reader_theme_key", Normal, Int, 1, "reader", "reader_theme";
    "crop_borders", Normal, Bool, false, "reader", "crop_borders";
    "pref_dual_page_split", Normal, Bool, false, "reader", "dual_page_split_paged";
    "player_volume_value", Normal, Float, -1.0f32, "player", "player_volume_value";
    "pref_auto_play_enabled", Normal, Bool, false, "player", "autoplay_enabled";
    "pref_audio_lang", Normal, Str, "", "player.audio", "preferred_audio_languages";
    "pref_subtitle_lang", Normal, Str, "", "player.subtitles", "preferred_sub_languages";
    "anime_ext_last_check", Normal, Long, 0, "sources", "anime_ext_last_check";
    "muted_notification_channels", Normal, Set, &[], "base", "muted_notification_channels";
    "check_app_updates", Normal, Bool, true, "base", "check_app_updates";
    "__APP_STATE_last_kind", AppState, Str, "anime", "base", "last_kind";
    "__APP_STATE_appimage_menu_removed", AppState, Bool, false, "base", "appimage_menu_removed";
    "auto_hide_bars", Normal, Bool, true, "base", "auto_hide_bars";
    "pref_player_default_quality", Normal, Int, 1080, "player", "default_quality";
    "enabled_sources", Normal, Set, &[], "sources", "enabled_sources";
    "__APP_STATE_source_keys", AppState, Set, &[], "sources", "source_keys";
    "cache_size_mb", Normal, Int, 500, "storage", "cache_size_mb";
    "library_cover_size", Normal, Int, 160, "library", "cover_size";
    "manga_reader_theme_{id}", Normal, Int, 1, "reader.manga_reader_theme", "";
    "manga_image_scale_type_{id}", Normal, Int, 1, "reader.manga_image_scale_type", "";
    "manga_zoom_start_{id}", Normal, Int, 1, "reader.manga_zoom_start", "";
    "manga_crop_borders_{id}", Normal, Bool, false, "reader.manga_crop_borders", "";
    "manga_dual_page_split_{id}", Normal, Bool, false, "reader.manga_dual_page_split", "";
}

/// Keys of features this desktop app does not have, and of settings the settings diet cut (docs/design/settings-diet.md).
/// The registry holds no row for them, a restore skips them ([`is_dropped`]) and nothing reads them, so an old
/// `config.toml` or an Aniyomi backup that still carries one loads and restores without it. Each key and its reason
/// is recorded in `docs/drop-decisions.md`.
pub const DROPPED_KEYS: [&str; 278] = [
    "extension_installer",
    "pref_hardware_bitmap_threshold",
    "eh_enable_source_blacklist",
    "sources_tab_categories",
    "sources_tab_categories_filter",
    "sources_tab_source_categories",
    "tablet_ui_mode",
    "pref_default_orientation_type_key",
    "pref_default_player_orientation_type_key",
    "secure_screen_v2",
    "secure_screen",
    "pref_episode_swipe_end_action",
    "pref_episode_swipe_start_action",
    "pref_chapter_swipe_end_action",
    "pref_chapter_swipe_start_action",
    "pref_gesture_volume_brightness",
    "pref_swap_volume_and_brightness",
    "pref_remember_brightness",
    "player_brightness_value",
    "cutout_short",
    "pref_app_theme",
    "pref_theme_dark_amoled_key",
    "relative_time_v2",
    "app_date_format",
    "bottom_rail_nav_style",
    "library_update_restriction",
    "library_update_manga_restriction",
    "auto_update_metadata",
    "display_continue_reading_button",
    "display_category_tabs",
    "display_number_of_items",
    "categorized_display",
    "hidden_categories",
    "pref_filter_library_interval_custom",
    "display_local_badge",
    "display_language_badge",
    "library_show_updates_count",
    "library_random_anime_sort_seed",
    "library_random_manga_sort_seed",
    "pref_animelib_columns_portrait_key",
    "pref_library_columns_portrait_key",
    "pref_filter_animelib_started_v2",
    "pref_filter_library_started_v2",
    "pref_filter_animelib_bookmarked_v2",
    "pref_filter_library_bookmarked_v2",
    "animelib_update_categories",
    "library_update_categories",
    "animelib_update_categories_exclude",
    "library_update_categories_exclude",
    "pref_update_season_on_refresh",
    "pref_update_season_on_library_update",
    "mark_duplicate_read_chapter_read",
    "mark_duplicate_seen_episode_seen",
    "default_episode_filter_by_seen",
    "default_chapter_filter_by_read",
    "default_episode_filter_by_downloaded",
    "default_chapter_filter_by_downloaded",
    "default_episode_filter_by_bookmarked",
    "default_chapter_filter_by_bookmarked",
    "default_episode_filter_by_fillermarked",
    "default_episode_sort_by_source_or_number",
    "default_chapter_sort_by_source_or_number",
    "default_chapter_display_by_name_or_number",
    "default_chapter_sort_by_ascending_or_descending",
    "default_episode_show_thumbnail_previews",
    "default_episode_show_summaries",
    "default_season_filter_by_downloaded",
    "default_season_filter_by_unseen",
    "default_season_filter_by_started",
    "default_season_filter_by_completed",
    "default_season_filter_by_bookmarked",
    "default_season_filter_by_fillermarked",
    "default_season_sort_by_source_or_number",
    "default_season_sort_by_ascending_or_descending",
    "default_season_grid_display_mode",
    "default_season_grid_display_size",
    "default_season_download_overlay",
    "default_season_unseen_overlay",
    "default_season_local_overlay",
    "default_season_lang_overlay",
    "default_season_continue_overlay",
    "default_season_display_mode",
    "pref_enable_transitions_key",
    "pref_reader_flash",
    "pref_reader_flash_duration",
    "pref_reader_flash_interval",
    "pref_reader_flash_mode",
    "pref_double_tap_anim_speed",
    "pref_show_page_number_key",
    "pref_show_reading_mode",
    "fullscreen",
    "pref_keep_screen_on_key",
    "pref_enable_double_tap_zoom_webtoon",
    "always_show_chapter_transition",
    "pref_preserve_reading_position",
    "navigate_pan",
    "landscape_zoom",
    "crop_borders_webtoon",
    "webtoon_side_padding",
    "reader_hide_threshold",
    "create_folder_per_manga",
    "skip_read",
    "skip_filtered",
    "skip_dupe",
    "webtoon_disable_zoom_out",
    "pref_dual_page_invert",
    "pref_dual_page_split_webtoon",
    "pref_dual_page_invert_webtoon",
    "pref_dual_page_rotate",
    "pref_dual_page_rotate_invert",
    "pref_dual_page_rotate_webtoon",
    "pref_dual_page_rotate_invert_webtoon",
    "pref_custom_brightness_key",
    "custom_brightness_value",
    "pref_color_filter_key",
    "color_filter_value",
    "color_filter_mode",
    "pref_grayscale",
    "pref_inverted_colors",
    "reader_long_tap",
    "reader_volume_keys",
    "reader_volume_keys_inverted",
    "reader_navigation_mode_pager",
    "reader_navigation_mode_webtoon",
    "reader_tapping_inverted",
    "reader_tapping_inverted_webtoon",
    "reader_navigation_overlay_new_user",
    "reader_navigation_overlay_on_start",
    "pref_preserve_watching_position",
    "pref_progress_preference",
    "pref_allow_gestures_in_panels",
    "pref_show_loading",
    "pref_show_current_chapter",
    "pref_remember_volume",
    "pref_show_failed_hosters",
    "pref_show_empty_hosters",
    "player_fullscreen",
    "player_hide_controls",
    "pref_display_vol_as_per",
    "pref_show_system_status_bar",
    "pref_reduce_motion",
    "pref_player_time_to_disappear",
    "pref_panel_opacity",
    "pref_enable_skip_intro",
    "pref_enable_auto_skip_ani_skip",
    "pref_enable_netflixStyle_aniskip",
    "pref_waiting_time_aniskip",
    "pref_enable_ani_skip",
    "pref_disabled_ani_skip_chapters",
    "pref_enable_pip",
    "pref_pip_episode_toasts",
    "pref_pip_on_exit",
    "pip_replace_with_previous",
    "pref_always_use_external_player",
    "external_player_preference",
    "pref_player_speed",
    "default_speed_presets",
    "invert_duration",
    "pref_player_aspect_state",
    "pref_gesture_horizontal_seek",
    "pref_show_seekbar",
    "pref_default_intro_length",
    "pref_skip_length_preference",
    "pref_player_smooth_seek",
    "pref_left_double_tap",
    "pref_center_double_tap",
    "pref_right_double_tap",
    "pref_media_previous",
    "pref_media_playpause",
    "pref_media_next",
    "pref_try_hwdec",
    "pref_gpu_next",
    "pref_video_debanding",
    "use_yuv420p",
    "pref_player_filter_brightness",
    "pref_player_filter_saturation",
    "pref_player_filter_contrast",
    "pref_player_filter_gamma",
    "pref_player_filter_hue",
    "pref_audio_pitch_correction",
    "pref_audio_config",
    "pref_audio_volume_boost_cap",
    "pref_audio_delay",
    "pref_subtitle_whitelist",
    "pref_subtitle_blacklist",
    "pref_screenshot_subtitles",
    "pref_subtitle_font",
    "pref_subtitles_font_size",
    "pref_sub_scale",
    "pref_sub_border_size",
    "pref_bold_subtitles",
    "pref_italic_subtitles",
    "pref_text_color_subtitles",
    "pref_border_color_subtitles",
    "pref_border_style_subtitles",
    "sub_shadow_offset",
    "pref_background_color_subtitles",
    "pref_sub_justify",
    "pref_sub_pos",
    "pref_override_subtitles_ass",
    "pref_subtitles_delay",
    "pref_subtitles_speed",
    "pref_subtitles_secondary_delay",
    "mpv_scripts",
    "pref_mpv_conf",
    "pref_mpv_input",
    "pref_player_statistics_page",
    "pref_remember_decoder",
    "player_decoder",
    "pref_download_only_over_wifi_key",
    "use_external_downloader",
    "external_downloader_selection",
    "save_chapter_as_cbz",
    "split_tall_images",
    "auto_download_while_reading",
    "auto_download_while_watching",
    "pref_remove_after_marked_as_read_key",
    "pref_remove_bookmarked",
    "pref_download_fillermarked",
    "remove_exclude_categories",
    "remove_exclude_anime_categories",
    "download_new_categories",
    "download_new_anime_categories",
    "download_new_categories_exclude",
    "download_new_anime_categories_exclude",
    "download_speed_limit",
    "download_new_unread_chapters_only",
    "download_new_unread_episodes_only",
    "pref_display_mode_catalogue",
    "pref_migration_sorting",
    "pref_migration_direction",
    "__APP_STATE_has_filters_toggle_state",
    "incognito_anime_extensions",
    "incognito_manga_extensions",
    "__APP_STATE_last_anime_catalogue_source",
    "__APP_STATE_last_catalogue_source",
    "ext_updates_count",
    "show_not_runnable",
    "browse_hide_in_anime_library_items",
    "browse_hide_in_library_items",
    "track_on_adding_to_library",
    "show_next_episode_airing_time",
    "pref_auto_update_manga_on_mark_read",
    "backup_flags",
    "auto_clear_chapter_cache",
    "use_biometric_lock",
    "lock_app_after",
    "hide_notification_content",
    "__APP_STATE_last_app_closed",
    "verbose_logging",
    "doh_provider",
    "default_user_agent",
    "pref_display_profile_key",
    "pref_always_decode_long_strip_with_ssiv",
    "__APP_STATE_last_app_check",
    "restore_after_crash",
    "migrate_flags",
    "anime_migrate_flags",
    "incognito_extensions",
    "upcoming.week_start",
    "upcoming.home_returns_today",
    "data_saver",
    "ignore_jpeg",
    "ignore_gif",
    "data_saver_image_quality",
    "data_saver_image_format_jpeg",
    "data_saver_server",
    "data_saver_color_bw",
    "data_saver_excluded",
    "data_saver_downloader",
    "player_open_detached",
    "__APP_STATE_onboarding_complete",
    // library badges and column count, cut after their controls went (round 5)
    "display_download_badge",
    "display_unread_badge",
    "pref_animelib_columns_landscape_key",
    "pref_library_columns_landscape_key",
    // source pinning, cut for one on/off switch per source (2026-10-23)
    "pinned_anime_catalogues",
    "pinned_catalogues",
];

/// Whether `key` (as stored, without the backup prefix) belongs to a dropped feature.
pub fn is_dropped(key: &str) -> bool {
    DROPPED_KEYS.contains(&key)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKDOWN: &str =
        include_str!("../../../plans/02-preferences-system/p1-topic-key-registry.md");

    /// Rows of the plan's registry tables as (key, store, kind, default).
    fn plan_rows() -> Vec<(String, String, String, String)> {
        MARKDOWN
            .lines()
            .filter_map(|line| {
                let cells: Vec<&str> = line.split('|').map(str::trim).collect();
                if cells.len() < 8 || cells[1].parse::<u32>().is_err() {
                    return None;
                }
                let key = cells[3]
                    .trim_matches('`')
                    .replace("${tracker.id}", "{id}")
                    .replace("${id}", "{id}");
                // T-19a-064: the anime update badge count is derived in memory, never registered
                // the default reading mode is left to right (1) by the user's decision of 2026-10-11, not the plan's 2
                // adult titles are hidden by default by the user's decision of 2026-10-19, not the plan's true
                let default = match key.as_str() {
                    "pref_default_reading_mode_key" => "1",
                    "show_nsfw_source" => "false",
                    _ => cells[6],
                };
                (key != "animeext_updates_count" && !DROPPED_KEYS.contains(&key.as_str())).then(|| (
                    key,
                    cells[4].to_owned(),
                    cells[5].to_owned(),
                    default.to_owned(),
                ))
            })
            .collect()
    }

    fn kind_matches(kind: Kind, plan_kind: &str) -> bool {
        match kind {
            Kind::Bool => plan_kind == "Boolean",
            Kind::Int => plan_kind == "Int",
            Kind::Long => plan_kind == "Long",
            Kind::Float => plan_kind == "Float",
            Kind::Set => plan_kind == "Set<String>",
            Kind::Str => {
                plan_kind == "String"
                    || plan_kind.starts_with("Enum ")
                    || plan_kind.starts_with("Object ")
            }
        }
    }

    /// `Some(matches)` when the plan's default is a literal; `None` for prose (checked by hand).
    fn default_matches(lit: Lit, plan: &str) -> Option<bool> {
        if let Some(quoted) = plan.strip_prefix('"') {
            let Lit::Str(s) = lit else { return Some(false) };
            return Some(quoted.strip_suffix('"')? == s);
        }
        if plan.starts_with('{') {
            let Lit::Set(items) = lit else {
                return Some(false);
            };
            if plan.contains('<') {
                return None;
            }
            let body = plan.split('}').next()?.trim_start_matches('{');
            let mut plan_items: Vec<&str> = body
                .split(',')
                .map(|s| s.trim().trim_matches('"'))
                .filter(|s| !s.is_empty())
                .collect();
            let mut want: Vec<&str> = items.to_vec();
            plan_items.sort_unstable();
            want.sort_unstable();
            return Some(plan_items == want);
        }
        let first = plan.split_whitespace().next()?;
        let rest = &plan[first.len()..];
        match lit {
            Lit::Bool(b) => Some(first == b.to_string()),
            Lit::Int(i) => Some(first.parse::<i64>().ok() == Some(i64::from(i))),
            Lit::Long(l) => Some(first.trim_end_matches('L').parse::<i64>().ok() == Some(l)),
            Lit::Float(f) => Some((first.parse::<f64>().ok()? - f64::from(f)).abs() < 1e-6),
            Lit::Str(s) => (rest.is_empty() || rest.starts_with(" (")).then(|| first == s),
            Lit::Set(_) => None,
        }
    }

    /// Defaults the plan gives as prose, pinned to the value the registry must hold on Linux.
    const PROSE_DEFAULTS: [(&str, Lit); 3] = [
        ("source_languages", Lit::Set(&["all", "en"])),
        ("__APP_STATE_storage_dir", Lit::Str("")),
        ("pref_torrserver_tackers", Lit::Str(TORRSERVER_TRACKERS)),
    ];

    #[test]
    fn registry_has_the_59_plan_rows_in_order() {
        let rows = plan_rows();
        assert_eq!(rows.len(), 59, "the plan's rows minus the derived animeext_updates_count and the dropped (cut) plan rows");
        // four Kurobayomi rows after the plan's rows: `anime_ext_last_check` (T-19a-064), `muted_notification_channels`
        // (T-req-platform-004), `check_app_updates` (the update toggle, back from the settings diet, on by default) and
        // `last_kind` (the Anime | Manga choice every page shares), `appimage_menu_removed` (the user took the AppImage
        // out of the applications menu), `auto_hide_bars` (the reader's and the player's bars hide by themselves) and
        // `pref_player_default_quality` (the height an episode starts at), `enabled_sources` (the sources that are on) and
        // `source_keys` (which repository entry each source id came from), `cache_size_mb` (the one cache size limit),
        // `library_cover_size` (the column width of the cover grids, a slider), and the five reader settings of one manga
        // (`MangaReaderPrefs`)
        assert_eq!(REGISTRY.len(), rows.len() + 16);
        assert_eq!(REGISTRY[rows.len()].key, "anime_ext_last_check");
        assert_eq!(REGISTRY[rows.len() + 1].key, "muted_notification_channels");
        assert_eq!(REGISTRY[rows.len() + 2].key, "check_app_updates");
        assert_eq!(REGISTRY[rows.len() + 3].key, "__APP_STATE_last_kind");
        assert_eq!(REGISTRY[rows.len() + 4].key, "__APP_STATE_appimage_menu_removed");
        assert_eq!(REGISTRY[rows.len() + 5].key, "auto_hide_bars");
        assert_eq!(REGISTRY[rows.len() + 6].key, "pref_player_default_quality");
        assert_eq!(REGISTRY[rows.len() + 7].key, "enabled_sources");
        assert_eq!(REGISTRY[rows.len() + 8].key, "__APP_STATE_source_keys");
        assert_eq!(REGISTRY[rows.len() + 9].key, "cache_size_mb");
        assert_eq!(REGISTRY[rows.len() + 10].key, "library_cover_size");
        assert_eq!(REGISTRY[rows.len() + 11].key, "manga_reader_theme_{id}");
        assert_eq!(REGISTRY[rows.len() + 15].key, "manga_dual_page_split_{id}");
        let mut pinned = 0;
        for (meta, (key, store, kind, default)) in REGISTRY.iter().zip(&rows) {
            assert_eq!(meta.key, key);
            let ns = match store.as_str() {
                "normal" => Namespace::Normal,
                "app-state" => Namespace::AppState,
                "private" => Namespace::Private,
                other => panic!("unknown store {other} for {key}"),
            };
            assert_eq!(meta.ns, ns, "{key}");
            assert_eq!(Namespace::of(meta.key), ns, "{key}");
            assert!(kind_matches(meta.kind, kind), "{key}: kind {kind}");
            match default_matches(meta.default, default) {
                Some(matches) => assert!(matches, "{key}: default {default} vs {:?}", meta.default),
                None => {
                    let (_, want) = PROSE_DEFAULTS
                        .iter()
                        .find(|(k, _)| *k == meta.key)
                        .unwrap_or_else(|| panic!("{key}: prose default is not pinned"));
                    assert_eq!(meta.default, *want, "{key}");
                    pinned += 1;
                }
            }
        }
        assert_eq!(pinned, PROSE_DEFAULTS.len());
    }

    #[test]
    fn dropped_keys_are_not_registered() {
        for key in DROPPED_KEYS {
            assert!(REGISTRY.iter().all(|meta| meta.key != key), "{key} is dropped");
        }
    }

    #[test]
    fn search_terms_name_the_table_and_the_field_of_a_key() {
        assert_eq!(search_terms("pref_default_reading_mode_key"), Some(["reader", "default_reading_mode"]));
        assert_eq!(search_terms("reader_tapping_inverted"), None, "the tapping inversion was cut");
        assert_eq!(search_terms("no_such_key"), None);
    }

    #[test]
    fn keys_are_unique() {
        let mut seen = std::collections::HashMap::new();
        for meta in REGISTRY {
            *seen.entry(meta.key).or_insert(0) += 1;
        }
        for (key, count) in seen {
            assert_eq!(count, 1, "{key}");
        }
    }

    #[test]
    fn lookup_finds_exact_and_family_keys() {
        let (meta, id) = lookup("crop_borders").unwrap();
        assert_eq!(meta.key, "crop_borders");
        assert_eq!(id, None);
        let (meta, id) = lookup("pref_filter_library_tracked_7_v2").unwrap();
        assert_eq!(meta.key, "pref_filter_library_tracked_{id}_v2");
        assert_eq!(id, Some("7"));
        let (meta, id) = lookup("__PRIVATE_pref_tracker_auth_expired_3").unwrap();
        assert_eq!(meta.table, "tracking.auth_expired");
        assert_eq!(id, Some("3"));
    }

    #[test]
    fn lookup_rejects_unknown_and_empty_family_ids() {
        assert!(lookup("no_such_key").is_none());
        assert!(lookup("pref_filter_library_tracked__v2").is_none());
        assert!(lookup("pref_filter_library_tracked_1_v2_extra").is_none());
    }

    #[test]
    fn literal_helpers_cover_each_kind() {
        assert_eq!(Lit::Long(5).value(), PrefValue::Long(5));
        assert_eq!(
            Lit::Set(&["wifi"]).value(),
            PrefValue::Set(BTreeSet::from(["wifi".to_owned()]))
        );
        assert_eq!(
            default_matches(Lit::Str("DISABLED"), "DISABLED"),
            Some(true)
        );
        assert_eq!(
            default_matches(
                Lit::Str("PACKAGEINSTALLER"),
                "PACKAGEINSTALLER (LEGACY on MIUI)"
            ),
            Some(true)
        );
        assert_eq!(
            default_matches(
                Lit::Str("x"),
                "MONET if dynamic colour available else DEFAULT"
            ),
            None
        );
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn a_table_dot_field_path_finds_its_row() {
        assert_eq!(
            find_path("library.display_mode").map(|m| m.key),
            Some("pref_display_mode_library")
        );
        assert_eq!(
            find_path("reader.default_reading_mode").map(|m| m.key),
            Some("pref_default_reading_mode_key")
        );
        assert!(find_path("player.gestures.gesture_horizontal_seek").is_none(), "the gesture was cut");
        assert!(find_path("library.no_such_field").is_none());
        assert!(find_path("no_dot").is_none());
    }
}
