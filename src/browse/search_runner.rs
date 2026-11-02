//! The fetch runner of global search and migrate search (T-40b-060, T-40b-064). `GlobalSearch` decides what to fetch; this
//! runs it: at most `MAX_IN_FLIGHT` source fetches at once, the fetches of an older search aborted when a new one starts,
//! results of an older generation dropped, and every result applied by one writer on the GTK thread. The single-result
//! auto-open runs after each applied result. Needs the tokio runtime entered, as the app keeps it, and a GLib main loop.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use kuro_domain::source::browse::BrowseQuery;
use kuro_domain::source::model::Source;
use kuro_i18n::{Arg, MsgKey, trf};
use kuro_source::filter_list::FilterList;

use super::global_search::{AutoOpen, GlobalSearch, SearchPlan, auto_open, shared_gate, take_starts};
use super::glue::{consume_events, keep_alive, own_tasks};
use super::deep_link::is_link;
use super::errors::{BACKSTOP, couldnt_load, settle};
use super::listing::{LinkFuture, ListItem, PageLoader};
use super::pages::{GlobalSearchSnapshot, SourceFilter};
use super::primitives::stub_source_label;
use super::tasks::{PageTasks, Tagged};
use crate::background::run_async_cancellable;
use tokio_util::sync::CancellationToken;

/// The loader of a source's listings by id, `None` when the source is not installed.
pub type Loaders = Arc<dyn Fn(i64) -> Option<Arc<dyn PageLoader>> + Send + Sync>;

/// A source's fresh default filter list by id (`SourceDirectory::filters`); an unknown source searches with none.
pub type DefaultFilters = Arc<dyn Fn(i64) -> Option<FilterList> + Send + Sync>;

/// Resolves a pasted link to the entry it names, or `None` when nothing can resolve it (`SourceDirectory::resolve_link`).
pub type LinkResolver = Arc<dyn Fn(&str) -> Option<LinkFuture>>;

/// What the runner reports to the page.
pub struct RunnerHooks {
    /// The rows, the progress or the query changed: redraw from the snapshot.
    pub changed: Rc<dyn Fn(&GlobalSearchSnapshot)>,
    /// The single-result auto-open: replace the search with this manga's page (library id).
    pub open_entry: Rc<dyn Fn(i64)>,
}

/// One source's answer, tagged with the plan (the engine's generation) it was started for.
struct Fetched {
    plan: u64,
    source_id: i64,
    result: Result<Vec<ListItem>, String>,
}

pub struct SearchRunner {
    engine: RefCell<GlobalSearch>,
    loaders: Loaders,
    /// Where each fan-out fetch gets its filters (`source.getFilterList()`); empty lists when unset.
    filters: RefCell<Option<DefaultFilters>>,
    hooks: RunnerHooks,
    tasks: Rc<RefCell<PageTasks<Fetched>>>,
    queue: RefCell<VecDeque<Source>>,
    running: Cell<usize>,
    plan: Cell<u64>,
    query: RefCell<String>,
    /// The extension package of a search started by an intent; only such a search can auto-open (GS-EC-13).
    extension: RefCell<Option<String>>,
    opened: Cell<bool>,
    /// Resolves a pasted link before any catalogue search (T-20a-031).
    links: RefCell<Option<LinkResolver>>,
    /// Ends a link lookup that a newer search or the page has outlived.
    link_lookup: RefCell<CancellationToken>,
    /// The fetch permits: shared by every search screen in the app (E-GS-08).
    gate: Arc<tokio::sync::Semaphore>,
}

impl Drop for SearchRunner {
    fn drop(&mut self) {
        self.link_lookup.borrow().cancel();
    }
}

impl SearchRunner {
    pub fn new(engine: GlobalSearch, loaders: Loaders, hooks: RunnerHooks) -> Rc<Self> {
        Self::with_gate(engine, loaders, hooks, shared_gate())
    }

