//! The player settings root page (task T-48a-010): the kept settings as rows, then the links to the pages that stay.
//! The rows and the link targets are in `mod.rs`.

use kuro_i18n::{Arg, MsgKey, tr, trf};
use gtk::prelude::*;
use kuro_prefs::pref::Pref;
use kuro_prefs::prefs::Prefs;

use super::language;
use crate::pref_model::bind;
use crate::pref_model::model::{ItemCommon, Preference, PreferenceItem};
use crate::pref_model::page;
use crate::screens::{built, Built};
use crate::settings_ui::rows::{group, switch};
use crate::settings_ui::search;

pub fn volume_title() -> String {
    tr(MsgKey::player_default_volume)
}
/// The stored value that means "the volume the player was last left at" (`player_volume_value`, not stored yet).
pub const LAST_USED: f32 = -1.0;

/// The volume levels offered by the Default volume row, 0 to 100 in steps of 10.
pub fn volume_levels() -> Vec<f32> {
    (0..=10).map(|step| (step * 10) as f32).collect()
}

fn label(level: f32) -> String {
    if level < 0.0 { tr(MsgKey::last_used_source).to_owned() } else { format!("{level:.0}%") }
}

/// The Default volume row. A stored level outside the list is kept, shown as `Custom (N%)` and listed first.
pub fn volume_row(pref: &Pref<f32>) -> PreferenceItem {
    let stored = pref.get();
    let in_list = stored == LAST_USED || volume_levels().iter().any(|level| (level - stored).abs() < 1e-6);
    let mut entries: Vec<(String, String)> = Vec::new();
    if !in_list {
        entries.push((format!("{stored}"), trf(MsgKey::player_volume_custom, &[("percent", Arg::Str(&format!("{stored:.0}")))])));
    }
    entries.push((format!("{LAST_USED}"), label(LAST_USED)));
    entries.extend(volume_levels().into_iter().map(|level| (format!("{level}"), label(level))));
    let value = format!("{stored}");
    let pref = pref.clone();
    let on_change = bind::veto(move |key: String| {
        let result = key
            .parse::<f32>()
            .map_err(|err| err.to_string())
            .and_then(|value| pref.set(value).map_err(|err| err.to_string()));
        async move { result.map(|()| true) }
    });
    PreferenceItem::BasicList {
        common: ItemCommon::bound(volume_title()),
        value,
        entries,
        on_change,
    }
}

/// The heights the Default quality row offers. An episode whose servers lack it plays the source's best video.
pub const QUALITIES: [i32; 5] = [2160, 1080, 720, 480, 360];

fn quality_options() -> Vec<(i32, String)> {
    QUALITIES.iter().map(|h| (*h, crate::player_view::quality_label(*h))).collect()
}

/// The root page rows, in three groups: Languages (the default for every anime; a track picked in the player is kept
/// for its anime), Playback (the automatic skips are a default the same way) and Controls.
pub fn page_rows(prefs: &Prefs) -> Vec<Preference> {
    let p = &prefs.player;
    let store = prefs.store();
    let subtitles = Pref::new(store.clone(), "pref_subtitle_lang", String::new());
    let audio = Pref::new(store.clone(), "pref_audio_lang", String::new());
    vec![
        group(
            &tr(MsgKey::settings_player_languages),
            vec![language::language_row(&tr(MsgKey::ui_subtitle_language), &subtitles), language::language_row(&tr(MsgKey::ui_audio_language), &audio)],
        ),
        group(
            &tr(MsgKey::settings_player_playback),
            vec![
                crate::settings_ui::rows::int_choice(&tr(MsgKey::player_default_quality), &p.default_quality, &quality_options(), true),
                volume_row(&p.player_volume_value),
                switch(&tr(MsgKey::ui_autoplay_next_episode), &p.autoplay_enabled, true),
                // the default for every anime; the player menu keeps a choice of its own per anime
                switch(&tr(MsgKey::player_auto_skip_openings), &p.auto_skip_opening, true),
                switch(&tr(MsgKey::player_auto_skip_endings), &p.auto_skip_ending, true),
            ],
        ),
        group(&tr(MsgKey::pref_category_controls), vec![crate::settings_ui::rows::auto_hide_bars(&prefs.base)]),
    ]
}

fn search_button() -> gtk::Widget {
    let button = crate::ui::icon_button("system-search-symbolic", &tr(MsgKey::action_search));
    button.update_property(&[gtk::accessible::Property::Label(&tr(MsgKey::action_search))]);
    button.connect_clicked(|_| {
        if let Some(page) = search::player_page() {
            crate::ui::push(&crate::ui::page(&tr(MsgKey::action_search_settings), &page.content, page.end, true));
        }
    });
    button.upcast()
}

/// The root page. The rows are read once when the page opens.
pub fn body(prefs: &Prefs) -> Option<Built> {
    let end = vec![search_button()];
    built(page::body(&page_rows(prefs)).widget, end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_prefs::store::PrefStore;

    #[test]
    fn the_root_groups_languages_playback_and_controls() {
        let prefs = Prefs::in_memory();
        let groups: Vec<(String, Vec<String>)> = page_rows(&prefs)
            .iter()
            .map(|entry| {
                let Preference::Group(group) = entry else { panic!("group") };
                (group.title.clone(), group.items.iter().map(|i| i.title().to_owned()).collect())
            })
            .collect();
        let row = |titles: &[&str]| titles.iter().map(|t| (*t).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            groups,
            [
                ("Languages".to_owned(), row(&["Subtitle language", "Audio language"])),
                ("Playback".to_owned(), row(&["Default quality", &volume_title(), "Autoplay next episode", "Skip openings automatically", "Skip endings automatically"])),
                ("Controls".to_owned(), row(&["Hide the bars automatically"])),
            ]
        );
    }

    #[test]
    fn the_default_volume_starts_at_last_used_and_offers_eleven_levels() {
        let store = PrefStore::in_memory();
        let pref = Pref::new(store, "player_volume_value", LAST_USED);
        let PreferenceItem::BasicList { value, entries, .. } = volume_row(&pref) else { panic!("a list") };
        assert_eq!(value, "-1");
        assert_eq!(entries.len(), 12);
        assert_eq!(entries[0].1, "Last used");
        assert_eq!(entries[11].1, "100%");
    }

    #[test]
    fn a_stored_level_outside_the_list_is_listed_first_as_custom() {
        let store = PrefStore::in_memory();
        let pref = Pref::new(store, "player_volume_value", 37.0f32);
        let PreferenceItem::BasicList { entries, .. } = volume_row(&pref) else { panic!("a list") };
        assert_eq!(entries[0].1, "Custom (37%)");
    }
}
