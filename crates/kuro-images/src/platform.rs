//! Where this crate meets the platform (area:34 design sections 3 and 10, T-34a-068).
//!
//! Every folder comes from `Paths`, and nothing in `kuro-images` names an operating system. Desktop notices,
//! opening a file and sharing go through `kuro_platform::notifier::Notifier` and `kuro_platform::opener`, which the
//! save and share code (T-34a-042 and later) calls; this crate never starts a program itself.

use std::path::PathBuf;

use kuro_platform::paths::Paths;

/// `<cache>/image_cache`, the disk cache of non-library images (behavior section 9).
pub fn image_cache_dir(paths: &dyn Paths) -> PathBuf {
    paths.image_cache_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestPaths;
    use std::path::Path;

    #[test]
    fn folders_are_under_the_cache_dir() {
        let paths = TestPaths::new(Path::new("/root"));
        assert_eq!(
            image_cache_dir(&paths),
            Path::new("/root/cache/image_cache")
        );
    }

    /// No operating-system conditionals and no direct process launching anywhere in the crate (addendum rules 1
    /// and 4). The patterns are split so this file does not match itself.
    #[test]
    fn no_platform_specifics_in_the_crate() {
        let forbidden = [
            concat!("cfg(target", "_os"),
            concat!("cfg(unix", ")"),
            concat!("cfg(windows", ")"),
            concat!("xdg", "-open"),
            concat!("std::process", "::Command"),
            concat!("XDG", "_"),
        ];
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for pattern in forbidden {
                        assert!(!text.contains(pattern), "{pattern} in {}", path.display());
                    }
                }
            }
        }
    }
}
