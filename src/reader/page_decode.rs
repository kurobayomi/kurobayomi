//! Page decode (T-43b-014, T-44a-050, decode half). The decode itself is pure (the `image` crate, no GTK) and runs on the
//! blocking pool. The page view cuts the RGBA into a tile pyramid (`tiles`) and turns the visible tiles into
//! `gdk::Texture`s on the GTK thread.
//!
//! `needs_tiling` is the predicate for a page too large for one texture; the pyramid itself is built for every page
//! and has one level when the page fits.

/// The hardware bitmap threshold default (`GLUtil.SAFE_TEXTURE_LIMIT`).
pub const DEFAULT_TILE_THRESHOLD: u32 = 2048;

/// The largest page edge that is kept as decoded (D-9, T-43b-021). Pages up to this size upload without a downscale
/// (as tiles of at most [`DEFAULT_TILE_THRESHOLD`]); a longer edge is shrunk to this by [`limit_edge`].
pub const MAX_TEXTURE_EDGE: u32 = 16384;

/// Why a page could not be decoded (E-07).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The bytes start like an image but do not decode, or are empty.
    Corrupt,
    /// The bytes are not a format this reader knows.
    UnsupportedFormat,
}

/// Where a page is in its load. Loading hides the page and shows the indicator; Error emits `image-load-error`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoadState {
    #[default]
    Empty,
    Loading,
    Ready,
    Error,
}

/// A decoded page: straight (not premultiplied) RGBA8, rows of `width * 4` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedPage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Whether a page this large must be tiled rather than uploaded whole (E-06): the longer edge exceeds the threshold.
pub fn needs_tiling(width: u32, height: u32, threshold: u32) -> bool {
    width.max(height) > threshold
}

/// Shrinks a page to `width` pixels wide, keeping its aspect ratio, when it is wider than that. A narrower page is never
/// enlarged (B§8.4: webtoon pages decode at the view width). A page that cannot be rebuilt comes back empty, which the
/// page view shows as a load error.
pub fn fit_to_width(page: DecodedPage, width: u32) -> DecodedPage {
    if width == 0 || page.width <= width {
        return page;
    }
    let height = (u64::from(page.height) * u64::from(width) / u64::from(page.width)).max(1) as u32;
    let DecodedPage { width: src_w, height: src_h, rgba } = page;
    match image::RgbaImage::from_raw(src_w, src_h, rgba) {
        Some(img) => {
            let out = image::imageops::resize(&img, width, height, image::imageops::FilterType::Triangle);
            DecodedPage { width: out.width(), height: out.height(), rgba: out.into_raw() }
        }
        None => DecodedPage { width: 0, height: 0, rgba: Vec::new() },
    }
}

/// Shrinks a page whose longer edge is above `max_edge` so that edge is `max_edge`, keeping the aspect ratio (D-9). A page
/// within the limit is returned as it is. A page that cannot be rebuilt comes back empty, which the page view shows as a
/// load error.
pub fn limit_edge(page: DecodedPage, max_edge: u32) -> DecodedPage {
    let longest = page.width.max(page.height);
    if max_edge == 0 || longest <= max_edge {
        return page;
    }
    let shrink = |edge: u32| (u64::from(edge) * u64::from(max_edge) / u64::from(longest)).max(1) as u32;
    let (width, height) = (shrink(page.width), shrink(page.height));
    let DecodedPage { width: src_w, height: src_h, rgba } = page;
    match image::RgbaImage::from_raw(src_w, src_h, rgba) {
        Some(img) => {
            let out = image::imageops::resize(&img, width, height, image::imageops::FilterType::Triangle);
            DecodedPage { width: out.width(), height: out.height(), rgba: out.into_raw() }
        }
        None => DecodedPage { width: 0, height: 0, rgba: Vec::new() },
    }
}

/// One frame of a page: its pixels and how long it stays on screen, in milliseconds (0 for a still image).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub page: DecodedPage,
    pub delay_ms: u32,
}