    /// A runner whose fetches take their permits from `gate`. `new` passes the app's shared gate.
    pub fn with_gate(engine: GlobalSearch, loaders: Loaders, hooks: RunnerHooks, gate: Arc<tokio::sync::Semaphore>) -> Rc<Self> {
        let runner = Rc::new(Self {
            engine: RefCell::new(engine),
            loaders,
            filters: RefCell::new(None),
            hooks,
            tasks: Rc::new(RefCell::new(PageTasks::new())),
            queue: RefCell::new(VecDeque::new()),
            running: Cell::new(0),
            plan: Cell::new(0),
            query: RefCell::new(String::new()),
            extension: RefCell::new(None),
            opened: Cell::new(false),
            links: RefCell::new(None),
            link_lookup: RefCell::new(CancellationToken::new()),
            gate,
        });
        let weak = Rc::downgrade(&runner);
        consume_events(&runner.tasks, move |fetched| {
            if let Some(runner) = weak.upgrade() {
                runner.arrived(fetched);
            }
        });
        runner
    }

    /// Ties the runner to the page: it lives as long as the widget, and popping the page aborts every fetch.
    pub fn attach(self: &Rc<Self>, widget: &impl IsA<gtk::Widget>) {
        own_tasks(widget, &self.tasks);
        keep_alive(widget, self);
    }

    /// Makes each source's fetch start from its own default filters instead of an empty list.
    pub fn set_default_filters(&self, filters: DefaultFilters) {
        *self.filters.borrow_mut() = Some(filters);
    }

    /// Makes a pasted link resolve through `resolver` before it is searched as text.
    pub fn set_link_resolver(&self, resolver: LinkResolver) {
        *self.links.borrow_mut() = Some(resolver);
    }

    /// Offers `text` to the link resolver when it is a link and a resolver has a source for it. The entry then opens and
    /// no catalogue is searched; a link that resolves to nothing is searched as text, like any query. `true` while the
    /// lookup runs, so the caller does not search yet.
    fn resolve_first(self: &Rc<Self>, text: &str, then: impl FnOnce(&Rc<Self>) + 'static) -> bool {
        self.link_lookup.borrow().cancel();
        let lookup = if is_link(text) { self.links.borrow().as_ref().and_then(|resolve| resolve(text)) } else { None };
        let Some(lookup) = lookup else { return false };
        let token = CancellationToken::new();
        *self.link_lookup.borrow_mut() = token.clone();
        let weak = Rc::downgrade(self);
        run_async_cancellable(token, lookup, move |item| {
            let Some(runner) = weak.upgrade() else { return };
            match item {
                Some(item) => {
                    if !runner.opened.replace(true) {
                        (runner.hooks.open_entry)(item.id);
                    }
                }
                None => then(&runner),
            }
        });
        true
    }

    /// Enter in the search box: a link opens its entry when a source resolves it, any other query searches, a blank or
    /// unchanged one does nothing.
    pub fn submit(self: &Rc<Self>, text: &str) {
        let owned = text.to_owned();
        if !self.resolve_first(text, move |runner| runner.search(&owned)) {
            self.search(text);
        }
    }

    fn search(self: &Rc<Self>, text: &str) {
        *self.query.borrow_mut() = text.to_owned();
        let plan = {
            let mut engine = self.engine.borrow_mut();
            engine.set_query(Some(text));
            engine.search()
        };
        self.begin(plan);
    }

    /// The Pinned and All chips: the same query searches again, fetching only the sources the chip adds.
    pub fn set_filter(self: &Rc<Self>, filter: SourceFilter) {
        let plan = self.engine.borrow_mut().set_filter(filter);
        self.begin(plan);
    }

    /// The Has results toggle: it filters the rows already here and fetches nothing.
    pub fn set_only_has_results(self: &Rc<Self>, only: bool) {
        self.engine.borrow_mut().set_only_has_results(only);
        self.changed();
    }

    /// A search with an extension filter (the SEARCH intent): forces All, and the single-result auto-open applies.
    pub fn request(self: &Rc<Self>, query: &str, extension: Option<&str>) {
        let (owned, extension_owned) = (query.to_owned(), extension.map(str::to_owned));
        if !self.resolve_first(query, move |runner| runner.search_extension(&owned, extension_owned.as_deref())) {
            self.search_extension(query, extension);
        }
    }

