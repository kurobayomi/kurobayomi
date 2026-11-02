//! Rows of a viewer and their live state (T-44a-006, T-44a-062, T-44a-064, T-44a-065, B§8.1, B§11.3, E-44). A page row
//! follows its page's status and download progress through `watch` receivers; a transition row follows the destination
//! chapter's load state. A row's subscriptions end when the row is dropped.
//!
//! A page whose bytes can be read (a file or an archive entry) shows its status as a spinner or a determinate ring, then
//! a `PageView` once its image is ready, fitted by the viewer's settings, and the error card when it fails. Any other
//! page keeps the text label. The state the label shows comes from the pure functions here.

use kuro_i18n::{MsgKey, tr};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use tokio::sync::watch;

use super::downloads::DownloadCheck;
use super::page_bytes::{self, Located};
use super::page_opts::{PageOpts, background_rgb};
use super::pager::nav::Phys;
use super::status::{PageView, StatusBranch};
use crate::reader::error_card;
use crate::reader::transition::{self, Metrics};
use crate::reader::model::ChapterTransition;
use kuro_data::item::Item;
use crate::reader::model::{ChapterState, PageStatus, ReaderChapter, ReaderPage, ViewerChapters};
use crate::reader::page_decode::LoadState;
use crate::reader::page_transform;
use crate::reader::page_view::PageView as PageWidget;
use crate::reader::page_zoom::{Hug, ZoomKey};
use crate::reader::progress::LoadingIndicator;

/// How often a fetched page's status is copied from its loader while it is not settled.
const POLL_MS: u64 = 100;

/// What a viewer hands a page row: the options it shows with, what a long press does (position in the page view) and
/// what a zoom does (the menu hides on any scale change, T-44a-071).
#[derive(Clone)]
pub struct PageHost {
    pub opts: PageOpts,
    pub on_long_tap: Rc<dyn Fn(Rc<ReaderPage>, f64, f64)>,
    /// The reader menu's flag: a long press also reports while the menu is shown (E-21).
    pub menu_open: Rc<Cell<bool>>,
    pub on_zoom: Rc<dyn Fn()>,
    /// A transform split a normal page in two: the pager places the insert page for it (T-44a-050).
    pub on_split: Rc<dyn Fn(Rc<ReaderPage>)>,
    /// A page decoded wider than tall (a spread already): two pages side by side show it alone.
    pub on_wide: Rc<dyn Fn(Rc<ReaderPage>)>,
    /// A row started or stopped showing its error card, so the strip lays out again (the card is 0.8 of the
    /// viewport, T-44a-064). May be called while a row is being built.
    pub on_layout: Rc<dyn Fn()>,
    /// The viewer is the webtoon strip, whose transition cards take the webtoon metrics (B§11.4).
    pub is_webtoon: bool,
    /// The pager reads right to left: a spread shows its second page on the left.
    pub right_to_left: bool,
    /// A transition's Retry asks for the destination chapter again (T-44a-067).
    pub on_retry: Rc<dyn Fn(Rc<ReaderChapter>)>,
    /// The download index, for a transition's destination (B§11.2). The lookup runs off the GTK thread.
    pub downloads: DownloadCheck,
}

/// Running subscriptions. Dropping the guard stops them.
#[derive(Default)]
pub struct Tasks(Vec<glib::JoinHandle<()>>);

impl Drop for Tasks {
    fn drop(&mut self) {
        self.0.drain(..).for_each(|task| task.abort());
    }
}

impl Tasks {
    /// Runs `on_change` on the main loop for every value `rx` takes after this call.
    fn follow<T: 'static>(&mut self, mut rx: watch::Receiver<T>, on_change: impl Fn(&T) + 'static) {
        rx.borrow_and_update();
        self.0.push(glib::spawn_future_local(async move {
            while rx.changed().await.is_ok() {
                on_change(&rx.borrow_and_update());
            }
        }));
    }
}

/// A realised row: the widget in the strip and the subscriptions that keep it current.
pub struct Row {
    pub widget: gtk::Widget,
    /// The row's page, or the two pages of a spread (left first); none for a transition or a label.
    images: Vec<Rc<Image>>,
    _tasks: Tasks,
    /// The loader's claim on a fetched page: dropping the row cancels what it queued and the page has not started.
    _guard: Option<Box<dyn Any>>,
}

impl Row {
    /// Applies changed settings to the shown page without reloading it (D-11): only what differs from the options the
    /// page holds is applied, so an unrelated change does not reset the user's zoom. A changed dual-page setting
    /// decodes the page again, since the transform is part of the decode.
    pub fn apply(&self, new: &PageOpts) {
        for image in &self.images {
            apply_to(image, new);
        }
    }

