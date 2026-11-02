//! The list options of the details pages (user decision 2026-10-12), one code path for the manga chapter list, the anime
//! episode list and the season grid of a `Seasons` anime. A list is described by a [`Spec`] over its flag word: every
//! toggle is a two-value field of the word, every radio group a masked field, and a pick becomes one masked write
//! `(flag, mask)`. The menu itself is the shared `widgets::view_options` button in the page's top bar; it replaces the
//! old floating Filter / Sort / Display windows and the second sort button above the list.
//!
//! "Show only" is include-or-off: a check writes the include value or clears its group (no exclude state). An exclude
//! value an older version stored reads as off. "Reverse order" is the ascending bit: every list sorts descending by
//! default (newest first). The Downloaded check is held on, and greyed out, while the app-wide downloaded-only mode is on.

use crate::ui::DialogOpen as _;
use kuro_db::flags::item as c;
use kuro_db::flags::season as sf;
use kuro_domain::entries::season_display::SeasonDisplayMode;
use kuro_i18n::{MsgKey, tr};
use kuro_platform::kind::Kind;

use crate::entry_core::flags::Write;
use crate::ui::{self, DialogButton, Role};
use crate::widgets::view_options::{Item, Pick, Section, ViewOptions};

/// The action of "Use as default for all titles".
pub const DEFAULT: &str = "default";
/// The manga's scanlator exclusion list, a plain action of its Show only block.
pub const SCANLATORS: &str = "scanlators";
/// The tooltip and accessible name of the button.
pub fn tooltip() -> String {
    tr(MsgKey::entry_list_options_tooltip)
}
/// The name of the Downloaded check, held on under downloaded-only.
const DOWNLOADED: &str = "downloaded";

/// One boolean field: shown on while `word & mask == on`; a pick writes `on` or `off`.
struct Bit {
    name: &'static str,
    label: String,
    on: i64,
    off: i64,
    mask: i64,
}

/// One radio group over `mask`: each option is a stored value. With `submenu` the radios sit in a submenu of that name.
struct Choice {
    name: &'static str,
    mask: i64,
    options: Vec<(i64, String)>,
    submenu: Option<String>,
}

enum Entry {
    Bit(Bit),
    Choice(Choice),
    Action(&'static str, String),
}

struct Group {
    title: Option<String>,
    filter: bool,
    entries: Vec<Entry>,
}

/// What one menu item shows for a flag word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    Toggle { on: bool, enabled: bool },
    Radio(String),
}

/// The options of one list kind.
pub struct Spec {
    groups: Vec<Group>,
}

fn check(name: &'static str, label: String, on: i64, mask: i64) -> Entry {
    Entry::Bit(Bit { name, label, on, off: 0, mask })
}

fn choice(name: &'static str, mask: i64, options: Vec<(i64, String)>) -> Entry {
    Entry::Choice(Choice { name, mask, options, submenu: None })
}

fn reverse(asc: i64, desc: i64, mask: i64) -> Entry {
    Entry::Bit(Bit { name: "reverse", label: tr(MsgKey::ui_reverse_order).to_owned(), on: asc, off: desc, mask })
}

fn default_group() -> Group {
    Group { title: None, filter: false, entries: vec![Entry::Action(DEFAULT, tr(MsgKey::ui_use_as_default_for_all_titles).to_owned())] }
}

