//! The WebView2 engine of [`crate::hidden_pages`]: one unshown WebView2 per site, all on one thread of their own.
//!
//! WebView2 works on a single-threaded apartment with a Windows message loop, and it answers through that loop, so the
//! pages live on a thread that runs nothing else. A plug-in's request reaches it through a channel and a wake-up message;
//! the page runs [`FETCH_JS`] and hands the answer back with `chrome.webview.postMessage`, so the requests of many
//! plug-ins run in a page at once. A timer on the same loop watches the pages load, clears the check and closes the idle
//! ones. The rules are the Linux engine's (`linux::hidden_pages`): the front page first, the address itself when the front
//! page is not enough, one reload when the pass has run out. Without the WebView2 Runtime (Windows 10 may lack it) every
//! request fails, and its source answers with the check.
//!
//! A page that is read for a plug-in ([`HiddenPages::read`]) is a page of its own, apart from the one the site's requests
//! are sent from, so loading an address never ends a request in flight. It takes one read at a time: it loads the read's
//! address, runs [`READ_JS`], answers, and goes back to an empty page until the next. Every request such a page makes is
//! judged first ([`page_may_ask`]): only the hosts of the plug-in's list, and no image, media file or font. All pages
//! connect through the gate the app gives ([`WebView2Pages::through`]).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use serde::Deserialize;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_PERMISSION_STATE_DENY, COREWEBVIEW2_WEB_ERROR_STATUS, COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT,
    COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED, COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FONT,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA,
    CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2, ICoreWebView2_8, ICoreWebView2Controller,
    ICoreWebView2Environment, ICoreWebView2EnvironmentOptions,
};
use webview2_com::{
    CoTaskMemPWSTR, CoreWebView2EnvironmentOptions, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, ExecuteScriptCompletedHandler, NavigationCompletedEventHandler,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler, PermissionRequestedEventHandler,
    WebMessageReceivedEventHandler, WebResourceRequestedEventHandler,
};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::System::Com::IStream;
use windows::core::{BOOL, Interface, PCWSTR, PWSTR, w};
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DispatchMessageW, GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetTimer,
    TranslateMessage, WM_APP, WM_TIMER, WS_POPUP,
};

use crate::hidden_pages::{
    ANSWER_TIMEOUT, FETCH_JS, HiddenPages, IDLE_CLOSE, PAGE_LANGUAGE, PageRead, READ_JS, READY_TIMEOUT, is_check_page, origin_of, page_may_ask, still_challenged, url_of,
};

/// How often the loop looks at the pages that load and at the idle ones.
const TICK: Duration = Duration::from_millis(250);
/// How long the WebView2 engine may take to make a site's page. Its first start in a session is cold (it sets up its
/// profile and browser process) and can pass the site check's [`READY_TIMEOUT`] on a busy or slow machine, so making
/// the page has its own budget, under the caller's `ANSWER_TIMEOUT`; the check's time starts once the page exists.
const CREATE_TIMEOUT: Duration = Duration::from_secs(60);
/// The size the unshown pages lay out at: a common desktop window, as the check expects a real one.
const PAGE_SIZE: (i32, i32) = (1280, 800);
/// Chromium switches of the pages' browser: an unshown page is otherwise taken for a background tab, whose timers are
/// slowed, and the check's script then runs past [`READY_TIMEOUT`]. A page never makes a sound.
const BROWSER_ARGUMENTS: &str =
    "--disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows --mute-audio";

/// How long past its own time a read's page may stay silent before the read is given up: the page moved to another
/// address under the script, or its process died.
const READ_GRACE: Duration = Duration::from_secs(5);
/// What an empty page shows: a page that is read rests there between reads, so the site's script does not keep running.
const EMPTY_PAGE: &str = "about:blank";

/// The switches of the pages' browser: [`BROWSER_ARGUMENTS`], and with a gate, every connection through it. The list
/// of addresses that skip the proxy is set to "none, not even this computer's own", which Chromium otherwise reaches
/// directly, and a page can open no UDP connection past the proxy.
fn browser_arguments(gate: Option<SocketAddr>) -> String {
    match gate {
        Some(gate) => format!("{BROWSER_ARGUMENTS} --proxy-server=http://{gate} --proxy-bypass-list=<-loopback> --force-webrtc-ip-handling-policy=disable_non_proxied_udp"),
        None => BROWSER_ARGUMENTS.to_owned(),
    }
}

