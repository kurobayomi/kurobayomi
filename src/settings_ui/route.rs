//! Puts the screens of this module into the app (task T-47a-041). `screens::body` asks [`body`] first; an id this
//! module builds shows rows bound to the typed preferences, so the rule table decides which rows exist. Where the
//! typed rows do not yet cover the whole spec page, the spec page is built and its rows are replaced one by one
//! (`merge.rs`).
use kuro_i18n::{MsgKey, tr};
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use kuro_platform::kind::Kind;
use kuro_prefs::prefs::Prefs;
use kuro_prefs::source::SourcePrefs;

use crate::pref_model::model::{Preference, PreferenceItem};
use crate::pref_model::page;
use crate::pref_model::spec_convert::Registry;
use crate::screens::{built, Built};
use crate::settings_ui::category::Category;
use crate::settings_ui::merge::merge;
use crate::settings_ui::tracker_host;
use crate::settings_ui::{about, appearance, browse, cache, chooser, downloads, library, library_export, maintenance, player, reader, rows, search, storage, tracking, usage, view};

/// The settings ids this module builds.
pub fn handles(id: &str) -> bool {
    matches!(
        id,
        "settings_appearance"
            | "settings_library"
            | "settings_reader"
            | "settings_player"
            | "settings_browse"
            | "settings_search"
            | "settings_data_storage"
            | "settings_about"
            | "settings_downloads"
            | "settings_open_source_licenses"
            | "settings_worker_info"
    )
}

/// Every preference section over the app's store. Tracker secrets are not read by these pages, so the secret store
/// is the in-memory one.
pub fn prefs() -> Prefs {
    Prefs::with_store(crate::data::app().base.store().clone(), Arc::new(kuro_prefs::memory::MemorySecrets::default()))
}

/// The spec tree of a screen, built with the registry (no typed rows yet).
pub fn spec_tree(id: &str, store: &kuro_prefs::store::PrefStore) -> Vec<Preference> {
    let registry = crate::ui::with_spec(Registry::new);
    let navigate: Rc<dyn Fn(&str)> = Rc::new(|target| crate::ui::open(target));
    registry.build(id, store, &navigate).unwrap_or_default()
}

/// Gives every icon-only button of a toolbar its tooltip as accessible name (T-47a-008), so a screen reader says
/// `Tracking guide` or `Search` instead of the icon.
fn named(widgets: Vec<gtk::Widget>) -> Vec<gtk::Widget> {
    for widget in &widgets {
        if let Some(tip) = widget.tooltip_text() {
            widget.update_property(&[gtk::accessible::Property::Label(&tip)]);
        }
    }
    widgets
}

/// The toolbar of a spec screen, every icon-only button named by its tooltip.
pub fn toolbar(id: &str) -> Vec<gtk::Widget> {
    named(crate::ui::toolbar(&crate::ui::screen(id)))
}

/// The toolbar of Data and storage. The spec's help button (`Tracking guide`) opens Aniyomi's Android storage FAQ,
/// which does not describe this app, so the button is left out (the dropped link is in the release notes).
fn data_storage_toolbar() -> Vec<gtk::Widget> {
    let mut bar = toolbar("settings_data_storage");
    bar.retain(|w| w.tooltip_text().as_deref() != Some("Tracking guide"));
    bar
}

/// The user categories of a kind (the Default category is not one of them), as the settings lists need them.
fn categories_of(kind: Kind) -> Vec<Category> {
    let all = crate::data::app().categories(kind).repo().all().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "the categories could not be read");
        Vec::new()
    });
    all.into_iter()
        .filter(|c| !c.is_system())
        .filter_map(|c| Some(Category { id: i32::try_from(c.id).ok()?, name: c.name, order: c.order }))
        .collect()
}

/// Pushes the Categories screen on the tab of `kind`.
fn open_categories(kind: Kind) {
    let page = crate::screens::build("categories");
    view::show_kind(&page, library::categories_tab(kind));
    crate::ui::push(&page);
}

fn reader_page() -> Option<Built> {
    let prefs = Rc::new(prefs());
    let store = prefs.store().clone();
    let typed = Rc::clone(&prefs);
    Some(view::live_page(
        &store,
        || (),
        move || reader::page::rows_all(&typed),
        toolbar("settings_reader"),
    ))
}

