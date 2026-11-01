//! The one download button of the app: chapter and episode rows, Updates rows and the download queue all use it. It
//! draws the five download states and turns a click into a request that its owner runs. Not downloaded is the download
//! arrow, Queued a hollow circle, Downloading the same circle filling clockwise with the progress (drawn in the theme
//! accent), Downloaded a check, and Error a retry arrow. A click starts (or retries), a click while queued or
//! downloading cancels at once, and a click on a downloaded item offers Delete in a small popover.

use std::cell::Cell;
use std::f64::consts::{FRAC_PI_2, TAU};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use kuro_downloads::DownloadState;
use kuro_i18n::{MsgKey, tr};

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

/// What a click asks for. `Delete` is asked only after the popover's Delete button.
pub fn on_click(look: Look) -> Request {
    match look {
        Look::NotDownloaded | Look::Error => Request::Start,
        Look::Queued | Look::Downloading(_) => Request::Cancel,
        Look::Downloaded => Request::Delete,
    }
}

/// The filled part of the circle for `percent`: cairo's start and end angles in radians, clockwise from twelve o'clock.
pub fn arc(percent: u8) -> (f64, f64) {
    let start = -FRAC_PI_2;
    (start, start + TAU * f64::from(percent.min(100)) / 100.0)
}

/// The icon of the looks that draw one; the queued and downloading looks draw the circle.
fn icon_name(look: Look) -> Option<&'static str> {
    match look {
        Look::NotDownloaded => Some(Icon::DownloadCircle.name()),
        Look::Downloaded => Some("object-select-symbolic"),
        Look::Error => Some("view-refresh-symbolic"),
        Look::Queued | Look::Downloading(_) => None,
    }
}

/// The tooltip and accessible label: what a click does, with the percent while downloading.
fn tooltip(look: Look) -> String {
    match look {
        Look::NotDownloaded => tr(MsgKey::action_download),
        Look::Queued => tr(MsgKey::action_cancel),
        Look::Downloading(percent) => format!("{} ({percent}%)", tr(MsgKey::action_cancel)),
        Look::Downloaded => tr(MsgKey::label_downloaded),
        Look::Error => tr(MsgKey::action_retry),
    }
}

/// The side of the drawn circle in px, the size of a symbolic icon in a flat button.
const SIZE: i32 = 18;

/// The theme accent from the style context: the app palette's primary, then the GTK accent, then the selection colour.
#[allow(deprecated)]
fn accent(area: &gtk::DrawingArea) -> gdk::RGBA {
    let context = area.style_context();
    ["kuro_primary", "accent_color", "theme_selected_bg_color"].iter().find_map(|name| context.lookup_color(name)).unwrap_or_else(|| area.color())
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

/// The small popover of a downloaded item: one Delete button.
fn offer_delete(button: &gtk::Button, on_request: Rc<dyn Fn(Request)>) {
    let popover = gtk::Popover::new();
    let delete = gtk::Button::builder().label(tr(MsgKey::action_delete)).css_classes(["flat", "destructive-action"]).build();
    delete.connect_clicked({
        let popover = popover.clone();
        move |_| {
            popover.popdown();
            on_request(Request::Delete);
        }
    });
    popover.set_child(Some(&delete));
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
    /// A button showing Not downloaded. `on_request` runs a click's request.
    pub fn new(on_request: impl Fn(Request) + 'static) -> Self {
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
            move |button| match on_click(look.get()) {
                Request::Delete => offer_delete(button, Rc::clone(&on_request)),
                request => on_request(request),
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
    fn a_click_starts_retries_cancels_or_offers_delete() {
        assert_eq!(on_click(Look::NotDownloaded), Request::Start);
        assert_eq!(on_click(Look::Error), Request::Start, "an error retries");
        assert_eq!(on_click(Look::Queued), Request::Cancel);
        assert_eq!(on_click(Look::Downloading(60)), Request::Cancel);
        assert_eq!(on_click(Look::Downloaded), Request::Delete);
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
    fn the_button_shows_the_state_it_is_given() {
        crate::testing::with_display(|| {
            let button = DownloadButton::new(|_| {});
            assert_eq!(button.stack.visible_child_name().as_deref(), Some("icon"));
            button.set(DownloadState::Downloading, 40);
            assert_eq!(button.stack.visible_child_name().as_deref(), Some("ring"));
            assert_eq!(button.widget.tooltip_text().as_deref(), Some("Cancel (40%)"));
            button.set(DownloadState::Downloaded, 0);
            assert_eq!(button.icon.icon_name().as_deref(), Some("object-select-symbolic"));
        });
    }
}
