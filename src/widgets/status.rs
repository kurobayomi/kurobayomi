//! The one builder of the loading, error and empty views of a page, on libadwaita's own widgets: a large turning spinner
//! while data loads, and an `adw::StatusPage` (icon, title, optional detail, buttons) for an error or an empty result. A
//! page never draws its own: it picks one of these for its `gtk::Stack` child.
//!
//! The spinner is GTK's own `gtk::Spinner`. It draws the `process-working-symbolic` icon, which the app's icon bundle
//! carries (`theme/icons`): without that icon the spinner drew nothing.

use adw::prelude::*;
use gtk::{Align, Orientation as O};
use crate::theme::tokens::{SPACE_M, SPACE_S};

/// The size of the spinner of a loading page, in px.
const SPINNER_PX: i32 = 48;

/// How long a page's state switcher fades between loading, content, empty and error.
pub const SWITCH_FADE_MS: u32 = 150;

/// How long a tab switch crossfades: what GTK's own stack crossfade takes by default, as the Anime and Manga switch of a
/// page does, so both switches feel the same.
pub const TAB_FADE_MS: u32 = 200;

/// `tabs` with its switches crossfaded as GTK's stack crossfade does (the Anime and Manga switch): over
/// [`TAB_FADE_MS`] the page being left fades out while the new one fades in. libadwaita's view stack has no transition
/// of its own before 1.7 and hides the page being left at once, so the last picture of that page, taken as the switch
/// happens (a frame later it is blank), lies over the new page and fades out. A switch during a fade ends that fade
/// first. libadwaita skips the fade when the desktop turns animations off. On libadwaita 1.7 or newer, delete this and
/// turn on the stack's own `enable_transitions` instead (it is off by default, so the two never both run; see
/// docs/UPGRADING-GTK.md).
pub fn crossfade_tabs(tabs: &adw::ViewStack) -> gtk::Overlay {
    let overlay = gtk::Overlay::builder().child(tabs).build();
    let leaving = gtk::Picture::builder().can_target(false).can_shrink(true).content_fit(gtk::ContentFit::Fill).visible(false).build();
    overlay.add_overlay(&leaving);
    let shown = std::cell::RefCell::new(tabs.visible_child().map(|page| gtk::WidgetPaintable::new(Some(&page))));
    let running: std::cell::RefCell<Option<adw::TimedAnimation>> = std::cell::RefCell::default();
    tabs.connect_visible_child_notify(move |tabs| {
        if let Some(fade) = running.take() {
            fade.skip();
        }
        let page = tabs.visible_child();
        let last = shown.replace(page.as_ref().map(|page| gtk::WidgetPaintable::new(Some(page)))).map(|left| left.current_image());
        let (Some(last), Some(page)) = (last, page) else { return };
        leaving.set_paintable(Some(&last));
        leaving.set_visible(true);
        // one value drives both: the old picture at 1 - v over the new page at v
        let step = {
            let (leaving, page) = (leaving.clone(), page.clone());
            move |v: f64| {
                leaving.set_opacity(1.0 - v);
                page.set_opacity(v);
            }
        };
        step(0.0);
        let fade = adw::TimedAnimation::builder()
            .widget(&leaving)
            .target(&adw::CallbackAnimationTarget::new(step.clone()))
            .value_from(0.0)
            .value_to(1.0)
            .duration(TAB_FADE_MS)
            .easing(adw::Easing::Linear)
            .build();
        let picture = leaving.clone();
        fade.connect_done(move |_| {
            step(1.0);
            picture.set_visible(false);
            picture.set_paintable(None::<&gtk::gdk::Paintable>);
        });
        fade.play();
        running.replace(Some(fade));
    });
    overlay
}

/// The switcher between a page's states (loading, content, empty, error): one crossfade of [`SWITCH_FADE_MS`] for every
/// page, so content never pops in. GTK drops the fade when the desktop turns animations off.
pub fn switcher() -> gtk::Stack {
    gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).transition_duration(SWITCH_FADE_MS).build()
}

