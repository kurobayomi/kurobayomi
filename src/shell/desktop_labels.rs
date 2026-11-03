//! Localised desktop action labels (Kotlin `shortcuts.xml` labels, T-53a-012).
//!
//! The desktop entry carries the English `Name=` of each launcher action. Packaging adds one `Name[locale]=` line per
//! installed locale next to it, with the text of the matching string key. This module holds the key table, reads a locale
//! folder of Fluent files for the texts, and renders the entry.
//!
//! The English values are the entry's `Name=` lines. `Updates`, `History` and `Browse` already have keys with
//! those values. "Manga library" and "Anime library" have no key yet: the table names the keys they need
//! (`desktop_action_manga_library`, `desktop_action_anime_library`, owned by the i18n crate) and falls back to the existing
//! `label_manga_library` and `label_anime_library` texts of each locale until those keys exist.

#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::path::Path;

#[cfg(test)]
/// One launcher action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionLabel {
    /// The `[Desktop Action <id>]` group.
    pub id: &'static str,
    /// The string key the label should come from.
    pub key: &'static str,
    /// The key used while `key` does not exist in a locale.
    pub fallback_key: &'static str,
    /// The English value.
    pub english: &'static str,
}

#[cfg(test)]
/// The five actions, in the order of the desktop entry.
pub const LABELS: [ActionLabel; 5] = [
    ActionLabel { id: "show-library", key: "desktop_action_manga_library", fallback_key: "label_manga_library", english: "Manga library" },
    ActionLabel { id: "show-animelib", key: "desktop_action_anime_library", fallback_key: "label_anime_library", english: "Anime library" },
    ActionLabel { id: "show-updates", key: "label_recent_updates", fallback_key: "label_recent_updates", english: "Updates" },
    ActionLabel { id: "show-history", key: "label_recent_manga", fallback_key: "label_recent_manga", english: "History" },
    ActionLabel { id: "show-browse", key: "browse", fallback_key: "browse", english: "Browse" },
];

#[cfg(test)]
/// Looks a string key up in a locale. `None` when the locale has no such key.
pub type Lookup<'a> = &'a dyn Fn(&str, &str) -> Option<String>;

#[cfg(test)]
/// A locale tag as a desktop-entry locale: `de` stays, `pt-BR` becomes `pt_BR`. A tag with a script or a numeric region
/// (`es-419`) has no entry form and gives `None`.
pub fn desktop_locale(tag: &str) -> Option<String> {
    let mut parts = tag.split('-');
    let language = parts.next().filter(|l| (2..=3).contains(&l.len()) && l.chars().all(|c| c.is_ascii_lowercase()))?;
    match (parts.next(), parts.next()) {
        (None, _) => Some(language.to_owned()),
        (Some(region), None) if region.len() == 2 && region.chars().all(|c| c.is_ascii_alphabetic()) => {
            Some(format!("{language}_{}", region.to_ascii_uppercase()))
        }
        _ => None,
    }
}

#[cfg(test)]
/// The translated names, by action id then locale. A locale whose text equals the English one, or is empty, adds nothing.
pub fn translated_names(lookup: Lookup, tags: &[&str]) -> BTreeMap<&'static str, BTreeMap<String, String>> {
    let mut out: BTreeMap<&'static str, BTreeMap<String, String>> = BTreeMap::new();
    for label in LABELS {
        for tag in tags {
            let Some(locale) = desktop_locale(tag) else { continue };
            // the base `Name=` is English; the fallback keys of an English locale are not the English labels
            if tag.split('-').next() == Some("en") {
                continue;
            }
            let text = lookup(tag, label.key).or_else(|| lookup(tag, label.fallback_key));
            let Some(text) = text.map(|t| t.trim().replace(['\n', '\r'], " ")).filter(|t| !t.is_empty() && t != label.english) else { continue };
            out.entry(label.id).or_default().insert(locale, text);
        }
    }
    out
}

#[cfg(test)]
/// The desktop entry with a `Name[locale]=` line after the `Name=` line of every action group.
pub fn render_entry(base: &str, lookup: Lookup, tags: &[&str]) -> String {
    let names = translated_names(lookup, tags);
    let mut out = String::with_capacity(base.len());
    let mut group: Option<&str> = None;
    for line in base.lines() {
        out.push_str(line);
        out.push('\n');
        if let Some(id) = line.strip_prefix("[Desktop Action ").and_then(|l| l.strip_suffix(']')) {
            group = Some(id);
        } else if line.starts_with('[') {
            group = None;
        } else if line.starts_with("Name=")
            && let Some(translations) = group.and_then(|id| names.get(id))
        {
            for (locale, text) in translations {
                out.push_str(&format!("Name[{locale}]={text}\n"));
            }
        }
    }
    out
}

