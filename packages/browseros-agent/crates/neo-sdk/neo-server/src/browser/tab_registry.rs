//! Per-tab facts, in one place.

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use browseros_core::{TabId, TargetId};
use neo_takeover::TabHold;
use neo_trace::{RunId, TaskId};
use tokio::time::{Duration, Instant};

use super::task_book::TaskBook;
use super::{PageRef, TaskError, TaskRecord};

/// How many tasks' last tabs memory keeps. A task forgotten here falls back to the trace store's
/// `task_last_tab`, so the bound costs a lookup, never a wrong tab.
const TASKS_REMEMBERED: usize = 1024;
/// Retained close history. Evicted history advances `forgotten`, so bounded memory never
/// turns an old admission into proof that a tab is still open.
const TOMBSTONES: usize = 256;

/// What Neo keeps in memory per tab: who opened it, when it was last in use (its idle clock),
/// which runs in flight used it, its feature state, and which tab each task used last.
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
    /// Each task's last tab, bounded by [`TASKS_REMEMBERED`] (the oldest is forgotten first).
    last_by_task: HashMap<TaskId, LastTab>,
    tasks: TaskBook,
    /// Popup relationships survive an opener whose creating op has not assigned its task yet.
    popups: HashMap<TabId, TabId>,
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

struct LastTab {
    tab: TabId,
    at: Instant,
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

    /// An op targeted `page`: its idle clock restarts and it becomes the task's last tab.
    pub(crate) fn touched(&self, page: &PageRef, seen: Seen, task: Option<&TaskId>) {
        let now = Instant::now();
        let mut state = self.lock();
        if !state.proves_open(page.tab, seen) {
            return;
        }
        state.entry(page, now).touched = now;
        state.tasks.touch(page.tab);
        state.remember(page.tab, task, now);
    }

