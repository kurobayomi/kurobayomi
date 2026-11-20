//! The action row widget (T-38a-017, T-39a-026): four equal buttons under the entry header, drawn from the rules of
//! `action_row`. The library button is the page's `LibraryButton`; the interval, tracking and Browser buttons are built
//! here. Tracking and Browser appear only when the page passes a callback for them.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Align, Box as GBox, Button, Label, Orientation as O};
use kuro_i18n::{MsgKey, tr, tr_plural};

use chrono::{Datelike, Timelike};

use crate::entry_core::action_row::{
    AirTime, NextUpdate, Tone, TrackingLabel, air_text, air_time, interval_click_opens_dialog, interval_tone, next_air, next_update, tracking_label,
    tracking_tone, interval_visible, tracking_visible, webview_visible,
};
use crate::entry_core::header_view::{LibraryButton, apply_tone};
use crate::theme::tokens::{SPACE_L, SPACE_S};

/// What the row shows, resolved by the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowState {
    pub favorite: bool,
    /// The source says the title is finished: it has no next update.
    pub completed: bool,
    pub next: NextUpdate,
    /// The weekly time of an anime the score site lists as on air, on the user's clock. `next` then counts the days to
    /// it, and the time itself is written under the count.
    pub air: Option<AirTime>,
    /// The entry has its own fetch interval (`fetch_interval < 0`).
    pub user_interval: bool,
    pub tracking_count: i64,
    /// The page has a tracking callback (anime), or always does (manga).
    pub has_tracking: bool,
    /// The source is HTTP and the page has a Browser callback.
    pub webview: bool,
}

impl RowState {
    /// The row of a manga or an anime page: the next update is counted from now, and a completed entry has none.
    /// `air` is the weekly slot of an anime on air ([`local_air_time`]), when the score site lists one.
    pub fn of_entry(favorite: bool, next_update_ms: i64, status: i64, fetch_interval: i32, tracking_count: i64, http_source: bool, air: Option<AirTime>) -> Self {
        let completed = status == kuro_domain::entries::flags::STATUS_COMPLETED;
        let next = match air {
            // an anime on air counts the days to its next weekly broadcast; every other title counts to the estimate
            Some(air) => {
                let now = chrono::Local::now();
                next_air(air, now.weekday(), now.hour() * 60 + now.minute())
            }
            None => next_update(Some(next_update_ms), chrono::Utc::now().timestamp_millis(), completed),
        };
        Self {
            favorite,
            completed,
            next,
            air,
            user_interval: fetch_interval < 0,
            tracking_count,
            has_tracking: true,
            webview: http_source,
        }
    }
}

/// A weekly slot of the score site on the clock of this computer, as it stands now (summer time included).
pub fn local_air_time(slot: kuro_scores::provider::AirSlot) -> AirTime {
    let offset_minutes = chrono::Local::now().offset().local_minus_utc() / 60;
    air_time(slot.utc_week_minute(), offset_minutes)
}

/// A button of the row with its icon and label, one child box so the icon and the text sit side by side.
#[derive(Clone)]
struct Slot {
    button: Button,
    label: Label,
    /// A second, smaller line under the label, shown only when there is something to add ([`Slot::detail`]).
    detail: Label,
    content: GBox,
}

impl Slot {
    fn new(icon: &str) -> Self {
        let icon_widget = gtk::Image::from_icon_name(icon);
        let label = Label::builder().ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(12).build();
        let child = GBox::new(O::Vertical, 2);
        child.set_halign(Align::Center);
        child.append(&icon_widget);
        child.append(&label);
        let detail = Label::builder().ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(12).css_classes(["caption"]).visible(false).build();
        child.append(&detail);
        let button = Button::builder().child(&child).css_classes(["flat"]).hexpand(true).focus_on_click(false).build();
        Self { button, label, detail, content: child }
    }

    /// The second line: `text` under the label, or no second line. The buttons of the row keep their icons and labels
    /// in line, since each packs them from the top.
    fn detail(&self, text: Option<&str>) {
        self.detail.set_visible(text.is_some());
        self.detail.set_label(text.unwrap_or_default());
    }

    fn text(&self, text: &str, tone: Tone) {
        self.label.set_label(text);
        apply_tone(&self.content, tone);
    }
}

#[derive(Clone)]
/// The four-button row. `on_interval`, `on_tracking` and `on_webview` run the buttons' own actions.
pub struct ActionRow {
    pub root: GBox,
    pub library: LibraryButton,
    interval: Slot,
    tracking: Slot,
    webview: Slot,
}

impl ActionRow {
    pub fn new(
        on_interval: Rc<dyn Fn()>,
        on_tracking: Option<Rc<dyn Fn()>>,
        on_webview: Option<Rc<dyn Fn()>>,
    ) -> Self {
        let root = GBox::new(O::Horizontal, 0);
        root.set_homogeneous(true);
        root.set_margin_start(SPACE_L);
        root.set_margin_end(SPACE_L);
        root.set_margin_top(SPACE_S);

        let library = LibraryButton::new();
        library.button.set_hexpand(true);
        library.button.set_halign(Align::Fill);
        library.button.set_margin_start(0);
        library.button.set_margin_end(0);
        root.append(&library.button);

        let interval = Slot::new("document-open-recent-symbolic");
        interval.button.connect_clicked(move |_| on_interval());
        root.append(&interval.button);

        let tracking = Slot::new("emblem-synchronizing-symbolic");
        if let Some(on_tracking) = on_tracking {
            tracking.button.connect_clicked(move |_| on_tracking());
        }
        root.append(&tracking.button);

        let webview = Slot::new("web-browser-symbolic");
        if let Some(on_webview) = on_webview {
            webview.button.connect_clicked(move |_| on_webview());
        }
        root.append(&webview.button);

        Self { root, library, interval, tracking, webview }
    }

    /// Draws the row for `state`: labels, colours, sensitivity and visibility.
    pub fn apply(&self, state: &RowState) {
        self.library.set(state.favorite);

        // the same words for every title; an anime on air adds its weekly time under them
        let next = match state.next {
            NextUpdate::NotApplicable => tr(MsgKey::not_applicable),
            NextUpdate::Soon => tr(MsgKey::manga_interval_expected_update_soon),
            NextUpdate::Days(days) => tr_plural(MsgKey::day, days, &[]),
        };
        self.interval.text(&next, interval_tone(state.user_interval));
        self.interval.detail(state.air.map(air_text).as_deref());
        let about_air = state.air.map(|_| tr(MsgKey::ui_airs_weekly_on_myanimelist));
        self.interval.button.set_tooltip_text(about_air.as_deref());
        self.interval.button.set_sensitive(interval_click_opens_dialog(state.favorite));
        self.interval.button.set_visible(interval_visible(state.next, state.favorite, state.completed, state.air.is_some()));

        let tracking_text = match tracking_label(state.tracking_count) {
            TrackingLabel::Tracking => tr(MsgKey::manga_tracking_tab),
            TrackingLabel::Trackers(n) => tr_plural(MsgKey::num_trackers, n, &[]),
        };
        self.tracking.text(&tracking_text, tracking_tone(state.tracking_count));
        self.tracking.button.set_visible(tracking_visible(state.has_tracking));

        self.webview.text(&tr(MsgKey::ui_browser), Tone::Inactive);
        self.webview.button.set_visible(webview_visible(state.webview));
    }
}
