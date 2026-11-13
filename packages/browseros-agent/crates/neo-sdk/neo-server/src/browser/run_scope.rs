//! What a run may reach, and the one answer the door asks about each tab it names.
//!
//! A run that carries a task, under task scope, reaches only tabs its task's family holds; every
//! other run reaches every tab but Neo's blank one, as before task scope. The scope is fixed when
//! the run starts, while which tabs it covers follows ownership live: the answer is read from the
//! task book at each ask ([`super::TabRegistry::sight`]).

use neo_trace::TaskId;

/// What a run may reach, fixed at its start.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the door's sight check reads it, and no run asks it until runs carry a scope"
    )
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RunScope {
    /// Every tab but Neo's blank one: a run with no task, or any run when task scope is off.
    Browser,
    /// Only the tabs a task of this family holds, and their popups that no task holds yet. The
    /// family is named by its root ([`super::TaskRecord::family`]).
    Family(TaskId),
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
