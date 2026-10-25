//! One on/off switch per repository source (owner, 2026-10-23). `enabled_sources` holds the entry ids (the `id` of an
//! `index.min.json` entry) of the sources that are on; a source not in it is off, so a source a repository adds later
//! starts off. Off means not on this computer: its file is not downloaded or refreshed, it is not loaded, and Browse,
//! global search and the library update leave it out. Its titles stay in the library and their downloads still open;
//! what needs the site says "This source is off" with a Turn on button.
//!
//! A source is known by its numeric id once it has loaded; `source_keys` remembers which entry each id came from, so a
//! source that is off now is still recognised by its titles.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use gtk::prelude::*;
use kuro_extensions::repo::{Entry, Index, Status};
use kuro_i18n::{Arg, MsgKey, tr, tr_plural, trf};
use kuro_platform::kind::Kind;
use kuro_prefs::source::SourcePrefs;
use kuro_source::SourceError;

/// Whether the source of entry `key` is on.
pub fn is_on(prefs: &SourcePrefs, key: &str) -> bool {
    prefs.enabled_sources.get().contains(key)
}

/// Turns the source of entry `key` on or off in the stored set. Only the set changes; [`turn_on`] and [`turn_off`] also
/// fetch or delete its file.
pub fn set_on(prefs: &SourcePrefs, key: &str, on: bool) {
    let mut set = prefs.enabled_sources.get();
    let changed = if on { set.insert(key.to_owned()) } else { set.remove(key) };
    if changed {
        if let Err(error) = prefs.enabled_sources.set(set) {
            tracing::warn!(%error, key, "the source switch could not be saved");
        }
    }
}

/// The entry ids every saved index lists.
fn all_keys(indexes: &[Index]) -> BTreeSet<String> {
    indexes.iter().flat_map(|index| index.sources.iter().map(|entry| entry.id.clone())).collect()
}

/// The one-time migration of a user who had repositories before the switches: when `enabled_sources` was never written,
/// every source of every repository already added is turned on, so nothing they had disappears. A store that holds the
/// key (even an empty set) is left alone, so it runs once. Returns whether it ran.
pub fn migrate_once(prefs: &SourcePrefs, indexes: &[Index]) -> bool {
    if prefs.enabled_sources.is_set() {
        return false;
    }
    if let Err(error) = prefs.enabled_sources.set(all_keys(indexes)) {
        tracing::warn!(%error, "the sources could not be turned on for the switch migration");
    }
    true
}

/// The ids a repository entry may have: the ones it loaded under before, and the id a source of its name and language
/// gets from the usual formula (`generate_id(name, lang, 1)`, as Aniyomi's extensions and the recipes compute it), so an
/// entry that never loaded here still matches an Aniyomi id.
fn ids_of(entry: &Entry, keys: &BTreeMap<i64, String>) -> HashSet<i64> {
    let mut ids: HashSet<i64> = keys.iter().filter(|(_, key)| **key == entry.id).map(|(id, _)| *id).collect();
    ids.insert(kuro_source::generate_id(&entry.name, &entry.lang, 1));
    ids
}

/// Aniyomi's backups store the disabled sources (`hidden_catalogues`, `hidden_anime_catalogues`); a restore writes them as
/// they are. Read once: every known source is on except those Aniyomi had disabled, and both keys are cleared. Returns
/// whether a restored list was there.
pub fn import_aniyomi_disabled(prefs: &SourcePrefs, indexes: &[Index]) -> bool {
    if !prefs.disabled_manga_sources.is_set() && !prefs.disabled_anime_sources.is_set() {
        return false;
    }
    let disabled: HashSet<i64> = prefs
        .disabled_manga_sources
        .get()
        .into_iter()
        .chain(prefs.disabled_anime_sources.get())
        .filter_map(|id| id.parse().ok())
        .collect();
    let keys = keys(prefs);
    let on: BTreeSet<String> = indexes
        .iter()
        .flat_map(|index| index.sources.iter())
        .filter(|entry| ids_of(entry, &keys).is_disjoint(&disabled))
        .map(|entry| entry.id.clone())
        .collect();
    if let Err(error) = prefs.enabled_sources.set(on) {
        tracing::warn!(%error, "the restored source switches could not be saved");
    }
    let _ = prefs.disabled_manga_sources.delete();
    let _ = prefs.disabled_anime_sources.delete();
    true
}

