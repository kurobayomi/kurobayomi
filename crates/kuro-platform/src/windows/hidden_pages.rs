//! The WebView2 engine of [`crate::hidden_pages`]: one unshown WebView2 per site, all on one thread of their own.
//!
//! WebView2 works on a single-threaded apartment with a Windows message loop, and it answers through that loop, so the
//! pages live on a thread that runs nothing else. A plug-in's request reaches it through a channel and a wake-up message;
//! the page runs [`FETCH_JS`] and hands the answer back with `chrome.webview.postMessage`, so the requests of many
//! plug-ins run in a page at once. A timer on the same loop watches the pages load, clears the check and closes the idle
//! ones. The rules are the Linux engine's (`linux::hidden_pages`): the front page first, the address itself when the front
//! page is not enough, one reload when the pass has run out. Without the WebView2 Runtime (Windows 10 may lack it) every
//! request fails, and its source answers with the check.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use serde::Deserialize;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_PERMISSION_STATE_DENY, COREWEBVIEW2_WEB_ERROR_STATUS, COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT,
    COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED, COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE,
    CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2, ICoreWebView2_8, ICoreWebView2Controller,
    ICoreWebView2Environment, ICoreWebView2EnvironmentOptions,
};
use webview2_com::{
    CoTaskMemPWSTR, CoreWebView2EnvironmentOptions, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, ExecuteScriptCompletedHandler, NavigationCompletedEventHandler,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler, PermissionRequestedEventHandler,
    WebMessageReceivedEventHandler,
};
use windows::Win32::Foundation::{HWND, RECT};
use windows::core::{BOOL, Interface, PCWSTR, PWSTR};
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DispatchMessageW, GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetTimer,
    TranslateMessage, WM_APP, WM_TIMER, WS_POPUP,
};

use crate::hidden_pages::{ANSWER_TIMEOUT, FETCH_JS, HiddenPages, IDLE_CLOSE, PAGE_LANGUAGE, READY_TIMEOUT, is_check_page, origin_of, still_challenged, url_of};

/// How often the loop looks at the pages that load and at the idle ones.
const TICK: Duration = Duration::from_millis(250);
/// The size the unshown pages lay out at: a common desktop window, as the check expects a real one.
const PAGE_SIZE: (i32, i32) = (1280, 800);
/// Chromium switches of the pages' browser: an unshown page is otherwise taken for a background tab, whose timers are
/// slowed, and the check's script then runs past [`READY_TIMEOUT`]. A page never makes a sound.
const BROWSER_ARGUMENTS: &str =
    "--disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows --mute-audio";

/// The pages, with the folder their cookies and cache are kept in. The thread starts at the first request.
pub struct WebView2Pages {
    data: PathBuf,
    /// The thread's channel and id, or why it could not start (no WebView2 Runtime): it is not tried again in this run.
    worker: Mutex<Option<Result<Worker, String>>>,
}

struct Worker {
    jobs: mpsc::Sender<Job>,
    thread: u32,
}

/// One plug-in request and where its answer goes.
struct Job {
    request: String,
    origin: String,
    target: String,
    reply: mpsc::Sender<Result<String, String>>,
    /// The page was reloaded for it once already, because its pass had run out.
    reloaded: bool,
}

impl WebView2Pages {
    pub fn new(data: PathBuf) -> Self {
        Self { data, worker: Mutex::new(None) }
    }

    fn send(&self, job: Job) -> Result<(), String> {
        let mut worker = self.worker.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let started = worker.get_or_insert_with(|| start(self.data.clone()));
        let Ok(running) = started else { return Err(started.as_ref().err().cloned().unwrap_or_default()) };
        running.jobs.send(job).map_err(|_| "the hidden pages stopped".to_owned())?;
        // SAFETY: a plain message to a thread that has a message queue; it carries no pointers
        unsafe { PostThreadMessageW(running.thread, WM_APP, 0, 0) };
        Ok(())
    }
}