/// The Library page. The interval, the category sets and the rows they turn on or off are rebuilt when they change.
fn library_page() -> Option<Built> {
    let prefs = Rc::new(prefs());
    let store = prefs.store().clone();
    let (for_state, for_tree) = (Rc::clone(&prefs), Rc::clone(&prefs));
    let open: Rc<dyn Fn(Kind)> = Rc::new(open_categories);
    Some(view::live_page(
        &store,
        move || crate::settings_ui::library_update::rule_state(&for_state),
        move || {
            let typed = library::page_rows(&for_tree, &categories_of(Kind::Anime), &categories_of(Kind::Manga), Rc::clone(&open));
            merge(spec_tree("settings_library", for_tree.store()), typed)
        },
        toolbar("settings_library"),
    ))
}

/// The Appearance page. Its rows are the theme mode and the start screen, so no state of its own decides the layout.
fn appearance_page() -> Option<Built> {
    let prefs = Rc::new(prefs());
    let store = prefs.store().clone();
    let for_tree = Rc::clone(&prefs);
    Some(view::live_page(
        &store,
        || (),
        move || merge(spec_tree("settings_appearance", for_tree.store()), appearance::page_rows(&for_tree)),
        toolbar("settings_appearance"),
    ))
}

/// The rows of Sources and tracking: the Browse rows, the tracker rows and the switches (`tracking`), the cookie and
/// WebView rows, and the revoke of trusted extensions. `changed` redraws the page after a login or a revoke.
fn browse_rows(prefs: &Prefs, counts: browse::RepoCounts, changed: Rc<dyn Fn()>) -> Vec<Preference> {
    let mut out = browse::page_rows(prefs, counts);
    out.extend(tracking::page_rows(prefs, Rc::clone(&changed)));
    out.push(rows::group(&tr(MsgKey::label_network), maintenance::networking_rows()));
    out.push(rows::group(&tr(MsgKey::label_extensions), vec![maintenance::revoke_trust_row(prefs, changed)]));
    out
}

/// Puts the Sources and tracking page into `holder`. The tracker services start here, and the page redraws itself
/// once they are up and after every login change (as the Tracking page did).
fn fill_browse(holder: &gtk::Box) {
    while let Some(child) = holder.first_child() {
        holder.remove(&child);
    }
    let prefs = prefs();
    let weak = holder.downgrade();
    let changed: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(holder) = weak.upgrade() {
            fill_browse(&holder);
        }
    });
    tracker_host::set_refresh(Rc::clone(&changed));
    tracker_host::ensure();
    let tree = merge(spec_tree("settings_browse", prefs.store()), browse_rows(&prefs, repo_counts(), changed));
    holder.append(&page::body(&tree).widget);
}

fn browse_page() -> Option<Built> {
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.set_vexpand(true);
    fill_browse(&holder);
    built(holder, toolbar("settings_browse"))
}

/// Downloads and storage: the spec page with the typed download rows in their place.
fn downloads_page() -> Option<Built> {
    let prefs = prefs();
    let tree = merge(spec_tree("settings_downloads", prefs.store()), downloads::page_rows(&prefs));
    built(page::body(&tree).widget, toolbar("settings_downloads"))
}

/// The rows of a settings screen for the search index: the spec rows, with this module's typed rows merged in.
pub fn tree(id: &str) -> Vec<Preference> {
    let prefs = prefs();
    let (anime, manga) = (categories_of(Kind::Anime), categories_of(Kind::Manga));
    tree_with(id, &prefs, &anime, &manga, repo_counts())
}

/// The repository links the Browse row counts (task T-47a-062), read from the one repository list.
fn repo_counts() -> browse::RepoCounts {
    let app = crate::data::app();
    let repos = SourcePrefs::new(app.base.store().clone()).manga_extension_repos.get().len() as i64;
    browse::RepoCounts { repos }
}

/// [`tree`] over the given preferences and category lists.
pub fn tree_with(id: &str, prefs: &Prefs, anime: &[Category], manga: &[Category], counts: browse::RepoCounts) -> Vec<Preference> {
    let spec = spec_tree(id, prefs.store());
    let open: Rc<dyn Fn(Kind)> = Rc::new(|_| {});
    match id {
        "settings_appearance" => merge(spec, appearance::page_rows(prefs)),
        "settings_library" => merge(spec, library::page_rows(prefs, anime, manga, open)),
        "settings_reader" => reader::page::rows_all(prefs),
        "settings_player" => player::root::page_rows(prefs),
        "settings_browse" => merge(spec, browse_rows(prefs, counts, Rc::new(|| {}))),
        "settings_data_storage" => merge(spec, data_storage_rows(prefs, Rc::new(|| {}))),
        "settings_downloads" => merge(spec, downloads::page_rows(prefs)),
        "settings_about" => merge(spec, about::page_rows()),
        _ => spec,
    }
}

