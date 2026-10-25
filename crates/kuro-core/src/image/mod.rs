//! Image helpers: magic-byte sniffing, MIME and extension mapping, decoding,
//! header-only dimensions, JPEG re-encoding, and the animated and wide-page
//! rules. Port of `core/common/.../util/system/ImageUtil.kt`.
//!
//! # Format decisions (OQ-13a-5, OQ-13a-6)
//!
//! - JPEG, PNG, GIF and WebP decode through the `image` crate; JXL through `jxl-oxide`.
//! - AVIF: the AV1 decoder is dav1d, pulled in by the `image` crate's `avif-native` feature. It is
//!   reachable only when the `heif` cargo feature is on; otherwise `Decode("feature disabled")`.
//! - HEIF: no decoder. `libheif` is not installed on the build host and `libheif-rs` is not a
//!   dependency. Even with the `heif` feature, HEIF stays an explicit `Decode("heif decoder not
//!   built")` error, and sniffing still names the kind so file-name checks keep working. Wiring a
//!   decoder means replacing that one arm of `format_of` and adding the binding behind `heif`.
//! - `heif` is off by default (area 13 overview, OQ-13a-5): HEIF and AVIF are rare in manga and
//!   anime sources. Turn it on if users read such files.
//! - libarchive binding (OQ-13a-6): the planned default stands, `libarchive3-sys` behind one safe
//!   wrapper under the `archive` feature; the spike T-13a-031 has not run, so no binding exists yet.

pub mod background;

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbImage, RgbaImage, imageops};
use jxl_oxide::integration::JxlDecoder;

/// Size of the head buffer the Kotlin decoder reads (`ImageUtil.getImageType`).
const HEAD_LEN: usize = 32;
const JXL_CONTAINER: &[u8] = b"\0\0\0\x0cJXL \r\n\x87\n";

#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("unknown image format")]
    UnknownFormat,
    #[error("decode failed: {0}")]
    Decode(String),
    #[error("encode failed: {0}")]
    Encode(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Avif,
    Gif,
    /// Deliberately unsupported: no HEIF decoder (libheif is not installed). Decoding and
    /// `dimensions` return an explicit `Decode` error for HEIF.
    Heif,
    Jpeg,
    Jxl,
    Png,
    Webp,
}

impl ImageKind {
    pub const ALL: [ImageKind; 7] = [
        ImageKind::Avif,
        ImageKind::Gif,
        ImageKind::Heif,
        ImageKind::Jpeg,
        ImageKind::Jxl,
        ImageKind::Png,
        ImageKind::Webp,
    ];

    pub fn mime(self) -> &'static str {
        match self {
            ImageKind::Avif => "image/avif",
            ImageKind::Gif => "image/gif",
            ImageKind::Heif => "image/heif",
            ImageKind::Jpeg => "image/jpeg",
            ImageKind::Jxl => "image/jxl",
            ImageKind::Png => "image/png",
            ImageKind::Webp => "image/webp",
        }
    }

    /// JPEG is `jpg`, not `jpeg` (ImageUtil.ImageType).
    pub fn extension(self) -> &'static str {
        match self {
            ImageKind::Avif => "avif",
            ImageKind::Gif => "gif",
            ImageKind::Heif => "heif",
            ImageKind::Jpeg => "jpg",
            ImageKind::Jxl => "jxl",
            ImageKind::Png => "png",
            ImageKind::Webp => "webp",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sniff {
    pub kind: ImageKind,
    pub animated: bool,
}

/// Magic-byte detection on the first 32 bytes, in Kotlin order. Shorter input is
/// zero-padded like the Kotlin buffer.
pub fn sniff(head: &[u8]) -> Option<Sniff> {
    let mut b = [0u8; HEAD_LEN];
    let n = head.len().min(HEAD_LEN);
    b[..n].copy_from_slice(&head[..n]);

    let still = |kind| Some(Sniff { kind, animated: false });
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return still(ImageKind::Jpeg);
    }
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        return still(ImageKind::Png);
    }
    // The table only checks RIFF; requiring WEBP also keeps WAV and AVI out.
    if b.starts_with(b"RIFF") && &b[8..12] == b"WEBP" {
        return Some(Sniff { kind: ImageKind::Webp, animated: webp_animated(&b) });
    }
    if b.starts_with(b"GIF8") {
        return Some(Sniff { kind: ImageKind::Gif, animated: true });
    }
    if b.starts_with(JXL_CONTAINER) || b.starts_with(&[0xFF, 0x0A]) {
        return still(ImageKind::Jxl);
    }
    if &b[4..8] == b"ftyp" {
        return ftyp_kind(&b).map(|kind| Sniff { kind, animated: false });
    }
    None
}

