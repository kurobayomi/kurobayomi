//! Updates and What's new, for both channels. The notice of an update (a stable release that is out, or a nightly that was
//! installed and waits for a restart) shows as a toast with one action and as a "New" pill on the sidebar's Settings row (and Settings, About) until
//! it is looked at. The notes are a stable release's changelog section, or for a nightly the commits between two builds.
//! After an update, the first start shows the notes of the new build once; the build of the last run is kept in
//! `Paths::last_build_file`. A local build has no channel build and shows none of this.

use kuro_i18n::{Arg, MsgKey, tr, tr_plural, trf};
use std::cell::RefCell;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use kuro_domain::release::Release;
use kuro_platform::paths::Paths;

use crate::settings_ui::about::notes_dialog;
use crate::ui::{DialogButton, Role};

/// The build this app is: a stable release (its version) or a nightly (its commit). `None` for a local build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Build {
    Stable(String),
    Nightly(String),
}

impl Build {
    pub fn current() -> Option<Build> {
        if let Some(commit) = crate::channel::commit() {
            return Some(Build::Nightly(commit.to_owned()));
        }
        let info = &*kuro_core::app_info::APP_INFO;
        (info.build_type == "release" && !crate::app_update::UPDATE_REPOSITORY.is_empty()).then(|| Build::Stable(info.version_name.clone()))
    }

    fn line(&self) -> String {
        match self {
            Build::Stable(version) => format!("stable {version}"),
            Build::Nightly(commit) => format!("nightly {commit}"),
        }
    }

    fn parse(line: &str) -> Option<Build> {
        match line.trim().split_once(' ')? {
            ("stable", version) => Some(Build::Stable(version.to_owned())),
            ("nightly", commit) => Some(Build::Nightly(commit.to_owned())),
            _ => None,
        }
    }
}

/// An update the user has not acted on yet.
#[derive(Clone)]
enum Notice {
    /// A newer stable release is out.
    Stable(Release),
    /// A newer nightly replaced the AppImage; it runs after a restart. The commit of the new build.
    Nightly(String),
    /// A newer stable release replaced the AppImage; it runs after a restart. The version of the new build.
    StableInstalled(String),
}

thread_local! {
    /// The notice and whether it was looked at (the pill shows while it was not).
    static NOTICE: RefCell<Option<(Notice, bool)>> = const { RefCell::new(None) };
    static PILLS: RefCell<Vec<glib::WeakRef<gtk::Label>>> = const { RefCell::new(Vec::new()) };
}

fn set_notice(notice: Notice) {
    NOTICE.with(|n| *n.borrow_mut() = Some((notice, false)));
    refresh_pills();
}

fn take_notice_for_viewing() -> Option<Notice> {
    let notice = NOTICE.with(|n| n.borrow_mut().as_mut().map(|(notice, seen)| {
        *seen = true;
        notice.clone()
    }));
    refresh_pills();
    notice
}

fn unseen() -> bool {
    NOTICE.with(|n| n.borrow().as_ref().is_some_and(|(_, seen)| !seen))
}

fn refresh_pills() {
    let show = unseen();
    PILLS.with(|pills| {
        pills.borrow_mut().retain(|pill| pill.upgrade().map(|pill| pill.set_visible(show)).is_some());
    });
}

/// The "New" pill, for the sidebar's Settings row and the About row of Settings, shown while an update notice is unseen.
pub fn pill() -> gtk::Label {
    let pill = gtk::Label::builder().label(&tr(MsgKey::ui_new)).css_classes(["kb-nav-badge"]).valign(gtk::Align::Center).visible(unseen()).build();
    pill.set_tooltip_text(Some(&tr(MsgKey::ui_update_waiting_open_about)));
    PILLS.with(|pills| pills.borrow_mut().push(pill.downgrade()));
    pill
}

/// The About page's What's new row: its subtitle, and whether it can open anything.
pub fn row_subtitle() -> Option<String> {
    NOTICE.with(|n| {
        n.borrow().as_ref().map(|(notice, _)| match notice {
            Notice::Stable(release) => trf(MsgKey::whats_new_stable_available, &[("version", Arg::Str(&release.version))]),
            Notice::Nightly(commit) => trf(MsgKey::whats_new_nightly_restart, &[("commit", Arg::Str(short(commit)))]),
            Notice::StableInstalled(version) => trf(MsgKey::whats_new_stable_restart, &[("version", Arg::Str(version))]),
        })
    })
}

pub fn row_enabled() -> bool {
    Build::current().is_some()
}

/// A newer stable release is out (the startup check found it).
pub fn stable_available(release: Release) {
    let text = trf(MsgKey::whats_new_stable_available, &[("version", Arg::Str(&release.version))]);
    set_notice(Notice::Stable(release));
    crate::toast::notify_action(&text, &tr(MsgKey::whats_new), Some(Duration::from_secs(10)), open);
}

