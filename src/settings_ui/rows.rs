//! Row builders shared by the settings pages. Each one binds a row to a typed preference; a value the list does not
//! offer shows the preference default (the stored value is kept).
// Not reachable from `route.rs` yet: text rows are used by pages that are not routed yet.
use kuro_i18n::{MsgKey, tr};
use std::collections::BTreeSet;
use std::fmt::Debug;
use std::rc::Rc;

use kuro_prefs::pref::Pref;
use kuro_prefs::values::PrefType;

use crate::pref_model::bind::{self, Veto};
use crate::pref_model::model::{ItemCommon, Preference, PreferenceGroup, PreferenceItem};

pub fn group(title: &str, items: Vec<PreferenceItem>) -> Preference {
    Preference::Group(PreferenceGroup::new(title, items))
}

pub fn item(item: PreferenceItem) -> Preference {
    Preference::Item(item)
}

#[cfg(test)]
/// Drops the rows that are not enabled (Android hides them instead of greying them out).
pub fn visible(items: Vec<PreferenceItem>) -> Vec<PreferenceItem> {
    items.into_iter().filter(PreferenceItem::enabled).collect()
}

fn common(title: &str, subtitle: Option<&str>, enabled: bool) -> ItemCommon {
    let mut common = ItemCommon::new(title);
    common.subtitle = subtitle.map(str::to_owned);
    common.enabled = enabled;
    common
}

pub fn text(title: &str, subtitle: Option<&str>, enabled: bool) -> PreferenceItem {
    PreferenceItem::Text { common: common(title, subtitle, enabled), on_click: None }
}

#[allow(dead_code)] // kept for the shape of the type; not built or read yet
/// The live parts of a row built by [`tap_item`], handed to its tap handler.
#[derive(Clone)]
pub struct TapRow {
    /// The row, whose subtitle the handler may change (`widgets::row::set_subtitle`).
    pub row: adw::ActionRow,
    /// The widget [`tap_item`]'s `trailing` builder made, if there was one.
    pub trailing: Option<gtk::Widget>,
}

/// A custom row that can be tapped (mouse, Enter or Space) and shows what a plain text row cannot: a trailing widget
/// (a check mark, a spinner) and a subtitle the handler may change. A disabled row ignores taps.
pub fn tap_item(
    title: &str,
    subtitle: Option<&str>,
    enabled: bool,
    trailing: Option<Rc<dyn Fn() -> gtk::Widget>>,
    logo: Option<&'static [u8]>,
    on_tap: Rc<dyn Fn(&TapRow)>,
) -> PreferenceItem {
    use crate::widgets::row::{self, RowSpec};
    use adw::prelude::*;
    let (label, sub) = (title.to_owned(), subtitle.map(str::to_owned));
    PreferenceItem::Custom {
        title: title.to_owned(),
        build: Rc::new(move || {
            let end = trailing.as_ref().map(|make| make());
            let built = row::build(RowSpec { title: &label, subtitle: sub.as_deref(), icon: None, trailing: end.as_ref(), on_click: None });
            if let Some(image) = logo.and_then(logo_image) {
                built.add_prefix(&image);
            }
            built.set_sensitive(enabled);
            // the handler gets the row back with the trailing widget; a weak handle keeps the row from owning itself
            built.set_activatable(true);
            let (weak, on_tap) = (built.downgrade(), Rc::clone(&on_tap));
            crate::widgets::row_activate::on_activate(&built, move |_| {
                if let Some(row) = weak.upgrade() {
                    on_tap(&TapRow { row, trailing: end.clone() });
                }
            });
            built.upcast()
        }),
    }
}

/// A tracker logo (bundled PNG or SVG bytes) as a 24 px picture, or `None` when the bytes cannot be decoded (the row
/// then shows no logo).
pub(crate) fn logo_image(bytes: &'static [u8]) -> Option<gtk::Image> {
    use gtk::prelude::*;
    let texture = gtk::gdk::Texture::from_bytes(&gtk::glib::Bytes::from_static(bytes)).ok()?;
    let image = gtk::Image::from_paintable(Some(&texture));
    image.set_pixel_size(24);
    image.set_margin_end(16);
    image.set_accessible_role(gtk::AccessibleRole::Presentation);
    Some(image)
}

