//! A run's explicit fetches, shared ambient task, and first-touch reply order.

use neo_trace::TaskId;
use serde::{Deserialize, Serialize};

use super::{ErrorCode, OperationError};
use crate::browser::{TabRegistry, TaskRecord};

/// Why the reply names a task. Closing it supersedes its first-touch kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TaskKind {
    Created,
    BuiltinPage,
    Fetched,
    Closed,
}

/// Task recovery information stored in the reply, including failed runs and replays.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TaskNote {
    pub(crate) id: String,
    pub(crate) name: Option<String>,
    pub(crate) kind: TaskKind,
    pub(crate) tabs: Vec<TabNote>,
}

/// A task's open tab and URL at reply time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TabNote {
    #[serde(rename = "tabId")]
    pub(crate) tab: i64,
    pub(crate) url: String,
}

/// Protected by the run's task lock. Ambient creation and publication are one critical section.
#[derive(Default)]
pub(super) struct RunTasks {
    fetched: Option<TaskId>,
    auto: Option<TaskId>,
    touched: Vec<TaskNote>,
}

impl RunTasks {
    pub(super) fn fetched(&self) -> Option<&TaskId> {
        self.fetched.as_ref()
    }

    pub(super) fn fetch(&mut self, task: &TaskRecord, kind: TaskKind) {
        self.fetched = Some(task.id.clone());
        self.touch(task, kind);
    }

    pub(super) fn touch(&mut self, task: &TaskRecord, kind: TaskKind) {
        if let Some(note) = self
            .touched
            .iter_mut()
            .find(|note| note.id == task.id.as_str())
        {
            if kind == TaskKind::Closed {
                note.kind = kind;
            }
            return;
        }
        self.touched.push(TaskNote {
            id: task.id.to_string(),
            name: task.name.clone(),
            kind,
            tabs: Vec::new(),
        });
    }

    pub(super) fn notes(&self) -> Vec<TaskNote> {
        self.touched.clone()
    }

    pub(super) fn ambient(
        &mut self,
        tabs: &TabRegistry,
        host: Option<&TaskId>,
    ) -> Result<TaskId, OperationError> {
        if let Some(host) = host {
            tabs.ensure_task(host)?;
            return Ok(host.clone());
        }
        if let Some(id) = &self.auto {
            // A closed or evicted auto task is not silently recreated under the same id.
            tabs.task(id)?;
            return Ok(id.clone());
        }
        let task = tabs.create_task(None)?;
        self.touch(&task, TaskKind::BuiltinPage);
        self.auto = Some(task.id.clone());
        Ok(task.id)
    }

    pub(super) fn check_new_page(&self) -> Result<(), OperationError> {
        if let Some(id) = &self.fetched {
            return Err(OperationError::script(
                ErrorCode::TASK_PAGE_REQUIRED,
                format!("this script uses task {id}: open tabs with `await task.newPage()`."),
            ));
        }
        Ok(())
    }
}
