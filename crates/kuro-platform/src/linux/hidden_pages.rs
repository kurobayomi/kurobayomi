//! The WebKitGTK engine of [`crate::hidden_pages`]: one unshown `WebView` per site, on the GTK thread.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use webkit6::glib;
use webkit6::prelude::*;

use crate::hidden_pages::{ANSWER_TIMEOUT, FETCH_JS, HiddenPages, IDLE_CLOSE, PAGE_LANGUAGE, READY_TIMEOUT, idle_sites, is_check_page, origin_of, still_challenged, url_of};

/// The pages, with the folders their cookies and cache are kept in.
pub struct WebKitPages {
    data: PathBuf,
    cache: PathBuf,
}

impl WebKitPages {
    pub fn new(data: PathBuf, cache: PathBuf) -> Self {
        Self { data, cache }
    }
}

impl HiddenPages for WebKitPages {
    /// Called on a plug-in's worker thread, never the window's: the page work runs on the window's thread and this
    /// waits for its answer.
    fn fetch(&self, request_json: &str) -> Result<String, String> {
        let (tx, rx) = mpsc::channel();
        let (json, data, cache) = (request_json.to_owned(), self.data.clone(), self.cache.clone());
        glib::MainContext::default().invoke(move || {
            glib::spawn_future_local(async move {
                let _ = tx.send(fetch_in_page(&data, &cache, &json).await);
            });
        });
        rx.recv_timeout(ANSWER_TIMEOUT).map_err(|_| "the site's hidden page did not answer in time".to_owned())?
    }
}

thread_local! {
    /// The page of each site (`scheme://host[:port]`), the session they share, when each was last used, and whether the
    /// sweep that closes idle pages runs.
    static VIEWS: RefCell<HashMap<String, webkit6::WebView>> = RefCell::new(HashMap::new());
    static SESSION: RefCell<Option<webkit6::NetworkSession>> = const { RefCell::new(None) };
    /// The pages' own web context, in [`PAGE_LANGUAGE`] whatever the system's: the check's page then keeps its English title.
    static CONTEXT: webkit6::WebContext = {
        let context = webkit6::WebContext::new();
        context.set_preferred_languages(&[PAGE_LANGUAGE]);
        context
    };
    static USED: RefCell<HashMap<String, Instant>> = RefCell::new(HashMap::new());
    static SWEEPING: Cell<bool> = const { Cell::new(false) };
    /// One turn at a time to load each site's page.
    static LOADING: RefCell<HashMap<String, Rc<futures_util::lock::Mutex<()>>>> = RefCell::new(HashMap::new());
}

/// The page's settings: no camera, microphone or WebRTC. Pages are only there to pass a site's browser check, and a
/// site's script that looks for capture devices must not wake the desktop's camera portal (a permission prompt).
fn quiet_settings() -> webkit6::Settings {
    let settings = webkit6::Settings::new();
    settings.set_enable_media_stream(false);
    settings.set_enable_mediasource(false);
    settings.set_enable_webrtc(false);
    settings.set_enable_webaudio(false);
    settings.set_enable_media(false);
    settings.set_media_playback_requires_user_gesture(true);
    settings
}

