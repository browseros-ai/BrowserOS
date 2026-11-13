//! Per-tab facts, in one place.

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use browseros_core::{TabId, TargetId};
use neo_takeover::TabHold;
use neo_trace::{RunId, TaskId};
use tokio::time::{Duration, Instant};

use super::task_book::TaskBook;
use super::{BookSink, Holder, PageRef, RunScope, SavedTask, Sight, TaskError, TaskRecord};

/// Retained close history. Evicted history advances `forgotten`, so bounded memory never
/// turns an old admission into proof that a tab is still open.
const TOMBSTONES: usize = 256;

/// Popups announced and not yet closed, remembered for sight. An evicted one follows its
/// opener's family only once a listing or an op registers it: until then a bound run is refused
/// it, never wrongly shown it.
const ANNOUNCED: usize = 1024;

/// How far sight follows a popup's openers to a held tab. A longer chain belongs to no family,
/// which refuses rather than grants, and the bound also ends a loop a bad announcement could make.
const OPENER_HOPS: usize = 16;

/// What Neo keeps in memory per tab: who opened it, when it was last in use (its idle clock),
/// which runs in flight used it, its feature state, and which task holds it ([`TaskBook`], whose
/// record of each task's last acted-on tab is the task's built-in `page`). From the holders and
/// the popups' openers it answers whether a run reaches a tab ([`TabRegistry::sight`]).
///
/// One owner, one lock, never held across an await. There is one way out: core's detach hook
/// calls [`TabRegistry::closed`], and a tab's entry and its slots leave together, so no
/// feature's state outlives its tab. `tab_watch` makes that hook fire promptly when someone
/// else closes the tab.
pub(crate) struct TabRegistry {
    state: Mutex<TabState>,
}

/// Who opened a tab. Core has no public `is_agent_created`, so Neo's own page opener marks its
/// tabs here. A popup an agent's tab opened is the agent's too (the settings design, decision
/// 3): with nothing else to close it, a short idle time would leave it open for good. The
/// owner's tabs and their popups stay `Found`, and the idle-tab janitor never closes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabOrigin {
    /// Opened for an agent (`context.newPage`, or the lazy default page) by this process, or a
    /// popup such a tab opened.
    Agent,
    /// Anything else: the owner's tabs and their popups, tabs from before a Neo restart.
    Found,
    /// Neo's own blank tab (`NeoTab`), which keeps the browser open when every other tab closes.
    /// No script sees or reaches it, no task takes it, and the janitor never closes it.
    Neo,
}

#[derive(Default)]
struct TabState {
    open: HashMap<TabId, TabEntry>,
    /// Destroyed bindings still visible in core's cache. One entry per cached tab, removed
    /// on detach or rebind. This evidence only shapes facts and never grants admission.
    gone: HashMap<TabId, TargetId>,
    tasks: TaskBook,
    /// Popup relationships survive an opener whose creating op has not assigned its task yet.
    popups: HashMap<TabId, TabId>,
    /// Popups as the browser announced them, before any listing registered them.
    announced: Announced,
    /// The tabs each run in flight used, from its start to its end record. The janitor never
    /// closes a tab a run in flight used, and the tab's idle clock restarts when the run ends,
    /// so a run longer than the idle time keeps its tabs. Bounded by the runs in flight.
    runs: HashMap<RunId, HashSet<TabId>>,
    /// Numbers every close. Tickets are sampled immediately before a core-cache read.
    closes: u64,
    closed_at: VecDeque<(TabId, u64)>,
    /// Newest evicted close: tickets below this cannot prove that a tab stayed open.
    forgotten: u64,
}

