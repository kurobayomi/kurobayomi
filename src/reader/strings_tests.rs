//! The reader's scope strings are message keys of the English catalog (T-43b-057, `p2-behavior.md` section 15). These tests
//! read the base catalog that `kuro-i18n` ships, so a key removed or renamed there fails here.

const EN_CATALOG: &str = include_str!("../../crates/kuro-i18n/locales/en/kurobayomi.ftl");

/// The keys of section 15 that have one message each (the plural `pref_pages` is checked on its own).
const SCOPE_KEYS: &[&str] = &[
    "label_default",
    "scale_type_fit_screen",
    "scale_type_stretch",
    "scale_type_fit_width",
    "scale_type_fit_height",
    "scale_type_original_size",
    "scale_type_smart_fit",
    "zoom_start_automatic",
    "zoom_start_left",
    "zoom_start_right",
    "zoom_start_center",
    "nav_zone_prev",
    "nav_zone_next",
    "nav_zone_left",
    "nav_zone_right",
    "action_menu",
    "left_to_right_viewer",
    "right_to_left_viewer",
    "vertical_viewer",
    "webtoon_viewer",
    "vertical_plus_viewer",
    "pref_highest",
    "pref_high",
    "pref_low",
    "pref_lowest",
    "pref_preserve_reading_position",
    "pref_show_page_number",
    "pref_double_tap_anim_speed",
    "pref_show_reading_mode",
];

fn has_key(key: &str) -> bool {
    EN_CATALOG.lines().any(|line| line.split_once(" =").is_some_and(|(name, _)| name == key))
}

/// The block of one message: its key line and the indented lines under it.
fn block(key: &str) -> Option<&'static str> {
    let start = EN_CATALOG.find(&format!("\n{key} ="))? + 1;
    let rest = &EN_CATALOG[start..];
    let end = rest.find("\n\n").unwrap_or(rest.len());
    Some(&rest[..end])
}

#[test]
fn every_scope_key_is_in_the_base_catalog() {
    let missing: Vec<&str> = SCOPE_KEYS.iter().copied().filter(|key| !has_key(key)).collect();
    assert!(missing.is_empty(), "scope keys missing from the English catalog: {missing:?}");
}

#[test]
fn the_pages_plural_has_every_form_the_english_catalog_uses() {
    let text = block("pref_pages").expect("pref_pages is in the catalog");
    assert!(text.contains("[one]"), "the singular form is missing: {text}");
    assert!(text.contains("*[other]"), "the default plural form is missing: {text}");
}

