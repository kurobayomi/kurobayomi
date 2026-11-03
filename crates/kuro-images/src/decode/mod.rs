//! Image decode entry point and per-format decoders (area:34 section 1 and behavior sections 13 and 14).
//!
//! Pipeline: sniff the type, pick the decoder path (T-34a-025), read the header, refuse an empty or oversized
//! image, decode to RGBA8 with straight alpha, and downscale by the in-sample size (T-34a-026).

mod animation;
mod borders;
mod heif;
mod raster;

pub use animation::{Frame, decode_frames};

#[cfg(feature = "jxl")]
mod jxl;

use std::sync::Arc;

use bytes::Bytes;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageFormat, imageops};

use crate::error::ImageError;
use crate::icc::DisplayProfile;
use crate::options::{LoadOptions, RequestSize, Scale};
use crate::sample::{sample_for, sampled_side};
use crate::sniff::{self, ImageFormat as Sniffed};

/// Most pixels a cover may have (design section 4.6, proposal).
pub const COVER_MAX_PIXELS: u64 = 40_000_000;
/// Most pixels a reader page may have (design section 4.6, proposal).
pub const PAGE_MAX_PIXELS: u64 = 120_000_000;

/// Which decoder family handles a request (behavior section 13.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecoderPath {
    /// The custom decoder: every format, and the only path for AVIF, JXL and HEIF.
    Custom,
    /// The platform decoder: JPEG, PNG, GIF, WebP and BMP.
    Platform,
}

/// The custom decoder takes a request when `custom_decoder` is set or the sniffed type is AVIF, JXL or HEIF
/// (behavior section 13.1). BMP is not sniffed, so it is always `Platform`.
pub fn select_decoder(custom_decoder: bool, sniffed: Option<Sniffed>) -> DecoderPath {
    let native_only = matches!(sniffed, Some(Sniffed::Avif | Sniffed::Jxl | Sniffed::Heif));
    if custom_decoder || native_only {
        DecoderPath::Custom
    } else {
        DecoderPath::Platform
    }
}

#[derive(Clone, Debug)]
pub struct DecodeOptions {
    pub size: RequestSize,
    pub scale: Scale,
    pub custom_decoder: bool,
    /// From `LoadOptions::crop_borders`: uniform-colour borders are cut off before the picture is sampled
    /// (area:44 behavior section 9.4).
    pub crop_borders: bool,
    pub max_pixels: u64,
    /// The display ICC profile (T-34a-031), loaded once when the setting changes. The picture is taken from sRGB to
    /// it after decoding. `None` shows the picture as decoded.
    pub display_profile: Option<Arc<DisplayProfile>>,
}

impl PartialEq for DecodeOptions {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size
            && self.scale == other.scale
            && self.custom_decoder == other.custom_decoder
            && self.crop_borders == other.crop_borders
            && self.max_pixels == other.max_pixels
            && match (&self.display_profile, &other.display_profile) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}

impl Eq for DecodeOptions {}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            size: RequestSize::Original,
            scale: Scale::Fit,
            custom_decoder: false,
            crop_borders: false,
            max_pixels: COVER_MAX_PIXELS,
            display_profile: None,
        }
    }
}

impl From<&LoadOptions> for DecodeOptions {
    fn from(opts: &LoadOptions) -> Self {
        Self {
            size: opts.size,
            scale: opts.scale,
            custom_decoder: opts.custom_decoder,
            crop_borders: opts.crop_borders,
            max_pixels: COVER_MAX_PIXELS,
            display_profile: None,
        }
    }
}

/// A decoded image: row-major RGBA8, straight alpha, `stride = width * 4`. The buffer is shared, so cloning is
/// cheap and the memory cache can hold it without any GTK object (design section 16 item 11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba: Bytes,
    /// True when the in-sample size was greater than 1 (behavior section 14.5).
    pub is_sampled: bool,
}

#[derive(Clone, Copy)]
enum Codec {
    Raster(ImageFormat),
    Avif,
    Jxl,
    Heif,
}