/// The typed rows of Data and storage: the storage location row and its info text (loose), the Restore backup row and
/// the storage usage group. `changed` runs after a new folder is stored or the cache is cleared.
fn data_storage_rows(prefs: &Prefs, changed: Rc<dyn Fn()>) -> Vec<Preference> {
    let mut tree: Vec<Preference> = storage::rows(prefs, Rc::clone(&changed)).into_iter().map(rows::item).collect();
    let interval = rows::ranged_choice(&tr(MsgKey::pref_backup_interval), &prefs.backup.backup_interval, |h| kuro_backup::settings::interval_label(h as u32).unwrap_or("").to_owned(), true);
    tree.push(rows::group(&tr(MsgKey::label_backup), vec![interval, restore_backup_row(), rows::info(&backup_info(prefs))]));
    tree.push(rows::group(&tr(MsgKey::pref_storage_usage), vec![cache::size_row(prefs), cache::clear_row(Rc::clone(&changed))]));
    tree.push(rows::group(&tr(MsgKey::ui_maintenance), maintenance::data_rows()));
    tree.push(library_export::group_row());
    tree
}

/// The spec group of the Library List dialog's switches (`export_columns`): the dialog is `library_export`, so the spec
/// group is left out of the page.
fn without_export_dialog(mut tree: Vec<Preference>) -> Vec<Preference> {
    tree.retain(|entry| !matches!(entry, Preference::Group(group) if group.title == library_export::dialog_title()));
    tree
}

/// The static backup info line of the spec page. The typed line below replaces it: it carries the last automatic backup.
pub fn keep_copies() -> String {
    tr(MsgKey::backup_info)
}

/// The backup info line: the advice to keep copies, then when the last automatic backup ran (`backup_interval` page).
fn backup_info(prefs: &Prefs) -> String {
    let now = i64::try_from(kuro_core::crash::now_ms()).unwrap_or(i64::MAX);
    let last = prefs.backup.last_auto_backup_timestamp.get();
    format!("{}\n\n{}", keep_copies(), kuro_backup::settings::last_auto_info(last, now))
}

/// The spec rows without the static backup info line, which the typed one replaces.
fn without_backup_info(mut tree: Vec<Preference>) -> Vec<Preference> {
    for entry in &mut tree {
        if let Preference::Group(group) = entry {
            group.items.retain(|item| !matches!(item, PreferenceItem::Info { text, .. } if text.starts_with(&keep_copies())));
        }
    }
    tree
}

/// The Restore backup row: a file chooser (`Select backup file`), then the Restore backup page on that file. A restore
/// that is already running keeps the chooser shut.
fn restore_backup_row() -> PreferenceItem {
    let mut row = rows::text(&tr(MsgKey::pref_restore_backup), None, true);
    if let PreferenceItem::Text { on_click, .. } = &mut row {
        *on_click = Some(Rc::new(choose_backup_file));
    }
    row
}

fn choose_backup_file() {
    if crate::backup_host::restore_running() {
        crate::toast::notify(&tr(MsgKey::restore_in_progress));
        return;
    }
    let dialog = gtk::FileDialog::new();
    dialog.set_title(&tr(MsgKey::file_select_backup));
    dialog.open(Some(&crate::ui::window()), None::<&gtk::gio::Cancellable>, |result| {
        let path = match result {
            Ok(file) => file.path(),
            Err(err) => return crate::toast::notify(chooser::failure_toast(&err).unwrap_or(tr(MsgKey::file_null_uri_error).to_string())),
        };
        match path {
            Some(path) => {
                crate::pref_model::backup_pages::set_restore_file(path);
                crate::ui::open("settings_restore_backup");
            }
            None => crate::toast::notify(&tr(MsgKey::file_null_uri_error)),
        }
    });
}

