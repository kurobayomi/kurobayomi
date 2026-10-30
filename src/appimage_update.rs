//! The app updates itself, on both channels, when it runs as an AppImage or as a copy the Windows setup installed
//! (`kuro_platform::self_update::Package`). At start it reads the release its channel follows, and when that is newer it
//! downloads the new file and checks it. An AppImage is moved into place at once; a Windows setup runs when the app quits
//! (or at Restart), since a running program cannot replace its own files there. The next launch runs the new build.
//! - A nightly build knows its commit (`channel::commit`, set by the workflow) and follows the `nightly` release: newer is
//!   a build from another commit.
//! - A stable build with the release check (`app_update::available`) follows the latest release: newer is a higher
//!   version.
//!
//! Any other build (a local one, a package) has no `APPIMAGE`, no commit or no release check, and never updates itself.
//!
//! The download can be paused and cancelled from the About page (`pause`, `resume`, `cancel`), and it reads the file
//! from the fastest of GitHub's download addresses (`fastest_route`), raced again for every connection: the first, one
//! after each pause, and one after each connection that breaks off (`RECONNECTS` times per download).

use kuro_i18n::{Arg, MsgKey, tr, trf};
use std::io::{Read, Write};
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use kuro_platform::self_update::Package;
use sha2::{Digest, Sha256};

const USER_AGENT: &str = concat!("kurobayomi/", env!("CARGO_PKG_VERSION"));

/// The releases a build follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Track {
    /// The `nightly` release; the commit of the running build.
    Nightly(&'static str),
    /// The latest release; the version of the running build.
    Stable(&'static str),
}

impl Track {
    /// The track of this build, when it can update itself.
    pub fn current() -> Option<Self> {
        match crate::channel::commit() {
            Some(commit) => Some(Self::Nightly(commit)),
            None if !crate::channel::NIGHTLY && crate::app_update::available() => Some(Self::Stable(env!("CARGO_PKG_VERSION"))),
            None => None,
        }
    }

    fn release_path(self) -> &'static str {
        match self {
            Self::Nightly(_) => "releases/tags/nightly",
            Self::Stable(_) => "releases/latest",
        }
    }

    /// The release file of this track for `package`: the AppImage, or the Windows setup.
    fn asset(self, package: &Package) -> &'static str {
        match (package, self) {
            (Package::AppImage(_), Self::Nightly(_)) => "Kurobayomi-nightly-x86_64.AppImage",
            (Package::AppImage(_), Self::Stable(_)) => "Kurobayomi-x86_64.AppImage",
            (Package::Installer, Self::Nightly(_)) => "Kurobayomi-Nightly-Setup-x86_64.exe",
            (Package::Installer, Self::Stable(_)) => "Kurobayomi-Setup-x86_64.exe",
        }
    }
}

/// A newer release, and the file to take.
#[derive(Debug, PartialEq, Eq)]
struct Newer {
    /// The commit of a nightly, the version of a stable release (its tag without the `v`).
    build: String,
    url: String,
    size: u64,
    sha256: Option<String>,
    /// The release's `SHA256SUMS`, read when GitHub gives no digest for the file.
    sums_url: Option<String>,
}

/// The 40-character commit in the release notes ("Built from commit <hash>.").
fn commit_in(notes: &str) -> Option<String> {
    let hash: String = notes.split("commit ").nth(1)?.chars().take_while(char::is_ascii_hexdigit).collect();
    (hash.len() == 40).then_some(hash)
}

/// `Some` when the release JSON is newer than the running build and carries the file `asset`.
fn newer(release: &[u8], track: Track, asset: &str) -> Option<Newer> {
    let json: serde_json::Value = serde_json::from_slice(release).ok()?;
    let build = match track {
        Track::Nightly(current) => Some(commit_in(json["body"].as_str()?)?).filter(|commit| commit != current)?,
        Track::Stable(current) => {
            let tag = json["tag_name"].as_str()?;
            kuro_domain::release::version::is_newer(tag, current).then(|| tag.trim_start_matches('v').to_owned())?
        }
    };
    let asset = json["assets"].as_array()?.iter().find(|entry| entry["name"] == asset)?;
    Some(Newer {
        build,
        url: asset["browser_download_url"].as_str()?.to_owned(),
        size: asset["size"].as_u64()?,
        sha256: asset["digest"].as_str().and_then(|digest| digest.strip_prefix("sha256:")).map(str::to_owned),
        sums_url: json["assets"]
            .as_array()?
            .iter()
            .find(|asset| asset["name"] == "SHA256SUMS")
            .and_then(|asset| asset["browser_download_url"].as_str())
            .map(str::to_owned),
    })
}

/// The hash `SHA256SUMS` lists for `name` (`<hash>  <name>` lines).
fn sum_of(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        (file.trim_start().trim_start_matches('*') == name).then(|| hash.to_owned())
    })
}

/// Downloads the update beside `target` and checks its size, its hash (when GitHub gives one) and that it is the
/// package's kind of file ([`header`]).
/// What the Pause and Cancel buttons ask of the running download; the download looks at it between reads.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Wish {
    #[default]
    Run,
    Pause,
    Cancel,
}

#[derive(Default)]
struct Control {
    wish: Mutex<Wish>,
    changed: Condvar,
}

/// The error of a download the user cancelled: not a failure, so the status goes back to idle without a message.
const CANCELLED: &str = "cancelled";

