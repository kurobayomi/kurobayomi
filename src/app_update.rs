//! The app release check on the GTK side (T-06a-070, T-07a-036, T-09a-042): the reqwest transport for the GitHub
//! client, the opt-in startup check that stays silent on failure, and the manual "Check for updates" action that
//! reports the error text. The throttle and the version rule are `kuro_domain::release::check`; no check time is stored,
//! so every call is un-throttled, and this file decides when the check may run at all.

use std::time::Duration;

use kuro_domain::release::check::{Outcome, check_for_update};
use kuro_domain::release::github::{GithubReleases, ReleaseHttp};
use kuro_domain::release::{ReleaseError, ReleaseService};
use kuro_i18n::{MsgKey, tr};


/// The GitHub `owner/name` that releases are read from (`UPDATE_REPOSITORY`). Empty turns the check off. A build
/// sets it with the `KB_UPDATE_REPOSITORY` environment variable.
pub const UPDATE_REPOSITORY: &str = match option_env!("KB_UPDATE_REPOSITORY") {
    Some(repository) => repository,
    None => "",
};

/// The title of the About row that starts a manual check (`spec/settings.json`, `settings_about`).
pub fn check_row_title() -> String {
    tr(MsgKey::check_for_updates)
}

const TIMEOUT: Duration = Duration::from_secs(30);

/// Whether the check exists in this build: the `updater` feature is on and a repository is set.
pub fn enabled(updater_feature: bool, repository: &str) -> bool {
    updater_feature && !repository.is_empty()
}

pub fn available() -> bool {
    enabled(kuro_core::app_info::APP_INFO.updater_enabled, UPDATE_REPOSITORY)
}

/// The About row is only listed when the check exists (`notes` of `settings_about`).
pub fn hides_row(title: &str) -> bool {
    title == check_row_title() && !available()
}

/// The GET behind [`GithubReleases`], over a blocking reqwest client. Built on the worker thread that runs the check.
struct Http(reqwest::blocking::Client);

impl Http {
    fn new() -> Result<Self, String> {
        reqwest::blocking::Client::builder().timeout(TIMEOUT).build().map(Self).map_err(|e| e.to_string())
    }
}

impl ReleaseHttp for Http {
    fn get(&self, url: &str, user_agent: &str) -> Result<(u16, Vec<u8>), String> {
        let response = self.0.get(url).header(reqwest::header::USER_AGENT, user_agent).send().map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let body = response.bytes().map_err(|e| e.to_string())?;
        Ok((status, body.to_vec()))
    }
}

/// One check at `now` (milliseconds). No check time is stored (the settings diet cut `last_app_check`), so the
/// throttle never applies; the startup check is off and the manual check forces anyway.
pub fn check(
    service: &dyn ReleaseService,
    repository: &str,
    current: &str,
    now: i64,
    force: bool,
) -> Result<Outcome, ReleaseError> {
    Ok(check_for_update(service, repository, current, 0, now, force)?.outcome)
}

/// Runs [`check`] over the real transport on a worker thread and hands the result to `deliver` on the main loop.
fn spawn_check(force: bool, deliver: impl FnOnce(Result<Outcome, ReleaseError>) + 'static) {
    crate::background::run(
        move || {
            Http::new().map_err(ReleaseError::Transport).and_then(|http| {
                let service = GithubReleases::new(http, std::env::consts::ARCH);
                check(&service, UPDATE_REPOSITORY, env!("CARGO_PKG_VERSION"), chrono::Utc::now().timestamp_millis(), force)
            })
        },
        deliver,
    );
}

/// The startup check: only when the build has the check and the user opted in. It is throttled, and any failure is
/// logged and otherwise silent. A newer release shows one toast. An AppImage updates itself instead
/// (`appimage_update::startup`).
pub fn startup() {
    if !available() || !crate::appimage_update::check_at_startup() || crate::appimage_update::can_update() {
        return;
    }
    spawn_check(false, |result| match result {
        Ok(Outcome::NewUpdate(release)) => crate::whats_new::stable_available(release),
        Ok(Outcome::NoNewUpdate) => {}
        Err(e) => tracing::debug!(error = %e, "the startup release check failed"),
    });
}

