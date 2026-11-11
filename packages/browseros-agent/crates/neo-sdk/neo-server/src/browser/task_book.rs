//! In-memory tasks and their ordered tab ownership, under the tab registry's lock.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use browseros_core::TabId;
use neo_trace::{IdError, RunId, TaskId};
use tokio::sync::Mutex;

const TASKS: usize = 4096;

/// A task's current tabs, in the order they joined it.
#[derive(Clone, Debug)]
pub(crate) struct TaskRecord {
    pub(crate) id: TaskId,
    pub(crate) name: Option<String>,
    pub(crate) tabs: Vec<TabId>,
    /// Serializes opens and closes. A held record also pins an empty task during an open.
    pub(crate) turn: Arc<Mutex<()>>,
}

/// A task operation refused before changing ownership.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TaskError {
    #[error(
        "no task `{0}` is open in Neo (tasks live in memory and end with `task.close()` or a Neo restart). Start one with `neo.createTask(name)`, or find your tabs with `neo.tabs()` and read `page.taskId`."
    )]
    NotFound(TaskId),
    #[error(
        "Neo already has 4096 tasks with open tabs; close a task with `task.close()` before starting another"
    )]
    Full,
    #[error("could not generate a task id: {0}")]
    InvalidId(#[from] IdError),
}

/// Updated with the tab registry, so a close removes its owner and task membership together.
#[derive(Default)]
pub(super) struct TaskBook {
    tasks: VecDeque<TaskRecord>,
    owners: HashMap<TabId, TaskId>,
}

impl TaskBook {
    pub(super) fn create(&mut self, name: Option<String>) -> Result<TaskRecord, TaskError> {
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
                id,
                name,
                tabs: Vec::new(),
                turn: Arc::default(),
            };
            self.tasks.push_back(task.clone());
            return Ok(task);
        }
    }

    pub(super) fn ensure(&mut self, id: &TaskId) -> Result<(), TaskError> {
        if self.get(id).is_ok() {
            return Ok(());
        }
        self.make_room()?;
        self.tasks.push_back(TaskRecord {
            id: id.clone(),
            name: None,
            tabs: Vec::new(),
            turn: Arc::default(),
        });
        Ok(())
    }

    pub(super) fn get(&self, id: &TaskId) -> Result<&TaskRecord, TaskError> {
        self.tasks
            .iter()
            .find(|task| task.id == *id)
            .ok_or_else(|| TaskError::NotFound(id.clone()))
    }

    pub(super) fn owner(&self, tab: TabId) -> Option<&TaskId> {
        self.owners.get(&tab)
    }

    pub(super) fn attach(&mut self, tab: TabId, id: &TaskId) -> Result<(), TaskError> {
        let task = self
            .tasks
            .iter_mut()
            .find(|task| task.id == *id)
            .ok_or_else(|| TaskError::NotFound(id.clone()))?;
        // A touch or a repeated popup notice must never move a tab between tasks.
        if self.owners.contains_key(&tab) {
            return Ok(());
        }
        task.tabs.push(tab);
        self.owners.insert(tab, id.clone());
        Ok(())
    }

    pub(super) fn detach(&mut self, tab: TabId) {
        if let Some(id) = self.owners.remove(&tab)
            && let Some(task) = self.tasks.iter_mut().find(|task| task.id == id)
        {
            task.tabs.retain(|owned| *owned != tab);
        }
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

    use browseros_core::TabId;
    use neo_trace::TaskId;

    use super::{TASKS, TaskBook, TaskError, slug};

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
}
