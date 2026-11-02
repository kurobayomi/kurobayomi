//! The preferred subtitle and audio language rows: a plain dropdown of languages. The stored value stays the
//! comma-separated code list the player reads (and Aniyomi backups carry), so a list typed in an older version still
//! shows up as its own entry.
use kuro_prefs::pref::Pref;

use crate::pref_model::bind;
use crate::pref_model::model::{Entry, ItemCommon, PreferenceItem};

/// The languages offered, by code; the labels are each language's own name.
const CODES: &[&str] = &[
    "en", "ja", "es", "pt", "fr", "de", "it", "ar", "ru", "zh", "ko", "id", "vi", "th", "tr", "pl", "hi",
];

/// A dropdown row bound to `pref`: Automatic (empty, the player's own choice) and one entry per language.
pub fn language_row(title: &str, pref: &Pref<String>) -> PreferenceItem {
    let value = pref.get();
    let pref = pref.clone();
    let on_change = bind::veto(move |code: String| {
        let result = pref.set(code).map(|()| true).map_err(|err| err.to_string());
        async move { result }
    });
    PreferenceItem::BasicList { common: ItemCommon::bound(title), entries: entries(&value), value, on_change }
}

fn entries(stored: &str) -> Vec<Entry> {
    let mut entries = vec![(String::new(), kuro_i18n::tr(kuro_i18n::MsgKey::language_automatic))];
    entries.extend(CODES.iter().map(|code| (code.to_string(), kuro_i18n::names::native_name(code))));
    if !stored.is_empty() && !CODES.contains(&stored) {
        entries.push((stored.to_owned(), stored.to_owned()));
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_comes_first_and_an_older_typed_list_is_kept_as_an_entry() {
        let plain = entries("ja");
        assert_eq!(plain[0], (String::new(), "Automatic".to_owned()));
        assert_eq!(plain.len(), CODES.len() + 1);
        let typed = entries("en, ja");
        assert_eq!(typed.last().map(|(code, _)| code.as_str()), Some("en, ja"));
    }
}
