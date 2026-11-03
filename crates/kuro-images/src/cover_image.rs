//! The GTK-free half of `CoverImage` (T-34a-052).
//!
//! `kuro-images` has no GTK, so the widget itself (a `gtk::Stack` of placeholder, `gtk::Picture` with
//! `content_fit = Cover` and the error image, inside a `gtk::AspectFrame`) lives in the GTK crate. What it needs
//! to decide is here: the three ratios, the placeholder colour, the state a load moves the cover through, and
//! the rule that drops a result that arrives after the widget was re-bound.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::decode::Decoded;
use crate::error::ImageError;
use crate::fetch::LoadedImage;

/// `ItemCover`. `ratio` is width over height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverRatio {
    Square,
    Book,
    Thumb,
}

impl CoverRatio {
    pub fn ratio(self) -> f64 {
        match self {
            CoverRatio::Square => 1.0,
            CoverRatio::Book => 2.0 / 3.0,
            CoverRatio::Thumb => 16.0 / 9.0,
        }
    }

    /// The height that goes with `width`, rounded to the nearest pixel.
    pub fn height_for_width(self, width: i32) -> i32 {
        (f64::from(width) / self.ratio()).round() as i32
    }
}

/// `CoverPlaceholderColor = Color(0x1F888888)`, as ARGB.
pub const PLACEHOLDER_ARGB: u32 = 0x1F88_8888;
/// The CSS class of the placeholder widget; the app stylesheet gives it `placeholder_css()` as its background.
pub const PLACEHOLDER_CLASS: &str = "kuro-cover-placeholder";
/// The bundled image shown on error, `R.drawable.cover_error`.
pub const ERROR_IMAGE: &str = "cover_error";

/// The placeholder colour as a CSS `rgba()` value: `rgba(136, 136, 136, 0.1216)`.
pub fn placeholder_css() -> String {
    let [alpha, red, green, blue] = PLACEHOLDER_ARGB.to_be_bytes();
    format!(
        "rgba({red}, {green}, {blue}, {:.4})",
        f64::from(alpha) / 255.0
    )
}

/// What the cover shows.
#[derive(Clone, Debug)]
pub enum CoverState {
    /// Loading, or not bound to anything.
    Placeholder,
    Loaded(Arc<Decoded>),
    /// The load failed; the widget shows `ERROR_IMAGE`.
    Error,
}

/// The state of one cover widget across `bind`, `finish` and `unbind`.
pub struct CoverBinding {
    generation: u64,
    cancel: Option<CancellationToken>,
    /// A load was started and has not finished.
    loading: bool,
    state: CoverState,
}

impl Default for CoverBinding {
    fn default() -> Self {
        Self {
            generation: 0,
            cancel: None,
            loading: false,
            state: CoverState::Placeholder,
        }
    }
}

impl CoverBinding {
    pub fn state(&self) -> &CoverState {
        &self.state
    }

    /// Starts a load: cancels the one in flight, shows the placeholder, and returns the generation to give back
    /// to `finish` together with the token to put in the `LoadRequest`.
    pub fn bind(&mut self) -> (u64, CancellationToken) {
        self.unbind();
        let token = CancellationToken::new();
        self.cancel = Some(token.clone());
        self.loading = true;
        (self.generation, token)
    }

    /// Cancels the load in flight and drops the image, as when the widget is recycled or unrealized.
    pub fn unbind(&mut self) {
        if let Some(token) = self.cancel.take() {
            token.cancel();
        }
        self.generation += 1;
        self.loading = false;
        self.state = CoverState::Placeholder;
    }

    /// Whether a load was started and has not finished yet.
    pub fn in_flight(&self) -> bool {
        self.loading
    }

    /// Applies the result of the load started with `generation`. A result of an older load is dropped, and so is
    /// a cancelled one, which is not an error to show. Returns whether the state changed.
    pub fn finish(&mut self, generation: u64, result: Result<LoadedImage, ImageError>) -> bool {
        if generation != self.generation {
            return false;
        }
        if !matches!(result, Err(ImageError::Cancelled)) {
            self.loading = false;
        }
        self.state = match result {
            Ok(loaded) => CoverState::Loaded(loaded.image),
            Err(ImageError::Cancelled) => return false,
            Err(error) => {
                tracing::debug!(%error, "cover failed to load");
                CoverState::Error
            }
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::DataSource;
    use bytes::Bytes;

    fn loaded() -> LoadedImage {
        LoadedImage {
            image: Arc::new(Decoded {
                width: 1,
                height: 1,
                rgba: Bytes::from_static(&[0, 0, 0, 255]),
                is_sampled: false,
            }),
            source: DataSource::Disk,
            key: "k".into(),
        }
    }

    #[test]
    fn ratios_are_square_book_and_thumb() {
        assert_eq!(CoverRatio::Square.ratio(), 1.0);
        assert!((CoverRatio::Book.ratio() - 2.0 / 3.0).abs() < 1e-12);
        assert!((CoverRatio::Thumb.ratio() - 16.0 / 9.0).abs() < 1e-12);
        assert_eq!(CoverRatio::Square.height_for_width(120), 120);
        assert_eq!(CoverRatio::Book.height_for_width(120), 180);
        assert_eq!(CoverRatio::Thumb.height_for_width(160), 90);
    }

    #[test]
    fn placeholder_is_1f888888() {
        assert_eq!(PLACEHOLDER_ARGB, 0x1F888888);
        assert_eq!(placeholder_css(), "rgba(136, 136, 136, 0.1216)");
    }

    #[test]
    fn starts_as_a_placeholder_and_shows_the_loaded_image() {
        let mut cover = CoverBinding::default();
        assert!(matches!(cover.state(), CoverState::Placeholder));
        let (generation, _) = cover.bind();
        assert!(matches!(cover.state(), CoverState::Placeholder));
        assert!(cover.finish(generation, Ok(loaded())));
        assert!(matches!(cover.state(), CoverState::Loaded(_)));
    }

    #[test]
    fn an_error_shows_the_error_image() {
        let mut cover = CoverBinding::default();
        let (generation, _) = cover.bind();
        assert!(cover.finish(generation, Err(ImageError::InvalidImage)));
        assert!(matches!(cover.state(), CoverState::Error));
        assert_eq!(ERROR_IMAGE, "cover_error");
    }

    #[test]
    fn a_cancelled_load_keeps_the_placeholder() {
        let mut cover = CoverBinding::default();
        let (generation, _) = cover.bind();
        assert!(!cover.finish(generation, Err(ImageError::Cancelled)));
        assert!(matches!(cover.state(), CoverState::Placeholder));
    }

    #[test]
    fn rebinding_cancels_the_old_load_and_drops_its_late_result() {
        let mut cover = CoverBinding::default();
        let (old, old_token) = cover.bind();
        let (new, new_token) = cover.bind();
        assert!(old_token.is_cancelled());
        assert!(!new_token.is_cancelled());
        assert!(!cover.finish(old, Ok(loaded())));
        assert!(matches!(cover.state(), CoverState::Placeholder));
        assert!(cover.finish(new, Ok(loaded())));
    }

    #[test]
    fn unbind_cancels_and_drops_the_texture() {
        let mut cover = CoverBinding::default();
        let (generation, token) = cover.bind();
        assert!(cover.finish(generation, Ok(loaded())));
        cover.unbind();
        assert!(token.is_cancelled());
        assert!(matches!(cover.state(), CoverState::Placeholder));
        assert!(!cover.finish(generation, Ok(loaded())));
    }
}
