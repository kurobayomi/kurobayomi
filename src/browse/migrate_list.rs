//! The manga Migrate tab (T-40b-070): the library's sources with their favourite counts, in the stored sort. The list is
//! the counts the library holds, each source taken from the directory or, when it is not installed, a stub (`getOrStub`);
//! the local source is left out by the sort, and an empty library shows `information_empty_library` (E-MS-01, E-MS-02).

#[cfg(test)]
use std::cmp::Ordering;
#[cfg(test)]
use std::rc::Rc;
#[cfg(test)]
use std::sync::Arc;

#[cfg(test)]
use gtk::prelude::*;
#[cfg(test)]
use kuro_domain::app::source::LOCAL_SOURCE_ID;
#[cfg(test)]
use kuro_domain::items::sorter::collate;
#[cfg(test)]
use kuro_domain::source::model::Source;
#[cfg(test)]
use kuro_prefs::source::{MigrationDirection, MigrationSortingMode, SourcePrefs};

#[cfg(test)]
use super::directory::{SourceDirectory, source_or_stub};
#[cfg(test)]
use kuro_platform::kind::Kind;
#[cfg(test)]
use super::pages::MigrateSourcesNav;
#[cfg(test)]
use super::pages::MigrateSourcesSnapshot;
#[cfg(test)]
use super::pages::migrate_sources;

#[cfg(test)]
/// The order of the migration list (T-41b-039, E-MS-03, E-MS-04): the local source is left out; a stub comes first when
/// ascending and last when descending; then the name (lowercase, collated) or the favourite count. Ties go by source id
/// ascending in both directions, so the order does not depend on the query's row order (a deliberate deviation from
/// Kotlin's input order).
pub fn sort_migrate_rows(rows: Vec<(Source, i64)>, mode: MigrationSortingMode, direction: MigrationDirection) -> Vec<(Source, i64)> {
    let mut kept: Vec<(Source, i64)> = rows.into_iter().filter(|(source, _)| source.id != LOCAL_SOURCE_ID).collect();
    kept.sort_by(|a, b| {
        let by_mode = match (a.0.is_stub, b.0.is_stub) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => match mode {
                MigrationSortingMode::ALPHABETICAL => collate(&a.0.name.to_lowercase(), &b.0.name.to_lowercase()),
                MigrationSortingMode::TOTAL => a.1.cmp(&b.1),
            },
        };
        let by_direction = match direction {
            MigrationDirection::ASCENDING => by_mode,
            MigrationDirection::DESCENDING => by_mode.reverse(),
        };
        by_direction.then(a.0.id.cmp(&b.0.id))
    });
    kept
}

#[cfg(test)]
/// The display order of the migration list: every library source with its count, in `mode` and `direction`. A source the
/// directory does not know is a stub and, with no name, shows its id.
pub fn migrate_list(counts: Vec<(i64, i64)>, directory: Option<&dyn SourceDirectory>, mode: MigrationSortingMode, direction: MigrationDirection) -> Vec<(Source, i64)> {
    let list = counts.into_iter().map(|(id, count)| (source_or_stub(directory, id), count)).collect();
    let mut sorted = sort_migrate_rows(list, mode, direction);
    for (source, _) in &mut sorted {
        if source.is_stub && source.name.trim().is_empty() {
            source.name = source.id.to_string();
        }
    }
    sorted
}

#[cfg(test)]
/// The empty state of the picker: upstream's "Your library is empty" for manga, and for anime the plan's own text, since
/// that one is a text bug there (T-41a-036).
pub fn empty_text(kind: Kind) -> String {
    match kind {
        Kind::Manga => kuro_i18n::tr(kuro_i18n::MsgKey::information_empty_library),
        Kind::Anime => super::texts::MIGRATE_SOURCES_EMPTY.to_owned(),
    }
}

#[allow(dead_code)] // kept for the shape of the type; not built or read yet
#[cfg(test)]
pub struct MigrateDeps {
    /// Which catalogue the tab lists: it words the empty state.
    pub kind: Kind,
    pub prefs: Rc<SourcePrefs>,
    /// `(source id, favourite count)` for every source with a favourite, handed to the callback on the GTK thread. A read
    /// that fails is an error, not an empty library (E-MS-06).
    pub counts: Rc<dyn Fn(Box<dyn FnOnce(Result<Vec<(i64, i64)>, String>)>)>,
    /// Shows a toast, used for a failed count read.
    pub toast: Rc<dyn Fn(String)>,
    pub directory: Rc<dyn Fn() -> Option<Arc<dyn SourceDirectory>>>,
    /// A row: the favourites of that source.
    pub open: Rc<dyn Fn(i64)>,
    /// The row menu's Copy: copies the source id.
    pub copy_id: Rc<dyn Fn(i64)>,
    /// Opens a link in the browser: the header's help action opens `texts::SOURCE_MIGRATION_HELP_URL`.
    pub open_url: Rc<dyn Fn(&str)>,
}

