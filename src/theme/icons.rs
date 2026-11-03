//! Icon names (`Icon`), the bundled icon theme registration, and the state-to-icon mappings for the
//! anime library tab, the entry caret, the reader modes and the download indicator (T-49ba-009,
//! T-49ba-012, T-49ba-015, T-49ba-019, T-49ba-025, T-49ba-026).
//!
//! Standard names come from the user's icon theme. The `kb-*` names are symbolic SVGs exported by
//! `gen/export_icons.py` into `icons/` and bundled as a GResource.

#[cfg(test)]
use gtk::prelude::*;
#[cfg(test)]
use kuro_i18n::{MsgKey, tr};

/// The application id, also the icon name of the full-colour app icon.
pub const APP_ID: &str = crate::channel::APP_ID;

/// The bundled icon theme root: `scalable/actions/`, `scalable/apps/` and `symbolic/apps/` below it.
pub const ICON_RESOURCE_PATH: &str = "/io/github/kurobayomi/Kurobayomi/icons";

#[allow(dead_code)] // kept for the shape of the type; not built or read yet
/// One icon the app shows. `name()` is the icon-theme name looked up at run time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Close,
    Play,
    Pause,
    SkipNext,
    SkipPrevious,
    Refresh,
    Share,
    Done,
    Info,
    Warning,
    Book,
    Folder,
    Extension,
    Photo,
    Server,
    UpdateAvailable,
    Calendar,
    DragHandle,
    CaretDown,
    CaretUp,
    MoreHorizontal,
    GoBack,
    HistoryTab,
    Cover,
    AnimeLibraryOutline,
    AnimeLibraryFilled,
    LibraryManga,
    UpdatesTab,
    Compass,
    /// The Browse tab: a globe with a search lens (Material `travel_explore`, user 2026-10-14).
    BrowseTab,
    DownloadCircle,
    DoneMarkPrevious,
    OverflowVertical,
    ReaderDefault,
    ReaderLtr,
    ReaderRtl,
    ReaderVertical,
    ReaderContinuousVertical,
    ReaderWebtoon,
    CropOn,
    CropOff,
    SeekTriangle,
    Forward10,
    TextShadow,
    FontFamily,
    Logo,
    LogoSymbolic,
    /// The logo with the box drawn behind it (welcome screen, README); the app icon is `Logo`.
    LogoShadow,
}

impl Icon {
    #[cfg(test)]
    /// Every variant, in declaration order.
    pub const ALL: [Icon; 48] = [
        Icon::Close,
        Icon::Play,
        Icon::Pause,
        Icon::SkipNext,
        Icon::SkipPrevious,
        Icon::Refresh,
        Icon::Share,
        Icon::Done,
        Icon::Info,
        Icon::Warning,
        Icon::Book,
        Icon::Folder,
        Icon::Extension,
        Icon::Photo,
        Icon::Server,
        Icon::UpdateAvailable,
        Icon::Calendar,
        Icon::DragHandle,
        Icon::CaretDown,
        Icon::CaretUp,
        Icon::MoreHorizontal,
        Icon::GoBack,
        Icon::HistoryTab,
        Icon::Cover,
        Icon::AnimeLibraryOutline,
        Icon::AnimeLibraryFilled,
        Icon::LibraryManga,
        Icon::UpdatesTab,
        Icon::Compass,
        Icon::BrowseTab,
        Icon::DownloadCircle,
        Icon::DoneMarkPrevious,
        Icon::OverflowVertical,
        Icon::ReaderDefault,
        Icon::ReaderLtr,
        Icon::ReaderRtl,
        Icon::ReaderVertical,
        Icon::ReaderContinuousVertical,
        Icon::ReaderWebtoon,
        Icon::CropOn,
        Icon::CropOff,
        Icon::SeekTriangle,
        Icon::Forward10,
        Icon::TextShadow,
        Icon::FontFamily,
        Icon::Logo,
        Icon::LogoSymbolic,
        Icon::LogoShadow,
    ];