    /// Whether the row's page has been decoded and is shown.
    #[cfg(test)]
    pub fn page_ready(&self) -> bool {
        !self.images.is_empty() && self.images.iter().all(|i| i.view.state() == LoadState::Ready)
    }

    /// Whether the shown page can pan toward `dir` (`canPanLeft`, `canPanRight`). A row with no page cannot.
    pub fn can_pan(&self, dir: Phys) -> bool {
        self.images.iter().any(|i| match dir {
            Phys::Left => i.view.can_pan_left(),
            Phys::Right => i.view.can_pan_right(),
        })
    }

    /// Whether the shown page has room to pan by `(dx, dy)` screen px.
    pub fn can_pan_by(&self, dx: f64, dy: f64) -> bool {
        self.images.iter().any(|i| i.view.can_pan_by(dx, dy))
    }

    /// Pans the shown page by `(dx, dy)` screen px at once. `false` when it is at its edge that way.
    pub fn scroll_pan(&self, dx: f64, dy: f64) -> bool {
        self.images.iter().any(|i| i.view.scroll_pan(dx, dy))
    }

    /// One notch of Ctrl+wheel zoom on the shown page.
    pub fn zoom_wheel(&self, up: bool) {
        self.images.iter().for_each(|i| i.view.zoom_wheel(up));
    }

    /// Pans the shown page one view width toward `dir`.
    pub fn pan(&self, dir: Phys) {
        for image in &self.images {
            match dir {
                Phys::Left => image.view.pan_left(),
                Phys::Right => image.view.pan_right(),
            }
        }
    }

    /// The row's page became the selected one; `forward` is the direction of the selection (`onPageSelected`).
    pub fn page_selected(&self, forward: bool) {
        self.images.iter().for_each(|i| i.view.on_page_selected(forward));
    }

    /// Applies a zoom key to the shown page.
    pub fn zoom_key(&self, key: ZoomKey) {
        self.images.iter().for_each(|i| i.view.zoom_by_key(key));
    }

    /// The decoded size of the row's page, `None` until the page is shown. The webtoon strip sizes the row from it.
    pub fn source_size(&self) -> Option<(u32, u32)> {
        self.images.first().and_then(|i| i.view.source_size())
    }

    /// Whether the row shows its error card (a webtoon row is then 0.8 of the viewport tall).
    pub fn errored(&self) -> bool {
        self.images.iter().any(|i| i.errored.get())
    }
}

/// Applies changed settings to one shown page (`Row::apply`).
fn apply_to(image: &Image, new: &PageOpts) {
    let old = image.opts.replace(*new);
    if old.config != new.config {
        image.view.set_config(new.config);
    }
    if old.background != new.background {
        image.view.set_page_background(Some(rgba(*new)));
    }
    if old.automatic_background != new.automatic_background {
        image.view.set_auto_background(new.automatic_background);
    }
    image.view.set_double_tap_anim_ms(new.double_tap_ms);
    image.view.set_long_tap_enabled(new.long_tap);
    image.view.set_landscape_zoom(new.landscape_zoom);
    // the crop and the decode width are part of the decode, so a change decodes the page again
    image.view.set_crop_borders(new.crop);
    image.view.set_decode_to_view(new.decode_to_view());
    let redecode = old.transform != new.transform || old.crop != new.crop || old.decode_to_view() != new.decode_to_view();
    if redecode && image.view.state() != LoadState::Empty {
        image.load();
    }
}

fn rgba(opts: PageOpts) -> gtk::gdk::RGBA {
    let (r, g, b) = background_rgb(opts.background);
    gtk::gdk::RGBA::new(r, g, b, 1.0)
}

fn label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_valign(gtk::Align::Center);
    label.set_justify(gtk::Justification::Center);
    label
}

/// Two page rows side by side, a spread of the page-by-page modes: `first` and `second` in reading order, so right to
/// left puts `second` on the left. Each half is half the row, its page fitted in it and hugging the gutter between
/// them; a zoom (keys, wheel, double tap) zooms each half in its own half.
pub fn spread_row(first: Row, second: Row, right_to_left: bool) -> Row {
    let (left, right) = if right_to_left { (second, first) } else { (first, second) };
    let pair = gtk::Box::builder().homogeneous(true).hexpand(true).vexpand(true).build();
    for (row, hug) in [(&left, Hug::End), (&right, Hug::Start)] {
        pair.append(&row.widget);
        for image in &row.images {
            image.view.set_hug(hug);
        }
    }
    let images = left.images.iter().chain(&right.images).cloned().collect();
    Row { widget: pair.upcast(), images, _tasks: Tasks::default(), _guard: Some(Box::new((left, right))) }
}

/// A row with no state to follow.
pub fn plain_row(text: &str) -> Row {
    Row { widget: label(text).upcast(), images: Vec::new(), _tasks: Tasks::default(), _guard: None }
}

