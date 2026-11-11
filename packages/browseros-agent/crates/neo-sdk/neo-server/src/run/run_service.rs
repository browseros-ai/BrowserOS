//! The orchestration of one run.

use std::collections::HashMap;
use std::convert::Infallible;
use std::future;
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use neo_trace::{EpochMillis, RunEnded, RunId, RunStarted, TaskId, TraceSink, TraceStore};
use tokio::task::JoinSet;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

use super::request_ledger::{Admission, LedgerLimits, RunTicket};
use super::{HostData, RequestLedger, RunHandle, RunMeta, RunReply, RunRequest};
use crate::browser::{BrowserLink, PageRef, RunView, TabRegistry};
use crate::builtins;
use crate::config::Defaults;
use crate::dispatch::{
    Dispatcher, ErrorCode, OperationError, PageFact, PageFacts, RunCtx, RunStart, StopReason,
};
use crate::runtime::{Script, run_script};

/// Runs `run` calls. One per process, shared by every request. There is no concurrency limit
/// (design D6): every admitted run executes at once, and a waiting run costs about 0.3 MB and
/// no thread.
pub(crate) struct RunService {
    dispatcher: Arc<Dispatcher>,
    browser: Arc<BrowserLink>,
    tabs: Arc<TabRegistry>,
    store: TraceStore,
    trace: TraceSink,
    ledger: Arc<RequestLedger>,
    /// Cancelled on SIGTERM: every run stops as `neo / shutting_down` and records its end.
    shutdown: CancellationToken,
    /// How long a run with a `requestId` outlives its last caller.
    orphan_grace: Duration,
    /// Makes the next run's task panic right after its start record, so tests can watch the
    /// panic boundary hold.
    #[cfg(test)]
    crash_next_run: AtomicBool,
}

/// What the run service is built from. A plain data bag.
pub(crate) struct RunServiceParts {
    /// The one door.
    pub(crate) dispatcher: Arc<Dispatcher>,
    /// The browser, for each run's view.
    pub(crate) browser: Arc<BrowserLink>,
    /// Each task's last tab, for the `page` default.
    pub(crate) tabs: Arc<TabRegistry>,
    /// The trace store: `requestId` lookups and a task's last tab after a restart.
    pub(crate) store: TraceStore,
    /// The trace writer.
    pub(crate) trace: TraceSink,
    /// Fires on SIGTERM.
    pub(crate) shutdown: CancellationToken,
    /// `Defaults::ORPHAN_GRACE` in the app.
    pub(crate) orphan_grace: Duration,
}

impl RunService {
    /// The other JS host uses precisely the product's initial tab listing, with the host
    /// observation that orders it against later replies.
    #[cfg(feature = "hosted-run")]
    pub(crate) async fn hosted_open_pages(&self, run: &RunCtx) -> PageFacts {
        self.open_pages(run).await
    }

    /// The service.
    #[must_use]
    pub(crate) fn new(parts: RunServiceParts) -> Arc<Self> {
        let RunServiceParts {
            dispatcher,
            browser,
            tabs,
            store,
            trace,
            shutdown,
            orphan_grace,
        } = parts;
        Arc::new(Self {
            ledger: RequestLedger::new(store.clone(), trace.clone(), LedgerLimits::default()),
            dispatcher,
            browser,
            tabs,
            store,
            trace,
            shutdown,
            orphan_grace,
            #[cfg(test)]
            crash_next_run: AtomicBool::new(false),
        })
    }

