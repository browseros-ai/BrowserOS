//! The owner's takeover lock, as the door and the janitor ask it.

use std::sync::Arc;

use neo_takeover::{LockTarget, Locks};
use tokio::sync::SetOnce;

use crate::dispatch::{ErrorCode, OperationError, Policy, PolicyCheck};

/// The takeover lock rule (design D7 rule 4, decision 19). The door asks it about every op, at
/// admission and again in each `OpCtx::recheck` before a write is sent; a held lock on the op's
/// tab refuses it with `host / owner_has_control`, which the door reports `not_sent`. The
/// idle-tab janitor reads the same locks, with the fits, through `HostHolds`.
///
/// The locks belong to neo-service, which starts only once the browser link's first
/// connect lands (its camera screenshots on the agent's own session). Until the host API arms
/// this rule it refuses nothing, which is exact rather than lenient: a lock is taken only
/// through the host API's routes, and those are served only after arming. `default()` is
/// unarmed.
#[derive(Default)]
pub(crate) struct TakeoverLock {
    locks: SetOnce<Arc<Locks>>,
}

impl TakeoverLock {
    /// The host API started: from now on its locks decide. Called once per process; a second
    /// call changes nothing.
    pub(crate) fn arm(&self, locks: Arc<Locks>) {
        let _ = self.locks.set(locks);
    }

    /// Whether the owner holds `tab` now, for tests that wait on a lock the door will see.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn is_locked(&self, tab: browseros_core::TabId) -> bool {
        self.locks.get().is_some_and(|locks| locks.is_locked(tab))
    }
}

impl Policy for TakeoverLock {
    fn check(&self, check: &PolicyCheck<'_>) -> Result<(), OperationError> {
        let Some(locks) = self.locks.get() else {
            return Ok(());
        };
        // The door resolves an op's page to its tab before any rule runs, and refuses a page it
        // cannot resolve, so `None` is an op with no page at all. `UnmappedPage` (a page whose
        // tab is unknown, refused under any lock) cannot arise from this door.
        let target = check.tab.map_or(LockTarget::NoPage, LockTarget::Tab);
        locks.check(target, check.method.effect).map_err(|refused| {
            OperationError::host(
                ErrorCode::OWNER_HAS_CONTROL,
                format_args!("{refused} (lock {})", refused.lock),
            )
        })
    }
}
