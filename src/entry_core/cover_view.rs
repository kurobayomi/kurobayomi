//! The cover widget (T-38a-059): a placeholder until the image is ready, then the image with a 150 ms
//! crossfade. The load runs on the blocking pool through the shared `kuro-images` service (fetch, disk cache and decode
//! all happen off the GTK thread); only the texture is built here. The image is keyed by the entry and its
//! `cover_last_modified`, so a newer stamp loads the cover again with no restart, and an image of an older load that
//! arrives late is dropped (`CoverBinding`).
//!
//! The service is installed by the app at startup (`install_images`). Without one the placeholder stays.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use gtk::prelude::*;
use gtk::{gdk, glib};
use kuro_images::art::ArtRef;
#[cfg(test)]
use kuro_platform::kind::Kind;
use kuro_images::cover_image::{CoverBinding, CoverState};
use kuro_images::error::ImageError;
use kuro_images::fetch::{ImageService, LoadRequest};
use kuro_images::options::{LoadOptions, Priority};
use kuro_images::texture::TextureSpec;

/// The crossfade between the placeholder and the image, and between an old image and its replacement.
pub const FADE_MS: u32 = 150;

/// The fade length for the desktop setting `gtk-enable-animations`: none when animations are off.
pub fn fade_ms(animations_enabled: bool) -> u32 {
    if animations_enabled { FADE_MS } else { 0 }
}

static SERVICE: OnceLock<Arc<ImageService>> = OnceLock::new();

/// Gives the covers the app's image service. The first call wins.
pub fn install_images(service: Arc<ImageService>) {
    let _ = SERVICE.set(service);
}

/// The image service the app installed, for the pieces that need picture bytes rather than a texture (the image window).
pub fn service() -> Option<Arc<ImageService>> {
    SERVICE.get().cloned()
}

/// Whether `wanted` is another image than the one shown: another entry or source, another url, or a newer
/// `cover_last_modified`. The favorite flag does not change the picture, so it does not reload it.
pub fn needs_load(shown: Option<&ArtRef>, wanted: &ArtRef) -> bool {
    shown.is_none_or(|s| {
        (s.owner, s.entry_id, s.source_id, s.kind, &s.url, s.last_modified) != (wanted.owner, wanted.entry_id, wanted.source_id, wanted.kind, &wanted.url, wanted.last_modified)
    })
}

/// A cover frame of a fixed size. Cloning shares the same widgets. It is the one cover widget of the app (entry headers,
/// list rows, listing cards; anime and manga alike).
///
/// A cover loads only while it is on screen: `load` on a hidden frame waits until it is shown, and a load still running
/// when the frame is hidden (a tab switched away, a page pushed over the list) is stopped and starts again on show. So a
/// screen the user is not looking at never holds the image queue up for the one they are.
#[derive(Clone)]
pub struct CoverView {
    pub frame: gtk::Frame,
    inner: Rc<Inner>,
}

struct Inner {
    stack: gtk::Stack,
    picture: gtk::Picture,
    /// The image on screen or loading.
    shown: RefCell<Option<ArtRef>>,
    /// The image asked for while the frame was hidden, loaded once it shows.
    wanted: RefCell<Option<ArtRef>>,
    priority: Cell<Priority>,
    binding: RefCell<CoverBinding>,
}

impl CoverView {
    pub fn new(width: i32, height: i32) -> Self {
        let animations = gtk::Settings::default().is_none_or(|s| s.is_gtk_enable_animations());
        let placeholder = gtk::Image::builder().icon_name("image-x-generic-symbolic").pixel_size(width / 3).css_classes(["dim-label"]).build();
        let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).can_shrink(true).build();
        let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).transition_duration(fade_ms(animations)).build();
        stack.add_named(&placeholder, Some("placeholder"));
        stack.add_named(&picture, Some("image"));
        // the image sits on an overlay over an empty base, so its natural size never reaches the layout: the cover is
        // always the size it is given (a large image made a history row as tall as the image)
        // the frame sets the size (border included); the overlay only keeps the image's natural size out of it
        let sized = crate::ui::sized(&stack, -1, -1);
        let frame = gtk::Frame::new(None);
        frame.set_size_request(width, height);
        frame.set_overflow(gtk::Overflow::Hidden);
        frame.set_child(Some(&sized));
        let inner = Rc::new(Inner { stack, picture, shown: RefCell::default(), wanted: RefCell::default(), priority: Cell::default(), binding: RefCell::default() });
        frame.connect_map({
            let inner = Rc::clone(&inner);
            move |_| Inner::start(&inner)
        });
        frame.connect_unmap({
            let inner = Rc::clone(&inner);
            move |_| inner.pause()
        });
        // a destroyed page cancels the load in flight
        frame.connect_destroy({
            let inner = Rc::clone(&inner);
            move |_| inner.binding.borrow_mut().unbind()
        });
        Self { frame, inner }
    }

    /// The image queue this cover waits in: `Priority::High` for the open page's own cover.
    pub fn with_priority(self, priority: Priority) -> Self {
        self.inner.priority.set(priority);
        self
    }

    /// Shows `art`. The same image again does nothing; another one (or a newer `cover_last_modified`) keeps what is on
    /// screen until the new image is ready and then fades to it. A hidden frame loads it once it is shown.
    pub fn load(&self, art: ArtRef) {
        let current = self.inner.wanted.borrow().clone().or_else(|| self.inner.shown.borrow().clone());
        if !needs_load(current.as_ref(), &art) {
            return;
        }
        *self.inner.wanted.borrow_mut() = Some(art);
        if self.frame.is_mapped() {
            Inner::start(&self.inner);
        }
    }
}