    /// Runs one call to its reply. Awaiting this is waiting for the run; dropping the future is
    /// leaving it, which is how the MCP layer reports that the client went away (stateless
    /// rmcp has no other cancel). The run itself lives in its own task: it is the panic
    /// boundary, and it outlives this future for a `requestId`'s orphan grace.
    pub(crate) async fn run(self: &Arc<Self>, request: RunRequest) -> Arc<RunReply> {
        let grace = if request.meta.request_id.is_some() {
            self.orphan_grace
        } else {
            Duration::ZERO
        };
        let handle = RunHandle::new(RunId::new(), grace);
        let waiter = handle.waiter();
        match self.ledger.admit(&request.meta, &handle).await {
            Admission::Fresh(ticket) => {
                // No await between the claim and the spawn, so a caller that leaves now still
                // leaves a run that publishes through its ticket.
                tokio::spawn(Arc::clone(self).execute(request, handle, ticket));
                waiter.reply().await
            }
            Admission::Join(first) => first.reply().await,
            Admission::Replay(reply) => reply,
            Admission::Duplicate(first) => {
                let reply = RunReply::duplicate(handle.id(), first);
                self.record_refused(request.code, request.meta, reply)
            }
            Admission::Unavailable(error) => {
                self.refuse_as(handle.id(), request.code, request.meta, &error)
            }
        }
    }

    /// A call refused before it became a run (bad arguments or `_meta`, from the MCP layer).
    pub(crate) fn refuse(
        &self,
        code: String,
        meta: RunMeta,
        error: &OperationError,
    ) -> Arc<RunReply> {
        self.refuse_as(RunId::new(), code, meta, error)
    }

    /// Refuses a call as run `id`, recorded.
    fn refuse_as(
        &self,
        id: RunId,
        code: String,
        meta: RunMeta,
        error: &OperationError,
    ) -> Arc<RunReply> {
        self.record_refused(code, meta, RunReply::refused(id, error))
    }

    /// Records a refused call as a run that ended at once, so the reply's `id` resolves in the
    /// trace. Without the request id, though: nothing ran, and a recorded request id would
    /// make the store refuse that request's retry as a duplicate for good. Its code and host
    /// data are stored all the same.
    fn record_refused(&self, code: String, meta: RunMeta, reply: RunReply) -> Arc<RunReply> {
        let id = reply.id;
        self.trace.run_started(RunStarted {
            id,
            code,
            request_id: None,
            task_id: meta.task_id,
            host_data: meta.host_data.map(HostData::into_value),
            at: EpochMillis::now(),
        });
        let end = reply.trace_end(None);
        self.trace.run_ended(RunEnded {
            id,
            status: end.status,
            error: end.error,
            result: end.result,
            replay: end.replay,
            at: EpochMillis::now(),
        });
        Arc::new(reply)
    }

    /// The run task's body. Every return publishes through the ticket, and a panic anywhere
    /// in it unwinds through the guard (`run_ended` as a crash) and the ticket (a crashed
    /// reply for every waiter), so a caller always gets an answer; no `catch_unwind` (G16).
    async fn execute(
        self: Arc<Self>,
        request: RunRequest,
        handle: Arc<RunHandle>,
        ticket: RunTicket,
    ) {
        let RunRequest {
            code,
            timeout,
            meta,
        } = request;
        let Some(view) = self.browser.view() else {
            let error = OperationError::neo(
                ErrorCode::BROWSER_UNAVAILABLE,
                "the browser is not connected yet; retry in a few seconds",
            );
            ticket.release(self.refuse_as(handle.id(), code, meta, &error));
            return;
        };
        let page = self.default_page(meta.task_id.as_ref(), &view).await;
        if let Some(page) = &page {
            // In use from now on, so the idle-tab janitor leaves it, and the task's tab.
            let seen = self.tabs.seen();
            if self.browser.page(page.tab).await.is_some() {
                self.tabs.touched(page, seen, meta.task_id.as_ref());
            }
        }
        let RunMeta {
            request_id,
            task_id,
            host_data,
        } = meta;
        let (run, guard) = RunCtx::start(
            &self.dispatcher,
            RunStart {
                id: handle.id(),
                code: code.clone(),
                request_id: request_id.clone(),
                task_id,
                host_data: host_data.map(HostData::into_value),
                view,
                deadline: Instant::now() + timeout.get(),
                default_page: page.clone(),
            },
        );
        #[cfg(test)]
        self.crash_if_asked();
        let script = async {
            let facts = self.open_pages(&run).await;
            run_script(
                &run,
                &self.dispatcher,
                Script {
                    code,
                    page,
                    pages: facts.pages,
                    observed: facts.observed,
                },
            )
            .await
        };
        let end = tokio::select! {
            end = script => end,
            never = self.stop_when_due(&run, &handle) => match never {},
        };
        // Ops still running end through the door as the script's (`script / not_awaited`),
        // with their page facts, and a background pull (the event pump) ends quietly; stragglers
        // past the grace are aborted and settle in their guards. Their records precede the
        // run's end.
        run.stop(StopReason::ScriptReturned);
        let mut ops = run.close_ops();
        if time::timeout(Defaults::TEARDOWN_GRACE, drain(&mut ops))
            .await
            .is_err()
        {
            ops.shutdown().await;
        }
        // Before the reply, so a host that reads a download right after it finds the file; and
        // before the end record, which drops the run state that lists the downloads.
        builtins::keep_downloads(&run, self.store.outputs()).await;
        // The run's end record is built from its reply, so the trace and a `requestId` replay keep
        // exactly what the caller got.
        let reply = Arc::new(RunReply::build(&run, end));
        guard.finish(reply.trace_end(request_id.as_ref()));
        ticket.finish(reply);
    }

