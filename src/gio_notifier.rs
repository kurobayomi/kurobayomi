//! `kuro_platform::notifier::Notifier` for the library crates (downloads, backup and restore). It hands every
//! notice to the app's one notifier (`misc::notify`) as a [`Post`], so these notices pass the same mute check, content
//! rules and pacing as all the others, and keep their clicks and buttons.

use kuro_platform::kind::Kind;
use kuro_platform::notifier::{Notice, Notifier, Tap};

use crate::misc::notify::{self, Post, Target};
use crate::misc::router::AppAction;

#[derive(Clone, Copy)]
pub struct GioNotifier;

impl GioNotifier {
    /// Call from the GTK main thread.
    pub fn new(_app: &gtk::Application) -> Self {
        notify::attach();
        Self
    }
}

/// The app's action for a click. A revealed file's folder joins the folders a reveal may show, since the app wrote it.
fn target(tap: &Tap) -> Target {
    match tap {
        Tap::Reveal(path) => {
            crate::misc::reveal::allow_folder_of(path);
            Target::Action(AppAction::RevealFile { path: path.clone() })
        }
        Tap::OpenUrl(url) => Target::Url(url.clone()),
        Tap::Downloads(Kind::Manga) => Target::Action(AppAction::OpenMangaDownloads),
        Tap::Downloads(Kind::Anime) => Target::Action(AppAction::OpenAnimeDownloads),
        Tap::App(action) => Target::Named { action: action.clone(), target: None },
    }
}

/// The post for a platform notice.
fn post(notice: &Notice) -> Post {
    Post {
        id: notice.id.clone(),
        title: notice.title.clone(),
        body: notice.body.clone(),
        low_priority: notice.low_priority,
        timeout: notice.timeout,
        tap: notice.tap.as_ref().map(target),
        buttons: notice.buttons.iter().map(|(label, tap)| (label.clone(), target(tap))).collect(),
        icon: None,
    }
}

impl Notifier for GioNotifier {
    fn show(&self, id: &str, title: &str, body: Option<&str>) {
        self.notify(&Notice { id: id.into(), title: title.into(), body: body.map(str::to_owned), ..Notice::default() });
    }

    fn notify(&self, notice: &Notice) {
        notify::post(post(notice));
    }

    fn withdraw(&self, id: &str) {
        notify::withdraw(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn a_platform_notice_keeps_its_clicks_buttons_priority_and_timeout() {
        let log = PathBuf::from("/home/u/backups/a.tachibk");
        let notice = Notice {
            id: "kurobayomi-backup-result".into(),
            title: "Backup created".into(),
            body: None,
            low_priority: true,
            tap: Some(Tap::App("app.show-library".into())),
            buttons: vec![("Open folder".into(), Tap::Reveal(log.clone())), ("Show anime".into(), Tap::Downloads(Kind::Anime))],
            timeout: Some(Duration::from_secs(30)),
        };
        let post = post(&notice);
        assert_eq!(post.tap, Some(Target::Named { action: "app.show-library".into(), target: None }));
        assert_eq!(post.buttons[0].1, Target::Action(AppAction::RevealFile { path: log }));
        assert_eq!(post.buttons[1].1, Target::Action(AppAction::OpenAnimeDownloads));
        assert!(post.low_priority && post.body.is_none());
        assert_eq!(post.timeout, Some(Duration::from_secs(30)));
        assert!(crate::misc::reveal::announced_folders().contains(&PathBuf::from("/home/u/backups")), "the folder may be revealed");
    }
}