pub fn info(text: &str) -> PreferenceItem {
    PreferenceItem::Info { text: text.to_owned(), enabled: true }
}

#[cfg(test)]
/// A single-line text field. The dialog OK needs a changed, non-blank (unless `can_be_blank`) and valid text; a text
/// that fails `validate` is refused with its message, and a blank one that is not allowed is not applied.
pub fn edit_text(
    title: &str,
    pref: &Pref<String>,
    can_be_blank: bool,
    validate: fn(&str) -> Result<(), String>,
    enabled: bool,
) -> PreferenceItem {
    PreferenceItem::EditText {
        common: common(title, Some("%s"), enabled),
        pref: pref.clone(),
        dialog_subtitle: None,
        can_be_blank,
        validate: Rc::new(move |text: &str| validate(text).is_ok()),
        error_message: Some(Rc::new(move |text: &str| validate(text).err().unwrap_or_default())),
        veto: text_veto(pref, can_be_blank, validate),
    }
}

/// The write of a text row: a blank text that is not allowed is not applied (`Ok(false)`), an invalid one is an error.
pub(crate) fn text_veto(pref: &Pref<String>, can_be_blank: bool, validate: fn(&str) -> Result<(), String>) -> Veto<String> {
    let pref = pref.clone();
    bind::veto(move |text: String| {
        let result = if !can_be_blank && text.trim().is_empty() {
            Ok(false)
        } else {
            validate(&text).and_then(|()| pref.set(text).map(|()| true).map_err(|err| err.to_string()))
        };
        async move { result }
    })
}

/// The one auto-hide switch of the reader and the player (`auto_hide_bars`): both settings pages show this row.
pub fn auto_hide_bars(base: &kuro_prefs::base::BasePrefs) -> PreferenceItem {
    switch_with(
        &tr(MsgKey::ui_hide_the_bars_automatically),
        Some(&tr(MsgKey::ui_while_reading_or_watching_the_top_bar_and_th)),
        &base.auto_hide_bars,
        true,
        bind::accept(),
    )
}

pub fn switch(title: &str, pref: &Pref<bool>, enabled: bool) -> PreferenceItem {
    switch_with(title, None, pref, enabled, bind::accept())
}

pub fn switch_with(title: &str, subtitle: Option<&str>, pref: &Pref<bool>, enabled: bool, veto: Veto<bool>) -> PreferenceItem {
    PreferenceItem::Switch { common: common(title, subtitle, enabled), pref: pref.clone(), veto }
}

#[cfg(test)]
/// A switch that, when turned on, turns `partner` off first (Aniyomi split and rotate are exclusive).
pub fn exclusive_switch(title: &str, pref: &Pref<bool>, partner: &Pref<bool>, enabled: bool) -> PreferenceItem {
    let partner = partner.clone();
    let veto = bind::veto(move |on: bool| {
        let result = if on { partner.set(false).map_err(|err| err.to_string()) } else { Ok(()) };
        async move { result.map(|()| true) }
    });
    switch_with(title, None, pref, enabled, veto)
}

/// A list of `i32` codes (Aniyomi stores these as ints).
pub fn int_choice<S: AsRef<str>>(title: &str, pref: &Pref<i32>, options: &[(i32, S)], enabled: bool) -> PreferenceItem {
    let stored = pref.get();
    let current = if options.iter().any(|(code, _)| *code == stored) { stored } else { pref.default_value() };
    let entries = options.iter().map(|(code, label)| (code.to_string(), label.as_ref().to_owned())).collect();
    let pref = pref.clone();
    let on_change = bind::veto(move |key: String| {
        let result = key
            .parse::<i32>()
            .map_err(|err| err.to_string())
            .and_then(|code| pref.set(code).map_err(|err| err.to_string()));
        async move { result.map(|()| true) }
    });
    PreferenceItem::BasicList { common: common(title, Some("%s"), enabled), value: current.to_string(), entries, on_change }
}