/// Both steps that turn stored state into the switches, before any source file is fetched or deleted, then the
/// languages of the sources that are on.
pub fn settle(prefs: &SourcePrefs, indexes: &[Index]) {
    if migrate_once(prefs, indexes) {
        NARROW_AFTER_LOAD.store(true, Ordering::SeqCst);
    }
    import_aniyomi_disabled(prefs, indexes);
    show_languages_of_on(prefs, indexes);
}

/// Set when the one-time migration ran at this start: it turned on every listed source, since before the switches each
/// one was active, so the first load knows every source's id; after it the switches keep only the sources in use
/// ([`after_load`]). ponytail: in memory only, so a start that quits before its first load ends keeps them all on.
static NARROW_AFTER_LOAD: AtomicBool = AtomicBool::new(false);

/// Called when a load has installed its sources (any thread). After the migration's first load, the sources nobody
/// used are turned off, on the GTK thread, where the library and the history are.
pub fn after_load() {
    if NARROW_AFTER_LOAD.swap(false, Ordering::SeqCst) {
        gtk::glib::MainContext::default().invoke(narrow_to_used);
    }
}

/// Keeps on only the sources with titles in the library or the history, of either kind: a source listed by a
/// repository is not one the user had (three Russian sources an English reader never opened came on).
fn narrow_to_used() {
    let Some(prefs) = live() else { return };
    let db = crate::data::app().db.clone();
    let entries = kuro_data::entry::EntryRepo::new(db.clone());
    let history = kuro_data::history::HistoryRepo::new(db);
    let mut used: HashSet<i64> = HashSet::new();
    for kind in [Kind::Manga, Kind::Anime] {
        used.extend(entries.favorite_counts_by_source(kind).unwrap_or_default().into_iter().map(|(source, _)| source));
        used.extend(history.list(kind, "").unwrap_or_default().into_iter().map(|item| item.source));
    }
    let on = prefs.enabled_sources.get();
    let kept = in_use(&on, &keys(&prefs), &used);
    if kept != on {
        tracing::info!(off = on.len() - kept.len(), "the sources not in the library or the history start off");
        if let Err(error) = prefs.enabled_sources.set(kept) {
            tracing::warn!(%error, "the sources in use could not be saved");
            return;
        }
        crate::recipe_sources::reload();
    }
}

/// The sources of `on` to keep: every one whose id is in `used`, and every one whose id is not known (it did not load),
/// since a source that cannot be told unused is not turned off.
fn in_use(on: &BTreeSet<String>, keys: &BTreeMap<i64, String>, used: &HashSet<i64>) -> BTreeSet<String> {
    let loaded_unused: HashSet<&String> = keys.iter().filter(|(id, _)| !used.contains(id)).map(|(_, key)| key).collect();
    let loaded_used: HashSet<&String> = keys.iter().filter(|(id, _)| used.contains(id)).map(|(_, key)| key).collect();
    on.iter().filter(|key| loaded_used.contains(key) || !loaded_unused.contains(key)).cloned().collect()
}

/// Turns on the language of every source that is on. Browse lists only the enabled languages, so without this a source
/// the user turned on (a Russian one on an English system) is on and never shown. To hide a source, turn it off.
pub fn show_languages_of_on(prefs: &SourcePrefs, indexes: &[Index]) {
    let on = prefs.enabled_sources.get();
    let mut languages = prefs.enabled_languages.get();
    let before = languages.len();
    languages.extend(indexes.iter().flat_map(|index| &index.sources).filter(|entry| on.contains(&entry.id)).map(|entry| entry.lang.clone()));
    if languages.len() != before {
        if let Err(error) = prefs.enabled_languages.set(languages) {
            tracing::warn!(%error, "the languages of the sources that are on could not be saved");
        }
    }
}

/// Source id to entry id, as the loads recorded them.
pub fn keys(prefs: &SourcePrefs) -> BTreeMap<i64, String> {
    prefs.source_keys.get().iter().filter_map(|pair| {
        let (id, key) = pair.split_once('=')?;
        Some((id.parse().ok()?, key.to_owned()))
    }).collect()
}