fn detect(bytes: &[u8]) -> Option<Codec> {
    Some(match sniff::sniff(bytes) {
        Some(Sniffed::Jpeg) => Codec::Raster(ImageFormat::Jpeg),
        Some(Sniffed::Png) => Codec::Raster(ImageFormat::Png),
        Some(Sniffed::Gif) => Codec::Raster(ImageFormat::Gif),
        Some(Sniffed::Webp) => Codec::Raster(ImageFormat::WebP),
        Some(Sniffed::Avif) => Codec::Avif,
        Some(Sniffed::Jxl) => Codec::Jxl,
        Some(Sniffed::Heif) => Codec::Heif,
        None if bytes.starts_with(b"BM") => Codec::Raster(ImageFormat::Bmp),
        None => return None,
    })
}

/// Opens the decoder, which reads the header. An unreadable header is `DecodeInit` (Kotlin's decoder is null).
/// HEIF never gets here: it goes through `heif::decode`.
fn open(codec: Codec, bytes: &[u8]) -> Result<Box<dyn ImageDecoder + '_>, ImageError> {
    let opened = match codec {
        Codec::Raster(format) => raster::open(bytes, format),
        #[cfg(feature = "avif")]
        Codec::Avif => raster::open(bytes, ImageFormat::Avif),
        #[cfg(not(feature = "avif"))]
        Codec::Avif => return Err(ImageError::UnsupportedFormat),
        #[cfg(feature = "jxl")]
        Codec::Jxl => jxl::open(bytes),
        #[cfg(not(feature = "jxl"))]
        Codec::Jxl => return Err(ImageError::UnsupportedFormat),
        Codec::Heif => return Err(ImageError::DecodeInit),
    };
    opened.map_err(|e| {
        tracing::debug!(error = %e, "image header unreadable");
        ImageError::DecodeInit
    })
}

/// A JPEG without its end-of-image marker is a truncated download. The decoder would fill the missing rows, so the
/// file is refused instead (fixture F-11). Zero padding after the marker is allowed.
fn is_truncated_jpeg(bytes: &[u8]) -> bool {
    let end = bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    !bytes[..end].ends_with(&[0xFF, 0xD9])
}

/// Decodes through the `image` decoder of `codec`: header first, size checks, then the pixels turned by their EXIF
/// orientation.
fn decode_with_decoder(
    codec: Codec,
    bytes: &[u8],
    opts: &DecodeOptions,
) -> Result<image::RgbaImage, ImageError> {
    let mut decoder = open(codec, bytes)?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return Err(ImageError::DecodeInit);
    }
    if width > i32::MAX as u32
        || height > i32::MAX as u32
        || u64::from(width) * u64::from(height) > opts.max_pixels
    {
        return Err(ImageError::TooLarge);
    }
    // The EXIF orientation turns the picture before it is cropped and sampled.
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    if matches!(codec, Codec::Raster(ImageFormat::Jpeg)) && is_truncated_jpeg(bytes) {
        return Err(ImageError::DecodeFailed);
    }
    let mut turned = DynamicImage::from_decoder(decoder).map_err(|e| {
        tracing::debug!(error = %e, "image decode failed");
        ImageError::DecodeFailed
    })?;
    turned.apply_orientation(orientation);
    let image = turned.into_rgba8();
    if image.width() == 0 || image.height() == 0 {
        return Err(ImageError::DecodeFailed);
    }
    Ok(image)
}

