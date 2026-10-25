//! Viewer settings (T-44a-004, T-43b-005 clamps on read, T-44a-005, T-44a-018, B§1, D§3). The initial values come from the preference store,
//! never from Kotlin field initialisers. `diff` is the reaction table of B§1.2: a change-only reaction fires for a
//! value that differs from the one the viewer holds, and never for the first value it reads.

use std::sync::mpsc::Receiver;

use kuro_prefs::reader::{MangaReaderPrefs, ReaderHideThreshold, TappingInvertMode};
#[cfg(test)]
use kuro_prefs::reader::ReaderPrefs;
use kuro_prefs::store::Changed;

use super::nav::NavMode;
use crate::misc::diet_defaults as diet;
use crate::reader::page_profile::set_display_profile;
use crate::reader::tiles::set_tile_threshold;
use crate::reader::pref_rules::{self, NAVIGATION_MODE, READER_THEME, ZOOM_START, clamp_to};

// `pref_always_decode_long_strip_with_ssiv` is dropped (T-44a-061): the viewer does not read it, and a stored value is ignored.

/// The settings a viewer reads, as plain values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerSettings {
    pub long_tap: bool,
    pub page_transitions: bool,
    pub double_tap_anim_ms: u32,
    pub volume_keys: bool,
    pub volume_keys_inverted: bool,
    pub always_show_transition: bool,
    pub overlay_on_start: bool,
    pub nav_mode_pager: NavMode,
    pub nav_mode_webtoon: NavMode,
    pub tapping_pager: TappingInvertMode,
    pub tapping_webtoon: TappingInvertMode,
    /// Read live on each scroll (B§7.6 proposal), so it is not part of the reactions.
    pub hide_threshold: i32,
    pub navigate_pan: bool,
    pub landscape_zoom: bool,
    pub image_scale_type: i32,
    pub zoom_start: i32,
    pub reader_theme: i32,
    pub crop_pager: bool,
    pub crop_webtoon: bool,
    pub side_padding: i32,
    pub zoom_out_disabled: bool,
    pub double_tap_zoom: bool,
    /// `pref_hardware_bitmap_threshold`: the largest tile edge of a page decoded from now on (T-44a-060).
    pub tile_threshold: i32,
    /// `pref_display_profile_key`: the ICC file pages are taken to when decoded from now on (T-43a-067).
    pub display_profile: String,
    pub dual_split_pager: bool,
    pub dual_invert_pager: bool,
    pub dual_rotate_pager: bool,
    pub dual_rotate_invert_pager: bool,
    pub dual_split_webtoon: bool,
    pub dual_invert_webtoon: bool,
    pub dual_rotate_webtoon: bool,
    pub dual_rotate_invert_webtoon: bool,
}

/// Which viewer's reactions to compute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Pager,
    Webtoon,
}

/// What a viewer does when a setting changes (B§1.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reaction {
    /// The navigation table or its inversion changed: re-apply the regions and the overlay.
    Navigation,
    /// Page images decode again in place, keeping the position (B§6.7, B§7.7, T-44a-007).
    RefreshImages,
    /// A dual-page split was switched off: remove the inserts (B§1.2 note).
    RemoveSplits,
    /// The theme changed on the webtoon, which restarts the screen upstream (restyled in place here).
    Restyle,
    ZoomOutDisabled,
    DoubleTapZoom,
}

/// The tile threshold is cut from the settings: the hard-coded value goes to the tile cutter for pages decoded from now on.
fn read_tile_threshold() -> i32 {
    set_tile_threshold(diet::READER_TILE_THRESHOLD);
    diet::READER_TILE_THRESHOLD
}

/// The display profile is cut from the settings: no profile file is applied to pages.
fn read_display_profile() -> String {
    set_display_profile(diet::READER_DISPLAY_PROFILE);
    diet::READER_DISPLAY_PROFILE.to_owned()
}

