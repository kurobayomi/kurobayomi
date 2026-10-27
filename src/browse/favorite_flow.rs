//! Add to library and Remove from library on a browsed entry (its right-click menu): the decision, the dialog it asks
//! for and the library write that follows (T-40b-041, T-40b-042, T-40b-044, T-40b-047). The rules are `favorite` and
//! `entries`; this file shows the dialogs and runs the store calls off the GTK thread.

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use kuro_domain::category::model::Category;
use kuro_domain::category::target::{AddTarget, resolve_add_target};
use kuro_platform::kind::Kind;

use super::dialogs::{ButtonStyle, DialogSlot, DialogSpec};
use crate::duplicate_dialog::DuplicateChoice;
use super::entries::{CategoryChoice, FavoriteStore, add_entry, add_entry_to, chooser_rows, remove_entry};
use super::favorite::{
    AddOutcome, LongClick, OptimisticFavorite, long_click, on_remove_answer,
    remove_confirm,
};
use super::listing::ListItem;
use super::primitives::ToastSpec;

/// What the flow reports to the page that owns the listing.
pub struct FavoriteHooks {
    /// Shows the favourite flag of the entry with this library id.
    pub set_flag: Rc<dyn Fn(i64, bool)>,
    pub toast: Rc<dyn Fn(ToastSpec)>,
    /// The duplicate dialog's Show manga: opens the existing entry.
    pub open_entry: Rc<dyn Fn(i64)>,
    /// The duplicate dialog's Migrate existing entry: the duplicate's id (the old entry) and the browsed entry's id (the new one).
    pub migrate: Rc<dyn Fn(i64, i64)>,
    pub now_ms: Rc<dyn Fn() -> i64>,
    /// The chooser's Edit categories: opens the Categories page on its manga section.
    pub edit_categories: Rc<dyn Fn()>,
}

pub struct FavoriteFlow {
    store: Arc<dyn FavoriteStore>,
    dialogs: DialogSlot,
    hooks: FavoriteHooks,
    anchor: glib::WeakRef<gtk::Widget>,
    /// Counts long presses, so the answer of an older one never opens its dialog over a newer one (B-EC-27).
    presses: Cell<u64>,
    /// The page's kind: the duplicate dialog of an anime page says Show anime.
    kind: Kind,
}

/// Runs `work` on a worker thread and `then` with its result on the main loop.
fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, then: impl FnOnce(T) + 'static) {
    glib::spawn_future_local(async move {
        if let Ok(value) = gtk::gio::spawn_blocking(work).await {
            then(value);
        }
    });
}

impl FavoriteFlow {

    /// `kind` is the kind of the page.
    pub fn new_for(store: Arc<dyn FavoriteStore>, anchor: &impl IsA<gtk::Widget>, hooks: FavoriteHooks, kind: Kind) -> Rc<Self> {
        Rc::new(Self { store, dialogs: DialogSlot::new(), hooks, anchor: anchor.upcast_ref::<gtk::Widget>().downgrade(), presses: Cell::new(0), kind })
    }

    #[cfg(test)]
    /// The dialog on screen, if any.
    pub fn dialog(&self) -> Option<gtk::Window> {
        self.dialogs.current()
    }

    /// A cover menu's Add to library or Remove from library: a favourite asks to be removed, a new entry with a duplicate
    /// asks first, anything else is added (B§4.8.4). A second ask replaces the dialog the first one opened.
    pub fn long_press(self: &Rc<Self>, item: ListItem) {
        let press = self.presses.get() + 1;
        self.presses.set(press);
        let store = Arc::clone(&self.store);
        let (id, title, favorite) = (item.id, item.title.clone(), item.favorite);
        let this = Rc::clone(self);
        blocking(
            move || {
                if favorite {
                    return Vec::new();
                }
                store.duplicates(id, &title).unwrap_or_else(|error| {
                    tracing::warn!(%error, "could not look for duplicates");
                    Vec::new()
                })
            },
            move |duplicates| {
                if this.presses.get() != press {
                    return;
                }
                match long_click(item.favorite, !duplicates.is_empty()) {
                    LongClick::RemoveConfirm => this.confirm_remove(item),
                    LongClick::DuplicateDialog => this.ask_duplicate(item, duplicates[0]),
                    LongClick::Add => this.add(item),
                }
            },
        );
    }

