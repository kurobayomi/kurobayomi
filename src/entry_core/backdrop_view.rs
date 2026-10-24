//! The backdrop behind the manga header (T-38a-014): the blurred cover at alpha 0.85 with a gradient from transparent to the
//! window background (`backdrop.rs`). The blur is made once per cover and kept in a cache keyed by the cover's url and
//! modification time, so a header drawn again shows it at once and a changed cover is blurred again.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;

use gtk::prelude::*;
use gtk::{gdk, glib};
use kuro_images::art::ArtRef;
use kuro_images::fetch::{ImageService, Payload};
use kuro_images::options::LoadOptions;
use tokio_util::sync::CancellationToken;

use super::backdrop::{BACKDROP_ALPHA, FADE_CLASS, cache_key, prepare};
use super::cover_view;

thread_local! {
    static CACHE: RefCell<HashMap<String, gdk::Texture>> = RefCell::default();
}

/// The blurred picture of a cover already made, if any.
fn cached(key: &str) -> Option<gdk::Texture> {
    CACHE.with(|c| c.borrow().get(key).cloned())
}

/// Runs `work` on the blocking pool of `handle` and yields its result on the GTK thread's receiver.
fn offload<T, F>(handle: &tokio::runtime::Handle, work: impl FnOnce() -> F + Send + 'static) -> async_channel::Receiver<T>
where
    T: Send + 'static,
    F: Future<Output = T>,
{
    let (tx, rx) = async_channel::bounded(1);
    let driver = handle.clone();
    handle.spawn_blocking(move || {
        let _ = tx.send_blocking(driver.block_on(work()));
    });
    rx
}

/// The bytes of the cover: the custom file when there is one, else what the image service fetches (its disk cache
/// first). No url or a failed fetch is no picture.
async fn cover_bytes(service: &ImageService, art: &ArtRef, custom: &Path) -> Option<Vec<u8>> {
    if tokio::fs::try_exists(custom).await.unwrap_or(false) {
        return tokio::fs::read(custom).await.ok();
    }
    art.url.as_ref()?;
    let fetched = service.fetch(art, &LoadOptions { use_custom_cover: false, ..LoadOptions::default() }, &CancellationToken::new()).await.ok()?;
    match fetched.payload {
        Payload::File(path) => tokio::fs::read(path).await.ok(),
        Payload::Bytes(bytes) => Some(bytes.to_vec()),
    }
}

/// The backdrop widget: it fills the space it is given and shows nothing until the picture is ready.
pub fn build(art: Option<&ArtRef>) -> gtk::Overlay {
    let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).can_shrink(true).hexpand(true).vexpand(true).opacity(BACKDROP_ALPHA).build();
    let fade = gtk::Box::builder().css_classes([FADE_CLASS]).hexpand(true).vexpand(true).build();
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&picture));
    overlay.add_overlay(&fade);
    overlay.set_can_target(false);
    if let Some(art) = art.filter(|a| a.url.is_some()) {
        load(art.clone(), &picture);
    }
    overlay
}

fn load(art: ArtRef, picture: &gtk::Picture) {
    let key = cache_key(art.url.as_deref().unwrap_or_default(), art.last_modified);
    if let Some(texture) = cached(&key) {
        picture.set_paintable(Some(&texture));
        return;
    }
    let (Some(service), Ok(runtime)) = (cover_view::service(), tokio::runtime::Handle::try_current()) else { return };
    let custom = kuro_platform::system_paths::SystemPaths::system().ok().map(|paths| kuro_images::cover_paths::custom_file_for(&paths, &art)).unwrap_or_default();
    let rx = offload(&runtime, move || async move {
        let bytes = cover_bytes(&service, &art, &custom).await?;
        prepare(&bytes)
    });
    let picture = picture.downgrade();
    glib::spawn_future_local(async move {
        let Ok(Some((w, h, rgba))) = rx.recv().await else { return };
        let bytes = glib::Bytes::from_owned(rgba);
        let texture: gdk::Texture = gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::R8g8b8a8, &bytes, (w * 4) as usize).upcast();
        CACHE.with(|c| c.borrow_mut().insert(key, texture.clone()));
        if let Some(picture) = picture.upgrade() {
            picture.set_paintable(Some(&texture));
        }
    });
}