#[cfg(test)]
/// Reads `key = value` lines of one locale's Fluent files (`kurobayomi.ftl`, `kurobayomi-extra.ftl`) under `dir/<tag>/`.
/// Only single-line messages are read, which is what the five labels are.
pub fn ftl_lookup(dir: &Path) -> impl Fn(&str, &str) -> Option<String> {
    let dir = dir.to_path_buf();
    move |tag, key| {
        for file in ["kurobayomi.ftl", "kurobayomi-extra.ftl"] {
            let Ok(text) = std::fs::read_to_string(dir.join(tag).join(file)) else { continue };
            for line in text.lines() {
                if let Some(value) = line.strip_prefix(key).and_then(|rest| rest.strip_prefix(" = ")) {
                    return Some(value.to_owned());
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::process::Command;

    use super::*;

    const BASE: &str = include_str!("data/io.github.anas1412.Kurobayomi.desktop");

    fn fake(table: &'static [(&'static str, &'static str, &'static str)]) -> impl Fn(&str, &str) -> Option<String> {
        let map: HashMap<(&str, &str), &str> = table.iter().map(|(t, k, v)| ((*t, *k), *v)).collect();
        move |tag, key| map.get(&(tag, key)).map(|v| (*v).to_owned())
    }

    #[test]
    fn the_english_values_match_the_desktop_entry_and_the_parity_table() {
        let names: Vec<&str> = BASE.lines().filter_map(|l| l.strip_prefix("Name=")).skip(1).collect();
        assert_eq!(names, LABELS.map(|l| l.english));
        let ids: Vec<&str> = BASE.lines().filter_map(|l| l.strip_prefix("[Desktop Action ")?.strip_suffix(']')).collect();
        assert_eq!(ids, LABELS.map(|l| l.id));
    }

    #[test]
    fn locale_tags_become_desktop_locales() {
        assert_eq!(desktop_locale("de").as_deref(), Some("de"));
        assert_eq!(desktop_locale("pt-BR").as_deref(), Some("pt_BR"));
        assert_eq!(desktop_locale("fil").as_deref(), Some("fil"));
        assert_eq!(desktop_locale("es-419"), None);
        assert_eq!(desktop_locale("zh-Hans-CN"), None);
        assert_eq!(desktop_locale("EN"), None);
    }

    #[test]
    fn each_installed_locale_gets_its_own_name_line_after_the_english_one() {
        let lookup = fake(&[("de", "label_recent_updates", "Neues"), ("de", "browse", "Stöbern"), ("pt-BR", "browse", "Navegar")]);
        let entry = render_entry(BASE, &lookup, &["de", "pt-BR"]);
        assert!(entry.contains("[Desktop Action show-updates]\nName=Updates\nName[de]=Neues\n"));
        assert!(entry.contains("[Desktop Action show-browse]\nName=Browse\nName[de]=Stöbern\nName[pt_BR]=Navegar\n"));
        // an action without a translation keeps only the English line
        assert!(entry.contains("[Desktop Action show-history]\nName=History\nExec="));
        // the main entry's Name= is untouched
        assert!(entry.starts_with("[Desktop Entry]\nType=Application\nName=Kurobayomi\nGenericName="));
    }

    #[test]
    fn a_missing_new_key_falls_back_to_the_legacy_label_and_the_english_text_adds_nothing() {
        let lookup = fake(&[("de", "label_manga_library", "Manga-Bibliothek"), ("fr", "label_manga_library", "Manga library"), ("fr", "desktop_action_manga_library", "Manga library")]);
        let names = translated_names(&lookup, &["de", "fr"]);
        assert_eq!(names["show-library"]["de"], "Manga-Bibliothek");
        assert!(!names["show-library"].contains_key("fr"), "equal to the English value");
    }

    #[test]
    fn an_english_locale_adds_no_name_line_because_its_fallback_is_not_the_english_label() {
        let lookup = fake(&[("en", "label_manga_library", "Manga"), ("en-GB", "label_anime_library", "Anime")]);
        assert!(translated_names(&lookup, &["en", "en-GB"]).is_empty());
    }

    #[test]
    fn the_new_key_wins_over_the_legacy_one() {
        let lookup = fake(&[("de", "desktop_action_anime_library", "Anime-Bibliothek"), ("de", "label_anime_library", "Anime")]);
        assert_eq!(translated_names(&lookup, &["de"])["show-animelib"]["de"], "Anime-Bibliothek");
    }

    #[test]
    fn a_line_break_in_a_translation_cannot_add_a_key() {
        let lookup = fake(&[("de", "browse", "A\nExec=evil")]);
        let entry = render_entry(BASE, &lookup, &["de"]);
        assert!(!entry.lines().any(|l| l == "Exec=evil"));
    }

    #[cfg(unix)]
    #[test]
    fn the_real_locale_files_render_an_entry_that_the_validator_accepts() {
        let dir = format!("{}/crates/kuro-i18n/locales", env!("CARGO_MANIFEST_DIR"));
        let tags: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        let tags: Vec<&str> = tags.iter().map(String::as_str).collect();
        let lookup = ftl_lookup(Path::new(&dir));
        let entry = render_entry(BASE, &lookup, &tags);
        assert!(entry.contains("Name[de]=Neues"), "German Updates label from the real file");
        assert!(entry.lines().filter(|l| l.starts_with("Name[")).count() > 100);
        let out = tempfile::tempdir().unwrap();
        let file = out.path().join("io.github.anas1412.Kurobayomi.desktop");
        std::fs::write(&file, &entry).unwrap();
        let status = Command::new("desktop-file-validate").arg(&file).output().expect("desktop-file-validate is required for this test");
        assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));
    }
}
