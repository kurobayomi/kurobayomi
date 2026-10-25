//! Tracker binding of an add from History (T-42a-019, EC-42-H-14, EC-42-H-17b): `bindEnhancedTrackers` runs once the
//! favourite is written, on the add path and on the confirm path alike, because both reach the store's `after_add`.
//!
//! The tracker services are owned by the tracking layer. It hands its [`TrackCtx`] to [`install`] when it starts them;
//! until then a binder finds no context and binds nothing, which is also what a user with no logged-in tracker gets.

use std::sync::{Arc, RwLock};

use kuro_platform::kind::Kind;
use kuro_track::ctx::TrackCtx;

use crate::browse::entries::TrackerBinder;

static CTX: RwLock<Option<Arc<TrackCtx>>> = RwLock::new(None);

thread_local! {
    /// The open entry pages, told on the GTK thread when auto-track bound an entry. A watcher returns false once its
    /// page is gone and is then dropped.
    static WATCHERS: std::cell::RefCell<Vec<Box<dyn Fn(i64) -> bool>>> = std::cell::RefCell::default();
}

/// Registers a page that wants to hear about auto-track binds (`f` gets the entry id). GTK thread only.
pub fn watch_binds(f: impl Fn(i64) -> bool + 'static) {
    WATCHERS.with(|w| w.borrow_mut().push(Box::new(f)));
}

fn bound(entry_id: i64) {
    WATCHERS.with(|w| w.borrow_mut().retain(|f| f(entry_id)));
}

/// Registers the tracking context the binders use. The last call wins.
pub fn install(ctx: Arc<TrackCtx>) {
    *CTX.write().unwrap_or_else(|e| e.into_inner()) = Some(ctx);
}

fn current() -> Option<Arc<TrackCtx>> {
    CTX.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The binder of one kind: every logged-in enhanced tracker that accepts the entry's source is matched and bound, then
/// auto-track binds the switched-on trackers when it is on and the match is certain (`kuro_track::auto_bind`), on the shell's runtime and
/// detached, so the add does not wait (B§5.2). Built on the GTK thread, where the shell is found.
pub fn binder(kind: Kind) -> TrackerBinder {
    let runtime = crate::shell::host::Shell::get().map(|shell| shell.runtime().clone()).or_else(|| tokio::runtime::Handle::try_current().ok());
    Arc::new(move |id, title, source| {
        let (Some(ctx), Some(runtime)) = (current(), runtime.as_ref()) else { return };
        let (title, source) = (title.to_owned(), source.to_string());
        runtime.spawn(async move {
            let failures = match kind {
                Kind::Anime => kuro_track::bind::bind_enhanced_anime(&ctx, id, &title, &source).await,
                Kind::Manga => kuro_track::bind::bind_enhanced_manga(&ctx, id, &title, &source).await,
            };
            for (tracker, error) in failures {
                tracing::warn!(entry = id, tracker = tracker.0, %error, "an enhanced tracker could not be bound");
            }
            let (bound_on, failed) = match kind {
                Kind::Anime => kuro_track::auto_bind::auto_bind_anime(&ctx, id, &title).await,
                Kind::Manga => kuro_track::auto_bind::auto_bind_manga(&ctx, id, &title).await,
            };
            for (tracker, error) in failed {
                tracing::warn!(entry = id, tracker = tracker.0, %error, "auto-track could not bind the title");
            }
            let names: Vec<String> = bound_on.iter().filter_map(|t| ctx.trackers.get(*t)).map(|t| t.name().to_owned()).collect();
            if !names.is_empty() {
                let text = kuro_i18n::trf(
                    kuro_i18n::MsgKey::track_auto_bound,
                    &[("title", kuro_i18n::Arg::Str(&title)), ("trackers", kuro_i18n::Arg::Str(&names.join(", ")))],
                );
                gtk::glib::MainContext::default().invoke(move || {
                    crate::toast::notify(text);
                    bound(id);
                });
            }
        });
    })
}

/// Binds the enhanced trackers of one entry that just joined the library, from the entry page: the entry's title and
/// source are read, then the binder of its kind runs, detached (B§5.2). Runs on the blocking pool.
pub fn bind_entry(db: &Arc<kuro_db::handle::Db>, kind: Kind, id: i64) {
    match kuro_data::entry::EntryRepo::new(Arc::clone(db)).by_id(id) {
        Ok(entry) => binder(kind)(entry.id, &entry.title, entry.source),
        Err(error) => tracing::warn!(entry = id, %error, "could not read the entry to bind its trackers"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bind_reaches_the_watching_pages_and_a_gone_page_is_dropped() {
        use std::cell::Cell;
        use std::rc::Rc;
        let (alive, gone) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let a = Rc::clone(&alive);
        watch_binds(move |id| {
            a.set(a.get() + id);
            true
        });
        let g = Rc::clone(&gone);
        watch_binds(move |_| {
            g.set(g.get() + 1);
            false
        });
        bound(5);
        bound(7);
        assert_eq!((alive.get(), gone.get()), (12, 1), "the second page left after the first bind");
    }

    #[test]
    fn without_a_tracking_context_the_binder_does_nothing() {
        // no context is installed in a unit test: the call must return without a runtime or a panic
        binder(Kind::Anime)(1, "T", 2);
        binder(Kind::Manga)(1, "T", 2);
    }
}