/// Puts the spec page of Data and storage, the live storage rows, the banner for a gone base folder and the real
/// usage rows into `holder`. Choosing a folder builds the page again so the subtitle shows it.
fn fill_data_storage(holder: &gtk::Box) {
    while let Some(child) = holder.first_child() {
        holder.remove(&child);
    }
    let prefs = prefs();
    let weak = holder.downgrade();
    let changed: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(holder) = weak.upgrade() {
            fill_data_storage(&holder);
        }
    });
    let mut tree = merge(without_export_dialog(without_backup_info(spec_tree("settings_data_storage", prefs.store()))), data_storage_rows(&prefs, changed));
    if let Some(banner) = storage::banner_row(&prefs) {
        tree.insert(0, rows::item(banner));
    }
    let body = page::body(&tree);
    usage::replace_usage_row(&body.widget, &usage::live_rows());
    holder.append(&body.widget);
}

fn data_storage_page() -> Option<Built> {
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.set_vexpand(true);
    fill_data_storage(&holder);
    built(holder, data_storage_toolbar())
}

/// About: the spec page with the Version and Check for updates rows in their place.
fn about_page() -> Option<Built> {
    let prefs = prefs();
    let tree = merge(spec_tree("settings_about", prefs.store()), about::page_rows());
    built(page::body(&tree).widget, toolbar("settings_about"))
}

