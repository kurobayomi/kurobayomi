//! `[reader]` preferences (Aniyomi `ReaderPreferences`) and the reader value tables (`ReadingMode`, scale types
//! and zoom starts).
//!
//! The preference value for the reading mode is an `Int` code, so the typed enum here is a plain Rust
//! type with `from_preference` for the stored number. The screen orientation and the cut reader keys are
//! dropped, see `registry::DROPPED_KEYS`; the orientation bits of a stored viewer word are left as they are.

use crate::pref::Pref;
use crate::pref_enum;
use crate::registry::define_prefs;
use crate::values::PrefType;


pref_enum! {
    /// Flash colour of the page-change flash (`ReaderPreferences.FlashColor`).
    pub enum FlashColor { BLACK, WHITE, WHITE_BLACK }
}

pref_enum! {
    /// Which tap directions are inverted (`ReaderPreferences.TappingInvertMode`).
    pub enum TappingInvertMode { NONE, HORIZONTAL, VERTICAL, BOTH }
}

pref_enum! {
    /// Hide threshold of the reader overlay (`ReaderPreferences.ReaderHideThreshold`).
    pub enum ReaderHideThreshold { HIGHEST, HIGH, LOW, LOWEST }
}


impl TappingInvertMode {
    pub const fn should_invert_horizontal(self) -> bool {
        matches!(self, Self::HORIZONTAL | Self::BOTH)
    }

    pub const fn should_invert_vertical(self) -> bool {
        matches!(self, Self::VERTICAL | Self::BOTH)
    }
}

impl ReaderHideThreshold {
    /// The Kotlin `threshold` value.
    pub const fn threshold(self) -> i32 {
        match self {
            Self::HIGHEST => 5,
            Self::HIGH => 13,
            Self::LOW => 31,
            Self::LOWEST => 47,
        }
    }
}

/// Flash duration is stored in units of 100 ms (`pref_reader_flash_duration`).
pub const MILLI_CONVERSION: i32 = 100;

/// The webtoon side padding clamped to its UI range (0 to 25).
pub fn clamp_webtoon_side_padding(value: i32) -> i32 {
    value.clamp(0, 25)
}

/// `ReadingMode` (`ReadingMode.kt`). The flag value is the stored `pref_default_reading_mode_key` code;
/// the mask is [`READING_MODE_MASK`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadingMode {
    Default,
    LeftToRight,
    RightToLeft,
    Vertical,
    Webtoon,
    ContinuousVertical,
}

pub const READING_MODE_MASK: i32 = 0x7;

/// The viewer family a reading mode opens (Kotlin `ViewerType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewerType {
    Pager,
    Webtoon,
}

/// Returned for [`ReadingMode::Default`]: the preference must be resolved to a concrete mode before a
/// viewer is opened (Kotlin throws `IllegalStateException`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("reading mode DEFAULT must be resolved to the preference default before a viewer is created")]
pub struct UnresolvedReadingMode;

impl ReadingMode {
    pub const ALL: [ReadingMode; 6] = [
        Self::Default,
        Self::LeftToRight,
        Self::RightToLeft,
        Self::Vertical,
        Self::Webtoon,
        Self::ContinuousVertical,
    ];

    pub const fn flag_value(self) -> i32 {
        match self {
            Self::Default => 0x0,
            Self::LeftToRight => 0x1,
            Self::RightToLeft => 0x2,
            Self::Vertical => 0x3,
            Self::Webtoon => 0x4,
            Self::ContinuousVertical => 0x5,
        }
    }

    /// The mode for a stored code. Any unknown code (and `None` in Kotlin) gives [`ReadingMode::Default`].
    pub fn from_preference(code: i32) -> Self {
        Self::ALL.into_iter().find(|m| m.flag_value() == code).unwrap_or(Self::Default)
    }

    /// The viewer family, or [`UnresolvedReadingMode`] for [`ReadingMode::Default`].
    pub fn viewer_type(self) -> Result<ViewerType, UnresolvedReadingMode> {
        match self {
            Self::Default => Err(UnresolvedReadingMode),
            Self::LeftToRight | Self::RightToLeft | Self::Vertical => Ok(ViewerType::Pager),
            Self::Webtoon | Self::ContinuousVertical => Ok(ViewerType::Webtoon),
        }
    }