#[cfg(test)]
/// The tab. It reads the counts each time it is shown and after a change of the sort.
pub fn migrate_tab(deps: MigrateDeps) -> gtk::Box {
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let deps = Rc::new(deps);
    let shown = (Rc::clone(&deps), holder.downgrade());
    holder.connect_map(move |_| draw(Rc::clone(&shown.0), shown.1.clone()));
    draw(deps, holder.downgrade());
    holder
}

#[cfg(test)]
/// Reads the counts and replaces the tab's content with the list in the stored sort.
fn draw(deps: Rc<MigrateDeps>, holder: gtk::glib::WeakRef<gtk::Box>) {
    let counts = Rc::clone(&deps.counts);
    counts(Box::new(move |read| {
        let Some(holder) = holder.upgrade() else { return };
        let failed = read.is_err();
        let list = read.unwrap_or_else(|error| {
            tracing::error!(%error, "the migration favourite counts could not be read");
            (deps.toast)(kuro_i18n::tr(kuro_i18n::MsgKey::internal_error));
            Vec::new()
        });
        // the sort is cut from the settings: alphabetical and ascending, as the upstream defaults
        let (mode, direction) = (MigrationSortingMode::ALPHABETICAL, MigrationDirection::ASCENDING);
        let directory = (deps.directory)();
        let error = failed.then(|| kuro_i18n::tr(kuro_i18n::MsgKey::internal_error));
        let snapshot = MigrateSourcesSnapshot { sources: migrate_list(list, directory.as_deref(), mode, direction), empty_text: empty_text(deps.kind), error };
        let (help_deps, retry_deps, retry_holder) = (Rc::clone(&deps), Rc::clone(&deps), holder.downgrade());
        let nav = MigrateSourcesNav {
            open_help: Rc::new(move || (help_deps.open_url)(super::texts::SOURCE_MIGRATION_HELP_URL)),
            retry: Rc::new(move || draw(Rc::clone(&retry_deps), retry_holder.clone())),
            open: Rc::clone(&deps.open),
            copy_id: Rc::clone(&deps.copy_id),
        };
        while let Some(child) = holder.first_child() {
            holder.remove(&child);
        }
        holder.append(&migrate_sources(&snapshot, nav));
    }));
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;

    use kuro_prefs::store::PrefStore;

    use super::*;
    use crate::browse::directory::tests::{FakeDirectory, entry};

    fn directory() -> Arc<dyn SourceDirectory> {
        Arc::new(FakeDirectory { entries: vec![entry(1, "Zulu", "en", "ext.z"), entry(2, "Alpha", "en", "ext.a")], answers: Arc::new(|_, _| Ok(Vec::new())) })
    }

    #[test]
    fn the_list_keeps_stubs_first_leaves_the_local_source_out_and_names_a_stub_by_its_id() {
        let counts = vec![(1, 3), (0, 9), (9, 1), (2, 5)];
        let list = migrate_list(counts.clone(), Some(&*directory()), MigrationSortingMode::ALPHABETICAL, MigrationDirection::ASCENDING);
        assert_eq!(list.iter().map(|(s, c)| (s.id, *c)).collect::<Vec<_>>(), [(9, 1), (2, 5), (1, 3)], "E-MS-02: no local source; the stub first, then by name");
        assert!(list[0].0.is_stub && list[0].0.name == "9", "a stub with no name shows its id");
        assert!(!list[1].0.is_stub, "an installed source is not a stub");
        let by_count = migrate_list(counts, Some(&*directory()), MigrationSortingMode::TOTAL, MigrationDirection::DESCENDING);
        assert_eq!(by_count.iter().map(|(s, _)| s.id).collect::<Vec<_>>(), [2, 1, 9], "descending by count reverses the whole order, so the stub is last");
    }

    #[test]
    fn ties_go_by_source_id_in_both_directions_and_the_local_source_is_left_out() {
        let counts = vec![(5, 3), (2, 3), (0, 9), (7, 1)];
        let ascending = migrate_list(counts.clone(), None, MigrationSortingMode::TOTAL, MigrationDirection::ASCENDING);
        assert_eq!(ascending.iter().map(|(s, _)| s.id).collect::<Vec<_>>(), [7, 2, 5], "E-MS-04: the tie 2 and 5 is by id, the local source 0 is gone");
        let descending = migrate_list(counts, None, MigrationSortingMode::TOTAL, MigrationDirection::DESCENDING);
        assert_eq!(descending.iter().map(|(s, _)| s.id).collect::<Vec<_>>(), [2, 5, 7], "the tie is by id ascending too");
    }

    #[test]
    fn with_no_directory_every_source_of_the_library_is_a_stub() {
        let list = migrate_list(vec![(4, 2), (5, 1)], None, MigrationSortingMode::TOTAL, MigrationDirection::ASCENDING);
        assert!(list.iter().all(|(source, _)| source.is_stub), "T-40b-070: the flag comes from the directory");
        assert_eq!(list.iter().map(|(s, _)| s.id).collect::<Vec<_>>(), [5, 4]);
    }

    fn labels(root: &gtk::Widget) -> Vec<String> {
        fn walk(widget: &gtk::Widget, out: &mut Vec<String>) {
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                out.push(label.label().to_string());
            }
            let mut child = widget.first_child();
            while let Some(next) = child {
                walk(&next, out);
                child = next.next_sibling();
            }
        }
        let mut out = Vec::new();
        walk(root, &mut out);
        out
    }

    fn deps(counts: Vec<(i64, i64)>, prefs: Rc<SourcePrefs>, opened: Rc<RefCell<Vec<i64>>>) -> MigrateDeps {
        MigrateDeps {
            kind: Kind::Manga,
            prefs,
            counts: Rc::new(move |deliver| deliver(Ok(counts.clone()))),
            toast: Rc::new(|_| {}),
            directory: Rc::new(|| Some(directory())),
            open: Rc::new(move |id| opened.borrow_mut().push(id)),
            copy_id: Rc::new(|_| {}),
            open_url: Rc::new(|_| {}),
        }
    }

    fn buttons_of(tab: &gtk::Box) -> Vec<gtk::Button> {
        fn walk(widget: &gtk::Widget, out: &mut Vec<gtk::Button>) {
            if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                out.push(button.clone());
            }
            let mut child = widget.first_child();
            while let Some(next) = child {
                walk(&next, out);
                child = next.next_sibling();
            }
        }
        let mut out = Vec::new();
        walk(tab.upcast_ref(), &mut out);
        out
    }

    fn a_failed_count_read_shows_a_toast_and_an_error_row_whose_retry_reads_again() {
        let prefs = Rc::new(SourcePrefs::new(PrefStore::in_memory()));
        let toasts = Rc::new(RefCell::new(Vec::<String>::new()));
        let reads = Rc::new(std::cell::Cell::new(0));
        let (sink, counted) = (Rc::clone(&toasts), Rc::clone(&reads));
        let deps = MigrateDeps {
            counts: Rc::new(move |deliver| {
                counted.set(counted.get() + 1);
                if counted.get() == 1 { deliver(Err("db closed".to_owned())) } else { deliver(Ok(vec![(1, 4)])) }
            }),
            toast: Rc::new(move |text| sink.borrow_mut().push(text)),
            ..deps(Vec::new(), prefs, Rc::default())
        };
        let tab = migrate_tab(deps);
        assert_eq!(*toasts.borrow(), [kuro_i18n::tr(kuro_i18n::MsgKey::internal_error)], "E-MS-06: the failure is not silent");
        let text = labels(tab.upcast_ref());
        assert!(text.contains(&kuro_i18n::tr(kuro_i18n::MsgKey::internal_error)), "E-MS-02: an error row: {text:?}");
        assert!(!text.contains(&kuro_i18n::tr(kuro_i18n::MsgKey::information_empty_library)), "not the empty state");
        let retry = buttons_of(&tab).into_iter().find(|b| b.label().as_deref() == Some(&*kuro_i18n::tr(kuro_i18n::MsgKey::action_retry))).expect("Retry");
        retry.emit_clicked();
        assert_eq!(reads.get(), 2, "Retry reads the counts again");
        let text = labels(tab.upcast_ref());
        assert!(text.contains(&"Zulu".to_owned()), "the list replaces the error row: {text:?}");
        assert!(!buttons_of(&tab).iter().any(|b| b.label().as_deref() == Some(&*kuro_i18n::tr(kuro_i18n::MsgKey::action_retry))), "and Retry is gone");
    }

    fn the_help_action_opens_the_source_migration_guide() {
        let prefs = Rc::new(SourcePrefs::new(PrefStore::in_memory()));
        let opened = Rc::new(RefCell::new(Vec::<String>::new()));
        let sink = Rc::clone(&opened);
        let deps = MigrateDeps { open_url: Rc::new(move |url| sink.borrow_mut().push(url.to_owned())), ..deps(vec![(1, 3)], prefs, Rc::default()) };
        let tab = migrate_tab(deps);
        let help = buttons_of(&tab).into_iter().find(|b| b.tooltip_text().as_deref() == Some(&*kuro_i18n::tr(kuro_i18n::MsgKey::label_help))).expect("help");
        help.emit_clicked();
        assert_eq!(*opened.borrow(), ["https://kurobayomi.github.io/docs/guides/source-migration"]);
    }

    fn the_tab_lists_the_library_sources_and_an_empty_library_shows_the_empty_state() {
        let prefs = Rc::new(SourcePrefs::new(PrefStore::in_memory()));
        let tab = migrate_tab(deps(vec![(1, 3), (2, 5)], Rc::clone(&prefs), Rc::default()));
        let text = labels(tab.upcast_ref());
        let (alpha, zulu) = (text.iter().position(|t| t == "Alpha"), text.iter().position(|t| t == "Zulu"));
        assert!(alpha.is_some() && zulu.is_some() && alpha < zulu, "listed by name: {text:?}");
        assert!(text.iter().any(|t| t == "5 in library") && text.iter().any(|t| t == "3 in library"), "with their favourite counts, in words: {text:?}");

        let empty = migrate_tab(deps(Vec::new(), prefs, Rc::default()));
        assert!(labels(empty.upcast_ref()).contains(&kuro_i18n::tr(kuro_i18n::MsgKey::information_empty_library)), "E-MS-01");
    }

    fn the_anime_picker_says_no_source_has_favorites_and_the_manga_picker_keeps_the_library_text() {
        let prefs = Rc::new(SourcePrefs::new(PrefStore::in_memory()));
        let tab = migrate_tab(MigrateDeps { kind: Kind::Anime, ..deps(Vec::new(), prefs, Rc::default()) });
        let text = labels(tab.upcast_ref());
        assert!(text.contains(&crate::browse::texts::MIGRATE_SOURCES_EMPTY.to_owned()), "T-41a-036: {text:?}");
        assert!(!text.contains(&kuro_i18n::tr(kuro_i18n::MsgKey::information_empty_library)), "not the library text bug");
        assert_eq!(empty_text(Kind::Manga), kuro_i18n::tr(kuro_i18n::MsgKey::information_empty_library));
    }

    pub(crate) fn smoke() {
        a_failed_count_read_shows_a_toast_and_an_error_row_whose_retry_reads_again();
        the_help_action_opens_the_source_migration_guide();
        the_anime_picker_says_no_source_has_favorites_and_the_manga_picker_keeps_the_library_text();
        the_tab_lists_the_library_sources_and_an_empty_library_shows_the_empty_state();
    }
}

