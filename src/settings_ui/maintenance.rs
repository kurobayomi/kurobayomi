//! The maintenance actions the Advanced page used to hold, now placed on the pages they belong to: the cookie and WebView
//! rows and the trusted-extension revoke sit on Sources and tracking, the reindex, cover refresh and reader reset rows
//! sit on Backup and restore, and the Share crash logs row sits on About. The verbose logging, DNS, user agent and
//! onboarding rows are cut.
use kuro_i18n::{MsgKey, tr};
use std::rc::Rc;

use gtk::prelude::*;
use kuro_data::entry::EntryRepo;
use kuro_platform::kind::Kind;
use kuro_prefs::prefs::Prefs;

use crate::pref_model::model::PreferenceItem;
use crate::settings_ui::rows::text;

/// Empties `jar` (memory and file) and returns the toast text on success. The jar is the one the running clients
/// use (`net_client::app_jar`), so the cookies stop being sent at once and none is written back at exit.
pub fn clear_cookies_in(jar: &kuro_net::cookies::CookieJar) -> std::io::Result<String> {
    jar.remove_all()?;
    Ok(tr(MsgKey::cookies_cleared))
}

fn clear_cookies() {
    match crate::net_client::app_jar().ok_or_else(|| std::io::Error::other("no cookie jar")).and_then(|jar| clear_cookies_in(&jar)) {
        Ok(text) => crate::toast::notify(text),
        Err(err) => tracing::warn!(%err, "the cookies could not be cleared"),
    }
}

/// The "Clear cookies" row: empties the HTTP cookie jar and says "Cookies cleared".
pub fn clear_cookies_row() -> PreferenceItem {
    let mut row = text(&tr(MsgKey::pref_clear_cookies), None, true);
    if let PreferenceItem::Text { on_click, .. } = &mut row {
        *on_click = Some(Rc::new(clear_cookies));
    }
    row
}

/// The WebView data clear (the cache, cookies and local storage of the WebKit view).
pub fn clear_webview_row() -> PreferenceItem {
    let mut row = text(&tr(MsgKey::pref_clear_webview_data), None, true);
    if let PreferenceItem::Text { on_click, .. } = &mut row {
        *on_click = Some(Rc::new(crate::settings::clear_webview_data));
    }
    row
}

/// The cookie and WebView rows of Sources and tracking, in list order.
pub fn networking_rows() -> Vec<PreferenceItem> {
    vec![clear_cookies_row(), clear_webview_row()]
}

/// Clears every manga's reader viewer flags (the reading mode) and says so in a toast.
pub fn reset_reader_settings() {
    let repo = EntryRepo::new(crate::data::app().db.clone());
    if kuro_domain::entries::viewer_flags::reset_viewer_flags(&repo, Kind::Manga) {
        crate::toast::notify(&tr(MsgKey::pref_reset_viewer_flags_success));
    } else {
        crate::toast::notify(&tr(MsgKey::pref_reset_viewer_flags_error));
    }
}

/// The "Reset per-series reader settings" row. The anime equivalent is not offered (Android parity).
pub fn reset_reader_row() -> PreferenceItem {
    let mut row = text(&tr(MsgKey::pref_reset_viewer_flags), Some(&tr(MsgKey::pref_reset_viewer_flags_summary)), true);
    if let PreferenceItem::Text { on_click, .. } = &mut row {
        *on_click = Some(Rc::new(reset_reader_settings));
    }
    row
}

/// How long the Share crash logs row shows its success and failure status before the subtitle comes back.
const COPIED_STATUS: std::time::Duration = std::time::Duration::from_secs(4);
const FAILED_STATUS: std::time::Duration = std::time::Duration::from_secs(2);
fn share_subtitle() -> String {
    tr(MsgKey::pref_dump_crash_logs_summary)
}
fn copied_text() -> String {
    tr(MsgKey::settings_maintenance_copied_text)
}
fn failed_text() -> String {
    tr(MsgKey::settings_maintenance_failed_text)
}