impl ViewerSettings {
    /// The settings of the defaults alone: a manga with no values of its own.
    #[cfg(test)]
    pub fn from_prefs(p: &ReaderPrefs) -> Self {
        Self::from_manga(&MangaReaderPrefs::new(p.clone(), 0))
    }

    /// Reads the kept values of one manga (its own, else the defaults); the cut ones are the hard-coded defaults. Unknown
    /// stored values fall back to their defaults (B§1.3).
    pub fn from_manga(p: &MangaReaderPrefs) -> Self {
        Self {
            long_tap: diet::READER_LONG_TAP,
            page_transitions: diet::READER_PAGE_TRANSITIONS,
            double_tap_anim_ms: diet::READER_DOUBLE_TAP_ANIM_MS,
            volume_keys: diet::READER_VOLUME_KEYS,
            volume_keys_inverted: diet::READER_VOLUME_KEYS_INVERTED,
            always_show_transition: diet::READER_ALWAYS_SHOW_CHAPTER_TRANSITION,
            overlay_on_start: diet::READER_OVERLAY_ON_START,
            nav_mode_pager: NavMode::from_stored(clamp_to(diet::READER_NAVIGATION_MODE, NAVIGATION_MODE)),
            nav_mode_webtoon: NavMode::from_stored(clamp_to(diet::READER_NAVIGATION_MODE, NAVIGATION_MODE)),
            tapping_pager: TappingInvertMode::NONE,
            tapping_webtoon: TappingInvertMode::NONE,
            hide_threshold: ReaderHideThreshold::LOW.threshold(),
            navigate_pan: diet::READER_NAVIGATE_PAN,
            landscape_zoom: diet::READER_LANDSCAPE_ZOOM,
            image_scale_type: p.image_scale_type().get(),
            zoom_start: clamp_to(p.zoom_start().get(), ZOOM_START),
            reader_theme: clamp_to(p.reader_theme().get(), READER_THEME),
            crop_pager: p.crop_borders().get(),
            crop_webtoon: p.crop_borders().get(),
            side_padding: pref_rules::side_padding(diet::READER_WEBTOON_SIDE_PADDING),
            zoom_out_disabled: diet::READER_WEBTOON_DISABLE_ZOOM_OUT,
            double_tap_zoom: diet::READER_WEBTOON_DOUBLE_TAP_ZOOM,
            tile_threshold: read_tile_threshold(),
            display_profile: read_display_profile(),
            dual_split_pager: p.dual_page_split_paged().get(),
            dual_invert_pager: diet::READER_DUAL_PAGE_INVERT,
            dual_rotate_pager: diet::READER_DUAL_PAGE_ROTATE,
            dual_rotate_invert_pager: diet::READER_DUAL_PAGE_ROTATE_INVERT,
            dual_split_webtoon: diet::READER_WEBTOON_DUAL_PAGE_SPLIT,
            dual_invert_webtoon: diet::READER_WEBTOON_DUAL_PAGE_INVERT,
            dual_rotate_webtoon: diet::READER_WEBTOON_DUAL_PAGE_ROTATE,
            dual_rotate_invert_webtoon: diet::READER_WEBTOON_DUAL_PAGE_ROTATE_INVERT,
        }
    }
}

impl ViewerSettings {
    /// The settings before the store's first emission: `use_page_transitions` starts false and takes the stored value
    /// with the first emission (E-22, T-43b-035). Everything else is read from the store at once (T-44a-004).
    pub fn before_first_emission(mut self) -> Self {
        self.page_transitions = false;
        self
    }
}