    fn confirm_remove(self: &Rc<Self>, item: ListItem) {
        let text = remove_confirm(&item.title);
        let spec = DialogSpec {
            title: text.title,
            body: Some(text.body),
            buttons: vec![(text.confirm, ButtonStyle::Destructive), (text.cancel, ButtonStyle::Plain)],
        };
        let this = Rc::clone(self);
        self.show(spec, None, move |chosen| {
            if on_remove_answer(chosen == Some(0)).is_some() {
                this.remove(item.clone());
            }
        });
    }

    fn ask_duplicate(self: &Rc<Self>, item: ListItem, existing: i64) {
        let this = Rc::clone(self);
        let window = crate::duplicate_dialog::open(
            self.kind,
            move |choice| match choice {
                DuplicateChoice::Show => (this.hooks.open_entry)(existing),
                DuplicateChoice::Migrate => (this.hooks.migrate)(existing, item.id),
                DuplicateChoice::AddAnyway => this.add(item.clone()),
            },
            || {},
        );
        // a second ask replaces the dialog the first one opened
        self.dialogs.adopt(&window);
    }

    fn show(&self, spec: DialogSpec, extra: Option<gtk::Widget>, on_answer: impl Fn(Option<usize>) + 'static) {
        if let Some(anchor) = self.anchor.upgrade() {
            self.dialogs.show_with(&anchor, spec, extra, on_answer);
        }
    }

    /// The favourite flips at once; a failed write puts the previous flag back and shows `internal_error`.
    fn remove(self: &Rc<Self>, item: ListItem) {
        let mut shown = OptimisticFavorite::start(true);
        (self.hooks.set_flag)(item.id, shown.shown());
        let (store, id, now) = (Arc::clone(&self.store), item.id, (self.hooks.now_ms)());
        let this = Rc::clone(self);
        blocking(move || remove_entry(store.as_ref(), id, now), move |result| {
            if let Err(error) = result {
                tracing::warn!(%error, "removing from the library failed");
                this.restore(id, &mut shown);
            }
        });
    }

    fn restore(&self, id: i64, shown: &mut OptimisticFavorite) {
        let toast = shown.fail();
        (self.hooks.set_flag)(id, shown.shown());
        (self.hooks.toast)(toast);
    }

    /// Adds the entry. A default category that asks opens the chooser and writes nothing until it is answered.
    pub fn add(self: &Rc<Self>, item: ListItem) {
        let mut shown = OptimisticFavorite::start(false);
        (self.hooks.set_flag)(item.id, shown.shown());
        let (store, id, now) = (Arc::clone(&self.store), item.id, (self.hooks.now_ms)());
        let this = Rc::clone(self);
        blocking(
            move || {
                let outcome = add_entry(store.as_ref(), id, now);
                let (categories, linked) = if outcome == AddOutcome::Chooser {
                    (store.categories().unwrap_or_default(), store.entry_categories(id).unwrap_or_default())
                } else {
                    (Vec::new(), Vec::new())
                };
                (outcome, categories, linked)
            },
            move |(outcome, categories, linked)| match outcome {
                AddOutcome::Added { .. } => {}
                AddOutcome::Failed(error) => {
                    tracing::warn!(%error, "adding to the library failed");
                    this.restore(id, &mut shown);
                }
                AddOutcome::Chooser => {
                    (this.hooks.set_flag)(id, false);
                    this.choose_categories(item, categories, linked);
                }
            },
        );
    }

