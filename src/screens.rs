// Hand-built screens. Labels, menus, sheets and dialogs come from spec/*.json; layout follows the
// "notes" Aniyomi layouts. Anything without a builder here falls back to ui::generic.
use kuro_i18n::{MsgKey, tr};
use crate::ui::*;
use gtk::prelude::*;
use gtk::{Align, Box as GBox, Button, Label, Orientation as O};
use kuro_platform::kind::Kind;

pub struct Built {
    pub content: gtk::Widget,
    pub end: Vec<gtk::Widget>,
}

pub fn built(content: impl IsA<gtk::Widget>, end: Vec<gtk::Widget>) -> Option<Built> {
    Some(Built { content: content.upcast(), end })
}

/// Builder for a screen's body + header actions, if it has a custom one.
pub fn body(id: &str) -> Option<Built> {
    match id {
        "library" => libraries(),
        "anime_library" => Some(library(id, Kind::Anime)),
        "manga_library" => Some(library(id, Kind::Manga)),
        "updates" => crate::updates::mount::updates().or_else(|| unavailable(id)),
        "history" => crate::updates::mount::history().or_else(|| unavailable(id)),
        "browse" => crate::browse::mount::browse().or_else(|| unavailable(id)),
        "global_search" | "migrate_search" => crate::browse::search_screen::body(id == "migrate_search").or_else(|| unavailable(id)),
        "download_queue" => downloads(),
        "categories" => categories(),
        "storage" => storage(),
        id if crate::settings_ui::handles(id) => crate::settings_ui::body(id),
        _ => crate::entry::body(id),
    }
}

/// A pushed page for `id`.
pub fn build(id: &str) -> GBox {
    match id {
        "upcoming" => {
            if let Some(page) = crate::updates::mount::upcoming() {
                return page;
            }
        }
        "anime_entry_details" => return crate::entry_core::details::route::build(),
        "manga_entry_details" => return crate::entry_core::details::route::build_manga(),
        "browse_source" | "migrate_source_search" => {
            if let Some(page) = crate::browse::browse_screen::page(id) {
                return page;
            }
        }
        "migrate_entries" => {
            if let Some(page) = crate::browse::migrate_screen::page() {
                return page;
            }
        }
        "source_preferences" => {
            if let Some(page) = crate::browse::source_prefs_screen::page() {
                return page;
            }
        }
        _ => {}
    }
    let s = screen(id);
    match body(id) {
        Some(b) => {
            // a page with its own one-row header (the kind pages) is used as it is
            let p = match b.content.clone().downcast::<GBox>() {
                Ok(own) if own.first_child().is_some_and(|w| w.is::<gtk::HeaderBar>()) => own,
                _ => page(&crate::spec::shown(&s.title), &b.content, b.end, true),
            };
            tag(&p, id);
            p
        }
        None => generic(&s),
    }
}

// ---------- shared pieces ----------

/// Whether manga row `id` belongs to the local source.
fn is_local_manga(id: i64) -> bool {
    crate::data::is_installed()
        && kuro_data::entry::EntryRepo::new(crate::data::app().db.clone())
            .by_id(id)
            .is_ok_and(|e| e.kind == Kind::Manga && e.source == kuro_local::LOCAL_SOURCE_ID)
}

/// Opens the details page of entry `id`, by id. A local manga is opened through the local flow, which also remembers
/// the id for the reader to tell two local folders with one name apart; any other manga is a plain entry page.
pub fn open_entry(kind: Kind, id: i64) {
    match kind {
        Kind::Anime => crate::entry_core::details::route::open(id),
        Kind::Manga if is_local_manga(id) => crate::local_flow::open::entry(id),
        Kind::Manga => crate::entry_core::details::route::open_kind(Kind::Manga, id),
    }
}