/// [`int_choice`] whose options are the values `kuro_prefs::ranges` allows for the preference's key, so the widget and
/// the stored-value rules cannot disagree (T-02a-036). `label` names each value.
pub fn ranged_choice(title: &str, pref: &Pref<i32>, label: impl Fn(i32) -> String, enabled: bool) -> PreferenceItem {
    let constraint = kuro_prefs::ranges::constraint(pref.key()).unwrap_or_else(|| panic!("{} has no entry in kuro_prefs::ranges", pref.key()));
    let options: Vec<(i32, String)> = constraint.values().into_iter().map(|v| (v as i32, label(v as i32))).collect();
    let borrowed: Vec<(i32, &str)> = options.iter().map(|(code, text)| (*code, text.as_str())).collect();
    int_choice(title, pref, &borrowed, enabled)
}

/// The subtitle of a row whose change applies at the next launch (task T-47a-010).
pub fn requires_restart() -> String {
    tr(MsgKey::requires_app_restart)
}

/// A list of a Rust enum stored by its constant name (`pref_enum!` types).
pub fn enum_choice<T>(title: &str, pref: &Pref<T>, options: &[(T, &str)], enabled: bool) -> PreferenceItem
where
    T: PrefType + Copy + Debug + PartialEq + 'static,
{
    let stored = pref.get();
    let current = if options.iter().any(|(value, _)| *value == stored) { stored } else { pref.default_value() };
    let entries = options.iter().map(|(value, label)| (format!("{value:?}"), (*label).to_owned())).collect();
    let values: Vec<T> = options.iter().map(|(value, _)| *value).collect();
    let pref = pref.clone();
    let on_change = bind::veto(move |key: String| {
        let result = values
            .iter()
            .find(|value| format!("{value:?}") == key)
            .copied()
            .ok_or_else(|| format!("unknown option {key}"))
            .and_then(|value| pref.set(value).map_err(|err| err.to_string()));
        async move { result.map(|()| true) }
    });
    PreferenceItem::BasicList {
        common: common(title, Some("%s"), enabled),
        value: format!("{current:?}"),
        entries,
        on_change,
    }
}

/// A multi-choice list whose selected keys are stored as a set of strings. `options` are `(key, label)` pairs; the
/// subtitle lists the selected labels.
pub fn multi_choice(title: &str, pref: &Pref<BTreeSet<String>>, options: &[(&str, &str)], enabled: bool) -> PreferenceItem {
    multi_choice_with(common(title, Some("%s"), enabled), pref, options)
}

