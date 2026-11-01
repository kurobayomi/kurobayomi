//! The welcome view of an empty library before any source repository exists (docs/design/app-flow.md, first run). One
//! focused screen in the app's empty-state style: the app icon, a title, one line, the one main action, and two quiet
//! links underneath, and in an AppImage a small line saying it updates itself. Everything else (storage folder, AniList) has a working default and lives in Settings.

use gtk::prelude::*;
use gtk::{Align, Justification, Orientation as O};
use kuro_i18n::{MsgKey, tr};

use crate::pref_model::router::Destination;
use crate::theme::icons::Icon;
use crate::theme::tokens::{SPACE_M, SPACE_XL, SPACE_XS};

/// The getting-started guide in the repository, opened by the last link.
const GUIDE_URL: &str = "https://github.com/kurobayomi/kurobayomi/blob/main/docs/GETTING-STARTED.md";

/// The welcome view.
#[derive(Clone)]
pub struct Welcome {
    page: gtk::Box,
}

impl Welcome {
    pub fn new() -> Self {
        let page = gtk::Box::new(O::Vertical, 0);
        page.set_valign(Align::Center);
        page.set_halign(Align::Center);
        page.set_vexpand(true);
        page.set_margin_start(SPACE_XL);
        page.set_margin_end(SPACE_XL);

        let logo = gtk::Image::builder().icon_name(Icon::LogoShadow.name()).pixel_size(128).margin_bottom(SPACE_XL).build();
        page.append(&logo);
        page.append(&label(&tr(MsgKey::welcome_title), &["title-1"], 8));
        page.append(&label(&tr(MsgKey::welcome_intro), &["dim-label"], 24));

        let add = gtk::Button::builder().label(tr(MsgKey::add_source_repository)).halign(Align::Center).build();
        add.add_css_class("suggested-action");
        add.add_css_class("pill");
        add.connect_clicked(|_| crate::browse::mount::open_repos_to_add());
        page.append(&add);

        let links = gtk::Box::new(O::Horizontal, SPACE_XS);
        links.set_halign(Align::Center);
        links.set_margin_top(SPACE_M);
        links.append(&link(&tr(MsgKey::restore_from_backup), || {
            crate::pref_model::highlight::request(crate::pref_model::strings::label_backup());
            crate::ui::open(Destination::DataAndStorage.screen_id());
        }));
        links.append(&link(&tr(MsgKey::welcome_guide), || crate::ui::open_link(GUIDE_URL, None)));
        page.append(&links);

        // an AppImage updates itself at start, so the first-run screen says so and where to turn it off
        if crate::appimage_update::can_update() {
            let notice = label(&tr(MsgKey::welcome_self_update), &["dim-label", "caption"], 0);
            notice.set_margin_top(SPACE_XL);
            page.append(&notice);
        }

        Self { page }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.page
    }
}

fn label(text: &str, classes: &[&str], below: i32) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .wrap(true)
        .max_width_chars(48)
        .justify(Justification::Center)
        .css_classes(classes)
        .margin_bottom(below)
        .build()
}

/// A flat button, the GTK way of a quiet secondary action.
fn link(text: &str, on_click: impl Fn() + 'static) -> gtk::Button {
    let button = gtk::Button::builder().label(text).build();
    button.add_css_class("flat");
    button.connect_clicked(move |_| on_click());
    button
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text of every label under `root`, in the order they are drawn (a button's text is its label child).
    fn texts(root: &gtk::Widget, out: &mut Vec<String>) {
        if let Some(label) = root.downcast_ref::<gtk::Label>() {
            out.push(label.label().to_string());
        }
        let mut child = root.first_child();
        while let Some(widget) = child {
            texts(&widget, out);
            child = widget.next_sibling();
        }
    }

    #[test]
    fn the_welcome_view_reads_title_line_the_main_action_then_two_links() {
        crate::testing::with_display(|| {
            let welcome = Welcome::new();
            let mut seen = Vec::new();
            texts(welcome.widget().upcast_ref(), &mut seen);
            assert_eq!(
                seen,
                [
                    "Welcome to Kurobayomi",
                    "Add a source repository to start watching anime and reading manga.",
                    "Add a source repository",
                    "Restore from a backup",
                    "Getting started guide",
                ]
            );
        });
    }
}