impl Control {
    fn wish(&self) -> Wish {
        *self.wish.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn ask(&self, wish: Wish) {
        *self.wish.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = wish;
        self.changed.notify_all();
    }

    /// Waits while the download is paused; an error once it is cancelled.
    fn hold(&self) -> Result<(), String> {
        let mut wish = self.wish.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        while *wish == Wish::Pause {
            wish = self.changed.wait(wish).unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        if *wish == Wish::Cancel { Err(CANCELLED.to_owned()) } else { Ok(()) }
    }
}

/// The control of the download that runs now, for the buttons.
static ACTIVE: Mutex<Option<Arc<Control>>> = Mutex::new(None);

fn active() -> Option<Arc<Control>> {
    ACTIVE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
}

/// Pauses the running download. The connection is closed and the file kept: resuming asks GitHub for the rest.
pub fn pause() {
    if let (Some(control), Status::Downloading { build, done, total }) = (active(), status()) {
        control.ask(Wish::Pause);
        set(Status::Paused { build, done, total });
    }
}

/// Carries on a paused download from where it stopped.
pub fn resume() {
    if let (Some(control), Status::Paused { build, done, total }) = (active(), status()) {
        control.ask(Wish::Run);
        set(Status::Downloading { build, done, total });
    }
}

/// Stops the running download and deletes what was downloaded. The AppImage in use is never touched.
pub fn cancel() {
    if let Some(control) = active() {
        control.ask(Wish::Cancel);
    }
}

/// How much of a file each address is asked for when the addresses are raced.
const PROBE_BYTES: u64 = 256 * 1024;

/// GitHub serves a release file from several download addresses, and from some connections one of them is far slower
/// than the rest (47 KB/s against 1.6 MB/s on one home line, and always the same one: it is the first address, which
/// is the one an ordinary client tries). This follows the redirect to the file's host, asks every address of that host
/// for the first [`PROBE_BYTES`] at once, and gives a client pinned to the first to answer, with the redirected url.
/// When there is no redirect, one address or no answer it gives the client and url it was given.
fn fastest_route(client: &reqwest::blocking::Client, url: &str) -> (reqwest::blocking::Client, String) {
    let Some(target) = redirect_target(url) else { return (client.clone(), url.to_owned()) };
    let plain = || (client.clone(), target.to_string());
    let Some(host) = target.host_str().filter(|host| host.parse::<std::net::IpAddr>().is_err()) else { return plain() };
    let port = target.port_or_known_default().unwrap_or(443);
    let addrs: Vec<SocketAddr> = (host, port).to_socket_addrs().map(Iterator::collect).unwrap_or_default();
    if addrs.len() < 2 {
        return plain();
    }
    let Some(fastest) = fastest_address(host, target.as_str(), &addrs) else { return plain() };
    let pinned = reqwest::blocking::Client::builder().resolve(host, fastest).timeout(Duration::from_secs(900)).user_agent(USER_AGENT).build();
    pinned.map_or_else(|_| plain(), |client| (client, target.to_string()))
}

/// Where `url` redirects to, `None` when it does not.
fn redirect_target(url: &str) -> Option<reqwest::Url> {
    let client = reqwest::blocking::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(30)).user_agent(USER_AGENT).build().ok()?;
    let response = client.get(url).send().ok()?;
    if !response.status().is_redirection() {
        return None;
    }
    let location = response.headers().get(reqwest::header::LOCATION)?.to_str().ok()?;
    reqwest::Url::parse(url).ok()?.join(location).ok()
}

/// The first of `addrs` that gives the start of `url` (served by `host`); the others are left to give up on their own
/// after a few seconds. `None` when none does in time.
fn fastest_address(host: &str, url: &str, addrs: &[SocketAddr]) -> Option<SocketAddr> {
    let (tx, rx) = std::sync::mpsc::channel();
    for &addr in addrs {
        let (tx, host, url) = (tx.clone(), host.to_owned(), url.to_owned());
        std::thread::spawn(move || {
            let read_start = || -> Option<()> {
                let client = reqwest::blocking::Client::builder().resolve(&host, addr).timeout(Duration::from_secs(8)).user_agent(USER_AGENT).build().ok()?;
                let response = client.get(&url).header(reqwest::header::RANGE, format!("bytes=0-{}", PROBE_BYTES - 1)).send().ok()?.error_for_status().ok()?;
                let mut got = Vec::new();
                response.take(PROBE_BYTES).read_to_end(&mut got).ok()?;
                (!got.is_empty()).then_some(())
            };
            if read_start().is_some() {
                let _ = tx.send(addr);
            }
        });
    }
    drop(tx);
    rx.recv_timeout(Duration::from_secs(10)).ok()
}

fn download(client: &reqwest::blocking::Client, update: &Newer, package: &Package, target: &Path, progress: &dyn Fn(u64), control: &Control) -> Result<PathBuf, String> {
    let mut name = target.as_os_str().to_owned();
    name.push(".update");
    let part = PathBuf::from(name);
    let result = fetch_checked(client, update, package, &part, progress, control);
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result.map(|()| part)
}

/// The first bytes of the package's kind of file, and what the file is called when they differ: an ELF program (the
/// AppImage), or a Windows program (`MZ`, the setup).
fn header(package: &Package) -> (&'static [u8], &'static str) {
    match package {
        Package::AppImage(_) => (b"\x7fELF", "the downloaded file is not an AppImage"),
        Package::Installer => (b"MZ", "the downloaded file is not a Windows setup"),
    }
}

/// How many broken connections one download replaces before it gives up.
const RECONNECTS: u32 = 3;

fn fetch_checked(client: &reqwest::blocking::Client, update: &Newer, package: &Package, part: &Path, progress: &dyn Fn(u64), control: &Control) -> Result<(), String> {
    let mut file = std::fs::File::create(part).map_err(|e| e.to_string())?;
    let (mut hasher, mut total, mut buf) = (Sha256::new(), 0_u64, [0_u8; 64 * 1024]);
    // a connection that breaks off is replaced by a new one, from a fresh race for the fastest address, that asks for
    // the rest of the file; a few times, then the error stands
    let mut reconnects = RECONNECTS;
    // one pass per connection: the first, a new one after each pause, and one after each break, asking for the rest
    loop {
        control.hold()?;
        let (client, url) = fastest_route(client, &update.url);
        let mut request = client.get(&url);
        if total > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={total}-"));
        }
        let mut response = match request.send().and_then(|r| r.error_for_status()) {
            Ok(response) => response,
            // an answer like 404 stays the same on another address; a connection that failed may not
            Err(e) if e.status().is_none() && reconnects > 0 => {
                reconnects -= 1;
                continue;
            }
            Err(e) => return Err(describe(&e)),
        };
        if total > 0 && response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(tr(MsgKey::ui_github_would_not_carry_on_the_download).to_owned());
        }
        let mut finished = false;
        while control.wish() == Wish::Run {
            let n = match response.read(&mut buf) {
                Ok(n) => n,
                Err(_) if reconnects > 0 => {
                    reconnects -= 1;
                    break;
                }
                Err(e) => return Err(format!("the download from GitHub broke off ({e})")),
            };
            if n == 0 {
                finished = true;
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            total += n as u64;
            progress(total);
        }
        if finished {
            break;
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    let hash: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if total != update.size {
        return Err(format!("downloaded {total} bytes, expected {}", update.size));
    }
    if update.sha256.as_deref().is_some_and(|expected| !expected.eq_ignore_ascii_case(&hash)) {
        return Err("the downloaded file does not match its checksum".to_owned());
    }
    let (expected, wrong) = header(package);
    let mut magic = vec![0_u8; expected.len()];
    std::fs::File::open(part).and_then(|mut file| file.read_exact(&mut magic)).map_err(|e| e.to_string())?;
    if magic != expected {
        return Err(wrong.to_owned());
    }
    Ok(())
}

const GITHUB_API: &str = "https://api.github.com";

/// A failed request in words: what went wrong, then the deepest cause it gives ("could not reach GitHub (dns error:
/// failed to lookup address information)"), where reqwest alone says only "error sending request for url".
fn describe(error: &reqwest::Error) -> String {
    let what = if error.is_timeout() {
        tr(MsgKey::ui_github_did_not_answer_in_time).to_owned()
    } else if let Some(status) = error.status() {
        trf(MsgKey::update_github_answered, &[("status", Arg::Str(&status.to_string()))])
    } else if error.is_connect() || error.is_request() {
        tr(MsgKey::update_could_not_reach_github)
    } else {
        tr(MsgKey::update_download_broke_off)
    };
    let mut cause = None;
    let mut source = std::error::Error::source(error);
    while let Some(inner) = source {
        cause = Some(inner.to_string());
        source = inner.source();
    }
    match cause {
        Some(cause) if !error.is_status() => format!("{what} ({cause})"),
        _ => what,
    }
}

/// What the worker tells the main loop while it updates.
#[derive(Debug)]
enum Step {
    /// A newer build was found; its download of `size` bytes starts.
    Found { build: String, size: u64 },
    /// Bytes downloaded so far.
    Bytes(u64),
}

/// One check against the releases at `api` and, when there is a newer release, the download to `target`: the AppImage,
/// which is replaced, or the Windows setup, which waits as [`PENDING_SETUP`]. `Some(build)` when the update is in place
/// or waiting. `step` hears what happens along the way.
fn update(package: &Package, target: &Path, track: Track, api: &str, step: &dyn Fn(Step), control: &Control) -> Result<Option<String>, String> {
    let http = |timeout| reqwest::blocking::Client::builder().timeout(Duration::from_secs(timeout)).user_agent(USER_AGENT).build().map_err(|e| e.to_string());
    let url = format!("{api}/repos/{}/{}", crate::channel::REPOSITORY, track.release_path());
    let response = http(30)?.get(url).send().map_err(|e| describe(&e))?;
    // no release while the workflow is replacing the nightly, or before the first stable one
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let body = response.error_for_status().and_then(|r| r.bytes()).map_err(|e| describe(&e))?;
    let asset = track.asset(package);
    let Some(mut newer) = newer(&body, track, asset) else { return Ok(None) };
    if newer.sha256.is_none()
        && let Some(url) = &newer.sums_url
    {
        let sums = http(30)?.get(url).send().and_then(|r| r.error_for_status()).and_then(|r| r.text()).map_err(|e| describe(&e))?;
        newer.sha256 = Some(sum_of(&sums, asset).ok_or(tr(MsgKey::ui_sha256sums_does_not_list_the_appimage))?);
    }
    step(Step::Found { build: newer.build.clone(), size: newer.size });
    // the setup's folder (the app's temporary folder) may not exist yet; an AppImage's folder always does
    if let Some(folder) = target.parent() {
        std::fs::create_dir_all(folder).map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
    }
    let part = download(&http(900)?, &newer, package, target, &|bytes| step(Step::Bytes(bytes)), control)?;
    kuro_platform::self_update::replace_file(&part, target).map_err(|e| e.to_string())?;
    if *package == Package::Installer {
        *PENDING_SETUP.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(target.to_path_buf());
    }
    Ok(Some(newer.build))
}

/// Where the update goes: over the AppImage, or the setup file in the app's temporary folder.
fn target(package: &Package, track: Track) -> Option<PathBuf> {
    match package {
        Package::AppImage(file) => Some(file.clone()),
        Package::Installer => {
            let paths = kuro_platform::system_paths::SystemPaths::system().ok()?;
            Some(kuro_platform::paths::Paths::temp_dir(&paths).join(track.asset(package)))
        }
    }
}

/// The downloaded Windows setup that installs the update when the app quits, or at Restart.
static PENDING_SETUP: Mutex<Option<PathBuf>> = Mutex::new(None);

fn take_pending_setup() -> Option<PathBuf> {
    PENDING_SETUP.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take()
}

/// Whether the setup starts the app again when it is done: Restart was pressed.
static RELAUNCH_AFTER_SETUP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// At quit: a downloaded Windows setup installs the update now, so the next launch runs the new build. It runs here, at
/// the very end of shutdown, also for Restart: started earlier, it had to close an app still saving its settings (or still
/// asking whether to quit while downloads run), and when that failed the app never came back.
pub fn on_quit() {
    let relaunch = RELAUNCH_AFTER_SETUP.load(std::sync::atomic::Ordering::Relaxed);
    if let Some(setup) = take_pending_setup()
        && let Err(error) = kuro_platform::self_update::run_installer(&setup, relaunch)
    {
        tracing::warn!(%error, "the downloaded update could not be installed");
    }
}

/// Starts the new build once the app has quit: the AppImage starts again, or the Windows setup installs the update at quit
/// ([`on_quit`]) and starts it. `false` when there is nothing to start; the caller then closes the window.
pub fn restart_into_update() -> bool {
    match kuro_platform::self_update::package() {
        Some(Package::AppImage(file)) => kuro_platform::self_update::relaunch_after_exit(&file).is_ok(),
        Some(Package::Installer) => {
            let waiting = PENDING_SETUP.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).is_some();
            RELAUNCH_AFTER_SETUP.store(waiting, std::sync::atomic::Ordering::Relaxed);
            waiting
        }
        None => false,
    }
}

/// Whether a downloaded update installs when the app closes (the Windows setup) rather than being in place already (the
/// AppImage): its notice then offers Install now instead of Restart, as installing takes a while.
pub fn installs_on_quit() -> bool {
    matches!(kuro_platform::self_update::package(), Some(Package::Installer))
}

/// Whether this run can update itself: an AppImage or an installed Windows copy, of a build that follows releases.
pub fn can_update() -> bool {
    kuro_platform::self_update::package().is_some() && Track::current().is_some()
}

/// Where the self-update is. The About page's Version row shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// Nothing checked in this run yet.
    #[default]
    Idle,
    Checking,
    UpToDate,
    Downloading { build: String, done: u64, total: u64 },
    /// The download was paused: the file is kept and resuming asks for the rest.
    Paused { build: String, done: u64, total: u64 },
    /// The new build replaced the AppImage; it runs after a restart.
    Installed(String),
    Failed(String),
}

