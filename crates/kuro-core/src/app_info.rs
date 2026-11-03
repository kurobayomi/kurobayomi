//! Application identity and build metadata.

use std::sync::LazyLock;

/// Integer version code; Android's `versionCode` is dropped (T-54a-022), this starts at 1.
pub const VERSION_CODE: u32 = 1;

/// Build identity read from the `KB_*` variables set by build.rs.
pub struct AppInfo {
    pub app_id: &'static str,
    pub version_name: String,
    pub version_code: u32,
    pub git_sha: &'static str,
    pub commit_count: u32,
    pub build_time: &'static str,
    pub build_type: &'static str,
    /// True when built with the `updater` feature.
    pub updater_enabled: bool,
}

pub static APP_INFO: LazyLock<AppInfo> = LazyLock::new(|| {
    let commit_count = env!("KB_COMMIT_COUNT").parse().unwrap_or(0);
    let build_type = env!("KB_BUILD_TYPE");
    AppInfo {
        app_id: env!("KB_APP_ID"),
        version_name: version_name(env!("CARGO_PKG_VERSION"), build_type, commit_count),
        version_code: VERSION_CODE,
        git_sha: env!("KB_GIT_SHA"),
        commit_count,
        build_time: env!("KB_BUILD_TIME"),
        build_type,
        updater_enabled: cfg!(feature = "updater"),
    }
});

/// Debug builds append `-<commit count>`; release builds use the bare version.
pub fn version_name(base: &str, build_type: &str, commit_count: u32) -> String {
    if build_type == "debug" { format!("{base}-{commit_count}") } else { base.to_owned() }
}

/// The `--version` line: `Kurobayomi <version> (<sha>, <code>, <build time>)`.
pub fn version_line() -> String {
    let i = &*APP_INFO;
    format!("Kurobayomi {} ({}, {}, {})", i.version_name, i.git_sha, i.version_code, i.build_time)
}

/// MIME types the image decoders in the workspace can read, in sorted order.
/// heif and jxl are dropped: the workspace `image` dependency enables no decoder for them.
pub fn supported_image_mime_types() -> &'static [&'static str] {
    &["image/avif", "image/gif", "image/jpeg", "image/png", "image/webp"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_version_name_appends_commit_count() {
        assert_eq!(version_name("0.1.0", "debug", 42), "0.1.0-42");
    }

    #[test]
    fn release_version_name_is_bare() {
        assert_eq!(version_name("0.1.0", "release", 42), "0.1.0");
    }

    #[test]
    fn version_line_has_four_fields() {
        let line = version_line();
        assert!(line.starts_with("Kurobayomi "), "{line}");
        assert!(line.contains(&format!("({}, 1, ", APP_INFO.git_sha)), "{line}");
        assert!(line.ends_with(&format!("{})", APP_INFO.build_time)), "{line}");
    }

    #[test]
    fn the_build_type_follows_the_profile() {
        // dev builds (debug assertions on) report "debug"; release, preview and dist report their own name
        assert_eq!(APP_INFO.build_type == "debug", cfg!(debug_assertions), "{}", APP_INFO.build_type);
        assert!(!APP_INFO.build_type.is_empty());
    }

    #[test]
    fn updater_flag_follows_the_cargo_feature() {
        assert_eq!(APP_INFO.updater_enabled, cfg!(feature = "updater"));
    }

    #[test]
    fn app_id_and_version_code() {
        assert_eq!(APP_INFO.app_id, "io.github.kurobayomi.Kurobayomi");
        assert_eq!(APP_INFO.version_code, 1);
    }

    #[test]
    fn image_mime_types_are_sorted_and_exact() {
        assert_eq!(
            supported_image_mime_types(),
            ["image/avif", "image/gif", "image/jpeg", "image/png", "image/webp"]
        );
    }
}