    /// Every tab open as the run starts, for its init (decision 25, PB input 4): Playwright's
    /// `context.pages()` is synchronous, so the prelude can list a tab only once something named
    /// it, and this names them all before the script's first op. The facts are a reply's, the
    /// realm's other entry. A listing that fails leaves the list empty, and replies then name the
    /// tabs the script touches. Raced with the run's stop, inside the race with its deadline.
    async fn open_pages(&self, run: &RunCtx) -> PageFacts {
        let listing = tokio::select! {
            biased;
            () = run.cancel_token().cancelled() => return PageFacts { pages: Vec::new(), observed: None },
            listing = run.view().list_pages() => listing,
        };
        match listing {
            Ok(listing) => {
                let pages: Vec<PageRef> = listing.pages.iter().map(PageRef::from).collect();
                if pages.is_empty() {
                    return PageFacts {
                        pages: Vec::new(),
                        observed: None,
                    };
                }
                let tabs: Vec<_> = pages.iter().map(|page| page.tab).collect();
                let observation = self
                    .dispatcher
                    .observer()
                    .observe(&self.browser, &self.tabs, &tabs, &HashMap::new(), None)
                    .await;
                let pages = pages
                    .iter()
                    .zip(observation.infos)
                    .map(|(page, info)| PageFact::in_view(page, info, run.view()))
                    .collect();
                PageFacts {
                    pages,
                    observed: Some(observation.observed),
                }
            }
            Err(error) => {
                tracing::debug!(%error, "could not list the tabs for the run's init; context.pages() starts empty");
                PageFacts {
                    pages: Vec::new(),
                    observed: None,
                }
            }
        }
    }

    /// Stops the run on the first of its deadline, shutdown and its callers leaving, then
    /// never resolves: the script's end ends the race.
    async fn stop_when_due(&self, run: &RunCtx, handle: &RunHandle) -> Infallible {
        let reason = tokio::select! {
            () = time::sleep_until(run.deadline()) => Some(StopReason::Deadline),
            () = self.shutdown.cancelled() => Some(StopReason::Shutdown),
            () = handle.orphaned() => Some(StopReason::CallerLeft),
            // Stopped already, by the interrupt handler's own deadline check.
            () = run.cancel_token().cancelled() => None,
        };
        if let Some(reason) = reason {
            run.stop(reason);
        }
        future::pending().await
    }

