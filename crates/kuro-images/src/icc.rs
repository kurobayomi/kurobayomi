//! Display ICC profile (deviation D-3, T-34a-031).
//!
//! The profile is read once when the setting changes ([`load_setting`]) and carried in `DecodeOptions`. Pictures
//! are taken from sRGB to the display profile after decoding. A profile that cannot be read or used gives `None`
//! and no error is shown: the picture is then shown without colour management. Colour management itself needs the
//! `icc` feature (lcms2); without it no profile ever loads.

use std::path::Path;
use std::sync::Arc;

use crate::decode::Decoded;

/// The sRGB-to-display transform of one display profile.
pub struct DisplayProfile {
    #[cfg(feature = "icc")]
    // lcms2 transforms are `Send` but not `Sync`; the profile is shared between decode threads, so it is locked
    transform: std::sync::Mutex<lcms2::Transform<[u8; 4], [u8; 4]>>,
}

impl std::fmt::Debug for DisplayProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DisplayProfile")
    }
}

/// The profile for the value of `pref_display_profile_key`: an empty value or any failure is `None`.
pub fn load_setting(value: &str) -> Option<Arc<DisplayProfile>> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let profile = DisplayProfile::load(Path::new(value));
    if profile.is_none() {
        tracing::debug!(path = value, "display profile not loaded");
    }
    profile.map(Arc::new)
}

#[cfg(feature = "icc")]
impl DisplayProfile {
    /// Reads the profile at `path` and builds the transform. `None` when the file cannot be read or is not a profile.
    pub fn load(path: &Path) -> Option<Self> {
        use lcms2::{Intent, PixelFormat, Profile, Transform};
        let display = Profile::new_file(path).ok()?;
        let srgb = Profile::new_srgb();
        let transform = Transform::new(
            &srgb,
            PixelFormat::RGBA_8,
            &display,
            PixelFormat::RGBA_8,
            Intent::Perceptual,
        )
        .ok()?;
        Some(Self { transform: std::sync::Mutex::new(transform) })
    }

    /// The picture converted to the display profile. Alpha is copied as it is.
    pub fn apply(&self, picture: &Decoded) -> Decoded {
        let src: Vec<[u8; 4]> = picture
            .rgba
            .chunks_exact(4)
            .map(|px| [px[0], px[1], px[2], px[3]])
            .collect();
        let mut dst = vec![[0u8; 4]; src.len()];
        self.transform.lock().unwrap_or_else(|e| e.into_inner()).transform_pixels(&src, &mut dst);
        // lcms2 does not write the extra alpha channel here, so the source alpha is put back.
        for (out, input) in dst.iter_mut().zip(&src) {
            out[3] = input[3];
        }
        Decoded {
            width: picture.width,
            height: picture.height,
            rgba: bytes::Bytes::from(dst.concat()),
            is_sampled: picture.is_sampled,
        }
    }
}

#[cfg(not(feature = "icc"))]
impl DisplayProfile {
    /// Colour management is not built, so no profile loads.
    pub fn load(_path: &Path) -> Option<Self> {
        None
    }

    /// The picture as it is.
    pub fn apply(&self, picture: &Decoded) -> Decoded {
        picture.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "icc")]
    use bytes::Bytes;
    #[cfg(feature = "icc")]
    use lcms2::Profile;

    #[test]
    fn an_empty_or_missing_setting_gives_no_profile() {
        assert!(load_setting("").is_none());
        assert!(load_setting("   ").is_none());
        assert!(load_setting("/no/such/display.icc").is_none());
    }

    #[test]
    fn a_file_that_is_not_a_profile_gives_no_profile_from_the_setting() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("junk.icc");
        std::fs::write(&path, b"not a profile at all")?;
        assert!(load_setting(path.to_str().unwrap_or_default()).is_none());
        Ok(())
    }

    #[cfg(feature = "icc")]
    fn one_pixel(rgba: [u8; 4]) -> Decoded {
        Decoded {
            width: 1,
            height: 1,
            rgba: Bytes::copy_from_slice(&rgba),
            is_sampled: false,
        }
    }

    #[cfg(feature = "icc")]
    #[test]
    fn an_srgb_display_profile_leaves_the_colours_alone() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("display.icc");
        let bytes = Profile::new_srgb().icc().map_err(std::io::Error::other)?;
        std::fs::write(&path, bytes)?;
        let profile = DisplayProfile::load(&path)
            .ok_or_else(|| std::io::Error::other("profile not loaded"))?;
        let out = profile.apply(&one_pixel([200, 40, 10, 128]));
        for (got, want) in out.rgba.iter().zip([200u8, 40, 10, 128]) {
            assert!(got.abs_diff(want) <= 2, "{:?}", out.rgba);
        }
        Ok(())
    }

    #[cfg(feature = "icc")]
    #[test]
    fn a_file_that_is_not_a_profile_gives_no_profile() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("junk.icc");
        std::fs::write(&path, b"not a profile at all")?;
        assert!(DisplayProfile::load(&path).is_none());
        assert!(DisplayProfile::load(&dir.path().join("missing.icc")).is_none());
        Ok(())
    }
}
