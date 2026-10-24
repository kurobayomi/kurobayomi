//! The reader settings window (T-043a-046, T-043a-047): a native modal window with its own header bar, opened from the
//! reader's menu. Its page is a boxed-list page of the shared pref_model rows: the reading mode first, then the kept
//! reader rows of `settings_ui::reader::page` (background colour, scale type, zoom start position, crop borders, split
//! wide pages). Every row is the open manga's own (user, 2026-10-24); Settings > Reader holds the defaults a manga
//! follows until a value is picked for it. Each row writes the manga's key at once, so the open reader follows the
//! change live; the mode row hands the chosen mode to the reader, which writes it to the manga's viewer flags.
//!
//! The settings diet cut every other reader setting (colour filters, E-Ink flash, tap zones, volume keys, long tap,
//! rotate-to-fit, the webtoon copies and the restore switch).

use kuro_i18n::{MsgKey, tr};
use std::rc::Rc;

use gtk::prelude::*;
use kuro_prefs::reader::{MangaReaderPrefs, ReadingMode};

use super::chrome::mode_label;
use super::strings;
use crate::pref_model::bind;
use crate::pref_model::model::{ItemCommon, Preference, PreferenceItem};
use crate::pref_model::page;
use crate::settings_ui::reader::page::{LayoutPrefs, layout_rows};
use crate::settings_ui::rows::group;
use crate::ui::{DialogButton, Role};

/// The reading mode of the open manga. The pick goes to `on_mode`, which stores it for the manga and applies it to the
/// open reader at once. "Default" is not offered: `mode` is already the resolved one.
fn mode_row(mode: ReadingMode, on_mode: Rc<dyn Fn(ReadingMode)>) -> PreferenceItem {
    let entries = ReadingMode::ALL.iter().filter(|m| **m != ReadingMode::Default).map(|m| (m.flag_value().to_string(), mode_label(*m).to_owned())).collect();
    let on_change = bind::veto(move |key: String| {
        let result = key.parse::<i32>().map(ReadingMode::from_preference).map_err(|err| err.to_string());
        if let Ok(picked) = &result {
            on_mode(*picked);
        }
        async move { result.map(|_| true) }
    });
    PreferenceItem::BasicList { common: ItemCommon::bound(strings::viewer()), value: mode.flag_value().to_string(), entries, on_change }
}

/// The rows of the page: the reading mode, then the shared Display and Paged groups.
pub fn page_rows(prefs: &MangaReaderPrefs, mode: ReadingMode, on_mode: Rc<dyn Fn(ReadingMode)>) -> Vec<Preference> {
    let mut rows = vec![group(&strings::pref_category_reading_mode(), vec![mode_row(mode, on_mode)])];
    rows.extend(layout_rows(&LayoutPrefs::manga(prefs)));
    rows
}

/// Opens the window over `parent`, capped at 75 percent of its height (`ReaderSettingsDialog.kt:38`). Escape closes it.
pub fn open(parent: &impl IsA<gtk::Window>, prefs: &MangaReaderPrefs, mode: ReadingMode, on_mode: Rc<dyn Fn(ReadingMode)>) {
    let body = page::body(&page_rows(prefs, mode, on_mode));
    let max_height = parent.as_ref().height().max(1) * 3 / 4;
    // the page scrolls itself: it is as tall as its rows up to the cap, so the dialog's own scroller never kicks in
    body.scrolled.set_vexpand(false);
    body.scrolled.set_propagate_natural_height(true);
    body.scrolled.set_max_content_height(max_height);
    let window = crate::ui::dialog(&strings::action_settings(), &body.widget, 420, vec![DialogButton::new(&tr(MsgKey::action_close), Role::Suggested, || {})]);
    window.set_transient_for(Some(parent));
    window.present();
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    use kuro_prefs::reader::ReaderPrefs;
    use kuro_prefs::store::PrefStore;

    use super::*;

    /// Runs a veto future that is ready at once. The rows never wait on the GTK loop.
    fn resolve<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("the veto is not ready at once"),
        }
    }

    fn group_titles(rows: &[Preference]) -> Vec<&str> {
        rows.iter()
            .filter_map(|entry| match entry {
                Preference::Group(g) => Some(g.title.as_str()),
                Preference::Item(_) => None,
            })
            .collect()
    }

    #[test]
    fn the_page_is_the_reading_mode_then_the_display_and_paged_groups() {
        let prefs = MangaReaderPrefs::new(ReaderPrefs::new(PrefStore::in_memory()), 1);
        let rows = page_rows(&prefs, ReadingMode::LeftToRight, Rc::new(|_| {}));
        assert_eq!(group_titles(&rows), [strings::pref_category_reading_mode().as_str(), "Display", "Paged"]);
    }

    #[test]
    fn the_mode_row_shows_the_mode_and_hands_the_pick_to_the_reader() {
        let prefs = MangaReaderPrefs::new(ReaderPrefs::new(PrefStore::in_memory()), 1);
        let picked = Rc::new(Cell::new(None));
        let sink = Rc::clone(&picked);
        let rows = page_rows(&prefs, ReadingMode::LeftToRight, Rc::new(move |mode: ReadingMode| sink.set(Some(mode))));
        let Preference::Group(mode_group) = &rows[0] else { panic!("the mode group") };
        let PreferenceItem::BasicList { value, on_change, .. } = &mode_group.items[0] else { panic!("the mode row") };
        assert_eq!(value, "1");
        assert_eq!(resolve(on_change("2".to_owned())), Ok(true));
        assert_eq!(picked.get(), Some(ReadingMode::RightToLeft));
    }

    #[test]
    fn the_split_row_is_bound_to_the_split_key() {
        let prefs = MangaReaderPrefs::new(ReaderPrefs::new(PrefStore::in_memory()), 1);
        let rows = page_rows(&prefs, ReadingMode::LeftToRight, Rc::new(|_| {}));
        let Preference::Group(paged) = &rows[2] else { panic!("the Paged group") };
        let PreferenceItem::Switch { pref, .. } = &paged.items[3] else { panic!("the split switch") };
        assert_eq!(pref.key(), "manga_dual_page_split_1", "the open manga's own key");
    }

    #[test]
    fn a_row_shows_the_default_until_the_manga_has_its_own_value() {
        let prefs = MangaReaderPrefs::new(ReaderPrefs::new(PrefStore::in_memory()), 1);
        prefs.defaults.crop_borders.set(true).unwrap();
        let rows = page_rows(&prefs, ReadingMode::LeftToRight, Rc::new(|_| {}));
        let Preference::Group(paged) = &rows[2] else { panic!("the Paged group") };
        let PreferenceItem::Switch { pref, .. } = &paged.items[2] else { panic!("the crop switch") };
        assert!(pref.get(), "the default");
        pref.set(false).unwrap();
        assert!(prefs.defaults.crop_borders.get() && !prefs.crop_borders().get(), "the write is the manga's own");
    }

    #[test]
    fn the_page_builds_with_the_kept_rows() {
        crate::testing::with_display(|| {
            let prefs = MangaReaderPrefs::new(ReaderPrefs::new(PrefStore::in_memory()), 1);
            let body = page::body(&page_rows(&prefs, ReadingMode::LeftToRight, Rc::new(|_| {})));
            assert!(body.targets.iter().any(|(title, _)| title.as_str() == "Split wide pages"));
        });
    }
}
