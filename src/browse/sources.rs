//! The installed-sources list: grouping and order, the Sources tab
//! actions (T-40a-007, T-40a-008, T-40b-010, T-40b-011, T-40b-012, T-40b-014). Ports
//! `GetEnabledMangaSources` and `MangaSourcesScreenModel.collectLatestSources`.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use kuro_domain::source::model::Source;

use super::names::{case_insensitive_cmp, localized_display_name, source_display_name};
#[cfg(test)]
use super::primitives::{ToastDuration, ToastSpec};

/// The local source has id 0. It is never dropped by the language filter (`LocalMangaSource.ID`).
pub const LOCAL_SOURCE_ID: i64 = 0;

/// A section header of the sources list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Section {
    LastUsed,
    /// A language code; `""` (unknown language) sorts last.
    Language(String),
}

/// One row of the sources list: a section header or a source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceRow {
    Header(Section),
    Item(Source),
}

/// The inputs of one list build, read from the preferences on the GTK thread.
pub struct ListInput<'a> {
    pub sources: &'a [Source],
    /// `source_languages`: language codes compared exactly.
    pub enabled_languages: &'a BTreeSet<String>,
    /// `hidden_catalogues`: decimal source ids.
    pub disabled: &'a BTreeSet<String>,
    /// `last_catalogue_source`, -1 for none.
    pub last_used: i64,
}

