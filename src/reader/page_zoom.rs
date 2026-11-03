//! Page zoom model (T-43b-015, T-43b-016, T-43b-017). Pure: no GTK types. Mirrors `SubsamplingScaleImageView.minScale`
//! (fork) and `ReaderPageImageView.setupZoom`.
//!
//! Coordinates: a scale is screen pixels per source pixel; a centre is in source pixels.

use kuro_prefs::reader::ReadingMode;

/// The largest zoom, as a multiple of the fitted scale (`MAX_ZOOM_SCALE` times `minScale`, E-02).
pub const MAX_ZOOM_FACTOR: f64 = 5.0;
/// The double-tap level of a fitted non-animated page (`setDoubleTapZoomScale(scale * 2)`).
pub const DOUBLE_TAP_FACTOR: f64 = 2.0;
/// One notch of ctrl+wheel zoom (desktop equivalent of the pinch, T-43b-016).
pub const WHEEL_ZOOM_STEP: f64 = 1.1;
/// Absolute zoom levels of an animated page: fit, double tap, maximum (E-41).
pub const ANIMATED_MEDIUM: f64 = 2.0;

const EPS: f64 = 1e-9;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

/// The image scale type of a page (`ImageScaleType`, stored as 1..=6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageScaleType {
    #[default]
    FitScreen,
    /// Crop to fill (`SCALE_TYPE_CENTER_CROP`, D-1): the larger of the two fits.
    Stretch,
    FitWidth,
    FitHeight,
    OriginalSize,
    SmartFit,
}

impl ImageScaleType {
    /// The type for a stored value. 1..=6 select the six types. Any other value (0, 7 CUSTOM, 99) is `FitScreen`
    /// (D-2, E-18).
    pub fn from_stored(value: i32) -> Self {
        match value {
            2 => Self::Stretch,
            3 => Self::FitWidth,
            4 => Self::FitHeight,
            5 => Self::OriginalSize,
            6 => Self::SmartFit,
            _ => Self::FitScreen,
        }
    }

    /// The type a page uses: webtoon pages always fit the width (`WebtoonPageHolder.kt:201`), others follow the
    /// stored value.
    pub fn effective(stored: i32, webtoon: bool) -> Self {
        if webtoon { Self::FitWidth } else { Self::from_stored(stored) }
    }
}

/// Where the image is centred when it first appears (`ZoomStartPosition`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ZoomStart {
    Left,
    #[default]
    Center,
    Right,
}

/// The zoom start for a stored `zoom_start` value and the reading mode. Automatic (1) is Left for left-to-right,
/// Right for right-to-left and Center otherwise. 2 is Left, 3 is Right, and any other value is Center.
pub fn zoom_start(stored: i32, mode: ReadingMode) -> ZoomStart {
    match stored {
        1 => match mode {
            ReadingMode::LeftToRight => ZoomStart::Left,
            ReadingMode::RightToLeft => ZoomStart::Right,
            _ => ZoomStart::Center,
        },
        2 => ZoomStart::Left,
        3 => ZoomStart::Right,
        _ => ZoomStart::Center,
    }
}

/// The settings a page is shown with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageConfig {
    pub scale: ImageScaleType,
    pub zoom_start: ZoomStart,
}

/// The fitted (minimum) scale for a view and source size, per `minScale()` (fork section 3 table). Before the view has
/// a size, and for an empty source, it is 1 (E-60), so no later step divides by zero.
pub fn min_scale(kind: ImageScaleType, view: Size, src: Size) -> f64 {
    if src.w <= 0.0 || src.h <= 0.0 || view.w <= 0.0 || view.h <= 0.0 {
        return 1.0;
    }
    let fit_w = view.w / src.w;
    let fit_h = view.h / src.h;
    match kind {
        ImageScaleType::FitScreen => fit_w.min(fit_h),
        ImageScaleType::Stretch => fit_w.max(fit_h),
        ImageScaleType::FitWidth => fit_w,
        ImageScaleType::FitHeight => fit_h,
        ImageScaleType::OriginalSize => 1.0,
        ImageScaleType::SmartFit => {
            if src.h > src.w {
                fit_w
            } else {
                fit_h
            }
        }
    }
}

/// Keeps a user zoom within `[min, 5 x min]` (E-02). Written with `max` and `min` so no input can panic.
pub fn clamp_scale(scale: f64, min: f64) -> f64 {
    scale.max(min).min(min * MAX_ZOOM_FACTOR)
}

