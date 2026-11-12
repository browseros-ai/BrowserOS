//! Per-tab facts, in one place.

use std::any::{Any, TypeId};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use browseros_core::{TabId, TargetId};
use neo_trace::TaskId;
use tokio::time::{Duration, Instant};

use super::task_book::TaskBook;
use super::{PageRef, TaskError, TaskRecord};

/// How many tasks' last tabs memory keeps. A task forgotten here falls back to the trace store's
/// `task_last_tab`, so the bound costs a lookup, never a wrong tab.
const TASKS_REMEMBERED: usize = 1024;
/// Retained close history. Evicted history advances `forgotten`, so bounded memory never
/// turns an old admission into proof that a tab is still open.
const TOMBSTONES: usize = 256;

/// What Neo keeps in memory per tab: who opened it, when an op last touched it, its feature
/// state, and which tab each task used last.
///
/// One owner, one lock, never held across an await. There is one way out: core's detach hook
/// calls [`TabRegistry::closed`], and a tab's entry and its slots leave together, so no
/// feature's state outlives its tab. `tab_watch` makes that hook fire promptly when someone
/// else closes the tab.
pub(crate) struct TabRegistry {
    state: Mutex<TabState>,
}

/// Who opened a tab. Core has no public `is_agent_created`, so Neo's own page opener marks its
/// tabs here. Popups and the owner's tabs stay `Found`, and the idle-tab janitor never closes
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabOrigin {
    /// Opened for an agent (`context.newPage`, or the lazy default page) by this process.
    Agent,
    /// Anything else: the owner's tabs, popups, tabs from before a Neo restart.
    Found,
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
    }

    /// A successful task close forgets it only once every owned tab is gone.
    pub(crate) fn forget_task(&self, id: &TaskId) -> bool {
        self.lock().tasks.remove_empty(id)
    }

    /// Agent-opened tabs that no op touched for `idle`, for the janitor.
    #[must_use]
    pub(crate) fn idle_agent_tabs(&self, idle: Duration, now: Instant) -> Vec<PageRef> {
        self.lock()
            .open
            .iter()
            .filter(|(_, entry)| {
                entry.origin == TabOrigin::Agent
                    && now.saturating_duration_since(entry.touched) >= idle
            })
            .map(|(tab, _)| PageRef { tab: *tab })
            .collect()
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

/// A point in the registry's close history, valid only for a read started after it was taken.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Seen(u64);
#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use browseros_core::{TabId, TargetId};
    use neo_trace::TaskId;
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
            tabs.idle_agent_tabs(idle, later),
            vec![agent.clone()],
            "the owner's tab is never the janitor's, however idle"
        );
        assert!(
            tabs.idle_agent_tabs(idle, Instant::now()).is_empty(),
            "a tab touched just now is not idle"
        );
        tabs.touched(&agent, tabs.seen(), None);
        assert!(
            tabs.idle_agent_tabs(idle, later).is_empty(),
            "a touch restarts the idle clock"
        );
        assert_eq!(
            tabs.idle_agent_tabs(idle, Instant::now() + idle),
            vec![agent],
            "a touch keeps an agent tab the janitor's"
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
            tabs.idle_agent_tabs(Duration::ZERO, Instant::now()),
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
