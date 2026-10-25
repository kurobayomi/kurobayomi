//! The page-area overlays (T-043a-064, T-043a-066, T-043a-063): the brightness layer, the colour filter layer with
//! its blend mode, grayscale, invert and the page-change flash. They sit over the pages only, never over the chrome.
//!
//! The values and the mapping are pure functions, tested without a display. [`Tint`] holds the three
//! `DrawingArea` layers and the CSS classes that apply them.
//!
//! The brightness layer is the black overlay of negative values. A positive value drives the display backlight through
//! [`super::backlight`] when a writable backlight exists (T-43a-065); without one the page is left unchanged and the
//! notice is shown once per session (E-BRIGHT-2).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use kuro_prefs::reader::FlashColor;

use super::blend_box::BlendBox;

use super::pref_rules;
use crate::misc::diet_defaults as diet;

/// The page-change flash is cut from the settings; these are its hard-coded interval (pages) and duration (stored units).
const FLASH_INTERVAL_PAGES: i32 = 1;
const FLASH_DURATION_STORED: i32 = 1;
use super::rules;

/// The custom brightness slider range (`ColorFilterPage.kt:41`).
/// The settings diet cut the custom brightness setting; its slider range stays as the overlay's own scale.
pub const BRIGHTNESS_RANGE: (i32, i32) = (-75, 100);

/// One byte of the packed ARGB colour filter value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Alpha,
    Red,
    Green,
    Blue,
}

impl Channel {
    pub const fn shift(self) -> u32 {
        match self {
            Self::Alpha => 24,
            Self::Red => 16,
            Self::Green => 8,
            Self::Blue => 0,
        }
    }

    pub const fn mask(self) -> u32 {
        0xFF << self.shift()
    }
}

/// One channel (`0..=255`) of the packed colour.
pub fn channel_get(packed: i32, channel: Channel) -> i32 {
    ((packed as u32 & channel.mask()) >> channel.shift()) as i32
}

/// The alpha of the black brightness layer: `|v| / 100` for a negative value while the feature is on, else none.
pub fn brightness_alpha(enabled: bool, value: i32) -> f64 {
    if enabled && value < 0 { f64::from(value.clamp(BRIGHTNESS_RANGE.0, -1).abs()) / 100.0 } else { 0.0 }
}

/// The colour of the filter layer as red, green, blue and alpha in `0..=1`, when the filter is on.
pub fn filter_rgba(enabled: bool, packed: i32) -> Option<(f64, f64, f64, f64)> {
    enabled.then(|| {
        let unit = |channel| f64::from(channel_get(packed, channel)) / 255.0;
        (unit(Channel::Red), unit(Channel::Green), unit(Channel::Blue), unit(Channel::Alpha))
    })
}

/// A colour matrix for a `GskColorMatrixNode`: sixteen floats, four rows of `[r, g, b, a]` in the row-vector layout that
/// graphene uses, and the four-float offset that is added after the matrix.
pub type ColourMatrix = ([f32; 16], [f32; 4]);

/// The luminance weights of grayscale (`ColorMatrix.setSaturation(0)`: red, green, blue).
const LUMA: [f32; 3] = [0.213, 0.715, 0.072];

/// The matrix of the page filter (T-043a-066, T-43b-049): grayscale first, then invert, so both together are `1 - luma`.
/// `None` when both are off. Alpha is never touched.
pub fn page_colour_matrix(grayscale: bool, inverted: bool) -> Option<ColourMatrix> {
    if !grayscale && !inverted {
        return None;
    }
    let sign = if inverted { -1.0 } else { 1.0 };
    let mut m = [0.0_f32; 16];
    for (row, weight) in LUMA.iter().enumerate() {
        for col in 0..3 {
            // grayscale: every output channel is the luma; invert only: each channel maps to itself, negated
            let own = if grayscale { *weight } else if row == col { 1.0 } else { 0.0 };
            m[row * 4 + col] = sign * own;
        }
    }
    m[15] = 1.0;
    let offset = if inverted { 1.0 } else { 0.0 };
    Some((m, [offset, offset, offset, 0.0]))
}

type Rgba = (f64, f64, f64, f64);

/// The overlay layers over the page area.
pub struct Tint {
    brightness: gtk::DrawingArea,
    /// Holds the pages and blends the colour filter over them.
    blend: BlendBox,
    flash: gtk::DrawingArea,
    brightness_colour: Rc<Cell<Rgba>>,
    flash_colour: Rc<Cell<Rgba>>,
    /// Counts flashes, so a timer only ends the flash it was started for.
    flash_generation: Rc<Cell<u64>>,
    /// Page changes seen while the flash is on (the interval counter).
    flash_trigger: Cell<u32>,
}