/// Adds the (source id, entry id) pairs of a load to the record. Pairs of sources that did not load now are kept, so a
/// source that is off stays known.
pub fn record(prefs: &SourcePrefs, loaded: impl IntoIterator<Item = (i64, String)>) {
    let mut all = keys(prefs);
    all.extend(loaded);
    let stored: BTreeSet<String> = all.into_iter().map(|(id, key)| format!("{id}={key}")).collect();
    if let Err(error) = prefs.source_keys.set(stored) {
        tracing::warn!(%error, "the source ids could not be recorded");
    }
}

/// The ids of the sources that are known and off.
pub fn off_ids_of(prefs: &SourcePrefs) -> HashSet<i64> {
    let on = prefs.enabled_sources.get();
    keys(prefs).into_iter().filter(|(_, key)| !on.contains(key)).map(|(id, _)| id).collect()
}

/// The app's preferences, once the app data is open.
fn live() -> Option<SourcePrefs> {
    crate::data::is_installed().then(|| SourcePrefs::new(crate::data::app().base.store().clone()))
}

/// The ids of the sources that are off now; empty before the app data is open. The library update skips their titles.
pub fn off_ids() -> HashSet<i64> {
    live().map(|prefs| off_ids_of(&prefs)).unwrap_or_default()
}

/// The entry id of a repository source, when it has loaded once.
pub fn key_of(source_id: i64) -> Option<String> {
    keys(&live()?).remove(&source_id)
}

/// The entry id of source `source_id` when that source is off.
fn off_key(source_id: i64) -> Option<String> {
    let prefs = live()?;
    let key = keys(&prefs).remove(&source_id)?;
    (!is_on(&prefs, &key)).then_some(key)
}

/// Whether source `source_id` is a repository source the user turned off.
pub fn is_off(source_id: i64) -> bool {
    off_key(source_id).is_some()
}

/// The status a Check now gives a source from its first page: it works when it lists something, it is blocked when the
/// site refuses or cannot be reached from this connection, and it is broken otherwise.
pub fn checked_status(result: &Result<usize, SourceError>) -> Status {
    match result {
        Ok(found) if *found > 0 => Status::Active,
        Ok(_) => Status::Broken,
        Err(SourceError::Http(403 | 429 | 451 | 503) | SourceError::UnknownHost { .. } | SourceError::Transport(_)) => Status::Blocked,
        Err(_) => Status::Broken,
    }
}

/// Fetches the file of entry `key`, turns it on and its language with it, and loads it, so it is usable when this
/// returns. Blocks: call it off the GTK thread, with `prefs` read on it (the app data is the GTK thread's). The switch
/// stays off when the file cannot be fetched.
pub fn turn_on(prefs: &SourcePrefs, key: &str) -> Result<(), String> {
    set_on(prefs, key, true);
    if let Err(error) = crate::recipe_sources::fetch_source(key) {
        tracing::warn!(%error, key, "a source could not be turned on");
        set_on(prefs, key, false);
        return Err(error.to_string());
    }
    let indexes: Vec<Index> = prefs.manga_extension_repos.get().iter().filter_map(|link| crate::recipe_sources::saved_index(link)).collect();
    show_languages_of_on(prefs, &indexes);
    crate::recipe_sources::reload_now();
    Ok(())
}

/// Turns the source of entry `key` off: the next load leaves it out and deletes its file.
pub fn turn_off(key: &str) {
    if let Some(prefs) = live() {
        set_on(&prefs, key, false);
        crate::recipe_sources::reload();
    }
}

/// How many library titles (both kinds) come from the sources of entry `key`.
fn library_titles(key: &str) -> i64 {
    let Some(prefs) = live() else { return 0 };
    let ids: HashSet<i64> = keys(&prefs).into_iter().filter(|(_, k)| k == key).map(|(id, _)| id).collect();
    let repo = kuro_data::entry::EntryRepo::new(crate::data::app().db.clone());
    [Kind::Manga, Kind::Anime]
        .into_iter()
        .flat_map(|kind| repo.favorite_counts_by_source(kind).unwrap_or_default())
        .filter(|(source, _)| ids.contains(source))
        .map(|(_, count)| count)
        .sum()
}