impl Inner {
    /// Starts the load of the image waiting for the frame to show, if any.
    fn start(this: &Rc<Self>) {
        let Some(art) = this.wanted.borrow_mut().take() else { return };
        *this.shown.borrow_mut() = Some(art.clone());
        let (generation, cancel) = this.binding.borrow_mut().bind();
        let (Some(service), Ok(runtime)) = (SERVICE.get().cloned(), tokio::runtime::Handle::try_current()) else { return };
        let options = LoadOptions { priority: this.priority.get(), ..LoadOptions::default() };
        let (done, loaded) = async_channel::bounded(1);
        // the service's future is not `Send`, so it runs to the end on a blocking-pool thread
        let driver = runtime.clone();
        runtime.spawn_blocking(move || {
            let url = art.url.clone();
            let result = driver.block_on(service.load(&LoadRequest { art, options, cancel }));
            // a cover that cannot be shown says why once, in the log; a cancelled load (the cover left the screen) is not
            // a failure
            if let Err(error) = &result
                && !matches!(error, ImageError::Cancelled)
            {
                tracing::warn!(url = url.as_deref().unwrap_or("none"), %error, "a cover could not be loaded");
            }
            let _ = done.send_blocking(result);
        });
        let this = Rc::clone(this);
        glib::spawn_future_local(async move {
            if let Ok(result) = loaded.recv().await
                && this.binding.borrow_mut().finish(generation, result)
            {
                this.show();
            }
        });
    }

    /// The frame was hidden: a load still running stops, and the image is asked for again when the frame shows. An
    /// image already on screen stays.
    fn pause(&self) {
        if !self.binding.borrow().in_flight() {
            return;
        }
        self.binding.borrow_mut().unbind();
        if let Some(art) = self.shown.borrow_mut().take() {
            self.wanted.borrow_mut().get_or_insert(art);
        }
    }

    /// Puts the loaded image on screen. A failed load leaves the placeholder.
    fn show(&self) {
        let binding = self.binding.borrow();
        let CoverState::Loaded(image) = binding.state() else { return };
        let spec = TextureSpec::from_decoded(image);
        let texture = gdk::MemoryTexture::new(spec.width, spec.height, gdk::MemoryFormat::R8g8b8a8, &glib::Bytes::from(&spec.pixels[..]), spec.stride);
        self.picture.set_paintable(Some(&texture));
        self.stack.set_visible_child_name("image");
    }
}

#[cfg(test)]
mod tests {
    use kuro_images::art::ArtKind;

    use super::*;

    fn art(last_modified: i64) -> ArtRef {
        ArtRef { owner: Kind::Anime, entry_id: 1, source_id: 7, kind: ArtKind::Cover, url: Some("https://x/a.jpg".into()), favorite: false, last_modified }
    }

    #[test]
    fn a_large_image_does_not_make_the_cover_larger_than_its_size() {
        crate::testing::with_display(|| {
            let view = CoverView::new(64, 96);
            let big = gdk::MemoryTexture::new(600, 900, gdk::MemoryFormat::R8g8b8a8, &glib::Bytes::from(&vec![0u8; 600 * 900 * 4][..]), 600 * 4);
            view.inner.picture.set_paintable(Some(&big));
            view.inner.stack.set_visible_child_name("image");
            let (_, natural_h, _, _) = view.frame.measure(gtk::Orientation::Vertical, 64);
            let (_, natural_w, _, _) = view.frame.measure(gtk::Orientation::Horizontal, -1);
            assert_eq!((natural_w, natural_h), (64, 96), "the image's 600x900 never reaches the layout");
        });
    }

    #[test]
    fn the_fade_is_150_ms_and_off_without_animations() {
        assert_eq!(fade_ms(true), 150);
        assert_eq!(fade_ms(false), 0);
    }

    #[test]
    fn nothing_shown_means_load() {
        assert!(needs_load(None, &art(5)));
    }

    #[test]
    fn a_newer_cover_stamp_loads_again_and_the_same_one_does_not() {
        assert!(!needs_load(Some(&art(5)), &art(5)));
        assert!(needs_load(Some(&art(5)), &art(6)), "cover_last_modified changed");
    }

    #[test]
    fn another_url_loads_but_a_favorite_flip_does_not() {
        let mut other = art(5);
        other.url = Some("https://x/b.jpg".into());
        assert!(needs_load(Some(&art(5)), &other));
        let mut favorite = art(5);
        favorite.favorite = true;
        assert!(!needs_load(Some(&art(5)), &favorite));
    }
}