    /// Bulk Add to library of a selection (issue #7): each item not in the library yet is added by the rules of one add.
    /// The default category decides once for all of them; when it asks, the chooser opens once and its answer goes to
    /// every item. Items with a duplicate then ask one after another with the duplicate dialog: Cancel skips that item,
    /// Show or Migrate leaves the page and asks no more.
    pub fn add_many(self: &Rc<Self>, items: Vec<ListItem>) {
        let items: Vec<ListItem> = items.into_iter().filter(|item| !item.favorite).collect();
        if items.is_empty() {
            return;
        }
        let press = self.presses.get() + 1;
        self.presses.set(press);
        let store = Arc::clone(&self.store);
        let this = Rc::clone(self);
        blocking(
            move || {
                let all = store.categories().unwrap_or_default();
                let ids: Vec<i64> = all.iter().map(|c| c.id).collect();
                let asks = resolve_add_target(store.default_category(), &ids) == AddTarget::Ask;
                let split: Vec<(ListItem, Option<i64>)> = items
                    .into_iter()
                    .map(|item| {
                        let duplicates = store.duplicates(item.id, &item.title).unwrap_or_else(|error| {
                            tracing::warn!(%error, "could not look for duplicates");
                            Vec::new()
                        });
                        let duplicate = duplicates.first().copied();
                        (item, duplicate)
                    })
                    .collect();
                (asks.then_some(all), split)
            },
            move |(ask, split)| {
                if this.presses.get() != press {
                    return;
                }
                let Some(categories) = ask else {
                    this.add_split(split, None, press);
                    return;
                };
                let rows = chooser_rows(&categories, &[]).into_iter().map(|(c, on)| (c.id, c.name, on)).collect();
                let (ok, edit) = (Rc::clone(&this), Rc::clone(&this));
                let window = crate::category_picker::open(
                    crate::entry_core::dialog::CategoryDialog::from_rows(rows),
                    move |d| ok.add_split(split.clone(), Some(d.checked_ids()), press),
                    move || (edit.hooks.edit_categories)(),
                    || {},
                );
                this.dialogs.adopt(&window);
            },
        );
    }

    /// Adds the items without a duplicate at once, then asks about the others in turn.
    fn add_split(self: &Rc<Self>, split: Vec<(ListItem, Option<i64>)>, categories: Option<Vec<i64>>, press: u64) {
        let mut asks = VecDeque::new();
        for (item, duplicate) in split {
            match duplicate {
                Some(existing) => asks.push_back((item, existing)),
                None => self.add_one(item, categories.clone()),
            }
        }
        self.ask_duplicates(asks, categories, press);
    }

    /// One item of a bulk add: to the chosen categories, or by the default category as one add does.
    fn add_one(self: &Rc<Self>, item: ListItem, categories: Option<Vec<i64>>) {
        match categories {
            Some(categories) => self.add_to(item, categories),
            None => self.add(item),
        }
    }

    /// The duplicate dialog of the first item of `asks`; once it closed, the next one, unless the choice left the page or
    /// a newer ask took over.
    fn ask_duplicates(self: &Rc<Self>, mut asks: VecDeque<(ListItem, i64)>, categories: Option<Vec<i64>>, press: u64) {
        let Some((item, existing)) = asks.pop_front() else { return };
        let left = Rc::new(Cell::new(false));
        let (this, chosen_left, chosen) = (Rc::clone(self), Rc::clone(&left), categories.clone());
        let next = Rc::clone(self);
        let window = crate::duplicate_dialog::open(
            self.kind,
            move |choice| match choice {
                DuplicateChoice::Show => {
                    chosen_left.set(true);
                    (this.hooks.open_entry)(existing);
                }
                DuplicateChoice::Migrate => {
                    chosen_left.set(true);
                    (this.hooks.migrate)(existing, item.id);
                }
                DuplicateChoice::AddAnyway => this.add_one(item.clone(), chosen.clone()),
            },
            // the choice runs on idle before this idle, so a Show or Migrate is known by then
            move || {
                let (next, asks, categories, left) = (Rc::clone(&next), asks.clone(), categories.clone(), Rc::clone(&left));
                glib::idle_add_local_once(move || {
                    if !left.get() && next.presses.get() == press {
                        next.ask_duplicates(asks, categories, press);
                    }
                });
            },
        );
        self.dialogs.adopt(&window);
    }