/// The reactions a change from `old` to `new` causes for `family`. Each reaction is listed once, in the order the
/// settings are declared. Values that did not change cause nothing.
pub fn diff(old: &ViewerSettings, new: &ViewerSettings, family: Family) -> Vec<Reaction> {
    let mut out = Vec::new();
    let mut push = |r: Reaction, changed: bool| {
        if changed && !out.contains(&r) {
            out.push(r);
        }
    };
    match family {
        Family::Pager => {
            push(Reaction::Navigation, old.nav_mode_pager != new.nav_mode_pager || old.tapping_pager != new.tapping_pager);
            push(
                Reaction::RefreshImages,
                old.image_scale_type != new.image_scale_type
                    || old.zoom_start != new.zoom_start
                    || old.reader_theme != new.reader_theme
                    || old.crop_pager != new.crop_pager
                    || old.landscape_zoom != new.landscape_zoom
                    || old.dual_invert_pager != new.dual_invert_pager
                    || old.dual_rotate_pager != new.dual_rotate_pager
                    || old.dual_rotate_invert_pager != new.dual_rotate_invert_pager
                    || old.dual_split_pager != new.dual_split_pager,
            );
            push(Reaction::RemoveSplits, old.dual_split_pager && !new.dual_split_pager);
        }
        Family::Webtoon => {
            push(Reaction::Navigation, old.nav_mode_webtoon != new.nav_mode_webtoon || old.tapping_webtoon != new.tapping_webtoon);
            push(
                Reaction::RefreshImages,
                old.crop_webtoon != new.crop_webtoon
                    || old.side_padding != new.side_padding
                    || old.dual_split_webtoon != new.dual_split_webtoon
                    || old.dual_invert_webtoon != new.dual_invert_webtoon
                    || old.dual_rotate_webtoon != new.dual_rotate_webtoon
                    || old.dual_rotate_invert_webtoon != new.dual_rotate_invert_webtoon,
            );
            push(Reaction::Restyle, old.reader_theme != new.reader_theme);
            push(Reaction::ZoomOutDisabled, old.zoom_out_disabled != new.zoom_out_disabled);
            push(Reaction::DoubleTapZoom, old.double_tap_zoom != new.double_tap_zoom);
        }
    }
    out
}

/// Follows the preference store for a live viewer (T-44a-005). Reads the store again only after it published a
/// change, and reports a result only when it differs from what the viewer holds (`distinctUntilChanged`). The first
/// read is the viewer's initial value and is never reported, so change-only reactions (`drop(1)`) do not fire on it.
pub struct SettingsWatch {
    prefs: MangaReaderPrefs,
    events: Receiver<Changed>,
    held: ViewerSettings,
}

impl SettingsWatch {
    /// Subscribes first and takes `held` as the initial value, so a write after this call is not missed.
    pub fn new(prefs: MangaReaderPrefs, held: ViewerSettings) -> Self {
        let events = prefs.defaults.store().subscribe();
        Self { prefs, events, held }
    }