impl HiddenPages for WebView2Pages {
    /// Called on a plug-in's worker thread: the page work runs on the pages' thread and this waits for its answer.
    fn fetch(&self, request_json: &str) -> Result<String, String> {
        let (reply, answer) = mpsc::channel();
        let job = Job { request: request_json.to_owned(), origin: origin_of(request_json)?, target: url_of(request_json)?, reply, reloaded: false };
        self.send(job)?;
        answer.recv_timeout(ANSWER_TIMEOUT).map_err(|_| "the site's hidden page did not answer in time".to_owned())?
    }
}

/// Starts the pages' thread and waits until its browser is up (or could not start).
fn start(data: PathBuf) -> Result<Worker, String> {
    let (ready, started) = mpsc::channel();
    let (jobs, inbox) = mpsc::channel();
    std::thread::Builder::new()
        .name("hidden-pages".to_owned())
        .spawn(move || run(&data, inbox, ready))
        .map_err(|e| format!("the hidden pages could not start: {e}"))?;
    let thread = started.recv().map_err(|_| "the hidden pages could not start".to_owned())??;
    Ok(Worker { jobs, thread })
}

/// The pages' thread: the browser, then the message loop until the app quits.
fn run(data: &Path, inbox: mpsc::Receiver<Job>, ready: mpsc::Sender<Result<u32, String>>) {
    // SAFETY: once, at the start of this thread, before any COM object exists on it
    unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // the thread's message queue exists only once it is asked for messages; a wake-up posted before that would be lost
    unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE) };
    let environment = match environment(data) {
        Ok(environment) => environment,
        Err(error) => {
            tracing::warn!(%error, "no hidden pages: WebView2 did not start");
            let _ = ready.send(Err(error));
            return;
        }
    };
    // SAFETY: the predefined STATIC class; the window is never shown, it only hosts the pages
    let window = unsafe {
        CreateWindowExW(0, windows_sys::w!("STATIC"), windows_sys::w!(""), WS_POPUP, 0, 0, PAGE_SIZE.0, PAGE_SIZE.1, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null())
    };
    if window.is_null() {
        let _ = ready.send(Err("the hidden pages' window could not be made".to_owned()));
        return;
    }
    let pages = Rc::new(Pages { environment, window: HWND(window), sites: RefCell::default(), waiting: RefCell::default(), next_id: Cell::new(0) });
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
    unsafe { SetTimer(std::ptr::null_mut(), 0, TICK.as_millis() as u32, None) };
    // SAFETY: the standard message loop of this thread; `msg` is written by GetMessageW before it is read
    while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        if msg.hwnd.is_null() && (msg.message == WM_APP || msg.message == WM_TIMER) {
            // the timer also takes the jobs: a wake-up can be swallowed by a loop WebView2 runs while it starts
            while let Ok(job) = inbox.try_recv() {
                Pages::take(&pages, job);
            }
            if msg.message == WM_TIMER {
                Pages::tick(&pages);
            }
            continue;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// The browser all pages share, with its cookies in `data`.
fn environment(data: &Path) -> Result<ICoreWebView2Environment, String> {
    let options = CoreWebView2EnvironmentOptions::default();
    // SAFETY: a plain setter of the options object made above
    unsafe { options.set_additional_browser_arguments(BROWSER_ARGUMENTS.to_owned()) };
    // SAFETY: as above; the check's page then has its English title on any system (`PAGE_LANGUAGE`)
    unsafe { options.set_language(PAGE_LANGUAGE.to_owned()) };
    let options: ICoreWebView2EnvironmentOptions = options.into();
    let folder = data.to_string_lossy().into_owned();
    let (tx, rx) = mpsc::channel();
    CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| unsafe {
            let folder = CoTaskMemPWSTR::from(folder.as_str());
            CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), *folder.as_ref().as_pcwstr(), &options, &handler)
                .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |error, environment| {
            let _ = tx.send(error.map(|()| environment));
            Ok(())
        }),
    )
    .map_err(|e| format!("the WebView2 Runtime is missing or did not start: {e:?}"))?;
    match rx.recv() {
        Ok(Ok(Some(environment))) => Ok(environment),
        Ok(Err(e)) => Err(format!("the WebView2 Runtime is missing or did not start: {e}")),
        _ => Err("the WebView2 Runtime did not start".to_owned()),
    }
}

