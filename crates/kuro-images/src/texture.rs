//! GTK-free description of a `gdk::MemoryTexture` (T-34a-036).
//!
//! `kuro-images` has no GTK. The GTK layer turns this into a texture on the main thread:
//!
//! ```text
//! gdk::MemoryTexture::new(spec.width, spec.height, gdk::MemoryFormat::R8g8b8a8,
//!                         &glib::Bytes::from(&spec.pixels[..]), spec.stride)
//! ```
//!
//! Building the spec copies nothing: `pixels` shares the decoded buffer. The only copy is the one
//! `glib::Bytes` makes on the main thread, so the main thread never decodes and never waits for the network.

use bytes::Bytes;

use crate::decode::Decoded;

/// The one pixel layout the decoders produce, `GDK_MEMORY_R8G8B8A8` (straight alpha).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureFormat {
    R8g8b8a8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureSpec {
    pub width: i32,
    pub height: i32,
    pub format: TextureFormat,
    /// Bytes per row, always `width * 4`. GTK needs at least that.
    pub stride: usize,
    pub pixels: Bytes,
}

impl TextureSpec {
    /// The spec of a decoded image. `decode` refuses sides above `i32::MAX`, so the casts are lossless.
    pub fn from_decoded(image: &Decoded) -> Self {
        Self {
            width: image.width as i32,
            height: image.height as i32,
            format: TextureFormat::R8g8b8a8,
            stride: image.width as usize * 4,
            pixels: image.rgba.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_uses_rgba8_with_the_decoded_stride() {
        let image = Decoded {
            width: 3,
            height: 2,
            rgba: Bytes::from(vec![0u8; 24]),
            is_sampled: false,
        };
        let spec = TextureSpec::from_decoded(&image);
        assert_eq!((spec.width, spec.height), (3, 2));
        assert_eq!(spec.format, TextureFormat::R8g8b8a8);
        assert_eq!(spec.stride, 12);
        assert_eq!(spec.pixels.len(), spec.stride * spec.height as usize);
    }

    #[test]
    fn pixels_share_the_decoded_buffer() {
        let rgba = Bytes::from(vec![7u8; 16]);
        let image = Decoded {
            width: 2,
            height: 2,
            rgba: rgba.clone(),
            is_sampled: false,
        };
        let spec = TextureSpec::from_decoded(&image);
        assert_eq!(spec.pixels.as_ptr(), rgba.as_ptr());
    }

    #[test]
    fn a_spec_can_cross_threads() {
        fn send<T: Send + Sync + 'static>() {}
        send::<TextureSpec>();
        send::<Decoded>();
    }
}
