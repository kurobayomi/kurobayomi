//! The files packaging installs (T-53a-002, T-53a-003, T-53a-011, T-53a-041): the desktop entry, the shared-mime-info
//! file, the hicolor icons and the post-install hook. They live in `data/` next to this module; the tests hold them to
//! a golden copy and to the launch code that serves them.

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use crate::shell::argv;

    const DESKTOP: &str = include_str!("data/io.github.anas1412.Kurobayomi.desktop");
    const MIME: &str = include_str!("data/io.github.anas1412.Kurobayomi.xml");
    const POST_INSTALL: &str = include_str!("data/post-install.sh");

    /// The golden copy of section 2.6, TC-53-36.
    const DESKTOP_GOLDEN: &str = "[Desktop Entry]
Type=Application
Name=Kurobayomi
GenericName=Manga and anime reader
Comment=Read manga and watch anime
Exec=kurobayomi %U
Icon=io.github.anas1412.Kurobayomi
Terminal=false
Categories=AudioVideo;Video;Player;
StartupNotify=true
DBusActivatable=true
StartupWMClass=io.github.anas1412.Kurobayomi
MimeType=application/x-tachibk;x-scheme-handler/kurobayomi;
Actions=show-library;show-animelib;show-updates;show-history;show-browse;

[Desktop Action show-library]
Name=Manga library
Exec=kurobayomi --action=show-library

[Desktop Action show-animelib]
Name=Anime library
Exec=kurobayomi --action=show-animelib

[Desktop Action show-updates]
Name=Updates
Exec=kurobayomi --action=show-updates

[Desktop Action show-history]
Name=History
Exec=kurobayomi --action=show-history

[Desktop Action show-browse]
Name=Browse
Exec=kurobayomi --action=show-browse
";

    const MIME_GOLDEN: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">
  <mime-type type=\"application/x-tachibk\">
    <comment>Kurobayomi backup</comment>
    <sub-class-of type=\"application/gzip\"/>
    <glob pattern=\"*.tachibk\"/>
  </mime-type>
</mime-info>
";

    fn data(name: &str) -> String {
        format!("{}/src/shell/data/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    /// Runs a checker. A missing tool is a failure: CI installs desktop-file-utils and libxml2-utils (EF-01, EF-05).
    #[cfg(unix)]
    fn run(tool: &str, args: &[&str]) -> bool {
        Command::new(tool).args(args).output().unwrap_or_else(|e| panic!("{tool} is required for this test: {e}")).status.success()
    }

    /// xmllint is required on CI (`CI` set); elsewhere a missing tool skips the check with a note.
    #[cfg(unix)]
    fn xmllint_available() -> bool {
        if Command::new("xmllint").arg("--version").output().is_ok() {
            return true;
        }
        assert!(std::env::var_os("CI").is_none(), "xmllint is required for this test on CI (libxml2-utils)");
        eprintln!("skipping the xmllint check: xmllint is not installed (package libxml2-utils)");
        false
    }

    #[test]
    fn tc_53_36_the_desktop_entry_is_the_golden_copy_and_has_the_launch_keys() {
        assert_eq!(DESKTOP, DESKTOP_GOLDEN);
        for key in ["Exec=kurobayomi %U", "Icon=io.github.anas1412.Kurobayomi", "StartupWMClass=io.github.anas1412.Kurobayomi", "DBusActivatable=true", "StartupNotify=true"] {
            assert!(DESKTOP.lines().any(|l| l == key), "{key}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn desktop_file_validate_exits_zero() {
        assert!(run("desktop-file-validate", &[&data("io.github.anas1412.Kurobayomi.desktop")]));
    }

    #[test]
    fn the_desktop_actions_are_five_distinct_groups_that_the_launcher_understands() {
        let groups: Vec<&str> = DESKTOP.lines().filter_map(|l| l.strip_prefix("[Desktop Action ")?.strip_suffix(']')).collect();
        assert_eq!(groups, ["show-library", "show-animelib", "show-updates", "show-history", "show-browse"]);
        // TC-53-19: the five action ids are distinct
        assert_eq!(groups.iter().collect::<std::collections::HashSet<_>>().len(), 5);
        let listed = DESKTOP.lines().find_map(|l| l.strip_prefix("Actions=")).unwrap_or_default();
        assert_eq!(listed.trim_end_matches(';').split(';').collect::<Vec<_>>(), groups);
        for name in &groups {
            let exec = format!("Exec=kurobayomi --action={name}");
            assert!(DESKTOP.lines().any(|l| l == exec), "{name} has its Exec");
            assert!(argv::ACTIONS.contains(name), "{name} is a launch action");
            let args = [format!("--action={name}")];
            let parsed = argv::parse_argv(&args).unwrap();
            assert!(matches!(&parsed[..], [argv::LaunchRequest::Action { .. }]), "{name} parses");
        }
    }

    #[cfg(unix)]
    #[test]
    fn tc_53_37_the_mime_file_declares_the_backup_type_and_the_entry_lists_it() {
        assert_eq!(MIME, MIME_GOLDEN);
        if xmllint_available() {
            assert!(run("xmllint", &["--noout", &data("io.github.anas1412.Kurobayomi.xml")]));
        }
        let mime_types = DESKTOP.lines().find_map(|l| l.strip_prefix("MimeType=")).unwrap_or_default();
        for t in ["application/x-tachibk", "x-scheme-handler/kurobayomi"] {
            assert!(mime_types.split(';').any(|m| m == t), "{t}");
        }
        // Aniyomi's and Tachiyomi's links name repositories of Android extensions, which Kurobayomi cannot use
        for t in ["x-scheme-handler/aniyomi", "x-scheme-handler/tachiyomi"] {
            assert!(!mime_types.split(';').any(|m| m == t), "{t} is not claimed");
        }
    }

    #[test]
    fn the_icon_resolves_in_hicolor_at_48_128_and_256_pixels() {
        for size in [48u32, 128, 256] {
            let path = data(&format!("icons/hicolor/{size}x{size}/apps/io.github.anas1412.Kurobayomi.png"));
            let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            assert_eq!(&bytes[1..4], b"PNG");
            // the PNG header stores the width and height at bytes 16 to 24
            assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), size);
            assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), size);
        }
        assert!(Path::new(&data("icons/hicolor/scalable/apps/io.github.anas1412.Kurobayomi.svg")).exists());
    }

    #[test]
    fn the_post_install_hook_refreshes_the_desktop_and_mime_databases() {
        assert!(POST_INSTALL.contains("update-desktop-database"));
        assert!(POST_INSTALL.contains("update-mime-database"));
        assert!(POST_INSTALL.starts_with("#!/bin/sh"));
    }

    #[test]
    fn the_search_provider_file_is_what_the_provider_serves() {
        // the file packaging installs under the shell's search-providers folder, next to the desktop entry
        let shipped = include_str!("data/io.github.anas1412.Kurobayomi.search-provider.ini");
        assert_eq!(shipped, crate::shell::search_provider::ini());
    }
}
