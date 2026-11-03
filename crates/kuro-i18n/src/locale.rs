//! Locale tags, startup resolution, and the RTL flag (plan 51-i18n topic locale sections 1, 5, 6).

use std::str::FromStr;

use unic_langid::LanguageIdentifier;

/// The locale a catalogue was built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocaleInfo {
    pub tag: &'static str,
    pub rtl: bool,
}

impl LocaleInfo {
    pub fn new(tag: &'static str) -> Self {
        let language = tag.split('-').next().unwrap_or(tag);
        Self {
            tag,
            rtl: matches!(language, "ar" | "fa" | "he" | "ur"),
        }
    }
}

/// The widget direction to apply at startup (T-51a-015). `None` means leave GTK's default (the
/// system direction) alone; `Some(rtl)` is the value for `gtk::Widget::set_default_direction`
/// (`true` is Rtl). It is `Some` only when the chosen language's direction differs from
/// `system_rtl`. `chosen` is `current_locale()`; `system_rtl` is the direction GTK reports
/// before the override.
pub fn direction_override(chosen: LocaleInfo, system_rtl: bool) -> Option<bool> {
    (chosen.rtl != system_rtl).then_some(chosen.rtl)
}

/// Android locale directory name to BCP 47 tag, following `LocalesConfigTask` (`base` is English,
/// `-r` and `+` separate the region). The legacy Indonesian directory `in` becomes `id`.
pub fn android_dir_to_tag(dir: &str) -> String {
    if dir == "base" {
        return "en".to_string();
    }
    match dir.replace("-r", "-").replace('+', "-").as_str() {
        "in" => "id".to_string(),
        tag => tag.to_string(),
    }
}

/// Canonical tag for one requested locale string, or `None` when the entry
/// cannot name a locale (`C`, `POSIX`, empty, unparsable). Encoding and modifier are dropped.
pub fn normalise(raw: &str) -> Option<String> {
    let base = raw.trim().split(['.', '@']).next()?.replace('_', "-");
    let mut parts = base.split('-').filter(|p| !p.is_empty());
    let language = parts.next()?.to_ascii_lowercase();
    if language == "c" || language == "posix" {
        return None;
    }
    let language = match language.as_str() {
        "in" => "id",
        "iw" => "he",
        "ji" => "yi",
        other => other,
    };
    let rest: Vec<&str> = parts.collect();
    if language == "zh" {
        let traditional = rest.iter().any(|p| {
            p.eq_ignore_ascii_case("Hant")
                || ["TW", "HK", "MO"].iter().any(|r| p.eq_ignore_ascii_case(r))
        });
        return Some(if traditional { "zh-TW" } else { "zh-CN" }.to_string());
    }
    let region = rest.iter().find(|p| {
        (p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic()))
            || (p.len() == 3 && p.chars().all(|c| c.is_ascii_digit()))
    });
    let candidate = match region {
        Some(r) => format!("{language}-{r}"),
        None => language.to_string(),
    };
    LanguageIdentifier::from_str(&candidate)
        .ok()
        .map(|id| id.to_string())
}

