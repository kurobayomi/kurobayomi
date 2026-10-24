//! The app's HTTP client. This build has no embedded WebKit view, so a Cloudflare challenge goes to the browser host:
//! the system browser opens the page and the cookies the user exports clear it (`NetworkClient::for_desktop`).
//!
//! Two things are shared by every client of the app: the one cookie jar ([`shared_jar`], T-12a-009), so Clear cookies,
//! the clearance import and the extension cookie removal reach the clients that are running, and the toast sender
//! ([`set_toaster`], T-12a-014), so the client's notices (Cloudflare, WebKit missing) reach the window overlay.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use kuro_net::cookies::CookieJar;
use kuro_net::{NetError, NetworkClient};
use kuro_platform::opener::SystemOpener;
use kuro_platform::paths::Paths;
use kuro_platform::toast::{Toaster, toast_channel};
use kuro_prefs::network::NetworkPrefs;

/// The jar of each cookie file that was opened. The app has one file, so one jar; tests with their own paths get their
/// own.
static JARS: Mutex<Option<HashMap<PathBuf, CookieJar>>> = Mutex::new(None);
static TOASTER: OnceLock<Toaster> = OnceLock::new();

/// Hands the window's toast sender to the network clients built from now on. Call once after `toast::attach`.
pub fn set_toaster(toaster: Toaster) {
    let _ = TOASTER.set(toaster);
}

/// The toast sender of the window, or a sender nobody reads before there is a window (tests).
fn toaster() -> Toaster {
    TOASTER.get().cloned().unwrap_or_else(|| toast_channel().0)
}

/// The cookie jar of `paths`, opened once: every call for the same file returns a clone of the same jar.
pub fn shared_jar(paths: &dyn Paths) -> CookieJar {
    let file = paths.cookie_jar_file();
    let mut jars = JARS.lock().unwrap_or_else(PoisonError::into_inner);
    jars.get_or_insert_with(HashMap::new).entry(file.clone()).or_insert_with(|| CookieJar::load(file)).clone()
}

/// Writes every open jar now; the jars live in a static, so their save-on-drop never runs at exit.
pub fn flush_jars() {
    let jars = JARS.lock().unwrap_or_else(PoisonError::into_inner);
    for jar in jars.iter().flat_map(|jars| jars.values()) {
        if let Err(error) = jar.flush() {
            tracing::warn!(%error, "the cookie jar could not be saved at exit");
        }
    }
}

/// The jar of this desktop, for the settings code. It is the one the clients use, whichever is built first.
pub fn app_jar() -> Option<CookieJar> {
    kuro_platform::system_paths::SystemPaths::system()
        .map(|paths| shared_jar(&paths))
        .map_err(|error| tracing::warn!(%error, "the cookie jar could not be located"))
        .ok()
}

/// The client over `paths`, with the browser host as the challenge solver and the app's shared jar.
pub fn build(prefs: &NetworkPrefs, paths: &dyn Paths) -> Result<NetworkClient, NetError> {
    NetworkClient::for_desktop_with_cookies(prefs, paths, shared_jar(paths), None, Arc::new(SystemOpener), toaster())
}

#[cfg(test)]
mod tests {
    use kuro_prefs::store::PrefStore;
    use kuro_testkit::TempPaths;

    use super::*;

    #[test]
    fn the_app_client_builds_without_a_web_view() {
        let paths = TempPaths::new().unwrap();
        assert!(build(&NetworkPrefs::new(PrefStore::in_memory()), &paths).is_ok());
    }

    #[test]
    fn every_client_and_the_jar_of_the_paths_share_one_cookie_store() {
        let paths = TempPaths::new().unwrap();
        let prefs = NetworkPrefs::new(PrefStore::in_memory());
        let (a, b) = (build(&prefs, &paths).unwrap(), build(&prefs, &paths).unwrap());
        let site = reqwest::Url::parse("https://site.test/").unwrap();
        a.cookies().import_clearance(&site, "abc", "Mozilla/5.0 Firefox/136.0").unwrap();
        for seen in [b.cookies().clone(), shared_jar(&paths)] {
            assert!(seen.get(&site).iter().any(|c| c.name() == "cf_clearance"), "the import reaches {seen:?}");
        }
        shared_jar(&paths).remove_all().unwrap();
        assert!(a.cookies().get(&site).is_empty() && b.cookies().get(&site).is_empty(), "Clear cookies reaches the clients");
        let other = TempPaths::new().unwrap();
        assert_ne!(shared_jar(&other).path(), shared_jar(&paths).path());
    }

    #[tokio::test]
    async fn the_clients_notices_go_to_the_window_toaster_once_it_is_set() {
        let (window, feed) = toast_channel();
        set_toaster(window);
        toaster().show("Cloudflare bypass failed", kuro_platform::toast::ToastLength::Long);
        assert_eq!(feed.next().await.unwrap().text, "Cloudflare bypass failed");
    }
}
