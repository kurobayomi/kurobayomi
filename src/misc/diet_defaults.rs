//! Hard-coded values of the preferences the settings diet cut (docs/design/settings-diet.md, column "If CUT: hard-code").
//! Every reader of a cut preference uses these constants instead of the store, so the stored key can go. Values that
//! are enums are written at the call site, next to the type they name.

// Reader (pager, long strip and the page-view overlay).
pub const READER_PAGE_TRANSITIONS: bool = true;
pub const READER_DOUBLE_TAP_ANIM_MS: u32 = 500;
pub const READER_LONG_TAP: bool = true;
pub const READER_VOLUME_KEYS: bool = false;
pub const READER_VOLUME_KEYS_INVERTED: bool = false;
pub const READER_ALWAYS_SHOW_CHAPTER_TRANSITION: bool = true;
pub const READER_OVERLAY_ON_START: bool = false;
pub const READER_OVERLAY_NEW_USER: bool = true;
/// Tap-zone layout code of both modes: the default layout, which follows the reading direction.
pub const READER_NAVIGATION_MODE: i32 = 0;
pub const READER_NAVIGATE_PAN: bool = true;
pub const READER_LANDSCAPE_ZOOM: bool = true;
pub const READER_WEBTOON_SIDE_PADDING: i32 = 0;
pub const READER_WEBTOON_DISABLE_ZOOM_OUT: bool = false;
pub const READER_WEBTOON_DOUBLE_TAP_ZOOM: bool = true;
pub const READER_DUAL_PAGE_INVERT: bool = false;
pub const READER_DUAL_PAGE_ROTATE: bool = false;
pub const READER_DUAL_PAGE_ROTATE_INVERT: bool = false;
pub const READER_WEBTOON_DUAL_PAGE_SPLIT: bool = false;
pub const READER_WEBTOON_DUAL_PAGE_INVERT: bool = false;
pub const READER_WEBTOON_DUAL_PAGE_ROTATE: bool = false;
pub const READER_WEBTOON_DUAL_PAGE_ROTATE_INVERT: bool = false;
pub const READER_SHOW_READING_MODE: bool = true;
pub const READER_FULLSCREEN: bool = false;
pub const READER_KEEP_SCREEN_ON: bool = true;
pub const READER_FLASH_ON_PAGE_CHANGE: bool = false;
pub const READER_SKIP_READ: bool = false;
pub const READER_SKIP_FILTERED: bool = true;
pub const READER_SKIP_DUPE: bool = false;
pub const READER_PRESERVE_READING_POSITION: bool = false;
pub const READER_CREATE_FOLDER_PER_MANGA: bool = false;
pub const READER_RESTORE_AFTER_CRASH: bool = true;
/// `pref_hardware_bitmap_threshold`: the largest image tile edge in pixels.
pub const READER_TILE_THRESHOLD: i32 = 4096;
/// `pref_display_profile_key`: no colour profile file.
pub const READER_DISPLAY_PROFILE: &str = "";
pub const READER_COLOUR_FILTER: bool = false;
pub const READER_GRAYSCALE: bool = false;
pub const READER_INVERTED_COLOURS: bool = false;
pub const READER_CUSTOM_BRIGHTNESS: bool = false;

// Library and its update jobs.
pub const LIBRARY_DOWNLOAD_BADGE: bool = false;
pub const LIBRARY_UNREAD_BADGE: bool = true;
pub const LIBRARY_LOCAL_BADGE: bool = true;
pub const LIBRARY_LANGUAGE_BADGE: bool = false;
pub const LIBRARY_CONTINUE_BUTTON: bool = false;
pub const LIBRARY_CATEGORY_TABS: bool = true;
pub const LIBRARY_ITEM_COUNT: bool = false;
/// The list size of the library List mode, at every window width: 0 is automatic. (The grids follow the Cover size setting.)
pub const LIBRARY_COLUMNS: i32 = 0;
pub const LIBRARY_CATEGORISED_DISPLAY: bool = false;
pub const LIBRARY_HIDE_HIDDEN_CATEGORIES: bool = false;
pub const LIBRARY_UPDATES_COUNT: bool = true;
pub const LIBRARY_AUTO_UPDATE_METADATA: bool = false;
pub const LIBRARY_UPDATE_SEASON: bool = false;
pub const LIBRARY_MARK_DUPLICATE_CHAPTER_READ: bool = false;
pub const LIBRARY_MARK_DUPLICATE_EPISODE_SEEN: bool = false;
pub const LIBRARY_RELATIVE_TIME: bool = true;
pub const LIBRARY_DATE_FORMAT: &str = "";
pub const LIBRARY_AUTO_CLEAR_CHAPTER_CACHE: bool = false;
pub const LIBRARY_UPCOMING_HOME_RETURNS_TODAY: bool = false;
/// The Upcoming week start: `auto`, so the locale decides.
pub const UPCOMING_WEEK_START: &str = "auto";
/// The item restrictions of the library update (Smart update is cut): only completed series are skipped, so entries with
/// unread items, entries not started and entries outside their release window are checked too.
pub const LIBRARY_UPDATE_ITEM_RESTRICTIONS: [&str; 1] = ["manga_ongoing"];

// Entry pages (anime and manga details, seasons, the track dialog).
pub const ENTRY_SHOW_AIRING_TIME: bool = true;
pub const ENTRY_TRACK_ON_ADDING: bool = true;

// Browse and migration.
pub const BROWSE_HIDE_IN_LIBRARY: bool = false;
pub const MIGRATE_FLAGS_ALL: i32 = i32::MAX;
/// The last source opened in Browse is not remembered (the setting is cut): -1 is "none".
pub const BROWSE_LAST_USED_SOURCE: i64 = -1;

// Downloads (only the basic settings stay; these are the hard-coded rest).
pub const DOWNLOAD_USE_EXTERNAL: bool = false;
pub const DOWNLOAD_NEW_UNREAD_ONLY: bool = false;
pub const DOWNLOAD_REMOVE_AFTER_MARKED_READ: bool = false;
/// How many units are queued ahead of a downloaded one while it is read or watched, past a quarter of it (one rule, both kinds).
pub const DOWNLOAD_AUTO_WHILE_AMOUNT: i32 = 1;

// Player. The external player is cut for now, so the always-external switch is off; the one-off "play in external
// player" action stays.
pub const PLAYER_ALWAYS_EXTERNAL: bool = false;

// Shell and notifications.
pub const NOTIFICATION_HIDE_CONTENT: bool = false;