    /// The new settings when the store changed them since the last call, otherwise `None`. Never blocks.
    pub fn poll(&mut self) -> Option<ViewerSettings> {
        let mut changed = false;
        while self.events.try_recv().is_ok() {
            changed = true;
        }
        if !changed {
            return None;
        }
        let new = ViewerSettings::from_manga(&self.prefs);
        (new != self.held).then(|| {
            self.held = new.clone();
            new
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuro_prefs::store::PrefStore;

    #[test]
    fn the_watch_reports_a_changed_value_once_and_never_the_initial_one() {
        let p = prefs();
        let mut watch = SettingsWatch::new(MangaReaderPrefs::new(p.clone(), 1), ViewerSettings::from_prefs(&p));
        assert_eq!(watch.poll(), None, "the initial value is not a change");
        p.reader_theme.set(2).unwrap();
        let new = watch.poll().expect("the write is reported");
        assert_eq!(new.reader_theme, 2);
        assert_eq!(watch.poll(), None, "reported once");
    }

    #[test]
    fn a_write_of_another_key_or_the_same_value_reports_nothing() {
        let p = prefs();
        let mut watch = SettingsWatch::new(MangaReaderPrefs::new(p.clone(), 1), ViewerSettings::from_prefs(&p));
        p.reader_theme.set(1).unwrap();
        p.crop_borders.set(false).unwrap();
        assert_eq!(watch.poll(), None, "the viewer holds the same settings");
    }

    #[test]
    fn several_writes_between_polls_give_the_latest_values_in_one_report() {
        let p = prefs();
        let mut watch = SettingsWatch::new(MangaReaderPrefs::new(p.clone(), 1), ViewerSettings::from_prefs(&p));
        p.reader_theme.set(2).unwrap();
        p.reader_theme.set(0).unwrap();
        p.crop_borders.set(true).unwrap();
        let new = watch.poll().expect("reported");
        assert_eq!((new.reader_theme, new.crop_pager), (0, true));
    }

    #[test]
    fn the_watch_follows_the_open_manga_and_not_another() {
        let p = prefs();
        let (open, other) = (MangaReaderPrefs::new(p.clone(), 1), MangaReaderPrefs::new(p.clone(), 2));
        let mut watch = SettingsWatch::new(open.clone(), ViewerSettings::from_manga(&open));
        other.crop_borders().set(true).unwrap();
        assert_eq!(watch.poll(), None, "another manga's value");
        open.crop_borders().set(true).unwrap();
        assert!(watch.poll().expect("the open manga's value").crop_pager);
    }

    fn prefs() -> ReaderPrefs {
        ReaderPrefs::new(PrefStore::in_memory())
    }

    #[test]
    fn an_empty_store_gives_the_observed_defaults() {
        let s = ViewerSettings::from_prefs(&prefs());
        assert!(s.long_tap && s.page_transitions && s.always_show_transition && s.navigate_pan && s.landscape_zoom);
        assert!(!s.volume_keys && !s.volume_keys_inverted && !s.overlay_on_start);
        assert_eq!(s.double_tap_anim_ms, 500);
        assert_eq!(s.hide_threshold, 31);
        assert_eq!((s.nav_mode_pager, s.nav_mode_webtoon), (NavMode::Default, NavMode::Default));
        assert_eq!((s.tapping_pager, s.tapping_webtoon), (TappingInvertMode::NONE, TappingInvertMode::NONE));
        assert_eq!((s.image_scale_type, s.zoom_start, s.reader_theme), (1, 1, 1));
        assert_eq!(s.side_padding, 0);
    }

    #[test]
    fn out_of_range_stored_values_are_clamped_on_read() {
        let p = prefs();
        p.zoom_start.set(9).unwrap();
        p.reader_theme.set(7).unwrap();
        let s = ViewerSettings::from_prefs(&p);
        assert_eq!((s.zoom_start, s.reader_theme), (4, 3));
        p.zoom_start.set(0).unwrap();
        assert_eq!(ViewerSettings::from_prefs(&p).zoom_start, 1);
    }

    #[test]
    fn e22_page_transitions_start_false_then_take_the_stored_value() {
        let s = ViewerSettings::from_prefs(&prefs());
        assert!(s.page_transitions, "the stored default");
        let start = s.clone().before_first_emission();
        assert!(!start.page_transitions);
        assert_eq!(ViewerSettings { page_transitions: true, ..start }, s);
    }

    #[test]
    fn an_unchanged_value_causes_no_reaction_and_the_first_read_causes_none() {
        let s = ViewerSettings::from_prefs(&prefs());
        assert!(diff(&s, &s.clone(), Family::Pager).is_empty());
        assert!(diff(&s, &s.clone(), Family::Webtoon).is_empty());
    }

    #[test]
    fn a_switched_off_split_removes_the_inserts_and_refreshes_once() {
        let mut old = ViewerSettings::from_prefs(&prefs());
        old.dual_split_pager = true;
        let mut new = old.clone();
        new.dual_split_pager = false;
        assert_eq!(diff(&old, &new, Family::Pager), [Reaction::RefreshImages, Reaction::RemoveSplits]);
    }

    #[test]
    fn the_webtoon_theme_restyles_and_the_pager_theme_refreshes() {
        let old = ViewerSettings::from_prefs(&prefs());
        let mut new = old.clone();
        new.reader_theme = 2;
        assert_eq!(diff(&old, &new, Family::Webtoon), [Reaction::Restyle]);
        assert_eq!(diff(&old, &new, Family::Pager), [Reaction::RefreshImages]);
    }
}
