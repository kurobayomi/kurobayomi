//! The backdrop behind the entry header (T-38a-014). Upstream: `MangaInfoHeader.kt`. The cover is drawn behind the header at
//! alpha 0.85 with a gradient from transparent to the window background. The blur is computed once per cover and cached by
//! the cover's url and modification time, so a changed cover is blurred again. Pure: the widget draws the result.

use image::RgbaImage;

/// The alpha of the blurred cover behind the header: strong enough to colour the whole header, the fade keeps the text
/// readable.
pub const BACKDROP_ALPHA: f64 = 0.85;

/// The alpha of the theme colour at the top of the header, under the cover, fading out downward.
pub const TINT_ALPHA: f64 = 0.2;

/// The blur strength in pixels on the shrunk copy (`BLUR_MAX_SIDE`): stretched over the header it is a soft wash of the
/// cover's colours with its big shapes still readable.
pub const BLUR_SIGMA_PX: f32 = 3.0;

/// The CSS class of the gradient layer over the backdrop. Its rule is in `theme/style.css`: clear at the top, the window
/// background at the bottom.
pub const FADE_CLASS: &str = "kuro-backdrop-fade";

/// The longest side of the picture that is blurred. The blur hides the detail, so a small copy is enough and the widget
/// stretches it over the header.
pub const BLUR_MAX_SIDE: u32 = 96;

/// Decodes a cover, shrinks it to `BLUR_MAX_SIDE` and blurs it: `(width, height, rgba)`. `None` for bytes that are not a
/// picture. Runs off the GTK thread.
pub fn prepare(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let small = image::load_from_memory(bytes).ok()?.thumbnail(BLUR_MAX_SIDE, BLUR_MAX_SIDE).to_rgba8();
    let (w, h) = small.dimensions();
    Some((w, h, blur_rgba(w, h, small.into_raw())?))
}

/// The cache key of a blurred cover. A new modification time gives a new key.
pub fn cache_key(cover_url: &str, last_modified: i64) -> String {
    format!("{cover_url}#{last_modified}")
}

/// Blurs an RGBA picture of `width` by `height` pixels. `None` when the byte count is not `width * height * 4`.
pub fn blur_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Option<Vec<u8>> {
    let picture = RgbaImage::from_raw(width, height, rgba)?;
    Some(image::imageops::blur(&picture, BLUR_SIGMA_PX).into_raw())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backdrop_shows_and_the_tint_stays_faint() {
        assert_eq!(BACKDROP_ALPHA, 0.85);
        assert_eq!(TINT_ALPHA, 0.2);
    }

    #[test]
    fn the_cache_key_changes_with_the_cover_time() {
        assert_ne!(cache_key("https://x/c.jpg", 1), cache_key("https://x/c.jpg", 2));
        assert_eq!(cache_key("https://x/c.jpg", 1), cache_key("https://x/c.jpg", 1));
    }

    #[test]
    fn a_flat_picture_stays_flat_and_a_bad_size_is_refused() {
        let flat = vec![200u8; 8 * 8 * 4];
        let out = blur_rgba(8, 8, flat.clone()).expect("sized");
        assert_eq!(out.len(), flat.len());
        assert!(out.iter().all(|&b| b == 200), "blurring a flat picture changes nothing");
        assert_eq!(blur_rgba(8, 8, vec![0; 10]), None);
    }

    #[test]
    fn one_bright_pixel_spreads_into_its_neighbours() {
        let (w, h) = (9u32, 9u32);
        let mut src = vec![0u8; (w * h * 4) as usize];
        let centre = ((4 * w + 4) * 4) as usize;
        src[centre..centre + 4].copy_from_slice(&[255, 255, 255, 255]);
        let out = blur_rgba(w, h, src).expect("sized");
        let neighbour = ((4 * w + 5) * 4) as usize;
        assert!(out[neighbour] > 0, "the blur reaches the next pixel");
        assert!(out[centre] < 255, "the centre gets dimmer");
    }

    #[test]
    fn prepare_shrinks_and_blurs_a_cover_and_refuses_other_bytes() {
        let mut png = Vec::new();
        let picture = image::RgbaImage::from_pixel(300, 400, image::Rgba([10, 20, 30, 255]));
        picture.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let (w, h, rgba) = prepare(&png).expect("a picture");
        assert!(w.max(h) <= BLUR_MAX_SIDE && w < h, "{w}x{h}");
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        assert!(prepare(b"not a picture").is_none());
    }
}
