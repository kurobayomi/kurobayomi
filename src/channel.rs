//! Which release channel this build belongs to. The nightly workflow sets `KB_CHANNEL=nightly` and `KB_COMMIT=<hash>`;
//! every other build (local, package) has neither.

pub const NIGHTLY: bool = kuro_platform::channel::NIGHTLY;

/// The GitHub repository the releases (stable and nightly) and the commits come from.
pub const REPOSITORY: &str = "kurobayomi/kurobayomi";

/// The application id: the nightly build is a separate app (own window class, own single instance, own data folders).
pub const APP_ID: &str = if NIGHTLY { "io.github.kurobayomi.Kurobayomi.Nightly" } else { "io.github.kurobayomi.Kurobayomi" };

/// The application ids this channel had before (`old_app_ids` of `identity.toml`, handed over by `build.rs`).
pub fn old_app_ids() -> impl Iterator<Item = String> {
    env!("KB_OLD_APP_IDS").split(',').filter(|id| !id.is_empty()).map(|id| if NIGHTLY { format!("{id}.Nightly") } else { id.to_owned() })
}

/// The icon name of the monochrome app icon.
pub const APP_ID_SYMBOLIC: &str = if NIGHTLY { "io.github.kurobayomi.Kurobayomi.Nightly-symbolic" } else { "io.github.kurobayomi.Kurobayomi-symbolic" };

/// What the window, the sidebar, the splash and the About page call the app.
pub const APP_NAME: &str = if NIGHTLY { "Kurobayomi Nightly" } else { "Kurobayomi" };

/// The commit a nightly build was made from; `None` for any other build.
pub fn commit() -> Option<&'static str> {
    option_env!("KB_COMMIT").filter(|commit| NIGHTLY && !commit.is_empty())
}

/// The first 8 characters of [`commit`], as shown to people.
pub fn short_commit() -> &'static str {
    commit().map_or("", |commit| &commit[..8.min(commit.len())])
}
