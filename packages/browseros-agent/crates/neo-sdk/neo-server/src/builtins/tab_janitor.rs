//! Closing agent tabs nobody uses any more.

use std::sync::Arc;
use std::time::Duration;

use futures_util::future;
use neo_trace::{RunId, RunStatus};
use serde_json::json;
use tokio::time::{self, Instant};

use crate::browser::{BrowserLink, PageRef, TabRegistry};
use crate::config::Defaults;
use crate::dispatch::{
    Call, Caller, Dispatcher, OpReply, PendingReply, RunCtx, RunEnd, RunStart, StopReason,
};
use crate::host::TakeoverLock;

/// The method the janitor submits for each idle tab.
const CLOSE: &str = "page.close";
/// The `code` a sweep's run records, so the trace says who closed the tabs and why.
const SWEEP_CODE: &str = "// Neo's tab janitor: page.close on each idle agent-opened tab";
/// One sweep's budget. Each close is one CDP round trip, so this only bounds a browser that
/// stopped answering; the next sweep tries again.
const SWEEP_BUDGET: Duration = Duration::from_secs(30);

/// Closes agent-opened tabs no op touched for [`Defaults::TAB_IDLE`] (decision 5). With no
/// sessions nothing else ever frees them, and a hosted Neo runs for weeks.
///
/// Each close is a `page.close` op that Neo submits through the dispatcher, in a run of its own
/// with no `requestId` and no `taskId` (synthesis graft 8), so the trace records what Neo closed.
/// Only `TabOrigin::Agent` tabs are candidates, so the owner's own tabs and popups are never
/// closed. A tab the owner holds is skipped (`TakeoverLock::is_locked`) and stays a candidate
/// for a later sweep, since Neo's ops do not restart a tab's idle clock (only script ops do);
/// skipping keeps a long takeover from leaving a refused close in the trace every minute. A lock
/// taken between the skip and the close is still the door's to refuse: the close is an op like
/// any other.
pub(crate) struct TabJanitor {
    dispatcher: Arc<Dispatcher>,
    link: Arc<BrowserLink>,
    tabs: Arc<TabRegistry>,
    takeover: Arc<TakeoverLock>,
}

impl TabJanitor {
    /// The janitor over the app's one dispatcher, whose trace records its runs, and the app's
    /// link, tab registry and takeover lock.
    #[must_use]
    pub(crate) fn new(
        dispatcher: Arc<Dispatcher>,
        link: Arc<BrowserLink>,
        tabs: Arc<TabRegistry>,
        takeover: Arc<TakeoverLock>,
    ) -> Self {
        Self {
            dispatcher,
            link,
            tabs,
            takeover,
        }
    }

    /// One pass, for the app's housekeeping to call every minute with `Instant::now()`. Returns
    /// the sweep's run, or `None` when nothing idle is free to close or the browser is not
    /// linked (no run is recorded then; the next pass tries again).
    pub(crate) async fn sweep(&self, now: Instant) -> Option<RunId> {
        let idle: Vec<PageRef> = self
            .tabs
            .idle_agent_tabs(Defaults::TAB_IDLE, now)
            .into_iter()
            .filter(|page| !self.takeover.is_locked(page.tab))
            .collect();
        if idle.is_empty() {
            return None;
        }
        let Some(view) = self.link.view() else {
            tracing::debug!(
                tabs = idle.len(),
                "idle agent tabs wait for the browser link"
            );
            return None;
        };
        let deadline = Instant::now() + SWEEP_BUDGET;
        let (run, guard) = RunCtx::start(
            &self.dispatcher,
            RunStart {
                id: RunId::new(),
                code: SWEEP_CODE.to_owned(),
                request_id: None,
                task_id: None,
                host_data: None,
                view,
                deadline,
                default_page: None,
            },
        );
        // Submitted together, so one slow close does not hold up the rest.
        let pending: Vec<PendingReply> = idle
            .iter()
            .map(|page| {
                let call = Call {
                    name: CLOSE.to_owned(),
                    input: json!({ "page": page }),
                    caller: Caller::Neo,
                    site: None,
                };
                self.dispatcher.submit(&run, call)
            })
            .collect();
        let replies = future::join_all(pending.into_iter().map(PendingReply::wait));
        tokio::pin!(replies);
        // Every op answers on every path, so after the stop the replies still arrive, as
        // `neo / run_timeout`.
        let replies: Vec<OpReply> = tokio::select! {
            replies = &mut replies => replies,
            () = time::sleep_until(deadline) => {
                run.stop(StopReason::Deadline);
                replies.await
            }
        };
        let mut closed = Vec::new();
        let mut kept = Vec::new();
        for (page, reply) in idle.iter().zip(replies) {
            match reply.outcome {
                Ok(_) => {
                    tracing::info!(run = %run.id(), tab = page.tab.0, "closed an idle agent tab");
                    closed.push(page.tab.0);
                }
                Err(failure) => {
                    tracing::info!(run = %run.id(), tab = page.tab.0, error = %failure.error, "kept an idle agent tab");
                    kept.push(page.tab.0);
                }
            }
        }
        // The ops' tasks end right after answering; drained so their records precede the run's.
        let mut ops = run.close_ops();
        while ops.join_next().await.is_some() {}
        let status = if run.stop_reason() == Some(StopReason::Deadline) {
            RunStatus::Timeout
        } else {
            RunStatus::Ok
        };
        guard.finish(RunEnd {
            status,
            error: None,
            result: Some(json!({ "closed": closed, "kept": kept })),
            replay: None,
        });
        Some(run.id())
    }
}
