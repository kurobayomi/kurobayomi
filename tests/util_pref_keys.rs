//! T-52a-065: every preference key the util code reads is registered with the
//! listed default and in the listed store.

use kuro_prefs::keys::Namespace;
use kuro_prefs::registry::{Lit, lookup};
use kuro_prefs::store::PrefStore;

/// Key, default and namespace as the plan lists them.
const SECTION_4: &[(&str, Lit, Namespace)] = &[
    ("__APP_STATE_pref_downloaded_only", Lit::Bool(false), Namespace::AppState),
    ("__APP_STATE_incognito_mode", Lit::Bool(false), Namespace::AppState),
    ("pref_theme_mode_key", Lit::Str("SYSTEM"), Namespace::Normal),
    ("pref_reader_theme_key", Lit::Int(1), Namespace::Normal),
];

#[test]
fn every_util_key_is_registered_with_its_default() {
    for (key, default, namespace) in SECTION_4 {
        let (meta, id) = lookup(key).unwrap_or_else(|| panic!("{key} is not registered"));
        assert_eq!(id, None, "{key} is not a family key");
        assert_eq!(meta.default, *default, "default of {key}");
        assert_eq!(meta.ns, *namespace, "namespace of {key}");
    }
}

#[test]
fn app_state_keys_carry_the_prefix_and_the_others_do_not() {
    for (key, _, namespace) in SECTION_4 {
        assert_eq!(key.starts_with("__APP_STATE_"), *namespace == Namespace::AppState, "{key}");
        assert_eq!(Namespace::of(key), *namespace, "{key}");
    }
}

#[test]
fn the_named_defaults_read_back_from_an_empty_store() {
    let store = PrefStore::in_memory();
    assert_eq!(store.pref("pref_reader_theme_key", 1i32).get(), 1);
}