/// VP8X chunk at offset 12; its flags byte is at offset 20, animation bit 0x02.
fn webp_animated(b: &[u8; HEAD_LEN]) -> bool {
    &b[12..16] == b"VP8X" && b[20] & 0x02 != 0
}

/// ISO-BMFF brand search. Brands start at offset 8, step 4, and must lie within
/// the box (`headerSize`) and the 32-byte head.
fn ftyp_kind(b: &[u8; HEAD_LEN]) -> Option<ImageKind> {
    let header_size = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as i64;
    let max_offset = header_size.min(HEAD_LEN as i64) - 4;
    let mut offset = 8usize;
    while offset as i64 <= max_offset {
        // The table's `hei?` pattern does not match its own `heic` vector, so accept the
        // `hei` and `hev` prefixes (heic, heix, hevc, ...).
        let brand = &b[offset..offset + 4];
        if brand.starts_with(b"hei") || brand.starts_with(b"hev") {
            return Some(ImageKind::Heif);
        }
        if brand.starts_with(b"avi") {
            return Some(ImageKind::Avif);
        }
        offset += 4;
    }
    None
}

/// Exact, case-sensitive MIME match, no parameters.
pub fn kind_from_mime(mime: &str) -> Option<ImageKind> {
    ImageKind::ALL.into_iter().find(|k| k.mime() == mime)
}

/// ImageUtil.isImage: the extension is case-sensitive, and a name without a dot
/// is its own extension. Otherwise the head bytes must sniff as an image.
pub fn is_image_name(name: &str, head: impl FnOnce() -> Option<Vec<u8>>) -> bool {
    let ext = name.rsplit_once('.').map_or(name, |(_, e)| e);
    ImageKind::ALL.iter().any(|k| k.extension() == ext)
        || head().is_some_and(|h| sniff(&h).is_some())
}

/// ImageUtil.getExtensionFromMimeType: exact MIME first, then the sniffed
/// format, then `jpg`.
pub fn extension_for(mime: Option<&str>, head: impl FnOnce() -> Option<Vec<u8>>) -> &'static str {
    let kind = mime
        .and_then(kind_from_mime)
        .or_else(|| head().and_then(|h| sniff(&h)).map(|s| s.kind));
    kind.map_or("jpg", ImageKind::extension)
}

/// The `image` crate format for `kind`. `None` for JXL, which decodes through jxl-oxide.
/// AVIF and HEIF need the `heif` feature; HEIF has no decoder yet.
fn format_of(kind: ImageKind) -> Result<Option<ImageFormat>, ImageError> {
    Ok(Some(match kind {
        ImageKind::Avif | ImageKind::Heif if !cfg!(feature = "heif") => {
            return Err(ImageError::Decode("feature disabled".into()));
        }
        ImageKind::Avif => ImageFormat::Avif,
        ImageKind::Heif => return Err(ImageError::Decode("heif decoder not built".into())),
        ImageKind::Gif => ImageFormat::Gif,
        ImageKind::Jpeg => ImageFormat::Jpeg,
        ImageKind::Png => ImageFormat::Png,
        ImageKind::Webp => ImageFormat::WebP,
        ImageKind::Jxl => return Ok(None),
    }))
}

