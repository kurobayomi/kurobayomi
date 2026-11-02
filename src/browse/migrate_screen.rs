//! The migrate favourites screen (`migrate_entries`; T-41a-037): the library entries of one source that a migration can
//! start from, for either catalogue. The rows are `pages::migrate_entries`; this reads the favourites, says what the
//! screen shows (`body_of`), and opens the page the `MigrateEntries` route parked. A row opens the migrate search for
//! that entry and its cover opens the entry. The title is the source's name, or its id for a source that is not installed
//! (a stub), where upstream reads `state.source!!`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use kuro_domain::source::model::Source;
use kuro_i18n::{MsgKey, tr};

use super::directory::source_or_stub;
use super::pages::{MigrateEntriesNav, MigrateEntry, migrate_entries};
use super::pending;
use super::route::BrowseRoute;
use kuro_platform::kind::Kind;
use super::view::file_cover_loader;

/// What the screen shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Loading,
    Entries(Vec<MigrateEntry>),
    /// The source has no favourites: `empty_screen`.
    Empty,
    /// The read failed: the `internal_error` error page, with Retry.
    Failed,
}

/// The body for a read that has finished (`Some`) or not yet (`None`).
pub fn body_of(read: Option<Result<Vec<MigrateEntry>, String>>) -> Body {
    match read {
        None => Body::Loading,
        Some(Err(_)) => Body::Failed,
        Some(Ok(entries)) if entries.is_empty() => Body::Empty,
        Some(Ok(entries)) => Body::Entries(entries),
    }
}

/// The page title: the source's name, or its id when it has none (a stub).
pub fn page_title(source: &Source) -> String {
    if source.name.trim().is_empty() { source.id.to_string() } else { source.name.clone() }
}

#[allow(dead_code)] // kept for the shape of the type; not built or read yet
/// What the screen needs of the app.
pub struct MigrateEnv {
    pub source: Source,
    /// The kind of the entries, for their covers' cache key.
    pub kind: Kind,
    /// A row: the migrate search for that entry, with its title.
    pub open_search: Rc<dyn Fn(i64, &str)>,
    /// A cover: the entry itself.
    pub open_entry: Rc<dyn Fn(i64)>,
}

/// The screen's body: a spinner until `set` is called with the result of the read.
pub struct MigrateEntriesView {
    root: gtk::Stack,
    env: MigrateEnv,
    list: gtk::Box,
    shown: RefCell<Vec<MigrateEntry>>,
    /// Reads the favourites again: the Retry of the error page, set by [`load`].
    retry: RefCell<Option<Rc<dyn Fn()>>>,
}

impl MigrateEntriesView {
    pub fn new(env: MigrateEnv) -> Rc<Self> {
        let root = gtk::Stack::new();
        root.add_named(&crate::widgets::status::loading(None), Some("loading"));
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.set_vexpand(true);
        root.add_named(&list, Some("list"));
        let failed = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_named(&failed, Some("failed"));
        root.add_named(&crate::widgets::status::empty("view-list-symbolic", &tr(MsgKey::empty_screen), None), Some("empty"));
        root.set_vexpand(true);
        root.set_visible_child_name("loading");
        Rc::new(Self { root, env, list, shown: RefCell::new(Vec::new()), retry: RefCell::new(None) })
    }

    pub fn widget(&self) -> &gtk::Stack {
        &self.root
    }

    /// A row was activated: the migrate search for that entry, which starts from its title.
    fn open_search(&self, id: i64) {
        let title = self.shown.borrow().iter().find(|entry| entry.id == id).map(|entry| entry.title.clone()).unwrap_or_default();
        (self.env.open_search)(id, &title);
    }

    /// Shows the result of the read.
    pub fn set(self: &Rc<Self>, read: Result<Vec<MigrateEntry>, String>) {
        match body_of(Some(read)) {
            Body::Loading => self.root.set_visible_child_name("loading"),
            Body::Empty => self.root.set_visible_child_name("empty"),
            Body::Failed => {
                let retry = self.retry.borrow().clone();
                let page = crate::widgets::status::error(&tr(MsgKey::internal_error), None, retry.map(|retry| move || retry()));
                if let Some(failed) = self.root.child_by_name("failed").and_downcast::<gtk::Box>() {
                    while let Some(old) = failed.first_child() {
                        failed.remove(&old);
                    }
                    failed.append(&page);
                }
                self.root.set_visible_child_name("failed");
            }
            Body::Entries(entries) => {
                while let Some(child) = self.list.first_child() {
                    self.list.remove(&child);
                }
                *self.shown.borrow_mut() = entries.clone();
                let (this, other) = (Rc::downgrade(self), Rc::clone(&self.env.open_entry));
                let nav = MigrateEntriesNav {
                    open_search: Rc::new(move |id| {
                        if let Some(this) = this.upgrade() {
                            this.open_search(id);
                        }
                    }),
                    open_entry: other,
                    load_cover: file_cover_loader(self.env.kind),
                };
                self.list.append(&migrate_entries(&entries, None, nav));
                self.root.set_visible_child_name("list");
            }
        }
    }
}

/// Reads the favourites of a source off the GTK thread and shows them.
pub fn load(view: &Rc<MigrateEntriesView>, read: Arc<dyn Fn() -> Result<Vec<MigrateEntry>, String> + Send + Sync>) {
    let again = (Rc::downgrade(view), Arc::clone(&read));
    *view.retry.borrow_mut() = Some(Rc::new(move || {
        if let Some(view) = again.0.upgrade() {
            view.root.set_visible_child_name("loading");
            load(&view, Arc::clone(&again.1));
        }
    }));
    let view = Rc::clone(view);
    gtk::glib::spawn_future_local(async move {
        let result = crate::background::blocking(move || read()).await.unwrap_or_else(|_| Err("the favourites read was cancelled".to_owned()));
        if let Err(error) = &result {
            tracing::error!(%error, "the favourites to migrate could not be read");
        }
        view.set(result);
    });
}

