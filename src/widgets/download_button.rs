//! The one download button of the app: chapter and episode rows, Updates rows and the download queue all use it. It
//! draws the five download states and turns a click into a request that its owner runs. Not downloaded is the download
//! arrow, Queued a clock, Downloading a circle filling clockwise with the progress (drawn in the accent colour: the
//! toolkit has no determinate circular indicator), Downloaded a check, and Error the error icon, each with a tooltip.
//! A click starts (or retries). A click while queued or downloading opens a native popover menu with Pause (Resume while
//! the queue is paused) and Cancel, and a click on a downloaded item offers Delete in the same kind of menu. Pause and
//! Resume act on the whole queue of the kind (the download manager has no per-item pause); Cancel and Delete are the
//! owner's requests.

use std::cell::Cell;
use std::f64::consts::{FRAC_PI_2, TAU};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use kuro_downloads::DownloadState;
use kuro_i18n::{Arg, MsgKey, tr, trf};
use kuro_platform::kind::Kind;

use crate::theme::icons::Icon;

/// What a press asks the owner to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Start,
    Cancel,
    Delete,
}

/// What the button shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    NotDownloaded,
    Queued,
    /// The percent done, 0 to 100.
    Downloading(u8),
    Downloaded,
    Error,
}

/// The look of a download state and its percent (clamped to 100).
pub fn look(state: DownloadState, percent: u8) -> Look {
    match state {
        DownloadState::NotDownloaded => Look::NotDownloaded,
        DownloadState::Queue => Look::Queued,
        DownloadState::Downloading => Look::Downloading(percent.min(100)),
        DownloadState::Downloaded => Look::Downloaded,
        DownloadState::Error => Look::Error,
    }
}

/// One item of the popover menu a click opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Pause,
    Resume,
    Cancel,
    Delete,
}

impl Choice {
    fn label(self) -> MsgKey {
        match self {
            Choice::Pause => MsgKey::action_pause,
            Choice::Resume => MsgKey::action_resume,
            Choice::Cancel => MsgKey::action_cancel,
            Choice::Delete => MsgKey::action_delete,
        }
    }
}

/// The menu a click on `look` opens: Pause (Resume while the queue of the kind is paused) and Cancel for an item that is
/// queued or downloading, Delete for a downloaded one. No menu means the click acts at once: Start, or a retry.
pub fn menu(look: Look, running: bool) -> Vec<Choice> {
    match look {
        Look::NotDownloaded | Look::Error => Vec::new(),
        Look::Queued | Look::Downloading(_) => vec![if running { Choice::Pause } else { Choice::Resume }, Choice::Cancel],
        Look::Downloaded => vec![Choice::Delete],
    }
}

/// The filled part of the circle for `percent`: cairo's start and end angles in radians, clockwise from twelve o'clock.
pub fn arc(percent: u8) -> (f64, f64) {
    let start = -FRAC_PI_2;
    (start, start + TAU * f64::from(percent.min(100)) / 100.0)
}

/// The icon of the looks that draw one; the downloading look draws the progress circle.
fn icon_name(look: Look) -> Option<&'static str> {
    match look {
        Look::NotDownloaded => Some(Icon::DownloadCircle.name()),
        Look::Queued => Some("document-open-recent-symbolic"),
        Look::Downloaded => Some("object-select-symbolic"),
        Look::Error => Some("dialog-error-symbolic"),
        Look::Downloading(_) => None,
    }
}

/// The tooltip and accessible label: the state, with the percent while downloading.
fn tooltip(look: Look) -> String {
    match look {
        Look::NotDownloaded => tr(MsgKey::action_download),
        Look::Queued => tr(MsgKey::download_queued),
        Look::Downloading(percent) => trf(MsgKey::download_downloading, &[("arg1", Arg::Int(i64::from(percent)))]),
        Look::Downloaded => tr(MsgKey::label_downloaded),
        Look::Error => tr(MsgKey::download_failed_retry),
    }
}

/// The action group of the popover menu on the button.
const GROUP: &str = "dl";

/// The side of the drawn circle in px, the size of a symbolic icon in a flat button.
const SIZE: i32 = 18;

/// libadwaita's accent colour from the style context, the text colour when it has none.
#[allow(deprecated)]
fn accent(area: &gtk::DrawingArea) -> gdk::RGBA {
    area.style_context().lookup_color("accent_color").unwrap_or_else(|| area.color())
}