/// The debug block of the crash dump. Components this build has not loaded say `not loaded`, as the crash report does.
fn dump_debug_info() -> kuro_core::crash_dump::DebugInfo {
    let info = &*kuro_core::app_info::APP_INFO;
    let facts: std::collections::BTreeMap<_, _> = kuro_platform::system::facts().into_iter().collect();
    let fact = |key: &str| facts.get(key).cloned().unwrap_or_else(|| "unknown".to_owned());
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    kuro_core::crash_dump::DebugInfo {
        app_id: info.app_id.to_owned(),
        app_version: format!("{} ({}, {}, {})", info.version_name, info.git_sha, info.version_code, info.build_time),
        os: fact("os"),
        kernel: fact("kernel"),
        desktop: fact("desktop"),
        gtk: format!("{}.{}.{}", gtk::major_version(), gtk::minor_version(), gtk::micro_version()),
        webview: "not loaded".to_owned(),
        current_time: kuro_core::utc::rfc3339(now),
        mpv: "not loaded".to_owned(),
        ffmpeg: "not loaded".to_owned(),
        sqlite: rusqlite::version().to_owned(),
        locale: fact("locale"),
    }
}

/// Composes the dump from the debug block, the extension lists and the log file in `log_dir`, writes it under
/// `<cache_dir>/crash/` and returns the file and its text.
fn write_crash_dump(
    cache_dir: &std::path::Path,
    log_dir: &std::path::Path,
    debug: &kuro_core::crash_dump::DebugInfo,
    manga: &[kuro_core::crash_dump::ExtensionStatus],
    anime: &[kuro_core::crash_dump::ExtensionStatus],
) -> std::io::Result<(std::path::PathBuf, String)> {
    use kuro_core::crash_dump::{compose, write_dump, DumpInputs};
    let log = std::fs::read(log_dir.join(kuro_core::logging::LOG_FILE_NAME)).ok().map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let text = compose(&DumpInputs { debug, manga_extensions: manga, anime_extensions: anime, exception: None, log: log.as_deref() });
    write_dump(cache_dir, &text).map(|path| (path, text))
}

/// What a tap on Share crash logs does once the dump is built: copy the text and reveal the file.
fn reveal_dump(path: &std::path::Path, text: &str) {
    if let Some(display) = gtk::gdk::Display::default() {
        display.clipboard().set_text(text);
    }
    if let Err(err) = kuro_platform::opener::Opener::reveal(&kuro_platform::opener::SystemOpener, path) {
        tracing::warn!(%err, "the crash log folder did not open");
    }
}

/// Builds the dump off the GTK thread. The row is insensitive until it is done, then it shows `Crash logs copied to the
/// clipboard` for 4 s or `Failed to get logs` for 2 s in place of its subtitle.
fn share_crash_logs(row: &gtk::Widget, subtitle: &gtk::Label) {
    row.set_sensitive(false);
    let debug = dump_debug_info();
    let (manga, anime): (Vec<kuro_core::crash_dump::ExtensionStatus>, Vec<kuro_core::crash_dump::ExtensionStatus>) = (Vec::new(), Vec::new());
    let dirs = kuro_platform::system_paths::SystemPaths::system().map(|p| (kuro_platform::paths::Paths::cache_dir(&p), kuro_platform::paths::Paths::log_dir(&p)));
    let (row, subtitle) = (row.clone(), subtitle.clone());
    gtk::glib::spawn_future_local(async move {
        let done = match dirs {
            Ok((cache, logs)) => gtk::gio::spawn_blocking(move || write_crash_dump(&cache, &logs, &debug, &manga, &anime)).await.map_err(|_| "the dump task panicked".to_owned()).and_then(|r| r.map_err(|e| e.to_string())),
            Err(err) => Err(err.to_string()),
        };
        let (text, shown) = match done {
            Ok((path, text)) => {
                reveal_dump(&path, &text);
                (copied_text(), COPIED_STATUS)
            }
            Err(err) => {
                tracing::warn!(%err, "the crash logs could not be written");
                (failed_text(), FAILED_STATUS)
            }
        };
        crate::pref_model::widgets::row::set_subtitle(&subtitle, Some(&text));
        row.set_sensitive(true);
        gtk::glib::timeout_add_local_once(shown, move || crate::pref_model::widgets::row::set_subtitle(&subtitle, Some(&share_subtitle())));
    });
}

