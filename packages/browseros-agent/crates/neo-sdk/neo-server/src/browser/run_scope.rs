//! What a run may reach, and the one answer the door asks about each tab it names.
//!
//! A run that carries a task, under task scope, reaches only tabs its task's family holds; every
//! other run reaches every tab but Neo's blank one, as before task scope. The scope is fixed when
//! the run starts, while which tabs it covers follows ownership live: the answer is read from the
//! task book at each ask ([`super::TabRegistry::sight`]).
//!
//! ```text
//!  setting  the run's host task  scope       reaches                 may pick / open in / close
//!  browser  any                  Browser     every tab but Neo's     every task
//!  task     none                 Taskless    every tab but Neo's     tasks agents created
//!  task     S (family root R)    Family(R)   R's family's tabs       R's family's tasks
//! ```

use neo_trace::{RunTabsScope, TaskId};

use super::{TaskOrigin, TaskRecord};

/// The tab scope setting as a run reads it, once, at its start (`Settings::tabs_scope`).
pub(crate) use neo_http::TabsScope;

/// What a run may reach, fixed at its start ([`RunScope::decide`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RunScope {
    /// Every tab but Neo's blank one, and every task: task scope is off.
    Browser,
    /// Task scope is on and the run carries no task. It reaches every tab but Neo's blank one,
    /// as `Browser` does, but may not pick, open in or close a task a host named: that would
    /// take over someone else's task.
    Taskless,
    /// Only the tabs a task of this family holds, their popups that no task holds yet, and the
    /// family's tasks. The family is named by its root ([`super::TaskRecord::family`]).
    Family(TaskId),
}

impl RunScope {
    /// The scope of a run from the setting it read at its start and the family of its host task
    /// as it started (`RunCtx::family`): bound only when both say so.
    #[must_use]
    pub(crate) fn decide(setting: TabsScope, family: Option<&TaskId>) -> Self {
        match (setting, family) {
            (TabsScope::Browser, _) => Self::Browser,
            (TabsScope::Task, None) => Self::Taskless,
            (TabsScope::Task, Some(root)) => Self::Family(root.clone()),
        }
    }

    /// What the run's reply and its trace record say of its tabs: `task` only for a bound run.
    /// A run with no task under task scope sees every tab, so it says `browser`.
    #[must_use]
    pub(crate) fn record(&self) -> RunTabsScope {
        if self.is_bound() {
            RunTabsScope::Task
        } else {
            RunTabsScope::Browser
        }
    }

    /// What a call refused before it became a run would have seen: bound when the setting is
    /// `task` and the call carries a task.
    #[must_use]
    pub(crate) fn would_be(setting: TabsScope, task: Option<&TaskId>) -> RunTabsScope {
        match (setting, task) {
            (TabsScope::Task, Some(_)) => RunTabsScope::Task,
            (TabsScope::Task, None) | (TabsScope::Browser, _) => RunTabsScope::Browser,
        }
    }

    /// Whether the door walks this run's ops for the tabs they name: only a bound run's. Every
    /// other run takes the path it took before task scope.
    #[must_use]
    pub(crate) fn is_bound(&self) -> bool {
        match self {
            Self::Family(_) => true,
            Self::Browser | Self::Taskless => false,
        }
    }

    /// Whether a run of this scope may pick `task` (`neo.task`), open a tab in it or close it. A
    /// task it may not use is answered as a missing one, so the answer reveals nothing about
    /// which tasks exist.
    #[must_use]
    pub(crate) fn may_use(&self, task: &TaskRecord) -> bool {
        match self {
            Self::Browser => true,
            Self::Taskless => task.origin == TaskOrigin::Agent,
            Self::Family(root) => task.family == *root,
        }
    }
}

/// Whether a run may reach a tab ([`super::TabRegistry::sight`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sight {
    /// The run may list it, name it, act on it, read from it and hear about it.
    Shown,
    /// It closed while the run's family held it: the run is told the page closed.
    Closed,
    /// Neo's own blank tab, which no run reaches in any scope.
    Reserved,
    /// Another task's tab, a tab no task holds, or a tab that never existed, answered alike so
    /// the answer reveals nothing about which tabs exist.
    Foreign,
}
