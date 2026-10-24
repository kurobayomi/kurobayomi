//! Help links: pages of the Kurobayomi website's help (the `kurobayomi.github.io` repository, `website/docs`). They open
//! through [`crate::opener::Opener`]. A page moved on the site is one edit here.

macro_rules! site {
    ($path:literal) => {
        concat!("https://kurobayomi.github.io", $path)
    };
}

/// The help's front page, opened by Help in the app menu.
pub const URL_HELP_HOME: &str = site!("/help");

/// The troubleshooting guide, opened from the Help rows and the browse, search and migration help buttons.
pub const URL_HELP: &str = site!("/docs/guides/troubleshooting");

/// The FAQ entry the upcoming screens link to.
pub const URL_HELP_UPCOMING: &str = site!("/docs/faq/updates#upcoming");

/// The source migration guide, opened by the migration screens' help button.
pub const URL_SOURCE_MIGRATION: &str = site!("/docs/guides/source-migration");

/// What to do about library update errors, named at the top of the update error log.
pub const URL_UPDATE_ERRORS: &str = site!("/docs/guides/troubleshooting#library-update-errors");

/// Why a large library update or download is warned about, opened from that warning.
pub const URL_LARGE_UPDATES: &str = site!("/docs/faq/library#large-updates");
