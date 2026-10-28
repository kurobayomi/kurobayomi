//! The reader (area 43). Pure models and loaders are GTK-free; the widget builders are `transition` and `chrome`.
//!
//! `model` holds the chapter, page and transition types. `flags` holds mode and crop state. `image`
//! and `cache` hold the page rules and the disk cache. `loader` holds the page loaders and the chapter state
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
//! Gap list G-01 to G-35 (`plans/43-reader-core/p1-ui-parity.md` section 12), the ones that were open at the review (T-43a-078):
//! - G-27 navigation overlay labels with fade: done in `nav_overlay` (zones, colours, labels, 1000 ms fades, shown by the
//!   on-start and new-user preferences and by a changed tap-zone layout). The tap zones themselves are area 44 (`viewers::nav`).
//! - G-34 the "Continuous vertical" label: `strings::vertical_plus_viewer()`.
//! - G-35 Open in WebView: assigned to area 36 (T-36a-053, T-36a-054, `webkit6` window in `src/web/window.rs`). DEFERRED, not done
//!   (T-43a-059): the in-app WebView with the source id and the manga title does not exist yet, so the reader keeps the entry in
//!   the overflow menu and opens the chapter in the desktop browser until that window exists. Open in browser and Share are done.
//! - T-43a-067 display ICC profile: `page_profile` reads `pref_display_profile_key` and takes decoded pages to it (lcms2 through
//!   `kuro_images::icc`); a missing or unreadable file means no transform and a logged warning. The app build does not enable
//!   the `icc` feature of `kuro-images`, so no profile is applied yet: display colour management is lost until it is on.
//! - T-43a-030 and T-43a-034 are in the session (`ReaderExtras::track_update`, `ReaderExtras::download_ahead`); the app starts no
//!   tracker layer or download manager yet, so `launch` gives neither sink and nothing is sent or queued until it does.
//! - Real page data for an online chapter waits for the source registry (area 17); `launch` documents the dependency and
//!   reports "Source not found" until the registry hands the loader a source.
//!
//! Decisions recorded here so they are not mistaken for gaps:
//! - Volume keys (T-43a-071 against T-44a-015): the paging stays, as the opt-in of T-44a-015. `reader_volume_keys` is off by
//!   default, the keys are claimed only while it is on and the menu is hidden (`viewers::keys`), and the preference keys are
//!   kept. T-43a-071 (P3, drop) is read as "not used by default"; the plan text of T-43a-071 should be amended to say so.
//! - The reader settings window (`settings_window`) is a native modal window whose page is built from the shared pref_model
//!   rows (`settings_ui::reader::page::layout_rows`), not from the spec items of `src/spec.rs`: the spec `Item` has no preference
//!   key. Each row writes its key straight away. The settings diet kept six reader settings and cut the dependent rows, so no row is disabled by a rule.
//! - The reader's top bar is the shared header of `widgets::page_header`, the same design as the player's. Pages open at the
//!   left-to-right default; right to left is a per-manga choice that the reader remembers. The page slider sits at the bottom.
//! - A chapter whose page list is still being fetched shows the spinner and is asked for again (`source_pages::is_pending`);
//!   it is not an error any more.
#![allow(dead_code, unused_imports)] // the viewers (area 44) and the source registry (area 17) attach later

pub mod backlight;
pub mod blend_box;
pub mod cache;
pub mod crop_borders;
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
pub mod page_profile;
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
pub mod settings_model;
pub mod settings_window;
pub mod sheet;
pub mod state;
pub mod strings;
#[cfg(test)]
mod strings_tests;
pub mod sync;
pub mod theme;
pub mod tiles;
pub mod transition;
pub mod viewer;
pub mod viewers;
pub mod writer;

pub use screen::{ReaderChrome, ReaderDeps, ReaderExtras, ReaderScreen, open, open_with};
