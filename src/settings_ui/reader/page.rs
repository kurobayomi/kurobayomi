//! Reader settings rows: the six kept reader settings (settings diet, 2026-10-11). The top row is the default reading
//! mode; the Display group holds the background colour; the Paged group holds the scale type, the zoom start position,
//! crop borders and split wide pages. Every other reader setting is cut.
use kuro_i18n::{MsgKey, tr};
use kuro_prefs::prefs::Prefs;
use kuro_prefs::pref::Pref;
use kuro_prefs::reader::{MangaReaderPrefs, ReaderPrefs};

use crate::pref_model::model::Preference;
use crate::settings_ui::rows::{group, int_choice, item, switch};

/// `ReadingMode` codes (`pref_default_reading_mode_key`); 0 (Default) is not offered.
pub fn reading_modes() -> [(i32, String); 5] {
    [
        (1, tr(MsgKey::left_to_right_viewer)),
        (2, tr(MsgKey::right_to_left_viewer)),
        (3, tr(MsgKey::vertical_viewer)),
        (4, tr(MsgKey::webtoon_viewer)),
        (5, tr(MsgKey::vertical_plus_viewer)),
    ]
}
/// Background codes (`pref_reader_theme_key`).
pub fn backgrounds() -> [(i32, String); 4] {
    [(1, tr(MsgKey::black_background)), (2, tr(MsgKey::gray_background)), (0, tr(MsgKey::white_background)), (3, tr(MsgKey::automatic_background))]
}
/// Scale type codes (`pref_image_scale_type_key`).
pub fn scale_types() -> [(i32, String); 6] {
    [
        (1, tr(MsgKey::scale_type_fit_screen)),
        (2, tr(MsgKey::scale_type_stretch)),
        (3, tr(MsgKey::scale_type_fit_width)),
        (4, tr(MsgKey::scale_type_fit_height)),
        (5, tr(MsgKey::scale_type_original_size)),
        (6, tr(MsgKey::scale_type_smart_fit)),
    ]
}
/// Zoom start codes (`pref_zoom_start_key`).
pub fn zoom_starts() -> [(i32, String); 4] {
    [(1, tr(MsgKey::zoom_start_automatic)), (2, tr(MsgKey::zoom_start_left)), (3, tr(MsgKey::zoom_start_right)), (4, tr(MsgKey::zoom_start_center))]
}

/// The five layout settings the Display and Paged groups show: the defaults on the Settings page, the open manga's own
/// values in the reader's settings window (user, 2026-10-24).
pub struct LayoutPrefs {
    pub reader_theme: Pref<i32>,
    pub image_scale_type: Pref<i32>,
    pub zoom_start: Pref<i32>,
    pub crop_borders: Pref<bool>,
    pub dual_page_split_paged: Pref<bool>,
}

impl LayoutPrefs {
    pub fn defaults(r: &ReaderPrefs) -> Self {
        Self {
            reader_theme: r.reader_theme.clone(),
            image_scale_type: r.image_scale_type.clone(),
            zoom_start: r.zoom_start.clone(),
            crop_borders: r.crop_borders.clone(),
            dual_page_split_paged: r.dual_page_split_paged.clone(),
        }
    }

    pub fn manga(m: &MangaReaderPrefs) -> Self {
        Self {
            reader_theme: m.reader_theme(),
            image_scale_type: m.image_scale_type(),
            zoom_start: m.zoom_start(),
            crop_borders: m.crop_borders(),
            dual_page_split_paged: m.dual_page_split_paged(),
        }
    }
}

/// The Display and Paged groups: the background colour and the paged layout. The Settings reader page and the reader's
/// own settings window both show them, so the rows are built here once.
pub fn layout_rows(r: &LayoutPrefs) -> Vec<Preference> {
    vec![
        group(&tr(MsgKey::action_display), vec![int_choice(&tr(MsgKey::pref_reader_theme), &r.reader_theme, &backgrounds(), true)]),
        group(
            &tr(MsgKey::pager_viewer),
            vec![
                int_choice(&tr(MsgKey::pref_image_scale_type), &r.image_scale_type, &scale_types(), true),
                int_choice(&tr(MsgKey::pref_zoom_start), &r.zoom_start, &zoom_starts(), true),
                switch(&tr(MsgKey::pref_crop_borders), &r.crop_borders, true),
                switch(&tr(MsgKey::pref_dual_page_split), &r.dual_page_split_paged, true),
            ],
        ),
    ]
}

/// The reader settings page: the default reading mode, then [`layout_rows`]. Every row is kept.
pub fn rows_all(prefs: &Prefs) -> Vec<Preference> {
    let mut tree = vec![
        item(int_choice(&tr(MsgKey::pref_viewer_type), &prefs.reader.default_reading_mode, &reading_modes(), true)),
        item(crate::settings_ui::rows::auto_hide_bars(&prefs.base)),
    ];
    tree.extend(layout_rows(&LayoutPrefs::defaults(&prefs.reader)));
    tree
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group_titles(tree: &[Preference]) -> Vec<&str> {
        tree.iter()
            .filter_map(|entry| match entry {
                Preference::Group(g) => Some(g.title.as_str()),
                Preference::Item(_) => None,
            })
            .collect()
    }

    #[test]
    fn the_page_has_the_top_rows_then_display_and_paged_groups() {
        let prefs = Prefs::in_memory();
        let tree = rows_all(&prefs);
        assert_eq!(tree.len(), 4);
        assert!(matches!(tree[0], Preference::Item(_)) && matches!(tree[1], Preference::Item(_)), "the reading mode, then auto-hide");
        assert_eq!(group_titles(&tree), ["Display", "Paged"]);
    }

    #[test]
    fn the_paged_group_holds_the_four_kept_layout_rows() {
        let prefs = Prefs::in_memory();
        let tree = rows_all(&prefs);
        let Preference::Group(paged) = &tree[3] else { panic!("the Paged group") };
        let titles: Vec<&str> = paged.items.iter().map(|i| i.title()).collect();
        assert_eq!(titles, ["Scale type", "Zoom start position", "Crop borders", "Split wide pages"]);
    }

    #[test]
    fn the_reading_mode_offers_the_five_modes_and_the_background_the_four_colours() {
        assert_eq!(reading_modes().len(), 5);
        assert_eq!(backgrounds().len(), 4);
        assert_eq!(scale_types().len(), 6);
        assert_eq!(zoom_starts().len(), 4);
    }
}
