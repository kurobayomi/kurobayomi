//! Key namespaces. The prefix decides the store: no prefix is a normal setting (`config.toml`),
//! `__APP_STATE_` is app state (`state.toml`), and `__PRIVATE_` is a credential (OS secret store).

pub const APP_STATE_PREFIX: &str = "__APP_STATE_";
pub const PRIVATE_PREFIX: &str = "__PRIVATE_";

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Namespace {
    Normal,
    AppState,
    Private,
}

impl Namespace {
    pub fn of(key: &str) -> Self {
        if is_app_state(key) {
            Self::AppState
        } else if is_private(key) {
            Self::Private
        } else {
            Self::Normal
        }
    }
}

pub fn is_app_state(key: &str) -> bool {
    key.starts_with(APP_STATE_PREFIX)
}

pub fn is_private(key: &str) -> bool {
    key.starts_with(PRIVATE_PREFIX)
}

pub fn app_state_key(key: &str) -> String {
    format!("{APP_STATE_PREFIX}{key}")
}

pub fn private_key(key: &str) -> String {
    format!("{PRIVATE_PREFIX}{key}")
}

/// Prefix of a Kurobayomi-only setting inside a backup (T-02a-040). Aniyomi's restorer accepts any string key, so
/// the entry restores there as an ordinary value and changes nothing else.
pub const KURO_PREFIX: &str = "kuro_";

/// Registry rows that Aniyomi does not have (no match in its source; `anime_ext_last_check` is Aniyomi's
/// `last_ext_check` under another name). They are exported with [`KURO_PREFIX`] like keys outside the registry.
pub const KURO_ONLY_KEYS: &[&str] = &["anime_ext_last_check", "kinds_shown"];

/// The backup key for a stored key. A key Aniyomi knows keeps its name. A Kurobayomi-only normal key (outside the
/// registry, or listed in [`KURO_ONLY_KEYS`]) gets [`KURO_PREFIX`]. App state and private keys are handled by their
/// own rules and are returned unchanged.
pub fn backup_key(key: &str) -> String {
    let normal = Namespace::of(key) == Namespace::Normal;
    if normal && (KURO_ONLY_KEYS.contains(&key) || crate::registry::lookup(key).is_none()) {
        format!("{KURO_PREFIX}{key}")
    } else {
        key.to_owned()
    }
}

/// The stored key for a key read from a backup: a `kuro_` key gets its prefix removed, every other key is kept.
pub fn restored_key(key: &str) -> String {
    key.strip_prefix(KURO_PREFIX).map_or_else(|| key.to_owned(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_follows_the_prefix() {
        assert_eq!(
            Namespace::of("__APP_STATE_pref_downloaded_only"),
            Namespace::AppState
        );
        assert_eq!(Namespace::of("__PRIVATE_track_token_3"), Namespace::Private);
        assert_eq!(Namespace::of("display_mode"), Namespace::Normal);
    }

    #[test]
    fn prefix_helpers_round_trip() {
        assert_eq!(
            app_state_key("incognito_mode"),
            "__APP_STATE_incognito_mode"
        );
        assert_eq!(private_key("x"), "__PRIVATE_x");
        assert!(is_app_state(&app_state_key("a")) && !is_private(&app_state_key("a")));
        assert!(is_private(&private_key("a")) && !is_app_state(&private_key("a")));
    }

    #[test]
    fn kurobayomi_only_keys_round_trip_through_the_kuro_prefix() {
        assert_eq!(backup_key("kuro_only_here"), "kuro_kuro_only_here");
        assert_eq!(restored_key(&backup_key("kuro_only_here")), "kuro_only_here");
        assert_eq!(backup_key("show_nsfw_source"), "show_nsfw_source", "a registry key keeps its name");
        assert_eq!(restored_key("show_nsfw_source"), "show_nsfw_source");
        assert_eq!(backup_key("__APP_STATE_incognito_mode"), "__APP_STATE_incognito_mode");
    }

    #[test]
    fn registry_rows_that_only_kurobayomi_has_get_the_prefix() {
        for key in KURO_ONLY_KEYS {
            assert!(crate::registry::lookup(key).is_some(), "{key} is a registry row");
            assert_eq!(backup_key(key), format!("kuro_{key}"));
            assert_eq!(restored_key(&backup_key(key)), *key);
        }
    }

    /// Port of Aniyomi `PreferenceRestorer.restorePreferences` for a store without category maps: a value is written
    /// when the key is absent or holds the same kind; anything else is skipped.
    fn aniyomi_restore(store: &mut std::collections::BTreeMap<String, crate::values::PrefValue>, backup: &[(String, crate::values::PrefValue)]) {
        for (key, value) in backup {
            let same_kind = store.get(key).is_none_or(|old| std::mem::discriminant(old) == std::mem::discriminant(value));
            if same_kind {
                store.insert(key.clone(), value.clone());
            }
        }
    }

    #[test]
    fn the_aniyomi_restorer_takes_kuro_keys_as_plain_values_and_changes_nothing_else() {
        use crate::values::PrefValue;
        let exported: Vec<(String, PrefValue)> = [
            ("show_nsfw_source".to_owned(), PrefValue::Bool(true)),
            ("anime_ext_last_check".to_owned(), PrefValue::Bool(true)),
            ("kuro_only_here".to_owned(), PrefValue::Int(9)),
        ]
        .into_iter()
        .map(|(k, v)| (backup_key(&k), v))
        .collect();
        let keys: Vec<&str> = exported.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["show_nsfw_source", "kuro_anime_ext_last_check", "kuro_kuro_only_here"]);
        let mut device: std::collections::BTreeMap<String, PrefValue> = [
            ("show_nsfw_source".to_owned(), PrefValue::Bool(false)),
            ("anime_ext_last_check".to_owned(), PrefValue::Bool(false)),
            ("unrelated".to_owned(), PrefValue::Str("keep".into())),
        ]
        .into_iter()
        .collect();
        let before = device.clone();
        aniyomi_restore(&mut device, &exported);
        assert_eq!(device["show_nsfw_source"], PrefValue::Bool(true), "the Aniyomi key restores");
        assert_eq!(device["kuro_anime_ext_last_check"], PrefValue::Bool(true), "a kuro_ key is stored as an ordinary value");
        assert_eq!(device["anime_ext_last_check"], before["anime_ext_last_check"], "the unprefixed key is not touched");
        assert_eq!(device["unrelated"], before["unrelated"]);
        assert_eq!(device.len(), before.len() + 2, "only the two kuro_ entries are new");
    }
}