    /// Bulk Set categories of a selection's entries that are in the library (issue #7): the Library's change-categories
    /// dialog, where a category only some of them have shows mixed, and each entry keeps the links the answer leaves
    /// untouched.
    pub fn set_categories_many(self: &Rc<Self>, items: Vec<ListItem>) {
        let ids: Vec<i64> = items.iter().filter(|item| item.favorite).map(|item| item.id).collect();
        if ids.is_empty() {
            return;
        }
        let store = Arc::clone(&self.store);
        let this = Rc::clone(self);
        blocking(
            move || {
                let categories = store.categories().unwrap_or_default();
                let links: Vec<Vec<i64>> = ids.iter().map(|id| store.entry_categories(*id).unwrap_or_default()).collect();
                (ids, categories, links)
            },
            move |(ids, categories, links)| {
                let categories: Vec<Category> = categories
                    .into_iter()
                    .enumerate()
                    .map(|(order, c)| Category { id: c.id, name: c.name, order: order as i64, flags: 0, hidden: false })
                    .collect();
                let dialog = crate::library::category_dialog::CategoryDialog::new(ids.clone(), &categories, &links);
                let (ok, edit) = (Rc::clone(&this), Rc::clone(&this));
                let window = crate::category_picker::open(
                    dialog,
                    move |d| {
                        let (include, exclude) = d.confirm();
                        ok.write_categories(ids.clone(), include, exclude);
                    },
                    move || (edit.hooks.edit_categories)(),
                    || {},
                );
                this.dialogs.adopt(&window);
            },
        );
    }

    /// Applies a change-categories answer to each entry: its current links, plus `include`, less `exclude`.
    fn write_categories(&self, ids: Vec<i64>, include: Vec<i64>, exclude: Vec<i64>) {
        let store = Arc::clone(&self.store);
        blocking(
            move || {
                for id in ids {
                    let links = match store.entry_categories(id) {
                        Ok(links) => links,
                        Err(error) => {
                            tracing::warn!(entry = id, %error, "reading the categories of an entry failed");
                            continue;
                        }
                    };
                    let next = crate::library::actions::next_membership(&links, &include, &exclude);
                    if let Err(error) = store.set_categories(id, &next) {
                        tracing::warn!(entry = id, %error, "saving the categories of an entry failed");
                    }
                }
            },
            |()| {},
        );
    }

    /// The category chooser: the user's categories with the entry's current links checked, OK, Cancel and Edit
    /// categories (which closes the chooser and opens the Categories page; nothing is written).
    fn choose_categories(self: &Rc<Self>, item: ListItem, categories: Vec<CategoryChoice>, linked: Vec<i64>) {
        let rows = chooser_rows(&categories, &linked).into_iter().map(|(c, on)| (c.id, c.name, on)).collect();
        let (this, edit) = (Rc::clone(self), Rc::clone(self));
        let window = crate::category_picker::open(
            crate::entry_core::dialog::CategoryDialog::from_rows(rows),
            move |d| this.add_to(item.clone(), d.checked_ids()),
            move || (edit.hooks.edit_categories)(),
            || {},
        );
        self.dialogs.adopt(&window);
    }