    /// The icon-theme name of this icon.
    pub const fn name(self) -> &'static str {
        match self {
            Icon::Close => "window-close-symbolic",
            Icon::Play => "media-playback-start-symbolic",
            Icon::Pause => "media-playback-pause-symbolic",
            Icon::SkipNext => "media-skip-forward-symbolic",
            Icon::SkipPrevious => "media-skip-backward-symbolic",
            Icon::Refresh => "view-refresh-symbolic",
            Icon::Share => "document-send-symbolic",
            Icon::Done => "object-select-symbolic",
            Icon::Info => "dialog-information-symbolic",
            Icon::Warning => "dialog-warning-symbolic",
            Icon::Book => "accessories-dictionary-symbolic",
            Icon::Folder => "folder-symbolic",
            Icon::Extension => "application-x-addon-symbolic",
            Icon::Photo => "image-x-generic-symbolic",
            Icon::Server => "network-server-symbolic",
            Icon::UpdateAvailable => "software-update-available-symbolic",
            Icon::Calendar => "x-office-calendar-symbolic",
            Icon::DragHandle => "list-drag-handle-symbolic",
            Icon::CaretDown => "pan-down-symbolic",
            Icon::CaretUp => "pan-up-symbolic",
            Icon::MoreHorizontal => "view-more-horizontal-symbolic",
            Icon::GoBack => "go-previous-symbolic",
            Icon::HistoryTab => "document-open-recent-symbolic",
            Icon::Cover => "image-x-generic-symbolic",
            Icon::AnimeLibraryOutline => "kb-animelibrary-outline-symbolic",
            Icon::AnimeLibraryFilled => "kb-animelibrary-filled-symbolic",
            Icon::LibraryManga => "kb-library-symbolic",
            Icon::UpdatesTab => "kb-updates-outline-symbolic",
            Icon::Compass => "kb-compass-symbolic",
            Icon::BrowseTab => "kb-travel-explore-symbolic",
            Icon::DownloadCircle => "kb-download-circle-symbolic",
            Icon::DoneMarkPrevious => "kb-done-previous-symbolic",
            Icon::OverflowVertical => "kb-overflow-vertical-symbolic",
            Icon::ReaderDefault => "kb-reader-default-symbolic",
            Icon::ReaderLtr => "kb-reader-ltr-symbolic",
            Icon::ReaderRtl => "kb-reader-rtl-symbolic",
            Icon::ReaderVertical => "kb-reader-vertical-symbolic",
            Icon::ReaderContinuousVertical => "kb-reader-continuous-vertical-symbolic",
            Icon::ReaderWebtoon => "kb-reader-webtoon-symbolic",
            Icon::CropOn => "kb-crop-symbolic",
            Icon::CropOff => "kb-crop-off-symbolic",
            Icon::SeekTriangle => "kb-seek-triangle-symbolic",
            Icon::Forward10 => "kb-forward-10-symbolic",
            Icon::TextShadow => "kb-text-shadow-symbolic",
            Icon::FontFamily => "kb-font-family-symbolic",
            Icon::Logo => APP_ID,
            Icon::LogoSymbolic => crate::channel::APP_ID_SYMBOLIC,
            Icon::LogoShadow => "kb-logo-shadow",
        }
    }

    /// The file this icon is bundled as, relative to the icon theme root, or `None` for a standard
    /// name that the user's icon theme provides.
    #[cfg(test)]
    pub const fn bundled_path(self) -> Option<&'static str> {
        Some(match self {
            Icon::Logo => "scalable/apps/io.github.kurobayomi.Kurobayomi.svg",
            Icon::LogoSymbolic => "symbolic/apps/io.github.kurobayomi.Kurobayomi-symbolic.svg",
            Icon::LogoShadow => "scalable/apps/kb-logo-shadow.svg",
            Icon::AnimeLibraryOutline => "scalable/actions/kb-animelibrary-outline-symbolic.svg",
            Icon::AnimeLibraryFilled => "scalable/actions/kb-animelibrary-filled-symbolic.svg",
            Icon::LibraryManga => "scalable/actions/kb-library-symbolic.svg",
            Icon::UpdatesTab => "scalable/actions/kb-updates-outline-symbolic.svg",
            Icon::Compass => "scalable/actions/kb-compass-symbolic.svg",
            Icon::BrowseTab => "scalable/actions/kb-travel-explore-symbolic.svg",
            Icon::DownloadCircle => "scalable/actions/kb-download-circle-symbolic.svg",
            Icon::DoneMarkPrevious => "scalable/actions/kb-done-previous-symbolic.svg",
            Icon::OverflowVertical => "scalable/actions/kb-overflow-vertical-symbolic.svg",
            Icon::ReaderDefault => "scalable/actions/kb-reader-default-symbolic.svg",
            Icon::ReaderLtr => "scalable/actions/kb-reader-ltr-symbolic.svg",
            Icon::ReaderRtl => "scalable/actions/kb-reader-rtl-symbolic.svg",
            Icon::ReaderVertical => "scalable/actions/kb-reader-vertical-symbolic.svg",
            Icon::ReaderContinuousVertical => "scalable/actions/kb-reader-continuous-vertical-symbolic.svg",
            Icon::ReaderWebtoon => "scalable/actions/kb-reader-webtoon-symbolic.svg",
            Icon::CropOn => "scalable/actions/kb-crop-symbolic.svg",
            Icon::CropOff => "scalable/actions/kb-crop-off-symbolic.svg",
            Icon::SeekTriangle => "scalable/actions/kb-seek-triangle-symbolic.svg",
            Icon::Forward10 => "scalable/actions/kb-forward-10-symbolic.svg",
            Icon::TextShadow => "scalable/actions/kb-text-shadow-symbolic.svg",
            Icon::FontFamily => "scalable/actions/kb-font-family-symbolic.svg",
            _ => return None,
        })
    }
}

