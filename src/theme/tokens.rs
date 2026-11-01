//! The spacing scale, the alpha and duration constants of the theme, and the text styles the guard tests read.
//!
//! Dp values are CSS px at scale 1; GTK does the scaling, so no constant is multiplied here.

// --- Spacing, alpha, duration and radius (T-49a-051) ---

/// The one spacing scale of the app, in px (docs/design/main-window.md). Every margin, padding and box spacing a screen sets
/// by hand is one of these; the stylesheet repeats the same steps. A built-in style class (`boxed-list`, `toolbar`, `linked`,
/// `card`, `rich-list`, `navigation-sidebar`, `adw::Clamp`, `adw::PreferencesGroup`) is always preferred over a margin.
/// `SPACE_XS` is the gap inside a control or between two badges; `SPACE_S` between a control and its label; `SPACE_M`
/// between a heading and its list and for a dialog's inner padding; `SPACE_L` the gutter between the window edge and a page's
/// content; `SPACE_XL` between two groups and around a centred, standalone message.
pub const SPACE_XS: i32 = 4;
pub const SPACE_S: i32 = 8;
pub const SPACE_M: i32 = 12;
pub const SPACE_L: i32 = 16;
pub const SPACE_XL: i32 = 24;

pub const RIPPLE_ALPHA: f32 = 0.10;
pub const SCROLLBAR_FADE_MS: u32 = 250;

#[allow(dead_code)] // kept for the shape of the type; not built or read yet
#[cfg(test)]
/// One text style: the CSS class it is emitted under, `font-size` in em, an optional weight
/// (omitted means normal) and an optional `line-height` in px.
pub struct TypeStyle {
    pub class: &'static str,
    pub size_em: f32,
    pub weight: Option<u16>,
    pub line_height_px: Option<f32>,
}

#[cfg(test)]
/// The Material 3 styles the app uses (`p1-rust-design.md` section 5).
pub const TYPOGRAPHY: [TypeStyle; 6] = [
    TypeStyle { class: "kuro-body-medium", size_em: 0.875, weight: None, line_height_px: Some(20.0) },
    TypeStyle { class: "kuro-label-large", size_em: 0.875, weight: Some(500), line_height_px: None },
    TypeStyle { class: "kuro-title-small", size_em: 0.875, weight: Some(500), line_height_px: None },
    TypeStyle { class: "kuro-title-medium", size_em: 1.0, weight: Some(500), line_height_px: Some(24.0) },
    TypeStyle { class: "kuro-headline-small", size_em: 1.5, weight: None, line_height_px: Some(32.0) },
    TypeStyle { class: "kuro-body-small", size_em: 0.75, weight: None, line_height_px: Some(16.0) },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spacing_scale_climbs_in_the_steps_the_stylesheet_repeats() {
        assert_eq!([SPACE_XS, SPACE_S, SPACE_M, SPACE_L, SPACE_XL], [4, 8, 12, 16, 24]);
    }

    #[test]
    fn typography_classes_are_unique_and_kuro_prefixed() {
        let mut names: Vec<&str> = TYPOGRAPHY.iter().map(|t| t.class).collect();
        assert!(names.iter().all(|n| n.starts_with("kuro-")));
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), TYPOGRAPHY.len());
    }
}