/// The offered tag that matches `tag`: exact first, then the language alone.
fn match_offered(tag: &str, offered: &[&'static str]) -> Option<&'static str> {
    let language = tag.split('-').next().unwrap_or(tag);
    offered
        .iter()
        .copied()
        .find(|o| *o == tag)
        .or_else(|| offered.iter().copied().find(|o| *o == language))
}

/// Chooses the locale chain. A non-empty `pref` wins when it names an offered
/// locale. Otherwise the system list is tried in order. The chain is the matched tag, its language
/// (when offered and different), then `en`. Nothing matching gives `["en"]`.
pub fn resolve(pref: &str, system: &[String], offered: &[&'static str]) -> Vec<&'static str> {
    let requested = (!pref.trim().is_empty())
        .then_some(pref)
        .into_iter()
        .chain(system.iter().map(String::as_str));
    for raw in requested {
        let Some(tag) = normalise(raw) else { continue };
        let Some(matched) = match_offered(&tag, offered) else {
            continue;
        };
        let mut chain = vec![matched];
        let language = tag.split('-').next().unwrap_or(&tag);
        if let Some(parent) = offered.iter().copied().find(|o| *o == language) {
            if parent != matched {
                chain.push(parent);
            }
        }
        if !chain.contains(&"en") {
            chain.push("en");
        }
        return chain;
    }
    vec!["en"]
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFFERED: &[&str] = &[
        "en", "de", "id", "he", "pt", "pt-BR", "zh-CN", "zh-TW", "ru", "ar", "ka-GE",
    ];

    fn sys(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn normalise_table() {
        let cases = [
            ("pt_BR.UTF-8", Some("pt-BR")),
            ("zh_CN.UTF-8", Some("zh-CN")),
            ("zh_Hant_TW", Some("zh-TW")),
            ("zh", Some("zh-CN")),
            ("zh-Hans-CN", Some("zh-CN")),
            ("zh-Hant", Some("zh-TW")),
            ("zh_HK", Some("zh-TW")),
            ("in_ID", Some("id-ID")),
            ("iw", Some("he")),
            ("he_IL", Some("he-IL")),
            ("pt_PT", Some("pt-PT")),
            ("de_DE@euro", Some("de-DE")),
            ("sr_RS@latin", Some("sr-RS")),
            ("sr-Latn", Some("sr")),
            ("ka-GE", Some("ka-GE")),
            ("C", None),
            ("POSIX", None),
            ("", None),
            ("bogus tag!", None),
        ];
        for (raw, expected) in cases {
            assert_eq!(normalise(raw).as_deref(), expected, "normalise({raw:?})");
        }
    }

    #[test]
    fn resolve_chain_examples() {
        assert_eq!(
            resolve("", &sys(&["pt-BR"]), OFFERED),
            ["pt-BR", "pt", "en"]
        );
        assert_eq!(resolve("", &sys(&["zh-Hant-TW"]), OFFERED), ["zh-TW", "en"]);
    }

    #[test]
    fn resolve_language_match_and_fallback() {
        // L4: pt-PT is not offered, so pt is used.
        assert_eq!(resolve("", &sys(&["pt_PT.UTF-8"]), OFFERED), ["pt", "en"]);
        // L2: the first entry that is offered wins.
        assert_eq!(resolve("", &sys(&["xx", "de"]), OFFERED), ["de", "en"]);
        // L10: unknown language gives English.
        assert_eq!(resolve("", &sys(&["xx_YY"]), OFFERED), ["en"]);
        // L1 and L14: C and POSIX give English.
        assert_eq!(resolve("", &sys(&["C"]), OFFERED), ["en"]);
        assert_eq!(resolve("", &sys(&["POSIX"]), OFFERED), ["en"]);
        // L13: empty entries are skipped.
        assert_eq!(resolve("", &sys(&["", "de"]), OFFERED), ["de", "en"]);
        // L15: modifier stripped.
        assert_eq!(resolve("", &sys(&["de_DE@euro"]), OFFERED), ["de", "en"]);
        // L7: bare Chinese is Simplified.
        assert_eq!(resolve("", &sys(&["zh"]), OFFERED), ["zh-CN", "en"]);
    }

    #[test]
    fn resolve_legacy_codes() {
        // L8: in_ID is Indonesian (offered as id).
        assert_eq!(resolve("", &sys(&["in_ID"]), OFFERED), ["id", "en"]);
        // L9: he_IL is Hebrew.
        assert_eq!(resolve("", &sys(&["he_IL"]), OFFERED), ["he", "en"]);
    }

    #[test]
    fn resolve_pref_wins_and_unknown_pref_is_ignored() {
        // L11: a saved tag that is not offered is treated as not set.
        assert_eq!(resolve("xx", &sys(&["de"]), OFFERED), ["de", "en"]);
        // L12: an unparsable saved tag is ignored.
        assert_eq!(resolve("bogus tag!", &sys(&["de"]), OFFERED), ["de", "en"]);
        // A valid saved tag beats the system list.
        assert_eq!(resolve("ru", &sys(&["de"]), OFFERED), ["ru", "en"]);
        assert_eq!(resolve("ar", &sys(&["de"]), OFFERED), ["ar", "en"]);
    }

    #[test]
    fn rtl_flag() {
        for tag in ["ar", "fa", "he", "ur"] {
            assert!(LocaleInfo::new(tag).rtl, "{tag}");
        }
        for tag in ["en", "de", "ka-GE", "zh-TW"] {
            assert!(!LocaleInfo::new(tag).rtl, "{tag}");
        }
    }

    #[test]
    fn direction_override_follows_the_chosen_language() {
        // R1: ar chosen, system LTR (en) -> Rtl.
        assert_eq!(direction_override(LocaleInfo::new("ar"), false), Some(true));
        // R2: RTL system, en chosen -> Ltr.
        assert_eq!(direction_override(LocaleInfo::new("en"), true), Some(false));
        // Same direction: nothing to override.
        assert_eq!(direction_override(LocaleInfo::new("he"), true), None);
        assert_eq!(direction_override(LocaleInfo::new("de"), false), None);
        for tag in ["ar", "fa", "he", "ur"] {
            assert_eq!(
                direction_override(LocaleInfo::new(tag), false),
                Some(true),
                "{tag}"
            );
        }
    }
}