/// The icon bundle compiled by `build.rs`.
static BUNDLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/kurobayomi.gresource"));

/// Adds the bundled icon theme to the default display and makes it the app's only icon theme. Must run before the first
/// window is shown. A missing bundle logs one error and the app keeps the user's icons.
///
/// The app draws its own icons on every desktop (user, 2026-10-14): the bundle holds a Material icon under every standard
/// name the app uses (`gen/material_icons.py`) besides the `kb-*` ones. An icon theme is looked up before the bundle, and
/// a theme whose "-symbolic" icons are colour drawings showed them as blobs, so the theme is set to a name only the bundle
/// answers; GTK's own widgets keep GTK's built-in icons.
pub fn register(display: &gtk::gdk::Display) {
    match gtk::gio::Resource::from_data(&gtk::glib::Bytes::from_static(BUNDLE)) {
        Ok(bundle) => gtk::gio::resources_register(&bundle),
        Err(e) => tracing::error!(error = %e, "the icon bundle is unreadable"),
    }
    let sentinel = format!("{ICON_RESOURCE_PATH}/scalable/actions/kb-library-symbolic.svg");
    if gtk::gio::resources_lookup_data(&sentinel, gtk::gio::ResourceLookupFlags::NONE).is_err() {
        tracing::error!(path = ICON_RESOURCE_PATH, "bundled icons are missing; the app keeps the system icons");
        return;
    }
    gtk::IconTheme::for_display(display).add_resource_path(ICON_RESOURCE_PATH);
    // the display's icon theme follows the settings (it refuses a name of its own); a value the app sets outranks the
    // desktop's
    gtk::Settings::for_display(display).set_gtk_icon_theme_name(Some(APP_ICON_THEME));
}

/// The icon theme the app sets: no such theme is installed, so every lookup falls through to the bundle.
pub const APP_ICON_THEME: &str = "kurobayomi-own-icons";

#[cfg(test)]
/// The anime library tab icon: outline when not selected, filled when selected (T-49ba-015).
pub fn anime_library_icon(selected: bool) -> Icon {
    if selected { Icon::AnimeLibraryFilled } else { Icon::AnimeLibraryOutline }
}

#[cfg(test)]
/// The entry info header caret: up when expanded, down when collapsed (T-49ba-019).
pub fn caret_icon(expanded: bool) -> Icon {
    if expanded { Icon::CaretUp } else { Icon::CaretDown }
}

#[cfg(test)]
/// The caret tooltip: the collapse label when expanded, the expand label otherwise.
pub fn caret_tooltip(expanded: bool) -> String {
    tr(if expanded { MsgKey::manga_info_collapse } else { MsgKey::manga_info_expand })
}

