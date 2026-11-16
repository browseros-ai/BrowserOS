//! One running run, as the callers waiting on it see it.

use std::pin::pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use neo_trace::{RunId, RunTabsScope};
use tokio::sync::{Notify, watch};
use tokio::time;

use super::RunReply;

/// One run's reply channel and who is still waiting for it (orphan grace, decision 17).
///
/// The caller that started the run and every repeat of its `requestId` that joined it hold a
/// [`Waiter`]; dropping it, as the MCP layer does once the client's connection goes, is how a
/// caller leaves. The run task watches the count: once it is zero for `grace`, the run stops as
/// `host / cancelled`. A transport timeout drops the connection exactly when the host retries
/// with the same `requestId`, so a run with one gets `Defaults::ORPHAN_GRACE` for the retry to
/// join; a run without one stops at once, because nobody could ever collect its reply.
pub(crate) struct RunHandle {
    id: RunId,
    grace: Duration,
    /// Which tabs the run saw or would have seen, set once at admission, so a reply the run
    /// never wrote itself (a crash) says it too.
    tabs_scope: OnceLock<RunTabsScope>,
    reply: watch::Sender<Option<Arc<RunReply>>>,
    waiters: AtomicUsize,
    left: Notify,
}

/// One caller waiting on a run. Dropping it is leaving.
pub(crate) struct Waiter {
    handle: Arc<RunHandle>,
    reply: watch::Receiver<Option<Arc<RunReply>>>,
}

impl RunHandle {
    /// The handle of run `id`, which outlives its last waiter by `grace`.
    #[must_use]
    pub(crate) fn new(id: RunId, grace: Duration) -> Arc<Self> {
        Arc::new(Self {
            id,
            grace,
            tabs_scope: OnceLock::new(),
            reply: watch::channel(None).0,
            waiters: AtomicUsize::new(0),
            left: Notify::new(),
        })
    }

    /// The run's id.
    #[must_use]
    pub(crate) fn id(&self) -> RunId {
        self.id
    }

    /// Which tabs the run sees, as admission decided it. Write-once.
    pub(crate) fn set_tabs_scope(&self, scope: RunTabsScope) {
        let _ = self.tabs_scope.set(scope);
    }

    /// The reply of a run whose task ended without one, with the run's tab scope.
    #[must_use]
    pub(crate) fn crashed(&self) -> RunReply {
        RunReply::crashed(self.id)
            .with_tabs_scope(self.tabs_scope.get().copied().unwrap_or_default())
    }

    /// A new waiter, counted before it is returned, so the count is never zero between a
    /// join and its waiter.
    #[must_use]
    pub(crate) fn waiter(self: &Arc<Self>) -> Waiter {
        self.waiters.fetch_add(1, Ordering::SeqCst);
        Waiter {
            handle: Arc::clone(self),
            reply: self.reply.subscribe(),
        }
    }

    /// Hands the reply to every waiter, present and future.
    pub(crate) fn finish(&self, reply: Arc<RunReply>) {
        self.reply.send_replace(Some(reply));
    }

    /// Resolves once no caller has waited on the run for `grace`, counted from the latest time
    /// the last waiter left. The run task races it with the script, so it ends with the run.
    pub(crate) async fn orphaned(&self) {
        loop {
            // Enabled before the count is read, so a departure in between still wakes it.
            let mut left = pin!(self.left.notified());
            left.as_mut().enable();
            if self.waiters.load(Ordering::SeqCst) != 0 {
                left.await;
                continue;
            }
            if self.grace.is_zero() {
                return;
            }
            // `left` fires only when the count drops to zero again, so it means a retry joined
            // and left during the grace: the grace starts over from that departure. Checked
            // first, so a departure at the very end of the grace still restarts it.
            tokio::select! {
                biased;
                () = left => {}
                () = time::sleep(self.grace) => {
                    if self.waiters.load(Ordering::SeqCst) == 0 {
                        return;
                    }
                }
            }
        }
    }
}

impl Waiter {
    /// The run's reply. Every run task publishes one on every path (its ticket's `Drop`
    /// publishes a crashed one), so this answers; a closed channel is that crash as well.
    pub(crate) async fn reply(mut self) -> Arc<RunReply> {
        let published = self
            .reply
            .wait_for(Option::is_some)
            .await
            .ok()
            .and_then(|seen| seen.clone());
        published.unwrap_or_else(|| Arc::new(self.handle.crashed()))
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        if self.handle.waiters.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.handle.left.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use neo_trace::RunId;
    use tokio::time::{self, Duration};

    use super::RunHandle;
    use crate::run::RunReply;

    #[tokio::test(start_paused = true)]
    async fn a_run_is_orphaned_only_once_no_waiter_came_back_within_the_grace()
    -> Result<(), Box<dyn Error>> {
        let handle = RunHandle::new(RunId::new(), Duration::from_secs(15));
        let first = handle.waiter();
        let watch = Arc::clone(&handle);
        let orphaned = tokio::spawn(async move { watch.orphaned().await });
        drop(first);
        time::sleep(Duration::from_secs(10)).await;
        let retry = handle.waiter();
        time::sleep(Duration::from_secs(10)).await;
        assert!(
            !orphaned.is_finished(),
            "a retry that joined within the grace keeps the run going"
        );
        drop(retry);
        time::sleep(Duration::from_secs(16)).await;
        assert!(orphaned.is_finished(), "nobody came back for 15 s");
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn the_grace_restarts_when_a_retry_that_joined_leaves() {
        let handle = RunHandle::new(RunId::new(), Duration::from_secs(15));
        let first = handle.waiter();
        let watch = Arc::clone(&handle);
        let orphaned = tokio::spawn(async move { watch.orphaned().await });
        drop(first);
        time::sleep(Duration::from_secs(10)).await;
        let retry = handle.waiter();
        time::sleep(Duration::from_secs(4)).await;
        drop(retry);
        time::sleep(Duration::from_secs(14)).await;
        assert!(
            !orphaned.is_finished(),
            "the retry left 14 s ago; it gets the whole 15 s to come back"
        );
        time::sleep(Duration::from_secs(2)).await;
        assert!(orphaned.is_finished(), "nobody came back for 15 s");
    }

    #[tokio::test(start_paused = true)]
    async fn without_a_grace_the_last_waiter_leaving_orphans_at_once() {
        let handle = RunHandle::new(RunId::new(), Duration::ZERO);
        let waiter = handle.waiter();
        let watch = Arc::clone(&handle);
        let orphaned = tokio::spawn(async move { watch.orphaned().await });
        drop(waiter);
        time::sleep(Duration::from_millis(1)).await;
        assert!(orphaned.is_finished());
    }

    #[tokio::test]
    async fn every_waiter_gets_the_one_reply() {
        let id = RunId::new();
        let handle = RunHandle::new(id, Duration::ZERO);
        let (early, late) = (handle.waiter(), handle.waiter());
        handle.finish(Arc::new(RunReply::crashed(id)));
        assert_eq!(early.reply().await.id, id);
        assert_eq!(
            late.reply().await.id,
            id,
            "a waiter that came after the reply still gets it"
        );
    }
}
