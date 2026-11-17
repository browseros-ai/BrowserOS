//! A run's current task and first-touch order for host metadata.

use neo_trace::TaskId;
use serde::{Deserialize, Serialize};

use crate::browser::TaskRecord;

/// Why the reply names a task. Closing it supersedes its first-touch kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TaskKind {
    Created,
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

/// Protected by the run's task lock; the latest explicit pick wins.
pub(super) struct RunTasks {
    current: Option<TaskId>,
    touched: Vec<TaskNote>,
}

impl RunTasks {
    pub(super) fn new(host: Option<TaskId>) -> Self {
        Self {
            current: host,
            touched: Vec::new(),
        }
    }

    pub(super) fn current(&self) -> Option<&TaskId> {
        self.current.as_ref()
    }

    pub(super) fn pick(&mut self, task: &TaskRecord, kind: TaskKind) {
        self.current = Some(task.id.clone());
        self.touch(task, kind);
    }

    pub(super) fn close(&mut self, task: &TaskRecord, host: Option<&TaskId>) {
        if self.current.as_ref() == Some(&task.id) {
            self.current = host.cloned();
        }
        self.touch(task, TaskKind::Closed);
    }

    fn touch(&mut self, task: &TaskRecord, kind: TaskKind) {
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
}
