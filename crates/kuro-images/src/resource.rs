//! `classify` for image resource strings.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    Http,
    File,
    Uri,
}

/// Classifies a cover or background string, checking in the Kotlin order. `None` means invalid: an
/// empty or missing string, or a string with no known prefix.
pub fn classify(value: Option<&str>) -> Option<Resource> {
    let value = value.filter(|v| !v.is_empty())?;
    if starts_with_ci(value, "http") {
        return Some(Resource::Http);
    }
    if starts_with_ci(value, "Custom-") {
        // No producer of this prefix exists; kept for parity with Kotlin (E-IMG-10).
        tracing::warn!(
            value,
            "Custom- image resource has no producer; treated as HTTP"
        );
        return Some(Resource::Http);
    }
    if std::path::Path::new(value).is_absolute() || value.starts_with("file://") {
        return Some(Resource::File);
    }
    if value.starts_with("content") {
        return Some(Resource::Uri);
    }
    None
}

fn starts_with_ci(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_missing_are_invalid() {
        assert_eq!(classify(None), None);
        assert_eq!(classify(Some("")), None);
    }

    #[test]
    fn http_prefix_is_case_insensitive() {
        assert_eq!(classify(Some("http://a/b.jpg")), Some(Resource::Http));
        assert_eq!(classify(Some("HTTPS://a/b.jpg")), Some(Resource::Http));
    }

    #[test]
    fn custom_prefix_is_http_for_parity() {
        assert_eq!(classify(Some("Custom-anything")), Some(Resource::Http));
        assert_eq!(classify(Some("custom-anything")), Some(Resource::Http));
    }

    #[test]
    fn an_absolute_path_and_the_file_scheme_are_files() {
        let cover = std::env::temp_dir().join("a").join("cover.jpg");
        assert_eq!(classify(cover.to_str()), Some(Resource::File));
        assert_eq!(classify(Some("file:///home/a.jpg")), Some(Resource::File));
        assert_eq!(classify(Some("a/cover.jpg")), None, "a relative path is not a file");
    }

    #[test]
    fn content_prefix_is_uri_and_case_matters() {
        assert_eq!(classify(Some("content://media/1")), Some(Resource::Uri));
        assert_eq!(classify(Some("Content://media/1")), None);
    }

    #[test]
    fn unknown_prefixes_are_invalid() {
        assert_eq!(classify(Some("ftp://host/x.jpg")), None);
        assert_eq!(classify(Some("relative/x.jpg")), None);
        assert_eq!(classify(Some("ht")), None);
    }
}