fn decode_err(e: image::ImageError) -> ImageError {
    ImageError::Decode(e.to_string())
}

fn jxl_decoder(bytes: &[u8]) -> Result<JxlDecoder<Cursor<&[u8]>>, ImageError> {
    JxlDecoder::new(Cursor::new(bytes)).map_err(decode_err)
}

/// Decodes the first frame to RGBA8.
pub fn decode_rgba(bytes: &[u8]) -> Result<RgbaImage, ImageError> {
    let kind = sniff(bytes).ok_or(ImageError::UnknownFormat)?.kind;
    let img = match format_of(kind)? {
        Some(format) => image::load_from_memory_with_format(bytes, format).map_err(decode_err)?,
        None => DynamicImage::from_decoder(jxl_decoder(bytes)?).map_err(decode_err)?,
    };
    Ok(img.into_rgba8())
}

/// Reads the header only; no pixel decode.
pub fn dimensions(bytes: &[u8]) -> Result<(u32, u32), ImageError> {
    let kind = sniff(bytes).ok_or(ImageError::UnknownFormat)?.kind;
    match format_of(kind)? {
        Some(format) => ImageReader::with_format(Cursor::new(bytes), format)
            .into_dimensions()
            .map_err(decode_err),
        None => Ok(jxl_decoder(bytes)?.dimensions()),
    }
}

/// JPEG has no alpha, so the alpha channel is dropped. Quality 100 is the
/// Kotlin re-encode setting.
pub fn encode_jpeg_q100(img: &RgbaImage) -> Result<Vec<u8>, ImageError> {
    let rgb: Vec<u8> = img.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let rgb = RgbImage::from_raw(img.width(), img.height(), rgb)
        .ok_or_else(|| ImageError::Encode("pixel buffer size mismatch".into()))?;
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 100)
        .encode_image(&rgb)
        .map_err(|e| ImageError::Encode(e.to_string()))?;
    Ok(out)
}

/// GIF is always animated-supported. WebP only when the VP8X animation bit is
/// set. HEIF and AVIF are never animated-supported (documented capability change).
pub fn is_animated_supported(sniff: Option<Sniff>) -> bool {
    match sniff {
        Some(Sniff { kind: ImageKind::Gif, .. }) => true,
        Some(Sniff { kind: ImageKind::Webp, animated }) => animated,
        _ => false,
    }
}

/// Double-page rule: strictly wider than tall.
pub fn is_wide(width: u32, height: u32) -> bool {
    width > height
}

/// Wide check from header dimensions only.
pub fn is_wide_image(bytes: &[u8]) -> Result<bool, ImageError> {
    dimensions(bytes).map(|(w, h)| is_wide(w, h))
}

/// Which half of a spread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    fn swapped(self) -> Self {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

/// Half `part` of a `width` x `height` image as a crop rectangle `(x, 0, width / 2, height)`. For an odd width
/// the middle column belongs to neither half.
fn half_rect(width: u32, side: Side) -> (u32, u32) {
    let half = width / 2;
    (if side == Side::Right { width - half } else { 0 }, half)
}

/// `splitInHalf`: the `W/2 x H` half of a spread (`5 x 1` Right gives columns 3 and 4).
pub fn split_in_half(img: &RgbaImage, side: Side) -> RgbaImage {
    let (x, half) = half_rect(img.width(), side);
    imageops::crop_imm(img, x, 0, half, img.height()).to_image()
}

/// Which half the pager shows for a page of a spread (`PagerPageHolder`): a left-to-right viewer shows the
/// first (non-inserted) half on the left and the inserted page on the right; every other viewer is the
/// reverse. `dual_page_invert` swaps the result.
pub fn pager_split_side(left_to_right_viewer: bool, is_insert_page: bool, dual_page_invert: bool) -> Side {
    let side = if left_to_right_viewer == is_insert_page { Side::Right } else { Side::Left };
    if dual_page_invert { side.swapped() } else { side }
}