/// The Share crash logs row: a tap builds `<cache>/crash/kurobayomi_crash_logs.txt`, copies the text and reveals the file.
pub fn share_crash_logs_row() -> PreferenceItem {
    use crate::pref_model::widgets::row::{self, RowSpec};
    PreferenceItem::Custom {
        title: tr(MsgKey::pref_dump_crash_logs).to_owned(),
        build: Rc::new(|| {
            let built = row::build(RowSpec { title: &tr(MsgKey::pref_dump_crash_logs), subtitle: Some(&share_subtitle()), icon: None, trailing: None, on_click: None });
            let content = built.row.child().and_downcast::<gtk::Box>().expect("the row content");
            built.row.set_child(None::<&gtk::Widget>);
            content.set_focusable(true);
            content.set_cursor_from_name(Some("pointer"));
            let (target, subtitle) = (content.clone().upcast::<gtk::Widget>(), built.subtitle.clone());
            let tap = {
                let (target, subtitle) = (target.clone(), subtitle.clone());
                Rc::new(move || if target.is_sensitive() { share_crash_logs(&target, &subtitle) })
            };
            let click = gtk::GestureClick::new();
            let on_click = Rc::clone(&tap);
            click.connect_released(move |_, _, _, _| on_click());
            content.add_controller(click);
            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed(move |_, key, _, _| {
                if crate::pref_model::widgets::dialog::is_enter(key) || key == gtk::gdk::Key::space {
                    tap();
                    gtk::glib::Propagation::Stop
                } else {
                    gtk::glib::Propagation::Proceed
                }
            });
            content.add_controller(keys);
            content.upcast()
        }),
    }
}

/// Invalidates the download index of both kinds (the one the banners and filters read): its next renewal rescans the
/// download folders, and the snapshot file on disk is gone.
fn reindex_downloads() {
    match crate::shell::host::Shell::get().and_then(|shell| shell.download_service()) {
        Some(downloads) => {
            downloads.reindex(None);
            crate::toast::notify(&tr(MsgKey::download_cache_invalidated));
        }
        None => crate::toast::notify(&tr(MsgKey::ui_failed_to_reindex_downloads)),
    }
}

/// The Reindex downloads row (task T-47a-074).
pub fn reindex_downloads_row() -> PreferenceItem {
    let mut row = text(&tr(MsgKey::pref_invalidate_download_cache), Some(&tr(MsgKey::pref_invalidate_download_cache_summary)), true);
    if let PreferenceItem::Text { on_click, .. } = &mut row {
        *on_click = Some(Rc::new(reindex_downloads));
    }
    row
}

/// Starts the manga and anime library update and the manga and anime metadata update (task T-47a-079). A job already
/// running is left alone, and nothing is shown.
pub fn refresh_covers() {
    for kind in [Kind::Manga, Kind::Anime] {
        crate::updates::library_trigger::start(kind);
    }
    crate::misc::runner::refresh_library_covers();
}

/// The Refresh library covers row (task T-47a-079).
pub fn refresh_covers_row() -> PreferenceItem {
    let mut row = text(&tr(MsgKey::pref_refresh_library_covers), None, true);
    if let PreferenceItem::Text { on_click, .. } = &mut row {
        *on_click = Some(Rc::new(refresh_covers));
    }
    row
}

/// The maintenance rows of Backup and restore, in list order.
pub fn data_rows() -> Vec<PreferenceItem> {
    vec![reindex_downloads_row(), refresh_covers_row(), reset_reader_row()]
}

