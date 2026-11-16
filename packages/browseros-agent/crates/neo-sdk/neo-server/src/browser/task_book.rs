//! In-memory tasks, their families and their ordered tab ownership, under the tab registry's
//! lock.
//!
//! A task's family decides what a task-scoped run may reach: a run bound to a task sees the tabs
//! that any task of the task's family holds ([`super::RunScope`]). A family is named by its root,
//! a task a host named, and holds the tasks created in its runs, fixed when each task is made.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use browseros_core::TabId;
use neo_trace::{IdError, RunId, TaskId};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::book_change::{BookChange, BookSink, SavedTask};

const TASKS: usize = 4096;

/// Closes remembered, so a family's own closed tab answers `page_closed` rather than "not your
/// task's". An evicted close answers as a tab outside the family: still refused, less precisely.
const CLOSED: usize = 1024;

/// Where a task came from. It decides who may pick it: only runs of its own family pick a host's
/// task, while any run may pick a task an agent created.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TaskOrigin {
    /// A host named it (`_meta.taskId`, [`TaskBook::ensure`]).
    Host,
    /// A script created it (`neo.createTask`, [`TaskBook::create`]).
    Agent,
}

/// A task's current tabs, in the order they joined it.
#[derive(Clone, Debug)]
pub(crate) struct TaskRecord {
    pub(crate) id: TaskId,
    pub(crate) name: Option<String>,
    pub(crate) tabs: Vec<TabId>,
    /// The task's own tab its own script last acted on ([`TaskBook::acted`]), or the tab that
    /// joined it last. Never another task's tab, and never moved by Neo's own operations.
    pub(crate) last_used_tab: Option<TabId>,
    /// Serializes opens and closes. A held record also pins an empty task during an open.
    pub(crate) turn: Arc<Mutex<()>>,
    /// The family's root, fixed when the task is made: itself for a host's task, or for one
    /// created in a run with no host task; the family of the creating run's host task otherwise.
    /// It never changes, so a [`Holder`]'s copy never goes stale.
    pub(crate) family: TaskId,
    /// Only ever moves from `Agent` to `Host` (a host names a task an agent created), never back.
    pub(crate) origin: TaskOrigin,
}

/// Who holds a tab now: one per held tab, and what a run's sight reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Holder {
    /// The task the tab belongs to.
    pub(crate) task: TaskId,
    /// That task's family root, copied from its record when the tab joins, so a sight check is
    /// one lookup.
    pub(crate) family: TaskId,
}

/// A task operation refused before changing ownership.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TaskError {
    #[error(
        "no task `{0}` is open in Neo (tasks live in memory and end with `task.close()`; a restart keeps only tasks with open tabs). Start one with `neo.createTask(name)`, or find your tabs in `context.pages()` and read `page.taskId`."
    )]
    NotFound(TaskId),
    #[error(
        "Neo already has 4096 tasks with open tabs; close a task with `task.close()` before starting another"
    )]
    Full,
    #[error("could not generate a task id: {0}")]
    InvalidId(#[from] IdError),
}

impl TaskRecord {
    /// The tab built-in `page` takes in this task: the last-used one, else the newest, among
    /// those `usable` admits. The lazy page's open (`Dispatcher::open_default_page`) and
    /// `neo.task`'s pick both choose through here, so a pick binds `page` to the very tab its
    /// first action would have taken.
    pub(crate) fn page_tab(&self, mut usable: impl FnMut(TabId) -> bool) -> Option<TabId> {
        self.last_used_tab
            .into_iter()
            .chain(self.tabs.iter().rev().copied())
            .find(|tab| usable(*tab))
    }
}

/// A tab that closed while a task held it, and the holder's family then.
struct ClosedTab {
    tab: TabId,
    family: TaskId,
}

/// Updated with the tab registry, so a close removes its holder and task membership together.
/// Every change a restart must see also goes to the sink, in book order ([`BookChange`]).
#[derive(Default)]
pub(super) struct TaskBook {
    tasks: VecDeque<TaskRecord>,
    holders: HashMap<TabId, Holder>,
    /// Oldest first, bounded by [`CLOSED`].
    closed: VecDeque<ClosedTab>,
    sink: Option<Arc<dyn BookSink>>,
}