/// The large turning spinner of a loading page, centred in the room it is given.
pub fn spinner() -> gtk::Spinner {
    let spinner = gtk::Spinner::builder().spinning(true).halign(Align::Center).valign(Align::Center).build();
    spinner.set_size_request(SPINNER_PX, SPINNER_PX);
    spinner
}

/// A loading page: a large spinning spinner centred in the room it is given, with `label` under it when there is one.
pub fn loading(label: Option<&str>) -> gtk::Widget {
    let spinner = spinner();
    let page = gtk::Box::new(O::Vertical, SPACE_M);
    page.set_halign(Align::Center);
    page.set_valign(Align::Center);
    page.set_hexpand(true);
    page.set_vexpand(true);
    page.append(&spinner);
    if let Some(label) = label {
        page.append(&gtk::Label::builder().label(label).css_classes(["dim-label"]).build());
    }
    page.upcast()
}

/// A status page with `icon` (none for a plain title), `title`, an optional `detail` line under it and `actions` as
/// pill buttons in a centred row. It fills its parent.
pub fn page(icon: Option<&str>, title: &str, detail: Option<&str>, actions: &[gtk::Button]) -> adw::StatusPage {
    let page = adw::StatusPage::builder().title(title).vexpand(true).hexpand(true).build();
    if let Some(icon) = icon {
        page.set_icon_name(Some(icon));
    }
    if let Some(detail) = detail.filter(|d| !d.is_empty()) {
        page.set_description(Some(detail));
    }
    if !actions.is_empty() {
        let row = gtk::Box::new(O::Horizontal, SPACE_S);
        row.set_halign(Align::Center);
        for action in actions {
            row.append(action);
        }
        page.set_child(Some(&row));
    }
    page
}

/// The empty page of a list that returned nothing.
pub fn empty(icon: &str, title: &str, detail: Option<&str>) -> adw::StatusPage {
    page(Some(icon), title, detail, &[])
}

/// An empty page with its own buttons (Add a category, Turn on a source).
pub fn empty_with(icon: &str, title: &str, detail: Option<&str>, actions: &[gtk::Button]) -> adw::StatusPage {
    page(Some(icon), title, detail, actions)
}

/// The error page of a load that failed: the error icon, `title`, the `detail` (the error text), and a Retry button when
/// `retry` is given.
pub fn error(title: &str, detail: Option<&str>, retry: Option<impl Fn() + 'static>) -> adw::StatusPage {
    let button = retry.map(|retry| {
        let button = gtk::Button::with_label(&kuro_i18n::tr(kuro_i18n::MsgKey::action_retry));
        button.connect_clicked(move |_| retry());
        button
    });
    error_with(title, detail, button.as_slice())
}

