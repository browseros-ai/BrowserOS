//! Focus emulation for input on tabs core did not create for the agent: popups, and tabs the
//! owner or another session opened (design D10, decisions.md).
//!
//! A background tab is throttled: Chromium stops hit testing and compositing there, and
//! measured on BrowserOS a click's move and press took about 5 s to be acknowledged without
//! focus emulation. Core turns emulation on for good on the tabs it opens for the agent. Any
//! other tab keeps its real focus state, so an input op leases emulation for its own length:
//!
//! ```text
//!  op starts ── acquire ── holders 0 → 1: Emulation.setFocusEmulationEnabled { true }
//!  op ends, fails or is dropped ── FocusLeaseGuard::drop ── spawned release
//!                                    holders 1 → 0: Emulation.setFocusEmulationEnabled { false }
//! ```
//!
//! The count is per tab, so overlapping ops (two clicks under `Promise.all`, whose waits are
//! not serialized) keep emulation on until the last one ends. The count and the CDP call change
//! together under one tokio mutex, so the on and off messages leave in the order the count
//! moved and a release can never turn emulation off under an op that just acquired it.

use std::sync::Arc;

use browseros_core::{CoreError, ProtocolSession};
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::Mutex;

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::{ErrorCode, PlaywrightError};

/// One tab's focus emulation holders. Lives in the tab's `PageState`.
#[derive(Debug, Default)]
pub(crate) struct FocusLease {
    holders: Mutex<u32>,
}

/// One op's hold on its tab's focus emulation. Dropping it releases the hold on every exit:
/// success, failure, the op's own timeout, or the host dropping the op's future.
#[must_use = "focus emulation is released when the guard drops"]
#[derive(Debug)]
pub(crate) struct FocusLeaseGuard {
    lease: Arc<FocusLease>,
    /// The tab's root session, where emulation is set.
    session: ProtocolSession,
}

impl FocusLease {
    /// Holds focus emulation on `session`'s tab until the guard drops; the first holder turns
    /// it on.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "the count and the CDP setting change together, so on and off messages leave \
                  in the order the count moved"
    )]
    pub(crate) async fn acquire(
        self: &Arc<Self>,
        session: &ProtocolSession,
    ) -> Result<FocusLeaseGuard, CoreError> {
        let mut holders = self.holders.lock().await;
        *holders += 1;
        // Counted before the call: if the call fails, or this future is dropped while it is in
        // flight, the guard's release turns emulation off again instead of leaving it on.
        let guard = FocusLeaseGuard {
            lease: Arc::clone(self),
            session: session.clone(),
        };
        if *holders == 1 {
            session
                .send::<_, Value>(
                    "Emulation.setFocusEmulationEnabled",
                    json!({ "enabled": true }),
                )
                .await?;
        }
        drop(holders);
        Ok(guard)
    }
}

/// Focus emulation for the op's length on a tab core did not create for the agent (D10):
/// without it a background tab barely hit-tests. `None` on agent tabs, which have it for good.
pub(crate) async fn lease(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
) -> Result<Option<FocusLeaseGuard>, PlaywrightError> {
    if scope.view().is_agent_created(page.id()).await {
        return Ok(None);
    }
    let root = page.root_session();
    match scope.within(page.state().focus().acquire(&root)).await {
        Ok(Ok(guard)) => Ok(Some(guard)),
        Ok(Err(error)) => Err(scope.core_error(&error)),
        Err(_expired) => Err(scope.timed_out(ErrorCode::NoMatch)),
    }
}

/// The release is spawned: it awaits the count's lock and a CDP call, which `Drop` cannot,
/// and it must run after a dropped op too. Without a runtime there is nothing left to restore.
impl Drop for FocusLeaseGuard {
    fn drop(&mut self) {
        let Ok(runtime) = Handle::try_current() else {
            return;
        };
        let lease = Arc::clone(&self.lease);
        let session = self.session.clone();
        drop(runtime.spawn(release(lease, session)));
    }
}

#[expect(
    clippy::await_holding_invalid_type,
    reason = "the count and the CDP setting change together; see FocusLease::acquire"
)]
async fn release(lease: Arc<FocusLease>, session: ProtocolSession) {
    let mut holders = lease.holders.lock().await;
    *holders = holders.saturating_sub(1);
    if *holders == 0 {
        // Best effort: a tab that closed has nothing left to restore.
        let _restored = session
            .send::<_, Value>(
                "Emulation.setFocusEmulationEnabled",
                json!({ "enabled": false }),
            )
            .await;
    }
}

#[cfg(test)]
mod tests;