/// The screen for `id`, or `None` when this module does not build it.
pub fn body(id: &str) -> Option<Built> {
    match id {
        "settings_appearance" => appearance_page(),
        "settings_library" => library_page(),
        "settings_reader" => reader_page(),
        "settings_player" => player::root::body(&prefs()),
        "settings_downloads" => downloads_page(),
        "settings_browse" => browse_page(),
        "settings_search" => search::page(),
        "settings_data_storage" => data_storage_page(),
        "settings_about" => about_page(),
        "settings_open_source_licenses" => super::licenses::list_page(),
        "settings_worker_info" => super::worker_info::page(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_handled_id_is_a_spec_screen_and_the_others_are_left_alone() {
        let ids = [
            "settings_appearance",
            "settings_library",
            "settings_reader",
            "settings_player",
            "settings_downloads",
            "settings_browse",
            "settings_search",
            "settings_data_storage",
            "settings_about",
        ];
        crate::ui::with_spec(|spec| {
            for id in ids {
                assert!(handles(id) && spec.screen(id).is_some(), "{id}");
            }
        });
        for other in ["settings", "library", "settings_debug_info"] {
            assert!(!handles(other), "{other}");
        }
    }

    /// Every row under `root`.
    fn rows_under(root: &gtk::Widget) -> usize {
        let mut n = usize::from(root.is::<gtk::ListBoxRow>());
        let mut child = root.first_child();
        while let Some(c) = child {
            n += rows_under(&c);
            child = c.next_sibling();
        }
        n
    }

    /// Every widget under `root` (itself included) that `keep` accepts.
    fn all_under(root: &gtk::Widget, keep: &dyn Fn(&gtk::Widget) -> bool) -> Vec<gtk::Widget> {
        let mut out = Vec::new();
        if keep(root) {
            out.push(root.clone());
        }
        let mut child = root.first_child();
        while let Some(c) = child {
            out.extend(all_under(&c, keep));
            child = c.next_sibling();
        }
        out
    }

    /// The label under `root` reading `text`.
    fn label_under(root: &gtk::Widget, text: &str) -> gtk::Label {
        all_under(root, &|w| w.downcast_ref::<gtk::Label>().is_some_and(|l| l.label() == text))
            .into_iter()
            .next()
            .and_downcast()
            .unwrap_or_else(|| panic!("no label {text}"))
    }

    /// Whether `w` and every parent up to `root` is visible.
    fn shown_in(w: &gtk::Widget, root: &gtk::Widget) -> bool {
        let mut cur = Some(w.clone());
        while let Some(c) = cur {
            if !c.is_visible() {
                return false;
            }
            if &c == root {
                return true;
            }
            cur = c.parent();
        }
        true
    }

    /// Runs the main loop until `done` or a second has passed.
    fn spin_until(done: impl Fn() -> bool) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while !done() && std::time::Instant::now() < until {
            gtk::glib::MainContext::default().iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Set in the child process that runs [`every_settings_page_builds_and_shows_rows`] on its own.
    const ALONE: &str = "KUROBAYOMI_TEST_ALONE";

    /// Builds every settings page the app routes (this module's, `pref_model::pages` and the shell in both layouts)
    /// over fresh data, and checks each shows rows; then the wide split view, the settings search and Categories. It
    /// installs the app data on the GTK thread for good, which would change what other display tests see, so the
    /// normal run starts this test binary again for this one test and checks that it passed.
    #[test]
    fn every_settings_page_builds_and_shows_rows() {
        if std::env::var_os(ALONE).is_none() {
            let path = module_path!().split_once("::").map_or(module_path!(), |(_, rest)| rest);
            let name = format!("{path}::every_settings_page_builds_and_shows_rows");
            let out = std::process::Command::new(std::env::current_exe().expect("the test binary"))
                .args([name.as_str(), "--exact", "--test-threads=1", "--nocapture"])
                .env(ALONE, "1")
                .output()
                .expect("the test binary runs");
            let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            assert!(out.status.success() && stdout.contains("1 passed"), "the child run failed:\n{stdout}\n{stderr}");
            return;
        }
        crate::testing::with_display(|| {
            // the app runs the GTK thread inside the async runtime; the live lists subscribe through it
            let runtime = tokio::runtime::Runtime::new().expect("a runtime");
            let _inside = runtime.enter();
            let dir = kuro_testkit::TempDir::new().expect("a temporary folder");
            crate::data::install(crate::data::open_in(dir.path()).expect("fresh data")).expect("installed");
            let _nav = crate::testing::nav();
            // pages whose rows come later or only on demand: a live list that loads, a search before a query
            let may_be_empty = [
                "settings_search",
                "settings_extension_repos",
            ];
            let ids = [
                "settings_appearance",
                "settings_library",
                "settings_reader",
                "settings_player",
                    "settings_downloads",
                "settings_browse",
                "settings_search",
                "settings_data_storage",
                "settings_about",
                "settings_open_source_licenses",
                "settings_worker_info",
            ];
            for id in ids.iter().chain(crate::pref_model::pages::IDS.iter()) {
                assert!(handles(id) || crate::pref_model::pages::IDS.contains(id), "{id} is routed here");
                eprintln!("building {id}");
                let page = crate::screens::build(id);
                let rows = rows_under(page.upcast_ref());
                assert!(rows > 0 || may_be_empty.contains(id), "{id} shows rows");
            }
            // the wide settings shell, opened on a plain page and on Player (whose own split view must not nest)
            for current in ["settings_appearance", "settings_player"] {
                let entries: Vec<_> = crate::ui::with_spec(|s| s.settings_main.clone());
                let split = crate::pref_model::shell::split(
                    entries.into_iter().map(|e| crate::pref_model::shell::NavEntry { id: e.id, title: e.title, subtitle: e.subtitle, icon: e.icon }).collect(),
                    current,
                );
                assert!(rows_under(&split) > 8, "the sidebar and the open page");
            }
            let player = crate::settings_ui::player::root::body(&prefs()).expect("the player page");
            assert!(rows_under(&player.content) >= 4, "the four kept settings");

            // the split view: one sidebar (Player inside it shows its plain list), the open page's row selected, and a
            // selected row swaps the page beside it
            let entries = || -> Vec<crate::pref_model::shell::NavEntry> {
                crate::ui::with_spec(|s| s.settings_main.clone())
                    .into_iter()
                    .map(|e| crate::pref_model::shell::NavEntry { id: e.id, title: e.title, subtitle: e.subtitle, icon: e.icon })
                    .collect()
            };
            let ids: Vec<String> = entries().into_iter().map(|e| e.id).collect();
            let sidebars = |root: &gtk::Widget| all_under(root, &|w| w.is::<gtk::ListBox>() && w.has_css_class("navigation-sidebar"));
            let split = crate::pref_model::shell::split(entries(), "settings_player");
            let found = sidebars(&split);
            assert_eq!(found.len(), 1, "Player inside the settings split shows its plain list, not a second sidebar");
            let list = found[0].clone().downcast::<gtk::ListBox>().expect("the sidebar");
            let at = ids.iter().position(|id| id == "settings_player").expect("Player is a settings page");
            assert_eq!(list.selected_row().map(|r| r.index()), i32::try_from(at).ok(), "the open page's row is selected");
            let right = split.last_child().expect("the page side");
            let open_page = || right.first_child().map(|w| w.widget_name().to_string());
            assert_eq!(open_page(), Some("screen:settings_player".to_owned()));
            list.select_row(list.row_at_index(2).as_ref());
            assert_eq!(open_page(), Some(format!("screen:{}", ids[2])), "selecting a row swaps the page");

            // the settings search: the field in the bar under the header, a hint before a query, "No results found" after a miss
            let search = crate::screens::build("settings_search");
            let header = search.first_child().and_downcast::<gtk::HeaderBar>().expect("the header row");
            let bar = header.next_sibling().expect("the search bar");
            let entry = all_under(&bar, &|w| w.is::<gtk::SearchEntry>());
            assert_eq!(entry.len(), 1, "the search field sits in the bar under the header");
            let entry = entry[0].clone().downcast::<gtk::SearchEntry>().expect("a search entry");
            let root = search.upcast_ref::<gtk::Widget>();
            let hint = label_under(root, "Type to search every setting");
            let miss = label_under(root, "No results found");
            let results = all_under(root, &|w| w.has_css_class("boxed-list"));
            assert!(shown_in(hint.upcast_ref(), root) && !shown_in(miss.upcast_ref(), root), "the hint before a query");
            entry.set_text("zzqxjv");
            spin_until(|| shown_in(miss.upcast_ref(), root));
            assert!(shown_in(miss.upcast_ref(), root), "a query with no hit says so");
            assert!(!shown_in(hint.upcast_ref(), root), "and the hint goes");
            assert!(results.iter().all(|r| !shown_in(r, root)), "and no result list shows");

            // Categories: one header row holding the Anime | Manga switch and Add; with no category the empty state
            let page = crate::screens::build("categories");
            let root = page.upcast_ref::<gtk::Widget>();
            let headers = all_under(root, &|w| w.is::<gtk::HeaderBar>());
            assert_eq!(headers.len(), 1, "one header row");
            let switches = all_under(root, &|w| w.is::<gtk::StackSwitcher>());
            assert_eq!(switches.len(), 2, "the header's switch and the narrow window's row under it");
            assert!(switches[0].is_ancestor(&headers[0]), "the kind switch is in the header row");
            let add = all_under(&headers[0], &|w| w.tooltip_text().as_deref() == Some("Add category"));
            assert_eq!(add.len(), 2, "one Add per kind in the header, the visible kind's shows");
            let empty = all_under(root, &|w| w.downcast_ref::<gtk::Label>().is_some_and(|l| l.label().starts_with("No categories yet")));
            assert!(empty.iter().any(|w| shown_in(w, root)), "the empty state of the visible kind");
        });
    }

    fn group_titles(tree: &[Preference], group: &str) -> Vec<String> {
        tree.iter()
            .find_map(|entry| match entry {
                Preference::Group(g) if g.title == group => Some(g.items.iter().map(|i| i.title().to_owned()).collect()),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[test]
    fn the_trees_the_index_reads_carry_the_typed_rows_and_the_spec_links() {
        let prefs = Prefs::in_memory();
        let appearance = tree_with("settings_appearance", &prefs, &[], &[], browse::RepoCounts::default());
        let display = group_titles(&appearance, "Display");
        for title in ["App language", "Start Screen"] {
            assert!(display.iter().any(|t| t == title), "{title} is on the Display group");
        }
        let browse = tree_with("settings_browse", &prefs, &[], &[], browse::RepoCounts::default());
        assert_eq!(group_titles(&browse, "Sources"), ["Source languages", "Repositories"]);
        let library = tree_with("settings_library", &prefs, &[], &[], browse::RepoCounts::default());
        assert!(group_titles(&library, "Season").is_empty(), "the season switches are cut");
        assert!(!group_titles(&library, "Categories").iter().any(|t| t == "Per-category settings for sort"), "per-category sort is cut");
        assert!(group_titles(&library, "Global update").iter().all(|t| t != "Smart update" && t != "Automatic updates device restrictions"));
    }

    /// The key a row writes, when it has one a test can read. Every typed row that holds a pref is covered.
    fn written_key(item: &PreferenceItem) -> Option<&str> {
        match item {
            PreferenceItem::Switch { pref, .. } => Some(pref.key()),
            PreferenceItem::List { pref, .. } => Some(pref.key()),
            PreferenceItem::MultiSelect { pref, .. } => Some(pref.key()),
            PreferenceItem::EditText { pref, .. }
            | PreferenceItem::EditTextInfo { pref, .. } => Some(pref.key()),
            _ => None,
        }
    }

    // T-02a-030: the spec converter binds a row it has no typed twin for to the made-up key "<page>/<title>", which
    // looks live and changes nothing. A typed row that is retitled would leave its spec row like that, so on every
    // settings page no switch, list, multi-select or text row may write a made-up key.
    // Slider rows carry no pref (only a value and a veto), so a key check cannot cover them.
    #[test]
    fn no_row_of_any_settings_page_writes_a_made_up_key() {
        let prefs = Prefs::in_memory();
        // The two backup pages are built by `pref_model::backup_pages` (check boxes over a typed screen state, not spec
        // rows), so their spec rows are never shown.
        let own_page = ["settings_create_backup", "settings_restore_backup"];
        let mut ids: Vec<String> = crate::ui::with_spec(|spec| {
            spec.screens.keys().filter(|id| id.starts_with("settings_") && !own_page.contains(&id.as_str())).cloned().collect()
        });
        ids.sort();
        assert!(ids.len() > 10, "the scan finds the pages ({ids:?})");
        let mut made_up = Vec::new();
        let trees = ids.iter().map(|id| (id.as_str(), tree_with(id, &prefs, &[], &[], browse::RepoCounts::default())));
        for (id, tree) in trees {
            for entry in tree {
                let items: Vec<&PreferenceItem> = match &entry {
                    Preference::Item(item) => vec![item],
                    Preference::Group(group) => group.items.iter().collect(),
                };
                for item in items {
                    if let Some(key) = written_key(item).filter(|key| key.starts_with(&format!("{id}/"))) {
                        made_up.push(format!("{id}: {:?} -> {key}", item.title()));
                    }
                }
            }
        }
        assert!(made_up.is_empty(), "rows bound to a made-up key:\n{}", made_up.join("\n"));
    }

    // A spec row whose typed twin does not match its title shows twice (the Tracking page once showed "Show next
    // episode's airing time" with and without a trailing space). No group of a typed page may repeat a title.
    #[test]
    fn no_group_of_a_typed_page_shows_a_row_twice() {
        let prefs = Prefs::in_memory();
        for id in [
            "settings_appearance",
            "settings_library",
            "settings_reader",
            "settings_downloads",
            "settings_browse",
            "settings_data_storage",
            "settings_about",
        ] {
            let tree = tree_with(id, &prefs, &[], &[], browse::RepoCounts::default());
            let loose: Vec<&PreferenceItem> = tree.iter().filter_map(|e| if let Preference::Item(i) = e { Some(i) } else { None }).collect();
            let mut groups: Vec<(String, Vec<&PreferenceItem>)> = vec![("(loose rows)".to_owned(), loose)];
            for entry in &tree {
                if let Preference::Group(g) = entry {
                    groups.push((g.title.clone(), g.items.iter().collect()));
                }
            }
            for (group, items) in groups {
                let mut seen = std::collections::BTreeSet::new();
                for item in items.iter().filter(|i| !i.title().is_empty()) {
                    assert!(seen.insert(item.title().to_owned()), "{id} / {group}: {:?} is shown twice", item.title());
                }
            }
        }
    }

    #[test]
    fn the_search_index_finds_the_subtitle_and_mpv_rows_on_their_player_pages() {
        let prefs = Prefs::in_memory();
        let index = search::index_from(|id| tree_with(id, &prefs, &[], &[], browse::RepoCounts::default()));
        let language = search::hits(&index, "subtitle language");
        assert!(language.iter().any(|hit| hit.screen == "settings_player"), "the subtitle language row is found on the Player page");
        let mpv = search::hits(&index, "mpv");
        assert!(mpv.is_empty(), "the Advanced page with the MPV rows is cut");
    }

    #[test]
    fn data_and_storage_carries_the_live_storage_row_in_place_of_the_spec_one() {
        let prefs = Prefs::in_memory();
        let tree = tree_with("settings_data_storage", &prefs, &[], &[], browse::RepoCounts::default());
        let Some(Preference::Item(first)) = tree.first() else { panic!("the page starts with the loose storage rows") };
        let PreferenceItem::Text { on_click, .. } = first else { panic!("the storage location row") };
        assert!(on_click.is_some(), "the row opens the folder chooser");
        assert_eq!(first.title(), "Storage location");
        assert!(group_titles(&tree, "Backup and restore").iter().any(|t| t == "Create backup"), "the rest of the spec page stays");
    }
}
