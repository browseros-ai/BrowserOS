//! The orchestration of one run.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::{future, io, mem};

use browseros_core::TabId;
use neo_http::Settings;
use neo_playwright::RunState;
use neo_trace::{
    EpochMillis, RunEnded, RunId, RunOutputs, RunStarted, TaskId, TraceSink, TraceStore, Values,
};
use tokio::task::JoinSet;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;

use super::request_ledger::{Admission, LedgerLimits, RunTicket};
use super::snapshot_tips::SnapshotTips;
use super::{HostData, RequestLedger, RunHandle, RunMeta, RunReply, RunRequest};
use crate::browser::{BrowserLink, NeoTab, PageRef, RunView, TabRegistry};
use crate::builtins;
use crate::config::Defaults;
use crate::dispatch::{
    Dispatcher, ErrorCode, OperationError, PageFact, PageFacts, RunCtx, RunStart, StopReason,
};
use crate::runtime::{Script, ScriptEnd, ScriptOutcome, run_script};

/// Runs `run` calls. One per process, shared by every request. There is no concurrency limit
/// (design D6): every admitted run executes at once, and a waiting run costs about 0.3 MB and
/// no thread.
pub(crate) struct RunService {
    dispatcher: Arc<Dispatcher>,
    browser: Arc<BrowserLink>,
    tabs: Arc<TabRegistry>,
    store: TraceStore,
    trace: TraceSink,
    settings: Settings,
    ledger: Arc<RequestLedger>,
    /// Cancelled on SIGTERM: every run stops as `neo / shutting_down` and records its end.
    shutdown: CancellationToken,
    /// How long a run with a `requestId` outlives its last caller.
    orphan_grace: Duration,
    /// How many delta tips each task was given (`snapshot_tips.rs`).
    snapshot_tips: SnapshotTips,
    /// Neo's blank tab, left out of a run's initial pages; `None` in the hosted embedding.
    blank_tab: Option<Arc<NeoTab>>,
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
    /// The live policy, read once when the run writes its result blobs.
    pub(crate) settings: Settings,
    /// Fires on SIGTERM.
    pub(crate) shutdown: CancellationToken,
    /// `Defaults::ORPHAN_GRACE` in the app.
    pub(crate) orphan_grace: Duration,
    /// Neo's blank tab (`NeoTab`), which the sidecar keeps; `None` in the hosted embedding.
    pub(crate) blank_tab: Option<Arc<NeoTab>>,
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
            settings,
            shutdown,
            orphan_grace,
            blank_tab,
        } = parts;
        Arc::new(Self {
            ledger: RequestLedger::new(store.clone(), trace.clone(), LedgerLimits::default()),
            dispatcher,
            browser,
            tabs,
            store,
            trace,
            settings,
            shutdown,
            orphan_grace,
            snapshot_tips: SnapshotTips::default(),
            blank_tab,
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
        // Before the reply, so a host that reads a download or a blob right after it finds the
        // file; and before the end record, which drops the run state that lists the downloads.
        builtins::keep_downloads(&run, self.store.outputs()).await;
        let values = self.settings.trace_policy().values;
        let end = keep_blobs(run.id(), end, self.store.outputs(), values).await;
        // A run's init scripts end with it: removed before the reply, so the next run's first
        // navigation, or the owner's, loads without them. A renderer that does not answer within
        // the grace leaves the rest to the run state's drop, which removes them in the background.
        let _removed = time::timeout(
            Defaults::TEARDOWN_GRACE,
            builtins::remove_init_scripts(&run),
        )
        .await;
        // The run's end record is built from its reply, so the trace and a `requestId` replay keep
        // exactly what the caller got.
        let mut reply = RunReply::build(&run, end);
        reply.tasks = self.dispatcher.task_notes(&run).await;
        // Both read the run's Playwright state, before the end record drops it. A snapshot's
        // text becomes its tab's delta baseline only now, with the reply in hand: when the run
        // ended ok and what the call returned is in what the caller gets.
        let playwright = run.slot::<RunState>();
        playwright.settle_snapshot_baselines(reply.error.is_none(), &|returned: &str| {
            reply.shows(returned)
        });
        let task = run.task_id().cloned().or_else(|| run.current_task());
        reply.tips = self.snapshot_tips.decide(
            reply.error.is_some(),
            task.as_ref(),
            &playwright.full_snapshot_repeats(),
            &self.tabs,
        );
        let reply = Arc::new(reply);
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
                // Not Neo's blank tab: the prelude listens on every page it names, and no script
                // sees that tab (`NeoTab`). One the owner browsed in is handed back first, so the
                // run starts with their page.
                if let Some(blank_tab) = &self.blank_tab {
                    let listed: Vec<(TabId, &str)> = listing
                        .pages
                        .iter()
                        .map(|page| (page.tab_id, page.url.as_str()))
                        .collect();
                    blank_tab.settle(&listed).await;
                }
                let pages: Vec<PageRef> = listing
                    .pages
                    .iter()
                    .map(PageRef::from)
                    .filter(|page| !self.tabs.is_neo(page.tab))
                    .collect();
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
                    .zip(observation.workers)
                    .map(|((page, info), workers)| {
                        let mut fact = PageFact::in_view(page, info, run.view());
                        fact.task_id = self.tabs.owner(page.tab).map(|id| id.to_string());
                        fact.workers = workers;
                        fact
                    })
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
    /// share one by accident. Without a `taskId`, a fresh tab opens on first use.
    async fn default_page(&self, task: Option<&TaskId>, view: &RunView) -> Option<PageRef> {
        let task = task?;
        if let Some(page) = self.tabs.default_page(task) {
            return Some(page);
        }
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

/// Writes the blobs `end`'s value names beside the trace (`outputs/<run>/blobs/<id>`), where the
/// host reads them once it has the reply. One that cannot be written fails the run as Neo's, and
/// the blobs already written go with it: a reply must never name bytes the host cannot read, and
/// a failed one names none.
async fn keep_blobs(
    run: RunId,
    mut end: ScriptEnd,
    outputs: &RunOutputs,
    values: Values,
) -> ScriptEnd {
    let blobs = mem::take(&mut end.blobs);
    let kept = async {
        // Read once by the caller: a concurrent settings change cannot give one reply's
        // blobs different lifetimes. Write expiry first, before any bytes can outlive a crash.
        if !blobs.is_empty() && values == Values::Off {
            let window = i64::try_from(Defaults::REPLAY_WINDOW.as_millis()).unwrap_or(i64::MAX);
            outputs
                .keep_blobs_until(run, EpochMillis::now().plus_millis(window))
                .await?;
        }
        for blob in &blobs {
            outputs.keep_blob(run, blob.id, &blob.bytes).await?;
        }
        Ok::<(), io::Error>(())
    }
    .await;
    if let Err(error) = kept {
        tracing::warn!(%error, %run, "could not keep result blobs");
        end.outcome = ScriptOutcome::Failed(OperationError::neo(
            ErrorCode::INTERNAL_ERROR,
            format_args!("Neo could not keep the result's bytes for the host: {error}"),
        ));
        if let Err(error) = outputs.forget_blobs(run).await {
            tracing::warn!(%error, %run, "the failed reply's blobs remain for a later cleanup");
        }
    }
    end
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
    use serde_json::Value;
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
    #[test]
    fn initial_facts_include_task_ownership_and_null_for_user_tabs() -> Result<(), TestError> {
        on_neo_runtime(async {
            let rig = TestRig::new().await?;
            let owned = rig.owner_tab("https://task.example/").await?;
            let user = rig.owner_tab("https://user.example/").await?;
            let task = rig.tabs.create_task(None)?;
            rig.tabs.touched(&owned, rig.tabs.seen(), None);
            rig.tabs.own(&owned, &task.id)?;
            let service = rig.run_service(Duration::ZERO);
            let (run, _guard) = rig.run(Duration::from_secs(30), None)?;
            let facts = service.open_pages(&run).await;
            let owned = facts
                .pages
                .iter()
                .find(|fact| fact.tab == owned.tab)
                .ok_or("owned fact")?;
            let user = facts
                .pages
                .iter()
                .find(|fact| fact.tab == user.tab)
                .ok_or("user fact")?;
            assert_eq!(owned.task_id.as_deref(), Some(task.id.as_str()));
            let wire = serde_json::to_value(user)?;
            assert_eq!(
                wire.get("taskId"),
                Some(&Value::Null),
                "null must be serialized, not omitted"
            );
            Ok(())
        })
    }
}