impl Status {
    /// A check or a download runs.
    pub fn busy(&self) -> bool {
        matches!(self, Self::Checking | Self::Downloading { .. } | Self::Paused { .. })
    }

    /// Something is at work now: busy, and not waiting for the user to resume.
    pub fn working(&self) -> bool {
        self.busy() && !matches!(self, Self::Paused { .. })
    }

    /// The line the Version row shows under the version, if any.
    pub fn line(&self) -> Option<String> {
        let short = |build: &str| build[..8.min(build.len())].to_owned();
        match self {
            Self::Idle => None,
            Self::Checking => Some(tr(MsgKey::ui_checking_for_updates).to_owned()),
            Self::UpToDate => Some(tr(MsgKey::ui_up_to_date).to_owned()),
            Self::Downloading { build, done, total } => {
                let percent = (*done * 100).checked_div(*total).unwrap_or(0);
                Some(trf(MsgKey::update_downloading, &[("build", Arg::Str(&short(build))), ("percent", Arg::Int(i64::try_from(percent).unwrap_or(0)))]))
            }
            Self::Paused { build, done, total } => {
                let percent = (*done * 100).checked_div(*total).unwrap_or(0);
                Some(trf(MsgKey::update_paused, &[("build", Arg::Str(&short(build))), ("percent", Arg::Int(i64::try_from(percent).unwrap_or(0)))]))
            }
            Self::Installed(build) => {
                let key = if installs_on_quit() { MsgKey::update_ready_install } else { MsgKey::update_installed_restart };
                Some(trf(key, &[("build", Arg::Str(&short(build)))]))
            }
            Self::Failed(error) => Some(trf(MsgKey::update_failed, &[("error", Arg::Str(error))])),
        }
    }