/// Where a site's page is.
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    /// WebView2 is making the page.
    Creating,
    /// It loads the site's front page.
    Front,
    /// It loads the address asked for: the front page was not enough, or the pass ran out.
    Target,
    Ready,
}

struct Site {
    controller: Option<ICoreWebView2Controller>,
    view: Option<ICoreWebView2>,
    phase: Phase,
    /// A navigation is under way (between its start and its end).
    navigating: bool,
    /// The page could not reach the site.
    failed: Option<String>,
    /// When the current phase began.
    since: Instant,
    used: Instant,
    /// The requests that wait for the page to be ready.
    queue: Vec<Job>,
}

struct Pages {
    environment: ICoreWebView2Environment,
    window: HWND,
    sites: RefCell<HashMap<String, Site>>,
    /// The requests sent to a page, by the id their answer comes back with.
    waiting: RefCell<HashMap<u64, (Job, Instant)>>,
    next_id: Cell<u64>,
}

/// The answer a page posts back.
#[derive(Deserialize)]
struct Posted {
    id: u64,
    answer: Option<String>,
    error: Option<String>,
}

impl Pages {
    /// A new request: sent now when its site's page is ready, else queued until it is (a page is made for a new site).
    fn take(pages: &Rc<Self>, job: Job) {
        let origin = job.origin.clone();
        let now = Instant::now();
        let mut sites = pages.sites.borrow_mut();
        let site = sites.entry(origin.clone()).or_insert_with(|| {
            Self::create(pages, &origin);
            Site { controller: None, view: None, phase: Phase::Creating, navigating: false, failed: None, since: now, used: now, queue: Vec::new() }
        });
        site.used = now;
        match site.view.clone() {
            Some(view) if site.phase == Phase::Ready && ready(&view, site.navigating, &origin) => {
                drop(sites);
                pages.send(&view, job);
            }
            Some(view) if site.phase == Phase::Ready => {
                // the page moved on (a redirect, a new check): it loads the front page again
                site.queue.push(job);
                navigate(site, &view, Phase::Front, &format!("{origin}/"));
            }
            _ => site.queue.push(job),
        }
    }