/// Opens unit `unit_id` (a chapter or an episode row id) of entry `entry_id` in the reader or the player, by id. A
/// chapter opens in the reader. An episode plays from its file, or from the online source that owns it; when neither
/// can, a toast says why.
pub fn open_unit(kind: Kind, entry_id: i64, unit_id: i64) {
    if !crate::data::is_installed() {
        return;
    }
    if !kind.is_anime() {
        push(&if is_local_manga(entry_id) { crate::local_flow::reader::open_local_chapter(entry_id, unit_id) } else { crate::reader::launch::open_chapter(entry_id, unit_id) });
        return;
    }
    if crate::local_flow::open::open_local_episode(unit_id) || crate::player_view::open_source_episode(&anime_sources(), unit_id) {
        return;
    }
    // a source the user turned off keeps its downloads playable
    if crate::player_view::open_downloaded(unit_id) {
        return;
    }
    let source = kuro_data::item::ItemRepo::new(crate::data::app().db.clone())
        .by_id(unit_id)
        .ok()
        .flatten()
        .and_then(|i| kuro_data::entry::EntryRepo::new(crate::data::app().db.clone()).by_id(i.entry_id).ok())
        .map(|e| e.source);
    if source.is_some_and(|source| crate::source_switch::offer_turn_on(source, move || open_unit(kind, entry_id, unit_id))) {
        return;
    }
    let source = source.map_or_else(|| unit_id.to_string(), |source| source.to_string());
    crate::toast::notify(crate::reader::strings::source_not_installed(&source));
}

/// Opens a unit by its entry's title and its own name, which finds only a local entry's file. Kept for
/// `entry_core::details::route::open_player`, whose fallback should call [`open_unit`] with the episode id instead.
// ponytail: delete when entry_core opens its fallback by id.
pub fn open_media(kind: Kind, title: &str, unit: &str) {
    if kind.is_anime() {
        if !crate::local_flow::open::open_episode(title, unit) {
            crate::toast::notify(crate::reader::strings::source_not_installed(title));
        }
    } else {
        push(&crate::local_flow::reader::page_for(title, unit));
    }
}

/// The anime sources this build runs: the builtin ones (none) and the recipe sources.
pub fn anime_sources() -> Vec<std::sync::Arc<dyn kuro_source::anime::AnimeSource>> {
    let mut sources = kuro_source::anime::registry::builtin_anime_sources();
    sources.extend(crate::recipe_sources::anime_sources());
    sources
}

/// What a page shows when its live builder cannot start (no app data, or nothing was parked for it): the screen's
/// empty text, never made-up rows.
pub(crate) fn unavailable(id: &str) -> Option<Built> {
    Some(empty_built(id))
}

fn empty_built(id: &str) -> Built {
    let text = screen(id).empty.unwrap_or_else(|| kuro_i18n::tr(kuro_i18n::MsgKey::no_results_found));
    Built { content: crate::browse::look::empty_state("system-search-symbolic", &text, &[]).0.upcast(), end: vec![] }
}

/// Opens the spec dialog `dialog` of screen `id`, its text naming `name` where it has `%s` (the delete-category
/// question). The last button runs `on_ok` before closing.
pub(crate) fn dialog_then_named(id: &str, dialog: &str, name: &str, on_ok: impl Fn() + 'static) {
    if let Some(d) = screen(id).dialogs.iter().find(|d| d.id == dialog) {
        let mut d = d.clone();
        d.text = d.text.map(|text| text.replace("%s", name));
        show_dialog_then(&d, on_ok);
    }
}

pub(crate) fn fab(overlay: &gtk::Overlay, label: &str, icon: &str) -> Button {
    let h = GBox::new(O::Horizontal, 6);
    h.append(&gtk::Image::from_icon_name(icon));
    h.append(&Label::new(Some(label)));
    let b = Button::builder().child(&h).css_classes(["suggested-action", "pill"]).halign(Align::End).valign(Align::End).build();
    margins(&b, 18);
    overlay.add_overlay(&b);
    b
}

// ---------- library ----------

/// The Library destination as a whole page: one header row (Anime | Manga switch, the visible kind's search, its
/// refresh and buttons) over the two library views (docs/design/main-window.md). The page has no title of its own.
fn libraries() -> Option<Built> {
    built(kind_page([Kind::Anime, Kind::Manga].map(|kind| library(if kind.is_anime() { "anime_library" } else { "manga_library" }, kind)), false), vec![])
}

