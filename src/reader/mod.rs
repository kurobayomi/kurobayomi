//! The reader (area 43). Pure models and loaders are GTK-free; the widget builders are `transition` and `chrome`.
//!
//! `model` holds the chapter, page and transition types. `flags` holds mode and crop state. `image`
//! and `cache` hold the page rules and the disk cache. Texts are `tr(MsgKey::...)` at the call site, as in the rest of
//! the app. `loader` holds the page loaders and the chapter state
//! machine. `chapters` builds the reader list. `session` runs the reader state. `launch` opens the reader for the app. `screen` is the reader page: open, failure card, chapter buttons, crop. `writer` is the database writer.
//!
//! Android items with no Linux port (T-43b-050 to T-43b-052, T-44a-017, T-44a-061, T-44a-072, T-44a-073):
//! - The display cutout setting is never offered. Its key stays in the store, and the
//!   general tab says it is not used on Linux.
//! - Hardware bitmap config, `BitmapRegionDecoder` and the SDK gates: none. `pref_hardware_bitmap_threshold` is the tile
//!   threshold (`tiles`).
//! - "Always decode the long strip with SSIV" is dropped: a webtoon page decodes at the view width, and a stored value is ignored.
//! - DirectionalViewPager is replaced by the pager of `viewers::pager`; no visible loss.
//! - The webtoon frame's touch translation, the touch-ignoring tile view and the ViewPager exception guards: not needed, the
//!   widgets keep the behaviour.
//! Replacements for the Android view APIs in scope (T-43b-052): AbstractComposeView -> plain GTK widgets; Handler.postDelayed
//! -> `glib::timeout_add_local_once` (`page_view`); ViewConfiguration -> the constants of `long_press`; MaterialButton ->
//! `gtk::Button`; Canvas.drawRect -> `gtk::Snapshot` (`blend_box`); FLAG_KEEP_SCREEN_ON -> the platform idle inhibitor (`keep_awake`).
//!
//! Gap list G-01 to G-35, the ones that were open at the review (T-43a-078):
//! - G-27 navigation overlay labels with fade: done in `nav_overlay` (zones, colours, labels, 1000 ms fades, shown by the
//!   on-start and new-user preferences and by a changed tap-zone layout). The tap zones themselves are area 44 (`viewers::nav`).
//! - G-34 the "Continuous vertical" label: `tr(MsgKey::vertical_plus_viewer)`.
//! - G-35 Open in WebView: assigned to area 36 (T-36a-053, T-36a-054, `webkit6` window in `src/web/window.rs`). DEFERRED, not done
//!   (T-43a-059): the in-app WebView with the source id and the manga title does not exist yet, so the reader keeps the entry in
//!   the overflow menu and opens the chapter in the desktop browser until that window exists. Open in browser and Share are done.
//! - T-43a-067 display ICC profile: cut. Its setting was cut and the build never enabled the `icc` feature, so the
//!   reader's own profile step was removed (2026-11-02); `kuro_images::icc` keeps the shared support.
//! - T-43a-030 and T-43a-034 are connected: `launch` gives the session the tracker progress sink, the download-ahead sink
//!   and the delete-after-read sink (`ReaderExtras`).
//! - Online chapters read through the source registry (`source_pages`); a source that is not reachable is
//!   "Source not found" (the message does not name the source yet).
//!
//! Decisions recorded here so they are not mistaken for gaps:
//! - Volume keys (T-43a-071 against T-44a-015): the paging stays, as the opt-in of T-44a-015. `reader_volume_keys` is off by
//!   default, the keys are claimed only while it is on and the menu is hidden (`viewers::keys`), and the preference keys are
//!   kept. T-43a-071 (P3, drop) is read as "not used by default"; the plan text of T-43a-071 should be amended to say so.
//! - The reader settings live in the top bar's menu (`chrome::overflow_menu`), the open manga's own values, written at
//!   once so the reader follows them live; Settings > Reader holds the defaults. There is no reader settings window.
//! - The reader's top bar is the shared header of `widgets::page_header`, the same design as the player's. Pages open at the
//!   left-to-right default; right to left is a per-manga choice that the reader remembers (picking the mode from the content
//!   is not done yet). The page slider sits at the bottom.
//! - A chapter whose page list is still being fetched shows the spinner and is asked for again (`source_pages::is_pending`);
//!   it is not an error any more.

pub mod backlight;
pub mod blend_box;
pub mod cache;
pub mod decode_guard;
pub mod decoded_lru;
pub mod delete_queue;
pub mod download_ahead;
pub mod error_card;
pub mod chapters;
pub mod chapter_panel;
pub mod chrome;
#[cfg(test)]
mod chrome_tests;
pub mod flags;
#[cfg(test)]
pub mod golden;
pub mod history_rules;
pub mod image;
pub mod keep_awake;
pub mod launch;
pub mod loader;
pub mod long_press;
pub mod model;
pub mod nav;
pub mod nav_overlay;
pub mod navigator;
pub mod overlay;
pub mod page_actions;
pub mod page_decode;
pub mod page_save;
pub mod page_transform;
pub mod page_view;
mod topbar_guard;
pub mod page_zoom;
pub mod pref_rules;
pub mod progress;
pub mod restore;
pub mod saved_notice;
pub mod quick_scale;
pub mod rules;
pub mod screen;
pub mod session;
pub mod source_pages;
pub mod sheet;
pub mod state;
pub mod sync;
pub mod theme;
pub mod tiles;
pub mod transition;
pub mod viewer;
pub mod viewers;
pub mod writer;