/// The frames a player keeps of one animation. A longer animation plays its first frames only (a bound on memory).
pub const MAX_FRAMES: usize = 300;
/// The shortest frame delay a player shows, in milliseconds (E-46).
pub const MIN_DELAY_MS: u32 = 10;

/// The frames of a page (T-44a-051), through the app's one decoder (`kuro_images::decode`: every raster format, JPEG XL,
/// HEIF and AVIF, turned by its EXIF orientation). An animated GIF or WebP gives every frame with its delay, clamped to
/// [`MIN_DELAY_MS`], at most [`MAX_FRAMES`]. Anything else, and a single-frame animation, is one frame with no delay.
/// `crop` cuts uniform borders off (the shared rule, `kuro_images::decode`).
pub fn decode_frames(bytes: &[u8], crop: bool) -> Result<Vec<Frame>, DecodeError> {
    if bytes.is_empty() {
        return Err(DecodeError::Corrupt);
    }
    let frames = kuro_images::decode::decode_frames(bytes, &page_options(crop)).map_err(decode_error)?;
    let animated = frames.len() > 1;
    Ok(frames
        .into_iter()
        .take(MAX_FRAMES)
        .map(|frame| Frame { page: page_of(frame.picture), delay_ms: if animated { frame.delay_ms.max(MIN_DELAY_MS) } else { 0 } })
        .collect())
}

/// The decode options of a reader page: the original size, up to the page pixel limit, borders cut off when `crop`.
fn page_options(crop: bool) -> kuro_images::DecodeOptions {
    kuro_images::DecodeOptions { max_pixels: kuro_images::decode::PAGE_MAX_PIXELS, crop_borders: crop, ..kuro_images::DecodeOptions::default() }
}

fn page_of(picture: kuro_images::Decoded) -> DecodedPage {
    DecodedPage { width: picture.width, height: picture.height, rgba: Vec::from(picture.rgba) }
}

/// The reader's two kinds of failure: bytes in no format the decoder knows, and anything else that did not decode.
fn decode_error(error: kuro_images::ImageError) -> DecodeError {
    match error {
        kuro_images::ImageError::UnsupportedFormat | kuro_images::ImageError::InvalidImage => DecodeError::UnsupportedFormat,
        _ => DecodeError::Corrupt,
    }
}