/// The one builder of a page with a view per kind, from the Anime and the Manga views in that order: the switch at the
/// start of the header row, the visible kind's search toggle first among its other `end` widgets at the end (the view
/// hands its `gtk::SearchEntry` first in `Built.end`, when it has one), and that kind's search bar under the header
/// (`ui::search_row`); `back` adds Back before the switch, for a pushed page. Library, Updates, History, Downloads,
/// Categories and Storage use it (`updates::mount` for the tabs).
pub(crate) fn kind_page(views: [Built; 2], back: bool) -> GBox {
    let kinds = gtk::Stack::new();
    kinds.set_transition_type(gtk::StackTransitionType::Crossfade);
    kinds.set_vexpand(true);
    let (bars, end) = (gtk::Stack::new(), gtk::Stack::new());
    // the visible kind's search toggle is the first of its actions at the end. A tab page has no ⋮ menu: its actions are
    // buttons at the end (guard test `no_tab_page_has_a_more_menu`)
    for (b, anime) in views.into_iter().zip([true, false]) {
        let name = if anime { "anime" } else { "manga" };
        kinds.add_titled(&b.content, Some(name), if anime { "Anime" } else { "Manga" });
        let mut rest = b.end.into_iter().peekable();
        let actions = GBox::new(O::Horizontal, 4);
        let bar: gtk::Widget = match rest.next_if(|w| w.is::<gtk::SearchEntry>()).and_downcast::<gtk::SearchEntry>() {
            // keys typed in this kind's content open its bar; the other kind's content is hidden and takes none
            Some(entry) => {
                let (bar, toggle) = crate::ui::search_row(&entry, &b.content);
                actions.append(&toggle);
                bar.upcast()
            }
            None => GBox::new(O::Horizontal, 0).upcast(),
        };
        bars.add_named(&bar, Some(name));
        for w in rest {
            actions.append(&w);
        }
        end.add_named(&actions, Some(name));
    }
    for follower in [&bars, &end] {
        kinds.bind_property("visible-child-name", follower, "visible-child-name").sync_create().build();
    }
    crate::shell::kind_choice::follow(&kinds, ["anime", "manga"]);
    let header = crate::ui::header_bar();
    header.set_decoration_layout(Some(&decoration(false)));
    if back {
        header.pack_start(&crate::ui::back_button());
    }
    let switch = gtk::StackSwitcher::builder().stack(&kinds).build();
    header.pack_start(&switch);
    let narrow_row = crate::ui::switch_with_narrow_row(&header, &switch);
    // an empty box, not None: without a title widget GTK shows the window title
    header.set_title_widget(Some(&GBox::new(O::Horizontal, 0)));
    header.pack_end(&end);
    no_sticky_focus(&header);
    let page = GBox::new(O::Vertical, 0);
    page.append(&header);
    page.append(&narrow_row);
    page.append(&bars);
    page.append(&kinds);
    page
}

fn library(id: &str, kind: Kind) -> Built {
    crate::library::mount::build(kind).unwrap_or_else(|| empty_built(id))
}

// ---------- downloads & categories ----------

/// Download queue: a page per kind from the managers' queue, else the Library's empty state with one way forward.
fn downloads() -> Option<Built> {
    built(kind_page([Kind::Anime, Kind::Manga].map(downloads_view), false), vec![])
}

fn downloads_view(kind: Kind) -> Built {
    if let Some((page, search, buttons)) = crate::downloads::queue_page::page(kind) {
        // refresh checks the download folders of this kind again (Settings' Reindex downloads, for one kind)
        let refresh = crate::ui::refresh_button(&kuro_i18n::tr(kuro_i18n::MsgKey::downloads_check_files));
        refresh.connect_clicked(move |button| {
            crate::ui::spin_once(button);
            if let Some(downloads) = crate::shell::host::Shell::get().and_then(|shell| shell.download_service()) {
                downloads.reindex(Some(kind));
                crate::toast::notify(&tr(MsgKey::ui_checking_the_downloaded_files_again));
            }
        });
        let end = crate::ui::HeaderActions { search: Some(search), refresh: Some(refresh.upcast()), extra: buttons, ..Default::default() }.widgets();
        return Built { content: scrolled(&page).upcast(), end };
    }
    let library = Button::with_label(&kuro_i18n::tr(kuro_i18n::MsgKey::label_library));
    library.connect_clicked(|_| open("library"));
    let (page, _) = crate::browse::look::empty_state("folder-download-symbolic", &screen("download_queue").empty.unwrap_or_default(), std::slice::from_ref(&library));
    // nothing to filter, so no search
    Built { content: page.upcast(), end: vec![] }
}

