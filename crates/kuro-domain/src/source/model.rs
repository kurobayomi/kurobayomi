//! The source value model: visual name, the stable key and the last-used key (`Source.kt`, `AnimeSource.kt`).
//! Manga and anime share this shape.

use kuro_platform::kind::Kind;

pub type Id = i64;

/// The name the source area uses for [`Source`] (`SourceEntry`, area 17 section 2.2). One type serves both kinds.
pub type SourceEntry = Source;

pub trait SourceKindExt {
    /// The table that holds this kind's stub rows (T-17a-004).
    fn stub_table(self) -> &'static str;
}

impl SourceKindExt for Kind {
    fn stub_table(self) -> &'static str {
        match self {
            Self::Manga => "stub_sources",
            Self::Anime => "stub_sources",
        }
    }
}

/// An installed or stub source as the lists show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub id: Id,
    pub lang: String,
    pub name: String,
    pub supports_latest: bool,
    /// `false` for a source with no popular listing: Browse opens it on Latest.
    pub supports_popular: bool,
    pub is_stub: bool,
    pub is_used_last: bool,
}

impl Source {
    /// A source with the defaults of both constructors: not used last.
    pub fn new(id: Id, name: &str, lang: &str) -> Self {
        Self {
            id,
            lang: lang.to_owned(),
            name: name.to_owned(),
            supports_latest: false,
            supports_popular: true,
            is_stub: false,
            is_used_last: false,
        }
    }

    /// `name`, or `name (LANG)` when the language is set.
    pub fn visual_name(&self) -> String {
        if self.lang.is_empty() { self.name.clone() } else { format!("{} ({})", self.name, self.lang.to_uppercase()) }
    }

    /// `{id}`, or `{id}-lastused` for the source used last.
    pub fn key(&self) -> String {
        if self.is_used_last { format!("{}-lastused", self.id) } else { self.id.to_string() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_names_its_own_stub_table() {
        assert_eq!(Kind::Manga.stub_table(), "stub_sources");
        assert_eq!(Kind::Anime.stub_table(), "stub_sources");
        let entry: SourceEntry = Source::new(3, "Named", "en");
        assert_eq!(entry.visual_name(), "Named (EN)");
    }

    #[test]
    fn visual_name_adds_the_upper_case_language_when_present() {
        assert_eq!(Source::new(1, "Example", "").visual_name(), "Example");
        assert_eq!(Source::new(1, "Example", "en").visual_name(), "Example (EN)");
        assert_eq!(Source::new(1, "Example", "pt-BR").visual_name(), "Example (PT-BR)");
    }

    #[test]
    fn key_adds_the_last_used_suffix_only_when_used_last() {
        let mut s = Source::new(42, "Example", "en");
        assert_eq!(s.key(), "42");
        s.is_used_last = true;
        assert_eq!(s.key(), "42-lastused");
    }

    #[test]
    fn constructor_defaults_match_the_upstream_constructors() {
        let s = Source::new(7, "n", "");
        assert!(!s.is_used_last);
        assert!(!s.is_stub && !s.supports_latest);
    }
}