    /// Neo opened `page` for an agent, so the idle-tab janitor may close it later.
    pub(crate) fn opened_by_agent(&self, page: &PageRef, seen: Seen, task: Option<&TaskId>) {
        let now = Instant::now();
        let mut state = self.lock();
        if !state.proves_open(page.tab, seen) {
            return;
        }
        let entry = state.entry(page, now);
        entry.origin = TabOrigin::Agent;
        entry.touched = now;
        state.tasks.touch(page.tab);
        state.remember(page.tab, task, now);
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
            state.tasks.detach(tab);
            state
                .popups
                .retain(|popup, opener| *popup != tab && *opener != tab);
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
    /// Neo remembers owns it. The trace's own task links are the caller's to check, since the
    /// registry starts empty after a server restart.
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

    /// The host task's last touched tab, if still open. No global default is kept.
    #[must_use]
    pub(crate) fn default_page(&self, task: &TaskId) -> Option<PageRef> {
        let state = self.lock();
        let tab = state.last_by_task.get(task)?.tab;
        state.open.contains_key(&tab).then_some(PageRef { tab })
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

    /// Assigns a registered tab synchronously after its child opener returns. A close that
    /// already removed its entry wins, so assignment cannot revive a closed tab.
    pub(crate) fn own(&self, page: &PageRef, id: &TaskId) -> Result<(), TaskError> {
        let mut state = self.lock();
        if state.open.contains_key(&page.tab) {
            state.tasks.attach(page.tab, id)?;
            state.inherit_popups();
        }
        Ok(())
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
    /// with a lock or a fit; its idle clock restarts when the last of those ends. A hold is kept
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
                // The owner was read under this lock, so its task still exists.
                let _ = self.tasks.attach(tab, &id);
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

    fn remember(&mut self, tab: TabId, task: Option<&TaskId>, now: Instant) {
        let Some(task) = task else {
            return;
        };
        if !self.last_by_task.contains_key(task) && self.last_by_task.len() >= TASKS_REMEMBERED {
            let oldest = self
                .last_by_task
                .iter()
                .min_by_key(|(_, last)| last.at)
                .map(|(task, _)| task.clone());
            if let Some(oldest) = oldest {
                self.last_by_task.remove(&oldest);
            }
        }
        self.last_by_task
            .insert(task.clone(), LastTab { tab, at: now });
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

    use super::{Seen, TASKS_REMEMBERED, TOMBSTONES, TabOrigin, TabRegistry};
    use crate::browser::PageRef;

    #[derive(Default)]
    struct Marker;

    fn page(_id: u32, tab: i64) -> PageRef {
        PageRef { tab: TabId(tab) }
    }

    #[test]
    fn closing_a_tab_drops_its_slots() {
        let tabs = TabRegistry::new();
        let tab = page(1, 101);
        tabs.touched(&tab, tabs.seen(), None);
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
        tabs.touched(&tab, tabs.seen(), None);
        let seen = tabs.seen();
        tabs.removing(tab.tab)();
        tabs.touched(&tab, seen, None);
        tabs.opened_by_agent(&tab, seen, None);
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
    fn a_task_never_inherits_another_tasks_tab() -> Result<(), Box<dyn Error>> {
        let tabs = TabRegistry::new();
        let first = TaskId::parse("task-1")?;
        let second = TaskId::parse("task-2")?;
        let tab = page(1, 101);
        tabs.touched(&tab, tabs.seen(), Some(&first));
        assert_eq!(
            tabs.default_page(&first),
            Some(tab.clone()),
            "the task's own last tab"
        );
        assert_eq!(
            tabs.default_page(&second),
            None,
            "a new task gets no one else's tab"
        );
        tabs.removing(tab.tab)();
        assert_eq!(
            tabs.default_page(&first),
            None,
            "a closed tab is no default"
        );
        Ok(())
    }

    #[test]
    fn the_oldest_task_is_forgotten_first() -> Result<(), Box<dyn Error>> {
        let tabs = TabRegistry::new();
        let tab = page(1, 101);
        let oldest = TaskId::parse("task-0")?;
        tabs.touched(&tab, tabs.seen(), Some(&oldest));
        for index in 1..=TASKS_REMEMBERED {
            tabs.touched(
                &tab,
                tabs.seen(),
                Some(&TaskId::parse(&format!("task-{index}"))?),
            );
        }
        assert_eq!(
            tabs.default_page(&oldest),
            None,
            "past the bound the oldest task goes"
        );
        let newest = TaskId::parse(&format!("task-{TASKS_REMEMBERED}"))?;
        assert_eq!(
            tabs.default_page(&newest),
            Some(tab),
            "the newest task stays"
        );
        Ok(())
    }

    #[test]
    fn only_idle_agent_tabs_are_offered_to_the_janitor() {
        let tabs = TabRegistry::new();
        let agent = page(1, 101);
        let owner = page(2, 102);
        tabs.opened_by_agent(&agent, tabs.seen(), None);
        tabs.touched(&owner, tabs.seen(), None);
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
        tabs.touched(&agent, tabs.seen(), None);
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
        tabs.opened_by_agent(&used, tabs.seen(), None);
        tabs.used_in_run(used.tab, run);
        tabs.opened_by_agent(&other, tabs.seen(), None);
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
        tabs.opened_by_agent(&tab, tabs.seen(), None);
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
        tabs.opened_by_agent(&tab, tabs.seen(), None);
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
        tabs.opened_by_agent(&tab, tabs.seen(), None);
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
        tabs.opened_by_agent(&opener, tabs.seen(), None);
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
        tabs.touched(&early, tabs.seen(), None);
        tabs.popup(&early_popup, early.tab, tabs.seen());
        assert!(
            !tabs.agent_tabs().contains(&early_popup.tab),
            "the opener is not known to be the agent's yet"
        );
        tabs.opened_by_agent(&early, tabs.seen(), None);
        assert!(
            tabs.agent_tabs().contains(&early_popup.tab),
            "a popup whose event came before its opener was marked follows it"
        );

        let owners = page(6, 106);
        let owners_popup = page(7, 107);
        tabs.touched(&owners, tabs.seen(), None);
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
        tabs.touched(&tab, stale, None);
        tabs.opened_by_agent(&tab, stale, None);
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
        tabs.touched(&tab, stale, None);
        assert!(!tabs.is_open(tab.tab));
        tabs.touched(&tab, tabs.seen(), None);
        assert!(tabs.is_open(tab.tab));
    }

    #[test]
    fn a_returning_tab_gets_fresh_slots_after_old_history_is_evicted() -> Result<(), Box<dyn Error>>
    {
        let tabs = TabRegistry::new();
        let tab = page(1, 101);
        tabs.opened_by_agent(&tab, tabs.seen(), None);
        let old = tabs.slot::<Marker>(&tab).ok_or("registered slot missing")?;
        tabs.removing(tab.tab)();
        flood(&tabs, TOMBSTONES + 1);
        tabs.touched(&tab, tabs.seen(), None);
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
        tabs.touched(&tab, seen, None);
        assert!(tabs.is_open(tab.tab));
        tabs.removing(tab.tab)();
        tabs.touched(&tab, tabs.seen(), None);
        assert!(tabs.is_open(tab.tab));
        flood(&tabs, TOMBSTONES * 10);
        let boundary = Seen(tabs.lock().forgotten);
        let other = page(2, 103);
        tabs.touched(&other, boundary, None);
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
        tabs.touched(&page, tabs.seen(), None);
        assert!(
            tabs.is_open(page.tab),
            "gone metadata cannot refuse an admitted touch"
        );
        tabs.opened_by_agent(&page, tabs.seen(), None);
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
}
