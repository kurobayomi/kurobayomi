//! The full-size cover and background viewer, as a model the UI draws (T-34a-053).
//!
//! Manga shows one page, the cover. Anime shows two: the cover, then the background. The viewer requests the
//! original size and never reads or writes the memory cache.

use kuro_platform::kind::Kind;
use std::time::Duration;

use crate::decode::{DecodeOptions, Decoded, Frame, decode_frames};
use crate::error::ImageError;
use crate::options::{CachePolicy, LoadOptions, RequestSize};

/// Length of the zoom animation when a page opens, in milliseconds.
pub const ZOOM_MS: u64 = 500;

/// How many pages the viewer has.
pub fn page_count(kind: Kind) -> usize {
    match kind {
        Kind::Manga => 1,
        Kind::Anime => 2,
    }
}

/// The page the switch button moves to: anime toggles between cover and background, manga has none (behavior
/// section 26.1, `1 - currentPage`).
pub fn switch_page(kind: Kind, page: usize) -> usize {
    match kind {
        Kind::Manga => 0,
        Kind::Anime => usize::from(page == 0),
    }
}

/// Request options of one page: original size, the background for page 1 of an anime entry, and the memory cache
/// disabled.
pub fn page_options(kind: Kind, page: usize) -> LoadOptions {
    LoadOptions {
        use_background: kind == Kind::Anime && page == 1,
        size: RequestSize::Original,
        memory_cache: CachePolicy::Disabled,
        ..LoadOptions::default()
    }
}

/// What the edit button does: with a custom image it opens a menu with Edit and
/// Delete, otherwise it opens the file picker straight away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditButton {
    Picker,
    Menu,
}

pub fn edit_button(has_custom: bool) -> EditButton {
    if has_custom {
        EditButton::Menu
    } else {
        EditButton::Picker
    }
}

/// Frames that ask for less than this play at [`MIN_DELAY_MS_FALLBACK`] instead, as browsers play GIFs: a zero or
/// near-zero delay would otherwise spin the main loop.
pub const MIN_DELAY_MS: u32 = 20;
pub const MIN_DELAY_MS_FALLBACK: u32 = 100;

/// One page of the full-cover viewer as the UI draws it: every frame with its delay (T-34a-033). A GIF or WebP with
/// several frames plays as a loop; every other picture is one frame. Grids never use this: they decode the first
/// frame through `decode`.
#[derive(Clone, Debug)]
pub struct ViewerPage {
    frames: Vec<Frame>,
    current: usize,
}

impl ViewerPage {
    /// Decodes every frame of `bytes`.
    pub fn from_bytes(bytes: &[u8], opts: &DecodeOptions) -> Result<Self, ImageError> {
        Self::from_frames(decode_frames(bytes, opts)?).ok_or(ImageError::DecodeFailed)
    }

    /// `None` for an empty list: there is nothing to show.
    pub fn from_frames(frames: Vec<Frame>) -> Option<Self> {
        (!frames.is_empty()).then_some(Self { frames, current: 0 })
    }

    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Index of the frame on screen.
    pub fn current_index(&self) -> usize {
        self.current
    }

    /// The picture to draw now.
    pub fn current(&self) -> &Decoded {
        &self.frames[self.current].picture
    }

    /// How long the current frame stays, or `None` for a still picture: nothing to schedule.
    pub fn current_delay(&self) -> Option<Duration> {
        if !self.is_animated() {
            return None;
        }
        let ms = self.frames[self.current].delay_ms;
        let ms = if ms < MIN_DELAY_MS { MIN_DELAY_MS_FALLBACK } else { ms };
        Some(Duration::from_millis(u64::from(ms)))
    }

    /// Moves to the next frame, wrapping to the first after the last (the loop count is not honoured: the viewer
    /// loops while it is open). Returns the new index. Call it when the timer of `current_delay` fires.
    pub fn advance(&mut self) -> usize {
        self.current = (self.current + 1) % self.frames.len();
        self.current
    }