/// The nightly AppImage was replaced by a newer build.
pub fn nightly_installed(commit: String) {
    let text = trf(MsgKey::whats_new_nightly_installed, &[("commit", Arg::Str(short(&commit)))]);
    set_notice(Notice::Nightly(commit));
    crate::toast::notify_action(&text, &tr(MsgKey::ui_restart), Some(Duration::from_secs(10)), restart);
}

/// The stable AppImage was replaced by a newer release.
pub fn stable_installed(version: String) {
    let text = trf(MsgKey::whats_new_stable_installed, &[("version", Arg::Str(&version))]);
    set_notice(Notice::StableInstalled(version));
    crate::toast::notify_action(&text, &tr(MsgKey::ui_restart), Some(Duration::from_secs(10)), restart);
}

/// What's new: the waiting update when there is one, else the notes of the running build.
pub fn open() {
    match take_notice_for_viewing() {
        Some(Notice::Stable(release)) => crate::settings_ui::about::show_update(&release),
        Some(Notice::Nightly(new)) => {
            let Some(Build::Nightly(running)) = Build::current() else { return };
            let restart_button = DialogButton::new(&tr(MsgKey::ui_restart), Role::Suggested, restart);
            fetch(move || commits_between(&running, &new).map(|c| (commits_markdown(&c), new)), move |(notes, new)| {
                notes_dialog(&tr(MsgKey::ui_nightly_update_installed), &format!("Nightly {}", short(&new)), &notes, None, Some(restart_button));
            });
        }
        Some(Notice::StableInstalled(version)) => {
            let restart_button = DialogButton::new(&tr(MsgKey::ui_restart), Role::Suggested, restart);
            fetch(move || stable_notes(&version).map(|notes| (notes, version)), move |((body, url), version)| {
                notes_dialog(&tr(MsgKey::ui_update_installed), &format!("Kurobayomi {version}"), &body, Some(&url), Some(restart_button));
            });
        }
        None => show_current(None),
    }
}

/// The notes of the running build. `since` is the build of the last run: a nightly then lists the commits since it.
fn show_current(since: Option<Build>) {
    match Build::current() {
        Some(Build::Stable(version)) => {
            let heading = if since.is_some() { trf(MsgKey::whats_new_updated_to_stable, &[("version", Arg::Str(&version))]) } else { tr(MsgKey::whats_new).to_owned() };
            fetch(move || stable_notes(&version).map(|notes| (notes, version)), move |((body, url), version)| {
                notes_dialog(&heading, &format!("Kurobayomi {version}"), &body, Some(&url), None);
            });
        }
        Some(Build::Nightly(commit)) => {
            let old = match since {
                Some(Build::Nightly(old)) => Some(old),
                _ => None,
            };
            let heading = if old.is_some() { trf(MsgKey::whats_new_updated_to_nightly, &[("commit", Arg::Str(short(&commit)))]) } else { tr(MsgKey::whats_new).to_owned() };
            fetch(
                move || {
                    let commits = match &old {
                        Some(old) => commits_between(old, &commit)?,
                        None => recent_commits(&commit)?,
                    };
                    Ok((commits_markdown(&commits), commit))
                },
                move |(notes, commit)| notes_dialog(&heading, &format!("Nightly {}", short(&commit)), &notes, None, None),
            );
        }
        None => {}
    }
}

/// At start: remember this build, and after an update show its notes once.
pub fn startup() {
    let Some(current) = Build::current() else { return };
    let Ok(paths) = kuro_platform::system_paths::SystemPaths::system() else { return };
    let file = paths.last_build_file();
    let last = std::fs::read_to_string(&file).ok().and_then(|line| Build::parse(&line));
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = std::fs::write(&file, current.line()) {
        tracing::debug!(%error, "could not keep the build of this run");
    }
    if let Some(last) = last.filter(|last| *last != current) {
        show_current(Some(last));
    }
}

/// Starts the new build once the app has quit (`appimage_update::restart_into_update`), then closes the window.
pub fn restart() {
    if crate::appimage_update::restart_into_update() {
        crate::ui::window().close();
    } else {
        crate::toast::notify(&tr(MsgKey::ui_restart_kurobayomi_to_use_the_new_version));
    }
}

fn short(commit: &str) -> &str {
    &commit[..8.min(commit.len())]
}

/// Runs `work` off the GTK thread and `done` with its result on it; a failure is a toast.
fn fetch<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static, done: impl FnOnce(T) + 'static) {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(work());
    });
    glib::spawn_future_local(async move {
        match rx.recv().await {
            Ok(Ok(value)) => done(value),
            Ok(Err(error)) => {
                tracing::debug!(%error, "What's new could not be loaded");
                crate::toast::notify(&tr(MsgKey::ui_could_not_load_whats_new));
            }
            Err(_) => {}
        }
    });
}