/// The manual check of the About row: it ignores the opt-in and the throttle, and says what happened, with the error
/// text when the lookup failed. An AppImage updates itself; another build opens a dialog with a button to the release.
pub fn manual() {
    if crate::appimage_update::can_update() {
        // the About page's Version row shows how it goes
        crate::appimage_update::check();
        return;
    }
    spawn_check(true, |result| match result {
        Ok(Outcome::NoNewUpdate) => crate::toast::notify(tr(MsgKey::update_check_no_new_updates)),
        Ok(Outcome::NewUpdate(release)) => crate::settings_ui::about::show_update(&release),
        Err(e) => crate::toast::notify(e.to_string()),
    });
}

/// Runs the About row's action. False for every other row.
pub fn on_action(screen: &str, title: &str) -> bool {
    if screen != "settings_about" || title != check_row_title() || !available() {
        return false;
    }
    manual();
    true
}

#[cfg(test)]
mod tests {
    use kuro_domain::release::Release;
    use std::cell::Cell;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    const DAY: i64 = 24 * 60 * 60 * 1000;

    struct Fixed {
        calls: Cell<u32>,
        reply: Result<Option<Release>, ReleaseError>,
    }

    impl ReleaseService for Fixed {
        fn latest(&self, _repository: &str) -> Result<Option<Release>, ReleaseError> {
            self.calls.set(self.calls.get() + 1);
            self.reply.clone()
        }
    }

    fn fixed(reply: Result<Option<Release>, ReleaseError>) -> Fixed {
        Fixed { calls: Cell::new(0), reply }
    }

    fn release(version: &str) -> Release {
        Release { version: version.into(), info: String::new(), release_link: "https://example.test/r".into(), download_link: "https://example.test/d".into() }
    }

    #[test]
    fn the_check_needs_the_updater_feature_and_a_repository() {
        assert!(enabled(true, "o/r"));
        assert!(!enabled(true, ""));
        assert!(!enabled(false, "o/r"));
    }

    #[test]
    fn the_about_row_is_listed_only_when_the_check_exists() {
        assert_eq!(hides_row(&check_row_title()), !available());
        assert!(!hides_row("Version"));
    }

    #[test]
    fn a_returned_release_is_compared_with_the_installed_version_and_no_time_is_stored() {
        let (service, now) = (fixed(Ok(Some(release("v2.0.0")))), 100 * DAY);
        let current = "1.0.0";
        let first = check(&service, "o/r", current, now, false).unwrap();
        assert_eq!(first, Outcome::NewUpdate(release("v2.0.0")));
        check(&service, "o/r", current, now + DAY, false).unwrap();
        assert_eq!(service.calls.get(), 2, "nothing is stored, so nothing throttles the second call");
        assert_eq!(check(&service, "o/r", "2.0.0", now, true), Ok(Outcome::NoNewUpdate));
    }

    #[test]
    fn an_error_is_returned_and_a_missing_asset_is_no_update() {
        let current = "1.0.0";
        let failing = fixed(Err(ReleaseError::Http(403)));
        assert_eq!(check(&failing, "o/r", current, 50 * DAY, true), Err(ReleaseError::Http(403)));
        let no_asset = fixed(Ok(None));
        assert_eq!(check(&no_asset, "o/r", current, 50 * DAY, true), Ok(Outcome::NoNewUpdate));
    }

    #[test]
    fn an_empty_repository_makes_no_call() {
        let service = fixed(Ok(Some(release("v2.0.0"))));
        let got = check(&service, "", "1.0.0", 100 * DAY, true);
        assert_eq!(got, Ok(Outcome::NoNewUpdate));
        assert_eq!(service.calls.get(), 0);
    }

    #[test]
    fn the_transport_returns_the_status_and_the_body_and_sends_the_user_agent() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/releases/latest", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap();
            let body = "{\"x\":1}";
            let reply = format!("HTTP/1.1 403 Forbidden\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase()
        });
        let (status, body) = Http::new().unwrap().get(&url, "Kurobayomi").unwrap();
        assert_eq!((status, body), (403, b"{\"x\":1}".to_vec()));
        assert!(server.join().unwrap().contains("user-agent: kurobayomi"));
    }
}
