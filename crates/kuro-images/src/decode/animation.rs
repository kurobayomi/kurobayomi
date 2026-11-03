//! Animated GIF and WebP frames for the full-cover viewer (T-34a-033).
//!
//! Grids use `decode`, which gives the first frame. The full-cover viewer uses `decode_frames`. Frames keep their own
//! size and are not sampled or border-cropped: the viewer scales the whole picture.

use std::io::Cursor;

use bytes::Bytes;
use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, Frames};

use super::{DecodeOptions, Decoded, decode};
use crate::error::ImageError;
use crate::sniff::{self, ImageFormat as Sniffed};

/// One frame of an animation: the picture and how long it stays on screen, in milliseconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub picture: Decoded,
    pub delay_ms: u32,
}

/// Every frame of `bytes` in order. A GIF or WebP with one frame gives one frame, and every other format gives the
/// still picture of `decode` with delay 0. `opts.max_pixels` is checked on each frame.
pub fn decode_frames(bytes: &[u8], opts: &DecodeOptions) -> Result<Vec<Frame>, ImageError> {
    match sniff::sniff(bytes) {
        Some(Sniffed::Gif) => {
            let decoder =
                GifDecoder::new(Cursor::new(bytes)).map_err(|_| ImageError::DecodeInit)?;
            collect(decoder.into_frames(), opts)
        }
        Some(Sniffed::Webp) => {
            let decoder =
                WebPDecoder::new(Cursor::new(bytes)).map_err(|_| ImageError::DecodeInit)?;
            if decoder.has_animation() {
                collect(decoder.into_frames(), opts)
            } else {
                still(bytes, opts)
            }
        }
        _ => still(bytes, opts),
    }
}

fn still(bytes: &[u8], opts: &DecodeOptions) -> Result<Vec<Frame>, ImageError> {
    Ok(vec![Frame {
        picture: decode(bytes, opts)?,
        delay_ms: 0,
    }])
}

fn collect(frames: Frames<'_>, opts: &DecodeOptions) -> Result<Vec<Frame>, ImageError> {
    let mut out = Vec::new();
    for frame in frames {
        let frame = frame.map_err(|error| {
            tracing::debug!(error = %error, "animation frame failed");
            ImageError::DecodeFailed
        })?;
        let (width, height) = frame.buffer().dimensions();
        if width == 0 || height == 0 {
            return Err(ImageError::DecodeFailed);
        }
        if u64::from(width) * u64::from(height) > opts.max_pixels {
            return Err(ImageError::TooLarge);
        }
        let (numer, denom) = frame.delay().numer_denom_ms();
        let delay_ms = numer.checked_div(denom).unwrap_or(0);
        let rgba = frame.into_buffer().into_raw();
        out.push(Frame {
            picture: Decoded {
                width,
                height,
                rgba: Bytes::from(rgba),
                is_sampled: false,
            },
            delay_ms,
        });
    }
    if out.is_empty() {
        return Err(ImageError::DecodeFailed);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fixture;

    fn opts() -> DecodeOptions {
        DecodeOptions::default()
    }

    #[test]
    fn an_animated_gif_gives_every_frame_in_order_with_its_delay() {
        let frames = decode_frames(&fixture("gif_animated_3f.gif"), &opts()).expect("decodes");
        assert_eq!(frames.len(), 3);
        let first_pixel = |f: &Frame| [f.picture.rgba[0], f.picture.rgba[1], f.picture.rgba[2]];
        assert!(first_pixel(&frames[0])[0] > 200, "red first");
        assert!(first_pixel(&frames[1])[1] > 200, "green second");
        assert!(first_pixel(&frames[2])[2] > 200, "blue third");
        assert!(
            frames.iter().all(|f| f.delay_ms == 100),
            "{:?}",
            frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>()
        );
        assert_eq!(
            (frames[0].picture.width, frames[0].picture.height),
            (64, 64)
        );
    }

    #[test]
    fn an_animated_webp_gives_every_frame_in_order_with_its_delay() {
        let frames = decode_frames(&fixture("webp_animated_3f.webp"), &opts()).expect("decodes");
        assert_eq!(frames.len(), 3);
        let first_pixel = |f: &Frame| [f.picture.rgba[0], f.picture.rgba[1], f.picture.rgba[2]];
        assert!(first_pixel(&frames[0])[0] > 200, "red first");
        assert!(first_pixel(&frames[1])[1] > 200, "green second");
        assert!(first_pixel(&frames[2])[2] > 200, "blue third");
        assert!(frames.iter().all(|f| f.delay_ms == 100));
        assert_eq!((frames[0].picture.width, frames[0].picture.height), (64, 64));
    }

    #[test]
    fn a_static_webp_gives_one_frame() {
        let frames = decode_frames(&fixture("webp_lossy_50.webp"), &opts()).expect("decodes");
        assert_eq!(frames.len(), 1);
    }

    #[test]
    fn a_static_gif_and_a_jpeg_give_one_frame_with_no_delay() {
        for name in ["gif_static_64.gif", "jpeg_baseline_320x480.jpg"] {
            let frames = decode_frames(&fixture(name), &opts()).expect("decodes");
            assert_eq!(frames.len(), 1, "{name}");
            assert_eq!(frames[0].delay_ms, 0, "{name}");
        }
    }

    #[test]
    fn a_frame_over_the_pixel_limit_is_refused() {
        let tight = DecodeOptions {
            max_pixels: 10,
            ..DecodeOptions::default()
        };
        assert!(matches!(
            decode_frames(&fixture("gif_animated_3f.gif"), &tight),
            Err(ImageError::TooLarge)
        ));
    }

    #[test]
    fn garbage_is_unsupported() {
        assert!(matches!(
            decode_frames(b"not an image", &opts()),
            Err(ImageError::UnsupportedFormat)
        ));
    }
}
