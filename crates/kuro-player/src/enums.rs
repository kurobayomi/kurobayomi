//! Player enums ported from `PlayerEnums.kt` and the preference enums the mpv options read
//! (area 45 data doc section 2.9). Stored values use the Kotlin enum names; the mpv-facing
//! strings are exposed through `mpv_value` and `value` helpers.

/// Result of a set-as-art action (`PlayerEnums.kt:29-33`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetAsArt {
    Success,
    AddToLibraryFirst,
    Error,
}

/// Artwork slot that a set-as-art action writes (`PlayerEnums.kt:35-39`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtType {
    Cover,
    Background,
    Thumbnail,
}

/// Video aspect mode (`PlayerEnums.kt:52-56`). Default `Fit` (`PlayerPreferences.kt:81`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VideoAspect {
    Crop,
    #[default]
    Fit,
    Stretch,
}

/// Rotation the desktop control sets (T-45a-035): mpv `video-rotate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VideoRotate {
    #[default]
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

impl VideoRotate {
    /// The value mpv takes for `video-rotate`.
    pub fn degrees(self) -> i64 {
        match self {
            Self::Deg0 => 0,
            Self::Deg90 => 90,
            Self::Deg180 => 180,
            Self::Deg270 => 270,
        }
    }
}

/// Bottom sheets (`PlayerEnums.kt:99-108`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Sheets {
    #[default]
    None,
    PlaybackSpeed,
    SubtitleTracks,
    AudioTracks,
    QualityTracks,
    Chapters,
    More,
    Screenshot,
}

impl Sheets {
    /// The id the mockup spec uses for each sheet (`spec/reader_player.json`).
    pub const fn spec_id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::PlaybackSpeed => "playback_speed",
            Self::SubtitleTracks => "subtitle_tracks",
            Self::AudioTracks => "audio_tracks",
            Self::QualityTracks => "quality_tracks",
            Self::Chapters => "chapters",
            Self::More => "more",
            Self::Screenshot => "screenshot",
        }
    }

    pub const ALL: [Self; 8] = [
        Self::None,
        Self::PlaybackSpeed,
        Self::SubtitleTracks,
        Self::AudioTracks,
        Self::QualityTracks,
        Self::Chapters,
        Self::More,
        Self::Screenshot,
    ];
}

/// Side panels (`PlayerEnums.kt:110-116`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Panels {
    #[default]
    None,
    SubtitleSettings,
    SubtitleDelay,
    AudioDelay,
    VideoFilters,
}

impl Panels {
    pub const ALL: [Self; 5] = [
        Self::None,
        Self::SubtitleSettings,
        Self::SubtitleDelay,
        Self::AudioDelay,
        Self::VideoFilters,
    ];
}

/// Dialogs (`PlayerEnums.kt:118-131`). The Kotlin `onChange` and `onDismissRequest` callbacks
/// stay in the GTK layer; this value carries the data only.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Dialogs {
    #[default]
    None,
    EpisodeList,
    IntegerPicker {
        default_value: i32,
        min_value: i32,
        max_value: i32,
        step: i32,
        name_format: String,
        title: String,
    },
}

/// Hardware decoder choice (`PlayerEnums.kt:81-91`). `mpv_value` is the Android value stored
/// by the Kotlin app; Linux candidates are resolved by `options::resolve_hwdec`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decoder {
    AutoCopy,
    Auto,
    Sw,
    Hw,
    HwPlus,
}

impl Decoder {
    pub const fn mpv_value(self) -> &'static str {
        match self {
            Self::AutoCopy => "auto-copy",
            Self::Auto => "auto",
            Self::Sw => "no",
            Self::Hw => "mediacodec-copy",
            Self::HwPlus => "mediacodec",
        }
    }
}

/// The GPU vendor that picks the Linux `hwdec` names (design table §8.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuKind {
    Intel,
    Amd,
    Nvidia,
    #[default]
    Other,
}

/// Video debanding filter (`PlayerEnums.kt:93-97`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Debanding {
    #[default]
    None,
    Cpu,
    Gpu,
}

/// Subtitle text justification (`SubtitlePreferences.kt:56-64`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SubtitleJustification {
    Left,
    Center,
    Right,
    #[default]
    Auto,
}

impl SubtitleJustification {
    pub const fn value(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
            Self::Auto => "auto",
        }
    }
}

/// Subtitle border style (`SubtitleSettingsTypographyCard.kt:368-375`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SubtitlesBorderStyle {
    #[default]
    OutlineAndShadow,
    OpaqueBox,
    BackgroundBox,
}

impl SubtitlesBorderStyle {
    pub const fn value(self) -> &'static str {
        match self {
            Self::OutlineAndShadow => "outline-and-shadow",
            Self::OpaqueBox => "opaque-box",
            Self::BackgroundBox => "background-box",
        }
    }
}

/// Audio channel layout (`AudioPreferences.kt:43-49`). `option` is the mpv option or property
/// name that receives `value`. `ReverseStereo` writes the `af` filter instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AudioChannels {
    Auto,
    #[default]
    AutoSafe,
    Mono,
    Stereo,
    ReverseStereo,
}

impl AudioChannels {
    pub const fn setting(self) -> (&'static str, &'static str) {
        match self {
            Self::Auto => ("audio-channels", "auto-safe"),
            Self::AutoSafe => ("audio-channels", "auto"),
            Self::Mono => ("audio-channels", "mono"),
            Self::Stereo => ("audio-channels", "stereo"),
            Self::ReverseStereo => ("af", "pan=[stereo|c0=c1|c1=c0]"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_preferences() {
        assert_eq!(VideoAspect::default(), VideoAspect::Fit);
        assert_eq!(Sheets::default(), Sheets::None);
        assert_eq!(Dialogs::default(), Dialogs::None);
        assert_eq!(Debanding::default(), Debanding::None);
        assert_eq!(AudioChannels::default(), AudioChannels::AutoSafe);
    }
}