/// The item list of an entry (`Entry::item_flags`): the manga chapter list or the anime episode list. The two kinds differ
/// in the unseen label, the number label, the scanlator exclusion list (manga) and the Filler check and the summaries
/// toggle (anime); everything else is one description over the one flag layout.
pub fn item_spec(kind: Kind) -> Spec {
    let anime = kind.is_anime();
    let unseen = if anime { tr(MsgKey::action_filter_unwatched) } else { tr(MsgKey::action_filter_unread) };
    let number = if anime { tr(MsgKey::show_episode_number) } else { tr(MsgKey::show_chapter_number) };
    let mut show_only = vec![
        check(DOWNLOADED, tr(MsgKey::label_downloaded), c::SHOW_DOWNLOADED, c::DOWNLOADED_MASK),
        check("unseen", unseen, c::SHOW_UNDONE, c::UNDONE_MASK),
        check("bookmarked", tr(MsgKey::action_filter_bookmarked), c::SHOW_BOOKMARKED, c::BOOKMARKED_MASK),
    ];
    show_only.push(if anime {
        check("filler", "Filler".to_owned(), c::SHOW_FILLERMARKED, c::FILLERMARKED_MASK)
    } else {
        Entry::Action(SCANLATORS, format!("{}…", tr(MsgKey::scanlator)))
    });
    // Episode summaries stay a Display toggle: the rows follow it.
    let mut display = vec![choice("display", c::DISPLAY_MASK, vec![(c::DISPLAY_NAME, tr(MsgKey::show_title)), (c::DISPLAY_NUMBER, number)])];
    if anime {
        display.push(Entry::Bit(Bit { name: "summaries", label: tr(MsgKey::show_episode_summaries), on: c::SHOW_SUMMARIES, off: c::SHOW_NOT_SUMMARIES, mask: c::SUMMARIES_MASK }));
    }
    Spec {
        groups: vec![
            Group { title: Some(tr(MsgKey::ui_show_only)), filter: true, entries: show_only },
            Group {
                title: Some(tr(MsgKey::ui_sort_by)),
                filter: false,
                entries: vec![
                    choice(
                        "sort",
                        c::SORTING_MASK,
                        vec![(c::SORTING_SOURCE, tr(MsgKey::ui_source_order).to_owned()), (c::SORTING_NUMBER, "Number".to_owned()), (c::SORTING_UPLOAD_DATE, tr(MsgKey::ui_upload_date).to_owned())],
                    ),
                    reverse(c::SORT_ASC, c::SORT_DESC, c::SORT_DIR_MASK),
                ],
            },
            Group { title: Some("Display".to_string()), filter: false, entries: display },
            default_group(),
        ],
    }
}

