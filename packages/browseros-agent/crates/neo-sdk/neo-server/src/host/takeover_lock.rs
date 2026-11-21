//! The owner's takeover lock, as the door and the janitor ask it.

use std::sync::Arc;

use browseros_core::TabId;
use neo_takeover::{LockId, LockTarget, Locks, NamedLock};
use tokio::sync::SetOnce;

use crate::dispatch::{ErrorCode, Listing, OperationError, Policy, PolicyCheck};

/// The takeover lock rule (design D7 rule 4, decision 19). The door asks it about every op, at
/// admission and again in each `OpCtx::recheck` before a write is sent; a held lock on the op's
/// tab refuses it with `host / owner_has_control`, which the door reports `not_sent`. The
/// idle-tab janitor reads the same locks, with the fits, through `HostHolds`.
///
/// The locks belong to neo-http, which starts only once the browser link's first
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
    pub(crate) fn is_locked(&self, tab: TabId) -> bool {
        self.locks.get().is_some_and(|locks| locks.is_locked(tab))
    }

    /// The armed locks, for tests that take one bound as a takeover's.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn locks(&self) -> Option<Arc<Locks>> {
        self.locks.get().cloned()
    }

    /// What lock `lock` is on `tab` now, for a fill that names it. `NotHeld` before the host
    /// API armed the rule, when no lock can exist.
    #[must_use]
    pub(crate) fn named(&self, lock: LockId, tab: TabId) -> NamedLock {
        self.locks
            .get()
            .map_or(NamedLock::NotHeld, |locks| locks.named(lock, tab))
    }
}

impl Policy for TakeoverLock {
    fn check(&self, check: &PolicyCheck<'_>) -> Result<(), OperationError> {
        let Some(locks) = self.locks.get() else {
            return Ok(());
        };
        // Browser and Target commands can reach every tab even on another page's session.
        // The disabled NeoCdp variant is the only hidden one. Hiding an enabled variant would
        // require changing this condition too, or it would silently lose lock protection.
        let target = if check.method.name == "neo.cdp" && check.method.listing != Listing::Hidden {
            LockTarget::UnmappedPage
        } else {
            check.tab.map_or(LockTarget::NoPage, LockTarget::Tab)
        };
        let decided = match check.lock_pass {
            // The host's private typing through its own lock: the pass holds only while that
            // lock does. Released or expired, the agent is no longer locked out as the host
            // believed, so the fill stops at its next ask of the rules (design section 10.1). A
            // takeover's lock is the owner's and is never passed, named or not (ruling R25).
            Some(pass) => match check.tab.map(|tab| locks.named(pass, tab)) {
                Some(NamedLock::NotHeld) => {
                    return Err(OperationError::host(
                        ErrorCode::LOCK_RELEASED,
                        "the lock this fill names is no longer held on its tab, so the agent is \
                         not locked out; nothing more was typed",
                    ));
                }
                Some(NamedLock::HostHeld) => locks.check_passing(target, check.method.effect, pass),
                Some(NamedLock::TakeoverHeld) | None => locks.check(target, check.method.effect),
            },
            None => locks.check(target, check.method.effect),
        };
        decided.map_err(|refused| {
            OperationError::host(
                ErrorCode::OWNER_HAS_CONTROL,
                format_args!("{refused} (lock {})", refused.lock),
            )
        })
    }
}