/// Section order: last used, then language codes in UTF-16 order, then `"other"` (the local source), then `""`.
pub fn section_cmp(a: &Section, b: &Section) -> Ordering {
    fn rank(section: &Section) -> u8 {
        match section {
            Section::LastUsed => 0,
            Section::Language(_) => 1,
        }
    }
    /// Where a language group sits: real languages first, then "other" (the local source), then an unknown language,
    /// so the groups titled "Other" end the list whatever the languages above them.
    fn tail(lang: &str) -> u8 {
        match lang {
            "" => 2,
            "other" => 1,
            _ => 0,
        }
    }
    match (a, b) {
        (Section::Language(x), Section::Language(y)) => tail(x).cmp(&tail(y)).then_with(|| x.encode_utf16().cmp(y.encode_utf16())),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// Builds the sectioned list. A last-used source also appears once more, as its
/// own copy under Last used (E-SL-04). Languages that are not enabled and disabled sources are dropped (E-SL-01, E-SL-02).
pub fn source_list(input: &ListInput) -> Vec<SourceRow> {
    let mut visible: Vec<&Source> = input
        .sources
        .iter()
        .filter(|s| input.enabled_languages.contains(&s.lang) || s.id == LOCAL_SOURCE_ID)
        .filter(|s| !input.disabled.contains(&s.id.to_string()))
        .collect();
    visible.sort_by(|a, b| case_insensitive_cmp(&a.name, &b.name));

    let mut flat: Vec<Source> = Vec::new();
    for source in visible {
        let mut copy = source.clone();
        copy.is_used_last = false;
        flat.push(copy.clone());
        if source.id == input.last_used {
            copy.is_used_last = true;
            flat.push(copy);
        }
    }

    let mut groups: Vec<(Section, Vec<Source>)> = Vec::new();
    for source in flat {
        let section = if source.is_used_last {
            Section::LastUsed
        } else {
            Section::Language(source.lang.clone())
        };
        match groups.iter_mut().find(|(s, _)| *s == section) {
            Some((_, items)) => items.push(source),
            None => groups.push((section, vec![source])),
        }
    }
    groups.sort_by(|(a, _), (b, _)| section_cmp(a, b));

    groups
        .into_iter()
        .flat_map(|(section, items)| {
            std::iter::once(SourceRow::Header(section)).chain(items.into_iter().map(SourceRow::Item))
        })
        .collect()
}

/// The header label of a section: the special names, or the language's own name (T-40a-033). A source with no
/// language sits under "Other".
pub fn section_title(section: &Section) -> String {
    match section {
        Section::LastUsed => source_display_name("last_used"),
        Section::Language(lang) if lang.is_empty() => source_display_name("other"),
        Section::Language(lang) => localized_display_name(lang),
    }
}

/// The language line of a row. A row in its own language group does not repeat it; the Last used group
/// mixes languages, so its rows show it.
pub fn language_line(section: &Section, source: &Source) -> Option<String> {
    match section {
        Section::Language(_) => None,
        Section::LastUsed if source.lang.is_empty() => None,
        Section::LastUsed => Some(localized_display_name(&source.lang)),
    }
}

#[cfg(test)]
/// The toast of a failed source-list subscription. The last list stays on screen (E-SL-10).
pub fn list_failure_toast() -> ToastSpec {
    ToastSpec {
        message: kuro_i18n::tr(kuro_i18n::MsgKey::internal_error),
        action: None,
        duration: ToastDuration::Seconds(3),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(id: i64, name: &str, lang: &str, supports_latest: bool) -> Source {
        let mut s = Source::new(id, name, lang);
        s.supports_latest = supports_latest;
        s
    }

    fn ids(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// Fixture: 5 is disabled, 6 is in a language that is not enabled, 4 has no language,
    /// and 0 is the local source.
    fn fixture() -> Vec<Source> {
        vec![
            src(1, "Beta", "en", true),
            src(2, "alpha", "all", false),
            src(3, "Gamma", "de", false),
            src(4, "Delta", "", false),
            src(5, "Epsilon", "de", false),
            src(6, "Zeta", "ja", false),
            src(7, "Eta", "en", true),
            src(0, "Local", "other", false),
        ]
    }

    fn build(last_used: i64, enabled: &[&str], disabled: &[&str]) -> Vec<SourceRow> {
        let all = fixture();
        let enabled = ids(enabled);
        let disabled = ids(disabled);
        source_list(&ListInput { sources: &all, enabled_languages: &enabled, disabled: &disabled, last_used })
    }

    fn headers(rows: &[SourceRow]) -> Vec<Section> {
        rows.iter()
            .filter_map(|r| match r {
                SourceRow::Header(s) => Some(s.clone()),
                SourceRow::Item(_) => None,
            })
            .collect()
    }

    fn item_ids(rows: &[SourceRow]) -> Vec<i64> {
        rows.iter()
            .filter_map(|r| match r {
                SourceRow::Item(s) => Some(s.id),
                SourceRow::Header(_) => None,
            })
            .collect()
    }

    #[test]
    fn sections_run_last_used_then_codes_with_empty_last() {
        let rows = build(7, &["en", "de", "other", "", "all"], &["5"]);
        assert_eq!(
            headers(&rows),
            vec![
                Section::LastUsed,
                Section::Language("all".into()),
                Section::Language("de".into()),
                Section::Language("en".into()),
                Section::Language("other".into()),
                Section::Language(String::new()),
            ]
        );
    }

    #[test]
    fn the_local_sources_other_group_comes_after_every_language() {
        // "ru" sorts after "other" as a code; the Other group still ends the list
        let mut all = fixture();
        all.push(src(8, "InkStory", "ru", false));
        let enabled = ids(&["en", "ru", "other", ""]);
        let rows = source_list(&ListInput { sources: &all, enabled_languages: &enabled, disabled: &ids(&[]), last_used: -1 });
        assert_eq!(
            headers(&rows),
            vec![Section::Language("en".into()), Section::Language("ru".into()), Section::Language("other".into()), Section::Language(String::new())]
        );
    }

    #[test]
    fn items_are_sorted_by_name_within_their_section() {
        let rows = build(-1, &["en", "de", "other", "", "all"], &["5"]);
        // all alpha (2), de Gamma (3), en Beta (1) and Eta (7), other Local (0), unknown language Delta (4).
        assert_eq!(item_ids(&rows), vec![2, 3, 1, 7, 0, 4]);
    }

    #[test]
    fn last_used_source_appears_twice_with_the_copy_under_last_used() {
        let rows = build(7, &["en", "de", "other", "", "all"], &[]);
        assert_eq!(item_ids(&rows).iter().filter(|id| **id == 7).count(), 2);
        assert_eq!(rows[0], SourceRow::Header(Section::LastUsed));
        match &rows[1] {
            SourceRow::Item(s) => assert!(s.is_used_last),
            other => panic!("expected the copy, got {other:?}"),
        }
    }

    #[test]
    fn hidden_languages_and_disabled_sources_are_dropped() {
        let rows = build(-1, &["en"], &["5"]);
        let shown = item_ids(&rows);
        assert!(!shown.contains(&6), "ja is not enabled");
        assert!(!shown.contains(&5), "5 is disabled");
        assert!(!shown.contains(&3), "de is not enabled");
    }

    #[test]
    fn a_disabled_last_used_source_leaves_no_copy_and_no_header() {
        let rows = build(5, &["en", "de", "other", "", "all"], &["5"]);
        assert!(!headers(&rows).contains(&Section::LastUsed));
        assert!(!item_ids(&rows).contains(&5));
    }

    #[test]
    fn the_local_source_is_kept_even_when_its_language_is_not_enabled() {
        let rows = build(-1, &["en"], &[]);
        assert!(item_ids(&rows).contains(&0));
    }

    #[test]
    fn a_failed_list_shows_the_internal_error_toast_for_three_seconds() {
        let toast = list_failure_toast();
        assert_eq!(toast.message, kuro_i18n::tr(kuro_i18n::MsgKey::internal_error));
        assert_eq!(toast.duration, ToastDuration::Seconds(3));
        assert_eq!(toast.action, None);
    }

    #[test]
    fn section_titles_use_the_four_special_names_and_the_language_name() {
        assert_eq!(section_title(&Section::LastUsed), "Last used");
        assert_eq!(section_title(&Section::Language(String::new())), "Other");
        assert_eq!(section_title(&Section::Language("all".into())), "Multi");
        assert_eq!(section_title(&Section::Language("zh-CN".into())), localized_display_name("zh-Hans"));
        assert_ne!(section_title(&Section::Language("en".into())), "en");
    }

    #[test]
    fn only_the_mixed_language_sections_repeat_the_language_on_a_row() {
        let source = src(1, "A", "en", false);
        assert_eq!(language_line(&Section::Language("en".into()), &source), None);
        assert_eq!(language_line(&Section::LastUsed, &source), Some(localized_display_name("en")));
        assert_eq!(language_line(&Section::LastUsed, &src(2, "B", "", false)), None);
    }
}