/// The pages, with the folder their cookies and cache are kept in. The thread starts at the first request.
pub struct WebView2Pages {
    data: PathBuf,
    /// The proxy every page connects through (`kuro_net::page_gate`); `None` only in tests, whose sites are on this computer.
    gate: Option<SocketAddr>,
    /// The thread's channel and id, or why it could not start (no WebView2 Runtime): it is not tried again in this run.
    worker: Mutex<Option<Result<Worker, String>>>,
}

struct Worker {
    jobs: mpsc::Sender<Job>,
    thread: u32,
}

/// One plug-in request, or one page to read, and where its answer goes.
struct Job {
    request: String,
    /// The page it runs in: the site for a request, [`PageRead::page_key`] for a read.
    key: String,
    origin: String,
    target: String,
    /// The read, when the job is one.
    read: Option<PageRead>,
    reply: mpsc::Sender<Result<String, String>>,
    /// The page was reloaded for it once already, because its pass had run out.
    reloaded: bool,
}

impl WebView2Pages {
    pub fn new(data: PathBuf) -> Self {
        Self { data, gate: None, worker: Mutex::new(None) }
    }

    /// The same pages with every connection sent through `gate`. Set before the first request: the browser takes its
    /// proxy when it starts.
    pub fn through(mut self, gate: Option<SocketAddr>) -> Self {
        self.gate = gate;
        self
    }

    fn send(&self, job: Job) -> Result<(), String> {
        let mut worker = self.worker.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let started = worker.get_or_insert_with(|| start(self.data.clone(), self.gate));
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
        let origin = origin_of(request_json)?;
        let job = Job { request: request_json.to_owned(), key: origin.clone(), origin, target: url_of(request_json)?, read: None, reply, reloaded: false };
        self.send(job)?;
        answer.recv_timeout(ANSWER_TIMEOUT).map_err(|_| "the site's hidden page did not answer in time".to_owned())?
    }

    /// Called on a plug-in's worker thread, as [`HiddenPages::fetch`] is.
    fn read(&self, read: &PageRead) -> Result<String, String> {
        let (reply, answer) = mpsc::channel();
        let job = Job { request: String::new(), key: read.page_key()?, origin: read.origin()?, target: read.url.clone(), read: Some(read.clone()), reply, reloaded: false };
        self.send(job)?;
        answer.recv_timeout(read.answer_timeout()).map_err(|_| "the page did not answer in time".to_owned())?
    }
}

/// Starts the pages' thread and waits until its browser is up (or could not start).
fn start(data: PathBuf, gate: Option<SocketAddr>) -> Result<Worker, String> {
    let (ready, started) = mpsc::channel();
    let (jobs, inbox) = mpsc::channel();
    std::thread::Builder::new()
        .name("hidden-pages".to_owned())
        .spawn(move || run(&data, gate, inbox, ready))
        .map_err(|e| format!("the hidden pages could not start: {e}"))?;
    let thread = started.recv().map_err(|_| "the hidden pages could not start".to_owned())??;
    Ok(Worker { jobs, thread })
}