/// The line a page row shows for its holder view.
pub fn page_text(base: &str, view: PageView) -> String {
    match view {
        PageView::Ready => base.to_owned(),
        PageView::Spinner => format!("{base}\n{}", tr(MsgKey::loading)),
        PageView::Progress(percent) => format!("{base}\n{percent}%"),
        PageView::Error => format!("{base}\n{}", tr(MsgKey::download_notifier_title_error)),
    }
}

/// The widgets of an image row and what drives them: the stack (status, page, error card), the page view, and the
/// load of the page's bytes. Callbacks hold it weakly, so the row owns it and a dropped row stops everything.
struct Image {
    me: std::rc::Weak<Image>,
    stack: gtk::Stack,
    view: PageWidget,
    indicator: LoadingIndicator,
    opts: Cell<PageOpts>,
    page: Rc<ReaderPage>,
    located: Located,
    on_layout: Rc<dyn Fn()>,
    errored: Cell<bool>,
    /// The page's image has been asked for since the status last left `Ready`.
    started: Cell<bool>,
    /// A load is waiting for the loader to make the page ready again.
    waiting: Cell<bool>,
    poll_gen: Cell<u64>,
}

impl Image {
    fn build(_base: &str, page: &Rc<ReaderPage>, host: &PageHost, located: Located) -> Rc<Self> {
        let stack = gtk::Stack::new();
        // A page that is not shown yet is a centred spinner on a neutral surface and nothing else; the row keeps the
        // height its viewer reserved for it.
        let indicator = LoadingIndicator::new();
        let status = gtk::Box::new(gtk::Orientation::Vertical, 0);
        status.add_css_class("background");
        status.set_hexpand(true);
        status.set_vexpand(true);
        // a vertical box packs a non-expanding child at the top: the indicator must expand to be centred
        indicator.widget().set_hexpand(true);
        indicator.widget().set_vexpand(true);
        status.append(indicator.widget());
        stack.add_named(&status, Some("status"));

        let view = PageWidget::new();
        view.set_page_background(Some(rgba(host.opts)));
        view.set_auto_background(host.opts.automatic_background);
        view.set_double_tap_anim_ms(host.opts.double_tap_ms);
        view.set_long_tap_enabled(host.opts.long_tap);
        view.set_menu_open(Rc::clone(&host.menu_open));
        view.set_decode_to_view(host.opts.decode_to_view());
        view.set_crop_borders(host.opts.crop);
        view.set_landscape_zoom(host.opts.landscape_zoom);
        // a webtoon page does not zoom on its own: the strip does (T-44a-039)
        view.set_zoomable(!host.opts.webtoon);
        stack.add_named(&view, Some("page"));

        let image = Rc::new_cyclic(|me| Image {
            me: me.clone(),
            stack,
            view,
            indicator,
            opts: Cell::new(host.opts),
            page: Rc::clone(page),
            located,
            on_layout: Rc::clone(&host.on_layout),
            errored: Cell::new(false),
            started: Cell::new(false),
            waiting: Cell::new(false),
            poll_gen: Cell::new(0),
        });

        let (on_long_tap, on_zoom, shown) = (Rc::clone(&host.on_long_tap), Rc::clone(&host.on_zoom), Rc::clone(page));
        image.view.connect_long_tap(move |x, y| on_long_tap(Rc::clone(&shown), x, y));
        image.view.connect_scale_changed(move |_| on_zoom());
        if !page.is_insert_page() {
            let (on_split, parent) = (Rc::clone(&host.on_split), Rc::clone(page));
            image.view.connect_page_split(move || on_split(Rc::clone(&parent)));
        }
        let (weak, on_wide) = (image.me.clone(), Rc::clone(&host.on_wide));
        image.view.connect_image_loaded(move || {
            if let Some(image) = weak.upgrade() {
                if !image.page.is_insert_page() && image.view.source_size().is_some_and(|(w, h)| w > h) {
                    on_wide(Rc::clone(&image.page));
                }
                image.set_child("page");
                // the row now has the page's own height
                (image.on_layout)();
            }
        });
        let weak = image.me.clone();
        image.view.connect_image_load_error(move || {
            if let Some(image) = weak.upgrade() {
                // the bytes arrived but did not decode: the card says what was wrong with them
                *image.page.error.borrow_mut() = image.view.load_error();
                image.show_error();
            }
        });
        image
    }

    /// Shows one child of the stack. A change into or out of the error card tells the strip to lay out again.
    fn set_child(&self, name: &str) {
        self.stack.set_visible_child_name(name);
        let errored = name == "error";
        if self.errored.replace(errored) != errored {
            (self.on_layout)();
        }
    }

