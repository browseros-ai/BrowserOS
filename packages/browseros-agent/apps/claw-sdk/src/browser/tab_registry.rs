//! Per-tab facts, in one place.

use std::any::{Any, TypeId};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use browseros_core::{PageId, TabId};
use browseros_trace::TaskId;
use tokio::sync::broadcast;
use tokio::time::{Duration, Instant};

use super::PageRef;

/// How many tasks' last tabs memory keeps. A task forgotten here falls back to the trace store's
/// `task_last_tab`, so the bound costs a lookup, never a wrong tab.
const TASKS_REMEMBERED: usize = 1024;
/// How many closed pages stay tombstoned. A late `touched` follows its close by one await at
/// most, so the newest few hundred closes are all that can race.
const TOMBSTONES: usize = 256;
/// Tab closes a slow subscriber may fall behind by before it sees `Lagged`.
const CLOSED_BACKLOG: usize = 256;

/// What Neo keeps in memory per tab: who opened it, when an op last touched it, its feature
/// state, and which tab each task used last.
///
/// One owner, one lock, never held across an await. There is one way out: core's detach hook
/// calls [`TabRegistry::closed`], and a tab's entry and its slots leave together, so no
/// feature's state outlives its tab. `tab_watch` makes that hook fire promptly when someone
/// else closes the tab.
pub(crate) struct TabRegistry {
    state: Mutex<TabState>,
    /// Tab closes, for state a feature keeps per tab outside this registry. Only tabs an op
    /// touched are reported; sdk-traces' privacy and live views hear of every close from
    /// `tab_watch`, through the host API's tab feed, instead.
    closed: broadcast::Sender<TabId>,
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
    open: HashMap<PageId, TabEntry>,
    /// Each task's last tab, bounded by [`TASKS_REMEMBERED`] (the oldest is forgotten first).
    last_by_task: HashMap<TaskId, LastTab>,
    /// The last tab any script op touched, for runs without a task.
    last_any: Option<TabId>,
    /// Recently closed pages. Core never reuses a page id within a process, so a late
    /// `touched` (resolved from core's cache just before the close) cannot re-create a dead
    /// entry whose slots nothing would ever drop.
    tombstones: VecDeque<PageId>,
}

struct LastTab {
    tab: TabId,
    at: Instant,
}

