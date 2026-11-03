//! JPEG, PNG, GIF, WebP and BMP through the `image` crate. GIF and WebP give their first frame.
//!
//! Every codec module has one `open` that reads the header and hands back a decoder, so the size is known before
//! any pixel is decoded and the same decoder then produces the pixels.

use std::io::Cursor;

use image::{ImageDecoder, ImageFormat, ImageReader};

pub(super) fn open(
    bytes: &[u8],
    format: ImageFormat,
) -> Result<Box<dyn ImageDecoder + '_>, String> {
    ImageReader::with_format(Cursor::new(bytes), format)
        .into_decoder()
        .map(|d| Box::new(d) as Box<dyn ImageDecoder + '_>)
        .map_err(|e| e.to_string())
}