/// The hollow circle, and for a download in progress the wedge of its percent inside it. The panel's Downloads item draws
/// the same circle (`shell::sidebar`).
pub(crate) fn draw(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, width: i32, height: i32, look: Look) {
    let color = accent(area);
    cr.set_source_rgba(f64::from(color.red()), f64::from(color.green()), f64::from(color.blue()), f64::from(color.alpha()));
    let (cx, cy) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
    let radius = f64::from(width.min(height)) / 2.0 - 1.5;
    cr.set_line_width(1.5);
    cr.arc(cx, cy, radius, 0.0, TAU);
    let _ = cr.stroke();
    if let Look::Downloading(percent) = look
        && percent > 0
    {
        let (start, end) = arc(percent);
        cr.move_to(cx, cy);
        cr.arc(cx, cy, radius - 2.5, start, end);
        cr.close_path();
        let _ = cr.fill();
    }
}

/// The popover menu of `choices`, a native `gtk::PopoverMenu` over a `gio::Menu` of actions on the button. `run` gets the
/// picked choice once the menu has closed.
fn open_menu(button: &gtk::Button, choices: &[Choice], run: Rc<dyn Fn(Choice)>) {
    let (model, group) = (gio::Menu::new(), gio::SimpleActionGroup::new());
    for (n, choice) in choices.iter().copied().enumerate() {
        let name = format!("item{n}");
        let action = gio::SimpleAction::new(&name, None);
        let run = Rc::clone(&run);
        action.connect_activate(move |_, _| run(choice));
        group.add_action(&action);
        model.append(Some(&tr(choice.label())), Some(&format!("{GROUP}.{name}")));
    }
    button.insert_action_group(GROUP, Some(&group));
    let popover = gtk::PopoverMenu::from_model(Some(&model));
    popover.set_parent(button);
    // the popover is parented to a button that a list may recycle, so it goes once closed
    popover.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    popover.popup();
}

/// The button. Cloning shares the same widgets.
#[derive(Clone)]
pub struct DownloadButton {
    pub widget: gtk::Button,
    stack: gtk::Stack,
    icon: gtk::Image,
    ring: gtk::DrawingArea,
    look: Rc<Cell<Look>>,
}

impl DownloadButton {
    /// A button showing Not downloaded, for an item of `kind`. `on_request` runs a click's request.
    pub fn new(kind: Kind, on_request: impl Fn(Request) + 'static) -> Self {
        let icon = gtk::Image::new();
        let ring = gtk::DrawingArea::builder().content_width(SIZE).content_height(SIZE).halign(gtk::Align::Center).valign(gtk::Align::Center).build();
        let stack = gtk::Stack::new();
        stack.add_named(&icon, Some("icon"));
        stack.add_named(&ring, Some("ring"));
        let widget = gtk::Button::builder().child(&stack).css_classes(["flat", "circular"]).focus_on_click(false).valign(gtk::Align::Center).build();
        let look = Rc::new(Cell::new(Look::NotDownloaded));
        ring.set_draw_func({
            let look = Rc::clone(&look);
            move |area, cr, width, height| draw(area, cr, width, height, look.get())
        });
        let on_request: Rc<dyn Fn(Request)> = Rc::new(on_request);
        widget.connect_clicked({
            let look = Rc::clone(&look);
            move |button| {
                let choices = menu(look.get(), crate::downloads::live::lane_running(kind));
                if choices.is_empty() {
                    return on_request(Request::Start);
                }
                let on_request = Rc::clone(&on_request);
                open_menu(
                    button,
                    &choices,
                    Rc::new(move |choice| match choice {
                        Choice::Pause => crate::downloads::live::pause(kind),
                        Choice::Resume => crate::downloads::live::resume(kind),
                        Choice::Cancel => on_request(Request::Cancel),
                        Choice::Delete => on_request(Request::Delete),
                    }),
                );
            }
        });
        let this = Self { widget, stack, icon, ring, look };
        this.show(Look::NotDownloaded);
        this
    }

    /// Shows a download state and its percent.
    pub fn set(&self, state: DownloadState, percent: u8) {
        let next = look(state, percent);
        if self.look.replace(next) != next {
            self.show(next);
        }
    }

