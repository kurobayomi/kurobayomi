//! Colour maths for the colours the app makes itself: the colour a title's page takes from its cover, and the colours
//! of a theme. One rule keeps both readable: a colour that carries text or an icon is moved in lightness, its hue
//! kept, until it stands out enough from what is behind it ([`readable`]). Pure: no GTK here.

/// A colour as three bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub const BLACK: Rgb = Rgb::new(0, 0, 0);
pub const WHITE: Rgb = Rgb::new(255, 255, 255);

/// The contrast text needs against its background to read (WCAG, level AA): 4.5 to 1.
pub const TEXT_CONTRAST: f64 = 4.5;

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// The colour as a style sheet writes it: `#rrggbb`.
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// How light the colour is to the eye, 0 (black) to 1 (white): WCAG's relative luminance.
    pub fn luminance(self) -> f64 {
        let linear = |byte: u8| {
            let c = f64::from(byte) / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * linear(self.r) + 0.7152 * linear(self.g) + 0.0722 * linear(self.b)
    }

    /// The contrast between two colours, 1 (the same lightness) to 21 (black on white): WCAG's contrast ratio.
    pub fn contrast(self, other: Rgb) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// Hue in degrees (0 to 360), saturation and lightness (0 to 1). A grey has hue 0 and saturation 0.
    pub fn hsl(self) -> (f64, f64, f64) {
        let (r, g, b) = (f64::from(self.r) / 255.0, f64::from(self.g) / 255.0, f64::from(self.b) / 255.0);
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let lightness = (max + min) / 2.0;
        let spread = max - min;
        if spread < 1e-9 {
            return (0.0, 0.0, lightness);
        }
        let saturation = (spread / (1.0 - (2.0 * lightness - 1.0).abs())).min(1.0);
        let sixth = if max == r {
            ((g - b) / spread).rem_euclid(6.0)
        } else if max == g {
            (b - r) / spread + 2.0
        } else {
            (r - g) / spread + 4.0
        };
        (sixth * 60.0, saturation, lightness)
    }

    /// The colour of a hue (degrees), a saturation and a lightness (0 to 1).
    pub fn from_hsl(hue: f64, saturation: f64, lightness: f64) -> Rgb {
        let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
        let sixth = hue.rem_euclid(360.0) / 60.0;
        let second = chroma * (1.0 - (sixth.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = match sixth as u32 {
            0 => (chroma, second, 0.0),
            1 => (second, chroma, 0.0),
            2 => (0.0, chroma, second),
            3 => (0.0, second, chroma),
            4 => (second, 0.0, chroma),
            _ => (chroma, 0.0, second),
        };
        let lift = lightness - chroma / 2.0;
        let byte = |value: f64| ((value + lift) * 255.0).round().clamp(0.0, 255.0) as u8;
        Rgb::new(byte(r), byte(g), byte(b))
    }
}

/// `seed` made readable on `background`: the seed itself when it already stands out enough, else its hue and
/// saturation at the nearest lightness that does, toward white on a dark background and toward black on a light one.
pub fn readable(seed: Rgb, background: Rgb) -> Rgb {
    if seed.contrast(background) >= TEXT_CONTRAST {
        return seed;
    }
    let toward_white = WHITE.contrast(background) >= BLACK.contrast(background);
    let (hue, saturation, mut lightness) = seed.hsl();
    // a hundredth of the lightness at a time: at most a hundred steps to white or black, one of which always reads
    for _ in 0..100 {
        lightness = if toward_white { (lightness + 0.01).min(1.0) } else { (lightness - 0.01).max(0.0) };
        let moved = Rgb::from_hsl(hue, saturation, lightness);
        if moved.contrast(background) >= TEXT_CONTRAST {
            return moved;
        }
    }
    if toward_white { WHITE } else { BLACK }
}

/// What is written on `colour`: black or white, whichever stands out more.
pub fn on(colour: Rgb) -> Rgb {
    if BLACK.contrast(colour) >= WHITE.contrast(colour) { BLACK } else { WHITE }
}

/// A pixel counts as a colour, and not as grey, black or white, from this saturation on and between these lightnesses.
const VIVID_SATURATION: f64 = 0.25;
const VIVID_LIGHTNESS: std::ops::RangeInclusive<f64> = 0.12..=0.92;
/// The hues are counted in slices of this many degrees.
const HUE_SLICE: f64 = 15.0;
const HUE_SLICES: usize = 24;
/// A picture has a colour of its own when at least this share of its pixels is vivid. Under it the picture is grey with
/// a few coloured specks, and a page coloured by the specks would not look like its cover.
const VIVID_SHARE: f64 = 0.05;

/// The colour of a picture: the most common vivid hue of its pixels, as the average of the pixels of that hue. `rgba` is
/// four bytes a pixel; a transparent pixel does not count, and a vivid one counts by how saturated it is. `None` for a
/// picture with no colour to speak of (grey, black and white), or with no pixels.
pub fn dominant(rgba: &[u8]) -> Option<Rgb> {
    // per slice of hue: the weight, and the weighted sums of the three channels
    let mut slices = [(0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64); HUE_SLICES];
    let (mut seen, mut vivid) = (0_u32, 0_u32);
    for pixel in rgba.chunks_exact(4) {
        if pixel[3] < 128 {
            continue;
        }
        seen += 1;
        let colour = Rgb::new(pixel[0], pixel[1], pixel[2]);
        let (hue, saturation, lightness) = colour.hsl();
        if saturation < VIVID_SATURATION || !VIVID_LIGHTNESS.contains(&lightness) {
            continue;
        }
        vivid += 1;
        let slice = &mut slices[((hue / HUE_SLICE) as usize) % HUE_SLICES];
        slice.0 += saturation;
        slice.1 += saturation * f64::from(colour.r);
        slice.2 += saturation * f64::from(colour.g);
        slice.3 += saturation * f64::from(colour.b);
    }
    if seen == 0 || f64::from(vivid) < f64::from(seen) * VIVID_SHARE {
        return None;
    }
    let (weight, r, g, b) = slices.into_iter().max_by(|a, b| a.0.total_cmp(&b.0))?;
    if weight <= 0.0 {
        return None;
    }
    let byte = |sum: f64| (sum / weight).round().clamp(0.0, 255.0) as u8;
    Some(Rgb::new(byte(r), byte(g), byte(b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The app's dark window background, near enough.
    const DARK: Rgb = Rgb::new(31, 32, 36);
    const LIGHT: Rgb = Rgb::new(250, 250, 250);

    fn close(a: Rgb, b: Rgb) -> bool {
        let near = |x: u8, y: u8| x.abs_diff(y) <= 1;
        near(a.r, b.r) && near(a.g, b.g) && near(a.b, b.b)
    }

    /// The distance between two hues, round the circle.
    fn hue_gap(a: Rgb, b: Rgb) -> f64 {
        let gap = (a.hsl().0 - b.hsl().0).abs();
        gap.min(360.0 - gap)
    }

    /// A picture of these colours, each on this many pixels, all opaque.
    fn picture(parts: &[(Rgb, usize)]) -> Vec<u8> {
        parts.iter().flat_map(|(c, count)| std::iter::repeat_n([c.r, c.g, c.b, 255], *count)).flatten().collect()
    }

    #[test]
    fn luminance_and_contrast_are_the_wcag_ones() {
        assert!((WHITE.luminance() - 1.0).abs() < 1e-9);
        assert!(BLACK.luminance().abs() < 1e-9);
        assert!((WHITE.contrast(BLACK) - 21.0).abs() < 1e-6);
        assert!((BLACK.contrast(WHITE) - 21.0).abs() < 1e-6, "the order does not matter");
        // the grey known as the lightest that reads on white
        let grey = Rgb::new(0x76, 0x76, 0x76);
        assert!((4.5..4.6).contains(&grey.contrast(WHITE)), "{}", grey.contrast(WHITE));
        assert_eq!(Rgb::new(0x1f, 0x20, 0x24).hex(), "#1f2024");
    }

    #[test]
    fn a_colour_goes_to_hue_saturation_and_lightness_and_back() {
        assert_eq!(Rgb::new(255, 0, 0).hsl(), (0.0, 1.0, 0.5));
        assert_eq!(Rgb::from_hsl(210.0, 1.0, 0.5), Rgb::new(0, 128, 255));
        assert_eq!(Rgb::new(120, 120, 120).hsl().1, 0.0, "a grey has no saturation");
        for colour in [Rgb::new(47, 95, 208), Rgb::new(224, 168, 58), Rgb::new(240, 111, 176), Rgb::new(12, 200, 90), Rgb::new(3, 3, 3), WHITE] {
            let (hue, saturation, lightness) = colour.hsl();
            assert!(close(Rgb::from_hsl(hue, saturation, lightness), colour), "{colour:?}");
        }
    }

    #[test]
    fn a_colour_that_does_not_read_keeps_its_hue_and_changes_its_lightness() {
        // a dark blue on the dark background: lightened until it reads
        let navy = Rgb::new(20, 30, 90);
        let lighter = readable(navy, DARK);
        assert!(lighter.contrast(DARK) >= TEXT_CONTRAST, "{}", lighter.contrast(DARK));
        assert!(lighter.luminance() > navy.luminance());
        assert!(hue_gap(lighter, navy) < 4.0, "still the same blue: {lighter:?}");
        // a pale yellow on the light background: darkened
        let yellow = Rgb::new(255, 220, 0);
        let darker = readable(yellow, LIGHT);
        assert!(darker.contrast(LIGHT) >= TEXT_CONTRAST && darker.luminance() < yellow.luminance());
        assert!(hue_gap(darker, yellow) < 4.0, "still yellow: {darker:?}");
        // one that reads already is left as it is
        let sky = Rgb::new(111, 160, 255);
        assert_eq!(readable(sky, DARK), sky);
        // a grey has no hue to keep: it only moves
        assert!(readable(Rgb::new(40, 40, 40), DARK).contrast(DARK) >= TEXT_CONTRAST);
    }

    #[test]
    fn black_or_white_is_written_on_a_colour_whichever_stands_out_more() {
        assert_eq!(on(Rgb::new(255, 220, 0)), BLACK);
        assert_eq!(on(Rgb::new(10, 20, 80)), WHITE);
        assert_eq!(on(WHITE), BLACK);
        assert_eq!(on(BLACK), WHITE);
    }

    #[test]
    fn the_colour_of_a_picture_is_its_most_common_vivid_hue() {
        let (blue, gold, red, green, grey) = (Rgb::new(47, 95, 208), Rgb::new(224, 168, 58), Rgb::new(220, 30, 40), Rgb::new(30, 180, 60), Rgb::new(120, 120, 120));
        assert_eq!(dominant(&picture(&[(blue, 100)])), Some(blue), "one colour: that colour");
        // more blue than gold: blue
        let cover = dominant(&picture(&[(blue, 60), (gold, 40)])).expect("a colour");
        assert!(hue_gap(cover, blue) < 8.0, "{cover:?}");
        // mostly grey with a real share of red: the red, not the grey
        let mostly_grey = dominant(&picture(&[(grey, 70), (red, 30)])).expect("a colour");
        assert!(hue_gap(mostly_grey, red) < 8.0, "{mostly_grey:?}");
        // a transparent pixel is not part of the picture
        let mut see_through = picture(&[(red, 100), (green, 10)]);
        for pixel in see_through.chunks_exact_mut(4).take(100) {
            pixel[3] = 0;
        }
        assert_eq!(dominant(&see_through), Some(green));
    }

    #[test]
    fn a_picture_with_no_colour_to_speak_of_has_none() {
        let (grey, red) = (Rgb::new(120, 120, 120), Rgb::new(220, 30, 40));
        assert_eq!(dominant(&picture(&[(grey, 100)])), None, "grey");
        assert_eq!(dominant(&picture(&[(BLACK, 50), (WHITE, 50)])), None, "black and white");
        assert_eq!(dominant(&picture(&[(grey, 98), (red, 2)])), None, "a few coloured specks");
        assert_eq!(dominant(&[]), None, "no pixels");
        assert_eq!(dominant(&[1, 2, 3]), None, "not a whole pixel");
        // a very dark or a very pale colour is not vivid: it reads as black or white
        assert_eq!(dominant(&picture(&[(Rgb::new(12, 2, 2), 100)])), None);
    }
}