/// [`multi_choice`] with its own subtitle template (`Restrictions: %s`), for a row whose subtitle names its labels.
pub fn multi_choice_with(common: ItemCommon, pref: &Pref<BTreeSet<String>>, options: &[(&str, &str)]) -> PreferenceItem {
    let entries = options.iter().map(|(key, label)| ((*key).to_owned(), (*label).to_owned())).collect();
    let writer = pref.clone();
    let veto = bind::veto(move |selected: BTreeSet<String>| {
        let result = writer.set(selected).map(|()| true).map_err(|err| err.to_string());
        async move { result }
    });
    PreferenceItem::MultiSelect { common, pref: pref.clone(), entries, veto }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_prefs::ui::ThemeMode;
    use kuro_prefs::prefs::Prefs;

    #[test]
    fn visible_drops_disabled_rows_and_keeps_the_rest() {
        let prefs = Prefs::in_memory();
        let rows = vec![
            switch("On", &prefs.reader.crop_borders, true),
            switch("Off", &prefs.reader.crop_borders, false),
        ];
        let kept = visible(rows);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].title(), "On");
    }

    #[test]
    fn ranged_choice_offers_exactly_the_values_the_ranges_allow() {
        let prefs = Prefs::in_memory();
        let PreferenceItem::BasicList { entries, .. } = ranged_choice("Max", &prefs.library.auto_update_interval, |h| format!("{h}h"), true) else {
            panic!("expected a list");
        };
        let codes: Vec<&str> = entries.iter().map(|(code, _)| code.as_str()).collect();
        assert_eq!(codes, ["0", "12", "24", "48", "72", "168"]);
        assert_eq!(entries[1].1, "12h");
    }

    #[test]
    fn int_choice_shows_the_stored_code_or_the_default_for_an_unlisted_one() {
        let prefs = Prefs::in_memory();
        prefs.reader.reader_theme.set(3).unwrap();
        let PreferenceItem::BasicList { value, .. } = int_choice("Background", &prefs.reader.reader_theme, &[(1, "Black"), (3, "Auto")], true) else {
            panic!("expected a list");
        };
        assert_eq!(value, "3");
        prefs.reader.reader_theme.set(9).unwrap();
        let PreferenceItem::BasicList { value, .. } = int_choice("Background", &prefs.reader.reader_theme, &[(1, "Black"), (3, "Auto")], true) else {
            panic!("expected a list");
        };
        assert_eq!(value, "1");
        // the unlisted stored value is not rewritten by reading it
        assert_eq!(prefs.reader.reader_theme.get(), 9);
    }

    #[test]
    fn list_choice_writes_the_selected_entry_through_its_veto() {
        let prefs = Prefs::in_memory();
        let PreferenceItem::BasicList { on_change, .. } = int_choice("Background", &prefs.reader.reader_theme, &[(0, "White"), (2, "Gray")], true) else {
            panic!("expected a list");
        };
        let accepted = futures_block(on_change("2".to_owned()));
        assert_eq!(accepted, Ok(true));
        assert_eq!(prefs.reader.reader_theme.get(), 2);
    }

    #[test]
    fn enum_choice_stores_the_variant_name_and_rejects_unknown_keys() {
        let prefs = Prefs::in_memory();
        let options = [(ThemeMode::LIGHT, "Light"), (ThemeMode::DARK, "Dark")];
        let PreferenceItem::BasicList { on_change, entries, .. } = enum_choice("Theme mode", &prefs.ui.theme_mode, &options, true) else {
            panic!("expected a list");
        };
        assert_eq!(entries[1], ("DARK".to_owned(), "Dark".to_owned()));
        assert!(futures_block(on_change("NOPE".to_owned())).is_err());
    }

    #[test]
    fn exclusive_switch_turns_the_partner_off_when_turned_on() {
        let prefs = Prefs::in_memory();
        prefs.reader.crop_borders.set(true).unwrap();
        let PreferenceItem::Switch { veto, .. } = exclusive_switch("Split", &prefs.reader.dual_page_split_paged, &prefs.reader.crop_borders, true) else {
            panic!("expected a switch");
        };
        assert_eq!(futures_block(veto(true)), Ok(true));
        assert!(!prefs.reader.crop_borders.get());
    }

    #[test]
    fn edit_text_applies_a_valid_text_and_refuses_blank_or_invalid_ones() {
        fn short(text: &str) -> Result<(), String> {
            if text.len() <= 3 { Ok(()) } else { Err("too long".to_owned()) }
        }
        let prefs = Prefs::in_memory();
        let store = Pref::new(prefs.store().clone(), "test_edit_text", "start".to_owned());
        let PreferenceItem::EditText { validate, error_message, veto, .. } = edit_text("Name", &store, false, short, true) else {
            panic!("expected an edit row");
        };
        assert!(validate("ab") && !validate("abcd"));
        assert_eq!(error_message.map(|msg| msg("abcd")), Some("too long".to_owned()));
        assert_eq!(futures_block(veto("   ".to_owned())), Ok(false));
        assert_eq!(store.get(), "start");
        assert_eq!(futures_block(veto("abcd".to_owned())), Err("too long".to_owned()));
        assert_eq!(futures_block(veto("ab".to_owned())), Ok(true));
        assert_eq!(store.get(), "ab");
    }

    /// Runs a veto future to completion without a GTK main loop (the futures resolve immediately).
    fn futures_block<F: std::future::Future>(future: F) -> F::Output {
        let waker = std::task::Waker::noop();
        let mut context = std::task::Context::from_waker(waker);
        let mut future = std::pin::pin!(future);
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(value) => value,
            std::task::Poll::Pending => panic!("veto future is not immediately ready"),
        }
    }
}
