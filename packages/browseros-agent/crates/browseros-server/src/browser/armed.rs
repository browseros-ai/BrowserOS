//! The armed table: which tasks `/mcp/{task}` answers for, and under which scope.
//!
//! A coding agent is a child process: it cannot send the task id or read-only access the
//! engine takes from `_meta`. The runner arms the conversation's task for one turn and hands
//! the agent `/mcp/{task}`; the endpoint looks the task up here and runs every call under the
//! scope the turn armed. Dropping the `Armed` removes the entry and fires its token, which
//! cancels the agent's calls still in flight.
//!
//! The same task can be armed again while an older arming is still held (a turn that ends while
//! the next starts). Each arming has its own generation, so dropping the older one never
//! disarms the newer.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use tokio_util::sync::CancellationToken;

use super::Scope;

#[derive(Default)]
pub(super) struct ArmedTable {
    entries: Mutex<Entries>,
}

#[derive(Default)]
struct Entries {
    by_task: HashMap<String, Entry>,
    next_generation: u64,
}

struct Entry {
    generation: u64,
    call: ArmedCall,
}

/// What a call through `/mcp/{task}` runs under: the turn's scope, and the token the turn's
/// `Armed` fires when it is dropped.
#[derive(Clone)]
pub(super) struct ArmedCall {
    pub(super) scope: Scope,
    pub(super) cancel: CancellationToken,
}

/// One arming's claim on its table entry. Holds the table weakly: an `Armed` outliving its
/// `Browser` must not keep the table alive.
pub(super) struct Slot {
    table: Weak<ArmedTable>,
    task: String,
    generation: u64,
}

impl ArmedTable {
    pub(super) fn arm(self: &Arc<Self>, scope: Scope) -> (CancellationToken, Slot) {
        let cancel = CancellationToken::new();
        let mut entries = self.lock();
        entries.next_generation += 1;
        let generation = entries.next_generation;
        let task = scope.task.clone();
        entries.by_task.insert(
            task.clone(),
            Entry {
                generation,
                call: ArmedCall {
                    scope,
                    cancel: cancel.clone(),
                },
            },
        );
        let slot = Slot {
            table: Arc::downgrade(self),
            task,
            generation,
        };
        (cancel, slot)
    }

    pub(super) fn get(&self, task: &str) -> Option<ArmedCall> {
        self.lock()
            .by_task
            .get(task)
            .map(|entry| entry.call.clone())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Entries> {
        // A panic while holding this lock leaves the map whole: every critical section is a
        // single insert, lookup or remove.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Slot {
    pub(super) fn release(&self) {
        let Some(table) = self.table.upgrade() else {
            return;
        };
        let mut entries = table.lock();
        if entries
            .by_task
            .get(&self.task)
            .is_some_and(|entry| entry.generation == self.generation)
        {
            entries.by_task.remove(&self.task);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::Access;

    fn scope(task: &str, access: Access) -> Scope {
        Scope {
            task: task.into(),
            access,
        }
    }

    #[test]
    fn releasing_an_older_arming_keeps_the_newer() {
        let table = Arc::new(ArmedTable::default());
        let (older_cancel, older) = table.arm(scope("c1", Access::Read));
        let (newer_cancel, newer) = table.arm(scope("c1", Access::Full));
        older.release();
        older_cancel.cancel();
        let call = table.get("c1").map(|call| call.scope);
        assert_eq!(call, Some(scope("c1", Access::Full)));
        assert!(!newer_cancel.is_cancelled());
        newer.release();
        assert!(table.get("c1").is_none());
    }

    #[test]
    fn a_slot_outliving_its_table_is_harmless() {
        let table = Arc::new(ArmedTable::default());
        let (_cancel, slot) = table.arm(scope("c1", Access::Full));
        drop(table);
        slot.release();
    }
}