/// The double-tap level of a page fitted at `min`: twice the fit, never above the maximum zoom (E-65).
pub fn double_tap_level(min: f64) -> f64 {
    (min * DOUBLE_TAP_FACTOR).min(min * MAX_ZOOM_FACTOR)
}

/// The double-tap target of a non-animated page (E-14, E-64): zoom in to the double-tap level when the scale is at the
/// fit or at most 0.9 of that level; otherwise go back to the fitted scale.
pub fn double_tap_target(scale: f64, min: f64) -> f64 {
    let level = double_tap_level(min);
    if scale <= min + EPS || scale <= 0.9 * level { level } else { min }
}

/// The zoom of one page (T-44a-044, TZ §1-§3): the scale, the centre in source pixels, the fitted minimum, the maximum
/// (5 times the fit) and the double-tap level. A `Copy` value, so the page view holds it in a `Cell`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ZoomState {
    pub scale: f64,
    pub center: (f64, f64),
    pub min: f64,
    pub max: f64,
    pub double_tap: f64,
}

impl ZoomState {
    /// A page shown at its fit, centred on `center`.
    pub fn fitted(min: f64, center: (f64, f64)) -> Self {
        Self { scale: min, center, min, max: min * MAX_ZOOM_FACTOR, double_tap: double_tap_level(min) }
    }

    /// Keeps a scale within `[min, max]`.
    pub fn clamp(&self, scale: f64) -> f64 {
        scale.max(self.min).min(self.max)
    }

    /// Whether the page is at its fitted scale.
    pub fn is_fitted(&self) -> bool {
        (self.scale - self.min).abs() < EPS
    }
}

/// The double-tap target of an animated page (E-41): absolute levels, 1 toggles to 2, anything above 1 goes to 1.
pub fn animated_double_tap_target(scale: f64) -> f64 {
    if scale > 1.0 { 1.0 } else { ANIMATED_MEDIUM }
}

/// The duration of a zoom animation for the GTK animation setting (T-43b-026): unchanged when animations are on, and
/// 1 ms (never 0) when they are off. Landscape zoom and pan are not scaled and do not call this.
pub fn animation_ms(ms: u32, animations_enabled: bool) -> u32 {
    if animations_enabled { ms } else { 1 }
}

/// Ease in and out over `t` in 0..=1 (the default easing of the zoom animation).
pub fn ease_in_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 2.0 * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(2) / 2.0 }
}

/// The scale at `elapsed_ms` of a zoom from `from` to `to` lasting `duration_ms`. A zero duration is at once `to`.
pub fn zoom_step(from: f64, to: f64, elapsed_ms: f64, duration_ms: u32) -> f64 {
    if duration_ms == 0 {
        return to;
    }
    from + (to - from) * ease_in_out(elapsed_ms / f64::from(duration_ms))
}

/// The centre a zoom start puts the image at (E-03). Left and Right sit on the top edge, Center on the middle.
pub fn anchor_center(start: ZoomStart, src: Size) -> (f64, f64) {
    match start {
        ZoomStart::Left => (0.0, 0.0),
        ZoomStart::Right => (src.w, 0.0),
        ZoomStart::Center => (src.w / 2.0, src.h / 2.0),
    }
}

/// Landscape auto-zoom starts this long after the page is selected (`postDelayed(500)`).
pub const LANDSCAPE_ZOOM_DELAY_MS: u32 = 500;
/// Length of the landscape auto-zoom and of the landscape zoom's easing (`withDuration(500)`, EASE_IN_OUT_QUAD).
pub const LANDSCAPE_ZOOM_MS: u32 = 500;
/// Length of one pan step (`withDuration(250)`, EASE_OUT_QUAD). Not scaled by the system animation factor.
pub const PAN_MS: u32 = 250;

/// Whether the landscape auto-zoom runs (`ReaderPageImageView.landscapeZoom`, T-43b-018): the preference is on, the page
/// is ready and fitted to the screen, it is wider than tall, and it sits at its minimum zoom (E-04).
pub fn landscape_zoom_applies(enabled: bool, kind: ImageScaleType, src: Size, scale: f64, min: f64, ready: bool) -> bool {
    enabled && ready && kind == ImageScaleType::FitScreen && src.w > src.h && (scale - min).abs() < EPS
}

