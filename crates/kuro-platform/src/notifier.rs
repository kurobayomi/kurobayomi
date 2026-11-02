//! The desktop notification boundary (CRATE-MAP C-04). The app implements [`Notifier`] over `gio::Notification`; library
//! code only calls it, so it never needs GTK.

use std::path::PathBuf;
use std::time::Duration;

use crate::kind::Kind;

/// Shows and withdraws desktop notices by id. A notice shown with an id that is already shown replaces it.
pub trait Notifier: Send + Sync {
    /// `body` is `None` when the notice must not show its content.
    fn show(&self, id: &str, title: &str, body: Option<&str>);
    /// Shows a notice with its click, buttons, priority and timeout. The default sends the title and the body through
    /// [`Notifier::show`]; the app's notifier draws all of it.
    fn notify(&self, notice: &Notice) {
        self.show(&notice.id, &notice.title, notice.body.as_deref());
    }
    /// Removes the notice with this id. A missing id is not an error.
    fn withdraw(&self, id: &str);
}

/// What a click on a notice, or on one of its buttons, does. The app maps each to its own action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tap {
    /// Shows the file in the file manager.
    Reveal(PathBuf),
    OpenUrl(String),
    /// The downloads page of a kind.
    Downloads(Kind),
    /// An application action by its detailed name (`app.show-library`).
    App(String),
}

/// A desktop notice with everything the desktop can show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Notice {
    /// Replaces an earlier notice with the same id.
    pub id: String,
    pub title: String,
    /// `None` when the notice must not show its content.
    pub body: Option<String>,
    /// No sound and no banner where the desktop supports it.
    pub low_priority: bool,
    pub tap: Option<Tap>,
    /// `(label, click)` in order.
    pub buttons: Vec<(String, Tap)>,
    /// Withdrawn after this long.
    pub timeout: Option<Duration>,
}

/// The app name that replaces a hidden notice's title.
pub const APP_NAME: &str = "Kurobayomi";

/// Sends a notice through `notifier`, applying `hide_notification_content` (T-01a-034).
///
/// When `hide_content` is set and the notice is `sensitive` (library-update, download and similar notices), the
/// title keeps only `app_name` and the body is replaced by `generic`, the category's generic text. Other notices
/// are sent unchanged, so crash notices pass `sensitive = false`.
#[allow(clippy::too_many_arguments)]
pub fn send(
    notifier: &dyn Notifier,
    hide_content: bool,
    sensitive: bool,
    id: &str,
    app_name: &str,
    title: &str,
    body: &str,
    generic: &str,
) {
    if hide_content && sensitive {
        notifier.show(id, app_name, Some(generic));
    } else {
        notifier.show(id, title, Some(body));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Records every `show` call.
    #[derive(Default)]
    struct Fake {
        shown: Mutex<Vec<(String, String, Option<String>)>>,
    }

    impl Notifier for Fake {
        fn show(&self, id: &str, title: &str, body: Option<&str>) {
            self.shown.lock().unwrap().push((id.into(), title.into(), body.map(str::to_owned)));
        }
        fn withdraw(&self, _id: &str) {}
    }

    fn one(hide: bool, sensitive: bool) -> (String, Option<String>) {
        let fake = Fake::default();
        send(&fake, hide, sensitive, "n", "Kurobayomi", "Naruto 12", "3 new chapters", "New chapters are available");
        let shown = fake.shown.lock().unwrap();
        assert_eq!(shown.len(), 1);
        (shown[0].1.clone(), shown[0].2.clone())
    }

    #[test]
    fn a_sensitive_notice_is_replaced_when_hiding_is_on() {
        assert_eq!(one(true, true), ("Kurobayomi".into(), Some("New chapters are available".into())));
    }

    #[test]
    fn hiding_leaves_insensitive_notices_alone() {
        assert_eq!(one(true, false), ("Naruto 12".into(), Some("3 new chapters".into())));
    }

    #[test]
    fn the_content_is_shown_when_hiding_is_off() {
        assert_eq!(one(false, true), ("Naruto 12".into(), Some("3 new chapters".into())));
    }
}
