//! `extension`, `stem` and `is_hidden` for file names.

/// Kotlin `UniFile.extension`: `substringAfterLast('.')`. A name without a dot is its own extension.
/// Not lowercased: callers apply their own case rule.
pub fn extension(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(_, ext)| ext)
}

/// Kotlin `UniFile.nameWithoutExtension`: `substringBeforeLast('.')`. A name without a dot is returned whole.
pub fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

/// Dot names are hidden, and every local listing skips them (behavior 4.3).
pub fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_takes_the_text_after_the_last_dot() {
        assert_eq!(extension("cover"), "cover");
        assert_eq!(extension(".nomedia"), "nomedia");
        assert_eq!(extension("a.b.cbz"), "cbz");
        assert_eq!(extension("EP01.MKV"), "MKV");
        assert_eq!(extension("dir."), "");
    }

    #[test]
    fn stem_takes_the_text_before_the_last_dot() {
        assert_eq!(stem(".nomedia"), "");
        assert_eq!(stem("a.b.cbz"), "a.b");
        assert_eq!(stem("cover"), "cover");
        assert_eq!(stem("dir."), "dir");
    }

    #[test]
    fn only_a_leading_dot_hides() {
        assert!(is_hidden(".noxml"));
        assert!(!is_hidden("a.b"));
        assert!(!is_hidden(""));
    }
}