/// The zoom the landscape auto-zoom animates to: the page height fits the view height (`height / sHeight`).
pub fn landscape_zoom_scale(view: Size, src: Size) -> f64 {
    view.h / src.h
}

/// Whether the image can pan in one direction: more than 1 px is left to pan (`canPan`, E-13). `remaining` is in screen px.
pub fn can_pan(remaining: f64) -> bool {
    remaining > 1.0
}

/// The centre after one pan step: one view width in source px (`view.width / scale`), left for `dir` -1 and right for +1.
/// A scale of zero or less leaves the centre where it is.
pub fn pan_step_centre(center: (f64, f64), view: Size, scale: f64, dir: f64) -> (f64, f64) {
    if scale <= 0.0 {
        return center;
    }
    (center.0 + dir.signum() * view.w / scale, center.1)
}

/// Ease out over `t` in 0..=1 (`EASE_OUT_QUAD`, the easing of a pan step).
pub fn ease_out_quad(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * (2.0 - t)
}

/// How far the image can still pan, in screen px: `(left, right)`. Zero on an axis where the view is as wide as the
/// image or wider, and never negative.
pub fn pan_remaining(center: (f64, f64), view: Size, src: Size, scale: f64) -> (f64, f64) {
    if scale <= 0.0 {
        return (0.0, 0.0);
    }
    let half = view.w / scale / 2.0;
    (((center.0 - half) * scale).max(0.0), ((src.w - (center.0 + half)) * scale).max(0.0))
}

/// The centre a landscape auto-zoom animates to (`landscapeZoom`): the zoom start decides the corner, and a backward
/// selection reads the page from its other end. Centred pages keep their centre.
pub fn landscape_zoom_centre(start: ZoomStart, forward: bool, src: Size, current: (f64, f64)) -> (f64, f64) {
    let (near, far) = ((0.0, 0.0), (src.w, 0.0));
    match (start, forward) {
        (ZoomStart::Left, true) | (ZoomStart::Right, false) => near,
        (ZoomStart::Right, true) | (ZoomStart::Left, false) => far,
        (ZoomStart::Center, _) => current,
    }
}

/// A zoom key of the viewer (T-43b-058, additive): plus zooms in, minus zooms out, 0 and Ctrl+0 go back to the fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomKey {
    In,
    Out,
    Reset,
}

/// One key press of zoom, as a factor of the scale.
pub const KEY_ZOOM_STEP: f64 = 1.25;

/// The scale after a zoom key: [`KEY_ZOOM_STEP`] up or down within `[min, 5 x min]`, or the fit for `Reset`.
pub fn key_zoom_target(scale: f64, min: f64, key: ZoomKey) -> f64 {
    match key {
        ZoomKey::In => clamp_scale(scale * KEY_ZOOM_STEP, min),
        ZoomKey::Out => clamp_scale(scale / KEY_ZOOM_STEP, min),
        ZoomKey::Reset => min,
    }
}

/// Where a page narrower than its view sits across it. The two pages of a spread hug the gutter between them (the left
/// one its right edge, the right one its left edge), so the pair reads as one picture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hug {
    #[default]
    Center,
    Start,
    End,
}

/// Keeps the view inside the image (`PAN_LIMIT_INSIDE`, E-12). An axis where the image is smaller than the view is
/// centred on the image.
pub fn clamp_center(center: (f64, f64), view: Size, src: Size, scale: f64) -> (f64, f64) {
    clamp_center_hugging(center, view, src, scale, Hug::Center)
}

/// [`clamp_center`], with a page narrower than the view placed across it by `hug`. Up and down it is always centred.
pub fn clamp_center_hugging(center: (f64, f64), view: Size, src: Size, scale: f64, hug: Hug) -> (f64, f64) {
    if scale <= 0.0 {
        return center;
    }
    (clamp_axis(center.0, view.w, src.w, scale, hug), clamp_axis(center.1, view.h, src.h, scale, Hug::Center))
}

fn clamp_axis(c: f64, view_px: f64, src_px: f64, scale: f64, hug: Hug) -> f64 {
    let visible = view_px / scale;
    if visible < src_px {
        return c.max(visible / 2.0).min(src_px - visible / 2.0);
    }
    match hug {
        Hug::Center => src_px / 2.0,
        // the view's left edge on the image's left edge
        Hug::Start => visible / 2.0,
        // the view's right edge on the image's right edge
        Hug::End => src_px - visible / 2.0,
    }
}