    fn show(&self, look: Look) {
        match icon_name(look) {
            Some(name) => {
                self.icon.set_icon_name(Some(name));
                // the failed state is the error icon in the error colour
                if look == Look::Error {
                    self.icon.add_css_class("error");
                } else {
                    self.icon.remove_css_class("error");
                }
                self.stack.set_visible_child_name("icon");
            }
            None => {
                self.stack.set_visible_child_name("ring");
                self.ring.queue_draw();
            }
        }
        let text = tooltip(look);
        self.widget.set_tooltip_text(Some(&text));
        self.widget.update_property(&[gtk::accessible::Property::Label(&text)]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_download_state_maps_to_its_look() {
        assert_eq!(look(DownloadState::NotDownloaded, 0), Look::NotDownloaded);
        assert_eq!(look(DownloadState::Queue, 30), Look::Queued, "a queued item shows no progress");
        assert_eq!(look(DownloadState::Downloading, 40), Look::Downloading(40));
        assert_eq!(look(DownloadState::Downloading, 250), Look::Downloading(100), "the percent is clamped");
        assert_eq!(look(DownloadState::Downloaded, 0), Look::Downloaded);
        assert_eq!(look(DownloadState::Error, 0), Look::Error);
    }

    #[test]
    fn a_click_starts_or_retries_at_once_and_otherwise_opens_a_menu() {
        assert!(menu(Look::NotDownloaded, true).is_empty(), "Start has no menu");
        assert!(menu(Look::Error, true).is_empty(), "an error retries at once");
        assert_eq!(menu(Look::Queued, true), [Choice::Pause, Choice::Cancel]);
        assert_eq!(menu(Look::Downloading(60), true), [Choice::Pause, Choice::Cancel]);
        assert_eq!(menu(Look::Downloading(60), false), [Choice::Resume, Choice::Cancel], "a paused queue offers Resume");
        assert_eq!(menu(Look::Downloaded, true), [Choice::Delete]);
    }

    #[test]
    fn the_arc_starts_at_twelve_and_turns_clockwise_with_the_percent() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let (start, end) = arc(0);
        assert!(close(start, -FRAC_PI_2) && close(end, -FRAC_PI_2), "nothing filled at 0 %");
        assert!(close(arc(25).1, 0.0), "a quarter ends at three o'clock");
        assert!(close(arc(50).1, FRAC_PI_2), "half ends at six o'clock");
        assert!(close(arc(100).1, -FRAC_PI_2 + TAU), "a full turn at 100 %");
        assert!(close(arc(200).1, arc(100).1), "past 100 % is a full turn");
    }

    #[test]
    fn the_button_shows_the_state_it_is_given_with_a_tooltip_for_each() {
        crate::testing::with_display(|| {
            let button = DownloadButton::new(Kind::Manga, |_| {});
            let shows = |state, percent, icon: Option<&str>, tip: &str| {
                button.set(state, percent);
                assert_eq!(button.icon.icon_name().as_deref().filter(|_| icon.is_some()), icon);
                assert_eq!(button.stack.visible_child_name().as_deref(), Some(if icon.is_some() { "icon" } else { "ring" }));
                assert_eq!(button.widget.tooltip_text().as_deref(), Some(tip));
            };
            assert_eq!(button.stack.visible_child_name().as_deref(), Some("icon"));
            shows(DownloadState::NotDownloaded, 0, Some(Icon::DownloadCircle.name()), "Download");
            shows(DownloadState::Queue, 0, Some("document-open-recent-symbolic"), "Queued");
            shows(DownloadState::Downloading, 40, None, "Downloading (40%)");
            shows(DownloadState::Downloaded, 0, Some("object-select-symbolic"), "Downloaded");
            shows(DownloadState::Error, 0, Some("dialog-error-symbolic"), "Download failed. Click to retry.");
            assert!(button.icon.has_css_class("error"), "the failed state is red");
            button.set(DownloadState::Downloaded, 0);
            assert!(!button.icon.has_css_class("error"));
        });
    }

    /// A click while queued opens the native menu with Pause and Cancel, and Cancel reaches the owner; a click on a
    /// not-downloaded button starts at once.
    #[test]
    fn a_click_on_a_queued_item_opens_the_pause_and_cancel_menu() {
        crate::testing::with_display(|| {
            let requests: Rc<std::cell::RefCell<Vec<Request>>> = Rc::default();
            let button = DownloadButton::new(Kind::Anime, {
                let requests = Rc::clone(&requests);
                move |request| requests.borrow_mut().push(request)
            });
            let window = gtk::Window::builder().child(&button.widget).build();
            window.present();
            button.widget.emit_clicked();
            assert_eq!(*requests.borrow(), [Request::Start], "not downloaded: no menu");
            button.set(DownloadState::Queue, 0);
            button.widget.emit_clicked();
            let popover = button.widget.last_child().and_downcast::<gtk::PopoverMenu>().expect("the menu is on the button");
            let model = popover.menu_model().expect("a menu model");
            let labels: Vec<String> = (0..model.n_items()).filter_map(|i| model.item_attribute_value(i, "label", None)).filter_map(|v| v.get::<String>()).collect();
            assert_eq!(labels, ["Pause", "Cancel"]);
            assert_eq!(*requests.borrow(), [Request::Start], "opening the menu asks for nothing");
            button.widget.activate_action("dl.item1", None).expect("the Cancel item");
            assert_eq!(*requests.borrow(), [Request::Start, Request::Cancel]);
            window.destroy();
        });
    }
}