/// Turns a source on off the GTK thread, then runs `done` on it with whether it worked. A failure is a toast.
pub fn turn_on_then(key: String, done: impl Fn(bool) + 'static) {
    // read here, on the GTK thread: the worker below cannot see the app data
    let prefs = live();
    gtk::glib::spawn_future_local(async move {
        let worked = match prefs {
            Some(prefs) => matches!(gtk::gio::spawn_blocking(move || turn_on(&prefs, &key)).await, Ok(Ok(()))),
            None => false,
        };
        if !worked {
            crate::toast::notify(tr(MsgKey::source_switch_failed));
        }
        done(worked);
    });
}

/// Turns a source off, asking first when library titles use it ("3 titles in your library use this source..."). `done`
/// gets whether it was turned off.
pub fn confirm_off(key: String, name: &str, done: impl Fn(bool) + 'static) {
    let count = library_titles(&key);
    if count == 0 {
        turn_off(&key);
        done(true);
        return;
    }
    let done = Rc::new(done);
    let (cancelled, confirmed) = (Rc::clone(&done), done);
    let body = crate::ui::message(&tr_plural(MsgKey::source_switch_off_body, count, &[]));
    let title = trf(MsgKey::source_switch_off_title, &[("source", Arg::Str(name))]);
    let buttons = vec![
        crate::ui::DialogButton::cancel(tr(MsgKey::action_cancel)),
        crate::ui::DialogButton::new(tr(MsgKey::source_switch_turn_off), crate::ui::Role::Destructive, move || {
            turn_off(&key);
            confirmed(true);
        }),
    ];
    crate::ui::dialog_or_dismiss(&title, &body, 420, buttons, move || cancelled(false)).present();
}

/// When source `source_id` is off: shows "This source is off" with a Turn on button, which turns it on and then runs
/// `after` (a refresh, say). Returns whether the source is off, so the caller shows no error of its own.
pub fn offer_turn_on(source_id: i64, after: impl Fn() + 'static) -> bool {
    let Some(key) = off_key(source_id) else { return false };
    let after = Rc::new(after);
    crate::toast::notify_action(&tr(MsgKey::source_switch_off), &tr(MsgKey::source_switch_turn_on), None, move || {
        let after = Rc::clone(&after);
        turn_on_then(key.clone(), move |worked| {
            if worked {
                after();
            }
        });
    });
    true
}

/// What the source lists show: the rows that match the search text, the kind and the language. Empty shows every row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourceFilter {
    pub query: String,
    /// Anime or manga only; `None` is both.
    pub kind: Option<Kind>,
    /// One language code only ("ru"); `None` is every language.
    pub lang: Option<String>,
}

impl SourceFilter {
    /// Whether a source of `kind` and `lang`, found by `text` ([`search_text`]), shows.
    pub fn shows(&self, text: &str, kind: Kind, lang: &str) -> bool {
        self.kind.is_none_or(|k| k == kind) && self.lang.as_deref().is_none_or(|l| l == lang) && crate::ui::title_matches(text, &self.query)
    }
}

/// A repository's sources, a row with a switch each, filtered by `filter`: the Repositories page shows it under the
/// repository's card, and the picker dialog on its own. With it come the language codes its sources use, for a language
/// filter. `None` when the repository has no saved index.
pub fn sources_list(link: &str, filter: Rc<std::cell::RefCell<SourceFilter>>) -> Option<(gtk::ListBox, Vec<String>)> {
    let index = crate::recipe_sources::saved_index(link)?;
    let prefs = live()?;
    let list = crate::ui::boxed_list();
    let mut entries: Vec<&Entry> = index.sources.iter().collect();
    entries.sort_by(|a, b| (a.kind != Kind::Anime, &a.lang, a.name.to_lowercase()).cmp(&(b.kind != Kind::Anime, &b.lang, b.name.to_lowercase())));
    let languages: Vec<String> = entries.iter().map(|entry| entry.lang.clone()).collect::<BTreeSet<_>>().into_iter().collect();
    // what each row is found by, in row order: rows are never removed, so a row's index names its source
    let rows: Vec<(String, Kind, String)> = entries.iter().map(|entry| (search_text(entry), entry.kind, entry.lang.clone())).collect();
    for entry in entries {
        list.append(&picker_row(&prefs, link, entry));
    }
    list.set_filter_func(move |row| {
        usize::try_from(row.index()).ok().and_then(|i| rows.get(i)).is_none_or(|(text, kind, lang)| filter.borrow().shows(text, *kind, lang))
    });
    Some((list, languages))
}