fn github(path: &str) -> Result<serde_json::Value, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("kurobayomi/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("https://api.github.com/repos/{}/{path}", crate::channel::REPOSITORY);
    let response = client.get(url).send().and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?;
    response.json().map_err(|e| e.to_string())
}

/// The changelog section and the page of the stable release `version`.
fn stable_notes(version: &str) -> Result<(String, String), String> {
    let release = github(&format!("releases/tags/v{version}"))?;
    Ok((release["body"].as_str().unwrap_or_default().to_owned(), release["html_url"].as_str().unwrap_or_default().to_owned()))
}

/// The commits after `old` up to `new`, newest first.
fn commits_between(old: &str, new: &str) -> Result<Vec<(String, String)>, String> {
    let mut commits = commit_list(&github(&format!("compare/{old}...{new}"))?["commits"]);
    commits.reverse();
    Ok(commits)
}

/// The 20 latest commits up to `commit`, newest first.
fn recent_commits(commit: &str) -> Result<Vec<(String, String)>, String> {
    Ok(commit_list(&github(&format!("commits?sha={commit}&per_page=20"))?))
}

/// The commit hash and the first line of the message of every commit of a GitHub commit list.
fn commit_list(list: &serde_json::Value) -> Vec<(String, String)> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| Some((c["sha"].as_str()?.to_owned(), c["commit"]["message"].as_str()?.lines().next().unwrap_or_default().to_owned())))
        .collect()
}

/// The most commits the notes list; older ones are counted in one last line.
const MAX_COMMITS: usize = 10;
/// The longest commit subject the notes show, in characters; a longer one ends with an ellipsis.
const MAX_SUBJECT_CHARS: usize = 140;

/// The notes of a list of commits (newest first): one bullet per commit, its message and short hash, for the newest
/// [`MAX_COMMITS`], then how many more there are.
fn commits_markdown(commits: &[(String, String)]) -> String {
    if commits.is_empty() {
        return tr(MsgKey::ui_no_changes_since_your_last_version).to_owned();
    }
    let mut notes = format!("### {}\n", tr(MsgKey::whats_new_changes));
    for (sha, subject) in commits.iter().take(MAX_COMMITS) {
        let subject = if subject.chars().count() > MAX_SUBJECT_CHARS {
            format!("{}…", subject.chars().take(MAX_SUBJECT_CHARS - 1).collect::<String>().trim_end())
        } else {
            subject.clone()
        };
        notes.push_str(&format!("- {subject} (`{}`)\n", short(sha)));
    }
    if let Some(more) = commits.len().checked_sub(MAX_COMMITS).filter(|more| *more > 0) {
        notes.push_str(&format!("\n{}\n", tr_plural(MsgKey::whats_new_more_changes, i64::try_from(more).unwrap_or(i64::MAX), &[])));
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_line_round_trips_and_odd_lines_are_none() {
        for build in [Build::Stable("0.2.0".into()), Build::Nightly("abcdef0123456789".into())] {
            assert_eq!(Build::parse(&build.line()), Some(build));
        }
        assert_eq!(Build::parse("debug 1"), None);
        assert_eq!(Build::parse(""), None);
    }

    #[test]
    fn a_commit_list_keeps_the_hash_and_the_first_line() {
        let list = serde_json::json!([
            { "sha": "1111111111aaaa", "commit": { "message": "Fix the grid\n\nlong text" } },
            { "sha": "2222222222bbbb", "commit": { "message": "Add a button" } },
            { "nothing": true }
        ]);
        assert_eq!(commit_list(&list), vec![("1111111111aaaa".into(), "Fix the grid".into()), ("2222222222bbbb".into(), "Add a button".into())]);
    }

    #[test]
    fn the_notes_list_each_commit_with_its_short_hash() {
        let notes = commits_markdown(&[("1111111111aaaa".into(), "Fix the grid".into())]);
        assert_eq!(notes, "### Changes\n- Fix the grid (`11111111`)\n");
        assert_eq!(commits_markdown(&[]), "No changes since your last version.");
    }

    #[test]
    fn the_notes_list_the_newest_ten_changes_shortened_and_count_the_rest() {
        let commits: Vec<(String, String)> = (0..13).map(|n| (format!("{n:040}"), format!("change {n}"))).collect();
        let notes = commits_markdown(&commits);
        assert_eq!(notes.matches("\n- ").count() + usize::from(notes.starts_with("- ")), 10);
        assert!(notes.contains("change 9") && !notes.contains("change 10"), "the newest ten");
        assert!(notes.ends_with("…and 3 more changes.\n"));
        let long = vec![("a".repeat(40), "x".repeat(300))];
        let line = commits_markdown(&long);
        assert!(line.contains(&format!("{}…", "x".repeat(MAX_SUBJECT_CHARS - 1))) && !line.contains(&"x".repeat(MAX_SUBJECT_CHARS)));
        assert!(!commits_markdown(&commits[..10]).contains("more"), "exactly ten: no count line");
    }
}
