//! Incognito indicator (T-01a-020): the banner precedence. The banner widget is GTK and lives in the GUI crate; this
//! module holds the parts that do not need a display.

/// The banner shown at the top of the main window. Only one is visible at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Banner {
    Indexing,
    DownloadedOnly,
    Incognito,
}

/// The banner that wins: indexing, then downloaded-only, then incognito (`MainActivity` order).
pub fn active_banner(indexing: bool, downloaded_only: bool, incognito: bool) -> Option<Banner> {
    if indexing {
        Some(Banner::Indexing)
    } else if downloaded_only {
        Some(Banner::DownloadedOnly)
    } else if incognito {
        Some(Banner::Incognito)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_precedence_is_indexing_then_downloaded_only_then_incognito() {
        assert_eq!(active_banner(true, true, true), Some(Banner::Indexing));
        assert_eq!(active_banner(false, true, true), Some(Banner::DownloadedOnly));
        assert_eq!(active_banner(false, false, true), Some(Banner::Incognito));
        assert_eq!(active_banner(false, false, false), None);
    }
}
