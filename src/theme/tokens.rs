//! The alpha, duration and window-size constants of the theme, and the text styles the guard tests read.
//!
//! Dp values are CSS px at scale 1; GTK does the scaling, so no constant is multiplied here.

// --- Spacing, alpha, duration and radius (T-49a-051) ---

pub const RIPPLE_ALPHA: f32 = 0.10;
pub const SCROLLBAR_FADE_MS: u32 = 250;

/// Window-size class breakpoints, dp (`WindowSize.kt`): medium from 600, expanded from 840.
pub const MEDIUM_WIDTH_WINDOW_SIZE: f32 = 600.0;
pub const EXPANDED_WIDTH_WINDOW_SIZE: f32 = 840.0;

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
    fn typography_classes_are_unique_and_kuro_prefixed() {
        let mut names: Vec<&str> = TYPOGRAPHY.iter().map(|t| t.class).collect();
        assert!(names.iter().all(|n| n.starts_with("kuro-")));
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), TYPOGRAPHY.len());
    }
}