    fn search_extension(self: &Rc<Self>, query: &str, extension: Option<&str>) {
        *self.query.borrow_mut() = query.to_owned();
        *self.extension.borrow_mut() = extension.map(str::to_owned);
        let plan = self.engine.borrow_mut().request(query, extension);
        self.begin(plan);
    }

    /// Redraws from the current state: the page's first draw, and the toggle.
    pub fn refresh(&self) {
        self.changed();
    }

    #[cfg(test)]
    /// The fetches running now.
    pub fn in_flight(&self) -> usize {
        self.running.get()
    }

    #[cfg(test)]
    /// The sources still waiting for a free slot.
    pub fn queued(&self) -> usize {
        self.queue.borrow().len()
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> GlobalSearchSnapshot {
        self.engine.borrow().snapshot()
    }

    /// A plan replaces the search that was running: its fetches are aborted, the queue starts over, and the results of the
    /// old plan that are already on their way are dropped by generation.
    fn begin(self: &Rc<Self>, plan: Option<SearchPlan>) {
        let Some(plan) = plan else { return };
        self.tasks.borrow_mut().restart();
        *self.queue.borrow_mut() = plan.sources.iter().cloned().collect();
        self.running.set(0);
        self.plan.set(plan.generation);
        // the saved results of the same search show while each source is asked again
        let query = BrowseQuery::Search(self.query.borrow().clone());
        for source in &plan.sources {
            if let Some(saved) = super::results_cache::read(source.id, &query, &FilterList::default()) {
                self.engine.borrow_mut().prefill(plan.generation, source.id, saved.items);
            }
        }
        self.pump();
        self.changed();
    }

    /// Starts the queued fetches that fit under the cap.
    fn pump(&self) {
        let starts = take_starts(&mut self.queue.borrow_mut(), self.running.get());
        for source in starts {
            self.running.set(self.running.get() + 1);
            let (generation, tx) = self.tasks.borrow().sender();
            let (plan, id) = (self.plan.get(), source.id);
            let query = self.query.borrow().clone();
            let loader = (self.loaders)(id);
            let filters = self.filters.borrow().as_ref().and_then(|defaults| defaults(id)).unwrap_or_default();
            let missing = trf(MsgKey::source_not_installed, &[("arg1", Arg::Str(&stub_source_label(&source.name, &source.lang, id)))]);
            let (gate, name) = (Arc::clone(&self.gate), source.name.clone());
            self.tasks.borrow_mut().spawn(async move {
                let result = {
                    let _permit = gate.acquire_owned().await;
                    match loader {
                        // one source that fails, hangs or panics is one line in its own section; the others go on
                        Some(loader) => settle(BACKSTOP, &name, loader.load(BrowseQuery::Search(query), filters, 1))
                            .await
                            .map(|page| page.items)
                            .map_err(|text| {
                                tracing::warn!(source = name.as_str(), error = text.as_str(), "a source did not answer the global search");
                                couldnt_load(&name)
                            }),
                        None => Err(missing),
                    }
                };
                let _ = tx.send(Tagged { generation, event: Fetched { plan, source_id: id, result } }).await;
            });
        }
    }

    /// The one writer: a result of the live plan is applied on the GTK thread, then the next queued fetch starts.
    fn arrived(self: &Rc<Self>, fetched: Fetched) {
        self.running.set(self.running.get().saturating_sub(1));
        if let (Ok(items), true) = (&fetched.result, fetched.plan == self.plan.get()) {
            super::results_cache::write(fetched.source_id, &BrowseQuery::Search(self.query.borrow().clone()), &FilterList::default(), items);
        }
        let applied = self.engine.borrow_mut().apply(fetched.plan, fetched.source_id, fetched.result);
        self.pump();
        if applied {
            self.changed();
        }
    }

    fn changed(&self) {
        let snapshot = self.engine.borrow().snapshot();
        (self.hooks.changed)(&snapshot);
        let decision = {
            let engine = self.engine.borrow();
            let extension = self.extension.borrow();
            auto_open(&snapshot.query, extension.as_deref().unwrap_or(""), engine.rows().len(), engine.rows())
        };
        if let AutoOpen::Open(id) = decision {
            if !self.opened.replace(true) {
                (self.hooks.open_entry)(id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use gtk::glib;

    use super::*;
    use crate::browse::global_search::MAX_IN_FLIGHT;
    use crate::browse::listing::{ListedPage, LoadFuture};
    use crate::browse::pages::SearchState;

    /// Counts the loads in flight and the highest count seen; each load waits for a permit from `gate`.
    struct Gated {
        live: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        started: Arc<AtomicUsize>,
        gate: Arc<tokio::sync::Semaphore>,
        queries: Arc<std::sync::Mutex<Vec<String>>>,
    }

    struct Guard(Arc<AtomicUsize>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl PageLoader for Gated {
        fn load(&self, query: BrowseQuery, _filters: FilterList, _page: u32) -> LoadFuture {
            let (live, peak, started, gate, queries) =
                (Arc::clone(&self.live), Arc::clone(&self.peak), Arc::clone(&self.started), Arc::clone(&self.gate), Arc::clone(&self.queries));
            Box::pin(async move {
                let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                let _guard = Guard(Arc::clone(&live));
                peak.fetch_max(now, Ordering::SeqCst);
                started.fetch_add(1, Ordering::SeqCst);
                if let BrowseQuery::Search(text) = &query {
                    queries.lock().expect("lock").push(text.clone());
                }
                gate.acquire().await.expect("gate").forget();
                Ok(ListedPage { items: vec![ListItem { id: 5, title: "found".into(), ..ListItem::default() }], has_next: false })
            })
        }
    }

    struct Rig {
        live: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        started: Arc<AtomicUsize>,
        gate: Arc<tokio::sync::Semaphore>,
        queries: Arc<std::sync::Mutex<Vec<String>>>,
        runner: Rc<SearchRunner>,
        snapshots: Rc<RefCell<Vec<GlobalSearchSnapshot>>>,
        opened: Rc<RefCell<Vec<i64>>>,
        threads: Rc<RefCell<HashSet<std::thread::ThreadId>>>,
    }

    fn candidates(count: i64) -> Vec<crate::browse::global_search::Candidate> {
        (1..=count)
            .map(|id| {
                let source = Source::new(id, &format!("S{id}"), "en");
                crate::browse::global_search::Candidate { source, package: format!("ext.{id}") }
            })
            .collect()
    }

    fn rig(count: i64) -> Rig {
        rig_on(count, Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT)))
    }

    /// A rig whose fetches take their permits from `runner_gate`, so two rigs can share one.
    fn rig_on(count: i64, runner_gate: Arc<tokio::sync::Semaphore>) -> Rig {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let (live, peak, started, queries) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::default());
        let loader = Arc::new(Gated {
            live: Arc::clone(&live),
            peak: Arc::clone(&peak),
            started: Arc::clone(&started),
            gate: Arc::clone(&gate),
            queries: Arc::clone(&queries),
        });
        let loaders: Loaders = Arc::new(move |_| Some(Arc::clone(&loader) as Arc<dyn PageLoader>));
        let engine = GlobalSearch::new(candidates(count), ["en".to_owned()].into(), BTreeSet::new(), None, false);
        let (snapshots, opened, threads): (Rc<RefCell<Vec<GlobalSearchSnapshot>>>, Rc<RefCell<Vec<i64>>>, Rc<RefCell<HashSet<_>>>) = Default::default();
        let (seen, seen_threads, open_log) = (Rc::clone(&snapshots), Rc::clone(&threads), Rc::clone(&opened));
        let hooks = RunnerHooks {
            changed: Rc::new(move |snapshot| {
                seen_threads.borrow_mut().insert(std::thread::current().id());
                seen.borrow_mut().push(snapshot.clone());
            }),
            open_entry: Rc::new(move |id| open_log.borrow_mut().push(id)),
        };
        Rig { live, peak, started, gate, queries, runner: SearchRunner::with_gate(engine, loaders, hooks, runner_gate), snapshots, opened, threads }
    }

    fn pump(context: &glib::MainContext, mut done: impl FnMut() -> bool) -> bool {
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

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_time().build().expect("runtime")
    }

    /// Runs `body` with the runtime entered and a main context of its own as the thread default.
    fn with_loop(body: impl FnOnce(&glib::MainContext)) {
        let rt = runtime();
        let _entered = rt.enter();
        let context = glib::MainContext::new();
        context.with_thread_default(|| body(&context)).expect("own main context");
    }

    /// Dropping the runner and then the runtime with fetches still pending (no abort first) must not drop or wake a GLib
    /// object on a tokio thread (the intermittent SIGABRT "Value dropped on a different thread").
    #[test]
    fn dropping_the_runner_and_the_runtime_with_a_search_pending_stays_on_the_gtk_thread() {
        let rt = runtime();
        let entered = rt.enter();
        let context = glib::MainContext::new();
        let rig = context
            .with_thread_default(|| {
                let rig = rig(3);
                rig.runner.submit("pending");
                assert!(pump(&context, || rig.started.load(Ordering::SeqCst) == 3), "all three are waiting on the gate");
                rig
            })
            .expect("own main context");
        let live = Arc::clone(&rig.live);
        drop(rig);
        drop(entered);
        drop(rt);
        assert_eq!(live.load(Ordering::SeqCst), 0, "the pending fetches were cancelled");
        drop(context);
    }

    #[test]
    fn at_most_five_fetches_are_in_flight_and_every_result_is_applied_by_the_gtk_thread() {
        with_loop(|context| {
            let rig = rig(7);
            rig.runner.submit("one");
            assert!(pump(context, || rig.started.load(Ordering::SeqCst) == 5), "five start at once");
            let _ = pump(context, || false);
            assert_eq!(rig.started.load(Ordering::SeqCst), 5, "the other two wait for a free slot");
            assert_eq!((rig.runner.in_flight(), rig.runner.queued()), (5, 2));
            rig.gate.add_permits(7);
            assert!(pump(context, || rig.runner.snapshot().answered == 7), "all seven are applied");
            assert!(rig.peak.load(Ordering::SeqCst) <= super::super::global_search::MAX_IN_FLIGHT, "never more than five at once");
            assert_eq!(*rig.threads.borrow(), HashSet::from([std::thread::current().id()]), "one writer: the thread that runs the loop");
            assert_eq!(rig.runner.in_flight(), 0);
        });
    }

    /// Answers at once with a result, except the source 3, which fails.
    struct OneFails;

    impl PageLoader for OneFails {
        fn load(&self, _query: BrowseQuery, _filters: FilterList, _page: u32) -> LoadFuture {
            Box::pin(async { Err("source 3 is down".to_owned()) })
        }
    }

    struct Quick;

    impl PageLoader for Quick {
        fn load(&self, _query: BrowseQuery, _filters: FilterList, _page: u32) -> LoadFuture {
            Box::pin(async { Ok(ListedPage { items: vec![ListItem { id: 5, title: "found".into(), ..ListItem::default() }], has_next: false }) })
        }
    }

    /// T-40b-085 `global_search_five_sources_one_fails`: the other four rows have results, the failing one is an
    /// error row, and the progress reaches the total.
    #[test]
    fn global_search_five_sources_one_fails() {
        with_loop(|context| {
            let loaders: Loaders = Arc::new(|id| Some(if id == 3 { Arc::new(OneFails) as Arc<dyn PageLoader> } else { Arc::new(Quick) }));
            let engine = GlobalSearch::new(candidates(5), ["en".to_owned()].into(), BTreeSet::new(), None, false);
            let hooks = RunnerHooks { changed: Rc::new(|_| {}), open_entry: Rc::new(|_| {}) };
            let runner = SearchRunner::new(engine, loaders, hooks);
            runner.submit("one");
            assert!(pump(context, || runner.snapshot().answered == 5), "an error counts as an answer");
            let snapshot = runner.snapshot();
            assert_eq!((snapshot.answered, snapshot.total), (5, 5), "progress = total");
            let errors: Vec<i64> = snapshot.rows.iter().filter(|row| matches!(row.state, SearchState::Error(_))).map(|row| row.source.id).collect();
            assert_eq!(errors, [3], "only the failing source is an error row");
            let line = snapshot.rows.iter().find_map(|row| match &row.state {
                SearchState::Error(message) => Some((row.source.name.clone(), message.clone())),
                _ => None,
            });
            let (name, message) = line.expect("the failing source has its line");
            assert_eq!(message, format!("Couldn't load {name}"), "one plain line, not the source's own text");
            assert_eq!(snapshot.rows.iter().filter(|row| matches!(&row.state, SearchState::Results(items) if items.len() == 1)).count(), 4);
        });
    }

    /// T-40b-085 `global_search_semaphore_limits_to_5`: eight sources that wait, never more than five in flight, and
    /// five really do run at once.
    #[test]
    fn global_search_semaphore_limits_to_5() {
        with_loop(|context| {
            let rig = rig(8);
            rig.runner.submit("one");
            assert!(pump(context, || rig.started.load(Ordering::SeqCst) == 5));
            let _ = pump(context, || false);
            rig.gate.add_permits(8);
            assert!(pump(context, || rig.runner.snapshot().answered == 8));
            assert_eq!(rig.peak.load(Ordering::SeqCst), 5, "max in-flight = 5");
        });
    }

    /// T-40b-085 `global_search_abort_on_new_query`: a new query aborts the blocked old requests, and nothing of the old
    /// query is shown afterwards.
    #[test]
    fn global_search_abort_on_new_query() {
        with_loop(|context| {
            let rig = rig(2);
            rig.runner.submit("old");
            assert!(pump(context, || rig.live.load(Ordering::SeqCst) == 2));
            let before = rig.snapshots.borrow().len();
            rig.runner.submit("new");
            assert!(pump(context, || rig.live.load(Ordering::SeqCst) == 2 && rig.started.load(Ordering::SeqCst) == 4), "the old two were dropped, the new two run");
            rig.gate.add_permits(4);
            assert!(pump(context, || rig.runner.snapshot().answered == 2));
            let _ = pump(context, || false);
            let late: Vec<_> = rig.snapshots.borrow().iter().skip(before).filter(|s| s.query == "old" && s.answered > 0).cloned().collect();
            assert!(late.is_empty(), "no result of the old query is applied after the new one was asked");
            assert_eq!(rig.runner.snapshot().query, "new");
        });
    }

    #[test]
    fn two_search_screens_share_one_gate_of_five_fetches() {
        with_loop(|context| {
            let shared = Arc::new(tokio::sync::Semaphore::new(MAX_IN_FLIGHT));
            let (first, second) = (rig_on(4, Arc::clone(&shared)), rig_on(4, Arc::clone(&shared)));
            first.runner.submit("one");
            second.runner.submit("two");
            let total = || first.started.load(Ordering::SeqCst) + second.started.load(Ordering::SeqCst);
            assert!(pump(context, || total() == 5), "five fetches run across both screens (E-GS-08)");
            let _ = pump(context, || false);
            assert_eq!(total(), 5, "the sixth waits for a permit");
            first.gate.add_permits(4);
            assert!(pump(context, || first.runner.snapshot().answered == 4), "the first screen finishes");
            assert!(pump(context, || second.started.load(Ordering::SeqCst) == 4), "the freed permits go to the second screen");
        });
    }

    #[test]
    fn a_new_search_aborts_the_old_fetches_and_drops_their_late_results() {
        with_loop(|context| {
            let rig = rig(2);
            rig.runner.submit("old");
            assert!(pump(context, || rig.live.load(Ordering::SeqCst) == 2));
            rig.runner.submit("new");
            // the two old loads are dropped (their guards go) and two new ones start
            assert!(pump(context, || rig.started.load(Ordering::SeqCst) == 4 && rig.live.load(Ordering::SeqCst) == 2), "old fetches aborted, new ones running");
            assert_eq!(*rig.queries.lock().expect("lock"), ["old", "old", "new", "new"]);
            rig.gate.add_permits(2);
            assert!(pump(context, || rig.runner.snapshot().answered == 2));
            let snapshot = rig.runner.snapshot();
            assert_eq!(snapshot.query, "new");
            assert!(snapshot.rows.iter().all(|row| matches!(row.state, SearchState::Results(_))));
        });
    }

    /// Records the filter list each fetch was given.
    struct Recording(Arc<std::sync::Mutex<Vec<usize>>>);
    impl PageLoader for Recording {
        fn load(&self, _query: BrowseQuery, filters: FilterList, _page: u32) -> LoadFuture {
            self.0.lock().expect("lock").push(filters.len());
            Box::pin(async { Ok(ListedPage { items: Vec::new(), has_next: false }) })
        }
    }

    #[test]
    fn each_fetch_starts_from_the_sources_default_filters() {
        with_loop(|context| {
            let seen: Arc<std::sync::Mutex<Vec<usize>>> = Arc::default();
            let loader = Arc::new(Recording(Arc::clone(&seen)));
            let loaders: Loaders = Arc::new(move |_| Some(Arc::clone(&loader) as Arc<dyn PageLoader>));
            let engine = GlobalSearch::new(candidates(2), ["en".to_owned()].into(), BTreeSet::new(), None, false);
            let hooks = RunnerHooks { changed: Rc::new(|_| {}), open_entry: Rc::new(|_| {}) };
            let runner = SearchRunner::new(engine, loaders, hooks);
            runner.set_default_filters(Arc::new(|id| Some(FilterList((0..id).map(|n| kuro_source::filter::Filter::check_box(&format!("f{n}"))).collect()))));
            runner.submit("x");
            assert!(pump(context, || runner.snapshot().answered == 2));
            let mut lengths = seen.lock().expect("lock").clone();
            lengths.sort();
            assert_eq!(lengths, [1, 2], "each source got its own default list, not an empty one");
        });
    }

    #[test]
    fn a_missing_loader_is_an_error_row_not_a_panic() {
        with_loop(|context| {
            let engine = GlobalSearch::new(candidates(1), ["en".to_owned()].into(), BTreeSet::new(), None, false);
            let snapshots: Rc<RefCell<Vec<GlobalSearchSnapshot>>> = Rc::default();
            let seen = Rc::clone(&snapshots);
            let hooks = RunnerHooks { changed: Rc::new(move |s| seen.borrow_mut().push(s.clone())), open_entry: Rc::new(|_| {}) };
            let runner = SearchRunner::new(engine, Arc::new(|_| None), hooks);
            runner.submit("x");
            assert!(pump(context, || runner.snapshot().answered == 1));
            assert!(matches!(&runner.snapshot().rows[0].state, SearchState::Error(message) if message.contains("S1")));
        });
    }

    #[test]
    fn the_chips_search_again_and_a_blank_query_searches_nothing() {
        with_loop(|context| {
            let rig = rig(2);
            rig.runner.submit("  ");
            assert_eq!(rig.started.load(Ordering::SeqCst), 0, "a blank query is a no-op");
            rig.runner.submit("one");
            rig.gate.add_permits(2);
            assert!(pump(context, || rig.runner.snapshot().answered == 2));
            rig.runner.set_filter(SourceFilter::All);
            let _ = pump(context, || false);
            assert_eq!(rig.started.load(Ordering::SeqCst), 2, "both answered: All fetches nothing new");
            let before = rig.snapshots.borrow().len();
            rig.runner.set_only_has_results(true);
            assert_eq!(rig.snapshots.borrow().len(), before + 1, "the toggle redraws and fetches nothing");
        });
    }

    #[test]
    fn one_source_with_an_extension_filter_and_one_result_opens_the_entry_once() {
        with_loop(|context| {
            let rig = rig(2);
            rig.runner.request("one", Some("ext.1"));
            assert!(rig.opened.borrow().is_empty(), "still loading: nothing opens");
            rig.gate.add_permits(1);
            assert!(pump(context, || !rig.opened.borrow().is_empty()), "GS-EC-13: the single result opens its page");
            assert_eq!(*rig.opened.borrow(), [5]);
            let _ = pump(context, || false);
            assert_eq!(rig.opened.borrow().len(), 1, "once");
        });
    }

    #[test]
    fn two_sources_or_no_extension_filter_show_the_results() {
        with_loop(|context| {
            let rig = rig(2);
            rig.runner.request("one", Some(""));
            rig.gate.add_permits(2);
            assert!(pump(context, || rig.runner.snapshot().answered == 2));
            assert!(rig.opened.borrow().is_empty(), "an empty extension filter selects every source: results, no auto-open");
        });
    }

    #[test]
    fn dropping_the_runner_ends_the_fetches() {
        with_loop(|context| {
            let rig = rig(2);
            rig.runner.submit("x");
            assert!(pump(context, || rig.live.load(Ordering::SeqCst) == 2));
            let Rig { runner, live, .. } = rig;
            drop(runner);
            assert!(pump(context, || live.load(Ordering::SeqCst) == 0), "the tasks go with the page");
        });
    }

    /// A resolver that logs each text it is asked about and answers with `entry` (`None`: nothing resolves it).
    fn resolver(entry: Option<i64>, asked: &Arc<std::sync::Mutex<Vec<String>>>) -> LinkResolver {
        let asked = Arc::clone(asked);
        Arc::new(move |uri| {
            asked.lock().expect("lock").push(uri.to_owned());
            Some(Box::pin(async move { entry.map(|id| ListItem { id, ..ListItem::default() }) }) as LinkFuture)
        })
    }

    #[test]
    fn a_pasted_link_opens_the_entry_a_source_resolves_and_searches_no_catalogue() {
        with_loop(|context| {
            let rig = rig(2);
            let asked = Arc::default();
            rig.runner.set_link_resolver(resolver(Some(77), &asked));
            rig.runner.submit("https://site.test/m/1");
            assert!(pump(context, || !rig.opened.borrow().is_empty()), "the resolved entry opens");
            assert_eq!(*rig.opened.borrow(), [77]);
            assert_eq!(*asked.lock().expect("lock"), ["https://site.test/m/1"]);
            let _ = pump(context, || false);
            assert_eq!(rig.started.load(Ordering::SeqCst), 0, "no catalogue was searched before or after");
        });
    }

    #[test]
    fn a_link_nothing_resolves_is_searched_as_text() {
        with_loop(|context| {
            let rig = rig(2);
            let asked = Arc::default();
            rig.runner.set_link_resolver(resolver(None, &asked));
            rig.runner.submit("https://site.test/m/1");
            assert!(pump(context, || rig.started.load(Ordering::SeqCst) == 2), "both catalogues are searched after the lookup");
            assert_eq!(*rig.queries.lock().expect("lock"), ["https://site.test/m/1", "https://site.test/m/1"]);
            assert!(rig.opened.borrow().is_empty());
        });
    }

    #[test]
    fn a_link_with_no_source_to_resolve_it_is_searched_at_once_and_plain_text_never_asks() {
        with_loop(|context| {
            let rig = rig(1);
            rig.runner.set_link_resolver(Arc::new(|_| None));
            rig.runner.submit("https://site.test/m/1");
            assert!(pump(context, || rig.started.load(Ordering::SeqCst) == 1), "no lookup runs: the catalogue is searched");
            let asked = Arc::default();
            rig.runner.set_link_resolver(resolver(Some(1), &asked));
            rig.runner.submit("one piece");
            assert!(asked.lock().expect("lock").is_empty(), "text that is not a link is not offered to the resolver");
        });
    }
}