/// Notes a use of `origin`'s page and keeps a once-a-minute sweep running while any page is open.
fn touch(origin: &str) {
    USED.with(|used| used.borrow_mut().insert(origin.to_owned(), Instant::now()));
    if SWEEPING.replace(true) {
        return;
    }
    glib::timeout_add_seconds_local(60, || {
        let idle = USED.with(|used| idle_sites(&used.borrow(), Instant::now(), IDLE_CLOSE));
        for site in idle {
            USED.with(|used| used.borrow_mut().remove(&site));
            if let Some(view) = VIEWS.with(|views| views.borrow_mut().remove(&site)) {
                view.terminate_web_process();
            }
        }
        let open = VIEWS.with(|views| !views.borrow().is_empty());
        SWEEPING.set(open);
        if open { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
    });
}

async fn fetch_in_page(data: &Path, cache: &Path, request_json: &str) -> Result<String, String> {
    let origin = origin_of(request_json)?;
    let target = url_of(request_json)?;
    touch(&origin);
    let mut last = Err("the site's hidden page did not answer".to_owned());
    for reload in [false, true] {
        let view = page_for(&origin, &target, data, cache, reload).await?;
        let args = glib::VariantDict::new(None);
        args.insert("req", request_json);
        let sent = view
            .call_async_javascript_function_future(FETCH_JS, Some(&args.end()), None, None)
            .await
            .map(|value| value.to_str().to_string())
            .map_err(|e| format!("the site's hidden page failed the request: {e}"));
        let done = matches!(&sent, Ok(answer) if !still_challenged(answer));
        last = sent;
        if done {
            break;
        }
    }
    last
}

/// The origin (`scheme://host[:port]`) of the page a view shows now.
fn origin_of_view(view: &webkit6::WebView) -> Option<String> {
    url::Url::parse(&view.uri()?).ok().map(|u| u.origin().ascii_serialization())
}

/// The page of `origin`, ready to run a `fetch` of `target` (an address of that site): on the site's own origin and past
/// its check. A new page loads the site's front page; when that lands on another origin (an image host's front page may
/// redirect elsewhere, and a page may not read an answer from another origin) or the check is still up (`reload`), it loads
/// `target` itself. One caller at a time loads a site's page, so the callers that come together wait for it instead of
/// running on a page that is still loading.
async fn page_for(origin: &str, target: &str, data: &Path, cache: &Path, reload: bool) -> Result<webkit6::WebView, String> {
    let lock = LOADING.with(|loading| Rc::clone(loading.borrow_mut().entry(origin.to_owned()).or_default()));
    let _turn = lock.lock().await;
    let existing = VIEWS.with(|views| views.borrow().get(origin).cloned());
    if let Some(view) = existing.as_ref().filter(|view| !reload && is_ready(view, origin)) {
        return Ok(view.clone());
    }
    let view = existing.unwrap_or_else(|| {
        let session = SESSION.with(|s| {
            s.borrow_mut()
                .get_or_insert_with(|| webkit6::NetworkSession::new(data.to_str(), cache.to_str()))
                .clone()
        });
        let context = CONTEXT.with(webkit6::WebContext::clone);
        let view = webkit6::WebView::builder().web_context(&context).network_session(&session).settings(&quiet_settings()).build();
        // a hidden page never gets to ask for anything (camera, microphone, location, notifications...)
        view.connect_permission_request(|_, request| {
            request.deny();
            true
        });
        VIEWS.with(|views| views.borrow_mut().insert(origin.to_owned(), view.clone()));
        view
    });
    let front = format!("{origin}/");
    let result = async {
        // a page that is not ready (new, left on the check, or moved to another site) loads the front page, or the address
        // asked for when the front page is not enough (`reload`, or it lands on another origin)
        load(&view, if reload { target } else { &front }).await?;
        if origin_of_view(&view).as_deref() != Some(origin) {
            load(&view, target).await?;
        }
        Ok::<(), String>(())
    }
    .await;
    match result {
        Ok(()) => Ok(view),
        Err(error) => {
            // a page that failed to load is not kept: the next request would take it for a ready one
            tracing::warn!(site = origin, %error, "the hidden page was dropped");
            VIEWS.with(|views| views.borrow_mut().remove(origin));
            USED.with(|used| used.borrow_mut().remove(origin));
            view.terminate_web_process();
            Err(error)
        }
    }
}

/// Whether `view` can run a `fetch` for `origin` now: it shows a page of that origin that has finished loading and is not the
/// site's browser check.
fn is_ready(view: &webkit6::WebView, origin: &str) -> bool {
    !view.is_loading() && origin_of_view(view).as_deref() == Some(origin) && !view.title().is_some_and(|t| is_check_page(&t))
}

/// Loads `uri` in `view` and waits until the load has finished and the page is not the site's browser check. The title is
/// polled because the check may be solved in place, which changes the title without another load event.
async fn load(view: &webkit6::WebView, uri: &str) -> Result<(), String> {
    let started = Instant::now();
    let finished = Rc::new(Cell::new(false));
    let failure = Rc::new(RefCell::new(None::<String>));
    let loaded = {
        let finished = Rc::clone(&finished);
        view.connect_load_changed(move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                finished.set(true);
            }
        })
    };
    let failed = {
        let failure = Rc::clone(&failure);
        view.connect_load_failed(move |_, _, uri, error| {
            *failure.borrow_mut() = Some(format!("the site could not be opened ({uri}): {error}"));
            false
        })
    };
    view.load_uri(uri);
    let outcome = loop {
        glib::timeout_future(Duration::from_millis(250)).await;
        if let Some(error) = failure.borrow_mut().take() {
            break Err(error);
        }
        if finished.get() && !view.is_loading() && !view.title().is_some_and(|t| is_check_page(&t)) {
            break Ok(());
        }
        if started.elapsed() > READY_TIMEOUT {
            break Err(format!("the site's browser check did not clear within {} s", READY_TIMEOUT.as_secs()));
        }
    };
    view.disconnect(loaded);
    view.disconnect(failed);
    tracing::info!(uri, ms = started.elapsed().as_millis() as u64, ok = outcome.is_ok(), "a hidden page loaded");
    outcome
}