/// Revoke trusted unknown extensions (task T-47a-084): the trusted set of manga and anime extensions is cleared and the
/// page is redrawn. The extension lists read the set when they are shown, so nothing waits for a restart.
pub fn revoke_trust_row(prefs: &Prefs, changed: Rc<dyn Fn()>) -> PreferenceItem {
    let trusted = prefs.sources.trusted_extensions.clone();
    let mut row = text(&tr(MsgKey::ext_revoke_trust), None, true);
    if let PreferenceItem::Text { on_click, .. } = &mut row {
        *on_click = Some(Rc::new(move || match trusted.set(std::collections::BTreeSet::new()) {
            Ok(()) => changed(),
            Err(err) => tracing::warn!(%err, "the trusted extensions were not cleared"),
        }));
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn clear_cookies_empties_the_open_jar_and_its_file_and_reports_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("jar.json");
        let jar = kuro_net::cookies::CookieJar::load(file.clone());
        let url = reqwest::Url::parse("https://example.org/").unwrap();
        let (n, v) = ("k", "v");
        reqwest::cookie::CookieStore::set_cookies(
            &*jar.provider(),
            &mut std::iter::once(&reqwest::header::HeaderValue::from_str(&format!("{n}={v}; Max-Age=3600")).unwrap()),
            &url,
        );
        jar.flush().unwrap();
        assert_eq!(jar.get(&url).len(), 1);
        assert_eq!(clear_cookies_in(&jar).unwrap(), "Cookies cleared");
        assert!(jar.get(&url).is_empty(), "the open jar is empty, not only the file");
        assert!(kuro_net::cookies::CookieJar::load(file).get(&url).is_empty());
    }

    #[test]
    fn revoke_clears_the_trusted_set_and_redraws_the_page() {
        let prefs = Prefs::in_memory();
        prefs.sources.trusted_extensions.set(std::collections::BTreeSet::from(["pkg.a".to_owned()])).unwrap();
        let redrawn = Rc::new(Cell::new(0));
        let counter = Rc::clone(&redrawn);
        let row = revoke_trust_row(&prefs, Rc::new(move || counter.set(counter.get() + 1)));
        let PreferenceItem::Text { on_click, common } = &row else { panic!("an action row") };
        assert_eq!(common.title, "Revoke trusted unknown extensions");
        (on_click.as_ref().expect("a tap action"))();
        assert!(prefs.sources.trusted_extensions.get().is_empty());
        assert_eq!(redrawn.get(), 1);
    }

    #[test]
    fn the_dump_goes_to_the_crash_folder_with_the_extension_blocks_and_the_last_500_error_lines() {
        use kuro_core::crash_dump::{dump_path, DebugInfo, ExtensionStatus};
        let (cache, logs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let log: String = (0..600).map(|i| format!("2026-10-10T10:00:00Z ERROR boom {i}\n2026-10-10T10:00:00Z INFO fine {i}\n")).collect();
        std::fs::write(logs.path().join(kuro_core::logging::LOG_FILE_NAME), log).unwrap();
        let old = ExtensionStatus { name: "Old".into(), installed_name: "1".into(), installed_code: 1, available: Some(("2".into(), 2)), obsolete: false };
        let (path, text) = write_crash_dump(cache.path(), logs.path(), &DebugInfo::default(), &[old], &[]).unwrap();
        assert_eq!(path, dump_path(cache.path()));
        assert!(path.ends_with("crash/kurobayomi_crash_logs.txt"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert!(text.contains("Problematic manga extensions:") && !text.contains("Problematic anime extensions:"));
        assert_eq!(text.lines().filter(|l| l.contains("ERROR")).count(), 500);
        assert!(text.contains("boom 599") && !text.contains("boom 99\n"), "the newest lines win");
    }

    #[test]
    fn a_missing_log_file_still_gives_a_dump() {
        let (cache, logs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (_, text) = write_crash_dump(cache.path(), logs.path(), &Default::default(), &[], &[]).unwrap();
        assert!(text.ends_with("(no log file)\n"));
    }

    #[test]
    fn the_refresh_covers_row_and_the_data_rows_are_tappable() {
        let PreferenceItem::Text { common, on_click, .. } = refresh_covers_row() else { panic!("a text row") };
        assert_eq!(common.title, "Refresh library covers");
        assert!(common.enabled && on_click.is_some());
        let titles: Vec<String> = data_rows().iter().map(|row| row.title().to_owned()).collect();
        assert_eq!(titles, ["Reindex downloads", "Refresh library covers", "Reset per-series reader settings"]);
    }

    #[test]
    fn the_crash_log_row_and_the_reindex_row_are_tappable() {
        let PreferenceItem::Custom { title, .. } = share_crash_logs_row() else { panic!("a custom row") };
        assert_eq!(title, "Share crash logs");
        let PreferenceItem::Text { common, on_click, .. } = reindex_downloads_row() else { panic!("a text row") };
        assert_eq!(common.subtitle.as_deref(), Some("Force app to recheck downloaded chapters and episodes"));
        assert!(on_click.is_some());
    }

    #[test]
    fn the_reader_reset_row_has_its_spec_subtitle() {
        let PreferenceItem::Text { common, on_click, .. } = reset_reader_row() else { panic!("text row") };
        assert_eq!(common.subtitle.as_deref(), Some("Resets reading mode of all series"));
        assert!(on_click.is_some());
    }

    #[test]
    fn the_networking_rows_are_clear_cookies_and_clear_webview() {
        let titles: Vec<String> = networking_rows().iter().map(|row| row.title().to_owned()).collect();
        assert_eq!(titles, ["Clear cookies", "Clear WebView data"]);
    }
}