    /// Whether a stored code opens a pager viewer (LTR, RTL or vertical pager).
    pub fn is_pager_type(code: i32) -> bool {
        Self::from_preference(code).viewer_type() == Ok(ViewerType::Pager)
    }
}

/// Image scale types (`ReaderPreferences.ImageScaleType`). The UI stores index plus one in `image_scale_type`.
pub const IMAGE_SCALE_TYPE: [&str; 6] = [
    "scale_type_fit_screen",
    "scale_type_stretch",
    "scale_type_fit_width",
    "scale_type_fit_height",
    "scale_type_original_size",
    "scale_type_smart_fit",
];

/// Zoom start positions (`ReaderPreferences.ZoomStart`). The UI stores index plus one in `zoom_start`.
pub const ZOOM_START: [&str; 4] = [
    "zoom_start_automatic",
    "zoom_start_left",
    "zoom_start_right",
    "zoom_start_center",
];

define_prefs! {
    /// `[reader]` section: the reader keys the settings diet kept (Reader page: reading mode, background, scale type,
    /// zoom start, crop borders and split wide pages).
    pub struct ReaderPrefs in "reader" {
        default_reading_mode: i32 = "pref_default_reading_mode_key" => 1;
        image_scale_type: i32 = "pref_image_scale_type_key" => 1;
        zoom_start: i32 = "pref_zoom_start_key" => 1;
        reader_theme: i32 = "pref_reader_theme_key" => 1;
        crop_borders: bool = "crop_borders" => false;
        dual_page_split_paged: bool = "pref_dual_page_split" => false;
    }
}

/// The reader settings of one manga (user, 2026-10-24): the reader's own settings window writes these, the Settings page
/// writes the [`ReaderPrefs`] defaults. A manga key that was never written reads as the default's current value, so a
/// manga follows the default until a value is picked for it. The reading mode is kept in the manga's viewer flags.
#[derive(Clone)]
pub struct MangaReaderPrefs {
    pub defaults: ReaderPrefs,
    manga_id: i64,
}

impl MangaReaderPrefs {
    pub fn new(defaults: ReaderPrefs, manga_id: i64) -> Self {
        Self { defaults, manga_id }
    }

    pub fn reader_theme(&self) -> Pref<i32> {
        self.own("manga_reader_theme", &self.defaults.reader_theme)
    }

    pub fn image_scale_type(&self) -> Pref<i32> {
        self.own("manga_image_scale_type", &self.defaults.image_scale_type)
    }

    pub fn zoom_start(&self) -> Pref<i32> {
        self.own("manga_zoom_start", &self.defaults.zoom_start)
    }

    pub fn crop_borders(&self) -> Pref<bool> {
        self.own("manga_crop_borders", &self.defaults.crop_borders)
    }

    pub fn dual_page_split_paged(&self) -> Pref<bool> {
        self.own("manga_dual_page_split", &self.defaults.dual_page_split_paged)
    }