/// A hidden, click-through layer that paints `colour` over its whole area.
fn layer(colour: &Rc<Cell<Rgba>>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_can_target(false);
    area.set_focusable(false);
    area.set_hexpand(true);
    area.set_vexpand(true);
    area.set_visible(false);
    let colour = Rc::clone(colour);
    area.set_draw_func(move |_, cr, width, height| {
        let (r, g, b, a) = colour.get();
        cr.set_source_rgba(r, g, b, a);
        cr.rectangle(0.0, 0.0, f64::from(width), f64::from(height));
        let _ = cr.fill();
    });
    area
}

impl Tint {
    /// Wraps `page` in the colour filter layer, makes that the child of `overlay`, and adds the brightness and flash
    /// layers above it.
    pub fn new(overlay: &gtk::Overlay, page: &impl IsA<gtk::Widget>) -> Self {
        let brightness_colour = Rc::new(Cell::new((0.0, 0.0, 0.0, 0.0)));
        let flash_colour = Rc::new(Cell::new((0.0, 0.0, 0.0, 1.0)));
        let blend = BlendBox::new(page);
        overlay.set_child(Some(&blend));
        let brightness = layer(&brightness_colour);
        let flash = layer(&flash_colour);
        overlay.add_overlay(&brightness);
        overlay.add_overlay(&flash);
        Self {
            brightness,
            blend,
            flash,
            brightness_colour,
            flash_colour,
            flash_generation: Rc::new(Cell::new(0)),
            flash_trigger: Cell::new(0),
        }
    }

    /// Applies the brightness, colour filter, grayscale and invert layers. The settings are cut, so the hard-coded
    /// values (all off) apply; grayscale and invert are colour-matrix nodes over the page area only.
    pub fn apply(&self) {
        let (enabled, value) = (diet::READER_CUSTOM_BRIGHTNESS, 0);
        if let super::backlight::Outcome::Unavailable { notify: true } =
            super::backlight::apply(std::path::Path::new(super::backlight::SYSFS_ROOT), enabled, value)
        {
            crate::toast::notify(super::strings::custom_brightness_unavailable());
        }
        let alpha = brightness_alpha(enabled, value);
        self.brightness_colour.set((0.0, 0.0, 0.0, alpha));
        self.brightness.set_visible(alpha > 0.0);
        self.brightness.queue_draw();

        self.blend.set_filter(filter_rgba(diet::READER_COLOUR_FILTER, 0), 0);

        self.blend.set_page_filter(page_colour_matrix(diet::READER_GRAYSCALE, diet::READER_INVERTED_COLOURS));
    }

    /// A page changed. Counts it toward the flash interval and starts a flash when it is due. Called only while the
    /// flash feature is on (E-FLASH-1), so a later enable does not flash at once.
    pub fn on_page_changed(&self) {
        let (due, next) = rules::flash_due(self.flash_trigger.get(), pref_rules::flash_interval(FLASH_INTERVAL_PAGES));
        self.flash_trigger.set(next);
        if due {
            self.start_flash(FlashColor::BLACK, pref_rules::flash_duration_ms(FLASH_DURATION_STORED));
        }
    }

    /// Clears the interval counter and any flash on screen, for when the feature is turned off.
    pub fn reset_flash(&self) {
        self.flash_trigger.set(0);
        self.flash_generation.set(self.flash_generation.get() + 1);
        self.flash.set_visible(false);
    }

    fn start_flash(&self, colour: FlashColor, duration_ms: i32) {
        let generation = self.flash_generation.get() + 1;
        self.flash_generation.set(generation);
        let duration = Duration::from_millis(duration_ms.max(1) as u64);
        let phases = rules::flash_phases(colour, duration);
        let paint = |cell: &Rc<Cell<Rgba>>, rgb: (f64, f64, f64)| cell.set((rgb.0, rgb.1, rgb.2, 1.0));
        paint(&self.flash_colour, phases[0].rgb);
        self.flash.set_visible(true);
        self.flash.queue_draw();
        let current = Rc::clone(&self.flash_generation);
        if let Some(second) = phases.get(1).copied() {
            let (area, cell, current) = (self.flash.clone(), Rc::clone(&self.flash_colour), Rc::clone(&current));
            glib::timeout_add_local_once(second.start, move || {
                if current.get() == generation {
                    cell.set((second.rgb.0, second.rgb.1, second.rgb.2, 1.0));
                    area.queue_draw();
                }
            });
        }
        let area = self.flash.clone();
        glib::timeout_add_local_once(duration, move || {
            if current.get() == generation {
                area.set_visible(false);
            }
        });
    }
}