    /// How far the download is, from 0 to 1, while one runs.
    pub fn fraction(&self) -> Option<f64> {
        match self {
            Self::Downloading { done, total, .. } | Self::Paused { done, total, .. } if *total > 0 => Some((*done as f64 / *total as f64).min(1.0)),
            _ => None,
        }
    }
}

thread_local! {
    static STATUS: std::cell::RefCell<Status> = std::cell::RefCell::new(Status::Idle);
    /// Called on every change; a watcher whose widget is gone returns `false` and is dropped.
    static WATCHERS: std::cell::RefCell<Vec<Box<dyn Fn(&Status) -> bool>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The self-update now.
pub fn status() -> Status {
    STATUS.with(|s| s.borrow().clone())
}

/// Calls `watch` now and on every change, until it returns `false`. Main thread only.
pub fn watch(watch: impl Fn(&Status) -> bool + 'static) {
    if watch(&status()) {
        WATCHERS.with(|w| w.borrow_mut().push(Box::new(watch)));
    }
}

fn set(next: Status) {
    STATUS.with(|s| *s.borrow_mut() = next.clone());
    let watchers = WATCHERS.with(|w| std::mem::take(&mut *w.borrow_mut()));
    let kept: Vec<_> = watchers.into_iter().filter(|watch| watch(&next)).collect();
    WATCHERS.with(|w| {
        let mut w = w.borrow_mut();
        let added = std::mem::take(&mut *w);
        *w = kept;
        w.extend(added);
    });
}

/// A small spinner that turns while the self-update checks or downloads, for any place that shows it is busy (the
/// sidebar's Settings row and the About row of Settings).
pub fn busy_spinner() -> gtk::Spinner {
    let spinner = crate::ui::loading(crate::ui::LoadSize::Inline);
    let weak = spinner.downgrade();
    watch(move |status| {
        let Some(spinner) = weak.upgrade() else { return false };
        spinner.set_visible(status.working());
        true
    });
    spinner
}

/// Checks and, when there is a newer release, updates, on a worker thread; the [`Status`] follows each step. Does
/// nothing while a check runs, after an install, or when this run cannot update itself. A new build shows the
/// installed toast.
pub fn check() {
    let status = status();
    if status.busy() || matches!(status, Status::Installed(_)) {
        return;
    }
    let (Some(package), Some(track)) = (kuro_platform::self_update::package(), Track::current()) else { return };
    let Some(target) = target(&package, track) else { return };
    set(Status::Checking);
    let control = Arc::new(Control::default());
    *ACTIVE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(control.clone());
    let (tx, rx) = async_channel::unbounded();
    let worker_control = control.clone();
    std::thread::spawn(move || {
        // a step per 1% at most, so the main loop is not flooded
        let last = std::cell::Cell::new((String::new(), 0_u64, 0_u64));
        let step = |step: Step| {
            let (build, size, shown) = last.take();
            let next = match step {
                Step::Found { build, size } => Status::Downloading { build: build.clone(), done: 0, total: size },
                Step::Bytes(done) if size > 0 && (done - shown) * 100 < size && done < size => {
                    last.set((build, size, shown));
                    return;
                }
                Step::Bytes(done) => Status::Downloading { build: build.clone(), done, total: size },
            };
            if let Status::Downloading { build, done, total } = &next {
                last.set((build.clone(), *total, *done));
            }
            let _ = tx.send_blocking(Ok(next));
        };
        let result = update(&package, &target, track, GITHUB_API, &step, &worker_control);
        // before the last message, so a check started by it finds no control of this run
        *ACTIVE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        let _ = tx.send_blocking(Err(result));
    });
    glib::spawn_future_local(async move {
        while let Ok(message) = rx.recv().await {
            match message {
                // a step queued before Pause or Cancel was pressed must not bring the download back on screen
                Ok(Status::Downloading { .. }) if control.wish() != Wish::Run => {}
                Ok(progress) => set(progress),
                Err(Ok(Some(build))) => {
                    set(Status::Installed(build.clone()));
                    announce(build);
                }
                Err(Ok(None)) => set(Status::UpToDate),
                Err(Err(error)) if error == CANCELLED => set(Status::Idle),
                Err(Err(error)) => {
                    tracing::warn!(%error, "the AppImage update failed");
                    set(Status::Failed(error));
                }
            }
        }
    });
}

/// The installed toast of a replaced AppImage.
pub fn announce(build: String) {
    match Track::current() {
        Some(Track::Nightly(_)) => crate::whats_new::nightly_installed(build),
        _ => crate::whats_new::stable_installed(build),
    }
}

/// Whether the user lets the app look for updates at start (`check_app_updates`, on by default).
pub fn check_at_startup() -> bool {
    !crate::data::is_installed() || crate::data::app().base.check_app_updates.get()
}

/// The startup check, when the user lets it run. On a metered connection (a phone hotspot) it does not run: the AppImage
/// is large, and About's "Check for updates" still downloads it when asked.
pub fn startup() {
    use gtk::gio::prelude::NetworkMonitorExt as _;
    if !check_at_startup() {
        return;
    }
    if gtk::gio::NetworkMonitor::default().is_network_metered() {
        tracing::info!("metered connection: the startup update check is skipped");
        return;
    }
    check();
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "1111111111111111111111111111111111111111";
    const NEW: &str = "2222222222222222222222222222222222222222";

    const NIGHTLY: &str = "Kurobayomi-nightly-x86_64.AppImage";
    const STABLE: &str = "Kurobayomi-x86_64.AppImage";

    fn release(commit: &str, asset: &str) -> Vec<u8> {
        serde_json::json!({
            "tag_name": "nightly",
            "body": format!("Built from commit {commit}."),
            "assets": [{ "name": asset, "browser_download_url": "https://example.test/a", "size": 7, "digest": "sha256:abc" }]
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn a_release_from_another_commit_is_an_update() {
        let found = newer(&release(NEW, NIGHTLY), Track::Nightly(OLD), NIGHTLY).expect("an update");
        assert_eq!(found, Newer { build: NEW.into(), url: "https://example.test/a".into(), size: 7, sha256: Some("abc".into()), sums_url: None });
    }

    #[test]
    fn the_same_commit_a_missing_asset_or_odd_notes_are_no_update() {
        assert_eq!(newer(&release(OLD, NIGHTLY), Track::Nightly(OLD), NIGHTLY), None);
        assert_eq!(newer(&release(NEW, "Other.AppImage"), Track::Nightly(OLD), NIGHTLY), None);
        assert_eq!(newer(&release(NEW, STABLE), Track::Nightly(OLD), NIGHTLY), None, "a nightly takes only the nightly file");
        assert_eq!(newer(br#"{"body":"nothing here","assets":[]}"#, Track::Nightly(OLD), NIGHTLY), None);
        assert_eq!(newer(b"not json", Track::Nightly(OLD), NIGHTLY), None);
    }

    fn stable(tag: &str) -> Vec<u8> {
        let asset = |name: &str| serde_json::json!({ "name": name, "browser_download_url": format!("https://example.test/{name}"), "size": 9 });
        serde_json::json!({
            "tag_name": tag,
            "body": "## Added",
            "assets": [asset("Kurobayomi-x86_64.AppImage.zsync"), asset(STABLE), asset("SHA256SUMS")]
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn a_higher_stable_version_is_an_update_and_takes_the_appimage_not_its_zsync() {
        let found = newer(&stable("v0.2.0"), Track::Stable("0.1.0"), STABLE).expect("an update");
        assert_eq!(found.build, "0.2.0");
        assert_eq!(found.url, format!("https://example.test/{STABLE}"));
        assert_eq!((found.sha256, found.sums_url.as_deref()), (None, Some("https://example.test/SHA256SUMS")), "no digest: the sums file is read");
    }

    #[test]
    fn the_sums_file_names_the_hash_of_one_file() {
        let sums = "aaa  Kurobayomi-x86_64.AppImage.zsync\nbbb  Kurobayomi-x86_64.AppImage\nccc *Other\n";
        assert_eq!(sum_of(sums, STABLE).as_deref(), Some("bbb"));
        assert_eq!(sum_of(sums, "Other").as_deref(), Some("ccc"));
        assert_eq!(sum_of(sums, "Missing"), None);
    }

    #[test]
    fn the_same_or_an_older_stable_version_is_no_update() {
        assert_eq!(newer(&stable("v0.1.0"), Track::Stable("0.1.0"), STABLE), None);
        assert_eq!(newer(&stable("v0.0.9"), Track::Stable("0.1.0"), STABLE), None);
        assert_eq!(newer(&stable("nightly"), Track::Stable("0.1.0"), STABLE), None, "a tag that is no version is never newer");
    }

    #[test]
    fn a_track_reads_its_own_release_and_file() {
        let appimage = Package::AppImage(PathBuf::new());
        assert_eq!((Track::Nightly(OLD).release_path(), Track::Nightly(OLD).asset(&appimage)), ("releases/tags/nightly", NIGHTLY));
        assert_eq!((Track::Stable("0.1.0").release_path(), Track::Stable("0.1.0").asset(&appimage)), ("releases/latest", STABLE));
        assert_eq!(Track::Nightly(OLD).asset(&Package::Installer), "Kurobayomi-Nightly-Setup-x86_64.exe");
        assert_eq!(Track::Stable("0.1.0").asset(&Package::Installer), "Kurobayomi-Setup-x86_64.exe");
    }

    /// Serves the files `files` builds from the base url (path, body) over HTTP on a local port, one request per
    /// connection, and gives the base url.
    fn serve(files: impl FnOnce(&str) -> Vec<(String, Vec<u8>)>) -> String {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let files = files(&base);
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut line = String::new();
                BufReader::new(&stream).read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap_or_default();
                let (status, body) = files.iter().find(|(p, _)| p == path).map_or(("404 Not Found", &[][..]), |(_, b)| ("200 OK", &b[..]));
                let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(body);
            }
        });
        base
    }

    /// A latest release `tag` with the AppImage `app` and a SHA256SUMS that gives `app_hash` for it.
    fn stable_release(tag: &'static str, app: Vec<u8>, app_hash: String) -> impl FnOnce(&str) -> Vec<(String, Vec<u8>)> {
        stable_release_of(STABLE, tag, app, app_hash)
    }

    /// A stable release whose file is called `name`, with its line in SHA256SUMS.
    fn stable_release_of(name: &'static str, tag: &'static str, app: Vec<u8>, app_hash: String) -> impl FnOnce(&str) -> Vec<(String, Vec<u8>)> {
        move |base| {
            let sums = format!("{app_hash}  {name}\n");
            let release = serde_json::json!({
                "tag_name": tag,
                "assets": [
                    { "name": name, "browser_download_url": format!("{base}/dl/app"), "size": app.len() },
                    { "name": "SHA256SUMS", "browser_download_url": format!("{base}/dl/sums"), "size": sums.len() }
                ]
            });
            vec![
                (format!("/repos/{}/releases/latest", crate::channel::REPOSITORY), release.to_string().into_bytes()),
                ("/dl/app".to_owned(), app),
                ("/dl/sums".to_owned(), sums.into_bytes()),
            ]
        }
    }

    fn update_quiet(file: &Path, track: Track, api: &str) -> Result<Option<String>, String> {
        update(&Package::AppImage(file.to_path_buf()), file, track, api, &|_| {}, &Control::default())
    }

    #[test]
    fn the_status_line_follows_the_update() {
        assert_eq!(Status::Idle.line(), None);
        let downloading = Status::Downloading { build: NEW.into(), done: 42, total: 100 };
        assert_eq!(downloading.line().as_deref(), Some("Downloading 22222222: 42%"));
        assert_eq!(downloading.fraction(), Some(0.42));
        assert!(downloading.busy() && Status::Checking.busy() && !Status::UpToDate.busy());
        assert_eq!(Status::Installed("0.2.0".into()).line().as_deref(), Some("0.2.0 is installed: restart to use it"));
        assert_eq!(Status::Downloading { build: "x".into(), done: 1, total: 0 }.fraction(), None);
    }

    #[test]
    fn a_paused_download_keeps_its_bar_and_is_busy_but_not_working() {
        let paused = Status::Paused { build: NEW.into(), done: 42, total: 100 };
        assert_eq!(paused.line().as_deref(), Some("Paused 22222222: 42%"));
        assert_eq!(paused.fraction(), Some(0.42));
        assert!(paused.busy() && !paused.working());
        assert!(Status::Checking.working() && Status::Downloading { build: NEW.into(), done: 1, total: 2 }.working());
    }

    /// Serves `body` at `/file` over HTTP on 127.0.0.1, in four pieces a little apart, and answers `Range: bytes=N-` with
    /// the rest of it (206), as GitHub's download servers do. One thread per connection.
    fn serve_ranged(body: Vec<u8>) -> String {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let body = body.clone();
                std::thread::spawn(move || {
                    let mut stream = stream;
                    let mut start = 0_usize;
                    let mut lines = BufReader::new(&stream).lines();
                    while let Some(Ok(line)) = lines.next() {
                        if line.is_empty() {
                            break;
                        }
                        if let Some(range) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                            start = range.split('-').next().and_then(|n| n.parse().ok()).unwrap_or(0);
                        }
                    }
                    let rest = &body[start.min(body.len())..];
                    let status = if start > 0 { "206 Partial Content" } else { "200 OK" };
                    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", rest.len());
                    for piece in rest.chunks(rest.len().div_ceil(4).max(1)) {
                        if stream.write_all(piece).is_err() {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                });
            }
        });
        base
    }

    fn newer_for(base: &str, body: &[u8]) -> Newer {
        Newer { build: NEW.into(), url: format!("{base}/file"), size: body.len() as u64, sha256: Some(hex(body)), sums_url: None }
    }

    fn client() -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder().timeout(Duration::from_secs(30)).build().unwrap()
    }

    #[test]
    fn a_paused_download_waits_and_resumes_where_it_stopped() {
        let mut body = b"\x7fELF".to_vec();
        body.extend((0..400_000_u32).map(|n| (n % 251) as u8));
        let base = serve_ranged(body.clone());
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app.AppImage");
        let control = Control::default();
        let paused_at = std::sync::atomic::AtomicU64::new(0);
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let progress = |bytes: u64| {
                    if paused_at.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                        paused_at.store(bytes, std::sync::atomic::Ordering::SeqCst);
                        control.ask(Wish::Pause);
                    }
                };
                download(&client(), &newer_for(&base, &body), &Package::AppImage(target.clone()), &target, &progress, &control)
            });
            while paused_at.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                std::thread::sleep(Duration::from_millis(5));
            }
            std::thread::sleep(Duration::from_millis(300));
            let part = dir.path().join("app.AppImage.update");
            let kept = std::fs::metadata(&part).unwrap().len();
            assert_eq!(kept, paused_at.load(std::sync::atomic::Ordering::SeqCst), "nothing is read while paused");
            assert!(kept < body.len() as u64);
            control.ask(Wish::Run);
            let done = worker.join().unwrap().expect("the download finishes after the resume");
            assert_eq!(std::fs::read(done).unwrap(), body, "the pieces before and after the pause make the whole file");
        });
    }

    #[test]
    fn a_cancelled_download_deletes_its_file_and_is_not_a_failure_message() {
        let body: Vec<u8> = vec![7; 400_000];
        let base = serve_ranged(body.clone());
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app.AppImage");
        let control = Control::default();
        let result = download(&client(), &newer_for(&base, &body), &Package::AppImage(target.clone()), &target, &|_| control.ask(Wish::Cancel), &control);
        assert_eq!(result, Err(CANCELLED.to_owned()));
        assert!(!dir.path().join("app.AppImage.update").exists());
    }

    #[test]
    fn a_cancel_while_paused_ends_the_wait() {
        let control = Control::default();
        control.ask(Wish::Pause);
        std::thread::scope(|scope| {
            let waiting = scope.spawn(|| control.hold());
            std::thread::sleep(Duration::from_millis(100));
            control.ask(Wish::Cancel);
            assert_eq!(waiting.join().unwrap(), Err(CANCELLED.to_owned()));
        });
    }

    #[test]
    fn the_fastest_address_wins_and_a_dead_one_loses() {
        // one host name, three loopback addresses on one port: a slow server, a fast one, and one nobody listens on
        let fast = std::net::TcpListener::bind("127.0.0.2:0").unwrap();
        let port = fast.local_addr().unwrap().port();
        let slow = std::net::TcpListener::bind(("127.0.0.3", port)).unwrap();
        // the slow one never answers in time (its probe gives up at 8 s): a fixed head start lost on a busy CI runner,
        // where building each probe's client took longer than the slow server's delay
        for (listener, delay) in [(fast, 0), (slow, 60_000)] {
            std::thread::spawn(move || {
                for mut stream in listener.incoming().flatten() {
                    std::thread::spawn(move || {
                        // read the whole request first: closing a socket with unread data resets the connection, and the
                        // reset could reach the probe before the answer did (the fast server then lost, or nobody won)
                        use std::io::{BufRead, BufReader};
                        let mut reader = BufReader::new(&stream);
                        let mut line = String::new();
                        while reader.read_line(&mut line).is_ok_and(|n| n > 0) && line != "\r\n" {
                            line.clear();
                        }
                        std::thread::sleep(Duration::from_millis(delay));
                        let _ = write!(stream, "HTTP/1.1 206 Partial Content\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabcd");
                    });
                }
            });
        }
        let addrs: Vec<SocketAddr> = ["127.0.0.1", "127.0.0.3", "127.0.0.2"].iter().map(|ip| SocketAddr::new(ip.parse().unwrap(), port)).collect();
        let url = format!("http://files.test:{port}/file");
        assert_eq!(fastest_address("files.test", &url, &addrs), Some(SocketAddr::new("127.0.0.2".parse().unwrap(), port)));
        assert_eq!(fastest_address("files.test", &url, &addrs[..1]), None, "no server at 127.0.0.1: no address is better than none");
    }

    #[test]
    fn a_url_that_does_not_redirect_is_used_as_it_is() {
        let base = serve(|_| vec![("/file".to_owned(), b"abc".to_vec())]);
        let url = format!("{base}/file");
        assert_eq!(fastest_route(&client(), &url).1, url);
    }

    #[test]
    fn a_watcher_hears_each_change_until_it_lets_go() {
        let heard = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let h = heard.clone();
        watch(move |status| {
            h.borrow_mut().push(status.clone());
            !matches!(status, Status::UpToDate)
        });
        set(Status::Checking);
        set(Status::UpToDate);
        set(Status::Idle);
        assert_eq!(*heard.borrow(), [Status::Idle, Status::Checking, Status::UpToDate], "dropped after it returned false");
    }

    fn hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The whole stable update over HTTP: the release, its SHA256SUMS, the download, the checks and the swap.
    #[test]
    fn a_stable_appimage_replaces_itself_with_a_newer_release_and_refuses_a_bad_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(STABLE);
        std::fs::write(&file, b"\x7fELF old").unwrap();
        let new = b"\x7fELF new build".to_vec();
        let track = Track::Stable("0.1.0");

        assert_eq!(update_quiet(&file, track, &serve(|_| Vec::new())), Ok(None), "no stable release yet");
        assert_eq!(update_quiet(&file, track, &serve(stable_release("v0.1.0", new.clone(), hex(&new)))), Ok(None), "the same version");

        let error = update_quiet(&file, track, &serve(stable_release("v0.2.0", new.clone(), hex(b"other")))).unwrap_err();
        assert!(error.contains("checksum"), "{error}");
        assert_eq!(std::fs::read(&file).unwrap(), b"\x7fELF old", "a file that fails its sum leaves the AppImage alone");
        assert!(!dir.path().join(format!("{STABLE}.update")).exists(), "and is removed");

        let bad_file = b"not an elf".to_vec();
        let error = update_quiet(&file, track, &serve(stable_release("v0.2.0", bad_file.clone(), hex(&bad_file)))).unwrap_err();
        assert!(error.contains("not an AppImage"), "{error}");

        assert_eq!(update_quiet(&file, track, &serve(stable_release("v0.2.0", new.clone(), hex(&new)))), Ok(Some("0.2.0".to_owned())));
        assert_eq!(std::fs::read(&file).unwrap(), new);
    }

    /// A Windows copy: the setup is downloaded to its target, checked as a Windows program and kept for the quit.
    #[test]
    fn a_windows_copy_downloads_the_setup_and_keeps_it_for_the_quit() {
        const SETUP: &str = "Kurobayomi-Setup-x86_64.exe";
        let dir = tempfile::tempdir().unwrap();
        // the app's temporary folder, not made yet on a fresh install
        let target = dir.path().join("cache").join("tmp").join(SETUP);
        let track = Track::Stable("0.1.0");
        let run = |body: Vec<u8>| {
            let hash = hex(&body);
            update(&Package::Installer, &target, track, &serve(stable_release_of(SETUP, "v0.2.0", body, hash)), &|_| {}, &Control::default())
        };
        let error = run(b"\x7fELF an AppImage".to_vec()).unwrap_err();
        assert!(error.contains("not a Windows setup"), "{error}");
        assert!(!target.exists());

        assert_eq!(run(b"MZ new setup".to_vec()), Ok(Some("0.2.0".to_owned())));
        assert_eq!(std::fs::read(&target).unwrap(), b"MZ new setup");
        assert_eq!(take_pending_setup(), Some(target.clone()), "the setup waits for the quit");
        assert_eq!(take_pending_setup(), None, "and runs once");
    }

    #[test]
    fn a_failed_request_says_what_went_wrong_and_why() {
        // nothing listens on this port: the connection is refused
        let error = reqwest::blocking::Client::new().get("http://127.0.0.1:9/").send().unwrap_err();
        let text = describe(&error);
        assert!(text.starts_with("could not reach GitHub ("), "{text}");
        assert!(!text.contains("error sending request"), "the cause, not reqwest's summary: {text}");
        let base = serve(|_| Vec::new());
        let status = reqwest::blocking::get(format!("{base}/missing")).unwrap().error_for_status().unwrap_err();
        assert_eq!(describe(&status), "GitHub answered 404 Not Found");
    }
}