/// How many times an animation plays through, read from the file itself because the image crate does not expose it
/// (T-44a-051). `None` loops forever. A GIF carries the NETSCAPE2.0 repeat count (N repeats after the first play, 0 is
/// forever, no extension is one play). A WebP carries the ANIM loop count (N plays, 0 is forever).
pub fn loop_passes(bytes: &[u8]) -> Option<u32> {
    match image::guess_format(bytes) {
        Ok(image::ImageFormat::Gif) => {
            const TAG: &[u8] = b"NETSCAPE2.0";
            let Some(at) = bytes.windows(TAG.len()).position(|w| w == TAG) else { return Some(1) };
            match bytes.get(at + TAG.len()..at + TAG.len() + 4) {
                Some([3, 1, lo, hi]) => match u16::from_le_bytes([*lo, *hi]) {
                    0 => None,
                    n => Some(u32::from(n) + 1),
                },
                _ => Some(1),
            }
        }
        Ok(image::ImageFormat::WebP) => {
            let mut at = 12;
            while let Some(head) = bytes.get(at..at + 8) {
                let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
                if &head[..4] == b"ANIM" {
                    let loops = bytes.get(at + 12..at + 14).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
                    return if loops == 0 { None } else { Some(u32::from(loops)) };
                }
                at += 8 + size + (size & 1);
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
/// Decodes page bytes to RGBA8 (the first frame of an animation). An empty input is `Corrupt`. Bytes in no known image
/// format are `UnsupportedFormat`. Bytes of a known format that fail to decode, including a truncated file, are `Corrupt`.
pub fn decode_rgba(bytes: &[u8]) -> Result<DecodedPage, DecodeError> {
    if bytes.is_empty() {
        return Err(DecodeError::Corrupt);
    }
    kuro_images::decode::decode(bytes, &page_options(false)).map(page_of).map_err(decode_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn encoded(format: image::ImageFormat, width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut out, format).expect("encode");
        out.into_inner()
    }

    #[test]
    fn e06_tiling_boundary_is_strictly_above_the_threshold() {
        assert!(!needs_tiling(2048, 100, DEFAULT_TILE_THRESHOLD));
        assert!(needs_tiling(2049, 100, DEFAULT_TILE_THRESHOLD));
        assert!(needs_tiling(100, 2049, DEFAULT_TILE_THRESHOLD));
    }

    #[test]
    fn a_png_decodes_to_rgba_with_its_size() {
        let page = decode_rgba(&encoded(image::ImageFormat::Png, 3, 2)).expect("png");
        assert_eq!((page.width, page.height), (3, 2));
        assert_eq!(page.rgba.len(), 3 * 2 * 4);
        assert_eq!(&page.rgba[0..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn e07_a_truncated_jpeg_is_corrupt_not_a_panic() {
        let jpeg = encoded(image::ImageFormat::Jpeg, 16, 16);
        // Keep only the signature and a few header bytes.
        assert_eq!(decode_rgba(&jpeg[..4]), Err(DecodeError::Corrupt));
    }

    #[test]
    fn e07_unknown_bytes_are_unsupported_and_empty_input_is_corrupt() {
        assert_eq!(decode_rgba(b"this is plain text, not an image"), Err(DecodeError::UnsupportedFormat));
        assert_eq!(decode_rgba(&[]), Err(DecodeError::Corrupt));
    }

    #[test]
    fn a_wide_page_is_shrunk_to_the_view_width_and_a_narrow_one_is_kept() {
        let wide = DecodedPage { width: 400, height: 200, rgba: vec![7; 400 * 200 * 4] };
        let shrunk = fit_to_width(wide, 200);
        assert_eq!((shrunk.width, shrunk.height), (200, 100));
        assert_eq!(shrunk.rgba.len(), 200 * 100 * 4);
        let narrow = DecodedPage { width: 100, height: 50, rgba: vec![7; 100 * 50 * 4] };
        let kept = fit_to_width(narrow, 800);
        assert_eq!((kept.width, kept.height), (100, 50), "never enlarged");
        let zero_view = DecodedPage { width: 10, height: 10, rgba: vec![0; 400] };
        assert_eq!(fit_to_width(zero_view, 0).width, 10, "no view width, no change");
    }

    #[test]
    fn a_page_up_to_the_texture_limit_is_kept_and_a_longer_one_is_shrunk_to_it() {
        let at_limit = DecodedPage { width: MAX_TEXTURE_EDGE, height: 2, rgba: vec![9; MAX_TEXTURE_EDGE as usize * 2 * 4] };
        let kept = limit_edge(at_limit, MAX_TEXTURE_EDGE);
        assert_eq!((kept.width, kept.height), (MAX_TEXTURE_EDGE, 2), "16384 px uploads as it is");
        let over = DecodedPage { width: 4, height: 40, rgba: vec![9; 4 * 40 * 4] };
        let shrunk = limit_edge(over, 20);
        assert_eq!((shrunk.width, shrunk.height), (2, 20), "the long edge is the limit and the ratio stays");
        assert_eq!(shrunk.rgba.len(), 2 * 20 * 4);
        let thin = DecodedPage { width: 1, height: 100, rgba: vec![9; 400] };
        assert_eq!(limit_edge(thin, 10).width, 1, "a side never shrinks to nothing");
        let wide = DecodedPage { width: 40, height: 4, rgba: vec![9; 40 * 4 * 4] };
        assert_eq!(limit_edge(wide, 0).width, 40, "no limit, no change");
    }

    #[test]
    fn the_texture_limit_is_the_d9_value_above_the_tile_threshold() {
        assert_eq!(MAX_TEXTURE_EDGE, 16384);
        assert!(needs_tiling(MAX_TEXTURE_EDGE, 10, DEFAULT_TILE_THRESHOLD), "a page at the limit is tiled, not downscaled");
    }

    #[test]
    fn t051_an_animated_gif_gives_every_frame_with_its_delay_and_a_delay_under_ten_ms_is_ten() {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            let red = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
            let blue = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 255, 255]));
            encoder
                .encode_frames(vec![
                    image::Frame::from_parts(red, 0, 0, image::Delay::from_numer_denom_ms(50, 1)),
                    image::Frame::from_parts(blue, 0, 0, image::Delay::from_numer_denom_ms(1, 1)),
                ])
                .expect("encode");
        }
        let frames = decode_frames(&bytes, false).expect("gif");
        assert_eq!(frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>(), vec![50, MIN_DELAY_MS]);
        assert_eq!((frames[0].page.width, frames[0].page.height), (4, 4));
        assert!(frames[0].page.rgba[0] > 200 && frames[0].page.rgba[1] < 50, "the first frame is the red one");
    }

    fn gif_with_loop(extension: Option<u16>) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut bytes);
            let _ = enc.set_repeat(match extension {
                Some(0) => image::codecs::gif::Repeat::Infinite,
                Some(n) => image::codecs::gif::Repeat::Finite(n),
                None => image::codecs::gif::Repeat::Finite(0),
            });
            let frame = image::Frame::new(image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255])));
            enc.encode_frames(vec![frame.clone(), frame]).expect("gif");
        }
        bytes
    }

    #[test]
    fn t051_a_gif_loop_count_is_read_from_the_netscape_extension() {
        assert_eq!(loop_passes(&gif_with_loop(Some(0))), None, "0 is forever");
        assert_eq!(loop_passes(&gif_with_loop(Some(2))), Some(3), "two repeats after the first play");
        let mut no_extension = gif_with_loop(Some(0));
        let at = no_extension.windows(11).position(|w| w == b"NETSCAPE2.0").expect("extension");
        no_extension[at] = b'X';
        assert_eq!(loop_passes(&no_extension), Some(1), "no extension plays once");
    }

    #[test]
    fn t051_a_webp_loop_count_is_read_from_the_anim_chunk() {
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        webp.extend_from_slice(&[10, 0, 0, 0]);
        webp.extend_from_slice(&[0; 10]);
        webp.extend_from_slice(b"ANIM");
        webp.extend_from_slice(&[6, 0, 0, 0, 0, 0, 0, 0, 4, 0]);
        assert_eq!(loop_passes(&webp), Some(4));
        let n = webp.len();
        webp[n - 2] = 0;
        assert_eq!(loop_passes(&webp), None);
    }

    #[test]
    fn t052_a_still_png_is_one_frame_without_a_delay() {
        let frames = decode_frames(&encoded(image::ImageFormat::Png, 3, 2), false).expect("png");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].delay_ms, 0);
    }

    #[test]
    fn the_default_load_state_is_empty() {
        assert_eq!(LoadState::default(), LoadState::Empty);
    }

    #[test]
    fn a_jpeg_xl_page_decodes_as_kagane_sends_it() {
        // the reader decodes through the shared decoder, which reads JPEG XL (the format Kagane's page host prefers)
        let bytes = include_bytes!("../../crates/kuro-images/tests/fixtures/jxl_still_96.jxl");
        let frames = decode_frames(bytes, false).expect("a JPEG XL page decodes");
        assert_eq!(frames.len(), 1, "one still frame");
        assert!(frames[0].page.width > 0 && frames[0].page.height > 0 && frames[0].delay_ms == 0);
    }
}