/// The season grid of a `Seasons` anime (`Anime.seasonFlags`): six checks, seven sort keys, the grid style, the items
/// per row (0 is automatic), the five tile badges and the tile title.
pub fn season_spec() -> Spec {
    let grid_modes = [
        (MsgKey::action_display_grid, SeasonDisplayMode::CompactGrid),
        (MsgKey::action_display_comfortable_grid, SeasonDisplayMode::ComfortableGrid),
        (MsgKey::action_display_cover_only_grid, SeasonDisplayMode::CoverOnlyGrid),
        (MsgKey::action_display_list, SeasonDisplayMode::List),
    ];
    let overlays = [
        (MsgKey::action_display_download_badge_anime, sf::OVERLAY_DOWNLOADED_MASK),
        (MsgKey::action_display_unseen_badge, sf::OVERLAY_UNSEEN_MASK),
        (MsgKey::action_display_local_badge_anime, sf::OVERLAY_LOCAL_MASK),
        (MsgKey::action_display_language_badge, sf::OVERLAY_LANG_MASK),
        (MsgKey::action_display_show_continue_watching_button, sf::OVERLAY_CONT_MASK),
    ];
    const OVERLAY_NAMES: [&str; 5] = ["badge_downloaded", "badge_unseen", "badge_local", "badge_language", "badge_continue"];
    let sizes = (0..=10).map(|n| (n << sf::GRID_DISPLAY_SIZE_SHIFT, if n == 0 { "Auto".to_owned() } else { n.to_string() })).collect();
    let mut display = vec![
        choice("grid", sf::GRID_DISPLAY_MODE_MASK, grid_modes.iter().map(|(label, mode)| (mode.to_i64() << sf::GRID_DISPLAY_MODE_SHIFT, tr(*label))).collect()),
        Entry::Choice(Choice { name: "size", mask: sf::GRID_DISPLAY_SIZE_MASK, options: sizes, submenu: Some(tr(MsgKey::pref_library_columns).to_owned()) }),
    ];
    display.extend(overlays.iter().zip(OVERLAY_NAMES).map(|((label, mask), name)| check(name, tr(*label), *mask, *mask)));
    display.push(choice("title", sf::DISPLAY_MODE_MASK, vec![(sf::DISPLAY_MODE_SOURCE, tr(MsgKey::show_title)), (sf::DISPLAY_MODE_NUMBER, tr(MsgKey::show_season_number))]));
    Spec {
        groups: vec![
            Group {
                title: Some(tr(MsgKey::ui_show_only)),
                filter: true,
                entries: vec![
                    check(DOWNLOADED, tr(MsgKey::label_downloaded), sf::SHOW_DOWNLOADED, sf::DOWNLOADED_MASK),
                    check("unseen", tr(MsgKey::action_filter_unwatched), sf::SHOW_UNSEEN, sf::UNSEEN_MASK),
                    check("started", tr(MsgKey::action_filter_started), sf::SHOW_STARTED, sf::STARTED_MASK),
                    check("completed", tr(MsgKey::completed), sf::SHOW_COMPLETED, sf::COMPLETED_MASK),
                    check("bookmarked", tr(MsgKey::action_filter_bookmarked), sf::SHOW_BOOKMARKED, sf::BOOKMARKED_MASK),
                    check("filler", "Filler".to_owned(), sf::SHOW_FILLERMARKED, sf::FILLERMARKED_MASK),
                ],
            },
            Group {
                title: Some(tr(MsgKey::ui_sort_by)),
                filter: false,
                entries: vec![
                    choice(
                        "sort",
                        sf::SORT_MASK,
                        vec![
                            (sf::SORT_SOURCE, tr(MsgKey::ui_source_order).to_owned()),
                            (sf::SORT_SEASON, tr(MsgKey::show_season_number).to_owned()),
                            (sf::SORT_UPLOAD, tr(MsgKey::ui_upload_date).to_owned()),
                            (sf::SORT_ALPHABET, tr(MsgKey::action_sort_alpha)),
                            (sf::SORT_COUNT, tr(MsgKey::action_sort_unseen_count)),
                            (sf::SORT_LAST_SEEN, tr(MsgKey::action_sort_last_seen)),
                            (sf::SORT_FETCHED, tr(MsgKey::action_sort_episode_fetch_date)),
                        ],
                    ),
                    reverse(sf::SORT_ASC, sf::SORT_DESC, sf::SORT_DIR_MASK),
                ],
            },
            Group { title: Some("Display".to_string()), filter: false, entries: display },
            default_group(),
        ],
    }
}

impl Spec {
    /// The menu description for `widgets::view_options`. A radio's value is its stored value in decimal.
    pub fn sections(&self) -> Vec<Section> {
        self.groups
            .iter()
            .map(|group| {
                let mut items = Vec::new();
                for entry in &group.entries {
                    match entry {
                        Entry::Bit(bit) => items.push(Item::toggle(bit.name, bit.label.clone())),
                        Entry::Choice(choice) => {
                            let radios: Vec<Item> = choice.options.iter().map(|(value, label)| Item::radio(choice.name, value.to_string(), label.clone())).collect();
                            match &choice.submenu {
                                Some(label) => items.push(Item::submenu(label.clone(), radios)),
                                None => items.extend(radios),
                            }
                        }
                        Entry::Action(name, label) => items.push(Item::action(*name, label.clone())),
                    }
                }
                Section { title: group.title.clone(), filter: group.filter, items }
            })
            .collect()
    }

    fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.groups.iter().flat_map(|g| g.entries.iter())
    }

    /// What every toggle and radio group shows for `word`.
    pub fn shown(&self, word: i64, downloaded_only: bool) -> Vec<(&'static str, Shown)> {
        self.entries()
            .filter_map(|entry| match entry {
                Entry::Bit(bit) if bit.name == DOWNLOADED && downloaded_only => Some((bit.name, Shown::Toggle { on: true, enabled: false })),
                Entry::Bit(bit) => Some((bit.name, Shown::Toggle { on: word & bit.mask == bit.on, enabled: true })),
                Entry::Choice(choice) => Some((choice.name, Shown::Radio((word & choice.mask).to_string()))),
                Entry::Action(..) => None,
            })
            .collect()
    }

    /// Puts what `word` holds on the menu.
    pub fn sync(&self, menu: &ViewOptions, word: i64, downloaded_only: bool) {
        for (name, shown) in self.shown(word, downloaded_only) {
            match shown {
                Shown::Toggle { on, enabled } => {
                    menu.set_toggle(name, on);
                    menu.set_enabled(name, enabled);
                }
                Shown::Radio(value) => menu.set_radio(name, &value),
            }
        }
    }

    /// The masked write of a pick; `None` for a plain action or a name this list does not have.
    pub fn write(&self, pick: &Pick) -> Option<Write> {
        self.entries().find_map(|entry| match (entry, pick) {
            (Entry::Bit(bit), Pick::Toggle(name, on)) if bit.name == *name => Some((if *on { bit.on } else { bit.off }, bit.mask)),
            (Entry::Choice(choice), Pick::Radio(name, value)) if choice.name == *name => {
                value.parse::<i64>().ok().filter(|v| choice.options.iter().any(|(o, _)| o == v)).map(|v| (v, choice.mask))
            }
            _ => None,
        })
    }
}

/// "Use as default for all titles": asks once, then runs `on_ok`. The list options of this title become the default of
/// new titles and are written to every title of the library.
pub fn confirm_default(on_ok: impl Fn() + 'static) {
    let text = tr(MsgKey::ui_every_title_in_your_library_takes_these_list);
    let buttons = vec![DialogButton::cancel(tr(MsgKey::action_cancel)), DialogButton::new(&tr(MsgKey::ui_use_for_all_titles), Role::Suggested, on_ok)];
    ui::alert(&tr(MsgKey::ui_use_as_default_for_all_titles), &text, buttons, || {}).open();
}

#[cfg(test)]
mod tests {
    use kuro_db::flags::set_flag;

    use super::*;

    fn apply(word: i64, (flag, mask): Write) -> i64 {
        set_flag(word, flag, mask)
    }

    fn shown_of(spec: &Spec, word: i64, downloaded_only: bool, name: &str) -> Shown {
        spec.shown(word, downloaded_only).into_iter().find(|(n, _)| *n == name).map(|(_, s)| s).expect("an item")
    }

    #[test]
    fn every_toggle_and_radio_round_trips_through_the_word() {
        for spec in [item_spec(Kind::Manga), item_spec(Kind::Anime), season_spec()] {
            for entry in spec.entries() {
                match entry {
                    Entry::Bit(bit) => {
                        let on = apply(0, spec.write(&Pick::Toggle(bit.name, true)).expect("a write"));
                        assert_eq!(shown_of(&spec, on, false, bit.name), Shown::Toggle { on: true, enabled: true }, "{}", bit.name);
                        let off = apply(on, spec.write(&Pick::Toggle(bit.name, false)).expect("a write"));
                        assert_eq!(shown_of(&spec, off, false, bit.name), Shown::Toggle { on: false, enabled: true }, "{}", bit.name);
                    }
                    Entry::Choice(choice) => {
                        for (value, _) in &choice.options {
                            let word = apply(-1, spec.write(&Pick::Radio(choice.name, value.to_string())).expect("a write"));
                            assert_eq!(shown_of(&spec, word, false, choice.name), Shown::Radio(value.to_string()), "{}", choice.name);
                        }
                    }
                    Entry::Action(name, _) => assert_eq!(spec.write(&Pick::Action(*name)), None),
                }
            }
        }
    }

