//! The cover image of a library notice (T-26a-127). The cover from the local cover cache is centre-cropped to a square,
//! scaled to 192 px, clipped to a circle and stored as PNG in the notification cache. Any failure gives `None`, and the
//! notice then has no image hint.
//!
//! Decision: decode, crop, scale, circle mask and PNG encode go through the `image` crate, which the app already uses for
//! covers (`feed::cover`). gdk-pixbuf would be the usual route, but this build's cairo has no PNG writer, so a cairo route would
//! need a new feature flag. The result is the same PNG.


#[cfg(test)]
use std::path::{Path, PathBuf};

#[cfg(test)]
use image::{Rgba, imageops::FilterType};

#[cfg(test)]
/// Side of the notification icon in pixels (T-26a-127).
pub const ICON_SIZE: u32 = 192;

#[cfg(test)]
/// Writes `cover` as a circle-cropped PNG named `name` inside `cache_dir` and returns its path. `None` when the cover
/// cannot be read or the PNG cannot be written.
pub fn circle_icon(cover: &Path, cache_dir: &Path, name: &str) -> Option<PathBuf> {
    let source = image::open(cover).ok()?;
    let side = source.width().min(source.height());
    let square = source.crop_imm((source.width() - side) / 2, (source.height() - side) / 2, side, side);
    let mut icon = square.resize_exact(ICON_SIZE, ICON_SIZE, FilterType::Triangle).to_rgba8();
    let radius = f64::from(ICON_SIZE) / 2.0;
    for (x, y, pixel) in icon.enumerate_pixels_mut() {
        let dx = f64::from(x) + 0.5 - radius;
        let dy = f64::from(y) + 0.5 - radius;
        if dx * dx + dy * dy > radius * radius {
            *pixel = Rgba([0, 0, 0, 0]);
        }
    }
    std::fs::create_dir_all(cache_dir).ok()?;
    let path = cache_dir.join(name);
    icon.save_with_format(&path, image::ImageFormat::Png).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    /// A solid red 300 by 200 PNG.
    fn source_png(dir: &Path) -> PathBuf {
        let path = dir.join("cover.png");
        RgbaImage::from_pixel(300, 200, Rgba([255, 0, 0, 255])).save(&path).unwrap();
        path
    }

    #[test]
    fn a_cover_becomes_a_192_pixel_circle_png_in_the_cache() {
        // T-26a-127
        let dir = tempfile::tempdir().unwrap();
        let cover = source_png(dir.path());
        let cache = dir.path().join("notifications");
        let icon = circle_icon(&cover, &cache, "library-1.png").expect("the icon is written");
        assert_eq!(icon, cache.join("library-1.png"));
        let decoded = image::open(&icon).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (ICON_SIZE, ICON_SIZE));
        let rgba = decoded.to_rgba8();
        assert_eq!(rgba.get_pixel(0, 0)[3], 0, "the corner is outside the circle");
        assert_eq!(rgba.get_pixel(96, 96)[3], 255, "the centre is inside the circle");
        assert_eq!(rgba.get_pixel(96, 96)[0], 255, "the cover colour is kept");
    }

    #[test]
    fn a_missing_cover_gives_no_icon_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("notifications");
        assert!(circle_icon(&dir.path().join("absent.jpg"), &cache, "x.png").is_none());
        assert!(!cache.join("x.png").exists());
    }

    #[test]
    fn a_cover_that_is_not_an_image_gives_no_icon() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("broken.png");
        std::fs::write(&cover, b"not an image").unwrap();
        assert!(circle_icon(&cover, &dir.path().join("cache"), "x.png").is_none());
    }
}
