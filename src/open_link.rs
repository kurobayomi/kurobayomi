//! The eleven `app.open-*` actions (T-01a-042). They back the second-launch command (`--open updates`), the
//! `.desktop` Actions entries and notification default actions, and all go through [`go`].

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use kuro_platform::kind::Kind;
use kuro_platform::deep_link::{DeepLink, NAMES};

use crate::{screens, ui};

#[derive(Debug, PartialEq)]
enum Dest {
    /// A nav tab or a pushed screen of the spec, by id.
    Open(&'static str),
    /// Library tab, then the entry's page, opened by its id.
    Entry { kind: Kind, id: i64 },
}

fn dest(link: &DeepLink) -> Dest {
    match link {
        DeepLink::AnimeLibrary | DeepLink::Library => Dest::Open("library"),
        DeepLink::Manga(id) => Dest::Entry { kind: Kind::Manga, id: *id },
        DeepLink::Anime(id) => Dest::Entry { kind: Kind::Anime, id: *id },
        DeepLink::Updates => Dest::Open("updates"),
        DeepLink::History => Dest::Open("history"),
        DeepLink::Sources | DeepLink::AnimeExtensions | DeepLink::Extensions => Dest::Open("browse"),
        DeepLink::AnimeDownloads | DeepLink::Downloads => Dest::Open("download_queue"),
    }
}

/// Opens the destination over the root, in one slide (`ui::over_root`). Called on the GTK thread.
fn go(win: &gtk::ApplicationWindow, link: &DeepLink) {
    win.present();
    ui::over_root(|| match dest(link) {
        Dest::Open(id) => ui::open(id),
        Dest::Entry { kind, id } => {
            ui::open("library");
            crate::library::mount::show_kind(kind);
            screens::open_entry(kind, id);
        }
    });
}

/// Adds `open-<name>` for every deep link to the application. `manga` and `anime` take the entry id as an int64.
pub fn register(app: &gtk::Application, win: &gtk::ApplicationWindow) {
    for name in NAMES {
        let with_id = matches!(name, "manga" | "anime");
        let action = gio::SimpleAction::new(&format!("open-{name}"), with_id.then_some(glib::VariantTy::INT64));
        let win = win.downgrade();
        action.connect_activate(move |_, param| {
            let arg = match param.and_then(|p| p.get::<i64>()) {
                Some(id) => format!("{name}={id}"),
                None => name.to_owned(),
            };
            if let (Some(link), Some(win)) = (DeepLink::parse(&arg), win.upgrade()) {
                go(&win, &link);
            }
        });
        app.add_action(&action);
    }
}

/// Runs a command line forwarded by a second launch: a deep link activates its action, anything else (a plain
/// second launch) raises the window.
pub fn dispatch(app: &gtk::Application, win: &gtk::ApplicationWindow, command: &str) {
    let Some(link) = DeepLink::parse(command) else {
        win.present();
        return;
    };
    let id = match link {
        DeepLink::Manga(id) | DeepLink::Anime(id) => Some(id.to_variant()),
        _ => None,
    };
    app.activate_action(link.action_name().trim_start_matches("app."), id.as_ref());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_link_has_a_destination() {
        assert_eq!(dest(&DeepLink::Updates), Dest::Open("updates"));
        assert_eq!(dest(&DeepLink::Manga(7)), Dest::Entry { kind: Kind::Manga, id: 7 });
        assert_eq!(dest(&DeepLink::Anime(3)), Dest::Entry { kind: Kind::Anime, id: 3 });
        for name in NAMES {
            let link = DeepLink::parse(name).or_else(|| DeepLink::parse(&format!("{name}=1"))).unwrap();
            if let Dest::Open(id) = dest(&link) {
                assert!(["library", "updates", "history", "browse", "download_queue"].contains(&id), "{name}");
            }
        }
    }

    #[test]
    fn the_nav_targets_exist_in_the_spec() {
        crate::ui::with_spec(|s| {
            for id in ["home", "library", "browse"] {
                assert!(s.nav.iter().any(|n| n.id == id), "{id}");
            }
            // Updates and History are pages Home opens (docs/design/home.md): a link pushes them
            for id in ["updates", "history"] {
                assert!(s.screen(id).is_some() && !s.nav.iter().any(|n| n.id == id), "{id}");
            }
            assert!(s.screen("download_queue").is_some());
        });
    }
}
