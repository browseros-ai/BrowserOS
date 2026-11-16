//! A bound run's opens whose tab the browser has not named yet, so its other ops can wait for
//! them before they judge a tab they do not know.
//!
//! ```text
//!  context.newPage (op A) ─ begin(A) ─ Browser.createTab ─ the engine's opened_tab(tab):
//!     named(A): the tab is the run's own (RunCtx::opened) ─ ready() ─ op A ends (guard dropped)
//!  events.next (op B) lists a new tab ─ tabs_settled ─ others_named(B): waits while any op
//!     but B has an open not named yet ─ then the tab is judged: the run's own, or hidden
//! ```
//!
//! Without the wait, a bound run's own new tab, listed or announced in the moment between the
//! browser making it and the open's reply, would be judged another task's and its `page` event
//! dropped (`Promise.all([context.waitForEvent('page'), context.newPage()])`). An op never waits
//! for its own open, and a cancelled op's guard ends its open on drop, so no wait outlives the
//! ops it waits for.

use std::collections::HashSet;
use std::pin::pin;
use std::sync::{Mutex, MutexGuard, PoisonError};

use neo_trace::OperationId;
use tokio::sync::Notify;

/// The run's ops with an open in flight. One per run, on its `RunCtx`.
#[derive(Default)]
pub(crate) struct OpensInFlight {
    ops: Mutex<HashSet<OperationId>>,
    /// Woken whenever an open is named or ends.
    changed: Notify,
}

/// `op`'s open, in flight until the browser names its tab or the op ends.
#[must_use]
pub(crate) struct OpenInFlight<'a> {
    opens: &'a OpensInFlight,
    op: OperationId,
}

impl OpensInFlight {
    /// `op` starts an open.
    pub(crate) fn begin(&self, op: OperationId) -> OpenInFlight<'_> {
        self.lock().insert(op);
        OpenInFlight { opens: self, op }
    }

    /// The browser named the tab `op` opened: its open is no longer in flight.
    pub(crate) fn named(&self, op: OperationId) {
        if self.lock().remove(&op) {
            self.changed.notify_waiters();
        }
    }

    /// Resolves once no op but `op` has an open in flight. A new open that starts meanwhile is
    /// waited for too; each ends when its tab is named or its op ends, the run's deadline
    /// included.
    pub(crate) async fn others_named(&self, op: OperationId) {
        loop {
            // Registered before the check, so a change between the two is not missed.
            let mut changed = pin!(self.changed.notified());
            changed.as_mut().enable();
            if !self.lock().iter().any(|other| *other != op) {
                return;
            }
            changed.await;
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashSet<OperationId>> {
        self.ops.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for OpenInFlight<'_> {
    fn drop(&mut self) {
        self.opens.named(self.op);
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::time::Duration;

    use neo_trace::{OperationId, RunId};
    use tokio::time;

    use super::OpensInFlight;

    #[tokio::test]
    async fn an_op_waits_for_the_runs_other_opens_never_its_own() -> Result<(), Box<dyn Error>> {
        let opens = OpensInFlight::default();
        let run = RunId::new();
        let (opener, reader) = (OperationId::new(run, 1), OperationId::new(run, 2));
        let open = opens.begin(opener);
        assert!(
            time::timeout(Duration::from_millis(50), opens.others_named(opener))
                .await
                .is_ok(),
            "never its own open"
        );
        assert!(
            time::timeout(Duration::from_millis(50), opens.others_named(reader))
                .await
                .is_err(),
            "another op's open in flight is waited for"
        );
        let (waited, ()) = tokio::join!(
            time::timeout(Duration::from_secs(5), opens.others_named(reader)),
            async {
                time::sleep(Duration::from_millis(20)).await;
                opens.named(opener);
            },
        );
        assert!(waited.is_ok(), "woken once the browser names its tab");
        drop(open);
        let again = opens.begin(opener);
        drop(again);
        assert!(
            time::timeout(Duration::from_millis(50), opens.others_named(reader))
                .await
                .is_ok(),
            "an op that ends, cancelled or not, ends its open"
        );
        Ok(())
    }
}
