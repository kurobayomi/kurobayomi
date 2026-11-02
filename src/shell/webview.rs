//! WebView screen chrome (Kotlin `WebViewScreenContent`, T-36a-049): the header with close, title and URL, Back and Forward,
//! the overflow menu in its fixed order, and the load progress bar.
//!
//! The rules are pure. The page engine (WebKitGTK, T-36a-052) reports `can_go_back`, `can_go_forward`, the page title and
//! the load state; this module turns them into what the header shows.


#[cfg(test)]
use kuro_i18n::{MsgKey, tr};
#[cfg(test)]
use kuro_platform::kind::Kind;

#[cfg(test)]
/// The title: the page title when the page has one, otherwise the title the screen was opened with.
pub fn title(page_title: Option<&str>, initial_title: &str) -> String {
    page_title.unwrap_or(initial_title).to_string()
}

#[cfg(test)]
/// What the screen is opened with (Kotlin extras `url_key`, `source_key`, `title_key`, `anime_key`, T-53a-071).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRequest {
    pub url: String,
    pub source_id: Option<i64>,
    /// `title_key`: the title shown until the page has one.
    pub title_key: Option<String>,
    pub kind: Kind,
}

#[cfg(test)]
impl OpenRequest {
    #[cfg(test)]
    /// The title the header starts with: the `title_key` value, or none.
    pub fn initial_title(&self) -> &str {
        self.title_key.as_deref().unwrap_or("")
    }
}

#[cfg(test)]
/// What the Back action does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackStep {
    /// Go back one page in the history.
    History,
    /// The history is at its start: close the screen.
    Close,
}

#[cfg(test)]
/// Back goes back in history while the page can, and closes the screen otherwise.
pub fn back_step(can_go_back: bool) -> BackStep {
    if can_go_back { BackStep::History } else { BackStep::Close }
}

#[cfg(test)]
/// The label of the Back action.
pub fn back_label() -> String {
    tr(MsgKey::action_webview_back)
}

#[cfg(test)]
/// The overflow actions, in the order the menu lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overflow {
    Refresh,
    Share,
    OpenInBrowser,
    ClearCookies,
}

#[cfg(test)]
/// Overflow menu order (`WebViewScreenContent`).
pub const OVERFLOW: [Overflow; 4] = [Overflow::Refresh, Overflow::Share, Overflow::OpenInBrowser, Overflow::ClearCookies];

#[cfg(test)]
impl Overflow {
    #[cfg(test)]
    pub fn label(self) -> String {
        tr(match self {
            Self::Refresh => MsgKey::action_webview_refresh,
            Self::Share => MsgKey::action_share,
            Self::OpenInBrowser => MsgKey::action_open_in_browser,
            Self::ClearCookies => MsgKey::pref_clear_cookies,
        })
    }
}

#[cfg(test)]
/// The page engine's load state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoadState {
    Idle,
    /// The engine is starting; no progress is known.
    Initialising,
    /// A page is loading. `None` when the engine reports no progress.
    Loading(Option<f64>),
}

#[cfg(test)]
/// What the progress bar shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    Hidden,
    Indeterminate,
    Fraction(f64),
}

#[cfg(test)]
/// Indeterminate while initialising, determinate while loading (a missing progress counts as complete), hidden when idle.
pub fn progress(state: LoadState) -> Progress {
    match state {
        LoadState::Idle => Progress::Hidden,
        LoadState::Initialising => Progress::Indeterminate,
        LoadState::Loading(fraction) => Progress::Fraction(fraction.unwrap_or(1.0).clamp(0.0, 1.0)),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_initial_title_is_the_title_key_value() {
        let request = OpenRequest { url: "https://x.test".into(), source_id: Some(3), title_key: Some("Source".into()), kind: Kind::Manga };
        assert_eq!(request.initial_title(), "Source");
        assert_eq!(title(None, request.initial_title()), "Source");
        assert_eq!(OpenRequest { title_key: None, ..request }.initial_title(), "");
    }

    #[test]
    fn back_goes_through_history_and_closes_at_its_start() {
        assert_eq!(back_step(true), BackStep::History);
        assert_eq!(back_step(false), BackStep::Close);
        assert_eq!(back_label(), "Back");
    }

    #[test]
    fn the_page_title_wins_over_the_initial_title() {
        assert_eq!(title(Some("Chapter list"), "Source"), "Chapter list");
        assert_eq!(title(None, "Source"), "Source");
    }

    #[test]
    fn progress_is_indeterminate_while_initialising_and_hidden_when_idle() {
        assert_eq!(progress(LoadState::Initialising), Progress::Indeterminate);
        assert_eq!(progress(LoadState::Idle), Progress::Hidden);
    }

    #[test]
    fn a_loading_page_without_progress_reads_as_complete() {
        assert_eq!(progress(LoadState::Loading(None)), Progress::Fraction(1.0));
        assert_eq!(progress(LoadState::Loading(Some(0.4))), Progress::Fraction(0.4));
        assert_eq!(progress(LoadState::Loading(Some(3.0))), Progress::Fraction(1.0));
    }

    #[test]
    fn overflow_lists_refresh_share_open_and_clear_cookies_in_that_order() {
        assert_eq!(OVERFLOW, [Overflow::Refresh, Overflow::Share, Overflow::OpenInBrowser, Overflow::ClearCookies]);
        assert_eq!(Overflow::Refresh.label(), tr(MsgKey::action_webview_refresh));
        assert_eq!(Overflow::ClearCookies.label(), tr(MsgKey::pref_clear_cookies));
    }
}