struct TabEntry {
    origin: TabOrigin,
    /// The idle clock: the last time a script op touched the tab, a run that used it ended, or
    /// the host let go of it (`idle_agent_tabs`). Never earlier than the tab's first sighting.
    touched: Instant,
    slots: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl TabRegistry {
    /// An empty registry.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::default(),
        }
    }

    /// A ticket for a liveness read. It must be taken immediately before asking core, never
    /// before an entire operation or a child operation that may open a tab.
    pub(crate) fn seen(&self) -> Seen {
        Seen(self.lock().closes)
    }

    /// `page` is in use (an op targeted it, or a run starts on it): its idle clock restarts.
    /// No task's page moves; that takes an act ([`TabRegistry::acted_on`]).
    pub(crate) fn touched(&self, page: &PageRef, seen: Seen) {
        self.acted_on(page, seen, &[]);
    }

    /// The script of a run acting for `tasks` (its host task and the task it picked, if any) ran
    /// an op on `page`: as [`TabRegistry::touched`], and `page` becomes the built-in `page` of
    /// whichever of `tasks` owns it. Only the script's own calls are acts. Neo's operations
    /// (the prelude's frame refresh after `waitForTimeout`, its listeners, the event pump, the
    /// janitor) only touch, or every wait would re-point the task at whatever tab Neo read last.
    pub(crate) fn acted_on(&self, page: &PageRef, seen: Seen, tasks: &[&TaskId]) {
        let now = Instant::now();
        let mut state = self.lock();
        if !state.proves_open(page.tab, seen) {
            return;
        }
        state.entry(page, now).touched = now;
        for task in tasks {
            state.tasks.acted(page.tab, task);
        }
    }

    /// Neo opened `page` for an agent, so the idle-tab janitor may close it later. The task that
    /// opened it takes it with [`TabRegistry::own`].
    pub(crate) fn opened_by_agent(&self, page: &PageRef, seen: Seen) {
        let now = Instant::now();
        let mut state = self.lock();
        if !state.proves_open(page.tab, seen) {
            return;
        }
        let entry = state.entry(page, now);
        entry.origin = TabOrigin::Agent;
        entry.touched = now;
        // A popup whose event came before its opener was marked.
        state.inherit_agent_origin();
    }

    /// A run started: the janitor keeps every tab it uses until it ends. `default_page`, when
    /// the run knows it at start, is in use from then on.
    pub(crate) fn run_started(&self, run: RunId, default_page: Option<TabId>) {
        let mut state = self.lock();
        let tabs = default_page
            .filter(|tab| state.open.contains_key(tab))
            .into_iter()
            .collect();
        state.runs.insert(run, tabs);
    }

    /// `run` used `tab`: a script op targeted it, or the run opened it. Nothing for a run that
    /// already ended, so an op that settles after its run's end cannot keep a tab forever, or
    /// for a tab that has no entry (it closed, or it was never admitted).
    pub(crate) fn used_in_run(&self, tab: TabId, run: RunId) {
        let mut state = self.lock();
        if !state.open.contains_key(&tab) {
            return;
        }
        if let Some(tabs) = state.runs.get_mut(&run) {
            tabs.insert(tab);
        }
    }

    /// `run`'s end record was written: the tabs it used stop being in use, and their idle
    /// clocks restart now.
    pub(crate) fn run_ended(&self, run: RunId) {
        let now = Instant::now();
        let mut state = self.lock();
        let Some(tabs) = state.runs.remove(&run) else {
            return;
        };
        for tab in tabs {
            if let Some(entry) = state.open.get_mut(&tab) {
                entry.touched = entry.touched.max(now);
            }
        }
    }

    /// Retires a cache entry while core still holds its removal lock. The returned work
    /// drops feature state after both locks are released. A later
    /// same-target admission can then retain its own evidence while this work is delayed.
    pub(crate) fn removing(&self, tab: TabId) -> Box<dyn FnOnce() + Send> {
        let entry = {
            let mut state = self.lock();
            state.closes += 1;
            let number = state.closes;
            state.closed_at.push_back((tab, number));
            if state.closed_at.len() > TOMBSTONES
                && let Some((_, evicted)) = state.closed_at.pop_front()
            {
                state.forgotten = evicted;
            }
            state.gone.remove(&tab);
            state.tasks.closed(tab);
            state
                .popups
                .retain(|popup, opener| *popup != tab && *opener != tab);
            state.announced.forget(tab);
            // A tab number may be reused after a browser restart; a run's old use must not
            // keep the new tab.
            for used in state.runs.values_mut() {
                used.remove(&tab);
            }
            state.open.remove(&tab)
        };
        Box::new(move || {
            // Slot destructors may re-enter this registry or core's cache.
            drop(entry);
        })
    }

    /// Whether a cache read has no intervening detach of this tab and enough retained
    /// history to establish that. Check after copying dead-binding metadata, since a
    /// matching detach clears that metadata.
    pub(crate) fn read_is_current(&self, tab: TabId, seen: Seen) -> bool {
        self.lock().proves_open(tab, seen)
    }

    /// Remembers a dead cached binding only while its cache-read ticket is still valid.
    /// A detach between the read and this write must not recreate metadata after cleanup.
    pub(crate) fn note_gone(&self, tab: TabId, target: TargetId, seen: Seen) -> bool {
        let mut state = self.lock();
        if !state.proves_open(tab, seen) {
            return false;
        }
        state.gone.insert(tab, target);
        true
    }

    /// The dead binding remembered for this tab, independent of its authorization state.
    pub(crate) fn gone_target(&self, tab: TabId) -> Option<TargetId> {
        self.lock().gone.get(&tab).cloned()
    }

    /// Core now shows a replacement target for this tab.
    pub(crate) fn forget_gone(&self, tab: TabId) {
        self.lock().gone.remove(&tab);
    }

    /// `tab` is Neo's blank tab from now on (`NeoTab`), and stops being any task's. Neo just
    /// listed or opened it, so it is open.
    pub(crate) fn make_neo(&self, tab: TabId) {
        let mut state = self.lock();
        let entry = state.entry(&PageRef { tab }, Instant::now());
        entry.origin = TabOrigin::Neo;
        state.tasks.detach(tab);
    }

    /// `tab` is no longer Neo's: the owner browsed in it, so it is theirs, as a tab they
    /// opened is (`Found`): scripts see it, and the janitor never closes it.
    pub(crate) fn release_neo(&self, tab: TabId) {
        if let Some(entry) = self.lock().open.get_mut(&tab)
            && entry.origin == TabOrigin::Neo
        {
            entry.origin = TabOrigin::Found;
        }
    }

    /// Whether `tab` is Neo's blank tab, which scripts never see and the door refuses.
    #[must_use]
    pub(crate) fn is_neo(&self, tab: TabId) -> bool {
        self.lock()
            .open
            .get(&tab)
            .is_some_and(|entry| entry.origin == TabOrigin::Neo)
    }

    /// Whether `tab` may become Neo's blank tab by adoption: it is not the agent's and no task
    /// Neo remembers owns it. The trace's own task links are the caller's to check: after a
    /// server restart the book holds its saved tabs only once the tab-holders file is restored.
    #[must_use]
    pub(crate) fn adoptable(&self, tab: TabId) -> bool {
        let state = self.lock();
        state
            .open
            .get(&tab)
            .is_none_or(|entry| entry.origin != TabOrigin::Agent)
            && state.tasks.owner(tab).is_none()
    }

    /// Whether an admitted operation has registered this tab and it has not since closed.
    pub(crate) fn is_open(&self, tab: TabId) -> bool {
        self.lock().open.contains_key(&tab)
    }

    /// Per-tab feature state (sdk-playwright's facade), created on first
    /// use. `None` once the tab closed. `T::default` runs under the registry's lock, so it must
    /// not call back into the registry.
    #[must_use]
    pub(crate) fn slot<T: Default + Send + Sync + 'static>(
        &self,
        page: &PageRef,
    ) -> Option<Arc<T>> {
        let mut state = self.lock();
        let entry = state.open.get_mut(&page.tab)?;
        let slot = entry
            .slots
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Arc::new(T::default()));
        // Keyed by `TypeId::of::<T>()`, so the downcast always succeeds.
        Arc::clone(slot).downcast::<T>().ok()
    }

    /// What memory says a host task's run starts on: the task's own open tab it last acted on,
    /// else its newest ([`TaskRecord::page_tab`]). A known task with no open tab gets a new one,
    /// never a neighbour's. No global default is kept.
    #[must_use]
    pub(crate) fn task_page(&self, task: &TaskId) -> TaskPage {
        let state = self.lock();
        let Ok(record) = state.tasks.get(task) else {
            return TaskPage::Unknown;
        };
        record
            .page_tab(|tab| state.open.contains_key(&tab))
            .map_or(TaskPage::NoTab, |tab| TaskPage::Tab(PageRef { tab }))
    }

    /// Starts an agent-named task, evicting only old empty tasks at the bound.
    pub(crate) fn create_task(&self, name: Option<String>) -> Result<TaskRecord, TaskError> {
        self.lock().tasks.create(name)
    }

    /// Registers a host task before its first tab opens.
    pub(crate) fn ensure_task(&self, id: &TaskId) -> Result<(), TaskError> {
        self.lock().tasks.ensure(id)
    }

    /// The task's current ordered membership.
    pub(crate) fn task(&self, id: &TaskId) -> Result<TaskRecord, TaskError> {
        self.lock().tasks.get(id).cloned()
    }

    /// The owner is independent of the last host task to touch this tab.
    pub(crate) fn owner(&self, tab: TabId) -> Option<TaskId> {
        self.lock().tasks.owner(tab).cloned()
    }

    /// The family root of task `id`, while the book has it.
    #[must_use]
    pub(crate) fn task_family(&self, id: &TaskId) -> Option<TaskId> {
        self.lock()
            .tasks
            .get(id)
            .ok()
            .map(|task| task.family.clone())
    }

    /// Who holds `tab` now: its task and that task's family.
    #[must_use]
    pub(crate) fn holder(&self, tab: TabId) -> Option<Holder> {
        self.lock().tasks.holder(tab).cloned()
    }

    /// Starts an agent-named task in `family`: one created in a run whose host task is of that
    /// family, so it counts as that run's own.
    pub(crate) fn create_task_in(
        &self,
        name: Option<String>,
        family: &TaskId,
    ) -> Result<TaskRecord, TaskError> {
        self.lock().tasks.create_in(name, family)
    }

    /// Whether a run of `scope` may reach `tab` now: the one check every door seam asks.
    ///
    /// Neo's blank tab is [`Sight::Reserved`] in every scope. A [`RunScope::Browser`] run is
    /// shown every other tab, as before task scope. A [`RunScope::Family`] run is shown a tab its
    /// family holds, or a popup no task holds yet whose opener's family is the run's (through
    /// popups of popups); a tab that closed while its family held it is [`Sight::Closed`];
    /// anything else is [`Sight::Foreign`]. The run's own opens in flight are the run's to
    /// admit: the registry learns of a tab only once core names it.
    #[must_use]
    pub(crate) fn sight(&self, scope: &RunScope, tab: TabId) -> Sight {
        let state = self.lock();
        if state
            .open
            .get(&tab)
            .is_some_and(|entry| entry.origin == TabOrigin::Neo)
        {
            return Sight::Reserved;
        }
        let RunScope::Family(family) = scope else {
            return Sight::Shown;
        };
        if state.family_of(tab) == Some(family) {
            Sight::Shown
        } else if state.tasks.was_closed(tab, family) {
            Sight::Closed
        } else {
            Sight::Foreign
        }
    }

    /// The browser announced `popup` with `opener` as its opener (`Target.targetCreated`), before
    /// any listing showed it. From now on sight follows the opener's family for it, so a bound
    /// run's popup is its own from its first moment. Only sight reads this: the popup joins a
    /// task, and becomes the agent's, only when a listing or an op registers it
    /// ([`TabRegistry::popup`]). A note can come after a close core already reported: one
    /// socket's events are ordered, so the popup's own destroy follows and the watch's confirmed
    /// close drops the note ([`TabRegistry::forget_announced`]), and a closed opener holds
    /// nothing for a note to follow.
    pub(crate) fn note_opener(&self, popup: TabId, opener: TabId) {
        if popup != opener {
            self.lock().announced.note(popup, opener);
        }
    }

    /// The tab watch confirmed `tab` closed. Its announced links go now: a popup that opened and
    /// closed between two listings never entered core's cache, so its close never reaches
    /// [`TabRegistry::removing`].
    pub(crate) fn forget_announced(&self, tab: TabId) {
        self.lock().announced.forget(tab);
    }

    /// The tab watch follows a new socket. Announced openers go: a restarted browser numbers
    /// its tabs anew, so an old note could tie a new tab to an old family. The new socket's
    /// discovery announces every live popup again.
    pub(crate) fn forget_openers(&self) {
        self.lock().announced.clear();
    }

    /// Where the task book's changes go from now on ([`BookSink`]), once: false when a sink is
    /// already set, which stays. Until one is set, changes go nowhere.
    pub(crate) fn set_book_sink(&self, sink: Arc<dyn BookSink>) -> bool {
        self.lock().tasks.set_sink(sink)
    }

    /// Rebuilds the task book from a saved record, once, at start, before the first run is
    /// admitted. Each task keeps its saved family and origin unless the book already has it.
    /// A tab joins only if this registry has it open (not Neo's blank tab) and no task holds it,
    /// so the caller registers each live tab first, as a run's start does for a task's tabs it
    /// restores ([`TabRegistry::touched`] after core resolved the tab). Emits no change for the
    /// tabs it restores, since the saved record already holds them all. A popup the tab watch
    /// named before the restore, whose opener the restore gave back (one opened while Neo's
    /// server was down), then joins the opener's task, and that join is reported as any is.
    pub(crate) fn restore_book(&self, tasks: Vec<SavedTask>) -> Result<(), TaskError> {
        let mut guard = self.lock();
        let state = &mut *guard;
        for task in tasks {
            state.tasks.restore_saved(task, |tab| {
                state
                    .open
                    .get(&tab)
                    .is_some_and(|entry| entry.origin != TabOrigin::Neo)
            })?;
        }
        state.inherit_popups();
        Ok(())
    }

    /// Gives a registered tab to task `id`, as a run that may use every task opens in it
    /// ([`TabRegistry::own_in_scope`]): tests set ownership up through here.
    #[cfg(test)]
    pub(crate) fn own(&self, page: &PageRef, id: &TaskId) -> Result<(), TaskError> {
        self.own_in_scope(page, id, &RunScope::Browser).map(|_| ())
    }

    /// Assigns a registered tab to task `id` after a run's open of it returns, only while the
    /// run's `scope` may still use the task, asked under the same lock as the join. A close that
    /// already removed the tab's entry wins, so assignment cannot revive a closed tab. A host may name an agent's task while a run
    /// with no task opens a tab in it, and such a run never adds a tab to a host's task. False
    /// when the task is no longer the run's to open in; the tab then joins no task.
    pub(crate) fn own_in_scope(
        &self,
        page: &PageRef,
        id: &TaskId,
        scope: &RunScope,
    ) -> Result<bool, TaskError> {
        let mut state = self.lock();
        if !scope.may_use(state.tasks.get(id)?) {
            return Ok(false);
        }
        if state.open.contains_key(&page.tab) {
            state.tasks.attach(page.tab, id)?;
            state.inherit_popups();
        }
        Ok(true)
    }

    /// A popup inherits its opener, even if its event preceded the opener's task assignment.
    pub(crate) fn popup(&self, page: &PageRef, opener: TabId, seen: Seen) {
        let mut state = self.lock();
        if !state.proves_open(page.tab, seen) || !state.proves_open(opener, seen) {
            return;
        }
        state.entry(page, Instant::now());
        state.popups.insert(page.tab, opener);
        state.inherit_popups();
        state.inherit_agent_origin();
    }

    /// A successful task close forgets it only once every owned tab is gone.
    pub(crate) fn forget_task(&self, id: &TaskId) -> bool {
        self.lock().tasks.remove_empty(id)
    }

    /// The agent's tabs (`TabOrigin::Agent`), the only ones the janitor may close, whether or
    /// not they are idle. The janitor asks the host about each before
    /// [`TabRegistry::idle_agent_tabs`].
    #[must_use]
    pub(crate) fn agent_tabs(&self) -> Vec<TabId> {
        self.lock()
            .open
            .iter()
            .filter(|(_, entry)| entry.origin == TabOrigin::Agent)
            .map(|(tab, _)| *tab)
            .collect()
    }

    /// The agent's tabs nobody used for `idle`, for the janitor, the longest idle first. A tab is
    /// in use while a run that used it is in flight, or while `holds` says the host holds it
    /// with a lock, a fit, or a keep; its idle clock restarts when the last of those ends. A hold is kept
    /// in the tab's clock here, so a claim the host forgets an hour after it ended still counts.
    pub(crate) fn idle_agent_tabs(
        &self,
        idle: Duration,
        now: Instant,
        holds: &HashMap<TabId, TabHold>,
    ) -> Vec<PageRef> {
        let mut state = self.lock();
        let in_runs: HashSet<TabId> = state.runs.values().flatten().copied().collect();
        let mut idle_tabs: Vec<(Instant, PageRef)> = Vec::new();
        for (tab, entry) in &mut state.open {
            let busy = in_runs.contains(tab);
            if entry.is_idle(busy, holds.get(tab).copied(), idle, now) {
                idle_tabs.push((entry.touched, PageRef { tab: *tab }));
            }
        }
        idle_tabs.sort_by_key(|(touched, page)| (*touched, page.tab.0));
        idle_tabs.into_iter().map(|(_, page)| page).collect()
    }

    /// Whether `tab` is still one of the agent's idle tabs, by the rule of
    /// [`TabRegistry::idle_agent_tabs`], with `hold` asked just now. The janitor asks it right
    /// before each close: a sweep closes one tab at a time, and a run may use a tab, or a host fit
    /// it, after the sweep chose it.
    pub(crate) fn still_idle(
        &self,
        tab: TabId,
        idle: Duration,
        now: Instant,
        hold: Option<TabHold>,
    ) -> bool {
        let mut state = self.lock();
        let busy = state.runs.values().any(|used| used.contains(&tab));
        state
            .open
            .get_mut(&tab)
            .is_some_and(|entry| entry.is_idle(busy, hold, idle, now))
    }

    /// Whether Neo saw `tab` close, within the close history it keeps. A browser's own listing
    /// can still show a tab for a moment after its close was answered.
    #[must_use]
    pub(crate) fn has_closed(&self, tab: TabId) -> bool {
        self.lock()
            .closed_at
            .iter()
            .any(|(closed, _)| *closed == tab)
    }

    fn lock(&self) -> MutexGuard<'_, TabState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl TabState {
    /// The family `tab` belongs to: its holder's, else, for a popup no task holds yet, its
    /// opener's, through popups of popups.
    fn family_of(&self, tab: TabId) -> Option<&TaskId> {
        let mut tab = tab;
        for _ in 0..OPENER_HOPS {
            if let Some(holder) = self.tasks.holder(tab) {
                return Some(&holder.family);
            }
            tab = self
                .popups
                .get(&tab)
                .copied()
                .or_else(|| self.announced.opener(tab))?;
        }
        None
    }

    fn inherit_popups(&mut self) {
        loop {
            let inherited: Vec<_> = self
                .popups
                .iter()
                .filter_map(|(tab, opener)| {
                    if self.tasks.owner(*tab).is_some() {
                        return None;
                    }
                    self.tasks.owner(*opener).map(|id| (*tab, id.clone()))
                })
                .collect();
            if inherited.is_empty() {
                return;
            }
            for (tab, id) in inherited {
                // The owner was read under this lock, so its task still exists. A popup is the
                // task's tab, but not its page until the task's own script acts on it.
                let _ = self.tasks.join(tab, &id);
            }
        }
    }

    /// Marks every popup of an agent's tab as the agent's, through popups of popups.
    fn inherit_agent_origin(&mut self) {
        loop {
            let inherited: Vec<TabId> = self
                .popups
                .iter()
                .filter(|(popup, opener)| {
                    self.open
                        .get(opener)
                        .is_some_and(|entry| entry.origin == TabOrigin::Agent)
                        && self
                            .open
                            .get(popup)
                            .is_some_and(|entry| entry.origin != TabOrigin::Agent)
                })
                .map(|(popup, _)| *popup)
                .collect();
            if inherited.is_empty() {
                return;
            }
            for popup in inherited {
                if let Some(entry) = self.open.get_mut(&popup) {
                    entry.origin = TabOrigin::Agent;
                }
            }
        }
    }

    fn proves_open(&self, tab: TabId, seen: Seen) -> bool {
        seen.0 >= self.forgotten
            && !self
                .closed_at
                .iter()
                .any(|(closed, number)| *closed == tab && *number > seen.0)
    }

    fn entry(&mut self, page: &PageRef, now: Instant) -> &mut TabEntry {
        self.open.entry(page.tab).or_insert_with(|| TabEntry {
            origin: TabOrigin::Found,
            touched: now,
            slots: HashMap::new(),
        })
    }
}