/// Categories: a page per kind under the shared header, with Add (for that kind) at the end.
fn categories() -> Option<Built> {
    built(kind_page([Kind::Anime, Kind::Manga].map(crate::live::categories_content), true), vec![])
}

fn storage() -> Option<Built> {
    crate::updates::mount::storage()
}

/// Harness for tests that need the app data installed. `data::install` binds the data to the GTK thread for good, so
/// the normal run starts this test binary again for the one test, with a temporary home and fresh databases, and the
/// child runs the body. Without a display the test skips itself.
#[cfg(test)]
pub(crate) mod alone {
    use gtk::glib;
    use gtk::prelude::*;

    const ALONE: &str = "KUROBAYOMI_TEST_ALONE";

    /// Runs `body` on the GTK thread of a child process: the data installed, a main context owned, a runtime entered.
    /// Call it as `alone::run(module_path!(), "this_test_name", |ctx| { .. })`.
    pub fn run(module: &str, name: &str, body: impl FnOnce(&glib::MainContext) + Send + 'static) {
        if std::env::var_os(ALONE).is_none() {
            if !crate::testing::has_display() {
                eprintln!("skipped: no display for this GTK test");
                return;
            }
            let home = tempfile::tempdir().expect("a temporary home");
            let path = module.split_once("::").map_or(module, |(_, rest)| rest);
            let test = format!("{path}::{name}");
            let mut child = std::process::Command::new(std::env::current_exe().expect("the test binary"));
            // the platform's base-directory variables would point past the temporary home
            for (key, _) in std::env::vars().filter(|(k, _)| ["_DATA_HOME", "_CONFIG_HOME", "_CACHE_HOME", "_STATE_HOME"].iter().any(|s| k.ends_with(s))) {
                child.env_remove(key);
            }
            let out = child.args([test.as_str(), "--exact", "--test-threads=1", "--nocapture"]).env(ALONE, "1").env("HOME", home.path()).output().expect("the test binary runs");
            let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            assert!(out.status.success() && stdout.contains("1 passed"), "the child run of {test} failed:\n{stdout}\n{stderr}");
            assert!(!stderr.contains("skipped: no display"), "the child lost the display:\n{stderr}");
            return;
        }
        crate::testing::with_display(move || {
            let ctx = glib::MainContext::default();
            let _owner = ctx.acquire().expect("main context");
            let runtime = tokio::runtime::Runtime::new().expect("a runtime");
            let _inside = runtime.enter();
            let dir = tempfile::tempdir().expect("a temporary folder");
            crate::data::install(crate::data::open_in(dir.path()).expect("fresh data")).expect("installed");
            body(&ctx);
        });
    }

    /// Pumps `ctx` until `done` holds or three seconds pass.
    pub fn pump(ctx: &glib::MainContext, done: impl Fn() -> bool) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(3) {
            while ctx.iteration(false) {}
            if done() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        done()
    }