    /// Back to the first frame, for when the page is shown again.
    pub fn rewind(&mut self) {
        self.current = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manga_has_one_page_and_anime_has_two() {
        assert_eq!(page_count(Kind::Manga), 1);
        assert_eq!(page_count(Kind::Anime), 2);
    }

    #[test]
    fn switch_toggles_anime_pages_only() {
        assert_eq!(switch_page(Kind::Anime, 0), 1);
        assert_eq!(switch_page(Kind::Anime, 1), 0);
        assert_eq!(switch_page(Kind::Manga, 0), 0);
    }

    #[test]
    fn page_one_of_anime_asks_for_the_background() {
        assert!(!page_options(Kind::Anime, 0).use_background);
        assert!(page_options(Kind::Anime, 1).use_background);
        assert!(!page_options(Kind::Manga, 0).use_background);
    }

    #[test]
    fn viewer_pages_skip_the_memory_cache_and_use_original_size() {
        for kind in [Kind::Manga, Kind::Anime] {
            for page in 0..page_count(kind) {
                let opts = page_options(kind, page);
                assert_eq!(opts.memory_cache, CachePolicy::Disabled);
                assert_eq!(opts.size, RequestSize::Original);
            }
        }
    }

    #[test]
    fn edit_button_opens_a_menu_only_for_a_custom_image() {
        assert_eq!(edit_button(true), EditButton::Menu);
        assert_eq!(edit_button(false), EditButton::Picker);
    }

    fn frame(shade: u8, delay_ms: u32) -> Frame {
        Frame {
            picture: Decoded {
                width: 1,
                height: 1,
                rgba: bytes::Bytes::from(vec![shade, shade, shade, 255]),
                is_sampled: false,
            },
            delay_ms,
        }
    }

    #[test]
    fn a_still_page_has_nothing_to_schedule() {
        let page = ViewerPage::from_frames(vec![frame(1, 0)]).unwrap();
        assert!(!page.is_animated());
        assert_eq!(page.current_delay(), None);
    }

    #[test]
    fn an_animated_page_walks_its_frames_with_their_delays_and_loops() {
        let mut page = ViewerPage::from_frames(vec![frame(1, 40), frame(2, 250), frame(3, 60)]).unwrap();
        assert!(page.is_animated());
        assert_eq!(page.frame_count(), 3);
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push((page.current().rgba[0], page.current_delay().unwrap().as_millis()));
            page.advance();
        }
        assert_eq!(seen, [(1, 40), (2, 250), (3, 60), (1, 40)]);
        page.rewind();
        assert_eq!(page.current_index(), 0);
    }

    #[test]
    fn a_tiny_delay_plays_at_100_ms() {
        let page = ViewerPage::from_frames(vec![frame(1, 0), frame(2, 19)]).unwrap();
        assert_eq!(page.current_delay(), Some(Duration::from_millis(100)));
        let at_threshold = ViewerPage::from_frames(vec![frame(1, 20), frame(2, 20)]).unwrap();
        assert_eq!(at_threshold.current_delay(), Some(Duration::from_millis(20)));
    }

    #[test]
    fn an_animated_gif_gives_every_frame_and_a_png_gives_one() {
        use crate::testing::{encode, gradient};
        use image::{Delay, Frame as ImgFrame, RgbaImage};
        let mut bytes = Vec::new();
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut bytes);
            for shade in [10u8, 200] {
                let img = RgbaImage::from_pixel(4, 4, image::Rgba([shade, 0, 0, 255]));
                enc.encode_frame(ImgFrame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(120, 1))).unwrap();
            }
        }
        let page = ViewerPage::from_bytes(&bytes, &DecodeOptions::default()).unwrap();
        assert_eq!(page.frame_count(), 2);
        assert_eq!(page.current_delay(), Some(Duration::from_millis(120)));
        let png = encode(&gradient(8, 8), image::ImageFormat::Png);
        assert!(!ViewerPage::from_bytes(&png, &DecodeOptions::default()).unwrap().is_animated());
    }
}
