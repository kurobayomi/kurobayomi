//! The page error card (T-44a-064, T-44a-065, UI §1.1, B§8.5, E-42, E-84). A page that cannot be shown keeps this card
//! in place of its image: the message, Retry (which loads the page again) and, only for an http or https image url,
//! Open in browser. The rules are pure; [`card`] is the thin widget around them.

use kuro_i18n::{MsgKey, tr};
use gtk::prelude::*;

use super::strings;

/// The message of a failed page (`reader_error`).
pub fn page_error_text() -> String {
    tr(MsgKey::decode_image_error)
}

/// Height of the webtoon's error card as a share of the viewport (B§8.5).
pub const WEBTOON_CARD_RATIO: f64 = 0.8;

/// Whether an image url may be opened in the default browser. Only http and https (E-84). Any other scheme, an empty
/// url, a url without a host, and no url at all hide the button.
pub fn can_open_in_browser(url: Option<&str>) -> bool {
    let Some(url) = url.map(str::trim) else { return false };
    let lower = url.to_ascii_lowercase();
    let host = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"));
    host.is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/') && !rest.chars().any(char::is_whitespace))
}

/// The height of a webtoon error card: 0.8 times the viewport, never negative.
pub fn webtoon_card_height(viewport: i32) -> i32 {
    (f64::from(viewport.max(0)) * WEBTOON_CARD_RATIO) as i32
}

/// The error card for a page: the message, Retry, and Open in browser when [`can_open_in_browser`] allows it.
pub fn card(on_retry: impl Fn() + 'static, image_url: Option<String>) -> adw::StatusPage {
    let retry = gtk::Button::with_label(&strings::retry());
    retry.connect_clicked(move |_| on_retry());
    let mut buttons = vec![retry];
    if can_open_in_browser(image_url.as_deref()) {
        let open = gtk::Button::with_label(&strings::action_open_in_browser());
        let url = image_url.unwrap_or_default();
        open.connect_clicked(move |button| {
            let window = button.root().and_downcast::<gtk::Window>();
            crate::ui::open_link(&url, window.as_ref());
        });
        buttons.push(open);
    }
    crate::widgets::status::error_with(&page_error_text(), None, &buttons)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_and_https_urls_get_the_open_in_browser_button() {
        assert!(can_open_in_browser(Some("https://example.org/a.png")));
        assert!(can_open_in_browser(Some("HTTP://example.org/a.png")));
        assert!(!can_open_in_browser(Some("file:///home/a.png")));
        assert!(!can_open_in_browser(Some("ftp://example.org/a.png")));
        assert!(!can_open_in_browser(Some("javascript:alert(1)")));
        assert!(!can_open_in_browser(Some("https://")));
        assert!(!can_open_in_browser(Some("https:///path")));
        assert!(!can_open_in_browser(Some("https://exa mple.org")));
        assert!(!can_open_in_browser(None));
    }

    #[test]
    fn the_webtoon_card_is_four_fifths_of_the_viewport() {
        assert_eq!(webtoon_card_height(1000), 800);
        assert_eq!(webtoon_card_height(0), 0);
        assert_eq!(webtoon_card_height(-5), 0);
    }

    #[test]
    fn the_message_is_the_upstream_text() {
        assert_eq!(page_error_text(), "The image couldn't be loaded");
    }
}