    /// What the page's status asks the row to show.
    fn on_view(&self, state: PageView) {
        match state {
            PageView::Spinner => {
                self.started.set(false);
                self.set_child("status");
                self.indicator.show_spinner();
            }
            PageView::Progress(percent) => {
                self.started.set(false);
                self.set_child("status");
                self.indicator.show();
                self.indicator.set_progress(percent);
            }
            PageView::Ready => {
                self.indicator.hide();
                if !self.started.replace(true) {
                    self.load();
                }
            }
            PageView::Error => {
                self.indicator.hide();
                self.show_error();
            }
        }
    }

    /// Reads the page's bytes and decodes them, or shows the decoded page from the cache. A fetched page whose file has
    /// gone waits for the loader (which queued it again) instead of failing (E-IMG-3).
    fn load(&self) {
        self.waiting.set(false);
        self.set_child("status");
        self.indicator.show_spinner();
        let Some(source) = self.located.resolve() else {
            if self.located.is_fetched() {
                self.wait();
            } else {
                self.show_error();
            }
            return;
        };
        let (config, intent) = self.decode_request();
        if self.view.cache_key_ready() && self.view.show_cached(source.clone(), config, intent) {
            return;
        }
        let (weak_view, weak_me) = (self.view.downgrade(), self.me.clone());
        glib::spawn_future_local(async move {
            let read_from = source.clone();
            let bytes = crate::background::blocking(move || page_bytes::read(&read_from)).await;
            let (Some(view), Some(me)) = (weak_view.upgrade(), weak_me.upgrade()) else { return };
            match bytes {
                Ok(Ok(bytes)) => {
                    let (config, intent) = me.decode_request();
                    // the view has its width by now, so a page that was decoded before is found under its key
                    if !view.show_cached(source.clone(), config, intent) {
                        view.set_image_from(source, bytes, config, intent);
                    }
                }
                _ => me.show_error(),
            }
        });
    }

    /// The page config and the transform the page decodes with, from the current options.
    fn decode_request(&self) -> (crate::reader::page_zoom::PageConfig, page_transform::DecodeIntent) {
        let opts = self.opts.get();
        (opts.config, page_transform::intent(opts.transform, self.page.is_insert_page()))
    }

    /// Shows the spinner and follows the loader until the page is ready again.
    fn wait(&self) {
        self.waiting.set(true);
        self.set_child("status");
        self.indicator.show_spinner();
        self.poll();
    }

    /// Copies a fetched page's status from its loader every `POLL_MS` until it is ready or has failed. A settled page
    /// that a load is waiting for is loaded then.
    fn poll(&self) {
        let Located::Fetched(root) = &self.located else { return };
        let token = self.poll_gen.get() + 1;
        self.poll_gen.set(token);
        let (weak, root) = (self.me.clone(), Rc::clone(root));
        glib::timeout_add_local(Duration::from_millis(POLL_MS), move || {
            let Some(me) = weak.upgrade() else { return glib::ControlFlow::Break };
            if me.poll_gen.get() != token {
                return glib::ControlFlow::Break;
            }
            if let Some(chapter) = root.chapter() {
                chapter.sync_page(&root);
            }
            match root.status.get() {
                PageStatus::Ready => {
                    if me.waiting.get() {
                        me.load();
                    }
                    glib::ControlFlow::Break
                }
                PageStatus::Error => {
                    if me.waiting.get() {
                        me.waiting.set(false);
                        me.show_error();
                    }
                    glib::ControlFlow::Break
                }
                _ => glib::ControlFlow::Continue,
            }
        });
    }

    /// The error card, built fresh so its Open in browser button sees the image url the loader has found by now.
    fn show_error(&self) {
        self.indicator.hide();
        if let Some(old) = self.stack.child_by_name("error") {
            self.stack.remove(&old);
        }
        let weak = self.me.clone();
        let card = error_card::card(
            move || {
                if let Some(image) = weak.upgrade() {
                    image.retry();
                }
            },
            self.page.image_url.borrow().clone(),
            self.page.error.borrow().clone(),
        );
        self.stack.add_named(&card, Some("error"));
        self.set_child("error");
    }

    /// The card's Retry (T-44a-065): a fetched page is queued again at the highest priority, a page on disk is read
    /// again.
    fn retry(&self) {
        match &self.located {
            Located::Fixed(_) => self.load(),
            Located::Fetched(root) => {
                if let Some(chapter) = root.chapter() {
                    chapter.retry_page(root.index as usize);
                }
                if root.status.get() == PageStatus::Ready {
                    // the file is there but could not be shown: read it again
                    self.load();
                } else {
                    self.started.set(false);
                    self.wait();
                }
            }
        }
    }
}