#[cfg(test)]
/// The reading-mode icon for a `ReadingMode` constant name, as in `ReadingMode.kt`. An unknown
/// name is `DEFAULT`, as `ReadingMode.fromPreference` does (T-49ba-025).
pub fn reading_mode_icon(mode: &str) -> Icon {
    match mode {
        "LEFT_TO_RIGHT" => Icon::ReaderLtr,
        "RIGHT_TO_LEFT" => Icon::ReaderRtl,
        "VERTICAL" => Icon::ReaderVertical,
        "WEBTOON" => Icon::ReaderWebtoon,
        "CONTINUOUS_VERTICAL" => Icon::ReaderContinuousVertical,
        _ => Icon::ReaderDefault,
    }
}

#[cfg(test)]
/// The download indicator icon: shown for a downloaded row, nothing otherwise (T-49ba-026).
pub fn download_indicator_icon(downloaded: bool) -> Option<Icon> {
    downloaded.then_some(Icon::DownloadCircle)
}

#[cfg(test)]
/// The crop-borders toggle icon: on when crop borders is on (T-49ba-011, `BottomReaderBar.kt:59`).
pub fn crop_icon(crop_borders: bool) -> Icon {
    if crop_borders { Icon::CropOn } else { Icon::CropOff }
}

#[cfg(test)]
/// The crop-borders toggle tooltip. The reader's label catalogue has no key for it yet, so it is
/// the English text.
pub const CROP_TOOLTIP: &str = "Crop borders";

#[cfg(test)]
/// The centre player button: pause while playing, play while paused (T-49ba-024).
pub fn play_pause_icon(playing: bool) -> Icon {
    if playing { Icon::Pause } else { Icon::Play }
}

#[cfg(test)]
/// The icon name of each desktop shortcut, by action name (T-49ba-038). The names are the standard
/// icon names, or the bundled `kb-*` names where Android had a Kurobayomi drawable.
pub const SHORTCUT_ICONS: [(&str, &str); 5] = [
    ("library", Icon::LibraryManga.name()),
    ("animelib", Icon::AnimeLibraryFilled.name()),
    ("updates", Icon::UpdatesTab.name()),
    ("history", Icon::HistoryTab.name()),
    ("browse", Icon::BrowseTab.name()),
];

#[cfg(test)]
/// The icon name of a desktop shortcut action, or `None` for an unknown action.
pub fn shortcut_icon_name(action: &str) -> Option<&'static str> {
    SHORTCUT_ICONS.iter().find(|(name, _)| *name == action).map(|(_, icon)| *icon)
}

#[cfg(test)]
/// The icon of a notification (T-49ba-013). Standard names come from the user's icon theme; the
/// logo uses the application id, and no notification sets an accent colour.
pub fn notification_icon(icon: Icon) -> gtk::gio::Icon {
    gtk::gio::ThemedIcon::new(icon.name()).upcast()
}

#[cfg(test)]
/// The brand and extension marks (T-49b-024). They are SVG files in the bundle under `brand/`, with
/// `fill="currentColor"` so the CSS colour tints them. Only their links use them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brand {
    Discord,
    Github,
    Magnet,
}