impl TaskBook {
    /// An agent's task with no host task behind its run: the root of its own family.
    pub(super) fn create(&mut self, name: Option<String>) -> Result<TaskRecord, TaskError> {
        self.make(name, None)
    }

    /// An agent's task created in a run whose host task is of `family`: it joins that family.
    pub(super) fn create_in(
        &mut self,
        name: Option<String>,
        family: &TaskId,
    ) -> Result<TaskRecord, TaskError> {
        self.make(name, Some(family))
    }

    fn make(
        &mut self,
        name: Option<String>,
        family: Option<&TaskId>,
    ) -> Result<TaskRecord, TaskError> {
        let slug = slug(name.as_deref().unwrap_or_default());
        loop {
            // UUIDv7's low bits are random; its timestamp and counter do not enter task ids.
            let suffix = RunId::new().uuid().as_u128() & 0xffff;
            let id = TaskId::parse(&format!("{slug}-{suffix:04x}"))?;
            if self.tasks.iter().any(|task| task.id == id) {
                continue;
            }
            self.make_room()?;
            let task = TaskRecord {
                family: family.unwrap_or(&id).clone(),
                id,
                name,
                tabs: Vec::new(),
                last_used_tab: None,
                turn: Arc::default(),
                origin: TaskOrigin::Agent,
            };
            self.tasks.push_back(task.clone());
            return Ok(task);
        }
    }

    /// A host named `id`: a new task is a host's and the root of its own family. A task an agent
    /// created becomes the host's, and keeps its family.
    pub(super) fn ensure(&mut self, id: &TaskId) -> Result<(), TaskError> {
        if let Some(task) = self.tasks.iter_mut().find(|task| task.id == *id) {
            if task.origin == TaskOrigin::Agent {
                task.origin = TaskOrigin::Host;
                self.emit_held(id);
            }
            return Ok(());
        }
        self.make_room()?;
        self.tasks.push_back(TaskRecord {
            id: id.clone(),
            name: None,
            tabs: Vec::new(),
            last_used_tab: None,
            turn: Arc::default(),
            family: id.clone(),
            origin: TaskOrigin::Host,
        });
        Ok(())
    }

    pub(super) fn get(&self, id: &TaskId) -> Result<&TaskRecord, TaskError> {
        self.tasks
            .iter()
            .find(|task| task.id == *id)
            .ok_or_else(|| TaskError::NotFound(id.clone()))
    }

    /// The task holding `tab`.
    pub(super) fn owner(&self, tab: TabId) -> Option<&TaskId> {
        self.holder(tab).map(|holder| &holder.task)
    }

    /// Who holds `tab`: its task and that task's family.
    pub(super) fn holder(&self, tab: TabId) -> Option<&Holder> {
        self.holders.get(&tab)
    }

    /// The task opened `tab` (the lazy page, `context.newPage`, `task.newPage`): it joins the
    /// task and becomes its page, as the script that opened it is working in it.
    pub(super) fn attach(&mut self, tab: TabId, id: &TaskId) -> Result<(), TaskError> {
        if self.join(tab, id)? {
            self.acted(tab, id);
        }
        Ok(())
    }

    /// `tab` joins the task without becoming its page: a popup one of its tabs opened, which the
    /// task's page moves to only once its own script acts on it ([`TaskBook::acted`]). True when
    /// it joined now; a tab that has a holder keeps it, since a touch or a repeated popup notice
    /// must never move a tab between tasks.
    pub(super) fn join(&mut self, tab: TabId, id: &TaskId) -> Result<bool, TaskError> {
        let task = self
            .tasks
            .iter_mut()
            .find(|task| task.id == *id)
            .ok_or_else(|| TaskError::NotFound(id.clone()))?;
        if self.holders.contains_key(&tab) {
            return Ok(false);
        }
        task.tabs.push(tab);
        let origin = task.origin;
        self.holders.insert(
            tab,
            Holder {
                task: id.clone(),
                family: task.family.clone(),
            },
        );
        if let Some(holder) = self.holders.get(&tab) {
            self.emit(|| BookChange::Held {
                tab,
                holder: holder.clone(),
                origin,
            });
        }
        Ok(true)
    }