/// An error page with its own buttons (Retry and Help); the first is in the accent colour.
pub fn error_with(title: &str, detail: Option<&str>, actions: &[gtk::Button]) -> adw::StatusPage {
    if let Some(first) = actions.first() {
        first.add_css_class("suggested-action");
    }
    page(Some("dialog-error-symbolic"), title, detail, actions)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    fn buttons(root: &gtk::Widget, out: &mut Vec<gtk::Button>) {
        if let Some(b) = root.downcast_ref::<gtk::Button>() {
            out.push(b.clone());
        }
        let mut child = root.first_child();
        while let Some(w) = child {
            buttons(&w, out);
            child = w.next_sibling();
        }
    }

    #[test]
    fn loading_spins_and_labels_and_the_pages_carry_their_texts_and_one_retry() {
        crate::testing::with_display(|| {
            let spinning = |root: &gtk::Widget| {
                let mut stack = vec![root.clone()];
                while let Some(w) = stack.pop() {
                    if w.downcast_ref::<gtk::Spinner>().is_some_and(gtk::Spinner::is_spinning) {
                        return true;
                    }
                    let mut c = w.first_child();
                    while let Some(x) = c {
                        c = x.next_sibling();
                        stack.push(x);
                    }
                }
                false
            };
            assert!(spinning(&loading(None)) && spinning(&loading(Some("Loading"))));

            let empty = empty("view-list-symbolic", "Nothing here", Some("Try later"));
            assert_eq!((empty.title().as_str(), empty.description().as_deref()), ("Nothing here", Some("Try later")));
            assert!(empty.child().is_none(), "no buttons without actions");

            let retried = Rc::new(Cell::new(0));
            let count = Rc::clone(&retried);
            let error = error("Failed", Some("offline"), Some(move || count.set(count.get() + 1)));
            assert_eq!(error.icon_name().as_deref(), Some("dialog-error-symbolic"));
            let mut found = Vec::new();
            buttons(error.upcast_ref(), &mut found);
            assert_eq!(found.len(), 1);
            assert!(found[0].has_css_class("suggested-action"));
            found[0].emit_clicked();
            assert_eq!(retried.get(), 1);
        });
    }

    #[test]
    fn a_tab_switch_leaves_no_picture_of_the_old_page_behind() {
        crate::testing::with_display(|| {
            let tabs = adw::ViewStack::new();
            let (a, b) = (gtk::Label::new(Some("a")), gtk::Label::new(Some("b")));
            tabs.add_named(&a, Some("a"));
            tabs.add_named(&b, Some("b"));
            let overlay = crossfade_tabs(&tabs);
            for name in ["b", "a", "b"] {
                tabs.set_visible_child_name(name);
            }
            // off screen libadwaita ends a fade at once; on screen the next switch ends it (`skip`)
            let picture = overlay.last_child().and_downcast::<gtk::Picture>().expect("the fading picture lies over the tabs");
            assert!(!picture.is_visible() && picture.paintable().is_none());
            assert_eq!((a.opacity(), b.opacity()), (1.0, 1.0), "no page is left half faded");
            assert_eq!(overlay.child().as_ref(), Some(tabs.upcast_ref()));
        });
    }

    #[test]
    fn on_screen_the_old_page_fades_out_while_the_new_one_fades_in() {
        crate::testing::with_display(|| {
            if !gtk::Settings::default().is_some_and(|s| s.is_gtk_enable_animations()) {
                return;
            }
            let tabs = adw::ViewStack::new();
            let (a, b) = (gtk::Label::new(Some("first page")), gtk::Label::new(Some("second page")));
            tabs.add_named(&a, Some("a"));
            tabs.add_named(&b, Some("b"));
            let overlay = crossfade_tabs(&tabs);
            let window = gtk::Window::new();
            window.set_default_size(300, 200);
            window.set_child(Some(&overlay));
            window.present();
            let pump = |ms: u64| {
                let (ctx, end) = (gtk::glib::MainContext::default(), std::time::Instant::now() + std::time::Duration::from_millis(ms));
                while std::time::Instant::now() < end {
                    while ctx.iteration(false) {}
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            };
            pump(300);
            tabs.set_visible_child_name("b");
            pump(80);
            let picture = overlay.last_child().and_downcast::<gtk::Picture>().unwrap();
            assert!(picture.is_visible(), "the old page's picture lies over the new one");
            assert!(picture.paintable().is_some_and(|p| p.intrinsic_width() > 0), "the picture holds the old page, not a blank");
            let (old, new) = (picture.opacity(), b.opacity());
            assert!(old > 0.0 && old < 1.0 && new > 0.0 && new < 1.0 && (old + new - 1.0).abs() < 1e-6, "mid-fade: {old} and {new}");
            pump(300);
            assert!(!picture.is_visible() && b.opacity() == 1.0, "the fade ends on the new page");
            window.destroy();
        });
    }
}