    /// The manga's key `<prefix>_<id>`, read fresh each time so an unset key follows the default as it is now.
    fn own<T: PrefType>(&self, prefix: &str, default: &Pref<T>) -> Pref<T> {
        self.defaults.store().pref(format!("{prefix}_{}", self.manga_id), default.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{REGISTRY, lookup};
    use crate::store::PrefStore;
    use crate::testutil::assert_section_registered;
    use crate::values::{Decoded, PrefType, PrefValue};

    #[test]
    fn a_manga_follows_the_default_until_its_own_value_is_picked() {
        let reader = ReaderPrefs::new(PrefStore::in_memory());
        let (one, two) = (MangaReaderPrefs::new(reader.clone(), 1), MangaReaderPrefs::new(reader.clone(), 2));
        reader.crop_borders.set(true).unwrap();
        assert!(one.crop_borders().get() && two.crop_borders().get(), "both follow the default");
        one.crop_borders().set(false).unwrap();
        assert!(!one.crop_borders().get(), "the manga's own value");
        assert!(two.crop_borders().get() && reader.crop_borders.get(), "the other manga and the default are untouched");
        reader.crop_borders.set(false).unwrap();
        reader.crop_borders.set(true).unwrap();
        assert!(!one.crop_borders().get(), "a picked value stays when the default changes");
        reader.reader_theme.set(2).unwrap();
        one.zoom_start().set(3).unwrap();
        assert_eq!((one.reader_theme().get(), one.zoom_start().get(), two.zoom_start().get()), (2, 3, 1));
    }

    #[test]
    fn the_manga_keys_are_registered_families() {
        let one = MangaReaderPrefs::new(ReaderPrefs::new(PrefStore::in_memory()), 7);
        for key in [
            one.reader_theme().key().to_owned(),
            one.image_scale_type().key().to_owned(),
            one.zoom_start().key().to_owned(),
            one.crop_borders().key().to_owned(),
            one.dual_page_split_paged().key().to_owned(),
        ] {
            assert_eq!(lookup(&key).and_then(|(_, id)| id), Some("7"), "{key}");
        }
    }

    #[test]
    fn the_section_holds_every_reader_row() {
        assert_eq!(ReaderPrefs::FIELDS.len(), 6);
        let reader = ReaderPrefs::new(PrefStore::in_memory());
        assert_section_registered(ReaderPrefs::TABLE, ReaderPrefs::FIELDS, &reader.defaults());
    }

    #[test]
    fn cut_reader_keys_are_dropped_not_registered() {
        for key in ["reader_volume_keys", "reader_volume_keys_inverted", "pref_default_orientation_type_key"] {
            assert!(lookup(key).is_none(), "{key}");
            assert!(crate::registry::is_dropped(key), "{key}");
        }
    }

    #[test]
    fn reading_mode_codes_and_pager_check() {
        for (code, mode) in [
            (0, ReadingMode::Default),
            (1, ReadingMode::LeftToRight),
            (2, ReadingMode::RightToLeft),
            (3, ReadingMode::Vertical),
            (4, ReadingMode::Webtoon),
            (5, ReadingMode::ContinuousVertical),
        ] {
            assert_eq!(ReadingMode::from_preference(code), mode);
            assert_eq!(mode.flag_value(), code);
        }
        assert_eq!(ReadingMode::from_preference(9), ReadingMode::Default);
        assert_eq!(ReadingMode::from_preference(-1), ReadingMode::Default);
        assert!(ReadingMode::is_pager_type(1) && ReadingMode::is_pager_type(2) && ReadingMode::is_pager_type(3));
        assert!(!ReadingMode::is_pager_type(4) && !ReadingMode::is_pager_type(5));
        assert_eq!(ReadingMode::Default.viewer_type(), Err(UnresolvedReadingMode));
        assert_eq!(ReadingMode::ContinuousVertical.viewer_type(), Ok(ViewerType::Webtoon));
        assert_eq!(READING_MODE_MASK, 0x7);
    }

    #[test]
    fn hide_threshold_and_tap_inversion_values() {
        let thresholds: Vec<i32> = [
            ReaderHideThreshold::HIGHEST,
            ReaderHideThreshold::HIGH,
            ReaderHideThreshold::LOW,
            ReaderHideThreshold::LOWEST,
        ]
        .iter()
        .map(|t| t.threshold())
        .collect();
        assert_eq!(thresholds, [5, 13, 31, 47]);
        assert!(TappingInvertMode::BOTH.should_invert_horizontal() && TappingInvertMode::BOTH.should_invert_vertical());
        assert!(TappingInvertMode::HORIZONTAL.should_invert_horizontal());
        assert!(!TappingInvertMode::HORIZONTAL.should_invert_vertical());
        assert!(!TappingInvertMode::NONE.should_invert_horizontal());
    }

    #[test]
    fn webtoon_padding_is_clamped_to_its_range() {
        assert_eq!(clamp_webtoon_side_padding(-4), 0);
        assert_eq!(clamp_webtoon_side_padding(30), 25);
        assert_eq!(clamp_webtoon_side_padding(12), 12);
        assert_eq!(MILLI_CONVERSION, 100);
    }

    #[test]
    fn enum_values_are_stored_by_their_kotlin_names() {
        assert_eq!(FlashColor::WHITE_BLACK.encode(), PrefValue::Str("WHITE_BLACK".into()));
        assert_eq!(
            TappingInvertMode::decode(&PrefValue::Str("VERTICAL".into())),
            Decoded::Ok(TappingInvertMode::VERTICAL)
        );
        assert!(!REGISTRY.iter().any(|m| m.key == "reader_tapping_inverted_webtoon"), "the tapping inversion was cut");
        assert!(crate::registry::is_dropped("reader_tapping_inverted_webtoon"));
    }

}