struct TabEntry {
    tab: TabId,
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
            closed: broadcast::channel(CLOSED_BACKLOG).0,
        }
    }

    /// An op targeted `page`: its idle clock restarts and it becomes the task's last tab.
    pub(crate) fn touched(&self, page: &PageRef, task: Option<&TaskId>) {
        let now = Instant::now();
        let mut state = self.lock();
        if state.is_tombstoned(&page.id) {
            return;
        }
        state.entry(page, now).touched = now;
        state.remember(page.tab, task, now);
    }

    /// Neo opened `page` for an agent, so the idle-tab janitor may close it later.
    pub(crate) fn opened_by_agent(&self, page: &PageRef, task: Option<&TaskId>) {
        let now = Instant::now();
        let mut state = self.lock();
        if state.is_tombstoned(&page.id) {
            return;
        }
        let entry = state.entry(page, now);
        entry.origin = TabOrigin::Agent;
        entry.touched = now;
        state.remember(page.tab, task, now);
    }

    /// Core's detach hook: the tab is gone. Sync, because the hook runs on core's task right
    /// after the page left core's registry.
    pub(crate) fn closed(&self, page: &PageId) {
        let entry = {
            let mut state = self.lock();
            if !state.is_tombstoned(page) {
                state.tombstones.push_back(page.clone());
                if state.tombstones.len() > TOMBSTONES {
                    state.tombstones.pop_front();
                }
            }
            state.open.remove(page)
        };
        // Dropped after the lock is released: a slot's own Drop may do anything, including
        // asking this registry about another tab.
        if let Some(entry) = entry {
            // No subscriber is not an error.
            let _ = self.closed.send(entry.tab);
        }
    }

    /// The tab id of a page an op touched, without asking core.
    #[must_use]
    pub(crate) fn tab_of(&self, page: &PageId) -> Option<TabId> {
        self.lock().open.get(page).map(|entry| entry.tab)
    }

    /// Per-tab feature state (sdk-playwright's facade, sdk-privacy's journal), created on first
    /// use. `None` once the tab closed. `T::default` runs under the registry's lock, so it must
    /// not call back into the registry.
    #[must_use]
    pub(crate) fn slot<T: Default + Send + Sync + 'static>(
        &self,
        page: &PageRef,
    ) -> Option<Arc<T>> {
        let mut state = self.lock();
        if state.is_tombstoned(&page.id) {
            return None;
        }
        let entry = state.entry(page, Instant::now());
        let slot = entry
            .slots
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Arc::new(T::default()));
        // Keyed by `TypeId::of::<T>()`, so the downcast always succeeds.
        Arc::clone(slot).downcast::<T>().ok()
    }

    /// The `page` default from memory: the task's last tab while it is still open, or, for a run
    /// without a task, the last tab any script op touched. With a task this never answers
    /// another task's tab: a task Neo forgot asks the trace store, and a new task gets a fresh
    /// tab, so parallel tasks never share one by accident.
    #[must_use]
    pub(crate) fn default_page(&self, task: Option<&TaskId>) -> Option<PageRef> {
        let state = self.lock();
        let tab = match task {
            Some(task) => state.last_by_task.get(task).map(|last| last.tab)?,
            None => state.last_any?,
        };
        state
            .open
            .iter()
            .find(|(_, entry)| entry.tab == tab)
            .map(|(id, entry)| PageRef {
                id: id.clone(),
                tab: entry.tab,
            })
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
            .map(|(id, entry)| PageRef {
                id: id.clone(),
                tab: entry.tab,
            })
            .collect()
    }

    /// Tab closes from now on. Only tabs an op touched are reported: the registry learns a
    /// tab's id from its first op.
    #[must_use]
    pub(crate) fn subscribe_closed(&self) -> broadcast::Receiver<TabId> {
        self.closed.subscribe()
    }

    fn lock(&self) -> MutexGuard<'_, TabState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl TabState {
    fn is_tombstoned(&self, page: &PageId) -> bool {
        self.tombstones.contains(page)
    }

    fn entry(&mut self, page: &PageRef, now: Instant) -> &mut TabEntry {
        self.open
            .entry(page.id.clone())
            .or_insert_with(|| TabEntry {
                tab: page.tab,
                origin: TabOrigin::Found,
                touched: now,
                slots: HashMap::new(),
            })
    }

    fn remember(&mut self, tab: TabId, task: Option<&TaskId>, now: Instant) {
        self.last_any = Some(tab);
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

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use browseros_core::{PageId, TabId};
    use browseros_trace::TaskId;
    use tokio::time::{Duration, Instant};

    use super::{TASKS_REMEMBERED, TabRegistry};
    use crate::browser::PageRef;

    #[derive(Default)]
    struct Marker;

    fn page(id: u32, tab: i64) -> PageRef {
        PageRef {
            id: PageId(id),
            tab: TabId(tab),
        }
    }

    #[test]
    fn closing_a_tab_drops_its_slots() {
        let tabs = TabRegistry::new();
        let tab = page(1, 101);
        tabs.touched(&tab, None);
        let held = tabs.slot::<Marker>(&tab);
        assert!(held.is_some(), "a touched tab gets a slot");
        tabs.closed(&tab.id);
        assert_eq!(
            held.as_ref().map(Arc::strong_count),
            Some(1),
            "the registry let go of the closed tab's slot"
        );
        assert!(
            tabs.slot::<Marker>(&tab).is_none(),
            "a closed tab has no slots"
        );
        assert_eq!(tabs.tab_of(&tab.id), None, "a closed tab has no entry");
    }

    #[test]
    fn a_late_touch_cannot_revive_a_closed_tab() {
        let tabs = TabRegistry::new();
        let tab = page(1, 101);
        tabs.touched(&tab, None);
        tabs.closed(&tab.id);
        tabs.touched(&tab, None);
        tabs.opened_by_agent(&tab, None);
        assert_eq!(
            tabs.tab_of(&tab.id),
            None,
            "page ids are never reused, so a touch after the close is stale"
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
        tabs.touched(&tab, Some(&first));
        assert_eq!(
            tabs.default_page(Some(&first)),
            Some(tab.clone()),
            "the task's own last tab"
        );
        assert_eq!(
            tabs.default_page(Some(&second)),
            None,
            "a new task gets no one else's tab"
        );
        assert_eq!(
            tabs.default_page(None),
            Some(tab.clone()),
            "a run without a task gets the last tab"
        );
        tabs.closed(&tab.id);
        assert_eq!(
            tabs.default_page(Some(&first)),
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
        tabs.touched(&tab, Some(&oldest));
        for index in 1..=TASKS_REMEMBERED {
            tabs.touched(&tab, Some(&TaskId::parse(&format!("task-{index}"))?));
        }
        assert_eq!(
            tabs.default_page(Some(&oldest)),
            None,
            "past the bound the oldest task goes"
        );
        let newest = TaskId::parse(&format!("task-{TASKS_REMEMBERED}"))?;
        assert_eq!(
            tabs.default_page(Some(&newest)),
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
        tabs.opened_by_agent(&agent, None);
        tabs.touched(&owner, None);
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
        tabs.touched(&agent, None);
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

    #[test]
    fn closes_are_broadcast_with_the_tab_id() -> Result<(), Box<dyn Error>> {
        let tabs = TabRegistry::new();
        let tab = page(1, 101);
        let mut closes = tabs.subscribe_closed();
        tabs.touched(&tab, None);
        tabs.closed(&tab.id);
        assert_eq!(
            closes.try_recv()?,
            tab.tab,
            "subscribers learn the closed tab's id"
        );
        tabs.closed(&tab.id);
        assert!(
            closes.try_recv().is_err(),
            "a second detach of the same page says nothing"
        );
        Ok(())
    }
}
