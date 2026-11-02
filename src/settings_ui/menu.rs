//! The settings main menu (task T-47a-003) and the two-pane background rule.
//! Titles, subtitles and icons follow the Linux spec (`spec/settings.json`, `main`); About shows the build.
// Not reachable from `route.rs` yet: the main list in `pref_model/shell.rs` still reads `spec/settings.json`.

#[cfg(test)]
/// One row of the main menu. `target` is the screen id the row opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRow {
    pub title: &'static str,
    pub subtitle: String,
    pub icon: &'static str,
    pub target: &'static str,
}

/// About subtitle: `"<app> Stable <version>"` for release builds, `"Debug <commit>"` otherwise.
pub fn about_subtitle(app: &str, release: bool, version: &str, commit: &str) -> String {
    if release { format!("{app} Stable {version}") } else { format!("Debug {commit}") }
}

#[cfg(test)]
/// The eight rows in their order. `about` is the About subtitle (see [`about_subtitle`]).
pub fn rows(about: &str) -> Vec<MenuRow> {
    let row = |title, subtitle: &str, icon, target| MenuRow { title, subtitle: subtitle.to_owned(), icon, target };
    vec![
        row("Appearance", "Theme and start screen", "preferences-desktop-appearance-symbolic", "settings_appearance"),
        row("Library", "Categories, global update", "view-grid-symbolic", "settings_library"),
        row("Reader", "Reading mode, display, navigation", "x-office-document-symbolic", "settings_reader"),
        row("Player", "Languages, quality, volume, autoplay, controls", "video-display-symbolic", "settings_player"),
        row("Downloads and storage", "Maximum downloads, auto-download, cleanup", "folder-download-symbolic", "settings_downloads"),
        row("Sources and tracking", "Sources, repositories, AniList, cookies", "compass-symbolic", "settings_browse"),
        row("Backup and restore", "Automatic and manual backups, maintenance", "drive-harddisk-symbolic", "settings_data_storage"),
        row("About", about, "help-about-symbolic", "settings_about"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_rows_in_the_task_order() {
        let titles: Vec<&str> = rows("x").iter().map(|r| r.title).collect();
        assert_eq!(
            titles,
            [
                "Appearance",
                "Library",
                "Reader",
                "Player",
                "Downloads and storage",
                "Sources and tracking",
                "Backup and restore",
                "About"
            ]
        );
    }

    #[test]
    fn no_row_routes_to_the_storage_tab() {
        // T-47ba-017: the Storage tab is reached from Updates; the settings list opens the settings screens only
        assert!(rows("x").iter().all(|r| !r.target.starts_with("storage")));
    }

    #[test]
    fn player_row_opens_the_player_root_and_about_takes_the_given_subtitle() {
        let rows = rows("Kurobayomi Stable 0.1.0");
        assert_eq!(rows[3].target, "settings_player");
        assert_eq!(rows[7].subtitle, "Kurobayomi Stable 0.1.0");
    }

    #[test]
    fn about_subtitle_is_release_or_debug() {
        assert_eq!(about_subtitle("Kurobayomi", true, "0.1.0", "abc"), "Kurobayomi Stable 0.1.0");
        assert_eq!(about_subtitle("Kurobayomi", false, "0.1.0", "abc1234"), "Debug abc1234");
    }

}