/// A page row. The row follows `page` (the parent of an insert page) when there is one. With a `host` and a page whose
/// bytes can be found (on disk, in an archive, or fetched by the chapter's loader), the page is shown as an image with
/// its status and error card (T-43b-014, T-44a-062).
pub fn page_row(base: String, page: Option<Rc<ReaderPage>>, host: Option<&PageHost>) -> Row {
    let Some(page) = page else { return plain_row(&base) };
    let shown = host.and_then(|h| page_bytes::locate(&page).map(|l| (h.clone(), l)));
    let (widget, image, show): (gtk::Widget, Option<Rc<Image>>, Rc<dyn Fn(PageView)>) = match shown {
        Some((host, located)) => {
            let image = Image::build(&base, &page, &host, located);
            let weak = Rc::downgrade(&image);
            let show: Rc<dyn Fn(PageView)> = Rc::new(move |state: PageView| {
                if let Some(image) = weak.upgrade() {
                    image.on_view(state);
                }
            });
            (image.stack.clone().upcast(), Some(image), show)
        }
        None => {
            // A page whose chapter has no loader stays a plain label with no status (E-40): it shows the page name only.
            let text = label(&base);
            let weak = text.downgrade();
            let base = base.clone();
            let page = Rc::clone(&page);
            let show: Rc<dyn Fn(PageView)> = Rc::new(move |view| {
                let loader = page.chapter().is_some_and(|c| c.has_loader());
                if let Some(text) = weak.upgrade() {
                    text.set_text(&if loader { page_text(&base, view) } else { base.clone() });
                }
            });
            (text.upcast(), None, show)
        }
    };
    // A fetched page asks its loader for the image once its row exists. An insert page shares its parent's request.
    let guard = image.as_ref().and_then(|image| match &image.located {
        Located::Fetched(root) if !page.is_insert_page() => root.chapter().and_then(|c| c.request_page(root.index as usize)),
        _ => None,
    });
    let branch = Rc::new(RefCell::new(StatusBranch::default()));
    let mut tasks = Tasks::default();
    {
        let (branch, show) = (Rc::clone(&branch), Rc::clone(&show));
        let update = move |status: &_| show(branch.borrow_mut().on_status(*status));
        update(&page.status.get());
        tasks.follow(page.status.subscribe(), update);
    }
    // Progress applies only while the download branch is the latest status (collectLatest, E-44).
    let (progress_branch, progress_show) = (Rc::clone(&branch), Rc::clone(&show));
    let on_progress = move |value: &i32| {
        let view = progress_branch.borrow().on_progress(*value);
        if let Some(view) = view {
            progress_show(view);
        }
    };
    on_progress(&page.progress.get());
    tasks.follow(page.progress.subscribe(), on_progress);
    if let Some(image) = &image {
        // a fetched page is followed from the first frame, so its status reaches the cells the row watches
        if matches!(image.located, Located::Fetched(_)) && page.status.get() != PageStatus::Ready {
            image.poll();
        }
    }
    Row { widget, images: image.into_iter().collect(), _tasks: tasks, _guard: guard }
}

fn set_child(slot: &gtk::Box, child: &impl IsA<gtk::Widget>) {
    clear(slot);
    slot.append(child);
}

fn clear(slot: &gtk::Box) {
    while let Some(old) = slot.first_child() {
        slot.remove(&old);
    }
}

/// How often a destination that is not downloaded is looked up again, so a download that finishes while the card is
/// shown updates its flag (T-43b-032).
// ponytail: polls the in-memory index every few seconds; subscribe to the download manager's completion events once the
// app has one.
const RECHECK_MS: u64 = 3_000;

/// A transition card (T-44a-066). The card shows the chapters and their downloaded flags. The destination's flag
/// comes from the lookup off the GTK thread, and the status line under the card follows the destination's state
/// (T-44a-067). The spacing is the viewer's metrics (T-44a-068). [`TransitionCard::bind`] with `None` clears it.
pub struct TransitionCard {
    root: gtk::Box,
    card_slot: gtk::Box,
    status_slot: gtk::Box,
    host: PageHost,
    transition: RefCell<Option<ChapterTransition>>,
    tasks: RefCell<Tasks>,
    /// Bumped by every bind, so the answer of an older lookup is dropped.
    generation: Cell<u64>,
}