    /// `root` and every widget below it, in tree order.
    pub fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut all = vec![root.clone()];
        let mut child = root.first_child();
        while let Some(c) = child {
            all.extend(descendants(&c));
            child = c.next_sibling();
        }
        all
    }

    /// The toast overlay of a test: the label the toast presenter writes to.
    pub fn toast_label() -> gtk::Label {
        let overlay = gtk::Overlay::new();
        crate::toast::attach(&overlay);
        let revealer = overlay.last_child().and_downcast::<gtk::Revealer>().expect("the toast revealer");
        revealer.child().and_then(|bar| bar.first_child()).and_downcast::<gtk::Label>().expect("the toast label")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The widgets a user can see: a stack contributes only its visible page.
    fn shown(root: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
        if root.is::<gtk::StackSwitcher>() {
            return; // its button labels are the switch, not titles
        }
        if let Some(stack) = root.downcast_ref::<gtk::Stack>() {
            if let Some(page) = stack.visible_child() {
                out.push(page.clone());
                shown(&page, out);
            }
            return;
        }
        let mut child = root.first_child();
        while let Some(w) = child {
            if w.is_visible() {
                out.push(w.clone());
                shown(&w, out);
            }
            child = w.next_sibling();
        }
    }

    fn view(word: &str) -> Built {
        let search = gtk::SearchEntry::new();
        let refresh = icon_button("view-refresh-symbolic", "Update category");
        let menu = crate::ui::quiet_menu(gtk::MenuButton::builder().icon_name("open-menu-symbolic").build());
        Built { content: Label::new(Some(word)).upcast(), end: vec![search.upcast(), refresh.upcast(), menu.upcast()] }
    }

    /// Needs a display and runs in a child process with fresh app data (see [`alone`]).
    #[test]
    fn opening_by_id_reaches_the_entry_page_the_reader_and_the_source_not_installed_toast() {
        use kuro_data::entry::{Entry, EntryRepo};
        use kuro_data::item::{Item, ItemRepo};
        alone::run(module_path!(), "opening_by_id_reaches_the_entry_page_the_reader_and_the_source_not_installed_toast", |ctx| {
            let (nav, _window) = crate::testing::nav();
            let toast = alone::toast_label();
            let app = crate::data::app();
            let top = || nav.visible_child().map(|c| c.widget_name().to_string());
            let (entries, items) = (EntryRepo::new(app.db.clone()), ItemRepo::new(app.db.clone()));
            let manga = |source: i64, url: &str| {
                let id = entries.insert(&Entry { source, url: url.into(), title: url.into(), ..Entry::create(Kind::Manga) }).expect("manga");
                let added = items.add_all(&[Item { entry_id: id, url: "/c1".into(), name: "Chapter 1".into(), number: 1.0, ..Item::create(Kind::Manga) }]).expect("chapter");
                (id, added[0].id)
            };
            let (remote, remote_chapter) = manga(7, "/remote");
            let (local, local_chapter) = manga(kuro_local::LOCAL_SOURCE_ID, "/local");
            assert!(!is_local_manga(remote) && is_local_manga(local), "only the local source's rows are local");
            assert!(!is_local_manga(99_999), "a missing row is not local");

            // a remote manga's page leaves the reader's local-entry memory alone; a local one sets it
            assert_eq!(crate::local_flow::open::current(), None);
            open_entry(Kind::Manga, remote);
            assert!(alone::pump(ctx, || top().as_deref() == Some("screen:manga_entry_details")), "a remote manga opens its entry page");
            assert_eq!(crate::local_flow::open::current(), None, "a remote manga is not the local entry the reader disambiguates");
            open_entry(Kind::Manga, local);
            assert!(alone::pump(ctx, || crate::local_flow::open::current() == Some(local)), "a local manga is remembered for the reader");
            assert_eq!(top().as_deref(), Some("screen:manga_entry_details"));

            // a chapter opens in the reader, from the online or the local path
            open_unit(Kind::Manga, remote, remote_chapter);
            assert!(alone::pump(ctx, || top().as_deref() == Some("screen:reader")), "a remote chapter opens the reader");
            crate::testing::nav(); // an empty stack, so the second reader is the one that is seen
            open_unit(Kind::Manga, local, local_chapter);
            assert!(alone::pump(ctx, || top().as_deref() == Some("screen:reader")), "a local chapter opens the reader");

            // an episode of a source that is not installed says so, and opens nothing
            let anime = entries.insert(&Entry { source: 7, url: "/a".into(), title: "A".into(), ..Entry::create(Kind::Anime) }).expect("anime");
            let episode = items.add_all(&[Item { entry_id: anime, url: "/e1".into(), name: "Episode 1".into(), number: 1.0, ..Item::create(Kind::Anime) }]).expect("episode")[0].id;
            let before = top();
            open_unit(Kind::Anime, anime, episode);
            assert!(alone::pump(ctx, || toast.text() == crate::reader::strings::source_not_installed("7")), "the toast names the missing source, got {:?}", toast.text());
            assert_eq!(top(), before, "no page was opened");
            open_entry(Kind::Anime, anime);
            assert!(alone::pump(ctx, || top().as_deref() == Some("screen:anime_entry_details")), "an anime opens its entry page by id");
        });
    }

    #[test]
    fn the_delete_category_question_names_the_category() {
        let text = screen("categories").dialogs.iter().find(|d| d.id == "category_delete").and_then(|d| d.text.clone()).expect("the dialog");
        assert_eq!(text.replace("%s", "Reading"), "Do you wish to delete the category \"Reading\"?");
    }

    /// Needs a display, and skips itself without one.
    #[test]
    fn the_download_queue_is_an_empty_state_with_one_way_forward() {
        crate::testing::with_display(|| {
            let page = build("download_queue");
            let mut all = vec![page.clone().upcast::<gtk::Widget>()];
            shown(page.upcast_ref(), &mut all);
            let count = |f: &dyn Fn(&gtk::Widget) -> bool| all.iter().filter(|w| f(w)).count();
            assert_eq!(count(&|w| w.is::<gtk::HeaderBar>()), 1, "one header row");
            assert_eq!(count(&|w| w.is::<gtk::ListBoxRow>()), 0, "no made-up queue rows");
            assert_eq!(count(&|w| w.downcast_ref::<Label>().is_some_and(|l| l.label() == "No downloads")), 1);
            assert_eq!(count(&|w| w.downcast_ref::<Button>().is_some_and(|b| b.label().as_deref() == Some("Library"))), 1, "one action");
            assert_eq!(count(&|w| w.is::<gtk::MenuButton>()), 0, "no menus that do nothing");
            assert_eq!(count(&|w| w.is::<gtk::StackSwitcher>()), 1, "the Anime | Manga switch");
            assert_eq!(count(&|w| w.is::<gtk::SearchEntry>()), 0, "nothing to search in an empty queue");
        });
    }

    #[test]
    fn the_library_header_is_one_row_with_each_control_once() {
        crate::testing::with_display(|| {
            let page = kind_page([view("anime grid"), view("manga grid")], false);
            assert!(page.first_child().is_some_and(|w| w.is::<gtk::HeaderBar>()), "the header row comes first");
            let bars = page.first_child().and_then(|w| w.next_sibling()).and_then(|row| row.next_sibling()).expect("the search bars follow the header and its narrow switch row");
            assert!(bars.is::<gtk::Stack>() && bars.next_sibling().is_some_and(|w| w.next_sibling().is_none()), "then the views");
            let mut all = vec![];
            shown(page.upcast_ref(), &mut all);
            let count = |f: &dyn Fn(&gtk::Widget) -> bool| all.iter().filter(|w| f(w)).count();
            assert_eq!(count(&|w| w.is::<gtk::StackSwitcher>()), 1, "the Anime | Manga switch");
            assert_eq!(count(&|w| w.is::<gtk::SearchEntry>()), 1, "one search");
            assert_eq!(count(&|w| w.is::<gtk::SearchBar>()), 1, "one search bar, under the header");
            assert_eq!(count(&|w| w.downcast_ref::<gtk::ToggleButton>().is_some_and(|b| b.icon_name().as_deref() == Some("system-search-symbolic"))), 1, "one search toggle");
            assert_eq!(count(&|w| w.is::<gtk::MenuButton>()), 1, "one menu");
            let refresh = |w: &gtk::Widget| w.downcast_ref::<Button>().is_some_and(|b| b.icon_name().as_deref() == Some("view-refresh-symbolic"));
            assert_eq!(count(&refresh), 1, "one refresh");
            assert_eq!(count(&|w| w.downcast_ref::<Label>().is_some_and(|l| matches!(l.label().as_str(), "Library" | "Anime" | "Manga"))), 0, "no page title");
            assert_eq!(count(&|w| w.downcast_ref::<Label>().is_some_and(|l| l.label() == "anime grid")), 1, "Anime shows first");
        });
    }
}
