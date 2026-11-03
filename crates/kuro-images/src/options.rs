//! `LoadOptions`, `CachePolicy`, `RequestSize` and `Scale`.

/// Whether each cache layer is read and written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CachePolicy {
    #[default]
    Enabled,
    ReadOnly,
    WriteOnly,
    Disabled,
}

impl CachePolicy {
    pub fn read_enabled(self) -> bool {
        matches!(self, CachePolicy::Enabled | CachePolicy::ReadOnly)
    }

    pub fn write_enabled(self) -> bool {
        matches!(self, CachePolicy::Enabled | CachePolicy::WriteOnly)
    }
}

/// The size the image is wanted at. `Original` skips downsampling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RequestSize {
    #[default]
    Original,
    /// Target pixels. `(0, 0)` is undefined and samples at 1 (edge E-IMG-7).
    Pixels { width: u32, height: u32 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scale {
    #[default]
    Fit,
    Fill,
}

/// Which queue a request waits in. `High` is for what the user is looking at right now (the open entry page's cover and
/// backdrop): it has slots of its own, so it never waits behind the thumbnails of a long list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Priority {
    #[default]
    Normal,
    High,
}

/// Per-request options. Defaults are Coil's, plus `use_custom_cover`, the only
/// Kotlin extra that defaults to true.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadOptions {
    pub use_custom_cover: bool,
    pub use_background: bool,
    pub crop_borders: bool,
    pub custom_decoder: bool,
    pub memory_cache: CachePolicy,
    pub disk_cache: CachePolicy,
    pub network_cache: CachePolicy,
    pub size: RequestSize,
    pub scale: Scale,
    pub priority: Priority,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            use_custom_cover: true,
            use_background: false,
            crop_borders: false,
            custom_decoder: false,
            memory_cache: CachePolicy::Enabled,
            disk_cache: CachePolicy::Enabled,
            network_cache: CachePolicy::Enabled,
            size: RequestSize::Original,
            scale: Scale::Fit,
            priority: Priority::Normal,
        }
    }
}

impl LoadOptions {
    /// Options of a webtoon reader page: no memory or disk cache, the native decoder, and the crop setting.
    /// The crossfade length comes from `motion::crossfade`.
    pub fn webtoon_page(crop_borders: bool) -> Self {
        Self {
            crop_borders,
            custom_decoder: true,
            memory_cache: CachePolicy::Disabled,
            disk_cache: CachePolicy::Disabled,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_data_table() {
        let o = LoadOptions::default();
        assert!(o.use_custom_cover);
        assert!(!o.use_background);
        assert!(!o.crop_borders);
        assert!(!o.custom_decoder);
        assert_eq!(o.memory_cache, CachePolicy::Enabled);
        assert_eq!(o.disk_cache, CachePolicy::Enabled);
        assert_eq!(o.network_cache, CachePolicy::Enabled);
        assert_eq!(o.size, RequestSize::Original);
        assert_eq!(o.scale, Scale::Fit);
    }

    #[test]
    fn cache_policy_read_and_write_flags() {
        assert!(CachePolicy::Enabled.read_enabled() && CachePolicy::Enabled.write_enabled());
        assert!(CachePolicy::ReadOnly.read_enabled() && !CachePolicy::ReadOnly.write_enabled());
        assert!(!CachePolicy::WriteOnly.read_enabled() && CachePolicy::WriteOnly.write_enabled());
        assert!(!CachePolicy::Disabled.read_enabled() && !CachePolicy::Disabled.write_enabled());
    }

    #[test]
    fn webtoon_page_disables_memory_and_disk_cache() {
        let o = LoadOptions::webtoon_page(true);
        assert!(o.crop_borders);
        assert!(o.custom_decoder);
        assert_eq!(o.memory_cache, CachePolicy::Disabled);
        assert_eq!(o.disk_cache, CachePolicy::Disabled);
        assert_eq!(o.network_cache, CachePolicy::Enabled);
        assert!(o.use_custom_cover);
    }
}