    /// `task`'s own script acted on `tab`. Only the holder's act counts: a script that reaches
    /// another task's tab, a task-less tab or the browser's New Tab through `context.pages()`
    /// changes no task's page, its own included.
    pub(super) fn acted(&mut self, tab: TabId, task: &TaskId) {
        if self.owner(tab) != Some(task) {
            return;
        }
        let Some(record) = self.tasks.iter_mut().find(|record| record.id == *task) else {
            return;
        };
        if record.last_used_tab != Some(tab) {
            record.last_used_tab = Some(tab);
            self.emit(|| BookChange::Page { tab });
        }
    }

    /// `tab` leaves its task without closing: Neo's blank tab took it.
    pub(super) fn detach(&mut self, tab: TabId) {
        self.release(tab);
    }

    /// `tab` closed. Its family remembers the close, so that family's runs are told the tab
    /// closed rather than that it is not theirs.
    pub(super) fn closed(&mut self, tab: TabId) {
        let Some(holder) = self.release(tab) else {
            return;
        };
        self.closed.push_back(ClosedTab {
            tab,
            family: holder.family,
        });
        if self.closed.len() > CLOSED {
            self.closed.pop_front();
        }
    }

    /// Whether `tab` closed while a task of `family` held it, within the closes remembered.
    pub(super) fn was_closed(&self, tab: TabId, family: &TaskId) -> bool {
        self.closed
            .iter()
            .any(|closed| closed.tab == tab && closed.family == *family)
    }

    fn release(&mut self, tab: TabId) -> Option<Holder> {
        let holder = self.holders.remove(&tab)?;
        if let Some(task) = self.tasks.iter_mut().find(|task| task.id == holder.task) {
            task.tabs.retain(|held| *held != tab);
            if task.last_used_tab == Some(tab) {
                task.last_used_tab = None;
            }
        }
        self.emit(|| BookChange::Gone { tab });
        Some(holder)
    }

    pub(super) fn remove_empty(&mut self, id: &TaskId) -> bool {
        if let Some(index) = self
            .tasks
            .iter()
            .position(|task| task.id == *id && task.tabs.is_empty())
        {
            self.tasks.remove(index);
            return true;
        }
        false
    }

    /// Rebuilds one saved task: the record as saved unless this book already has it, then each
    /// of its tabs that `open` admits and no task holds, in saved order, and its page among
    /// them. Emits nothing: the saved record already holds what this restores.
    pub(super) fn restore_saved(
        &mut self,
        saved: SavedTask,
        mut open: impl FnMut(TabId) -> bool,
    ) -> Result<(), TaskError> {
        let SavedTask {
            id,
            family,
            origin,
            tabs,
            page,
        } = saved;
        if self.get(&id).is_err() {
            self.make_room()?;
            self.tasks.push_back(TaskRecord {
                id: id.clone(),
                name: None,
                tabs: Vec::new(),
                last_used_tab: None,
                turn: Arc::default(),
                family,
                origin,
            });
        }
        let task = self
            .tasks
            .iter_mut()
            .find(|task| task.id == id)
            .ok_or_else(|| TaskError::NotFound(id.clone()))?;
        for tab in tabs {
            if !open(tab) || self.holders.contains_key(&tab) {
                continue;
            }
            task.tabs.push(tab);
            self.holders.insert(
                tab,
                Holder {
                    task: id.clone(),
                    family: task.family.clone(),
                },
            );
        }
        if let Some(page) = page
            && task.tabs.contains(&page)
        {
            task.last_used_tab = Some(page);
        }
        Ok(())
    }

    /// Sets where changes go, once. False when a sink is already set; it stays.
    pub(super) fn set_sink(&mut self, sink: Arc<dyn BookSink>) -> bool {
        if self.sink.is_some() {
            return false;
        }
        self.sink = Some(sink);
        true
    }

    /// One `Held` for each tab task `id` holds, with its origin now.
    fn emit_held(&self, id: &TaskId) {
        let Ok(task) = self.get(id) else {
            return;
        };
        for tab in &task.tabs {
            if let Some(holder) = self.holders.get(tab) {
                self.emit(|| BookChange::Held {
                    tab: *tab,
                    holder: holder.clone(),
                    origin: task.origin,
                });
            }
        }
    }