/// The favourites of `source_id` of `kind`, as rows.
fn favourites(kind: Kind, source_id: i64) -> Result<Vec<MigrateEntry>, String> {
    use kuro_data::entry::EntryRepo;

    EntryRepo::new(crate::data::app().db.clone())
        .favorites_by_source(kind, source_id)
        .map(|list| list.into_iter().map(|e| MigrateEntry { id: e.id, title: e.title, cover: e.thumbnail_url }).collect())
        .map_err(|error| error.to_string())
}

/// The source of a route: the installed one, or a stub.
fn source_of(kind: Kind, source_id: i64) -> Source {
    source_or_stub(Some(&*super::directory::current(kind)), source_id)
}

/// The page for the parked `MigrateEntries` route, as `screens::build` asks for it. `None` leaves the screen to its sample
/// page: no app data, or nothing was parked for it.
pub fn page() -> Option<gtk::Box> {
    if !crate::data::is_installed() {
        return None;
    }
    let Some(BrowseRoute::MigrateEntries { kind, source_id }) = pending::take_for("migrate_entries") else { return None };
    let source = source_of(kind, source_id);
    let title = page_title(&source);
    let env = MigrateEnv {
        source,
        kind,
        open_search: Rc::new(move |entry_id, title| {
            crate::entry_core::handoff::open_migrate(crate::entry_core::handoff::MigrateRequest { entry_id, kind, title: title.to_owned() });
        }),
        open_entry: Rc::new(move |id| super::browse_screen::open_entry(kind, id)),
    };
    let view = MigrateEntriesView::new(env);
    load(&view, Arc::new(move || favourites(kind, source_id)));
    let page = crate::ui::page(&title, view.widget(), Vec::new(), true);
    super::glue::keep_alive(&page, &view);
    crate::ui::tag(&page, "migrate_entries");
    Some(page)
}

#[cfg(test)]
mod pure_tests {
    use super::*;

    fn entry(id: i64, title: &str) -> MigrateEntry {
        MigrateEntry { id, title: title.into(), cover: None }
    }

    #[test]
    fn the_body_follows_the_read() {
        assert_eq!(body_of(None), Body::Loading);
        assert_eq!(body_of(Some(Err("db closed".into()))), Body::Failed, "a failed read is not an empty source");
        assert_eq!(body_of(Some(Ok(Vec::new()))), Body::Empty);
        assert_eq!(body_of(Some(Ok(vec![entry(1, "A")]))), Body::Entries(vec![entry(1, "A")]));
    }

    #[test]
    fn the_title_is_the_sources_name_or_its_id_for_a_stub() {
        assert_eq!(page_title(&Source::new(5, "Alpha", "en")), "Alpha");
        let stub = Source { is_stub: true, ..Source::new(9, "", "") };
        assert_eq!(page_title(&stub), "9", "a missing source shows its id instead of crashing");
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

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

    fn view() -> (Rc<MigrateEntriesView>, Rc<RefCell<Vec<String>>>) {
        let log: Rc<RefCell<Vec<String>>> = Rc::default();
        let (search, entry) = (Rc::clone(&log), Rc::clone(&log));
        let env = MigrateEnv {
            source: Source::new(1, "Alpha", "en"),
            kind: Kind::Manga,
            open_search: Rc::new(move |id, title| search.borrow_mut().push(format!("search {id} {title}"))),
            open_entry: Rc::new(move |id| entry.borrow_mut().push(format!("entry {id}"))),
        };
        (MigrateEntriesView::new(env), log)
    }

    fn the_view_is_a_spinner_then_the_rows_the_empty_screen_or_the_error_page() {
        let (view, _) = view();
        assert_eq!(view.widget().visible_child_name().as_deref(), Some("loading"));
        view.set(Ok(vec![MigrateEntry { id: 2, title: "zeta".into(), cover: None }, MigrateEntry { id: 1, title: "Alpha".into(), cover: None }]));
        assert_eq!(view.widget().visible_child_name().as_deref(), Some("list"));
        let text = labels(view.widget().upcast_ref());
        let (alpha, zeta) = (text.iter().position(|t| t == "Alpha"), text.iter().position(|t| t == "zeta"));
        assert!(alpha.is_some() && zeta.is_some() && alpha < zeta, "rows sort by title without case: {text:?}");
        view.set(Ok(Vec::new()));
        assert_eq!(view.widget().visible_child_name().as_deref(), Some("empty"));
        // the status page also holds its empty description label
        let text: Vec<String> = labels(view.widget().visible_child().expect("child").upcast_ref()).into_iter().filter(|t| !t.is_empty()).collect();
        assert_eq!(text, [tr(MsgKey::empty_screen)], "E: Well, this is awkward");
        view.set(Err("db closed".into()));
        assert_eq!(view.widget().visible_child_name().as_deref(), Some("failed"));
        assert!(labels(view.widget().visible_child().expect("child").upcast_ref()).contains(&tr(MsgKey::internal_error)), "the error page says the internal error");
    }

    fn a_row_opens_the_migrate_search_with_its_title() {
        let (view, log) = view();
        view.set(Ok(vec![MigrateEntry { id: 7, title: "Seven".into(), cover: None }]));
        view.open_search(7);
        assert_eq!(*log.borrow(), ["search 7 Seven"]);
    }

    pub(crate) fn smoke() {
        the_view_is_a_spinner_then_the_rows_the_empty_screen_or_the_error_page();
        a_row_opens_the_migrate_search_with_its_title();
    }
}