    /// Makes the page of `origin` in the hidden window; it loads the front page once it exists.
    fn create(pages: &Rc<Self>, origin: &str) {
        let (weak, site_origin) = (Rc::downgrade(pages), origin.to_owned());
        let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |error, controller| {
            let origin = site_origin.as_str();
            let Some(pages) = weak.upgrade() else { return Ok(()) };
            let made = error.map_err(|e| e.to_string()).and_then(|()| controller.ok_or_else(|| "no page".to_owned())).and_then(|controller| {
                // SAFETY: COM calls on the thread that owns the page
                unsafe { configure(&pages, origin, &controller) }.map(|view| (controller, view)).map_err(|e| e.to_string())
            });
            let Ok(mut sites) = pages.sites.try_borrow_mut() else { return Ok(()) };
            let Some(site) = sites.get_mut(origin) else { return Ok(()) };
            match made {
                Ok((controller, view)) => {
                    site.controller = Some(controller);
                    site.view = Some(view.clone());
                    navigate(site, &view, Phase::Front, &format!("{origin}/"));
                }
                Err(error) => site.failed = Some(format!("the site's hidden page could not be made: {error}")),
            }
            Ok(())
        }));
        // SAFETY: COM call on the pages' thread with its live window
        if let Err(error) = unsafe { pages.environment.CreateCoreWebView2Controller(pages.window, &handler) } {
            tracing::warn!(%error, site = origin, "a hidden page could not be made");
        }
    }

    /// Runs `job`'s request in `view`; the answer comes back through the page's message.
    fn send(&self, view: &ICoreWebView2, job: Job) {
        let id = self.next_id.get() + 1;
        self.next_id.set(id);
        let request = serde_json::to_string(&job.request).unwrap_or_default();
        let script = format!(
            "(async (req) => {{ {FETCH_JS} }})({request}).then(\
             a => chrome.webview.postMessage(JSON.stringify({{ id: {id}, answer: a }})),\
             e => chrome.webview.postMessage(JSON.stringify({{ id: {id}, error: String(e) }})));"
        );
        let script = CoTaskMemPWSTR::from(script.as_str());
        let done = ExecuteScriptCompletedHandler::create(Box::new(|_, _| Ok(())));
        // SAFETY: COM call on the pages' thread; the script text outlives the call
        match unsafe { view.ExecuteScript(*script.as_ref().as_pcwstr(), &done) } {
            Ok(()) => {
                self.waiting.borrow_mut().insert(id, (job, Instant::now()));
            }
            Err(error) => {
                let _ = job.reply.send(Err(format!("the site's hidden page failed the request: {error}")));
            }
        }
    }

    /// A page posted an answer: it goes back to its request, or, when the pass ran out, the page reloads once with the
    /// request's own address and the request waits for it.
    fn answered(pages: &Rc<Self>, message: &str) {
        let Ok(posted) = serde_json::from_str::<Posted>(message) else { return };
        let Some((mut job, _)) = pages.waiting.try_borrow_mut().ok().and_then(|mut waiting| waiting.remove(&posted.id)) else { return };
        let answer = match (posted.answer, posted.error) {
            (Some(answer), _) => Ok(answer),
            (None, error) => Err(format!("the site's hidden page failed the request: {}", error.unwrap_or_default())),
        };
        if !job.reloaded && answer.as_deref().is_ok_and(still_challenged) {
            job.reloaded = true;
            if let Ok(mut sites) = pages.sites.try_borrow_mut()
                && let Some(site) = sites.get_mut(&job.origin)
                && let Some(view) = site.view.clone()
            {
                let target = job.target.clone();
                site.queue.push(job);
                if site.phase == Phase::Ready {
                    navigate(site, &view, Phase::Target, &target);
                }
                return;
            }
        }
        let _ = job.reply.send(answer);
    }

    /// Every [`TICK`]: pages that finished loading take their queue, pages that failed or took too long drop it, idle
    /// pages close, and requests whose page never answered are forgotten (their caller has given up).
    fn tick(pages: &Rc<Self>) {
        let now = Instant::now();
        let mut ready_jobs = Vec::new();
        let mut dropped = Vec::new();
        for (origin, site) in pages.sites.borrow_mut().iter_mut() {
            if let Some(error) = site.failed.take() {
                fail(&mut site.queue, &error);
                dropped.push(origin.clone());
                continue;
            }
            let Some(view) = site.view.clone() else {
                if now.duration_since(site.since) > READY_TIMEOUT {
                    fail(&mut site.queue, "the site's hidden page could not be made in time");
                    dropped.push(origin.clone());
                }
                continue;
            };
            match site.phase {
                Phase::Front | Phase::Target if !site.navigating && !title(&view).is_some_and(|t| is_check_page(&t)) => {
                    if site.phase == Phase::Front && source_origin(&view).as_deref() != Some(origin.as_str()) {
                        // the front page went to another site: the page loads the address asked for instead
                        if let Some(target) = site.queue.first().map(|job| job.target.clone()) {
                            navigate(site, &view, Phase::Target, &target);
                            continue;
                        }
                    }
                    tracing::info!(site = origin, ms = now.duration_since(site.since).as_millis() as u64, "a hidden page loaded");
                    site.phase = Phase::Ready;
                    ready_jobs.extend(site.queue.drain(..).map(|job| (view.clone(), job)));
                }
                Phase::Front | Phase::Target if now.duration_since(site.since) > READY_TIMEOUT => {
                    let error = format!("the site's browser check did not clear within {} s", READY_TIMEOUT.as_secs());
                    fail(&mut site.queue, &error);
                    dropped.push(origin.clone());
                }
                Phase::Ready if site.queue.is_empty() && now.duration_since(site.used) > IDLE_CLOSE => dropped.push(origin.clone()),
                _ => {}
            }
        }
        for origin in dropped {
            if let Some(site) = pages.sites.borrow_mut().remove(&origin)
                && let Some(controller) = site.controller
            {
                // SAFETY: COM call on the pages' thread
                let _ = unsafe { controller.Close() };
            }
        }
        for (view, job) in ready_jobs {
            pages.send(&view, job);
        }
        pages.waiting.borrow_mut().retain(|_, (_, sent)| now.duration_since(*sent) < ANSWER_TIMEOUT);
    }
}