impl TransitionCard {
    pub fn new(host: &PageHost) -> Rc<Self> {
        let metrics: Metrics = if host.is_webtoon { transition::WEBTOON_METRICS } else { transition::PAGER_METRICS };
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        // the root fills its slot and the card sits centred in it, both ways
        root.set_halign(gtk::Align::Fill);
        root.set_valign(gtk::Align::Fill);
        root.set_margin_start(metrics.side);
        root.set_margin_end(metrics.side);
        root.set_margin_top(metrics.top);
        root.set_margin_bottom(metrics.bottom);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.set_halign(gtk::Align::Center);
        content.set_valign(gtk::Align::Center);
        content.set_hexpand(true);
        content.set_vexpand(true);
        let card_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let status_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        status_slot.set_margin_top(metrics.status_margin);
        status_slot.set_margin_bottom(metrics.status_margin);
        content.append(&card_slot);
        content.append(&status_slot);
        root.append(&content);
        Rc::new(Self {
            root,
            card_slot,
            status_slot,
            host: host.clone(),
            transition: RefCell::new(None),
            tasks: RefCell::default(),
            generation: Cell::new(0),
        })
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Shows `transition`, or clears the card for `None`. Whatever an earlier binding was waiting for is dropped.
    pub fn bind(self: &Rc<Self>, transition: Option<ChapterTransition>) {
        self.generation.set(self.generation.get() + 1);
        *self.tasks.borrow_mut() = Tasks::default();
        clear(&self.card_slot);
        clear(&self.status_slot);
        *self.transition.borrow_mut() = transition.clone();
        let Some(transition) = transition else { return };
        self.show_card(false);
        if let Some(chapter) = transition.to() {
            self.show_status(&chapter.state());
            let weak = Rc::downgrade(self);
            self.tasks.borrow_mut().follow(chapter.subscribe_state(), move |state: &ChapterState| {
                if let Some(card) = weak.upgrade() {
                    card.show_status(state);
                }
            });
        }
        self.lookup_downloaded(self.generation.get());
    }

    fn show_card(&self, to_downloaded: bool) {
        let Some(transition) = self.transition.borrow().clone() else { return };
        // the current chapter is downloaded when its loader is local (B§11.2), and every chapter of a local manga is
        let from_downloaded = self.host.downloads.manga_is_local || transition.from().attached_is_local() == Some(true);
        let content = transition::content(&transition, from_downloaded, to_downloaded);
        set_child(&self.card_slot, &transition::build(&content));
    }

    fn show_status(&self, state: &ChapterState) {
        let Some(to) = self.transition.borrow().as_ref().and_then(|t| t.to().cloned()) else { return };
        let on_retry = Rc::clone(&self.host.on_retry);
        let retry: Rc<dyn Fn()> = Rc::new(move || on_retry(Rc::clone(&to)));
        match transition::status_box(state, retry) {
            Some(column) => set_child(&self.status_slot, &column),
            None => clear(&self.status_slot),
        }
    }

    /// Asks the download index about the destination off the GTK thread, and asks again later while it is not
    /// downloaded. An answer for an older binding is dropped.
    fn lookup_downloaded(self: &Rc<Self>, generation: u64) {
        let downloads = &self.host.downloads;
        let to: Option<Item> = self.transition.borrow().as_ref().and_then(|t| t.to().map(|c| c.chapter.clone()));
        let has_destination = to.is_some();
        let weak = Rc::downgrade(self);
        transition::going_to_downloaded_off_thread(downloads.manga_is_local, to, Arc::clone(&downloads.lookup), move |found| {
            let Some(card) = weak.upgrade() else { return };
            if card.generation.get() != generation {
                return;
            }
            card.show_card(found);
            if found || !has_destination {
                return;
            }
            let weak = Rc::downgrade(&card);
            glib::timeout_add_local_once(Duration::from_millis(RECHECK_MS), move || {
                if let Some(card) = weak.upgrade() {
                    if card.generation.get() == generation {
                        card.lookup_downloaded(generation);
                    }
                }
            });
        });
    }
}

/// The row of a transition card. The row keeps the card alive, and dropping the row ends its subscriptions.
pub fn transition_row(transition: ChapterTransition, host: &PageHost) -> Row {
    let card = TransitionCard::new(host);
    card.bind(Some(transition));
    Row { widget: card.widget(), images: Vec::new(), _tasks: Tasks::default(), _guard: Some(Box::new(card)) }
}

/// Calls `on_change` on the main loop whenever the load state of the current chapter or a neighbour changes
/// (B§11.3), so the viewer can hand its chapters over again. Dropping the guard stops the watch.
pub fn watch_chapters(chapters: &ViewerChapters, on_change: Rc<dyn Fn()>) -> Tasks {
    let mut tasks = Tasks::default();
    for chapter in std::iter::once(&chapters.curr).chain(chapters.prev.iter()).chain(chapters.next.iter()) {
        let on_change = Rc::clone(&on_change);
        tasks.follow(chapter.subscribe_state(), move |_: &ChapterState| on_change());
    }
    tasks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::model::{LoadError, PageLoader, PageSource};
    use crate::reader::viewers::ViewerKind;
    use crate::reader::viewers::page_opts::page_opts;
    use crate::reader::viewers::settings::ViewerSettings;
    use kuro_prefs::reader::ReaderPrefs;
    use kuro_prefs::store::PrefStore;
    use std::path::PathBuf;

    #[test]
    fn a_page_row_shows_each_holder_view() {
        assert_eq!(page_text("Page 1", PageView::Ready), "Page 1");
        assert_eq!(page_text("Page 1", PageView::Spinner), "Page 1\nLoading…");
        assert_eq!(page_text("Page 1", PageView::Progress(40)), "Page 1\n40%");
        assert_eq!(page_text("Page 1", PageView::Error), "Page 1\nError");
    }

    #[test]
    fn a_transition_status_follows_the_destination_state() {
        let error = ChapterState::Error(Rc::new(LoadError::new("No pages found")));
        assert_eq!(transition::status_text(&ChapterState::Wait), None);
        assert_eq!(transition::status_text(&ChapterState::Loading).as_deref(), Some("Loading pages…"));
        assert_eq!(transition::status_text(&error).as_deref(), Some("Failed to load pages: No pages found"));
        assert_eq!(transition::status_text(&ChapterState::Loaded(Rc::default())), None);
    }

    /// What a fetching loader has done, shared with the test that drives it.
    struct Fetch {
        status: Cell<PageStatus>,
        file: RefCell<Option<PathBuf>>,
        requested: Cell<u32>,
        retried: Cell<u32>,
        released: Rc<Cell<u32>>,
    }

    impl Fetch {
        fn new() -> Rc<Self> {
            Rc::new(Self {
                status: Cell::new(PageStatus::Queued),
                file: RefCell::default(),
                requested: Cell::new(0),
                retried: Cell::new(0),
                released: Rc::default(),
            })
        }
    }

    struct Guard(Rc<Cell<u32>>);

    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    struct FakeLoader(Rc<Fetch>);

    impl PageLoader for FakeLoader {
        fn recycle(&self) {}
        fn is_local(&self) -> bool {
            false
        }
        fn load_page(&self, _index: usize) -> Option<Box<dyn Any>> {
            self.0.requested.set(self.0.requested.get() + 1);
            Some(Box::new(Guard(Rc::clone(&self.0.released))))
        }
        fn retry_page(&self, _index: usize) {
            self.0.retried.set(self.0.retried.get() + 1);
            if self.0.status.get() == PageStatus::Error {
                self.0.status.set(PageStatus::Queued);
            }
        }
        fn image_file(&self, _index: usize) -> Option<PathBuf> {
            if self.0.status.get() == PageStatus::Ready { self.0.file.borrow().clone() } else { None }
        }
        fn sync_page(&self, page: &ReaderPage) {
            page.status.set(self.0.status.get());
        }
    }

    fn host(layouts: Rc<Cell<u32>>) -> PageHost {
        let settings = ViewerSettings::from_prefs(&ReaderPrefs::new(PrefStore::in_memory()));
        PageHost {
            opts: page_opts(&settings, ViewerKind::LeftToRight, false),
            on_long_tap: Rc::new(|_, _, _| {}),
            menu_open: Rc::default(),
            on_zoom: Rc::new(|| {}),
            on_split: Rc::new(|_| {}),
            on_wide: Rc::new(|_| {}),
            on_layout: Rc::new(move || layouts.set(layouts.get() + 1)),
            is_webtoon: false,
            right_to_left: false,
            on_retry: Rc::new(|_| {}),
            downloads: DownloadCheck::default(),
        }
    }

    fn png_file(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("kuro-holder-{}-{name}.png", std::process::id()));
        image::RgbaImage::from_pixel(6, 4, image::Rgba([1, 2, 3, 255])).save(&path).expect("write the test image");
        path
    }