/// The list of a repository's sources with a switch each, as a dialog: shown right after the repository is added and
/// from the "new sources" notice.
pub fn open_picker(link: &str) {
    let Some(index) = crate::recipe_sources::saved_index(link) else { return };
    let filter = Rc::new(std::cell::RefCell::new(SourceFilter::default()));
    let Some((list, _)) = sources_list(link, Rc::clone(&filter)) else { return };
    let repo = index.name.clone().filter(|name| !name.trim().is_empty()).unwrap_or_else(|| link.to_owned());
    let col = gtk::Box::new(gtk::Orientation::Vertical, 12);
    col.append(&crate::ui::message(&tr(MsgKey::source_switch_picker_intro)));
    let search = gtk::SearchEntry::builder().placeholder_text(tr(MsgKey::action_search_hint)).hexpand(true).build();
    col.append(&search);
    let weak = list.downgrade();
    search.connect_search_changed(move |entry| {
        filter.borrow_mut().query = entry.text().to_string();
        if let Some(list) = weak.upgrade() {
            list.invalidate_filter();
        }
    });
    col.append(&list);
    let title = trf(MsgKey::source_switch_picker_title, &[("repo", Arg::Str(&repo))]);
    let window = crate::ui::dialog(&title, &col, 480, vec![crate::ui::DialogButton::new(tr(MsgKey::action_ok), crate::ui::Role::Suggested, || {})]);
    // type to search, as GNOME lists do: typing anywhere in the dialog goes to the search field
    search.set_key_capture_widget(Some(&window));
    window.present();
}

/// What a picker row is found by: the source's name, language, kind and description.
fn search_text(entry: &Entry) -> String {
    let kind = tr(if entry.kind == Kind::Anime { MsgKey::label_anime } else { MsgKey::label_manga });
    let language = crate::browse::names::localized_display_name(&entry.lang);
    let about = crate::recipe_sources::described(entry).unwrap_or_default();
    format!("{} {language} {kind} {about}", entry.name)
}

/// One row of the picker: icon, name, language and kind, and the switch.
fn picker_row(prefs: &SourcePrefs, link: &str, entry: &Entry) -> gtk::ListBoxRow {
    let kind = tr(if entry.kind == Kind::Anime { MsgKey::label_anime } else { MsgKey::label_manga });
    let mut subtitle = format!("{} · {kind}", crate::browse::names::localized_display_name(&entry.lang));
    if let Some(about) = crate::recipe_sources::described(entry) {
        subtitle = format!("{subtitle}\n{about}");
    }
    if let Some(dates) = crate::recipe_sources::dated(entry) {
        subtitle = format!("{subtitle}\n{dates}");
    }
    let switch = gtk::Switch::builder().active(is_on(prefs, &entry.id)).valign(gtk::Align::Center).build();
    switch.update_property(&[gtk::accessible::Property::Label(&entry.name)]);
    let spinner = crate::ui::loading(crate::ui::LoadSize::Inline);
    spinner.set_visible(false);
    let (key, name) = (entry.id.clone(), entry.name.clone());
    let spinner_weak = spinner.downgrade();
    switch.connect_state_set(move |switch, on| {
        // a state that matches the stored switch is a change this handler made: let it through
        if live().is_some_and(|prefs| is_on(&prefs, &key) == on) {
            return gtk::glib::Propagation::Proceed;
        }
        let weak = switch.downgrade();
        let spinner = spinner_weak.clone();
        if on {
            switch.set_sensitive(false);
            if let Some(spinner) = spinner.upgrade() {
                spinner.set_visible(true);
            }
            turn_on_then(key.clone(), move |worked| {
                if let Some(spinner) = spinner.upgrade() {
                    spinner.set_visible(false);
                }
                if let Some(switch) = weak.upgrade() {
                    switch.set_sensitive(true);
                    switch.set_active(worked);
                    switch.set_state(worked);
                }
            });
        } else {
            switch.set_sensitive(false);
            confirm_off(key.clone(), &name, move |off| {
                if let Some(switch) = weak.upgrade() {
                    switch.set_sensitive(true);
                    switch.set_active(!off);
                    switch.set_state(!off);
                }
            });
        }
        gtk::glib::Propagation::Stop
    });
    let suffix = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    suffix.append(&spinner);
    suffix.append(&switch);
    let row = crate::ui::row(&entry.name, Some(&subtitle), Some(suffix.upcast()));
    if let Some(icon) = crate::recipe_sources::saved_icon(link, entry) {
        if let Some(line) = row.child().and_downcast::<gtk::Box>() {
            let picture = gtk::Image::from_file(icon);
            picture.set_pixel_size(32);
            line.prepend(&picture);
        }
    }
    row
}