/// Lays the new page out in the hidden window, keeps it quiet (no permission, popup, dialog or sound) and listens to its
/// loads and its answers. The listeners only try to borrow the site table: should WebView2 raise an event from inside one
/// of our own calls, while the table is in use, a panic there would abort the app.
unsafe fn configure(pages: &Rc<Pages>, origin: &str, controller: &ICoreWebView2Controller) -> windows::core::Result<ICoreWebView2> {
    unsafe {
        controller.SetBounds(RECT { left: 0, top: 0, right: PAGE_SIZE.0, bottom: PAGE_SIZE.1 })?;
        controller.SetIsVisible(true)?;
        let view = controller.CoreWebView2()?;
        let settings = view.Settings()?;
        settings.SetAreDevToolsEnabled(false)?;
        settings.SetAreDefaultScriptDialogsEnabled(false)?;
        if let Ok(view8) = view.cast::<ICoreWebView2_8>() {
            let _ = view8.SetIsMuted(true);
        }
        let mut token = 0i64;
        view.add_PermissionRequested(
            &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        view.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetHandled(true)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        let (weak, site) = (Rc::downgrade(pages), origin.to_owned());
        view.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, _| {
                if let Some(pages) = weak.upgrade()
                    && let Ok(mut sites) = pages.sites.try_borrow_mut()
                    && let Some(page) = sites.get_mut(&site)
                {
                    page.navigating = true;
                }
                Ok(())
            })),
            &mut token,
        )?;
        let (weak, site) = (Rc::downgrade(pages), origin.to_owned());
        view.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                let Some(pages) = weak.upgrade() else { return Ok(()) };
                let Ok(mut sites) = pages.sites.try_borrow_mut() else { return Ok(()) };
                let Some(page) = sites.get_mut(&site) else { return Ok(()) };
                page.navigating = false;
                if let Some(args) = args {
                    let mut success = BOOL::default();
                    let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                    if args.IsSuccess(&mut success).is_ok() && !success.as_bool() && args.WebErrorStatus(&mut status).is_ok() && unreachable(status) {
                        page.failed = Some(format!("the site could not be opened ({site}): error {}", status.0));
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
        let weak = Rc::downgrade(pages);
        view.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                let (Some(pages), Some(args)) = (weak.upgrade(), args) else { return Ok(()) };
                let mut message = PWSTR::null();
                if args.TryGetWebMessageAsString(&mut message).is_ok() {
                    Pages::answered(&pages, &CoTaskMemPWSTR::from(message).to_string());
                }
                Ok(())
            })),
            &mut token,
        )?;
        Ok(view)
    }
}

/// Whether a failed load means the site cannot be reached at all; a check page that answers 403 or 503 is not that.
fn unreachable(status: COREWEBVIEW2_WEB_ERROR_STATUS) -> bool {
    [COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED, COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT, COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE].contains(&status)
}

