//! Display names for the source-language filter (plan 51-i18n T-51a-034).

use std::cmp::Ordering;
use std::collections::BTreeSet;

use crate::names;
use crate::{MsgKey, current_locale, tr};

/// The label for one filter key: the three special groups, `all` ("Multi"), or the native name
/// of a language tag.
pub fn display_name(key: &str) -> String {
    match key {
        "last_used" => tr(MsgKey::last_used_source),
        "pinned" => tr(MsgKey::pinned_sources),
        "other" => tr(MsgKey::other_source),
        "all" => tr(MsgKey::multi_lang),
        tag => names::native_name(tag),
    }
}

/// Sorts language keys with `all` first, then by display name.
pub fn sort_keys(keys: &mut [String]) {
    keys.sort_by(|a, b| match (a.as_str(), b.as_str()) {
        ("all", "all") => Ordering::Equal,
        ("all", _) => Ordering::Less,
        (_, "all") => Ordering::Greater,
        _ => display_name(a).cmp(&display_name(b)),
    });
}

/// The language part of a tag: `pt-BR` gives `pt`, `zh_CN` gives `zh`.
pub fn language_part(tag: &str) -> &str {
    tag.split(['-', '_']).next().unwrap_or(tag)
}

/// The device language used by the first-run source rule (T-51a-035): the language part of the resolved
/// UI locale, never a raw environment read.
pub fn device_language() -> String {
    language_part(current_locale().tag).to_owned()
}

/// `LocaleHelper.getDefaultEnabledLanguages()`: `all`, `en` and the language subtag of `locale` in lower case. A locale
/// with no usable language gives `all` and `en` only.
pub fn default_enabled_languages(locale: &str) -> BTreeSet<String> {
    let mut languages = BTreeSet::from(["all".to_owned(), "en".to_owned()]);
    let subtag = locale.split(['.', '@', '_', '-']).next().unwrap_or_default().to_ascii_lowercase();
    let usable = (2..=3).contains(&subtag.len()) && subtag.chars().all(|c| c.is_ascii_lowercase());
    if usable && subtag != "c" && subtag != "posix" {
        languages.insert(subtag);
    }
    languages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_keys_have_their_labels() {
        assert_eq!(display_name("last_used"), "Last used");
        assert_eq!(display_name("pinned"), "Pinned");
        assert_eq!(display_name("other"), "Other");
        assert_eq!(display_name("all"), "Multi");
    }

    #[test]
    fn language_tags_use_the_native_name() {
        assert_eq!(display_name("de"), "Deutsch");
        assert_eq!(display_name("zh-CN"), "简体中文");
    }

    #[test]
    fn all_sorts_first_then_by_display_name() {
        let mut keys: Vec<String> = ["tr", "all", "de", "ar"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        sort_keys(&mut keys);
        assert_eq!(keys[0], "all");
        let rest: Vec<&str> = keys[1..].iter().map(String::as_str).collect();
        // Latin native names sort before Arabic in code-point order: Deutsch, Türkçe, العربية.
        assert_eq!(rest, ["de", "tr", "ar"]);
    }

    #[test]
    fn language_part_takes_the_text_before_the_first_separator() {
        assert_eq!(language_part("pt-BR"), "pt");
        assert_eq!(language_part("zh_CN"), "zh");
        assert_eq!(language_part("de"), "de");
    }

    #[test]
    fn device_language_is_the_language_part_of_the_resolved_locale() {
        assert_eq!(device_language(), language_part(crate::current_locale().tag));
    }
}