/// The centre after zooming from `old` to `new` while the source point under `focus` (view pixels) stays under it.
pub fn zoom_about(center: (f64, f64), view: Size, old: f64, new: f64, focus: (f64, f64)) -> (f64, f64) {
    if old <= 0.0 || new <= 0.0 {
        return center;
    }
    let dx = focus.0 - view.w / 2.0;
    let dy = focus.1 - view.h / 2.0;
    let under = (center.0 + dx / old, center.1 + dy / old);
    (under.0 - dx / new, under.1 - dy / new)
}

/// The scale during a pinch: the scale the pinch began at times the gesture's factor, kept within the page's range
/// (`scale = min(max, dist / distStart * scaleStart)`, never under the fit; TZ §4.1).
pub fn pinch_scale(start: f64, factor: f64, zoom: &ZoomState) -> f64 {
    zoom.clamp(start * factor)
}

/// The scale after one ctrl+wheel notch: up zooms in, down zooms out, both bounded by [`clamp_scale`].
pub fn wheel_zoom(scale: f64, min: f64, up: bool) -> f64 {
    let factor = if up { WHEEL_ZOOM_STEP } else { 1.0 / WHEEL_ZOOM_STEP };
    clamp_scale(scale * factor, min)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: Size = Size { w: 800.0, h: 600.0 };

    #[test]
    fn e61_e62_stretch_and_smart_fit_on_the_spec_vectors() {
        let view = Size { w: 200.0, h: 200.0 };
        let wide = Size { w: 400.0, h: 200.0 };
        assert!(close(min_scale(ImageScaleType::Stretch, view, wide), 1.0), "crop to fill takes the larger fit");
        assert!(close(min_scale(ImageScaleType::FitScreen, view, wide), 0.5));
        let square = Size { w: 600.0, h: 600.0 };
        assert!(close(min_scale(ImageScaleType::SmartFit, square, Size { w: 300.0, h: 900.0 }), 2.0), "tall fits the width");
        assert!(close(min_scale(ImageScaleType::SmartFit, square, Size { w: 900.0, h: 300.0 }), 2.0), "wide fits the height");
    }

    #[test]
    fn e60_before_the_view_has_a_size_the_minimum_is_one() {
        let zero = Size { w: 0.0, h: 0.0 };
        assert_eq!(min_scale(ImageScaleType::FitScreen, zero, TALL), 1.0);
        assert_eq!(min_scale(ImageScaleType::Stretch, Size { w: 800.0, h: 0.0 }, TALL), 1.0);
        assert_eq!(min_scale(ImageScaleType::FitScreen, VIEW, Size { w: 0.0, h: 0.0 }), 1.0);
    }

    #[test]
    fn e65_the_double_tap_level_is_twice_the_fit_within_the_maximum() {
        assert!(close(double_tap_level(1.0), 2.0));
        assert!(close(double_tap_level(0.3), 0.6));
        assert!(double_tap_level(0.3) <= 0.3 * MAX_ZOOM_FACTOR);
    }

    #[test]
    fn e64_double_tap_zooms_in_up_to_the_boundary_of_nine_tenths_of_the_level() {
        let min = 0.3;
        let level = 0.6;
        assert!(close(double_tap_target(min, min), level), "at the fit");
        assert!(close(double_tap_target(0.9 * level, min), level), "exactly at 0.9 of the level zooms in");
        assert!(close(double_tap_target(0.9 * level + 0.001, min), min), "just above goes back to the fit");
        assert!(close(double_tap_target(0.5, min), level), "between the fit and the boundary zooms in");
    }

    #[test]
    fn t044_a_fitted_zoom_state_has_the_maximum_and_the_double_tap_level() {
        let state = ZoomState::fitted(0.3, (5.0, 6.0));
        assert!(state.is_fitted());
        assert!(close(state.max, 1.5) && close(state.double_tap, 0.6));
        assert!(close(state.clamp(9.0), 1.5) && close(state.clamp(0.1), 0.3));
        assert_eq!(state.center, (5.0, 6.0));
    }
    const TALL: Size = Size { w: 1000.0, h: 2000.0 };
    const WIDE: Size = Size { w: 2000.0, h: 1000.0 };

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn zoom_durations_follow_the_animation_setting() {
        assert_eq!(animation_ms(500, true), 500);
        assert_eq!(animation_ms(500, false), 1);
        assert_eq!(animation_ms(0, false), 1);
    }

    #[test]
    fn the_zoom_step_runs_from_start_to_target() {
        assert!(close(zoom_step(1.0, 3.0, 0.0, 500), 1.0));
        assert!(close(zoom_step(1.0, 3.0, 250.0, 500), 2.0));
        assert!(close(zoom_step(1.0, 3.0, 500.0, 500), 3.0));
        assert!(close(zoom_step(1.0, 3.0, 900.0, 500), 3.0));
        assert!(close(zoom_step(1.0, 3.0, 10.0, 0), 3.0));
        assert!(zoom_step(1.0, 3.0, 100.0, 500) < 1.0 + 2.0 * 0.2, "eases in");
    }

    #[test]
    fn e01_min_scale_per_type_for_a_tall_page() {
        assert!(close(min_scale(ImageScaleType::FitScreen, VIEW, TALL), 0.3));
        assert!(close(min_scale(ImageScaleType::Stretch, VIEW, TALL), 0.8));
        assert!(close(min_scale(ImageScaleType::FitWidth, VIEW, TALL), 0.8));
        assert!(close(min_scale(ImageScaleType::FitHeight, VIEW, TALL), 0.3));
        assert!(close(min_scale(ImageScaleType::OriginalSize, VIEW, TALL), 1.0));
        // SmartFit on a tall page fits the width.
        assert!(close(min_scale(ImageScaleType::SmartFit, VIEW, TALL), 0.8));
    }

    #[test]
    fn e01b_smart_fit_on_a_wide_page_fits_the_height() {
        assert!(close(min_scale(ImageScaleType::SmartFit, VIEW, WIDE), 0.6));
        assert!(close(min_scale(ImageScaleType::FitScreen, VIEW, WIDE), 0.4));
    }

    #[test]
    fn e17_stretch_is_crop_to_fill_not_a_non_uniform_stretch() {
        assert_eq!(ImageScaleType::from_stored(2), ImageScaleType::Stretch);
        assert!(close(min_scale(ImageScaleType::Stretch, VIEW, TALL), 0.8));
    }

    #[test]
    fn e18_invalid_stored_scale_types_fall_back_to_fit_screen() {
        for v in [0, 7, 99, -3] {
            assert_eq!(ImageScaleType::from_stored(v), ImageScaleType::FitScreen, "{v}");
        }
        assert_eq!(ImageScaleType::from_stored(6), ImageScaleType::SmartFit);
        assert_eq!(ImageScaleType::from_stored(1), ImageScaleType::FitScreen);
    }

    #[test]
    fn webtoon_always_fits_the_width_whatever_is_stored() {
        assert_eq!(ImageScaleType::effective(1, true), ImageScaleType::FitWidth);
        assert_eq!(ImageScaleType::effective(5, true), ImageScaleType::FitWidth);
        assert_eq!(ImageScaleType::effective(5, false), ImageScaleType::OriginalSize);
    }

    #[test]
    fn e02_zoom_is_clamped_to_min_and_five_times_min() {
        let min = 0.3;
        let got: Vec<f64> = [0.1, 0.3, 1.0, 1.6, 9.0].iter().map(|s| clamp_scale(*s, min)).collect();
        let want = [0.3, 0.3, 1.0, 1.5, 1.5];
        for (g, w) in got.iter().zip(want) {
            assert!(close(*g, w), "{g} != {w}");
        }
    }

    #[test]
    fn e03_zoom_start_anchors_for_a_tall_page() {
        assert_eq!(anchor_center(ZoomStart::Left, TALL), (0.0, 0.0));
        assert_eq!(anchor_center(ZoomStart::Right, TALL), (1000.0, 0.0));
        assert_eq!(anchor_center(ZoomStart::Center, TALL), (500.0, 1000.0));
    }

    #[test]
    fn t017_zoom_start_from_the_stored_value_and_the_mode() {
        use ReadingMode::*;
        assert_eq!(zoom_start(1, LeftToRight), ZoomStart::Left);
        assert_eq!(zoom_start(1, RightToLeft), ZoomStart::Right);
        assert_eq!(zoom_start(1, Vertical), ZoomStart::Center);
        assert_eq!(zoom_start(1, Webtoon), ZoomStart::Center);
        assert_eq!(zoom_start(2, RightToLeft), ZoomStart::Left);
        assert_eq!(zoom_start(3, LeftToRight), ZoomStart::Right);
        assert_eq!(zoom_start(4, LeftToRight), ZoomStart::Center);
        assert_eq!(zoom_start(0, LeftToRight), ZoomStart::Center);
    }

    #[test]
    fn e12_pan_centre_is_forced_on_an_axis_the_image_does_not_fill() {
        // At 0.3 the 1000x2000 page is smaller than the 800x600 view on both axes: centred.
        assert_eq!(clamp_center((10.0, 10.0), VIEW, TALL, 0.3), (500.0, 1000.0));
        // At 2.0 the visible window is 400x300 source px and the centre stays inside the edges.
        assert_eq!(clamp_center((0.0, 0.0), VIEW, TALL, 2.0), (200.0, 150.0));
        assert_eq!(clamp_center((5000.0, 5000.0), VIEW, TALL, 2.0), (800.0, 1850.0));
    }

    #[test]
    fn a_spread_page_narrower_than_its_half_hugs_the_gutter_and_a_zoomed_one_pans_as_usual() {
        // At 0.3 the 1000 px wide page shows 300 px wide in an 800 px view: 2667 source px are visible across.
        let visible = 800.0 / 0.3;
        let (start, end) = (clamp_center_hugging((10.0, 10.0), VIEW, TALL, 0.3, Hug::Start), clamp_center_hugging((10.0, 10.0), VIEW, TALL, 0.3, Hug::End));
        assert!((start.0 - visible / 2.0).abs() < 1e-9, "the view starts at the page's left edge");
        assert!((end.0 - (1000.0 - visible / 2.0)).abs() < 1e-9, "the view ends at the page's right edge");
        assert_eq!((start.1, end.1), (1000.0, 1000.0), "always centred up and down");
        // zoomed in past the view, the hug changes nothing
        assert_eq!(clamp_center_hugging((0.0, 0.0), VIEW, TALL, 2.0, Hug::End), (200.0, 150.0));
    }

    #[test]
    fn e14_double_tap_toggles_between_fit_and_twice_fit() {
        let min = 0.3;
        let first = double_tap_target(min, min);
        assert!(close(first, 0.6));
        let back = double_tap_target(first, min);
        assert!(close(back, min));
    }

    #[test]
    fn zoom_about_keeps_the_point_under_the_focus_fixed() {
        let center = (500.0, 1000.0);
        let focus = (100.0, 50.0);
        let before = (center.0 + (focus.0 - 400.0) / 0.3, center.1 + (focus.1 - 300.0) / 0.3);
        let new_center = zoom_about(center, VIEW, 0.3, 0.9, focus);
        let after = (new_center.0 + (focus.0 - 400.0) / 0.9, new_center.1 + (focus.1 - 300.0) / 0.9);
        assert!(close(before.0, after.0) && close(before.1, after.1));
    }

    #[test]
    fn wheel_notches_zoom_by_one_point_one_and_stay_in_range() {
        assert!(close(wheel_zoom(1.0, 0.3, true), 1.1));
        assert!(close(wheel_zoom(1.0, 0.3, false), 1.0 / 1.1));
        assert!(close(wheel_zoom(0.3, 0.3, false), 0.3));
        assert!(close(wheel_zoom(1.5, 0.3, true), 1.5));
    }

    #[test]
    fn landscape_zoom_needs_every_precondition() {
        let wide = Size { w: 2000.0, h: 1000.0 };
        let tall = Size { w: 1000.0, h: 2000.0 };
        assert!(landscape_zoom_applies(true, ImageScaleType::FitScreen, wide, 0.5, 0.5, true));
        assert!(!landscape_zoom_applies(false, ImageScaleType::FitScreen, wide, 0.5, 0.5, true), "the preference is off");
        assert!(!landscape_zoom_applies(true, ImageScaleType::FitWidth, wide, 0.5, 0.5, true), "not fitted to the screen");
        assert!(!landscape_zoom_applies(true, ImageScaleType::FitScreen, tall, 0.5, 0.5, true), "not a wide page");
        assert!(!landscape_zoom_applies(true, ImageScaleType::FitScreen, wide, 0.6, 0.5, true), "already zoomed past the minimum");
        assert!(!landscape_zoom_applies(true, ImageScaleType::FitScreen, wide, 0.5, 0.5, false), "the page is not ready");
    }

    #[test]
    fn landscape_zoom_fits_the_height_in_five_hundred_ms() {
        assert_eq!((LANDSCAPE_ZOOM_DELAY_MS, LANDSCAPE_ZOOM_MS), (500, 500));
        assert!(close(landscape_zoom_scale(Size { w: 800.0, h: 600.0 }, Size { w: 2000.0, h: 1200.0 }), 0.5));
    }

    #[test]
    fn a_pan_is_possible_only_past_one_pixel() {
        assert!(can_pan(40.0));
        assert!(!can_pan(1.0), "exactly 1 px is not enough (E-13)");
        assert!(!can_pan(0.0));
    }

    #[test]
    fn the_pan_remaining_is_what_is_left_to_each_side_in_screen_pixels() {
        let (view, src) = (Size { w: 800.0, h: 600.0 }, Size { w: 2000.0, h: 600.0 });
        // at scale 1 the view covers 800 source px; centred at 1000 it has 600 left on each side
        assert_eq!(pan_remaining((1000.0, 300.0), view, src, 1.0), (600.0, 600.0));
        assert_eq!(pan_remaining((400.0, 300.0), view, src, 1.0), (0.0, 1200.0), "at the left edge");
        assert_eq!(pan_remaining((1600.0, 300.0), view, src, 1.0), (1200.0, 0.0), "at the right edge");
        // a page narrower than the view has nothing to pan
        assert_eq!(pan_remaining((500.0, 300.0), view, Size { w: 1000.0, h: 600.0 }, 0.5), (0.0, 0.0));
        assert_eq!(pan_remaining((1.0, 1.0), view, src, 0.0), (0.0, 0.0));
        assert!(can_pan(pan_remaining((1000.0, 300.0), view, src, 1.0).0));
        assert!(!can_pan(pan_remaining((400.5, 300.0), view, src, 1.0).0), "half a pixel left is not a pan");
    }

    #[test]
    fn the_pan_easing_starts_fast_and_ends_at_one() {
        assert!(close(ease_out_quad(0.0), 0.0) && close(ease_out_quad(1.0), 1.0) && close(ease_out_quad(2.0), 1.0));
        assert!(close(ease_out_quad(0.5), 0.75), "faster than linear at the start");
    }

    #[test]
    fn the_landscape_zoom_goes_to_the_corner_the_zoom_start_names() {
        let src = Size { w: 2000.0, h: 1000.0 };
        let here = (700.0, 500.0);
        assert_eq!(landscape_zoom_centre(ZoomStart::Left, true, src, here), (0.0, 0.0));
        assert_eq!(landscape_zoom_centre(ZoomStart::Left, false, src, here), (2000.0, 0.0));
        assert_eq!(landscape_zoom_centre(ZoomStart::Right, true, src, here), (2000.0, 0.0));
        assert_eq!(landscape_zoom_centre(ZoomStart::Right, false, src, here), (0.0, 0.0));
        assert_eq!(landscape_zoom_centre(ZoomStart::Center, true, src, here), here);
    }

    #[test]
    fn the_zoom_keys_step_within_the_range_and_reset_goes_to_the_fit() {
        let min = 0.4;
        assert!(close(key_zoom_target(min, min, ZoomKey::In), min * KEY_ZOOM_STEP));
        assert!(close(key_zoom_target(min, min, ZoomKey::Out), min), "never under the fit");
        assert!(close(key_zoom_target(min * 5.0, min, ZoomKey::In), min * 5.0), "never over five times the fit");
        assert!(close(key_zoom_target(1.0, min, ZoomKey::Out), 1.0 / KEY_ZOOM_STEP));
        assert!(close(key_zoom_target(1.3, min, ZoomKey::Reset), min));
    }

    #[test]
    fn a_pan_step_moves_one_view_width_in_source_pixels() {
        let view = Size { w: 800.0, h: 600.0 };
        assert!(close(pan_step_centre((1000.0, 500.0), view, 0.5, 1.0).0, 2600.0), "right by 800 px at half zoom");
        assert!(close(pan_step_centre((1000.0, 500.0), view, 0.5, -1.0).0, -600.0), "left by 800 px at half zoom");
        assert_eq!(pan_step_centre((1000.0, 500.0), view, 0.5, 1.0).1, 500.0, "the pan is horizontal only");
        assert_eq!(pan_step_centre((1000.0, 500.0), view, 0.0, 1.0), (1000.0, 500.0), "no scale, no move");
    }
}