/// The pages' thread: the browser, then the message loop until the app quits.
fn run(data: &Path, gate: Option<SocketAddr>, inbox: mpsc::Receiver<Job>, ready: mpsc::Sender<Result<u32, String>>) {
    // SAFETY: once, at the start of this thread, before any COM object exists on it
    unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // the thread's message queue exists only once it is asked for messages; a wake-up posted before that would be lost
    unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE) };
    let environment = match environment(data, gate) {
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
fn environment(data: &Path, gate: Option<SocketAddr>) -> Result<ICoreWebView2Environment, String> {
    let options = CoreWebView2EnvironmentOptions::default();
    // SAFETY: a plain setter of the options object made above
    unsafe { options.set_additional_browser_arguments(browser_arguments(gate)) };
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
    /// The navigation that is under way, once it has started, and the one it replaced while that was still under way:
    /// the end of the replaced one does not end this one.
    navigation: Option<u64>,
    superseded: Option<u64>,
    /// The page is one that is read for a plug-in, not the one requests are sent from.
    reading: bool,
    /// A read runs in the page now; the next waits in the queue.
    busy: bool,
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
        let origin = job.key.clone();
        let now = Instant::now();
        let mut sites = pages.sites.borrow_mut();
        let hosts = job.read.as_ref().map(|read| read.hosts.clone());
        let site = sites.entry(origin.clone()).or_insert_with(|| {
            let reading = hosts.is_some();
            Self::create(pages, &origin, hosts);
            Site { controller: None, view: None, phase: Phase::Creating, navigating: false, navigation: None, superseded: None, reading, busy: false, failed: None, since: now, used: now, queue: Vec::new() }
        });
        site.used = now;
        if site.reading {
            // a read waits its turn; a page that is free loads its address now
            site.queue.push(job);
            if let Some(view) = site.view.clone() {
                next_read(site, &view);
            }
            return;
        }
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

    /// Makes the page of `origin` in the hidden window; it loads the front page once it exists. With `hosts` it is a page
    /// that is read (`origin` is then its key): it asks those hosts only, and loads the first read that waits.
    fn create(pages: &Rc<Self>, origin: &str, hosts: Option<Vec<String>>) {
        let (weak, site_origin) = (Rc::downgrade(pages), origin.to_owned());
        let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |error, controller| {
            let origin = site_origin.as_str();
            let Some(pages) = weak.upgrade() else { return Ok(()) };
            let made = error.map_err(|e| e.to_string()).and_then(|()| controller.ok_or_else(|| "no page".to_owned())).and_then(|controller| {
                // SAFETY: COM calls on the thread that owns the page
                unsafe { configure(&pages, origin, hosts.as_deref(), &controller) }.map(|view| (controller, view)).map_err(|e| e.to_string())
            });
            let Ok(mut sites) = pages.sites.try_borrow_mut() else { return Ok(()) };
            let Some(site) = sites.get_mut(origin) else { return Ok(()) };
            match made {
                Ok((controller, view)) => {
                    site.controller = Some(controller);
                    site.view = Some(view.clone());
                    if site.reading {
                        site.phase = Phase::Ready;
                        next_read(site, &view);
                    } else {
                        navigate(site, &view, Phase::Front, &format!("{origin}/"));
                    }
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
        let (code, request) = match &job.read {
            Some(read) => (READ_JS, read.script_request()),
            None => (FETCH_JS, job.request.clone()),
        };
        let request = serde_json::to_string(&request).unwrap_or_default();
        let script = format!(
            "(async (req) => {{ {code} }})({request}).then(\
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
                if job.read.is_some() {
                    self.read_done(&job.key);
                }
            }
        }
    }

    /// The read that ran in the page `key` is over: the page loads the next read that waits, or goes back to an empty
    /// page.
    fn read_done(&self, key: &str) {
        let Ok(mut sites) = self.sites.try_borrow_mut() else { return };
        let Some(site) = sites.get_mut(key) else { return };
        site.busy = false;
        site.used = Instant::now();
        let Some(view) = site.view.clone() else { return };
        if site.queue.is_empty() {
            navigate(site, &view, Phase::Ready, EMPTY_PAGE);
        } else {
            next_read(site, &view);
        }
    }

    /// A page posted an answer: it goes back to its request, or, when the pass ran out, the page reloads once with the
    /// request's own address and the request waits for it.
    ///
    /// `page` is the page the message came from. A page answers only what was asked of it: the site's own script can post
    /// messages too, and must not be able to answer a request that runs in another site's page.
    fn answered(pages: &Rc<Self>, page: &str, message: &str) {
        let Ok(posted) = serde_json::from_str::<Posted>(message) else { return };
        let asked_here = |waiting: &HashMap<u64, (Job, Instant)>| waiting.get(&posted.id).is_some_and(|(job, _)| job.key == page);
        let Some((mut job, _)) = pages.waiting.try_borrow_mut().ok().and_then(|mut waiting| if asked_here(&waiting) { waiting.remove(&posted.id) } else { None }) else { return };
        if job.read.is_some() {
            let answer = match (posted.answer, posted.error) {
                (Some(answer), _) => Ok(answer),
                (None, error) => Err(format!("the page could not be read: {}", error.unwrap_or_default())),
            };
            let _ = job.reply.send(answer);
            pages.read_done(&job.key);
            return;
        }
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
                if now.duration_since(site.since) > CREATE_TIMEOUT {
                    fail(&mut site.queue, "the site's hidden page could not be made in time");
                    dropped.push(origin.clone());
                }
                continue;
            };
            match site.phase {
                // a page that is read has loaded the address of the read that waits first: that read runs in it now
                Phase::Target if site.reading && !site.navigating && !title(&view).is_some_and(|t| is_check_page(&t)) => {
                    tracing::info!(page = origin, ms = now.duration_since(site.since).as_millis() as u64, "a hidden page loaded to be read");
                    site.phase = Phase::Ready;
                    if !site.queue.is_empty() {
                        site.busy = true;
                        ready_jobs.push((view.clone(), site.queue.remove(0)));
                    }
                }
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
                    let error = if site.reading && !title(&view).is_some_and(|t| is_check_page(&t)) {
                        format!("the page did not finish loading within {} s", READY_TIMEOUT.as_secs())
                    } else {
                        format!("the site's browser check did not clear within {} s", READY_TIMEOUT.as_secs())
                    };
                    fail(&mut site.queue, &error);
                    dropped.push(origin.clone());
                }
                Phase::Ready if site.queue.is_empty() && !site.busy && now.duration_since(site.used) > IDLE_CLOSE => dropped.push(origin.clone()),
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
        // a read whose page stayed silent past its time is answered with that, and its page is free again
        let silent: Vec<u64> = pages
            .waiting
            .borrow()
            .iter()
            .filter(|(_, (job, sent))| job.read.as_ref().is_some_and(|read| now.duration_since(*sent) > read.timeout + READ_GRACE))
            .map(|(id, _)| *id)
            .collect();
        for id in silent {
            let Some((job, _)) = pages.waiting.borrow_mut().remove(&id) else { continue };
            let _ = job.reply.send(Err("the page stopped answering while it was read (it may have moved to another address)".to_owned()));
            pages.read_done(&job.key);
        }
        pages.waiting.borrow_mut().retain(|_, (_, sent)| now.duration_since(*sent) < ANSWER_TIMEOUT);
    }
}

/// A page that is read and is free takes the read that waits first: it loads that read's address.
fn next_read(site: &mut Site, view: &ICoreWebView2) {
    if site.busy || site.phase != Phase::Ready {
        return;
    }
    if let Some(target) = site.queue.first().map(|job| job.target.clone()) {
        navigate(site, view, Phase::Target, &target);
    }
}

/// Lays the new page out in the hidden window, keeps it quiet (no permission, popup, dialog or sound) and listens to its
/// loads and its answers. The listeners only try to borrow the site table: should WebView2 raise an event from inside one
/// of our own calls, while the table is in use, a panic there would abort the app.
unsafe fn configure(pages: &Rc<Pages>, origin: &str, hosts: Option<&[String]>, controller: &ICoreWebView2Controller) -> windows::core::Result<ICoreWebView2> {
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
        let (weak, site, allowed) = (Rc::downgrade(pages), origin.to_owned(), hosts.map(<[String]>::to_vec));
        view.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                let mut id = None;
                if let Some(args) = args {
                    let mut number = 0u64;
                    id = args.NavigationId(&mut number).is_ok().then_some(number);
                    // a page that is read goes nowhere but the hosts of its plug-in's list
                    if let Some(allowed) = &allowed {
                        let mut uri = PWSTR::null();
                        if args.Uri(&mut uri).is_err() || !page_may_ask(allowed, &CoTaskMemPWSTR::from(uri).to_string()) {
                            args.SetCancel(true)?;
                        }
                    }
                }
                if let Some(pages) = weak.upgrade()
                    && let Ok(mut sites) = pages.sites.try_borrow_mut()
                    && let Some(page) = sites.get_mut(&site)
                {
                    page.navigating = true;
                    page.navigation = id;
                }
                Ok(())
            })),
            &mut token,
        )?;
        if let Some(allowed) = hosts {
            let (weak, allowed) = (Rc::downgrade(pages), allowed.to_vec());
            view.AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
            view.add_WebResourceRequested(
                &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                    let (Some(pages), Some(args)) = (weak.upgrade(), args) else { return Ok(()) };
                    let mut context = COREWEBVIEW2_WEB_RESOURCE_CONTEXT::default();
                    args.ResourceContext(&mut context)?;
                    let mut uri = PWSTR::null();
                    args.Request()?.Uri(&mut uri)?;
                    let uri = CoTaskMemPWSTR::from(uri).to_string();
                    let unwanted = unwanted(context);
                    if unwanted || !page_may_ask(&allowed, &uri) {
                        // a host a page needs and its manifest lacks shows here, for whoever writes the source
                        if !unwanted {
                            tracing::debug!(address = %uri.chars().take(200).collect::<String>(), "a page that is read asked a host outside its list");
                        }
                        // answered here, so the request never leaves the page
                        let refusal = pages.environment.CreateWebResourceResponse(None::<&IStream>, 403, w!("Blocked"), PCWSTR::null())?;
                        args.SetResponse(&refusal)?;
                    }
                    Ok(())
                })),
                &mut token,
            )?;
        }
        let (weak, site) = (Rc::downgrade(pages), origin.to_owned());
        view.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                let Some(pages) = weak.upgrade() else { return Ok(()) };
                let Ok(mut sites) = pages.sites.try_borrow_mut() else { return Ok(()) };
                let Some(page) = sites.get_mut(&site) else { return Ok(()) };
                let Some(args) = args else {
                    page.navigating = false;
                    return Ok(());
                };
                // the end of a navigation that a newer one replaced says nothing about the newer one
                let mut id = 0u64;
                if args.NavigationId(&mut id).is_ok() && (page.superseded == Some(id) || page.navigation.is_some_and(|current| current != id)) {
                    return Ok(());
                }
                page.navigating = false;
                {
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
        let (weak, site) = (Rc::downgrade(pages), origin.to_owned());
        view.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                let (Some(pages), Some(args)) = (weak.upgrade(), args) else { return Ok(()) };
                let mut message = PWSTR::null();
                if args.TryGetWebMessageAsString(&mut message).is_ok() {
                    Pages::answered(&pages, &site, &CoTaskMemPWSTR::from(message).to_string());
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

/// What a page that is read never loads: it is read for its text, and a reader's pictures are on hosts of their own.
fn unwanted(context: COREWEBVIEW2_WEB_RESOURCE_CONTEXT) -> bool {
    [COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FONT].contains(&context)
}

/// Starts loading `uri` in the site's page, as `phase`.
fn navigate(site: &mut Site, view: &ICoreWebView2, phase: Phase, uri: &str) {
    site.phase = phase;
    site.since = Instant::now();
    // set here too: the start event comes later, and a tick in between must not take the old page for the new one
    if site.navigating && site.navigation.is_some() {
        site.superseded = site.navigation;
    }
    site.navigating = true;
    site.navigation = None;
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

    /// A site on this machine for a page that is read: a reader whose 30 page slots get their picture only when they
    /// come into view, as a lazy reader does. Its script also asks `other` (a host outside the list) twice. Returns the
    /// site's address and the paths asked of it.
    fn reader_site(other: &str) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let asked = Arc::new(Mutex::new(Vec::new()));
        let (seen, other) = (Arc::clone(&asked), other.to_owned());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (seen, other) = (Arc::clone(&seen), other.clone());
                std::thread::spawn(move || {
                    let mut stream = stream;
                    let mut head = [0u8; 4096];
                    let n = stream.read(&mut head).unwrap_or(0);
                    let head = String::from_utf8_lossy(&head[..n]).into_owned();
                    let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    seen.lock().unwrap().push(path.clone());
                    let body = if path.starts_with("/chapter") {
                        format!(
                            r#"<html><head><title>Reader</title></head><body><div id="list"></div><script>
for (let i = 1; i <= 30; i++) {{
  const slot = document.createElement('div');
  slot.className = 'slot';
  slot.dataset.page = i;
  slot.style.height = '900px';
  document.getElementById('list').appendChild(slot);
}}
const inView = new IntersectionObserver((entries) => {{
  for (const entry of entries) {{
    if (!entry.isIntersecting || entry.target.dataset.asked) continue;
    entry.target.dataset.asked = '1';
    setTimeout(() => {{
      const picture = document.createElement('img');
      picture.src = '/img/' + entry.target.dataset.page + '.png';
      entry.target.appendChild(picture);
    }}, 40);
  }}
}});
document.querySelectorAll('.slot').forEach((slot) => inView.observe(slot));
fetch('{other}/leak').catch(() => {{}});
new Image().src = '{other}/pixel.png';
</script></body></html>"#
                        )
                    } else {
                        "<html><head><title>A site</title></head><body>front</body></html>".to_owned()
                    };
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                });
            }
        });
        (address, asked)
    }

    fn read_of(address: &str, steps: Vec<crate::hidden_pages::Step>, seconds: u64) -> PageRead {
        PageRead { url: address.to_owned(), hosts: vec!["127.0.0.1".to_owned()], steps, timeout: Duration::from_secs(seconds) }
    }

    #[test]
    fn a_page_is_read_after_its_slots_were_scrolled_into_view_and_asks_only_its_hosts() {
        use crate::hidden_pages::Step;
        // the same machine under another name: a host the read's list does not have
        let (outside, outside_asked) = reader_site("http://127.0.0.1:1");
        let outside = outside.replace("127.0.0.1", "localhost");
        let (address, asked) = reader_site(&outside);
        let data = tempfile::tempdir().unwrap();
        let pages = WebView2Pages::new(data.path().to_path_buf());

        let steps = vec![Step::Wait(".slot".into()), Step::Scroll(".slot".into()), Step::WaitGone(".slot:not(:has(img))".into()), Step::Read(".slot img".into()), Step::Read("title".into())];
        let answer = pages.read(&read_of(&format!("{address}/chapter/1"), steps.clone(), 30)).expect("the page is read");
        let answer: serde_json::Value = serde_json::from_str(&answer).unwrap();
        assert_eq!(answer["title"], "Reader");
        assert_eq!(answer["url"], format!("{address}/chapter/1"));
        let pictures = answer["reads"][0].as_str().unwrap();
        assert_eq!(pictures.matches("<img").count(), 30, "every slot got its picture's address: {pictures}");
        assert!(pictures.contains(r#"src="/img/30.png""#), "{pictures}");
        assert_eq!(answer["reads"][1], "<title>Reader</title>");

        // a second read waits for its turn in the same page and works as the first did
        let again = pages.read(&read_of(&format!("{address}/chapter/2"), steps, 30)).expect("the page is read again");
        assert!(again.contains("img/30.png"), "{again}");

        let asked = asked.lock().unwrap().clone();
        assert!(asked.iter().any(|path| path == "/chapter/1") && asked.iter().any(|path| path == "/chapter/2"), "{asked:?}");
        assert!(!asked.iter().any(|path| path.starts_with("/img/")), "no picture was loaded: {asked:?}");
        assert!(outside_asked.lock().unwrap().is_empty(), "a host outside the list was asked: {:?}", outside_asked.lock().unwrap());
    }

    #[test]
    fn a_read_that_waits_for_nothing_ends_at_its_time_and_says_where_the_page_is() {
        use crate::hidden_pages::Step;
        let (address, _asked) = reader_site("http://127.0.0.1:1");
        let data = tempfile::tempdir().unwrap();
        let pages = WebView2Pages::new(data.path().to_path_buf());
        let started = Instant::now();
        let error = pages.read(&read_of(&format!("{address}/"), vec![Step::Wait("ul.chapters > li".into())], 2)).expect_err("nothing matches");
        assert!(error.contains("nothing matched ul.chapters > li"), "{error}");
        assert!(error.contains(r#"the page is "A site" at "#) && error.contains(&address), "{error}");
        assert!(started.elapsed() < Duration::from_secs(90), "the read ended at its own time, not the caller's");
        // the page is free again afterwards
        let answer = pages.read(&read_of(&format!("{address}/"), vec![Step::Read("body".into())], 10)).expect("the next read works");
        assert!(answer.contains("front"), "{answer}");
        // a page that is read does not go to a host outside its list
        let refused = pages.read(&PageRead { url: format!("{}/", address.replace("127.0.0.1", "localhost")), hosts: vec!["127.0.0.1".into()], steps: vec![Step::Read("body".into())], timeout: Duration::from_secs(5) });
        assert!(refused.is_ok_and(|answer| !answer.contains("front")), "the other host's page was not shown");
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