    #[test]
    fn a_check_is_include_or_off_and_an_old_exclude_reads_as_off() {
        let spec = item_spec(Kind::Manga);
        assert_eq!(spec.write(&Pick::Toggle("unseen", true)), Some((c::SHOW_UNDONE, c::UNDONE_MASK)));
        assert_eq!(spec.write(&Pick::Toggle("unseen", false)), Some((0, c::UNDONE_MASK)));
        assert_eq!(shown_of(&spec, c::SHOW_DONE, false, "unseen"), Shown::Toggle { on: false, enabled: true });
        let seasons = season_spec();
        assert_eq!(seasons.write(&Pick::Toggle("completed", true)), Some((sf::SHOW_COMPLETED, sf::COMPLETED_MASK)));
    }

    #[test]
    fn downloaded_only_holds_the_downloaded_check_on_without_touching_the_bits() {
        for spec in [item_spec(Kind::Manga), item_spec(Kind::Anime), season_spec()] {
            assert_eq!(shown_of(&spec, 0, true, DOWNLOADED), Shown::Toggle { on: true, enabled: false });
            assert_eq!(shown_of(&spec, 0, false, DOWNLOADED), Shown::Toggle { on: false, enabled: true });
        }
    }

    #[test]
    fn reverse_is_the_ascending_bit_and_a_new_key_keeps_the_direction() {
        let spec = item_spec(Kind::Anime);
        assert_eq!(shown_of(&spec, 0, false, "reverse"), Shown::Toggle { on: false, enabled: true }, "the default is newest first");
        let reversed = apply(0, spec.write(&Pick::Toggle("reverse", true)).expect("a write"));
        assert_eq!(reversed & c::SORT_DIR_MASK, c::SORT_ASC);
        let by_number = apply(reversed, spec.write(&Pick::Radio("sort", c::SORTING_NUMBER.to_string())).expect("a write"));
        assert_eq!(by_number & c::SORTING_MASK, c::SORTING_NUMBER);
        assert_eq!(by_number & c::SORT_DIR_MASK, c::SORT_ASC, "picking a key keeps Reverse order");
    }

    #[test]
    fn an_unknown_radio_value_writes_nothing_and_an_old_alphabet_sort_shows_no_radio() {
        let spec = item_spec(Kind::Manga);
        assert_eq!(spec.write(&Pick::Radio("sort", "12345".into())), None);
        assert_eq!(spec.write(&Pick::Radio("sort", "x".into())), None);
        assert_eq!(spec.write(&Pick::Toggle("nope", true)), None);
        let Shown::Radio(value) = shown_of(&spec, c::SORTING_ALPHABET, false, "sort") else { panic!("a radio") };
        let offered: Vec<String> = [c::SORTING_SOURCE, c::SORTING_NUMBER, c::SORTING_UPLOAD_DATE].iter().map(i64::to_string).collect();
        assert!(!offered.contains(&value));
    }

    #[test]
    fn the_menus_hold_what_the_user_asked_for() {
        let names = |spec: &Spec| -> Vec<&'static str> { spec.shown(0, false).into_iter().map(|(n, _)| n).collect() };
        assert_eq!(names(&item_spec(Kind::Manga)), vec!["downloaded", "unseen", "bookmarked", "sort", "reverse", "display"]);
        assert_eq!(names(&item_spec(Kind::Anime)), vec!["downloaded", "unseen", "bookmarked", "filler", "sort", "reverse", "display", "summaries"]);
        for spec in [item_spec(Kind::Manga), item_spec(Kind::Anime), season_spec()] {
            let sections = spec.sections();
            assert_eq!(sections[0].title.as_deref(), Some("Show only"));
            assert!(sections[0].filter && !sections[1].filter);
            assert!(matches!(sections.last().and_then(|s| s.items.last()), Some(Item::Action { name: DEFAULT, .. })));
        }
        // the season sizes sit in a submenu: Auto and 1 to 10
        let seasons = season_spec().sections();
        let size = seasons[2].items.iter().find_map(|i| match i {
            Item::Submenu { items, .. } => Some(items.len()),
            _ => None,
        });
        assert_eq!(size, Some(11));
    }
}