/// The notice of a daily check that found sources a repository added since: "2 new sources in <repo>", with Choose,
/// which opens the picker. Callable from any thread.
pub fn announce_new(link: String, repo: String, count: usize) {
    gtk::glib::MainContext::default().invoke(move || {
        let text = tr_plural(MsgKey::source_switch_new_sources, i64::try_from(count).unwrap_or(i64::MAX), &[("repo", Arg::Str(&repo))]);
        crate::toast::notify_action(&text, &tr(MsgKey::source_switch_choose), None, move || open_picker(&link));
    });
}

/// Check now: fetches the source's file again and loads its first page from this connection, then shows the status
/// from that result instead of the repository's (whose check runs on a US server, wrong for a site that blocks by
/// region). The toast says what came out.
pub fn check_now(kind: Kind, source_id: i64, name: String) {
    crate::toast::notify(trf(MsgKey::source_switch_checking, &[("source", Arg::Str(&name))]));
    gtk::glib::spawn_future_local(async move {
        let result = gtk::gio::spawn_blocking(move || crate::recipe_sources::check(kind, source_id)).await;
        let message = match result {
            Ok(Some(Status::Active)) => MsgKey::source_switch_check_works,
            Ok(Some(Status::Blocked)) => MsgKey::source_switch_check_blocked,
            Ok(Some(Status::Broken)) => MsgKey::source_switch_check_broken,
            // offline or not a repository source: nothing was learned
            _ => return,
        };
        crate::toast::notify(trf(message, &[("source", Arg::Str(&name))]));
    });
}

#[cfg(test)]
mod tests {
    use kuro_prefs::store::PrefStore;

    use super::*;