/// One part of a tall image: rows `top..bottom`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TallPart {
    pub index: u32,
    pub top: u32,
    pub bottom: u32,
}

/// `isTallImage`: `H / W > 3` with integer division (500 x 2000 splits, 500 x 1999 does not).
pub fn is_tall(width: u32, height: u32) -> bool {
    width > 0 && height / width > 3
}

/// The `splitData` loop. `optimal_height` is twice the largest monitor side in pixels. The last part takes the
/// remainder, and an image exactly `optimal_height` tall is one part.
pub fn tall_split_plan(height: u32, optimal_height: u32) -> Vec<TallPart> {
    if height == 0 {
        return Vec::new();
    }
    let part_count = (height - 1) / optimal_height.max(1) + 1;
    let each = height / part_count;
    (0..part_count)
        .map(|index| {
            let top = index * each;
            let bottom = if index == part_count - 1 { height } else { top + each };
            TallPart { index, top, bottom }
        })
        .collect()
}

/// `007__001.jpg` for prefix `007` and index 0 (`%03d`, so four digits past 999).
pub fn split_part_name(prefix: &str, index: u32) -> String {
    format!("{prefix}__{:03}.jpg", index + 1)
}

fn io_err(e: std::io::Error) -> ImageError {
    ImageError::Encode(e.to_string())
}

/// `splitTallImage`: splits the page file `src` into `<prefix>__NNN.jpg` parts (JPEG quality 100) in `dst_dir`
/// and removes `src`. Returns the part paths, or an empty list when nothing needed splitting (animated or not
/// tall). A failure removes every part for the prefix, keeps `src` and returns the error; the download caller
/// ignores it, as the Kotlin ignores the boolean.
pub fn split_tall(
    src: &Path,
    dst_dir: &Path,
    prefix: &str,
    optimal_height: u32,
) -> Result<Vec<PathBuf>, ImageError> {
    let bytes = std::fs::read(src).map_err(|e| ImageError::Decode(e.to_string()))?;
    if is_animated_supported(sniff(&bytes)) {
        return Ok(Vec::new());
    }
    let (w, h) = dimensions(&bytes)?;
    if !is_tall(w, h) {
        return Ok(Vec::new());
    }
    let plan = tall_split_plan(h, optimal_height);
    let result = write_parts(&bytes, dst_dir, prefix, &plan);
    match result {
        Ok(paths) => {
            std::fs::remove_file(src).map_err(io_err)?;
            Ok(paths)
        }
        Err(e) => {
            for part in &plan {
                let _ = std::fs::remove_file(dst_dir.join(split_part_name(prefix, part.index)));
            }
            Err(e)
        }
    }
}