#[cfg(test)]
mod props {
    use super::*;
    use crate::browse::directory::tests::{FakeDirectory, entry};
    use proptest::prelude::*;

    fn ids(list: &[(Source, i64)]) -> Vec<i64> {
        list.iter().map(|(source, _)| source.id).collect()
    }

    proptest! {
        #[test]
        fn the_migration_sort_keeps_every_non_local_source_and_sorting_again_changes_nothing(
            counts in proptest::collection::vec((0i64..6, 0i64..9), 0..12),
            by_count in any::<bool>(),
            descending in any::<bool>(),
        ) {
            let mode = if by_count { MigrationSortingMode::TOTAL } else { MigrationSortingMode::ALPHABETICAL };
            let direction = if descending { MigrationDirection::DESCENDING } else { MigrationDirection::ASCENDING };
            let directory: Arc<dyn SourceDirectory> = Arc::new(FakeDirectory {
                entries: vec![entry(1, "Zulu", "en", "ext.z"), entry(2, "Alpha", "en", "ext.a")],
                answers: Arc::new(|_, _| Ok(Vec::new())),
            });
            let raw: Vec<(Source, i64)> = counts.iter().map(|(id, count)| (source_or_stub(Some(&*directory), *id), *count)).collect();
            let once = sort_migrate_rows(raw, mode, direction);
            let twice = sort_migrate_rows(once.clone(), mode, direction);
            prop_assert_eq!(once.len(), counts.iter().filter(|(id, _)| *id != 0).count(), "E-MS-02: the local source is the only one left out");
            prop_assert_eq!(ids(&once), ids(&twice), "sorting a sorted list changes nothing");
        }
    }
}
