//! The backdrop behind the manga header (T-38a-014): the blurred cover at alpha 0.85 with a gradient from transparent to the
//! window background (`backdrop.rs`). The blur is made once per cover and kept in a cache keyed by the cover's url and
//! modification time, so a header drawn again shows it at once and a changed cover is blurred again.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use gtk::prelude::*;
use gtk::{gdk, glib};
use kuro_images::art::ArtRef;
use kuro_images::fetch::{ImageService, Payload};
use kuro_images::options::{LoadOptions, Priority};
use tokio_util::sync::CancellationToken;

use super::backdrop::{BACKDROP_ALPHA, FADE_CLASS, cache_key, prepare};
use crate::theme::palette::Rgb;
use super::cover_view;

thread_local! {
    /// The blurred picture of a cover and the cover's own colour, made together.
    static CACHE: RefCell<HashMap<String, (gdk::Texture, Option<Rgb>)>> = RefCell::default();
}

/// The blurred picture of a cover already made, with the cover's colour, if any.
fn cached(key: &str) -> Option<(gdk::Texture, Option<Rgb>)> {
    CACHE.with(|c| c.borrow().get(key).cloned())
}

/// Gives the page the backdrop is on the colour of its cover (`theme::tint`), or takes the tint off for a cover that
/// has none. The backdrop may be made before it is put in its page, so this waits until it is shown.
fn tint(picture: &gtk::Picture, colour: Option<Rgb>) {
    let apply = move |picture: &gtk::Picture| {
        let Some(page) = picture.ancestor(adw::NavigationPage::static_type()) else { return };
        match colour {
            Some(seed) => crate::theme::tint::apply(&page, seed),
            None => crate::theme::tint::clear(&page),
        }
    };
    if picture.is_mapped() {
        apply(picture);
    } else {
        picture.connect_map(apply);
    }
}

/// The bytes of the cover: the custom file when there is one, else what the image service fetches (its disk cache
/// first). No url or a failed fetch is no picture.
async fn cover_bytes(service: &ImageService, art: &ArtRef, custom: &Path) -> Option<Vec<u8>> {
    if tokio::fs::try_exists(custom).await.unwrap_or(false) {
        return tokio::fs::read(custom).await.ok();
    }
    art.url.as_ref()?;
    let fetched = service.fetch(art, &LoadOptions { use_custom_cover: false, priority: Priority::High, ..LoadOptions::default() }, &CancellationToken::new()).await.ok()?;
    match fetched.payload {
        Payload::File(path) => tokio::fs::read(path).await.ok(),
        Payload::Bytes(bytes) => Some(bytes.to_vec()),
    }
}

/// The backdrop widget: it fills the space it is given and shows nothing until the picture is ready.
pub fn build(art: Option<&ArtRef>) -> gtk::Overlay {
    let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).can_shrink(true).hexpand(true).vexpand(true).opacity(BACKDROP_ALPHA).build();
    let fade = gtk::Box::builder().css_classes([FADE_CLASS]).hexpand(true).vexpand(true).build();
    // the picture fades in when it is ready, like the cover in front of it, instead of popping in
    let reveal = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::Crossfade)
        .transition_duration(crate::widgets::status::SWITCH_FADE_MS)
        .child(&picture)
        .hexpand(true)
        .vexpand(true)
        .build();
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&reveal));
    overlay.add_overlay(&fade);
    overlay.set_can_target(false);
    if let Some(art) = art.filter(|a| a.url.is_some()) {
        load(art.clone(), &picture);
    }
    overlay
}

fn load(art: ArtRef, picture: &gtk::Picture) {
    let key = cache_key(art.url.as_deref().unwrap_or_default(), art.last_modified);
    if let Some((texture, colour)) = cached(&key) {
        show(picture, &texture);
        tint(picture, colour);
        return;
    }
    let Some(service) = cover_view::service() else { return };
    let custom = kuro_platform::system_paths::SystemPaths::system().ok().map(|paths| kuro_images::cover_paths::custom_file_for(&paths, &art)).unwrap_or_default();
    let picture = picture.downgrade();
    crate::background::run_local_async(
        move || async move {
            let bytes = cover_bytes(&service, &art, &custom).await?;
            prepare(&bytes)
        },
        move |prepared| {
        let Some((w, h, rgba, colour)) = prepared else { return };
        let bytes = glib::Bytes::from_owned(rgba);
        let texture: gdk::Texture = gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::R8g8b8a8, &bytes, (w * 4) as usize).upcast();
        CACHE.with(|c| c.borrow_mut().insert(key, (texture.clone(), colour)));
        if let Some(picture) = picture.upgrade() {
            show(&picture, &texture);
            tint(&picture, colour);
        }
        },
    );
}

/// Puts `texture` in the backdrop and fades it in.
fn show(picture: &gtk::Picture, texture: &gdk::Texture) {
    picture.set_paintable(Some(texture));
    if let Some(reveal) = picture.parent().and_downcast::<gtk::Revealer>() {
        reveal.set_reveal_child(true);
    }
}