    fn index(ids: &[&str]) -> Index {
        let sources = ids.iter().map(|id| format!(
            r#"{{"id":"{id}","name":"Name {id}","lang":"en","kind":"manga","version":1,"file":"https://raw.example/{id}.toml","sha256":"{}"}}"#,
            "0".repeat(64)
        ));
        kuro_extensions::repo::parse_index(&format!(r#"{{"version":1,"sources":[{}]}}"#, sources.collect::<Vec<_>>().join(","))).expect("valid index")
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    /// Browse lists only the enabled languages: a source that is on must bring its language, or it is on and invisible.
    #[test]
    fn a_source_that_is_on_turns_its_language_on_and_one_that_is_off_does_not() {
        let entry = |id: &str, lang: &str| format!(r#"{{"id":"{id}","name":"{id}","lang":"{lang}","kind":"manga","version":1,"file":"https://raw.example/{id}.wasm","sha256":"{}"}}"#, "0".repeat(64));
        let repo = kuro_extensions::repo::parse_index(&format!(r#"{{"version":1,"sources":[{},{},{}]}}"#, entry("senkuro", "ru"), entry("usagi", "ru"), entry("raw", "ja"))).expect("valid index");
        let prefs = SourcePrefs::new(PrefStore::in_memory());
        prefs.enabled_languages.set(set(&["all", "en"])).expect("languages");
        set_on(&prefs, "senkuro", true);
        show_languages_of_on(&prefs, std::slice::from_ref(&repo));
        assert_eq!(prefs.enabled_languages.get(), set(&["all", "en", "ru"]), "ru for Senkuro; ja stays off, its source is off");
        set_on(&prefs, "senkuro", false);
        show_languages_of_on(&prefs, &[repo]);
        assert!(prefs.enabled_languages.get().contains("ru"), "turning a source off leaves the languages as the user has them");
    }

    /// The first start after the switches came: a listed source is not one the user had. Only the sources with titles
    /// in the library or the history stay on; one that did not load (id unknown) is not turned off.
    #[test]
    fn the_migration_keeps_on_only_the_sources_in_use() {
        let on = set(&["mangadex", "kagane", "senkuro", "usagi", "broken"]);
        let keys: BTreeMap<i64, String> = [(1, "mangadex"), (2, "kagane"), (3, "senkuro"), (4, "usagi")].into_iter().map(|(id, key)| (id, key.to_owned())).collect();
        let used: HashSet<i64> = [1, 2].into_iter().collect();
        assert_eq!(in_use(&on, &keys, &used), set(&["mangadex", "kagane", "broken"]));
        assert_eq!(in_use(&on, &keys, &HashSet::new()), set(&["broken"]), "nothing in use keeps only what cannot be told");
    }

    #[test]
    fn the_store_holds_the_sources_that_are_on_and_a_new_source_is_off() {
        let prefs = SourcePrefs::new(PrefStore::in_memory());
        assert!(!is_on(&prefs, "a"), "a source never turned on is off");
        set_on(&prefs, "a", true);
        set_on(&prefs, "b", true);
        set_on(&prefs, "a", false);
        assert_eq!(prefs.enabled_sources.get(), set(&["b"]));
        assert!(is_on(&prefs, "b") && !is_on(&prefs, "a"));
    }

    #[test]
    fn the_migration_turns_on_every_source_of_the_repositories_already_added_once() {
        let prefs = SourcePrefs::new(PrefStore::in_memory());
        let repos = [index(&["a", "b"]), index(&["c"])];
        assert!(migrate_once(&prefs, &repos), "an existing user's first start");
        assert_eq!(prefs.enabled_sources.get(), set(&["a", "b", "c"]));
        set_on(&prefs, "b", false);
        assert!(!migrate_once(&prefs, &[index(&["a", "b", "c", "d"])]), "it runs once");
        assert_eq!(prefs.enabled_sources.get(), set(&["a", "c"]), "a later start keeps the user's choice and a new source off");
    }

    #[test]
    fn a_new_user_starts_with_an_empty_set_that_counts_as_migrated() {
        let prefs = SourcePrefs::new(PrefStore::in_memory());
        assert!(migrate_once(&prefs, &[]));
        assert!(prefs.enabled_sources.is_set());
        assert!(!migrate_once(&prefs, &[index(&["a"])]), "a repository added later starts with its sources off");
        assert!(prefs.enabled_sources.get().is_empty());
    }

    #[test]
    fn an_aniyomi_restore_turns_on_every_known_source_but_the_disabled_ones() {
        let prefs = SourcePrefs::new(PrefStore::in_memory());
        prefs.enabled_sources.set(set(&["b"])).unwrap();
        // "a" loaded once under id 11; "b" never loaded, so only its formula id is known
        record(&prefs, [(11, "a".to_owned())]);
        let b_id = kuro_source::generate_id("Name b", "en", 1);
        prefs.disabled_manga_sources.set(set(&["11", "not an id"])).unwrap();
        prefs.disabled_anime_sources.set(set(&[&b_id.to_string()])).unwrap();
        assert!(import_aniyomi_disabled(&prefs, &[index(&["a", "b", "c"])]));
        assert_eq!(prefs.enabled_sources.get(), set(&["c"]), "a and b were disabled in Aniyomi, c is on");
        assert!(!prefs.disabled_manga_sources.is_set() && !prefs.disabled_anime_sources.is_set(), "read once");
        assert!(!import_aniyomi_disabled(&prefs, &[index(&["a", "b", "c"])]));
        assert_eq!(prefs.enabled_sources.get(), set(&["c"]));
    }

    #[test]
    fn a_restore_with_an_empty_disabled_list_turns_every_source_on() {
        let prefs = SourcePrefs::new(PrefStore::in_memory());
        prefs.enabled_sources.set(BTreeSet::new()).unwrap();
        prefs.disabled_manga_sources.set(BTreeSet::new()).unwrap();
        settle(&prefs, &[index(&["a", "b"])]);
        assert_eq!(prefs.enabled_sources.get(), set(&["a", "b"]));
    }

    #[test]
    fn the_recorded_ids_survive_a_load_without_the_source_and_name_the_off_ones() {
        let prefs = SourcePrefs::new(PrefStore::in_memory());
        record(&prefs, [(11, "a".to_owned()), (12, "a".to_owned()), (21, "b".to_owned())]);
        record(&prefs, [(21, "b".to_owned())]);
        assert_eq!(keys(&prefs).len(), 3, "a did not load this time and stays known");
        set_on(&prefs, "b", true);
        assert_eq!(off_ids_of(&prefs), HashSet::from([11, 12]));
    }

    #[test]
    fn the_filter_shows_a_source_only_when_its_text_kind_and_language_all_match() {
        let everything = SourceFilter::default();
        assert!(everything.shows("Senkuro", Kind::Manga, "ru"));
        let manga = SourceFilter { kind: Some(Kind::Manga), ..SourceFilter::default() };
        assert!(manga.shows("Senkuro", Kind::Manga, "ru") && !manga.shows("AnimeGG", Kind::Anime, "en"));
        let russian = SourceFilter { lang: Some("ru".into()), ..SourceFilter::default() };
        assert!(russian.shows("Senkuro", Kind::Manga, "ru") && !russian.shows("MangaDex", Kind::Manga, "en"));
        let both = SourceFilter { query: "ink".into(), kind: Some(Kind::Manga), lang: Some("ru".into()) };
        assert!(both.shows("InkStory", Kind::Manga, "ru") && !both.shows("Senkuro", Kind::Manga, "ru"));
    }

    #[test]
    fn a_picker_row_is_found_by_its_name_language_kind_or_description() {
        let mut entry = index(&["a"]).sources.remove(0);
        entry.name = "Senkuro".into();
        entry.lang = "ru".into();
        entry.description = Some("Chapters with teams".into());
        let text = search_text(&entry);
        let language = crate::browse::names::localized_display_name("ru");
        for query in ["senk", language.as_str(), "manga", "teams", ""] {
            assert!(crate::ui::title_matches(&text, query), "{query}");
        }
        assert!(!crate::ui::title_matches(&text, "anime"));
    }

    /// The picker draws one row per source with a switch that shows whether it is on.
    #[test]
    fn a_picker_row_has_a_switch_that_shows_the_stored_state() {
        crate::testing::with_display(|| {
            let prefs = SourcePrefs::new(PrefStore::in_memory());
            set_on(&prefs, "b", true);
            let repo = index(&["a", "b"]);
            let states: Vec<bool> = repo
                .sources
                .iter()
                .map(|entry| {
                    let row = picker_row(&prefs, "https://repo.test/index.min.json", entry);
                    picker_switch(&row).is_active()
                })
                .collect();
            assert_eq!(states, [false, true]);
        });
    }

    /// The suffix of a picker row: its spinner and its switch.
    fn picker_switch(row: &gtk::ListBoxRow) -> gtk::Switch {
        let line = row.child().and_downcast::<gtk::Box>().expect("the row's line");
        let suffix = line.last_child().and_downcast::<gtk::Box>().expect("the spinner and switch end the row");
        suffix.last_child().and_downcast::<gtk::Switch>().expect("the switch ends the suffix")
    }

    /// A turn-on shows the row's spinner and locks the switch until it ends; the row starts with the spinner hidden.
    #[test]
    fn a_picker_row_shows_a_spinner_while_a_turn_on_is_pending() {
        crate::testing::with_display(|| {
            let prefs = SourcePrefs::new(PrefStore::in_memory());
            let repo = index(&["a"]);
            let row = picker_row(&prefs, "https://repo.test/index.min.json", &repo.sources[0]);
            let switch = picker_switch(&row);
            let spinner = switch.prev_sibling().and_downcast::<gtk::Spinner>().expect("the spinner is beside the switch");
            assert!(!spinner.is_visible(), "no spinner at rest");
            switch.set_active(true);
            assert!(spinner.is_visible(), "the spinner shows while the file is fetched");
            assert!(!switch.is_sensitive(), "the switch is locked meanwhile");
        });
    }

    #[test]
    fn a_check_reads_the_first_page_as_working_blocked_or_broken() {
        assert_eq!(checked_status(&Ok(5)), Status::Active);
        assert_eq!(checked_status(&Ok(0)), Status::Broken, "an empty first page is not working");
        assert_eq!(checked_status(&Err(SourceError::Http(403))), Status::Blocked);
        assert_eq!(checked_status(&Err(SourceError::Transport("connection reset".into()))), Status::Blocked);
        assert_eq!(checked_status(&Err(SourceError::Parse("no list".into()))), Status::Broken);
        assert_eq!(checked_status(&Err(SourceError::Http(500))), Status::Broken);
    }
}
