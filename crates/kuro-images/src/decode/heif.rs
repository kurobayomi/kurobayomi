//! HEIF and HEIC stills through libheif (T-34a-029). Built with the `heif` feature only;
//! without it every HEIF file is `DecodeInit` ("Failed to initialize decoder"). Animated HEIF is not decoded: the
//! primary image is the still.

use image::RgbaImage;

use crate::error::ImageError;

/// The primary image of `bytes` as RGBA8. libheif applies the rotation and mirror transforms itself, so no EXIF
/// turn follows. A file libheif cannot open is `DecodeInit`; a picture it cannot decode is `DecodeFailed`; one with
/// more than `max_pixels` pixels is `TooLarge` before any pixel is decoded.
#[cfg(feature = "heif")]
pub(super) fn decode(bytes: &[u8], max_pixels: u64) -> Result<RgbaImage, ImageError> {
    use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};

    let init = |e: libheif_rs::HeifError| {
        tracing::debug!(error = %e, "heif header unreadable");
        ImageError::DecodeInit
    };
    let failed = |e: libheif_rs::HeifError| {
        tracing::debug!(error = %e, "heif decode failed");
        ImageError::DecodeFailed
    };
    let context = HeifContext::read_from_bytes(bytes).map_err(init)?;
    let handle = context.primary_image_handle().map_err(init)?;
    let (width, height) = (handle.width(), handle.height());
    if width == 0 || height == 0 {
        return Err(ImageError::DecodeInit);
    }
    if width > i32::MAX as u32 || height > i32::MAX as u32 || u64::from(width) * u64::from(height) > max_pixels {
        return Err(ImageError::TooLarge);
    }
    let decoded = LibHeif::new()
        .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgba), None)
        .map_err(failed)?;
    let plane = decoded.planes().interleaved.ok_or(ImageError::DecodeFailed)?;
    let row = plane.width as usize * 4;
    if plane.stride < row || plane.data.len() < plane.stride * (plane.height as usize - 1) + row {
        return Err(ImageError::DecodeFailed);
    }
    let mut rgba = Vec::with_capacity(row * plane.height as usize);
    for y in 0..plane.height as usize {
        rgba.extend_from_slice(&plane.data[y * plane.stride..y * plane.stride + row]);
    }
    RgbaImage::from_raw(plane.width, plane.height, rgba).ok_or(ImageError::DecodeFailed)
}

#[cfg(not(feature = "heif"))]
pub(super) fn decode(_bytes: &[u8], _max_pixels: u64) -> Result<RgbaImage, ImageError> {
    Err(ImageError::DecodeInit)
}