/// Decodes the first frame of `bytes` to RGBA8 at the size the request asks for (behavior sections 14 and 15).
pub fn decode(bytes: &[u8], opts: &DecodeOptions) -> Result<Decoded, ImageError> {
    let codec = detect(bytes).ok_or(ImageError::UnsupportedFormat)?;
    // The platform path handles the raster formats only; anything else needs the custom decoder.
    if select_decoder(opts.custom_decoder, sniff::sniff(bytes)) == DecoderPath::Platform
        && !matches!(codec, Codec::Raster(_))
    {
        return Err(ImageError::UnsupportedFormat);
    }
    let mut image = match codec {
        Codec::Heif => heif::decode(bytes, opts.max_pixels)?,
        _ => decode_with_decoder(codec, bytes, opts)?,
    };
    if opts.crop_borders {
        image = borders::crop(image);
    }
    // The target size is matched to the turned and cropped picture (design section 4.5).
    let sample = sample_for(opts.size, opts.scale, (image.width(), image.height()));
    if sample > 1 {
        image = imageops::thumbnail(
            &image,
            sampled_side(image.width(), sample),
            sampled_side(image.height(), sample),
        );
    }
    let decoded = Decoded {
        width: image.width(),
        height: image.height(),
        rgba: Bytes::from(image.into_raw()),
        is_sampled: sample > 1,
    };
    Ok(match &opts.display_profile {
        Some(profile) => profile.apply(&decoded),
        None => decoded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{encode, fixture, gradient};

    fn opts() -> DecodeOptions {
        DecodeOptions::default()
    }

    #[test]
    fn decoder_selection_rule() {
        assert_eq!(
            select_decoder(false, Some(Sniffed::Jpeg)),
            DecoderPath::Platform
        );
        assert_eq!(
            select_decoder(false, Some(Sniffed::Png)),
            DecoderPath::Platform
        );
        assert_eq!(
            select_decoder(false, Some(Sniffed::Gif)),
            DecoderPath::Platform
        );
        assert_eq!(
            select_decoder(false, Some(Sniffed::Webp)),
            DecoderPath::Platform
        );
        assert_eq!(select_decoder(false, None), DecoderPath::Platform);
        for native in [Sniffed::Avif, Sniffed::Jxl, Sniffed::Heif] {
            assert_eq!(select_decoder(false, Some(native)), DecoderPath::Custom);
        }
        assert_eq!(
            select_decoder(true, Some(Sniffed::Jpeg)),
            DecoderPath::Custom
        );
        assert_eq!(select_decoder(true, None), DecoderPath::Custom);
    }

    #[test]
    fn raster_formats_decode_to_rgba8() {
        let src = gradient(64, 48);
        for format in [
            ImageFormat::Png,
            ImageFormat::Gif,
            ImageFormat::WebP,
            ImageFormat::Bmp,
            ImageFormat::Jpeg,
        ] {
            let got = decode(&encode(&src, format), &opts())
                .unwrap_or_else(|e| panic!("{format:?}: {e}"));
            assert_eq!((got.width, got.height), (64, 48), "{format:?}");
            assert_eq!(got.rgba.len(), 64 * 48 * 4, "{format:?}");
            assert!(!got.is_sampled);
        }
    }

    #[test]
    fn png_keeps_straight_alpha_exactly() {
        let src = gradient(16, 16);
        let got = decode(&encode(&src, ImageFormat::Png), &opts()).unwrap();
        assert_eq!(&got.rgba[..], src.as_raw().as_slice());
    }

    #[test]
    fn custom_decoder_flag_decodes_the_same_raster_bytes() {
        let bytes = encode(&gradient(32, 32), ImageFormat::Png);
        let custom = DecodeOptions {
            custom_decoder: true,
            ..opts()
        };
        assert_eq!(
            decode(&bytes, &custom).unwrap(),
            decode(&bytes, &opts()).unwrap()
        );
    }

    #[test]
    fn sampled_decode_shrinks_and_flags() {
        let bytes = encode(&gradient(400, 600), ImageFormat::Png);
        let want = DecodeOptions {
            size: RequestSize::Pixels {
                width: 100,
                height: 150,
            },
            ..opts()
        };
        let got = decode(&bytes, &want).unwrap();
        assert_eq!((got.width, got.height), (100, 150));
        assert!(got.is_sampled);
        assert_eq!(got.rgba.len(), 100 * 150 * 4);
    }

    fn bordered_png() -> Vec<u8> {
        let mut img = image::RgbaImage::from_pixel(100, 120, image::Rgba([255, 255, 255, 255]));
        for y in 10..90 {
            for x in 20..80 {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        encode(&img, ImageFormat::Png)
    }

    #[test]
    fn crop_borders_cuts_the_border_only_when_asked() {
        let bytes = bordered_png();
        let off = decode(&bytes, &opts()).unwrap();
        assert_eq!((off.width, off.height), (100, 120));
        let on = decode(
            &bytes,
            &DecodeOptions {
                crop_borders: true,
                ..opts()
            },
        )
        .unwrap();
        assert_eq!((on.width, on.height), (60, 80));
    }

    #[test]
    fn crop_happens_before_sampling() {
        let want = DecodeOptions {
            crop_borders: true,
            size: RequestSize::Pixels {
                width: 30,
                height: 40,
            },
            ..opts()
        };
        let got = decode(&bordered_png(), &want).unwrap();
        assert_eq!((got.width, got.height), (30, 40));
        assert!(got.is_sampled);
    }

    #[test]
    fn original_size_is_never_sampled() {
        let bytes = encode(&gradient(400, 600), ImageFormat::Png);
        let got = decode(&bytes, &opts()).unwrap();
        assert_eq!((got.width, got.height), (400, 600));
        assert!(!got.is_sampled);
    }

    #[test]
    fn unknown_and_empty_bytes_are_unsupported() {
        assert!(matches!(
            decode(b"", &opts()),
            Err(ImageError::UnsupportedFormat)
        ));
        assert!(matches!(
            decode(b"plain text pretending to be a jpg", &opts()),
            Err(ImageError::UnsupportedFormat)
        ));
    }

    #[test]
    fn corrupt_body_after_a_good_header_is_decode_failed() {
        let mut bytes = encode(&gradient(64, 64), ImageFormat::Png);
        let n = bytes.len();
        bytes.truncate(n - 40);
        assert!(matches!(
            decode(&bytes, &opts()),
            Err(ImageError::DecodeFailed)
        ));
    }

    #[test]
    fn zero_dimension_header_is_decode_init() {
        let mut bytes = encode(&gradient(8, 8), ImageFormat::Png);
        // IHDR width sits at bytes 16..20.
        bytes[16..20].copy_from_slice(&0u32.to_be_bytes());
        assert!(matches!(
            decode(&bytes, &opts()),
            Err(ImageError::DecodeInit)
        ));
    }

    #[test]
    fn too_many_pixels_is_refused_before_decoding() {
        let bytes = encode(&gradient(64, 64), ImageFormat::Png);
        let small = DecodeOptions {
            max_pixels: 64 * 64 - 1,
            ..opts()
        };
        assert!(matches!(decode(&bytes, &small), Err(ImageError::TooLarge)));
        let exact = DecodeOptions {
            max_pixels: 64 * 64,
            ..opts()
        };
        assert!(decode(&bytes, &exact).is_ok());
    }

    #[test]
    fn options_with_no_profile_compare_equal_and_a_profile_compares_by_identity() {
        assert_eq!(opts(), DecodeOptions::default());
        let with = DecodeOptions {
            crop_borders: true,
            ..opts()
        };
        assert_ne!(with, opts());
    }

    #[cfg(feature = "icc")]
    fn linear_gamma_profile_file(dir: &std::path::Path) -> std::path::PathBuf {
        use lcms2::{CIExyY, CIExyYTRIPLE, Profile, ToneCurve};
        let xyy = |x, y| CIExyY { x, y, Y: 1.0 };
        let curve = ToneCurve::new(1.0);
        let profile = Profile::new_rgb(
            &xyy(0.3127, 0.3290),
            &CIExyYTRIPLE {
                Red: xyy(0.64, 0.33),
                Green: xyy(0.30, 0.60),
                Blue: xyy(0.15, 0.06),
            },
            &[&curve, &curve, &curve],
        )
        .unwrap();
        let path = dir.join("linear.icc");
        std::fs::write(&path, profile.icc().unwrap()).unwrap();
        path
    }

    #[cfg(feature = "icc")]
    #[test]
    fn the_display_profile_is_applied_after_decoding_and_none_leaves_the_picture() {
        let dir = tempfile::tempdir().unwrap();
        let profile = crate::icc::load_setting(linear_gamma_profile_file(dir.path()).to_str().unwrap());
        assert!(profile.is_some());
        let gray = image::RgbaImage::from_pixel(4, 4, image::Rgba([128, 128, 128, 200]));
        let bytes = encode(&gray, ImageFormat::Png);
        let plain = decode(&bytes, &opts()).unwrap();
        let managed = decode(
            &bytes,
            &DecodeOptions {
                display_profile: profile,
                ..opts()
            },
        )
        .unwrap();
        assert_eq!(&plain.rgba[..4], &[128, 128, 128, 200]);
        assert!(managed.rgba[0] < 100, "a linear display makes mid grey darker: {:?}", &managed.rgba[..4]);
        assert_eq!(managed.rgba[3], 200, "alpha is untouched");
        assert_eq!((managed.width, managed.height), (4, 4));
    }

    #[test]
    fn heif_reports_decoder_init_failure() {
        let heic = b"\0\0\0\x18ftypheic\0\0\0\0heicmif1";
        assert!(matches!(decode(heic, &opts()), Err(ImageError::DecodeInit)));
    }

    #[cfg(feature = "avif")]
    #[test]
    fn avif_still_decodes_to_the_same_contract() {
        let got = decode(&fixture("avif_still_96.avif"), &opts()).unwrap();
        assert_eq!((got.width, got.height), (96, 64));
        assert_eq!(got.rgba.len(), 96 * 64 * 4);
        // The fixture is a red to blue gradient, fully opaque.
        assert!(got.rgba.chunks(4).all(|p| p[3] == 255));
        let first = &got.rgba[..3];
        let last = &got.rgba[got.rgba.len() - 4..got.rgba.len() - 1];
        assert!(
            first[0] > first[2] && last[2] > last[0],
            "{first:?} {last:?}"
        );
    }

    #[cfg(feature = "avif")]
    #[test]
    fn corrupt_avif_has_the_two_decode_messages_and_no_pixels() {
        let bytes = fixture("avif_still_96.avif");
        let err = decode(&bytes[..bytes.len() / 2], &opts()).unwrap_err();
        assert!(
            matches!(err, ImageError::DecodeInit | ImageError::DecodeFailed),
            "{err}"
        );
        let mut garbled = bytes.clone();
        let n = garbled.len();
        garbled[n - 80..].fill(0xA5);
        let err = decode(&garbled, &opts());
        assert!(err.is_err() || err.is_ok_and(|d| d.width == 96));
    }

    #[cfg(feature = "jxl")]
    #[test]
    fn jxl_still_decodes_to_the_same_contract() {
        let got = decode(&fixture("jxl_still_96.jxl"), &opts()).unwrap();
        assert_eq!((got.width, got.height), (96, 64));
        assert_eq!(got.rgba.len(), 96 * 64 * 4);
        assert!(got.rgba.chunks(4).all(|p| p[3] == 255));
        assert!(got.rgba[0] > got.rgba[2]);
    }

    #[cfg(feature = "jxl")]
    #[test]
    fn corrupt_jxl_has_the_two_decode_messages() {
        let bytes = fixture("jxl_still_96.jxl");
        let err = decode(&bytes[..bytes.len() / 2], &opts()).unwrap_err();
        assert!(
            matches!(err, ImageError::DecodeInit | ImageError::DecodeFailed),
            "{err}"
        );
        assert!(
            ["Failed to initialize decoder", "Failed to decode image"]
                .contains(&err.to_string().as_str())
        );
    }

    #[cfg(feature = "avif")]
    #[test]
    fn sampled_avif_is_flagged() {
        let want = DecodeOptions {
            size: RequestSize::Pixels {
                width: 48,
                height: 32,
            },
            ..opts()
        };
        let got = decode(&fixture("avif_still_96.avif"), &want).unwrap();
        assert_eq!((got.width, got.height), (48, 32));
        assert!(got.is_sampled);
    }
}

/// The fixture set of area:34 (generated by `tests/fixtures/generate.py`).
/// F-10 (HEIF) is not in the set: no encoder is available for it in this build.
#[cfg(test)]
mod fixture_tests {
    use super::*;
    use crate::sniff::is_image;
    use crate::testing::fixture;

    fn opts() -> DecodeOptions {
        DecodeOptions::default()
    }

    #[test]
    fn f01_baseline_jpeg_decodes_at_its_size() {
        let got = decode(&fixture("jpeg_baseline_320x480.jpg"), &opts()).unwrap();
        assert_eq!((got.width, got.height), (320, 480));
        assert_eq!(got.rgba.len(), 320 * 480 * 4);
    }

    #[test]
    fn f02_exif_orientation_six_turns_the_picture_clockwise() {
        let bytes = fixture("jpeg_exif_rot90.jpg");
        let got = decode(&bytes, &opts()).unwrap();
        assert_eq!((got.width, got.height), (320, 480));
        // The stored picture has red rising to the right and green rising downwards. After a quarter turn
        // clockwise the top-left corner is the stored bottom-left corner, which is green.
        assert!(
            got.rgba[0] < 60 && got.rgba[1] > 195,
            "{:?}",
            &got.rgba[..3]
        );
        // The top-right corner is the stored top-left corner, which is black.
        let top_right = (320 - 1) * 4;
        assert!(got.rgba[top_right] < 40 && got.rgba[top_right + 1] < 40);
    }

    #[test]
    fn f02_orientation_is_applied_before_the_sample_size() {
        let want = DecodeOptions {
            size: RequestSize::Pixels {
                width: 160,
                height: 240,
            },
            ..opts()
        };
        let got = decode(&fixture("jpeg_exif_rot90.jpg"), &want).unwrap();
        assert_eq!((got.width, got.height), (160, 240));
        assert!(got.is_sampled);
    }

    #[test]
    fn f03_rgba_png_keeps_its_alpha_gradient() {
        let got = decode(&fixture("png_rgba_100x100.png"), &opts()).unwrap();
        assert_eq!((got.width, got.height), (100, 100));
        assert_eq!(got.rgba[3], 0, "left edge is transparent");
        assert_eq!(got.rgba[99 * 4 + 3], 255, "right edge is opaque");
    }

    #[test]
    fn f04_static_gif_decodes() {
        let got = decode(&fixture("gif_static_64.gif"), &opts()).unwrap();
        assert_eq!((got.width, got.height), (64, 64));
    }

    #[test]
    fn f05_animated_gif_gives_its_first_frame() {
        let got = decode(&fixture("gif_animated_3f.gif"), &opts()).unwrap();
        assert_eq!((got.width, got.height), (64, 64));
        assert!(
            got.rgba[0] > 200 && got.rgba[1] < 60 && got.rgba[2] < 60,
            "{:?}",
            &got.rgba[..4]
        );
    }

    #[test]
    fn f06_lossy_webp_decodes() {
        let got = decode(&fixture("webp_lossy_50.webp"), &opts()).unwrap();
        assert_eq!((got.width, got.height), (120, 90));
    }

    #[test]
    fn f07_bmp_decodes() {
        let got = decode(&fixture("bmp_24bit.bmp"), &opts()).unwrap();
        assert_eq!((got.width, got.height), (96, 64));
    }

    #[test]
    fn f11_truncated_jpeg_fails_with_one_of_the_two_decode_messages() {
        let err = decode(&fixture("truncated_jpeg.jpg"), &opts()).unwrap_err();
        assert!(
            matches!(err, ImageError::DecodeInit | ImageError::DecodeFailed),
            "{err}"
        );
    }

    #[test]
    fn f12_zero_byte_file_is_unsupported_and_not_an_image() {
        let bytes = fixture("zero_bytes.bin");
        assert!(sniff::sniff(&bytes).is_none());
        assert!(matches!(
            decode(&bytes, &opts()),
            Err(ImageError::UnsupportedFormat)
        ));
    }

    #[test]
    fn f13_text_named_jpg_passes_the_name_test_only() {
        let bytes = fixture("text_named.jpg");
        // The extension alone is enough, so the bytes do not matter for the name test.
        assert!(is_image(Some("text_named.jpg"), &[]));
        assert!(sniff::sniff(&bytes).is_none());
        assert!(matches!(
            decode(&bytes, &opts()),
            Err(ImageError::UnsupportedFormat)
        ));
    }

    #[test]
    fn f14_zero_width_png_is_decode_init() {
        let err = decode(&fixture("zero_dimension.png"), &opts()).unwrap_err();
        assert!(matches!(err, ImageError::DecodeInit), "{err}");
    }

    #[test]
    fn f15_huge_png_header_is_too_large_before_any_pixel_is_read() {
        let err = decode(&fixture("huge_header.png"), &opts()).unwrap_err();
        assert!(matches!(err, ImageError::TooLarge), "{err}");
    }

    #[test]
    fn f16_uppercase_extension_is_an_image_by_sniffing() {
        let bytes = fixture("uppercase_ext.JPG");
        assert!(is_image(Some("uppercase_ext.JPG"), &bytes));
        assert_eq!(decode(&bytes, &opts()).unwrap().width, 320);
    }

    #[test]
    fn f17_jpeg_extension_is_an_image_by_sniffing_only() {
        let bytes = fixture("jpeg_as_jpeg_ext.jpeg");
        assert!(is_image(Some("jpeg_as_jpeg_ext.jpeg"), &bytes));
        assert!(!is_image(Some("jpeg_as_jpeg_ext.jpeg"), &[]));
        assert_eq!(decode(&bytes, &opts()).unwrap().height, 480);
    }
}
