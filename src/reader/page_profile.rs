//! Display ICC profile for page colour (T-43a-067). The file named by `pref_display_profile_key` is read once when the
//! setting changes, and every page decoded after that is taken from sRGB to it. A missing or unreadable file gives no
//! transform and a logged warning, never an error on screen.
//!
//! The transform itself is `kuro_images::icc` (lcms2). That crate loads a profile only when it is built with its `icc`
//! feature, which the app build does not turn on yet: without it every profile reads as unusable, so pages keep their
//! decoded colours and the warning is logged. The loss is therefore: no display colour management until the feature is
//! on in `Cargo.toml`.

use std::sync::{Arc, Mutex};

use kuro_images::Decoded;
use kuro_images::icc::{DisplayProfile, load_setting};

use super::page_decode::DecodedPage;

/// The path that was loaded last and what it gave.
static CURRENT: Mutex<Option<(String, Option<Arc<DisplayProfile>>)>> = Mutex::new(None);

/// Takes the preference value. The file is read only when the value differs from the last one; an empty value turns the
/// transform off.
pub fn set_display_profile(path: &str) {
    let path = path.trim();
    let mut current = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    if current.as_ref().is_some_and(|(loaded, _)| loaded == path) {
        return;
    }
    let profile = load_setting(path);
    if profile.is_none() && !path.is_empty() {
        tracing::warn!(path, "the display profile could not be used; pages keep their colours");
    }
    *current = Some((path.to_owned(), profile));
}

/// The page taken to the display profile, or as it is when there is none.
pub fn apply(page: DecodedPage) -> DecodedPage {
    let profile = CURRENT.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|(_, p)| p.clone());
    let Some(profile) = profile else { return page };
    let DecodedPage { width, height, rgba } = page;
    let out = profile.apply(&Decoded { width, height, rgba: rgba.into(), is_sampled: false });
    DecodedPage { width: out.width, height: out.height, rgba: out.rgba.to_vec() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> DecodedPage {
        DecodedPage { width: 2, height: 1, rgba: vec![10, 20, 30, 255, 40, 50, 60, 128] }
    }

    #[test]
    fn no_profile_leaves_the_page_as_decoded() {
        set_display_profile("");
        assert_eq!(apply(page()), page());
    }
}