impl TabEntry {
    /// The janitor's rule for one entry: an agent's tab, used by no run in flight (`busy`), not
    /// held by the host now, and whose clock is `idle` old. A hold seen now, or the instant one
    /// ended, is kept in the clock, so a claim the host forgets an hour later still counts.
    fn is_idle(&mut self, busy: bool, hold: Option<TabHold>, idle: Duration, now: Instant) -> bool {
        if self.origin != TabOrigin::Agent || busy {
            return false;
        }
        match hold {
            Some(TabHold::Now) => {
                self.touched = self.touched.max(now);
                return false;
            }
            Some(TabHold::Until(ended)) => self.touched = self.touched.max(ended),
            None => {}
        }
        now.saturating_duration_since(self.touched) >= idle
    }
}

/// Popup → opener as the browser announced each popup, oldest first, bounded by [`ANNOUNCED`].
/// A link goes when either tab closes, as `TabState::popups`' do, through core's removal or, for
/// a popup core never cached, the watch's confirmed close; every link goes at a new socket.
#[derive(Default)]
struct Announced {
    openers: HashMap<TabId, TabId>,
    order: VecDeque<TabId>,
}

impl Announced {
    fn note(&mut self, popup: TabId, opener: TabId) {
        if self.openers.insert(popup, opener).is_some() {
            return;
        }
        self.order.push_back(popup);
        if self.order.len() > ANNOUNCED
            && let Some(oldest) = self.order.pop_front()
        {
            self.openers.remove(&oldest);
        }
    }

