//! Crop-borders rule (a port of `borders.cpp` of the `image-decoder` library).
//!
//! The rule works on the grayscale plane. A border is a run of rows or columns at an edge that holds only one
//! kind of pixel, either "black" or "white". The thresholds keep the names and overlap of the original: black is
//! below 191 and white is above 63, so mid greys count as both.

use image::{RgbaImage, imageops};

const FILLED_RATIO_LIMIT: f64 = 0.0025;
const BLACK_BELOW: u8 = 191;
const WHITE_ABOVE: u8 = 63;

/// The rectangle left after the borders are cut off: `(left, top, width, height)`.
pub type Rect = (u32, u32, u32, u32);

fn is_black(p: u8) -> bool {
    p < BLACK_BELOW
}

fn is_white(p: u8) -> bool {
    p > WHITE_ABOVE
}

fn limit(side: u32) -> u32 {
    (f64::from(side) * FILLED_RATIO_LIMIT / 2.0).round() as u32
}

/// Which pixels end a border, chosen from the edge line: `None` when the edge holds both kinds (no crop).
fn detector(black: u32, white: u32, limit: u32) -> Option<fn(u8) -> bool> {
    if white > limit && black > limit {
        None
    } else if black > limit {
        Some(is_white)
    } else {
        Some(is_black)
    }
}

struct Plane<'a> {
    px: &'a [u8],
    w: u32,
}

impl Plane<'_> {
    fn at(&self, x: u32, y: u32) -> u8 {
        self.px[(y * self.w + x) as usize]
    }

    fn row_counts(&self, y: u32) -> (u32, u32) {
        (0..self.w).step_by(2).fold((0, 0), |(b, w), x| {
            let p = self.at(x, y);
            (b + u32::from(is_black(p)), w + u32::from(is_white(p)))
        })
    }

    fn col_counts(&self, x: u32, top: u32, bottom: u32) -> (u32, u32) {
        (top..bottom).step_by(2).fold((0, 0), |(b, w), y| {
            let p = self.at(x, y);
            (b + u32::from(is_black(p)), w + u32::from(is_white(p)))
        })
    }

    fn row_hits(&self, y: u32, hit: fn(u8) -> bool) -> u32 {
        (0..self.w)
            .step_by(2)
            .filter(|&x| hit(self.at(x, y)))
            .count() as u32
    }

    fn col_hits(&self, x: u32, top: u32, bottom: u32, hit: fn(u8) -> bool) -> u32 {
        (top..bottom)
            .step_by(2)
            .filter(|&y| hit(self.at(x, y)))
            .count() as u32
    }
}

fn find_top(g: &Plane, h: u32) -> u32 {
    let lim = limit(g.w);
    let (b, w) = g.row_counts(0);
    let Some(hit) = detector(b, w, lim) else {
        return 0;
    };
    (1..h).find(|&y| g.row_hits(y, hit) > lim).unwrap_or(0)
}

fn find_bottom(g: &Plane, h: u32) -> u32 {
    let lim = limit(g.w);
    let (b, w) = g.row_counts(h - 1);
    let Some(hit) = detector(b, w, lim) else {
        return h;
    };
    (1..h - 1)
        .rev()
        .find(|&y| g.row_hits(y, hit) > lim)
        .map_or(h, |y| y + 1)
}

fn find_left(g: &Plane, h: u32, top: u32, bottom: u32) -> u32 {
    let lim = limit(h);
    let (b, w) = g.col_counts(0, top, bottom);
    let Some(hit) = detector(b, w, lim) else {
        return 0;
    };
    (1..g.w)
        .find(|&x| g.col_hits(x, top, bottom, hit) > lim)
        .unwrap_or(0)
}

fn find_right(g: &Plane, h: u32, top: u32, bottom: u32) -> u32 {
    let lim = limit(h);
    let (b, w) = g.col_counts(g.w - 1, top, bottom);
    let Some(hit) = detector(b, w, lim) else {
        return g.w;
    };
    (1..g.w - 1)
        .rev()
        .find(|&x| g.col_hits(x, top, bottom, hit) > lim)
        .map_or(g.w, |x| x + 1)
}

/// The part of `image` inside its borders. `None` when nothing can be cut: no border, a mixed edge, or a
/// result with no area.
pub fn find_borders(image: &RgbaImage) -> Option<Rect> {
    let (w, h) = image.dimensions();
    if w < 2 || h < 2 {
        return None;
    }
    let gray = imageops::grayscale(image);
    let g = Plane {
        px: gray.as_raw(),
        w,
    };
    let top = find_top(&g, h);
    let bottom = find_bottom(&g, h);
    if bottom <= top {
        return None;
    }
    let left = find_left(&g, h, top, bottom);
    let right = find_right(&g, h, top, bottom);
    if right <= left || (left, top, right, bottom) == (0, 0, w, h) {
        return None;
    }
    Some((left, top, right - left, bottom - top))
}

/// Crops `image` to its borders, or returns it whole when there is nothing to cut.
pub fn crop(image: RgbaImage) -> RgbaImage {
    match find_borders(&image) {
        Some((x, y, w, h)) => imageops::crop_imm(&image, x, y, w, h).to_image(),
        None => image,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// A white canvas with a black block at `(x, y)` of size `w` by `h`.
    fn block(cw: u32, ch: u32, x: u32, y: u32, w: u32, h: u32) -> RgbaImage {
        let mut img = RgbaImage::from_pixel(cw, ch, Rgba([255, 255, 255, 255]));
        for py in y..y + h {
            for px in x..x + w {
                img.put_pixel(px, py, Rgba([0, 0, 0, 255]));
            }
        }
        img
    }

    #[test]
    fn white_border_around_black_content_is_found() {
        let img = block(100, 120, 20, 10, 60, 80);
        assert_eq!(find_borders(&img), Some((20, 10, 60, 80)));
        assert_eq!(crop(img).dimensions(), (60, 80));
    }

    #[test]
    fn black_border_around_white_content_is_found() {
        let mut img = RgbaImage::from_pixel(100, 100, Rgba([0, 0, 0, 255]));
        for y in 30..70 {
            for x in 10..90 {
                img.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        assert_eq!(find_borders(&img), Some((10, 30, 80, 40)));
    }

    #[test]
    fn an_image_without_borders_is_returned_whole() {
        let img = block(50, 50, 0, 0, 50, 50);
        assert_eq!(find_borders(&img), None);
        assert_eq!(crop(img).dimensions(), (50, 50));
    }

    #[test]
    fn a_blank_image_is_returned_whole() {
        let img = RgbaImage::from_pixel(40, 40, Rgba([255, 255, 255, 255]));
        assert_eq!(find_borders(&img), None);
    }

    #[test]
    fn a_mixed_edge_row_means_no_top_crop() {
        // The first row is half black and half white, so the top edge is not a uniform border.
        let mut img = block(100, 100, 20, 20, 60, 60);
        for x in 0..50 {
            img.put_pixel(x, 0, Rgba([0, 0, 0, 255]));
        }
        let (_, top, _, _) = find_borders(&img).unwrap();
        assert_eq!(top, 0);
    }
}