fn write_parts(bytes: &[u8], dst_dir: &Path, prefix: &str, plan: &[TallPart]) -> Result<Vec<PathBuf>, ImageError> {
    let img = decode_rgba(bytes)?;
    std::fs::create_dir_all(dst_dir).map_err(io_err)?;
    let mut paths = Vec::with_capacity(plan.len());
    for part in plan {
        let strip = imageops::crop_imm(&img, 0, part.top, img.width(), part.bottom - part.top).to_image();
        let path = dst_dir.join(split_part_name(prefix, part.index));
        let temp = path.with_extension("jpg.tmp");
        std::fs::write(&temp, encode_jpeg_q100(&strip)?).map_err(io_err)?;
        if let Err(e) = std::fs::rename(&temp, &path) {
            let _ = std::fs::remove_file(&temp);
            return Err(io_err(e));
        }
        paths.push(path);
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba};

    fn padded(prefix: &[u8]) -> Vec<u8> {
        let mut v = prefix.to_vec();
        v.resize(HEAD_LEN, 0);
        v
    }

    fn webp_head(flags: u8) -> Vec<u8> {
        let mut v = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0".to_vec();
        v.push(flags);
        padded(&v)
    }

    fn encode(img: DynamicImage, fmt: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, fmt).unwrap();
        out.into_inner()
    }

    fn png_with_alpha(w: u32, h: u32) -> Vec<u8> {
        let img = RgbaImage::from_fn(w, h, |x, y| Rgba([x as u8 * 40, y as u8 * 60, 128, 255]));
        encode(DynamicImage::ImageRgba8(img), ImageFormat::Png)
    }

    /// Header-only JPEG: the bytes end right after the SOS segment, so the scan data is missing.
    fn jpeg_headers_only(w: u32, h: u32) -> Vec<u8> {
        let full = encode(DynamicImage::ImageRgb8(RgbImage::new(w, h)), ImageFormat::Jpeg);
        let sos = full.windows(2).position(|p| p == [0xFF, 0xDA]).unwrap();
        let len = u16::from_be_bytes([full[sos + 2], full[sos + 3]]) as usize;
        full[..sos + 2 + len].to_vec()
    }

    fn still(kind: ImageKind) -> Option<Sniff> {
        Some(Sniff { kind, animated: false })
    }

    #[test]
    fn sniff_jpeg_png_gif() {
        assert_eq!(sniff(&padded(&[0xFF, 0xD8, 0xFF, 0xE0])), still(ImageKind::Jpeg));
        assert_eq!(sniff(&padded(&[0xFF, 0xD8, 0xFF])[..31]), still(ImageKind::Jpeg));
        assert_eq!(sniff(&padded(&[0xFF, 0xD8, 0x00])), None);
        assert_eq!(sniff(&padded(b"\x89PNG\r\n\x1a\n")), still(ImageKind::Png));
        assert_eq!(
            sniff(b"GIF89a"),
            Some(Sniff { kind: ImageKind::Gif, animated: true })
        );
    }

    #[test]
    fn sniff_webp_animation_bit() {
        assert_eq!(sniff(&webp_head(0x00)), still(ImageKind::Webp));
        assert_eq!(
            sniff(&webp_head(0x02)),
            Some(Sniff { kind: ImageKind::Webp, animated: true })
        );
        assert_eq!(sniff(&webp_head(0x10)), still(ImageKind::Webp));
        assert_eq!(sniff(&padded(b"RIFF\0\0\0\0WAVE")), None);
    }

    #[test]
    fn sniff_jxl_container_and_codestream() {
        assert_eq!(sniff(&padded(JXL_CONTAINER)), still(ImageKind::Jxl));
        assert_eq!(sniff(&padded(&[0xFF, 0x0A])), still(ImageKind::Jxl));
    }

    #[test]
    fn sniff_ftyp_brands() {
        assert_eq!(sniff(&padded(b"\0\0\0\x18ftypheic")), still(ImageKind::Heif));
        assert_eq!(sniff(&padded(b"\0\0\0\x1cftypavif")), still(ImageKind::Avif));
        assert_eq!(sniff(&padded(b"\0\0\0\x0cftyp")), None);
        // headerSize 8 gives maxOffset 4, so the brand at offset 8 is out of range.
        assert_eq!(sniff(&padded(b"\0\0\0\x08ftypheic")), None);
    }

    #[test]
    fn sniff_empty_and_unknown() {
        assert_eq!(sniff(&[]), None);
        assert_eq!(sniff(&padded(b"hello")), None);
    }

    #[test]
    fn mime_and_extension_mapping() {
        assert_eq!(kind_from_mime("image/jpeg"), Some(ImageKind::Jpeg));
        assert_eq!(kind_from_mime("image/jpg"), None);
        assert_eq!(kind_from_mime("IMAGE/JPEG"), None);
        assert_eq!(ImageKind::Jpeg.extension(), "jpg");
        assert_eq!(ImageKind::Jxl.mime(), "image/jxl");
        assert_eq!(ImageKind::ALL.len(), 7);
    }

    #[test]
    fn is_image_name_rules() {
        let no_sniff = || None;
        let jpeg_head = || Some(padded(&[0xFF, 0xD8, 0xFF]));
        assert!(!is_image_name("x.JPG", no_sniff));
        assert!(is_image_name("x.JPG", jpeg_head));
        assert!(is_image_name("x.jpg", no_sniff));
        assert!(!is_image_name("x.jpeg", no_sniff));
        assert!(is_image_name("x.jpeg", jpeg_head));
        assert!(is_image_name("x", jpeg_head));
    }

    #[test]
    fn extension_for_rules() {
        let no_head = || None;
        let png_head = || Some(padded(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(extension_for(Some("image/jpeg"), no_head), "jpg");
        assert_eq!(extension_for(Some("image/webp"), no_head), "webp");
        assert_eq!(extension_for(None, no_head), "jpg");
        assert_eq!(extension_for(None, png_head), "png");
        // Unknown MIME falls back to the sniff.
        assert_eq!(extension_for(Some("image/jpg"), png_head), "png");
        assert_eq!(extension_for(Some("image/jpg"), no_head), "jpg");
    }

    #[test]
    fn decode_png_roundtrip_keeps_pixels() {
        let img = decode_rgba(&png_with_alpha(4, 3)).unwrap();
        assert_eq!((img.width(), img.height()), (4, 3));
        assert_eq!(img.get_pixel(1, 2), &Rgba([40, 120, 128, 255]));
    }

    #[test]
    fn decode_jpeg_gif_webp_fixtures() {
        let rgb = DynamicImage::ImageRgb8(RgbImage::new(5, 2));
        let rgba = DynamicImage::ImageRgba8(RgbaImage::new(5, 2));
        for bytes in [
            encode(rgb, ImageFormat::Jpeg),
            encode(rgba.clone(), ImageFormat::Gif),
            encode(rgba, ImageFormat::WebP),
        ] {
            let img = decode_rgba(&bytes).unwrap();
            assert_eq!((img.width(), img.height()), (5, 2));
        }
    }

    #[test]
    fn decode_jxl_codestream_and_container_fixtures() {
        for bytes in [
            &include_bytes!("../../tests/fixtures/small.jxl")[..],
            &include_bytes!("../../tests/fixtures/small_container.jxl")[..],
        ] {
            assert_eq!(dimensions(bytes).unwrap(), (6, 4));
            let img = decode_rgba(bytes).unwrap();
            assert_eq!((img.width(), img.height()), (6, 4));
        }
    }

    #[test]
    fn truncated_jxl_is_a_decode_error_not_a_panic() {
        let full = include_bytes!("../../tests/fixtures/small_container.jxl");
        assert!(matches!(decode_rgba(&full[..40]), Err(ImageError::Decode(_))));
    }

    #[cfg(not(feature = "heif"))]
    #[test]
    fn avif_and_heif_report_feature_disabled_by_default() {
        for head in [&b"\0\0\0\x1cftypavif"[..], &b"\0\0\0\x18ftypheic"[..]] {
            match decode_rgba(&padded(head)) {
                Err(ImageError::Decode(msg)) => assert_eq!(msg, "feature disabled"),
                other => panic!("expected Decode, got {other:?}"),
            }
        }
    }

    #[test]
    fn heif_is_explicitly_unsupported() {
        let head = padded(b"\0\0\0\x18ftypheic");
        let want = if cfg!(feature = "heif") { "heif decoder not built" } else { "feature disabled" };
        assert!(matches!(decode_rgba(&head), Err(ImageError::Decode(m)) if m == want));
        assert!(matches!(dimensions(&head), Err(ImageError::Decode(m)) if m == want));
    }

    #[test]
    fn decode_errors() {
        assert!(matches!(decode_rgba(b"hello"), Err(ImageError::UnknownFormat)));
        let full = encode(DynamicImage::ImageRgb8(RgbImage::new(64, 64)), ImageFormat::Jpeg);
        assert!(matches!(decode_rgba(&full[..full.len() / 2]), Err(ImageError::Decode(_))));
    }

    #[test]
    fn dimensions_read_headers() {
        assert_eq!(dimensions(&png_with_alpha(4, 3)).unwrap(), (4, 3));
        assert!(matches!(dimensions(b"hello"), Err(ImageError::UnknownFormat)));
    }

    #[test]
    fn wide_detection_uses_headers_only() {
        let truncated = jpeg_headers_only(1200, 800);
        assert_eq!(dimensions(&truncated).unwrap(), (1200, 800));
        assert!(is_wide_image(&truncated).unwrap());
    }

    #[test]
    fn encode_jpeg_drops_alpha_and_keeps_size() {
        let src = decode_rgba(&png_with_alpha(4, 3)).unwrap();
        let jpg = encode_jpeg_q100(&src).unwrap();
        assert_eq!(sniff(&jpg).map(|s| s.kind), Some(ImageKind::Jpeg));
        let back = decode_rgba(&jpg).unwrap();
        assert_eq!((back.width(), back.height()), (4, 3));
    }

    #[test]
    fn animated_support_rules() {
        assert!(is_animated_supported(Some(Sniff { kind: ImageKind::Gif, animated: true })));
        assert!(is_animated_supported(Some(Sniff { kind: ImageKind::Webp, animated: true })));
        assert!(!is_animated_supported(Some(Sniff { kind: ImageKind::Webp, animated: false })));
        assert!(!is_animated_supported(Some(Sniff { kind: ImageKind::Heif, animated: true })));
        assert!(!is_animated_supported(Some(Sniff { kind: ImageKind::Avif, animated: true })));
        assert!(!is_animated_supported(Some(Sniff { kind: ImageKind::Png, animated: false })));
        assert!(!is_animated_supported(None));
    }

    #[test]
    fn wide_is_strictly_wider_than_tall() {
        assert!(is_wide(1200, 800));
        assert!(!is_wide(800, 800));
        assert!(!is_wide(800, 1200));
        // The edge-case table lists 1000 x 800 as false. That contradicts width > height and is not applied.
        assert!(is_wide(1000, 800));
    }

    /// A `w` x 1 image whose red channel is the column number.
    fn columns(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, _| Rgba([(x as u8).wrapping_mul(10), 0, 0, 255]))
    }

    fn first_row_reds(img: &RgbaImage) -> Vec<u8> {
        (0..img.width()).map(|x| img.get_pixel(x, 0)[0]).collect()
    }

    #[test]
    fn split_in_half_takes_the_column_ranges_of_the_spec() {
        let img = columns(25, 2);
        assert_eq!(first_row_reds(&split_in_half(&img, Side::Left)), (0..12).map(|x| x * 10).collect::<Vec<u8>>());
        // 25 wide: Right starts at 25 - 12 = 13, the middle column 12 is dropped.
        assert_eq!(first_row_reds(&split_in_half(&img, Side::Right)), (13..25).map(|x| x * 10).collect::<Vec<u8>>());
        let five = columns(5, 1);
        let right = split_in_half(&five, Side::Right);
        assert_eq!((right.width(), right.height()), (2, 1));
        assert_eq!(first_row_reds(&right), vec![30, 40]);
        assert_eq!(first_row_reds(&split_in_half(&five, Side::Left)), vec![0, 10]);
        let wide = split_in_half(&RgbaImage::new(1000, 600), Side::Right);
        assert_eq!((wide.width(), wide.height()), (500, 600));
    }

    #[test]
    fn split_output_encodes_as_jpeg_q100() {
        let jpg = encode_jpeg_q100(&split_in_half(&columns(20, 4), Side::Left)).unwrap();
        assert_eq!(sniff(&jpg).map(|s| s.kind), Some(ImageKind::Jpeg));
        assert_eq!(dimensions(&jpg).unwrap(), (10, 4));
    }

    #[test]
    fn pager_side_table_covers_all_eight_combinations() {
        use Side::{Left, Right};
        // (left_to_right_viewer, is_insert_page, invert) -> side
        let table = [
            (true, true, false, Right),
            (false, true, false, Left),
            (true, false, false, Left),
            (false, false, false, Right),
            (true, true, true, Left),
            (false, true, true, Right),
            (true, false, true, Right),
            (false, false, true, Left),
        ];
        for (l2r, insert, invert, want) in table {
            assert_eq!(pager_split_side(l2r, insert, invert), want, "{l2r} {insert} {invert}");
        }
    }

    fn plan(h: u32, optimal: u32) -> Vec<(u32, u32, u32)> {
        tall_split_plan(h, optimal).iter().map(|p| (p.index, p.top, p.bottom)).collect()
    }

    #[test]
    fn tall_split_plan_matches_section_10_4() {
        assert_eq!(plan(10000, 4320), vec![(0, 0, 3333), (1, 3333, 6666), (2, 6666, 10000)]);
        assert_eq!(plan(4320, 4320), vec![(0, 0, 4320)]);
        assert_eq!(plan(4321, 4320), vec![(0, 0, 2160), (1, 2160, 4321)]);
        assert!(plan(0, 4320).is_empty());
    }

    #[test]
    fn tall_trigger_is_integer_division_greater_than_three() {
        assert!(is_tall(500, 2000));
        assert!(!is_tall(500, 1999));
        assert!(!is_tall(0, 1000));
    }

    #[test]
    fn split_part_names_use_three_digits_then_grow() {
        assert_eq!(split_part_name("007", 0), "007__001.jpg");
        assert_eq!(split_part_name("007", 999), "007__1000.jpg");
    }

    fn write_png(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
        let path = dir.join(name);
        let img = RgbaImage::from_fn(w, h, |x, y| Rgba([(x * 7) as u8, (y * 3) as u8, 128, 255]));
        std::fs::write(&path, encode(DynamicImage::ImageRgba8(img), ImageFormat::Png)).unwrap();
        path
    }

    #[test]
    fn split_tall_writes_parts_and_removes_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let src = write_png(dir.path(), "007.png", 10, 100);
        let parts = split_tall(&src, dir.path(), "007", 40).unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].file_name().unwrap(), "007__001.jpg");
        assert!(!src.exists());
        let heights: Vec<u32> = parts.iter().map(|p| dimensions(&std::fs::read(p).unwrap()).unwrap().1).collect();
        assert_eq!(heights, vec![33, 33, 34]);
    }

    #[test]
    fn split_tall_leaves_wide_enough_and_animated_images_alone() {
        let dir = tempfile::tempdir().unwrap();
        let squat = write_png(dir.path(), "a.png", 10, 39);
        assert!(split_tall(&squat, dir.path(), "a", 40).unwrap().is_empty());
        assert!(squat.exists());
        let gif = dir.path().join("b.gif");
        std::fs::write(&gif, b"GIF89a").unwrap();
        assert!(split_tall(&gif, dir.path(), "b", 40).unwrap().is_empty());
        assert!(gif.exists());
    }

    #[test]
    fn split_tall_failure_removes_the_parts_and_keeps_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let src = write_png(dir.path(), "007.png", 10, 100);
        // A folder where part 2 belongs makes the rename of that part fail after part 1 was written.
        std::fs::create_dir(dir.path().join("007__002.jpg")).unwrap();
        assert!(split_tall(&src, dir.path(), "007", 40).is_err());
        assert!(src.exists());
        assert!(!dir.path().join("007__001.jpg").exists());
        assert!(!dir.path().join("007__003.jpg").exists());
    }

    #[test]
    fn split_tall_keeps_a_corrupted_original() {
        let dir = tempfile::tempdir().unwrap();
        let src = write_png(dir.path(), "007.png", 10, 100);
        let bytes = std::fs::read(&src).unwrap();
        std::fs::write(&src, &bytes[..bytes.len() / 2]).unwrap();
        assert!(split_tall(&src, dir.path(), "007", 40).is_err());
        assert!(src.exists());
        assert!(!dir.path().join("007__001.jpg").exists());
    }
}