#[cfg(test)]
impl Brand {
    /// The file name inside the bundle's `brand/` directory.
    pub const fn file_name(self) -> &'static str {
        match self {
            Brand::Discord => "discord.svg",
            Brand::Github => "github.svg",
            Brand::Magnet => "magnet.svg",
        }
    }

    /// The GResource path the image loads from.
    pub fn resource_path(self) -> String {
        format!("{ICON_RESOURCE_PATH}/brand/{}", self.file_name())
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A file under the bundled icon theme, by its path relative to the theme root.
    fn icon_file(relative: &str) -> PathBuf {
        PathBuf::from(format!("{}/src/theme/icons/{relative}", env!("CARGO_MANIFEST_DIR")))
    }

    #[test]
    fn names_match_the_table() {
        assert_eq!(Icon::Close.name(), "window-close-symbolic");
        assert_eq!(Icon::Pause.name(), "media-playback-pause-symbolic");
        assert_eq!(Icon::SkipPrevious.name(), "media-skip-backward-symbolic");
        assert_eq!(Icon::Warning.name(), "dialog-warning-symbolic");
        assert_eq!(Icon::CaretUp.name(), "pan-up-symbolic");
        assert_eq!(Icon::DownloadCircle.name(), "kb-download-circle-symbolic");
        assert_eq!(Icon::ReaderContinuousVertical.name(), "kb-reader-continuous-vertical-symbolic");
        assert_eq!(Icon::Logo.name(), APP_ID);
        assert_eq!(Icon::LogoSymbolic.name(), "io.github.kurobayomi.Kurobayomi-symbolic");
    }

    #[test]
    fn every_bundled_icon_has_its_file() {
        for icon in Icon::ALL {
            if let Some(path) = icon.bundled_path() {
                assert!(icon_file(path).is_file(), "{icon:?} missing {path}");
            }
        }
    }

    /// Every "<name>-symbolic" the code names has its file in the bundle: the app's icon theme answers only from it, so
    /// a name it lacks shows GTK's missing-icon picture on every desktop (the Repositories button did). Add the name to
    /// `src/theme/gen/material_icons.py` and run it.
    #[test]
    fn every_symbolic_name_in_the_code_is_bundled() {
        fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    sources(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        sources(&root.join("src"), &mut files);
        let mut missing = std::collections::BTreeSet::new();
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for (at, _) in text.match_indices("-symbolic\"") {
                let start = text[..at].rfind('"').map_or(at, |q| q + 1);
                let name = &text[start..at];
                if name.is_empty() || !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
                    continue;
                }
                let found = ["actions", "apps"].iter().any(|dir| root.join(format!("src/theme/icons/scalable/{dir}/{name}-symbolic.svg")).is_file());
                if !found {
                    missing.insert(format!("{name}-symbolic"));
                }
            }
        }
        assert!(missing.is_empty(), "not in the icon bundle: {missing:?}");
    }

    #[test]
    fn every_kb_name_is_bundled() {
        for icon in Icon::ALL {
            let name = icon.name();
            if name.starts_with("kb-") {
                assert!(icon.bundled_path().is_some(), "{name} has no bundled file");
            }
        }
    }

    /// T-49ba-009 (T2): every icon resolves after `register`. Needs a display and the GResource
    /// bundle; it skips itself when there is no display.
    #[test]
    fn every_icon_resolves_after_register() {
        crate::testing::with_display(|| {
            let display = gtk::gdk::Display::default().expect("a display");
            register(&display);
            let theme = gtk::IconTheme::for_display(&display);
            for icon in Icon::ALL {
                assert!(theme.has_icon(icon.name()), "{icon:?} ({}) does not resolve", icon.name());
            }
        });
    }

    #[test]
    fn standard_names_are_not_bundled() {
        for icon in [Icon::Close, Icon::Play, Icon::Warning, Icon::CaretDown, Icon::Cover] {
            assert_eq!(icon.bundled_path(), None);
        }
    }

    #[test]
    fn anime_library_follows_the_selection() {
        assert_eq!(anime_library_icon(false), Icon::AnimeLibraryOutline);
        assert_eq!(anime_library_icon(true), Icon::AnimeLibraryFilled);
    }

    #[test]
    fn caret_follows_the_expansion() {
        assert_eq!(caret_icon(true), Icon::CaretUp);
        assert_eq!(caret_icon(false), Icon::CaretDown);
        assert_eq!(caret_tooltip(true), "Less");
        assert_eq!(caret_tooltip(false), "More");
    }

    #[test]
    fn reading_modes_map_to_their_icons() {
        assert_eq!(reading_mode_icon("DEFAULT"), Icon::ReaderDefault);
        assert_eq!(reading_mode_icon("LEFT_TO_RIGHT"), Icon::ReaderLtr);
        assert_eq!(reading_mode_icon("RIGHT_TO_LEFT"), Icon::ReaderRtl);
        assert_eq!(reading_mode_icon("VERTICAL"), Icon::ReaderVertical);
        assert_eq!(reading_mode_icon("WEBTOON"), Icon::ReaderWebtoon);
        assert_eq!(reading_mode_icon("CONTINUOUS_VERTICAL"), Icon::ReaderContinuousVertical);
        assert_eq!(reading_mode_icon("nonsense"), Icon::ReaderDefault);
    }

    #[test]
    fn download_indicator_only_for_downloaded_rows() {
        assert_eq!(download_indicator_icon(true), Some(Icon::DownloadCircle));
        assert_eq!(download_indicator_icon(false), None);
    }

    /// T-49ba-011: the crop toggle shows the on icon while crop borders is on.
    #[test]
    fn crop_toggle_follows_the_setting() {
        assert_eq!(crop_icon(true), Icon::CropOn);
        assert_eq!(crop_icon(false), Icon::CropOff);
        assert_eq!(CROP_TOOLTIP, "Crop borders");
    }

    /// T-49ba-024: the centre button shows pause while playing and play while paused.
    #[test]
    fn play_button_shows_the_action_it_takes() {
        assert_eq!(play_pause_icon(true), Icon::Pause);
        assert_eq!(play_pause_icon(false), Icon::Play);
    }

    /// T-49ba-038: the five shortcut icon names, and nothing for an unknown action.
    #[test]
    fn shortcut_icons_use_the_planned_names() {
        assert_eq!(shortcut_icon_name("library"), Some("kb-library-symbolic"));
        assert_eq!(shortcut_icon_name("animelib"), Some("kb-animelibrary-filled-symbolic"));
        assert_eq!(shortcut_icon_name("updates"), Some("kb-updates-outline-symbolic"));
        assert_eq!(shortcut_icon_name("history"), Some("document-open-recent-symbolic"));
        assert_eq!(shortcut_icon_name("browse"), Some("kb-travel-explore-symbolic"));
        assert_eq!(shortcut_icon_name("show_recently_updated"), None);
    }

    /// T-49ba-013: notification icons are themed icons with the standard names.
    #[test]
    fn notification_icons_are_themed_with_standard_names() {
        let book = notification_icon(Icon::Book);
        let themed = book.downcast_ref::<gtk::gio::ThemedIcon>().expect("themed icon");
        assert_eq!(themed.names()[0].as_str(), "accessories-dictionary-symbolic");
        let logo = notification_icon(Icon::Logo);
        let logo = logo.downcast_ref::<gtk::gio::ThemedIcon>().expect("themed icon");
        assert_eq!(logo.names()[0].as_str(), "io.github.kurobayomi.Kurobayomi");
    }

    #[test]
    fn app_icon_is_full_colour_and_symbolic_is_separate() {
        assert_eq!(Icon::Logo.bundled_path(), Some("scalable/apps/io.github.kurobayomi.Kurobayomi.svg"));
        assert_eq!(
            Icon::LogoSymbolic.bundled_path(),
            Some("symbolic/apps/io.github.kurobayomi.Kurobayomi-symbolic.svg")
        );
    }

    fn svg_files(relative: &str) -> Vec<std::path::PathBuf> {
        let mut files: Vec<_> = std::fs::read_dir(icon_file(relative))
            .expect("icon directory")
            .map(|entry| entry.expect("entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "svg"))
            .collect();
        files.sort();
        files
    }

    /// T7: every symbolic export is filled with `#bebebe` and has no white fill.
    #[test]
    fn symbolic_exports_use_the_symbolic_fill_only() {
        let mut files = svg_files("scalable/actions");
        files.extend(svg_files("symbolic/apps"));
        assert!(files.len() >= 21, "found {} symbolic icons", files.len());
        for path in files {
            let text = std::fs::read_to_string(&path).expect("svg");
            assert!(text.contains("#bebebe"), "{} has no #bebebe fill", path.display());
            assert!(!text.contains("#FFF\""), "{} has a #FFF fill", path.display());
            assert!(!text.contains("#FFFFFF"), "{} has a #FFFFFF fill", path.display());
            assert!(text.contains("viewBox=\"0 0 "), "{} has no viewBox", path.display());
        }
    }

    /// 4.7: every action icon parses as one balanced `<svg>` element and has `viewBox="0 0 24 24"`.
    /// The two launcher icons are not 24 px glyphs and are checked above.
    #[test]
    fn action_icons_parse_and_use_the_24_px_viewbox() {
        for path in svg_files("scalable/actions") {
            let text = std::fs::read_to_string(&path).expect("svg");
            let name = path.display();
            let body = text.trim();
            assert!(body.starts_with("<svg") && body.ends_with("</svg>"), "{name} is not one svg element");
            assert_eq!(body.matches('<').count(), body.matches('>').count(), "{name} has unbalanced tags");
            assert_eq!(body.matches("<svg").count(), 1, "{name} nests an svg");
            assert!(body.contains("viewBox=\"0 0 24 24\""), "{name} has no 24 px viewBox");
        }
    }

    /// T-49ba-017: the full-colour app icon (黒箱 in white on a black box) keeps its colours.
    #[test]
    fn app_icon_keeps_the_launcher_colours() {
        let text = std::fs::read_to_string(icon_file("scalable/apps/io.github.kurobayomi.Kurobayomi.svg")).expect("svg");
        for colour in ["#101012", "#ffffff"] {
            assert!(text.contains(&format!("fill=\"{colour}\"")), "missing {colour}");
        }
        assert!(!text.contains("#bebebe"));
    }
}

#[cfg(test)]
mod brand_tests {
    use super::Brand;

    // T-49b-024: each mark is a 24-unit viewBox with one fill, and the bundle path matches the file.
    #[test]
    fn brand_svgs_are_tintable_single_fill_glyphs() {
        let files = [
            (Brand::Discord, include_str!("icons/brand/discord.svg")),
            (Brand::Github, include_str!("icons/brand/github.svg")),
            (Brand::Magnet, include_str!("icons/brand/magnet.svg")),
        ];
        for (brand, svg) in files {
            assert!(svg.contains(r#"viewBox="0 0 24 24""#), "{brand:?}");
            assert_eq!(svg.matches(" fill=").count(), 1, "{brand:?} has one fill");
            assert!(svg.contains(r#"fill="currentColor""#), "{brand:?} takes the CSS colour");
        }
    }

    #[test]
    fn brand_resource_paths_point_into_the_bundle() {
        assert_eq!(
            Brand::Magnet.resource_path(),
            "/io/github/kurobayomi/Kurobayomi/icons/brand/magnet.svg"
        );
    }
}

/// The release gate of T-47ba-049: every themed icon name the app asks for exists in the stock Adwaita theme or in
/// the bundle, so a desktop whose icon theme lacks a name still shows an icon.
#[cfg(test)]
mod name_gate {
    use std::collections::BTreeSet;
    use std::path::Path;

    #[cfg(unix)]
    fn svg_names(dir: &Path, out: &mut BTreeSet<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                svg_names(&path, out);
            } else if path.extension().is_some_and(|e| e == "svg") {
                if let Some(stem) = path.file_stem() {
                    out.insert(stem.to_string_lossy().into_owned());
                }
            }
        }
    }

    /// Every `"<name>-symbolic"` literal in the Rust sources and the spec files under `dir`.
    #[cfg(unix)]
    fn symbolic_names(dir: &Path, out: &mut BTreeSet<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // the bundle itself and generated data hold no icon requests
                if path.file_name().is_some_and(|n| n == "icons" || n == "fixtures" || n == "gen") {
                    continue;
                }
                symbolic_names(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs" || e == "json") {
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                for line in text.lines() {
                    for piece in line.split('"') {
                        let valid = !piece.is_empty() && piece.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
                        if valid && piece.len() > "-symbolic".len() && piece.ends_with("-symbolic") && !piece.starts_with("kb-") {
                            out.insert(piece.to_owned());
                        }
                    }
                }
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn every_symbolic_icon_name_the_app_asks_for_is_in_adwaita_or_bundled() {
        let adwaita = Path::new("/usr/share/icons/Adwaita");
        if !adwaita.is_dir() {
            // The gate has nothing to check against without Adwaita: fail, unless the skip is asked for.
            assert!(
                std::env::var_os("KURO_SKIP_ICON_GATE").is_some(),
                "the Adwaita icon theme is not installed at /usr/share/icons/Adwaita; install adwaita-icon-theme or set KURO_SKIP_ICON_GATE=1 to skip this gate"
            );
            eprintln!("skipped: KURO_SKIP_ICON_GATE is set and the Adwaita icon theme is not installed");
            return;
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut have = BTreeSet::new();
        svg_names(adwaita, &mut have);
        svg_names(&root.join("src/theme/icons"), &mut have);
        let mut asked = BTreeSet::new();
        symbolic_names(&root.join("src"), &mut asked);
        symbolic_names(&root.join("spec"), &mut asked);
        assert!(asked.len() > 50, "the scan finds the icon names ({} found)", asked.len());
        let missing: Vec<&String> = asked.iter().filter(|name| !have.contains(*name)).collect();
        assert!(missing.is_empty(), "add these to src/theme/icons/scalable/actions or use a stock name: {missing:?}");
    }
}