    /// `page` at the start of a run. With a `taskId`: the task's last tab, from memory, or
    /// after a restart from the trace (`task_last_tab`), resolved to a live page; with no
    /// history, `None`, and the run opens a fresh tab on first use, so parallel tasks never
    /// share one by accident. Without a `taskId`: the last tab any script op touched.
    async fn default_page(&self, task: Option<&TaskId>, view: &RunView) -> Option<PageRef> {
        if let Some(page) = self.tabs.default_page(task) {
            return Some(page);
        }
        let task = task?;
        let tab = match self.store.task_last_tab(task).await {
            Ok(tab) => tab?,
            Err(error) => {
                tracing::warn!(%error, %task, "cannot read the task's last tab; the run opens a fresh one");
                return None;
            }
        };
        match view.view.resolve_tab_ids(&[tab]).await {
            Ok(pages) => pages.get(&tab).map(|_| PageRef { tab }),
            Err(error) => {
                tracing::debug!(%error, %task, "the task's last tab is gone; the run opens a fresh one");
                None
            }
        }
    }

    /// Asks the next run to panic in its own task (tests only).
    #[cfg(test)]
    pub(crate) fn crash_next_run(&self) {
        self.crash_next_run.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    fn crash_if_asked(&self) {
        assert!(
            !self.crash_next_run.swap(false, Ordering::SeqCst),
            "a test asked this run's task to panic"
        );
    }
}

/// Waits for every op. A panicked op already answered and settled through its own guard.
async fn drain(ops: &mut JoinSet<()>) {
    while let Some(joined) = ops.join_next().await {
        if let Err(error) = joined
            && error.is_panic()
        {
            tracing::error!(%error, "an operation's task panicked; its guard settled it");
        }
    }
}

#[cfg(test)]
mod observation_tests {
    use crate::test_rig::{TestError, TestRig, on_neo_runtime, within};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time;

    #[test]
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "the test pauses observations while the separate core task prunes the tab"
    )]
    fn init_facts_recheck_a_tab_pruned_after_listing() -> Result<(), TestError> {
        on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let page = rig.owner_tab("https://closing.example/").await?;
            let live = rig.owner_tab("https://live.example/?token=7Qx").await?;
            let service = rig.run_service(Duration::ZERO);
            let (run, _guard) = rig.run(Duration::from_secs(30), None)?;
            let held = service.dispatcher.observer().hold().await;
            // Publish a distinct cache value only through this listing. Counting the CDP
            // request alone can race its reply and let the close precede the snapshot.
            rig.fake.hold_events();
            let listed_url = "https://live.example/?token=7Qx&snapshot=ready";
            assert!(rig.fake.navigate_tab(live.tab, listed_url, "Live"));
            let pending_service = Arc::clone(&service);
            let pending = tokio::spawn(async move { pending_service.open_pages(&run).await });
            within(Duration::from_secs(2), async {
                while rig
                    .link
                    .page(live.tab)
                    .await
                    .is_none_or(|page| page.url != listed_url)
                {
                    time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await?;
            assert!(
                !pending.is_finished(),
                "the observation waits behind the held serial"
            );
            assert!(rig.fake.close_tab(page.tab));
            rig.fake.release_events();
            within(Duration::from_secs(2), async {
                while rig.link.page(page.tab).await.is_some() {
                    time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await?;
            drop(held);
            let facts = pending.await?;
            assert!(facts.observed.is_some());
            assert_eq!(facts.pages.len(), 2, "the listing had named both tabs");
            let closed = facts
                .pages
                .iter()
                .find(|fact| fact.tab == page.tab)
                .ok_or("closed tab fact")?;
            assert!(
                closed.closed,
                "observation rechecked the cache after pruning"
            );
            let live = facts
                .pages
                .iter()
                .find(|fact| fact.tab == live.tab)
                .ok_or("live tab fact")?;
            assert_eq!(
                live.url.as_deref(),
                Some(listed_url),
                "the init carries the URL the observation read"
            );
            Ok(())
        })
    }
}
