//! Localisation for Kurobayomi: the Fluent catalogue, locale resolution, and typed message keys.
//!
//! Plan 51-i18n. `tr`, `trf` and `tr_plural` return text for a [`MsgKey`]. The catalogue is
//! built once (see [`init`]). Before `init` runs, English is used. GTK is not a dependency here.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use fluent_bundle::{FluentArgs, FluentValue};
use tracing::{debug, warn};

pub mod catalog;
pub mod date;
mod dropped;
pub mod import;
pub mod locale;
pub mod names;
pub mod notice;
pub mod number;
pub mod picker;
pub mod source_names;
pub mod time;

mod keys {
    include!(concat!(env!("OUT_DIR"), "/keys.rs"));
}

mod sources {
    include!(concat!(env!("OUT_DIR"), "/locales.rs"));
}

pub use keys::{MSG_COUNT, MsgKey};
pub use locale::LocaleInfo;

use catalog::Catalog;

/// A message argument. `Str` is for `%s` and `%c`, `Int` for `%d`, `Dec` for `%.Nf`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Arg<'a> {
    Str(&'a str),
    Int(i64),
    Dec(f64),
}

static CATALOG: OnceLock<Catalog> = OnceLock::new();
static WARNED: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);

/// Sets the language for the whole process. `pref` is the `app_language` preference (empty for
/// Default). `system` is the list from `glib::language_names()`, in priority order. A second call,
/// or a call after the first `tr`, is ignored with a warning.
pub fn init(pref: &str, system: &[String]) {
    let offered = offered_locales();
    let chain = locale::resolve(pref, system, &offered);
    install(&CATALOG, &chain, sources::LOCALE_SOURCES);
}

/// The language tags the catalogue ships, for the language picker (`picker::rows`,
/// `picker::settings_subtitle`) and for [`init`].
pub fn offered_locales() -> Vec<&'static str> {
    sources::LOCALE_SOURCES
        .iter()
        .map(|(tag, _)| *tag)
        .collect()
}

fn install(slot: &OnceLock<Catalog>, chain: &[&'static str], sources: &[(&str, &str)]) -> bool {
    if slot.get().is_some() {
        warn!("[i18n] init ignored: the catalogue is already in use");
        return false;
    }
    if slot.set(Catalog::build(chain, sources)).is_err() {
        warn!("[i18n] init ignored: the catalogue was set concurrently");
        return false;
    }
    true
}

fn catalog() -> &'static Catalog {
    CATALOG.get_or_init(|| Catalog::build(&["en"], sources::LOCALE_SOURCES))
}

/// The active locale. English before `init`.
pub fn current_locale() -> LocaleInfo {
    catalog().info()
}

/// Plain message (`<string>`).
///
/// An unknown key does not compile, because [`MsgKey`] is generated from the English file:
///
/// ```compile_fail
/// let _ = kuro_i18n::tr(kuro_i18n::MsgKey::no_such_message);
/// ```
pub fn tr(key: MsgKey) -> String {
    format_key(key, None)
}

/// Message with named arguments `arg1`, `arg2`, ... (`<string>` with `%s`, `%d`, `%.Nf`).
pub fn trf(key: MsgKey, args: &[(&str, Arg<'_>)]) -> String {
    format_key(key, Some(&to_args(args, None)))
}

/// Plural message (`<plurals>`). `count` selects the CLDR category and is available as `$count`.
/// Other arguments are `arg1`, `arg2`, ... as for [`trf`].
pub fn tr_plural(key: MsgKey, count: i64, args: &[(&str, Arg<'_>)]) -> String {
    format_key(key, Some(&to_args(args, Some(count))))
}

/// The message whose English text is exactly `english`, in the active language; `None` when no message reads so.
///
/// For text that comes from data and keeps its English as its identity, such as the UI spec files (`spec/*.json`), whose
/// titles also name the stored preference of a row: the data stays English and only what is drawn goes through here.
/// Messages with arguments or plural forms are not in the lookup. Where two messages read the same English, the first in
/// the English file wins. Never use it on text the user or a source wrote (titles, names): it would translate a manga
/// called "Library".
pub fn translate_known(english: &str) -> Option<String> {
    static BY_ENGLISH: OnceLock<std::collections::HashMap<String, Vec<MsgKey>>> = OnceLock::new();
    let index = BY_ENGLISH.get_or_init(|| {
        let english_only = Catalog::build(&["en"], sources::LOCALE_SOURCES);
        let mut index: std::collections::HashMap<String, Vec<MsgKey>> = std::collections::HashMap::new();
        for key in MsgKey::ALL {
            if !key.args().is_empty() {
                continue;
            }
            if let Some((text, errors)) = english_only.format(key.id(), None) {
                if errors.is_empty() {
                    index.entry(text).or_default().push(*key);
                }
            }
        }
        index
    });
    let candidates = index.get(english)?;
    // Several messages may read the same English (the word "Display" is a heading and an action). The first one that the
    // active language has translated wins, so a word is not left in English because its first message was never translated.
    let translated = candidates.iter().map(|key| tr(*key)).find(|text| text != english);
    translated.or_else(|| candidates.first().map(|key| tr(*key)))
}

fn to_args<'a>(args: &[(&str, Arg<'a>)], count: Option<i64>) -> FluentArgs<'a> {
    let mut out = FluentArgs::with_capacity(args.len() + 1);
    if let Some(count) = count {
        out.set("count", FluentValue::from(count));
    }
    for (name, arg) in args {
        let value = match *arg {
            Arg::Str(s) => FluentValue::from(s),
            Arg::Int(i) => FluentValue::from(i),
            Arg::Dec(d) => FluentValue::from(d),
        };
        out.set(name.to_string(), value);
    }
    out
}

fn format_key(key: MsgKey, args: Option<&FluentArgs>) -> String {
    let id = key.id();
    match catalog().format(id, args) {
        Some((text, errors)) => {
            for error in errors {
                debug!("[i18n] {id}: {error}");
            }
            text
        }
        None => {
            if warn_once(id) {
                warn!("[i18n] missing message {id}");
            }
            id.to_string()
        }
    }
}

/// True the first time `id` is reported.
fn warn_once(id: &'static str) -> bool {
    let mut guard = WARNED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.get_or_insert_with(HashSet::new).insert(id)
}

#[cfg(test)]
mod import_tests;
#[cfg(test)]
mod tests;