    /// A fetched page of a chapter whose loader is the fake. The chapter must be kept alive by the caller.
    fn fetched(fetch: &Rc<Fetch>) -> (Rc<ReaderChapter>, Rc<ReaderPage>) {
        let chapter = ReaderChapter::new(kuro_data::item::Item::create(kuro_platform::kind::Kind::Manga));
        chapter.set_loader(Box::new(FakeLoader(Rc::clone(fetch))));
        let page = ReaderPage::new(0, "p0", Some("https://img/p0.png".into()), PageSource::Http);
        chapter.finish_loading(vec![Rc::clone(&page)]).unwrap();
        (chapter, page)
    }

    fn pump_until(done: impl Fn() -> bool) -> bool {
        let ctx = glib::MainContext::default();
        for _ in 0..600 {
            while ctx.iteration(false) {}
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    fn visible(row: &Row) -> String {
        row.widget.clone().downcast::<gtk::Stack>().expect("an image row is a stack").visible_child_name().map(|n| n.to_string()).unwrap_or_default()
    }

    #[test]
    fn a_network_page_shows_the_spinner_then_the_image_once_the_loader_has_it() {
        crate::testing::with_display(|| {
            let fetch = Fetch::new();
            let (_chapter, page) = fetched(&fetch);
            let row = page_row("Page 1".into(), Some(page), Some(&host(Rc::default())));
            assert_eq!(visible(&row), "status", "queued: the spinner");
            assert_eq!(fetch.requested.get(), 1, "the row asks its loader for the page");
            *fetch.file.borrow_mut() = Some(png_file("fetched"));
            fetch.status.set(PageStatus::Ready);
            assert!(pump_until(|| visible(&row) == "page"), "ready: the page view shows the decoded image");
            assert!(!row.errored());
            assert_eq!(fetch.released.get(), 0);
            drop(row);
            assert_eq!(fetch.released.get(), 1, "dropping the row lets go of what it queued");
        });
    }

    #[test]
    fn a_failed_network_page_shows_the_error_card_and_retry_asks_the_loader_again() {
        crate::testing::with_display(|| {
            let fetch = Fetch::new();
            fetch.status.set(PageStatus::Error);
            let (_chapter, page) = fetched(&fetch);
            let layouts = Rc::new(Cell::new(0));
            let row = page_row("Page 1".into(), Some(page), Some(&host(Rc::clone(&layouts))));
            assert!(pump_until(|| visible(&row) == "error"), "the loader's failure reaches the row");
            assert!(row.errored());
            assert_eq!(layouts.get(), 1, "the strip is told the row changed");
            row.images[0].retry();
            assert_eq!(fetch.retried.get(), 1);
            assert_eq!(visible(&row), "status", "retry shows the spinner again");
            assert!(!row.errored());
            *fetch.file.borrow_mut() = Some(png_file("retried"));
            fetch.status.set(PageStatus::Ready);
            assert!(pump_until(|| visible(&row) == "page"));
            assert_eq!(layouts.get(), 3, "leaving the error card is reported, and so is the page's own size");
        });
    }

    #[test]
    fn a_page_on_disk_that_cannot_be_read_or_decoded_gets_the_error_card() {
        crate::testing::with_display(|| {
            let missing = ReaderPage::new(0, "p0", None, PageSource::File(std::env::temp_dir().join("kuro-holder-missing.png")));
            missing.status.set(PageStatus::Ready);
            let row = page_row("Page 1".into(), Some(missing), Some(&host(Rc::default())));
            assert!(pump_until(|| visible(&row) == "error"));
            let garbage = std::env::temp_dir().join(format!("kuro-holder-{}-garbage.png", std::process::id()));
            std::fs::write(&garbage, b"not an image").unwrap();
            let broken = ReaderPage::new(1, "p1", None, PageSource::File(garbage));
            broken.status.set(PageStatus::Ready);
            let row = page_row("Page 2".into(), Some(broken), Some(&host(Rc::default())));
            assert!(pump_until(|| visible(&row) == "error"), "a decode failure is the error state, not a crash (E-45)");
        });
    }

    #[test]
    fn a_network_page_of_a_chapter_with_no_loader_stays_a_plain_label() {
        crate::testing::with_display(|| {
            let page = ReaderPage::new(0, "p0", None, PageSource::Http);
            let row = page_row("Page 1".into(), Some(page), Some(&host(Rc::default())));
            assert!(row.widget.clone().downcast::<gtk::Label>().is_ok(), "no loader, no status and no error (E-40)");
            assert!(!row.errored());
        });
    }

    #[test]
    fn an_insert_row_reads_the_halves_of_its_parents_file() {
        crate::testing::with_display(|| {
            let path = std::env::temp_dir().join(format!("kuro-holder-{}-spread.png", std::process::id()));
            image::RgbaImage::from_pixel(8, 4, image::Rgba([9, 9, 9, 255])).save(&path).unwrap();
            let parent = ReaderPage::new(0, "p0", None, PageSource::File(path));
            parent.status.set(PageStatus::Ready);
            let insert = ReaderPage::insert_page(&parent);
            let mut settings = ViewerSettings::from_prefs(&ReaderPrefs::new(PrefStore::in_memory()));
            settings.dual_split_pager = true;
            let mut host = host(Rc::default());
            host.opts = page_opts(&settings, ViewerKind::LeftToRight, false);
            let row = page_row("Page 1 (split)".into(), Some(insert), Some(&host));
            assert!(pump_until(|| visible(&row) == "page"));
            let (_, intent) = row.images[0].decode_request();
            assert_eq!(intent.transform, page_transform::PageTransform::SplitRight, "left to right: the insert is the right half");
            assert!(!intent.only_if_wide);
        });
    }
}