/// Starts loading `uri` in the site's page, as `phase`.
fn navigate(site: &mut Site, view: &ICoreWebView2, phase: Phase, uri: &str) {
    site.phase = phase;
    site.since = Instant::now();
    // set here too: the start event comes later, and a tick in between must not take the old page for the new one
    site.navigating = true;
    let uri = CoTaskMemPWSTR::from(uri);
    // SAFETY: COM call on the pages' thread
    if let Err(error) = unsafe { view.Navigate(*uri.as_ref().as_pcwstr()) } {
        site.failed = Some(format!("the site could not be opened: {error}"));
    }
}

fn fail(queue: &mut Vec<Job>, error: &str) {
    for job in queue.drain(..) {
        let _ = job.reply.send(Err(error.to_owned()));
    }
}

/// Whether `view` can run a `fetch` for `origin` now: done loading, on that origin and past the check.
fn ready(view: &ICoreWebView2, navigating: bool, origin: &str) -> bool {
    !navigating && source_origin(view).as_deref() == Some(origin) && !title(view).is_some_and(|t| is_check_page(&t))
}

fn title(view: &ICoreWebView2) -> Option<String> {
    let mut text = PWSTR::null();
    // SAFETY: COM call on the pages' thread; WebView2 allocates the text and CoTaskMemPWSTR frees it
    unsafe { view.DocumentTitle(&mut text) }.ok()?;
    Some(CoTaskMemPWSTR::from(text).to_string())
}

/// The origin (`scheme://host[:port]`) of the page a view shows now.
fn source_origin(view: &ICoreWebView2) -> Option<String> {
    let mut text = PWSTR::null();
    // SAFETY: as in `title`
    unsafe { view.Source(&mut text) }.ok()?;
    url::Url::parse(&CoTaskMemPWSTR::from(text).to_string()).ok().map(|u| u.origin().ascii_serialization())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    use super::*;

    /// A site on this machine: its front page, and a JSON address that answers with what the request carried.
    fn site() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                std::thread::spawn(move || {
                    let mut stream = stream;
                    let mut head = [0u8; 4096];
                    let n = stream.read(&mut head).unwrap_or(0);
                    let head = String::from_utf8_lossy(&head[..n]).into_owned();
                    let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    let (kind, body) = if path.starts_with("/api") {
                        let tag = head.lines().find_map(|l| l.strip_prefix("X-Tag: ")).unwrap_or("none").to_owned();
                        ("application/json", format!(r#"{{"path":"{path}","tag":"{tag}"}}"#))
                    } else {
                        ("text/html", "<html><head><title>A site</title></head><body>front</body></html>".to_owned())
                    };
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                });
            }
        });
        address
    }

    #[test]
    fn requests_go_out_from_the_sites_page_several_at_once() {
        let data = tempfile::tempdir().unwrap();
        let pages = Arc::new(WebView2Pages::new(data.path().to_path_buf()));
        let address = site();
        let answers: Vec<String> = (0..5)
            .map(|i| {
                let (pages, request) = (Arc::clone(&pages), format!(r#"{{"method":"GET","url":"{address}/api?n={i}","headers":[["X-Tag","t{i}"]]}}"#));
                std::thread::spawn(move || pages.fetch(&request))
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|t| t.join().unwrap().expect("the page answers"))
            .collect();
        for (i, answer) in answers.iter().enumerate() {
            let answer: serde_json::Value = serde_json::from_str(answer).unwrap();
            assert_eq!(answer["status"], 200);
            let body: serde_json::Value = serde_json::from_str(answer["body"].as_str().unwrap()).unwrap();
            assert_eq!(body["path"], format!("/api?n={i}"));
            assert_eq!(body["tag"], format!("t{i}"), "the request's headers go with it");
        }
    }
}