    /// Builds the change only when a sink takes it, so a book with none clones nothing.
    fn emit(&self, change: impl FnOnce() -> BookChange) {
        if let Some(sink) = &self.sink {
            sink.changed(change());
        }
    }

    fn make_room(&mut self) -> Result<(), TaskError> {
        if self.tasks.len() < TASKS {
            return Ok(());
        }
        let index = self
            .tasks
            .iter()
            .position(|task| task.tabs.is_empty() && Arc::strong_count(&task.turn) == 1)
            .ok_or(TaskError::Full)?;
        self.tasks.remove(index);
        Ok(())
    }
}

fn slug(name: &str) -> String {
    let mut slug = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() == 24 {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "t".to_owned()
    } else {
        slug.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use browseros_core::TabId;
    use neo_trace::TaskId;

    use super::{CLOSED, Holder, TASKS, TaskBook, TaskError, slug};
    use crate::browser::book_change::BookLog;
    use crate::browser::{BookChange, TaskOrigin};

    #[test]
    fn names_make_short_ascii_slugs() -> Result<(), Box<dyn Error>> {
        for (name, expected) in [
            ("HN top 5", "hn-top-5"),
            (" --é-- ", "t"),
            (" A___B ", "a-b"),
            ("abcdefghijklmnopqrstuvw xyz", "abcdefghijklmnopqrstuvw"),
        ] {
            assert_eq!(slug(name), expected);
        }
        let mut book = TaskBook::default();
        let task = book.create(Some("HN top 5".to_owned()))?;
        let suffix = task
            .id
            .as_str()
            .strip_prefix("hn-top-5-")
            .ok_or("slug missing")?;
        assert_eq!(suffix.len(), 4);
        assert!(
            suffix
                .chars()
                .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
        );
        Ok(())
    }

    #[test]
    fn ownership_is_ordered_and_ends_with_the_tab() -> Result<(), Box<dyn Error>> {
        let mut book = TaskBook::default();
        let first = book.create(None)?;
        let second = book.create(None)?;
        book.attach(TabId(9), &first.id)?;
        book.attach(TabId(3), &first.id)?;
        book.attach(TabId(9), &second.id)?;
        assert_eq!(book.get(&first.id)?.tabs, vec![TabId(9), TabId(3)]);
        assert!(
            book.get(&second.id)?.tabs.is_empty(),
            "ownership never moves on a touch"
        );
        book.detach(TabId(9));
        assert_eq!(book.owner(TabId(9)), None);
        assert!(!book.remove_empty(&first.id), "a live tab keeps its task");
        book.detach(TabId(3));
        assert!(
            book.get(&first.id)?.tabs.is_empty(),
            "external closes retain the empty task"
        );
        assert!(book.remove_empty(&first.id));
        Ok(())
    }

    #[test]
    fn the_bound_forgets_only_the_oldest_empty_task() -> Result<(), Box<dyn Error>> {
        let mut book = TaskBook::default();
        for index in 0..TASKS {
            let id = TaskId::parse(&format!("host-{index}"))?;
            book.ensure(&id)?;
            if index != 1 {
                book.attach(TabId(i64::try_from(index)?), &id)?;
            }
        }
        let newest = book.create(None)?;
        assert!(book.get(&TaskId::parse("host-1")?).is_err());
        assert!(
            book.get(&TaskId::parse("host-0")?).is_ok(),
            "live tasks survive the bound"
        );
        book.attach(TabId(9999), &newest.id)?;
        assert!(matches!(book.create(None), Err(TaskError::Full)));
        assert_eq!(book.tasks.len(), TASKS);
        Ok(())
    }

    #[test]
    fn a_task_family_is_fixed_when_the_task_is_made() -> Result<(), Box<dyn Error>> {
        let mut book = TaskBook::default();
        let root = book.create(None)?;
        assert_eq!(
            (&root.family, root.origin),
            (&root.id, TaskOrigin::Agent),
            "a task created in a run with no host task roots its own family"
        );
        let host = TaskId::parse("host-s")?;
        book.ensure(&host)?;
        let named = book.get(&host)?;
        assert_eq!(
            (&named.family, named.origin),
            (&host, TaskOrigin::Host),
            "a host's task roots its own family"
        );
        let created = book.create_in(None, &host)?;
        assert_eq!(
            (&created.family, created.origin),
            (&host, TaskOrigin::Agent),
            "a task created in the host task's runs is in its family"
        );
        book.attach(TabId(7), &created.id)?;
        assert_eq!(
            book.holder(TabId(7)),
            Some(&Holder {
                task: created.id.clone(),
                family: host.clone(),
            }),
            "a tab's holder carries its task's family"
        );
        book.ensure(&created.id)?;
        let taken = book.get(&created.id)?;
        assert_eq!(
            (&taken.family, taken.origin),
            (&host, TaskOrigin::Host),
            "a host naming an agent's task makes it the host's, in the family it was made in"
        );
        Ok(())
    }

    #[test]
    fn the_book_tells_its_sink_each_change_in_book_order() -> Result<(), Box<dyn Error>> {
        let log = Arc::new(BookLog::default());
        let sink: Arc<BookLog> = Arc::clone(&log);
        let mut book = TaskBook::default();
        assert!(book.set_sink(sink));
        assert!(
            !book.set_sink(Arc::new(BookLog::default())),
            "the first sink stays"
        );
        let host = TaskId::parse("host-s")?;
        book.ensure(&host)?;
        let created = book.create(None)?;
        assert!(
            log.take().is_empty(),
            "a task with no tab has nothing to save"
        );

        book.attach(TabId(1), &host)?;
        book.join(TabId(2), &host)?;
        book.acted(TabId(2), &host);
        book.acted(TabId(2), &host);
        book.acted(TabId(1), &created.id);
        book.attach(TabId(3), &created.id)?;
        book.ensure(&created.id)?;
        book.ensure(&host)?;
        book.detach(TabId(1));
        book.closed(TabId(2));
        book.closed(TabId(9));
        assert!(book.remove_empty(&host));

        let held = |tab: i64, task: &TaskId, origin: TaskOrigin| BookChange::Held {
            tab: TabId(tab),
            holder: Holder {
                task: task.clone(),
                family: task.clone(),
            },
            origin,
        };
        assert_eq!(
            log.take(),
            vec![
                held(1, &host, TaskOrigin::Host),
                BookChange::Page { tab: TabId(1) },
                held(2, &host, TaskOrigin::Host),
                BookChange::Page { tab: TabId(2) },
                held(3, &created.id, TaskOrigin::Agent),
                BookChange::Page { tab: TabId(3) },
                held(3, &created.id, TaskOrigin::Host),
                BookChange::Gone { tab: TabId(1) },
                BookChange::Gone { tab: TabId(2) },
            ],
            "an open, a popup, a page move only when it moves, an origin move per held tab, and \
             each tab leaving; never a repeat, another task's act, an unheld close or an empty \
             task's end"
        );
        Ok(())
    }

    #[test]
    fn a_close_is_remembered_for_the_family_that_held_the_tab() -> Result<(), Box<dyn Error>> {
        let mut book = TaskBook::default();
        let mine = TaskId::parse("host-s")?;
        let theirs = TaskId::parse("host-r")?;
        book.ensure(&mine)?;
        book.ensure(&theirs)?;
        let created = book.create_in(None, &mine)?;
        book.attach(TabId(1), &created.id)?;
        book.attach(TabId(2), &mine)?;
        book.closed(TabId(1));
        assert!(
            book.was_closed(TabId(1), &mine),
            "a created task's close is its family's"
        );
        assert!(!book.was_closed(TabId(1), &theirs), "and no other family's");
        book.detach(TabId(2));
        assert!(
            !book.was_closed(TabId(2), &mine),
            "Neo's blank tab taking a tab is no close"
        );
        book.closed(TabId(3));
        assert!(
            !book.was_closed(TabId(3), &mine),
            "a tab no task held closes for no family"
        );
        for index in 0..CLOSED {
            let tab = TabId(100 + i64::try_from(index)?);
            book.attach(tab, &mine)?;
            book.closed(tab);
        }
        assert!(
            !book.was_closed(TabId(1), &mine),
            "the oldest close is forgotten at the bound"
        );
        assert!(book.was_closed(TabId(100), &mine));
        Ok(())
    }
}