    fn add_to(self: &Rc<Self>, item: ListItem, categories: Vec<i64>) {
        let mut shown = OptimisticFavorite::start(false);
        (self.hooks.set_flag)(item.id, shown.shown());
        let (store, id, now) = (Arc::clone(&self.store), item.id, (self.hooks.now_ms)());
        let this = Rc::clone(self);
        blocking(move || add_entry_to(store.as_ref(), id, now, categories), move |outcome| {
            if let AddOutcome::Failed(error) = outcome {
                tracing::warn!(%error, "adding to the library failed");
                this.restore(id, &mut shown);
            }
        });
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use kuro_i18n::{MsgKey, tr};
    use std::cell::RefCell;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::browse::dialogs::buttons_of;
    use crate::browse::entries::fake::FakeStore;

    struct Rig {
        flow: Rc<FavoriteFlow>,
        store: Arc<FakeStore>,
        flags: Rc<RefCell<Vec<(i64, bool)>>>,
        toasts: Rc<RefCell<Vec<String>>>,
        opened: Rc<RefCell<Vec<String>>>,
        _anchor: gtk::Label,
    }

    fn rig() -> Rig {
        rig_for(Kind::Manga)
    }

    fn rig_for(kind: Kind) -> Rig {
        let store = Arc::new(FakeStore::default());
        let anchor = gtk::Label::new(None);
        let (flags, toasts, opened): (Rc<RefCell<Vec<(i64, bool)>>>, Rc<RefCell<Vec<String>>>, Rc<RefCell<Vec<String>>>) =
            Default::default();
        let (f, t, o1, o2, o3) = (Rc::clone(&flags), Rc::clone(&toasts), Rc::clone(&opened), Rc::clone(&opened), Rc::clone(&opened));
        let hooks = FavoriteHooks {
            set_flag: Rc::new(move |id, flag| f.borrow_mut().push((id, flag))),
            toast: Rc::new(move |spec| t.borrow_mut().push(spec.message)),
            open_entry: Rc::new(move |id| o1.borrow_mut().push(format!("open {id}"))),
            migrate: Rc::new(move |old, new| o2.borrow_mut().push(format!("migrate {old} to {new}"))),
            now_ms: Rc::new(|| 5_000),
            edit_categories: Rc::new(move || o3.borrow_mut().push("categories".to_owned())),
        };
        let flow = FavoriteFlow::new_for(Arc::clone(&store) as Arc<dyn FavoriteStore>, &anchor, hooks, kind);
        Rig { flow, store, flags, toasts, opened, _anchor: anchor }
    }

    fn item(favorite: bool) -> ListItem {
        ListItem { id: 7, source_id: 5, url: "/series/1".into(), title: "Berserk".into(), favorite, cover: None }
    }

    fn pump(mut done: impl FnMut() -> bool) -> bool {
        let context = glib::MainContext::default();
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            while context.iteration(false) {}
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        done()
    }

    fn a_favourite_asks_to_remove_and_confirm_runs_the_remove_write() {
        let rig = rig();
        *rig.store.covers.lock().expect("lock") = 1;
        rig.flow.long_press(item(true));
        assert!(pump(|| rig.flow.dialog().is_some()), "the remove confirm opens");
        let window = rig.flow.dialog().expect("dialog");
        let confirm = remove_confirm("Berserk");
        assert_eq!(window.title().map(|t| t.to_string()), Some(confirm.title));
        let labels: Vec<String> = buttons_of(&window).iter().filter_map(|b| b.label().map(|l| l.to_string())).collect();
        assert_eq!(labels, [confirm.confirm, confirm.cancel]);
        assert!(rig.store.calls().iter().all(|c| !c.starts_with("favorite")), "nothing is written before the answer");
        buttons_of(&window)[1].emit_clicked();
        assert!(rig.store.calls().iter().all(|c| !c.starts_with("favorite")), "Cancel changes nothing");

        rig.flow.long_press(item(true));
        assert!(pump(|| rig.flow.dialog().is_some()));
        buttons_of(&rig.flow.dialog().expect("dialog"))[0].emit_clicked();
        assert!(pump(|| rig.store.calls().contains(&"stamp 7 5000".to_owned())));
        assert_eq!(rig.flags.borrow().last(), Some(&(7, false)), "the flag flips at once");
        assert_eq!(rig.store.calls()[..2], ["favorite 7 false 0", "clear_cover 7"]);
    }

    fn a_new_entry_with_a_duplicate_shows_the_duplicate_dialog_and_add_anyway_adds() {
        let rig = rig();
        *rig.store.duplicates.lock().expect("lock") = vec![42];
        rig.flow.long_press(item(false));
        assert!(pump(|| rig.flow.dialog().is_some()));
        let window = rig.flow.dialog().expect("dialog");
        // Cancel, Show, Migrate existing entry, Add anyway
        assert_eq!(buttons_of(&window).len(), 4);
        buttons_of(&window)[1].emit_clicked();
        assert!(pump(|| *rig.opened.borrow() == ["open 42"]));

        rig.flow.long_press(item(false));
        assert!(pump(|| rig.flow.dialog().is_some()));
        buttons_of(&rig.flow.dialog().expect("dialog"))[2].emit_clicked();
        assert!(pump(|| *rig.opened.borrow() == ["open 42", "migrate 42 to 7"]), "T-40b-046: old = the duplicate, new = the browsed entry");

        rig.flow.long_press(item(false));
        assert!(pump(|| rig.flow.dialog().is_some()));
        buttons_of(&rig.flow.dialog().expect("dialog"))[3].emit_clicked();
        assert!(pump(|| rig.store.calls().contains(&"favorite 7 true 5000".to_owned())), "Add anyway adds");
        assert_eq!(rig.flags.borrow().first(), Some(&(7, true)));
    }

    /// T-41b-021: the anime page's duplicate dialog offers Cancel, Show anime, Migrate existing entry and Add anyway.
    fn the_anime_duplicate_dialog_says_show_anime_and_each_row_runs_its_action() {
        let rig = rig_for(Kind::Anime);
        *rig.store.duplicates.lock().expect("lock") = vec![42];
        rig.flow.long_press(item(false));
        assert!(pump(|| rig.flow.dialog().is_some()));
        let window = rig.flow.dialog().expect("dialog");
        let labels: Vec<String> = buttons_of(&window).iter().filter_map(|b| b.label().map(|l| l.to_string())).collect();
        assert_eq!(labels[1], tr(MsgKey::action_show_anime));
        buttons_of(&window)[1].emit_clicked();
        assert!(pump(|| *rig.opened.borrow() == ["open 42"]));
    }

    fn a_second_long_press_replaces_the_open_dialog() {
        let rig = rig();
        *rig.store.duplicates.lock().expect("lock") = vec![42];
        rig.flow.long_press(item(false));
        assert!(pump(|| rig.flow.dialog().is_some()));
        let first = rig.flow.dialog().expect("dialog");
        let mut second = item(true);
        second.title = "Other".into();
        rig.flow.long_press(second);
        assert!(pump(|| rig.flow.dialog().is_some_and(|w| w != first)));
        assert_eq!(rig.flow.dialog().and_then(|w| w.title()).map(|t| t.to_string()), Some(remove_confirm("Other").title));
        rig.flow.dialog().expect("dialog").close();
    }

    fn a_plain_entry_is_added_and_a_failed_write_restores_the_flag_with_the_error_toast() {
        let rig = rig();
        rig.flow.long_press(item(false));
        assert!(pump(|| rig.store.calls().contains(&"favorite 7 true 5000".to_owned())));
        assert!(rig.toasts.borrow().is_empty());

        rig.store.fail.lock().expect("lock").push("write_favorite");
        rig.flags.borrow_mut().clear();
        rig.flow.long_press(item(false));
        assert!(pump(|| !rig.toasts.borrow().is_empty()));
        assert_eq!(*rig.flags.borrow(), [(7, true), (7, false)], "flipped at once, then put back");
        assert_eq!(rig.toasts.borrow()[0], tr(MsgKey::internal_error));
    }

    fn an_ask_default_category_opens_the_chooser_and_writes_nothing_until_it_is_answered() {
        let rig = rig();
        *rig.store.default_category.lock().expect("lock") = -1;
        *rig.store.categories.lock().expect("lock") =
            vec![CategoryChoice { id: 3, name: "Reading".into() }, CategoryChoice { id: 4, name: "Later".into() }];
        *rig.store.links.lock().expect("lock") = vec![3];
        rig.flow.long_press(item(false));
        assert!(pump(|| rig.flow.dialog().is_some()), "the chooser opens");
        assert!(rig.store.calls().iter().all(|c| !c.starts_with("favorite")), "nothing is written yet");
        assert_eq!(rig.flags.borrow().last(), Some(&(7, false)), "the optimistic flag is taken back while the chooser is open");
        let window = rig.flow.dialog().expect("dialog");
        let checks: Vec<gtk::CheckButton> = {
            fn walk(w: &gtk::Widget, out: &mut Vec<gtk::CheckButton>) {
                if let Some(c) = w.downcast_ref::<gtk::CheckButton>() {
                    out.push(c.clone());
                }
                let mut child = w.first_child();
                while let Some(c) = child {
                    walk(&c, out);
                    child = c.next_sibling();
                }
            }
            let mut out = Vec::new();
            walk(&window.child().expect("child"), &mut out);
            out
        };
        assert_eq!(checks.len(), 2);
        assert_eq!(checks.iter().map(|c| c.is_active()).collect::<Vec<_>>(), [true, false], "the linked category 3 is preselected");
        assert_eq!(buttons_of(&window).len(), 3, "Edit categories, Cancel and OK");
        checks[0].set_active(false);
        checks[1].set_active(true);
        buttons_of(&window)[2].emit_clicked();
        assert!(pump(|| rig.store.calls().contains(&"favorite 7 true 5000".to_owned())));
        let calls = rig.store.calls();
        let categories = calls.iter().position(|c| c == "categories 7 [4]").expect("categories written");
        let favorite = calls.iter().position(|c| c == "favorite 7 true 5000").expect("favorite written");
        assert!(categories < favorite, "the category links are written before the favourite");
    }

    fn edit_categories_opens_the_categories_page_and_writes_nothing() {
        let rig = rig();
        *rig.store.default_category.lock().expect("lock") = -1;
        *rig.store.categories.lock().expect("lock") = vec![CategoryChoice { id: 3, name: "Reading".into() }];
        rig.flow.long_press(item(false));
        assert!(pump(|| rig.flow.dialog().is_some()), "the chooser opens");
        buttons_of(&rig.flow.dialog().expect("dialog"))[0].emit_clicked();
        assert!(pump(|| *rig.opened.borrow() == ["categories"]), "the categories page was asked for");
        assert!(rig.store.calls().iter().all(|c| !c.starts_with("favorite")), "nothing is added");
    }

    fn titled(id: i64, favorite: bool) -> ListItem {
        ListItem { id, source_id: 5, url: format!("/series/{id}"), title: format!("T{id}"), favorite, cover: None }
    }

    fn checks_of(window: &gtk::Window) -> Vec<gtk::CheckButton> {
        fn walk(w: &gtk::Widget, out: &mut Vec<gtk::CheckButton>) {
            if let Some(c) = w.downcast_ref::<gtk::CheckButton>() {
                out.push(c.clone());
            }
            let mut child = w.first_child();
            while let Some(c) = child {
                walk(&c, out);
                child = c.next_sibling();
            }
        }
        let mut out = Vec::new();
        walk(&window.child().expect("child"), &mut out);
        out
    }

    /// Issue #7: a bulk add adds every title that is not in the library by the default category, and asks nothing.
    fn a_bulk_add_adds_each_new_title_and_leaves_library_titles_alone() {
        let rig = rig();
        rig.flow.add_many(vec![titled(1, false), titled(2, true), titled(3, false)]);
        assert!(pump(|| ["favorite 1 true 5000", "favorite 3 true 5000"].iter().all(|c| rig.store.calls().contains(&(*c).to_owned()))));
        assert!(rig.store.calls().iter().all(|c| !c.starts_with("favorite 2")), "a title already in the library is left alone");
        assert!(rig.flow.dialog().is_none(), "the default category asks nothing");
        let mut added = rig.store.added.lock().expect("lock").clone();
        added.sort_unstable();
        assert_eq!(added, [1, 3], "each add runs the defaults and trackers step, as one add does");
    }

    /// Issue #7: with "always ask", the chooser opens once and its answer goes to every title.
    fn a_bulk_add_that_asks_opens_the_chooser_once_for_every_title() {
        let rig = rig();
        *rig.store.default_category.lock().expect("lock") = -1;
        *rig.store.categories.lock().expect("lock") =
            vec![CategoryChoice { id: 3, name: "Reading".into() }, CategoryChoice { id: 4, name: "Later".into() }];
        rig.flow.add_many(vec![titled(1, false), titled(3, false)]);
        assert!(pump(|| rig.flow.dialog().is_some()), "one chooser");
        assert!(rig.store.calls().iter().all(|c| !c.starts_with("favorite")), "nothing is written before the answer");
        let window = rig.flow.dialog().expect("dialog");
        let checks = checks_of(&window);
        assert_eq!(checks.iter().map(|c| c.is_active()).collect::<Vec<_>>(), [false, false]);
        checks[1].set_active(true);
        buttons_of(&window)[2].emit_clicked();
        assert!(pump(|| ["favorite 1 true 5000", "favorite 3 true 5000"].iter().all(|c| rig.store.calls().contains(&(*c).to_owned()))));
        let calls = rig.store.calls();
        assert!(calls.contains(&"categories 1 [4]".to_owned()) && calls.contains(&"categories 3 [4]".to_owned()), "{calls:?}");
    }

    /// Issue #7: titles with a duplicate ask one after another; Add anyway adds, Cancel skips only that title.
    fn a_bulk_add_asks_about_each_duplicate_in_turn() {
        let rig = rig();
        *rig.store.duplicates.lock().expect("lock") = vec![42];
        rig.flow.add_many(vec![titled(1, false), titled(3, false)]);
        assert!(pump(|| rig.flow.dialog().is_some()), "the first duplicate asks");
        let first = rig.flow.dialog().expect("dialog");
        buttons_of(&first)[3].emit_clicked();
        assert!(pump(|| rig.store.calls().contains(&"favorite 1 true 5000".to_owned())), "Add anyway adds the first");
        assert!(pump(|| rig.flow.dialog().is_some_and(|w| w != first)), "then the second asks");
        buttons_of(&rig.flow.dialog().expect("dialog"))[0].emit_clicked();
        let _ = pump(|| false);
        assert!(rig.store.calls().iter().all(|c| !c.starts_with("favorite 3")), "Cancel skips that title");
    }

    /// Issue #7: Set categories of library titles opens the Library's dialog over all of them; each keeps its other links.
    fn a_bulk_set_categories_adds_the_picked_category_to_every_library_title() {
        let rig = rig();
        *rig.store.categories.lock().expect("lock") =
            vec![CategoryChoice { id: 3, name: "Reading".into() }, CategoryChoice { id: 4, name: "Later".into() }];
        *rig.store.links.lock().expect("lock") = vec![3];
        rig.flow.set_categories_many(vec![titled(1, true), titled(2, true), titled(5, false)]);
        assert!(pump(|| rig.flow.dialog().is_some()));
        let window = rig.flow.dialog().expect("dialog");
        let checks = checks_of(&window);
        assert_eq!(checks.iter().map(|c| c.is_active()).collect::<Vec<_>>(), [true, false], "both titles are in Reading");
        checks[1].set_active(true);
        buttons_of(&window)[2].emit_clicked();
        let written = |id: i64| rig.store.calls().iter().any(|c| c.starts_with(&format!("categories {id} ")) && c.contains('3') && c.contains('4'));
        assert!(pump(|| written(1) && written(2)), "{:?}", rig.store.calls());
        assert!(rig.store.calls().iter().all(|c| !c.starts_with("categories 5 ")), "a title outside the library is not touched");
    }

    pub(crate) fn smoke() {
        a_favourite_asks_to_remove_and_confirm_runs_the_remove_write();
        a_bulk_add_adds_each_new_title_and_leaves_library_titles_alone();
        a_bulk_add_that_asks_opens_the_chooser_once_for_every_title();
        a_bulk_add_asks_about_each_duplicate_in_turn();
        a_bulk_set_categories_adds_the_picked_category_to_every_library_title();
        a_new_entry_with_a_duplicate_shows_the_duplicate_dialog_and_add_anyway_adds();
        the_anime_duplicate_dialog_says_show_anime_and_each_row_runs_its_action();
        a_second_long_press_replaces_the_open_dialog();
        a_plain_entry_is_added_and_a_failed_write_restores_the_flag_with_the_error_toast();
        an_ask_default_category_opens_the_chooser_and_writes_nothing_until_it_is_answered();
        edit_categories_opens_the_categories_page_and_writes_nothing();
    }
}