    fn opener(&self, popup: TabId) -> Option<TabId> {
        self.openers.get(&popup).copied()
    }

    fn forget(&mut self, tab: TabId) {
        let Self { openers, order } = self;
        let before = openers.len();
        openers.retain(|popup, opener| *popup != tab && *opener != tab);
        if openers.len() != before {
            order.retain(|popup| openers.contains_key(popup));
        }
    }

    fn clear(&mut self) {
        self.openers.clear();
        self.order.clear();
    }
}

/// A host task's page as memory knows it at a run's start ([`TabRegistry::task_page`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TaskPage {
    /// The task's own open tab: the one it last acted on, else its newest.
    Tab(PageRef),
    /// Neo knows the task and none of its tabs is open: the run opens a new tab for it.
    NoTab,
    /// Neo does not know the task: it never ran in this process, or Neo restarted since and the
    /// task had no open tab to give back. The run opens a new tab for it.
    Unknown,
}

/// A point in the registry's close history, valid only for a read started after it was taken.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Seen(u64);
#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use std::collections::{HashMap, HashSet};

    use browseros_core::{TabId, TargetId};
    use neo_takeover::TabHold;
    use neo_trace::{RunId, TaskId};
    use tokio::time::{Duration, Instant};

    use super::{ANNOUNCED, Seen, TOMBSTONES, TabOrigin, TabRegistry, TaskPage};
    use crate::browser::book_change::BookLog;
    use crate::browser::{Holder, PageRef, RunScope, SavedTask, Sight, TaskOrigin};

    #[derive(Default)]
    struct Marker;

    fn page(_id: u32, tab: i64) -> PageRef {
        PageRef { tab: TabId(tab) }
    }

    #[test]
    fn closing_a_tab_drops_its_slots() {
        let tabs = TabRegistry::new();
        let tab = page(1, 101);
        tabs.touched(&tab, tabs.seen());
        let held = tabs.slot::<Marker>(&tab);
        assert!(held.is_some(), "a touched tab gets a slot");
        tabs.removing(tab.tab)();
        assert_eq!(
            held.as_ref().map(Arc::strong_count),
            Some(1),
            "the registry let go of the closed tab's slot"
        );
        assert!(
            tabs.slot::<Marker>(&tab).is_none(),
            "a closed tab has no slots"
        );
        assert!(!tabs.is_open(tab.tab), "a closed tab has no entry");
    }

    #[test]
    fn a_late_touch_cannot_revive_a_closed_tab() {
        let tabs = TabRegistry::new();
        let tab = page(1, 101);
        tabs.touched(&tab, tabs.seen());
        let seen = tabs.seen();
        tabs.removing(tab.tab)();
        tabs.touched(&tab, seen);
        tabs.opened_by_agent(&tab, seen);
        assert!(
            !tabs.is_open(tab.tab),
            "the ticket predates the close, so neither writer may revive the entry"
        );
        assert!(
            tabs.slot::<Marker>(&tab).is_none(),
            "no slot for a dead tab"
        );
    }

    #[test]
    fn a_task_page_is_its_own_tab_its_own_script_last_acted_on() -> Result<(), Box<dyn Error>> {
        let tabs = TabRegistry::new();
        let first = TaskId::parse("task-1")?;
        let second = TaskId::parse("task-2")?;
        assert_eq!(tabs.task_page(&first), TaskPage::Unknown, "never ran here");
        tabs.ensure_task(&first)?;
        tabs.ensure_task(&second)?;
        assert_eq!(
            tabs.task_page(&first),
            TaskPage::NoTab,
            "a known task, no tab yet"
        );
        let (older, newer, theirs) = (page(1, 101), page(2, 102), page(3, 103));
        for tab in [&older, &newer, &theirs] {
            tabs.touched(tab, tabs.seen());
        }
        tabs.own(&older, &first)?;
        tabs.own(&newer, &first)?;
        tabs.own(&theirs, &second)?;
        assert_eq!(
            tabs.task_page(&first),
            TaskPage::Tab(newer.clone()),
            "the newest own tab"
        );
        tabs.acted_on(&older, tabs.seen(), &[&first]);
        assert_eq!(
            tabs.task_page(&first),
            TaskPage::Tab(older.clone()),
            "the own tab its script last acted on"
        );
        tabs.touched(&newer, tabs.seen());
        assert_eq!(
            tabs.task_page(&first),
            TaskPage::Tab(older.clone()),
            "Neo's own read of another own tab moves nothing"
        );
        tabs.acted_on(&newer, tabs.seen(), &[&second]);
        assert_eq!(
            tabs.task_page(&first),
            TaskPage::Tab(older.clone()),
            "another task's script acting on this task's tab moves nothing"
        );
        tabs.removing(older.tab)();
        assert_eq!(
            tabs.task_page(&first),
            TaskPage::Tab(newer.clone()),
            "a closed tab is no page; the task's other tab is"
        );
        tabs.removing(newer.tab)();
        assert_eq!(
            tabs.task_page(&first),
            TaskPage::NoTab,
            "with its tabs closed the task gets a new one, never a neighbour's"
        );
        Ok(())
    }

    #[test]
    fn a_script_acting_on_other_tabs_keeps_its_own_page() -> Result<(), Box<dyn Error>> {
        let tabs = TabRegistry::new();
        let mine = TaskId::parse("task-mine")?;
        let theirs = TaskId::parse("task-theirs")?;
        tabs.ensure_task(&mine)?;
        tabs.ensure_task(&theirs)?;
        let (own, their_tab, new_tab) = (page(1, 101), page(2, 102), page(3, 103));
        for tab in [&own, &their_tab, &new_tab] {
            tabs.touched(tab, tabs.seen());
        }
        tabs.own(&own, &mine)?;
        tabs.own(&their_tab, &theirs)?;
        // Reached through `context.pages()`: another task's tab and the browser's New Tab.
        tabs.acted_on(&their_tab, tabs.seen(), &[&mine]);
        tabs.acted_on(&new_tab, tabs.seen(), &[&mine]);
        assert_eq!(
            tabs.task_page(&mine),
            TaskPage::Tab(own),
            "still its own tab"
        );
        assert_eq!(
            tabs.task_page(&theirs),
            TaskPage::Tab(their_tab),
            "and the other task keeps its own"
        );
        assert_eq!(tabs.owner(new_tab.tab), None, "an act never takes a tab");
        Ok(())
    }

    #[test]
    fn a_popup_joins_its_openers_task_without_becoming_its_page() -> Result<(), Box<dyn Error>> {
        let tabs = TabRegistry::new();
        let task = TaskId::parse("task-1")?;
        tabs.ensure_task(&task)?;
        let (opener, popup) = (page(1, 101), page(2, 102));
        tabs.touched(&opener, tabs.seen());
        tabs.own(&opener, &task)?;
        // Discovered by the tab watcher, with no script op on it.
        tabs.popup(&popup, opener.tab, tabs.seen());
        assert_eq!(
            tabs.owner(popup.tab),
            Some(task.clone()),
            "the opener's task's tab"
        );
        assert_eq!(
            tabs.task_page(&task),
            TaskPage::Tab(opener.clone()),
            "not its page: the task's script last acted on the opener"
        );
        tabs.acted_on(&popup, tabs.seen(), &[&task]);
        assert_eq!(
            tabs.task_page(&task),
            TaskPage::Tab(popup.clone()),
            "until its script acts on it"
        );
        tabs.removing(popup.tab)();
        assert_eq!(
            tabs.task_page(&task),
            TaskPage::Tab(opener),
            "then the opener again"
        );
        Ok(())
    }

    #[test]
    fn only_idle_agent_tabs_are_offered_to_the_janitor() {
        let tabs = TabRegistry::new();
        let agent = page(1, 101);
        let owner = page(2, 102);
        tabs.opened_by_agent(&agent, tabs.seen());
        tabs.touched(&owner, tabs.seen());
        let idle = Duration::from_mins(30);
        let later = Instant::now() + idle;
        assert_eq!(
            tabs.idle_agent_tabs(idle, later, &HashMap::new()),
            vec![agent.clone()],
            "the owner's tab is never the janitor's, however idle"
        );
        assert!(
            tabs.idle_agent_tabs(idle, Instant::now(), &HashMap::new())
                .is_empty(),
            "a tab touched just now is not idle"
        );
        tabs.touched(&agent, tabs.seen());
        assert!(
            tabs.idle_agent_tabs(idle, later, &HashMap::new())
                .is_empty(),
            "a touch restarts the idle clock"
        );
        assert_eq!(
            tabs.idle_agent_tabs(idle, Instant::now() + idle, &HashMap::new()),
            vec![agent],
            "a touch keeps an agent tab the janitor's"
        );
    }

    #[test]
    fn a_run_in_flight_keeps_the_tabs_it_used_until_it_ends() {
        let tabs = TabRegistry::new();
        let idle = Duration::from_mins(5);
        let used = page(1, 101);
        let other = page(2, 102);
        let run = RunId::new();
        tabs.run_started(run, None);
        tabs.opened_by_agent(&used, tabs.seen());
        tabs.used_in_run(used.tab, run);
        tabs.opened_by_agent(&other, tabs.seen());
        let much_later = Instant::now() + Duration::from_mins(20);
        assert_eq!(
            tabs.idle_agent_tabs(idle, much_later, &HashMap::new()),
            vec![other],
            "a run longer than the idle time keeps the tab it used; the tab it never used goes"
        );
        tabs.run_ended(run);
        assert!(
            tabs.idle_agent_tabs(
                idle,
                Instant::now() + idle - Duration::from_secs(1),
                &HashMap::new()
            )
            .iter()
            .all(|page| page.tab != used.tab),
            "the idle clock restarts when the run ends"
        );
        assert!(
            tabs.idle_agent_tabs(idle, Instant::now() + idle, &HashMap::new())
                .contains(&used),
            "a full idle time after the run's end, the tab is the janitor's"
        );
    }

    #[test]
    fn a_use_after_its_run_ended_keeps_nothing() {
        let tabs = TabRegistry::new();
        let tab = page(1, 101);
        let run = RunId::new();
        tabs.opened_by_agent(&tab, tabs.seen());
        tabs.run_started(run, Some(tab.tab));
        tabs.run_ended(run);
        tabs.used_in_run(tab.tab, run);
        assert!(
            tabs.lock().runs.is_empty(),
            "an op that settles after its run's end must not keep a tab for good"
        );
        let closed = page(2, 102);
        let later = RunId::new();
        tabs.run_started(later, Some(closed.tab));
        tabs.used_in_run(closed.tab, later);
        assert!(
            tabs.lock().runs.get(&later).is_some_and(HashSet::is_empty),
            "a tab with no entry is not in use"
        );
    }

    #[test]
    fn a_host_hold_keeps_a_tab_and_its_end_restarts_the_clock() {
        let tabs = TabRegistry::new();
        let idle = Duration::from_mins(5);
        let tab = page(1, 101);
        tabs.opened_by_agent(&tab, tabs.seen());
        let start = Instant::now();
        let held = HashMap::from([(tab.tab, TabHold::Now)]);
        let at_lock = start + Duration::from_mins(20);
        assert!(
            tabs.idle_agent_tabs(idle, at_lock, &held).is_empty(),
            "a lock or a fit keeps the tab, however long"
        );
        assert!(
            tabs.idle_agent_tabs(
                idle,
                at_lock + idle - Duration::from_secs(1),
                &HashMap::new()
            )
            .is_empty(),
            "seen held at a pass, the tab stays in use from that pass, even once the host forgot it"
        );
        let released = start + Duration::from_mins(30);
        let ended = HashMap::from([(tab.tab, TabHold::Until(released))]);
        assert!(
            tabs.idle_agent_tabs(idle, released + Duration::from_secs(1), &ended)
                .is_empty(),
            "not closed right after the lock is released or the fit restored"
        );
        assert_eq!(
            tabs.idle_agent_tabs(idle, released + idle, &ended),
            vec![tab],
            "a full idle time after the let-go"
        );
    }

    #[test]
    fn a_tab_used_after_the_sweep_chose_it_is_no_longer_idle() {
        let tabs = TabRegistry::new();
        let idle = Duration::from_mins(5);
        let tab = page(1, 101);
        tabs.opened_by_agent(&tab, tabs.seen());
        let later = Instant::now() + idle;
        assert_eq!(
            tabs.idle_agent_tabs(idle, later, &HashMap::new()),
            vec![tab.clone()]
        );
        assert!(tabs.still_idle(tab.tab, idle, later, None));
        assert!(
            !tabs.still_idle(tab.tab, idle, later, Some(TabHold::Now)),
            "a host fitted or locked it since"
        );
        let run = RunId::new();
        tabs.run_started(run, None);
        tabs.used_in_run(tab.tab, run);
        assert!(
            !tabs.still_idle(tab.tab, idle, later + idle, None),
            "a run in flight uses it now"
        );
        tabs.run_ended(run);
        assert!(
            !tabs.still_idle(tab.tab, idle, Instant::now() + Duration::from_secs(1), None),
            "and its clock restarted when the run ended"
        );
        assert!(
            !tabs.still_idle(TabId(999), idle, later, None),
            "a tab Neo does not know is never the janitor's"
        );
    }

    #[test]
    fn a_popup_of_an_agent_tab_is_the_agents() {
        let tabs = TabRegistry::new();
        let opener = page(1, 101);
        let popup = page(2, 102);
        let nested = page(3, 103);
        tabs.opened_by_agent(&opener, tabs.seen());
        tabs.popup(&popup, opener.tab, tabs.seen());
        tabs.popup(&nested, popup.tab, tabs.seen());
        let mut agent = tabs.agent_tabs();
        agent.sort_by_key(|tab| tab.0);
        assert_eq!(
            agent,
            vec![opener.tab, popup.tab, nested.tab],
            "a popup of an agent's tab, and its own popups, are the agent's to close"
        );

        let early = page(4, 104);
        let early_popup = page(5, 105);
        tabs.touched(&early, tabs.seen());
        tabs.popup(&early_popup, early.tab, tabs.seen());
        assert!(
            !tabs.agent_tabs().contains(&early_popup.tab),
            "the opener is not known to be the agent's yet"
        );
        tabs.opened_by_agent(&early, tabs.seen());
        assert!(
            tabs.agent_tabs().contains(&early_popup.tab),
            "a popup whose event came before its opener was marked follows it"
        );

        let owners = page(6, 106);
        let owners_popup = page(7, 107);
        tabs.touched(&owners, tabs.seen());
        tabs.popup(&owners_popup, owners.tab, tabs.seen());
        assert!(
            !tabs.agent_tabs().contains(&owners_popup.tab),
            "a popup of the owner's tab stays the owner's"
        );
    }

    fn flood(tabs: &TabRegistry, count: usize) {
        for index in 0..count {
            tabs.removing(TabId(10_000 + i64::try_from(index).unwrap_or_default()))();
        }
    }

    #[test]
    fn evicted_close_history_still_refuses_stale_admissions() {
        let tabs = TabRegistry::new();
        let tab = page(1, 101);
        let stale = tabs.seen();
        tabs.removing(tab.tab)();
        flood(&tabs, TOMBSTONES + 1);
        assert!(
            tabs.lock()
                .closed_at
                .iter()
                .all(|(closed, _)| *closed != tab.tab)
        );
        assert!(tabs.lock().forgotten > stale.0);
        tabs.touched(&tab, stale);
        tabs.opened_by_agent(&tab, stale);
        assert!(!tabs.is_open(tab.tab));
        assert!(
            tabs.slot::<Marker>(&tab).is_none(),
            "missing history must never recreate state"
        );
    }

    #[test]
    fn incomplete_history_refuses_even_live_tabs_until_a_fresh_read() {
        let tabs = TabRegistry::new();
        let tab = page(1, 101);
        let stale = tabs.seen();
        flood(&tabs, TOMBSTONES + 1);
        tabs.touched(&tab, stale);
        assert!(!tabs.is_open(tab.tab));
        tabs.touched(&tab, tabs.seen());
        assert!(tabs.is_open(tab.tab));
    }

    #[test]
    fn a_returning_tab_gets_fresh_slots_after_old_history_is_evicted() -> Result<(), Box<dyn Error>>
    {
        let tabs = TabRegistry::new();
        let tab = page(1, 101);
        tabs.opened_by_agent(&tab, tabs.seen());
        let old = tabs.slot::<Marker>(&tab).ok_or("registered slot missing")?;
        tabs.removing(tab.tab)();
        flood(&tabs, TOMBSTONES + 1);
        tabs.touched(&tab, tabs.seen());
        let new = tabs
            .slot::<Marker>(&tab)
            .ok_or("fresh admission slot missing")?;
        assert!(!Arc::ptr_eq(&old, &new));
        assert_eq!(
            tabs.lock().open.get(&tab.tab).map(|entry| entry.origin),
            Some(TabOrigin::Found)
        );
        tabs.removing(tab.tab)();
        assert!(tabs.slot::<Marker>(&tab).is_none());
        Ok(())
    }

    #[test]
    fn recent_tickets_ignore_other_tabs_and_include_the_low_water_boundary() {
        let tabs = TabRegistry::new();
        let tab = page(1, 101);
        let seen = tabs.seen();
        tabs.removing(TabId(102))();
        tabs.touched(&tab, seen);
        assert!(tabs.is_open(tab.tab));
        tabs.removing(tab.tab)();
        tabs.touched(&tab, tabs.seen());
        assert!(tabs.is_open(tab.tab));
        flood(&tabs, TOMBSTONES * 10);
        let boundary = Seen(tabs.lock().forgotten);
        let other = page(2, 103);
        tabs.touched(&other, boundary);
        assert!(
            tabs.is_open(other.tab),
            "the low-water boundary itself is complete history"
        );
        assert_eq!(tabs.lock().closed_at.len(), TOMBSTONES);
        let closed = tabs.seen().0;
        assert_eq!(tabs.lock().forgotten, closed - TOMBSTONES as u64);
    }
    #[test]
    fn dead_bindings_have_independent_lifetime_and_no_ownership_effect() {
        let tabs = TabRegistry::new();
        let page = page(1, 51);
        let target = TargetId("old".into());
        assert!(tabs.note_gone(page.tab, target.clone(), tabs.seen()));
        assert!(!tabs.is_open(page.tab));
        assert_eq!(tabs.owner(page.tab), None);
        tabs.touched(&page, tabs.seen());
        assert!(
            tabs.is_open(page.tab),
            "gone metadata cannot refuse an admitted touch"
        );
        tabs.opened_by_agent(&page, tabs.seen());
        assert_eq!(
            tabs.idle_agent_tabs(Duration::ZERO, Instant::now(), &HashMap::new()),
            vec![page.clone()]
        );
        assert_eq!(tabs.gone_target(page.tab), Some(target.clone()));
        tabs.forget_gone(page.tab);
        assert_eq!(tabs.gone_target(page.tab), None);
        assert!(tabs.note_gone(page.tab, target.clone(), tabs.seen()));
        let old = tabs.seen();
        tabs.removing(page.tab)();
        assert_eq!(tabs.gone_target(page.tab), None);
        assert!(
            !tabs.note_gone(page.tab, target, old),
            "detach wins against a late write"
        );
        assert_eq!(tabs.gone_target(page.tab), None);
        assert!(tabs.lock().gone.is_empty());
    }

    #[test]
    fn a_family_run_sees_only_its_familys_tabs_and_a_browser_run_every_tab_but_neos()
    -> Result<(), Box<dyn Error>> {
        let tabs = TabRegistry::new();
        let mine = TaskId::parse("host-s")?;
        let theirs = TaskId::parse("host-r")?;
        tabs.ensure_task(&mine)?;
        tabs.ensure_task(&theirs)?;
        let created = tabs.create_task_in(None, &mine)?;
        let (held, sibling, their_tab, no_task, neo, closed, taken) = (
            page(1, 101),
            page(2, 102),
            page(3, 103),
            page(4, 104),
            page(5, 105),
            page(6, 106),
            page(7, 107),
        );
        for tab in [&held, &sibling, &their_tab, &no_task, &neo, &closed, &taken] {
            tabs.touched(tab, tabs.seen());
        }
        tabs.own(&held, &mine)?;
        tabs.own(&sibling, &created.id)?;
        tabs.own(&their_tab, &theirs)?;
        tabs.own(&closed, &mine)?;
        tabs.own(&taken, &theirs)?;
        tabs.make_neo(neo.tab);
        tabs.removing(closed.tab)();
        // Announced only: no listing has registered these popups.
        let (popup, nested, their_popup) = (TabId(111), TabId(112), TabId(113));
        tabs.note_opener(popup, held.tab);
        tabs.note_opener(nested, popup);
        tabs.note_opener(their_popup, their_tab.tab);
        tabs.note_opener(taken.tab, held.tab);

        let family = RunScope::Family(mine.clone());
        for (tab, sight, why) in [
            (held.tab, Sight::Shown, "its host task holds it"),
            (
                sibling.tab,
                Sight::Shown,
                "a task created in its family holds it",
            ),
            (popup, Sight::Shown, "an unheld popup follows its opener"),
            (nested, Sight::Shown, "and so does that popup's own popup"),
            (their_tab.tab, Sight::Foreign, "another family holds it"),
            (
                their_popup,
                Sight::Foreign,
                "an unheld popup of another family's tab",
            ),
            (taken.tab, Sight::Foreign, "a holder outranks the opener"),
            (no_task.tab, Sight::Foreign, "no task holds it"),
            (
                closed.tab,
                Sight::Closed,
                "it closed while its family held it",
            ),
            (neo.tab, Sight::Reserved, "Neo's blank tab"),
            (
                TabId(999),
                Sight::Foreign,
                "a tab that never existed answers as another's",
            ),
        ] {
            assert_eq!(tabs.sight(&family, tab), sight, "{why}");
        }
        let other = RunScope::Family(theirs);
        assert_eq!(tabs.sight(&other, their_tab.tab), Sight::Shown);
        assert_eq!(
            tabs.sight(&other, closed.tab),
            Sight::Foreign,
            "another family's closed tab is not the run's to know about"
        );
        for tab in [
            held.tab,
            sibling.tab,
            popup,
            nested,
            their_tab.tab,
            their_popup,
            no_task.tab,
            closed.tab,
            TabId(999),
        ] {
            assert_eq!(
                tabs.sight(&RunScope::Browser, tab),
                Sight::Shown,
                "a browser-scoped run reaches every tab, as before task scope"
            );
        }
        assert_eq!(
            tabs.sight(&RunScope::Browser, neo.tab),
            Sight::Reserved,
            "but never Neo's blank tab"
        );
        Ok(())
    }

    #[test]
    fn an_announced_popup_is_its_openers_before_any_listing_and_moves_nothing()
    -> Result<(), Box<dyn Error>> {
        let tabs = TabRegistry::new();
        let task = TaskId::parse("host-s")?;
        tabs.ensure_task(&task)?;
        let family = RunScope::Family(task.clone());
        let opener = page(1, 101);
        tabs.opened_by_agent(&opener, tabs.seen());
        tabs.own(&opener, &task)?;
        let popup = TabId(102);
        tabs.note_opener(popup, opener.tab);
        assert_eq!(
            tabs.sight(&family, popup),
            Sight::Shown,
            "the family's from its announcement"
        );
        assert_eq!(
            tabs.owner(popup),
            None,
            "it joins the task only once a listing or an op registers it"
        );
        assert!(
            !tabs.agent_tabs().contains(&popup),
            "nor does it become the agent's before that"
        );
        tabs.popup(&PageRef { tab: popup }, opener.tab, tabs.seen());
        assert_eq!(
            tabs.owner(popup),
            Some(task.clone()),
            "the listing's notice joins it"
        );

        let late = TabId(103);
        tabs.note_opener(late, opener.tab);
        tabs.removing(opener.tab)();
        assert_eq!(
            tabs.sight(&family, late),
            Sight::Foreign,
            "an opener's close drops its unregistered popups, as it does registered ones"
        );
        tabs.note_opener(TabId(104), opener.tab);
        assert_eq!(
            tabs.sight(&family, TabId(104)),
            Sight::Foreign,
            "a closed opener holds nothing, so a late announcement naming it shows nothing"
        );
        let quick = TabId(108);
        tabs.note_opener(quick, popup);
        assert_eq!(
            tabs.sight(&family, quick),
            Sight::Shown,
            "a popup of the family's held popup"
        );
        tabs.forget_announced(quick);
        assert_eq!(
            tabs.sight(&family, quick),
            Sight::Foreign,
            "the watch's confirmed close drops a popup core never cached"
        );

        let other = page(5, 105);
        tabs.touched(&other, tabs.seen());
        tabs.own(&other, &task)?;
        tabs.note_opener(TabId(106), other.tab);
        tabs.forget_openers();
        assert_eq!(
            tabs.sight(&family, TabId(106)),
            Sight::Foreign,
            "a new socket forgets every announcement: a restarted browser numbers tabs anew"
        );
        let reused = page(1, 101);
        tabs.touched(&reused, tabs.seen());
        tabs.own(&reused, &task)?;
        tabs.note_opener(TabId(107), reused.tab);
        assert_eq!(
            tabs.sight(&family, TabId(107)),
            Sight::Shown,
            "a restarted browser's tab may reuse a closed tab's number and still be announced"
        );
        for index in 0..=ANNOUNCED {
            tabs.note_opener(TabId(1000 + i64::try_from(index)?), other.tab);
        }
        assert_eq!(
            tabs.sight(&family, TabId(1000)),
            Sight::Foreign,
            "the oldest announcement goes at the bound"
        );
        assert_eq!(
            tabs.sight(&family, TabId(1000 + i64::try_from(ANNOUNCED)?)),
            Sight::Shown
        );
        Ok(())
    }

    #[test]
    fn a_saved_book_comes_back_on_open_unheld_tabs_and_emits_nothing() -> Result<(), Box<dyn Error>>
    {
        let tabs = TabRegistry::new();
        let log = Arc::new(BookLog::default());
        let sink: Arc<BookLog> = Arc::clone(&log);
        assert!(tabs.set_book_sink(sink));
        let (first, second, neo, kept, sibling, newest) = (
            page(1, 101),
            page(2, 102),
            page(3, 103),
            page(4, 104),
            page(5, 105),
            page(6, 106),
        );
        for tab in [&first, &second, &neo, &kept, &sibling, &newest] {
            tabs.touched(tab, tabs.seen());
        }
        tabs.make_neo(neo.tab);
        let theirs = TaskId::parse("host-r")?;
        tabs.ensure_task(&theirs)?;
        tabs.own(&kept, &theirs)?;
        log.take();

        let mine = TaskId::parse("host-s")?;
        let created = TaskId::parse("made-0001")?;
        tabs.restore_book(vec![
            SavedTask {
                id: mine.clone(),
                family: mine.clone(),
                origin: TaskOrigin::Host,
                tabs: vec![second.tab, first.tab, neo.tab, kept.tab, TabId(199)],
                page: Some(second.tab),
            },
            SavedTask {
                id: created.clone(),
                family: mine.clone(),
                origin: TaskOrigin::Agent,
                tabs: vec![sibling.tab, newest.tab],
                page: Some(kept.tab),
            },
        ])?;

        assert_eq!(
            tabs.task(&mine)?.tabs,
            vec![second.tab, first.tab],
            "open tabs no task holds join in saved order; never Neo's blank tab or an unknown one"
        );
        assert_eq!(
            tabs.task_page(&mine),
            TaskPage::Tab(second),
            "its saved page"
        );
        assert_eq!(
            tabs.owner(neo.tab),
            None,
            "Neo's blank tab, taken before the restore, is never held after it"
        );
        assert_eq!(
            tabs.owner(kept.tab),
            Some(theirs),
            "a held tab keeps its holder"
        );
        let record = tabs.task(&created)?;
        assert_eq!(
            (&record.family, record.origin),
            (&mine, TaskOrigin::Agent),
            "a created task keeps its saved family and origin"
        );
        assert_eq!(
            tabs.holder(sibling.tab),
            Some(Holder {
                task: created.clone(),
                family: mine.clone(),
            })
        );
        assert_eq!(
            tabs.task_page(&created),
            TaskPage::Tab(newest),
            "a saved page that is not the task's own leaves the newest as its page"
        );
        assert_eq!(
            tabs.sight(&RunScope::Family(mine), sibling.tab),
            Sight::Shown,
            "the family sees its restored tabs"
        );
        assert!(
            log.take().is_empty(),
            "a restore emits nothing: the saved record already holds it"
        );
        Ok(())
    }
}
