//! Neo: The entry point: owns the browser connection and runs each script from its requested tab.
//!
//! A run's order, after the tab check and the attach:
//!
//! ```text
//!  Tabs ─▶ PageEntry ─▶ Arc<PageSession> ─┬─▶ quickjs::run(…, dispatcher)   the deadline decides, as before
//!                                             ├─▶ dispatcher.finish(reason)  cancel calls, wait ≤ 1 s (highlights ≤ 1 s more), records
//!                                             └─▶ last Arc drops ─▶ Session::drop ─▶ detach in background
//! ```
//!
//! No step here detaches anything; ownership does (`page.rs`). The registry holds each page,
//! so sessions detach once the realm, dispatcher and their writers are gone, also when a
//! caller drops the run's future. Browser tabs themselves survive the run.

use std::sync::Arc;

use browseros_cdp::{CdpClient, CdpError, browser};
use serde_json::Value;
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio_util::task::TaskTracker;

use crate::artifacts::{AbsolutePaths, Artifacts};
use crate::browser_representations::cookies_and_storage::Permissions;
use crate::browser_representations::page::TabInputs;
use crate::browser_representations::tabs::Tabs;
use crate::chromium_bridge::browser_connection_settings::Switches;
use crate::deadline::{Deadline, Stop};
use crate::method_router::Dispatcher;
use crate::playwright_methods::getters::CLOSED_TAB_MESSAGE;
use crate::playwright_methods::screenshot::TabScreenshots;
use crate::progress::{ErrorName, RunScope, Trace};
use crate::quickjs::Script;
use crate::quickjs_bridge::{Getters, Logs};
use crate::run::{RunEnd, RunError, RunOutcome, RunRequest, RunStatus};
#[cfg(feature = "test-host")]
use crate::test_host::TestHost;
use crate::trace::{
    ConsoleLine, EndCause, RunEvent, RunId, RunSink, RunStarted, RunTap, TraceSink, redact,
};
use crate::{quickjs, quickjs_bridge};

/// A target id no tab can have, used to see how the browser reports a missing tab.
/// Neo: a nonexistent target tests BrowserOS support before accepting runs.
const PROBE_TARGET_ID: &str = "neo-runtime-connect-probe";

/// Runs agent scripts against tabs of one BrowserOS browser. Runs are independent: each gets
/// its own CDP session and its own QuickJS realm, so one `Runtime` serves any number of
/// concurrent runs. Runs on one tab share only its input and its screenshots: their clicks take
/// turns at its input, from interceptor setup to stop (`page.rs` `TabInputs`), and their
/// screenshots at its capture, cleanup included (`screenshot.rs` `TabScreenshots`). Clones share
/// those turns too.
///
/// Runs also share the connection's target discovery, on while any of them listens for new
/// pages (`discovery.rs`), and its download reports, on while any listens for downloads
/// (`downloads.rs`), which the `Runtime` counts (`browser_connection_settings.rs`). Give a `CdpClient` one `Runtime`
/// (clones are fine): a second `Runtime` on the same client would count its own runs, and the
/// first to let go would turn either off for the other's.
#[derive(Clone)]
pub struct Runtime {
    cdp: CdpClient,
    inputs: TabInputs,
    screenshots: TabScreenshots,
    /// Target discovery and download reports on `cdp`, each on while any run listens for them.
    switches: Switches,
    /// The runs' permission grants on `cdp`, reset once no run holds one (`cookies_and_storage.rs`).
    permissions: Permissions,
    /// How every run's `path` options treat an absolute path (`artifacts.rs`).
    absolute_paths: AbsolutePaths,
    cleanup: TaskTracker,
    /// Asked in order whether they listen to each run, before the run can fail.
    sinks: Arc<[Arc<dyn TraceSink>]>,
}

/// Close the run's tracker on every exit, including a dropped future or failed tab open.
/// The runtime's cleanup waiter must be able to finish on those paths too.
struct CloseRunTasks(TaskTracker);

impl Drop for CloseRunTasks {
    fn drop(&mut self) {
        self.0.close();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Cdp(#[from] CdpError),
    /// `page.isClosed()` reads BrowserOS's `Browser.getTabInfo`; a browser that lacks it, or
    /// reports a missing tab differently, cannot answer it.
    #[error(
        "Browser.getTabInfo does not report a missing tab as {CLOSED_TAB_MESSAGE:?}; is this BrowserOS? It answered: {0}"
    )]
    UnsupportedBrowser(String),
}

impl Runtime {
    /// Wraps a connected client after checking the browser reports a closed tab the way
    /// `page.isClosed()` expects.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::UnsupportedBrowser`] when `Browser.getTabInfo` is missing or reports
    /// an unknown tab differently, and [`RuntimeError::Cdp`] when the check itself fails.
    pub async fn connect(cdp: CdpClient) -> Result<Self, RuntimeError> {
        let probe = cdp
            .send_typed::<_, Value>(
                "Browser.getTabInfo",
                browser::GetTabInfoParams {
                    target_id: Some(PROBE_TARGET_ID.to_string()),
                    tab_id: None,
                },
                None,
            )
            .await;
        match probe {
            Err(CdpError::Protocol { message, .. }) if message.contains(CLOSED_TAB_MESSAGE) => {
                let cleanup = TaskTracker::new();
                Ok(Self {
                    cdp,
                    inputs: TabInputs::default(),
                    screenshots: TabScreenshots::default(),
                    switches: Switches::new(cleanup.clone()),
                    permissions: Permissions::new(cleanup.clone()),
                    absolute_paths: AbsolutePaths::default(),
                    cleanup,
                    sinks: Arc::default(),
                })
            }
            Err(CdpError::Protocol { message, .. }) => {
                Err(RuntimeError::UnsupportedBrowser(message))
            }
            Err(error) => Err(RuntimeError::Cdp(error)),
            Ok(reply) => Err(RuntimeError::UnsupportedBrowser(reply.to_string())),
        }
    }

    /// The same runtime, whose runs treat an absolute `path` option as `absolute_paths` says.
    /// The default, [`AbsolutePaths::Refuse`], refuses it; a host whose agents name paths on
    /// their own machine opts into [`AbsolutePaths::Redirect`], which still writes only inside
    /// each run's artifact directory ([`RunRequest::artifacts`]).
    #[must_use]
    pub fn with_absolute_paths(self, absolute_paths: AbsolutePaths) -> Self {
        Self {
            absolute_paths,
            ..self
        }
    }

    /// Waits for every run's remaining browser work, including late creation replies and
    /// their tab cleanup, and run-end input releases still writing. Hosts must stop admitting
    /// runs and cancel/await active runs first, then drain before disconnecting CDP; a
    /// returned outcome alone does not settle writes.
    pub async fn drain(&self) {
        self.cleanup.close();
        self.cleanup.wait().await;
    }

    /// Installs this runtime's trace sinks, replacing those previously installed. Each run
    /// asks them whether they listen before opening its starting tab.
    #[must_use]
    pub fn with_sinks(self, sinks: impl IntoIterator<Item = Arc<dyn TraceSink>>) -> Self {
        Self {
            sinks: sinks.into_iter().collect(),
            ..self
        }
    }

    /// Runs `request.code` against `request.page` and reports how it ended. Every failure,
    /// including a tab that is already closed, is a [`RunOutcome`] rather than an `Err`. The
    /// outcome carries one record per browser call the script made. The CDP session the run
    /// attaches to the tab is detached in the background once the run and every task its calls
    /// started have let go of it.
    ///
    /// Must be called from tokio's multi-threaded runtime. A synchronous getter such as
    /// `page.url()` waits for its CDP reply with `tokio::task::block_in_place`, which the
    /// current-thread runtime does not support; on it the run ends with an error before any
    /// script runs.
    pub async fn run(&self, request: RunRequest) -> RunOutcome {
        self.run_inner(
            request,
            #[cfg(feature = "test-host")]
            None,
        )
        .await
    }

    /// Neo: Runs the same native script lifecycle as [`Self::run`], with an opaque test transport
    /// installed as `__neo_test_host` in this run's realm. Ownership moves into the run, never
    /// into `Runtime`, so concurrent runs and runtime clones cannot inherit another host.
    /// The host does not dispatch browser methods or change call cancellation and cleanup.
    #[cfg(feature = "test-host")]
    pub async fn run_with_test_host(
        &self,
        request: RunRequest,
        host: Box<dyn TestHost>,
    ) -> RunOutcome {
        self.run_inner(request, Some(host)).await
    }

    async fn run_inner(
        &self,
        request: RunRequest,
        #[cfg(feature = "test-host")] test_host: Option<Box<dyn TestHost>>,
    ) -> RunOutcome {
        // Both public entry points share one guard so test-host transport cannot bypass
        // run identity, terminal records, or settlement of retained browser work.
        let guard = RunGuard::start(&self.sinks, &request);
        let (outcome, cause) = self
            .execute(
                &guard,
                request,
                #[cfg(feature = "test-host")]
                test_host,
            )
            .await;
        guard.end(outcome, cause)
    }

    /// Every exit returns to the run guard, which owns exactly one `RunEnded` emission.
    async fn execute(
        &self,
        guard: &RunGuard,
        request: RunRequest,
        #[cfg(feature = "test-host")] test_host: Option<Box<dyn TestHost>>,
    ) -> (RunOutcome, EndCause) {
        let run_id = guard.tap.run();
        let RunRequest {
            code,
            page,
            timeout,
            cancel,
            artifacts,
            origin: _,
        } = request;
        let deadline = Deadline::new(timeout, cancel);
        let tokio = match Handle::try_current() {
            Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => handle,
            _ => {
                let message = "neo-runtime runs need tokio's multi-threaded runtime".to_string();
                return (
                    RunOutcome::failed(run_id, RunError::new("Error", message), Vec::new()),
                    EndCause::Unsupported,
                );
            }
        };

        let tasks = TaskTracker::new();
        let _close_tasks = CloseRunTasks(tasks.clone());
        let remaining = tasks.clone();
        // Run outcomes have a bounded drain so callers get a timely report. Keep a
        // separate lifetime for hosts shutting down the shared browser connection.
        self.cleanup.spawn(async move { remaining.wait().await });
        let opened = tokio::select! {
            stop = deadline.reached() => {
                return (RunOutcome::stopped(run_id, stop, deadline.timeout(), Vec::new()), stopped_by(stop));
            }
            opened = Tabs::open(self.cdp.clone(), tokio.clone(), self.inputs.clone(), self.screenshots.clone(), self.switches.clone(), self.permissions.clone(), tasks.clone(), &page, Arc::clone(&guard.tap)) => opened,
        };
        let (tabs, url) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                let error = if error.name == ErrorName::TargetClosedError {
                    RunError::target_closed()
                } else {
                    RunError::new(error.name.as_str(), error.message)
                };
                return (
                    RunOutcome::failed(run_id, error, Vec::new()),
                    EndCause::TabUnavailable,
                );
            }
        };

        let script = Script {
            code,
            page_guid: quickjs_bridge::page_guid(&page),
            frame_guid: quickjs_bridge::frame_guid(&page),
            url,
            #[cfg(feature = "test-host")]
            test_host,
        };
        // One tracker for the run: call tasks, and the writer, interceptor-stop and
        // group-release tasks the page spawns, so `finish` waits for them all.
        let artifacts =
            Arc::new(Artifacts::new(artifacts).with_absolute_paths(self.absolute_paths));
        let run = RunScope::new(deadline.clone(), tasks, guard.tap.end_cell())
            .with_artifacts(Arc::clone(&artifacts));
        let getters = Getters::new(
            tabs.clone(),
            deadline.clone(),
            tokio,
            Arc::clone(&run.handles),
            Arc::clone(&guard.tap),
        );
        let dispatcher = Dispatcher::new(tabs.clone(), run, guard.trace.clone());
        let events = tabs.events().clone();
        // A run whose future is dropped lets go of its permission grants on the cleanup tracker.
        let grants = Grants {
            permissions: self.permissions.clone(),
            cdp: self.cdp.clone(),
            run: tabs.run_id(),
            released: false,
        };
        let outcome = quickjs::run(
            run_id,
            script,
            getters,
            dispatcher.clone(),
            deadline,
            events,
            guard.logs.clone(),
        )
        .await;
        let (calls, dropped) = dispatcher.finish(run_end_of(&outcome)).await;
        // `Dispatcher::finish` let go of the grants (`Tabs::release_run_scoped`).
        grants.disarm();
        for note in artifacts.notes() {
            guard.logs.push(note);
        }
        let cause = match outcome.status {
            RunStatus::Ok => EndCause::Returned,
            RunStatus::Error => EndCause::Threw,
            RunStatus::Timeout => EndCause::Deadline,
            RunStatus::Cancelled => EndCause::Cancelled,
        };
        let outcome = outcome
            .with_calls(calls, dropped)
            .with_pages(tabs.outcome())
            .with_artifacts(artifacts.written(), Vec::new());
        (outcome, cause)
    }
}

/// A run's hold on its permission grants (`cookies_and_storage.rs` `Permissions`), for a run whose
/// future is dropped before `Dispatcher::finish` let go of them (`Tabs::release_run_scoped`,
/// before the outcome returns, so the next run finds the browser's permissions as they were):
/// its grants are then released on the cleanup tracker.
struct Grants {
    permissions: Permissions,
    cdp: CdpClient,
    run: u64,
    released: bool,
}

impl Grants {
    /// The run's end let go of its grants: nothing left for the drop to release.
    fn disarm(mut self) {
        self.released = true;
    }
}

impl Drop for Grants {
    fn drop(&mut self) {
        if !self.released && Handle::try_current().is_ok() {
            self.permissions.release_later(self.cdp.clone(), self.run);
        }
    }
}

/// Why the run ended, for calls still running then. The realm reports `Timeout` or `Cancelled`
/// whenever the deadline or cancellation fired, even when the script caught the error and
/// returned, so the outcome's status is the one source for it.
fn run_end_of(outcome: &RunOutcome) -> RunEnd {
    match outcome.status {
        RunStatus::Timeout => RunEnd::Timeout,
        RunStatus::Cancelled => RunEnd::Cancelled,
        RunStatus::Ok | RunStatus::Error => RunEnd::Settled,
    }
}

/// One run's frame for its trace: built first, it owns what the run's end reports (the tap,
/// the trace, the console) and reports it exactly once, through [`RunGuard::end`] when the run
/// returns, or from `Drop` when the caller dropped the run's future.
///
/// Its `Arc` of the tap is one of the last to go: dropped with the guard, after the run's
/// `RunEnded`, so `RunSettled` waits for it and for every other holder.
struct RunGuard {
    tap: Arc<RunTap>,
    trace: Trace,
    logs: Logs,
    /// The script, kept for `RunEnded` only while a sink listens.
    script: Option<Arc<str>>,
    ended: bool,
}

impl RunGuard {
    /// Mints the run's id and asks each sink, in order, whether it listens, before anything
    /// can fail.
    fn start(sinks: &[Arc<dyn TraceSink>], request: &RunRequest) -> Self {
        let run = RunId::mint();
        let started = RunStarted::new(run, request);
        let listeners: Vec<Arc<dyn RunSink>> =
            sinks.iter().filter_map(|sink| sink.run(&started)).collect();
        let tap = RunTap::new(run, listeners);
        let script = tap.listening().then(|| Arc::from(request.code.as_str()));
        Self {
            logs: Logs::new(tap.start()),
            trace: Trace::new(Arc::clone(&tap)),
            script,
            tap,
            ended: false,
        }
    }

    /// The run's one exit: reports `RunEnded` from `outcome` and returns it with its console
    /// lines.
    fn end(mut self, outcome: RunOutcome, cause: EndCause) -> RunOutcome {
        self.ended = true;
        let lines = self.logs.take();
        if self.tap.listening() {
            let ended = self.ended_event(
                outcome.status,
                cause,
                outcome.error.as_ref(),
                outcome.value.as_ref(),
                &lines,
            );
            self.tap.emit(&ended);
        }
        outcome.with_logs(lines.into_iter().map(|line| line.text).collect())
    }

    /// `RunEnded`, its text scrubbed of every secret the run registered and cut to Neo's caps
    /// (`trace/redact.rs`).
    fn ended_event(
        &self,
        status: RunStatus,
        cause: EndCause,
        error: Option<&RunError>,
        value: Option<&Value>,
        lines: &[ConsoleLine],
    ) -> RunEvent {
        let scrubber = self.tap.secrets().scrubber();
        // Reject oversized input before copying it. Scrubbing can also expand small values
        // (replacement markers), so check the bounded copy again before emitting it.
        let fits = |value: &Value| redact::json_len(value) <= redact::MAX_VALUE_BYTES;
        let (value, value_truncated) = match value {
            None => (None, false),
            Some(value) => {
                let redacted = fits(value).then(|| scrubber.scrub_value(value));
                match redacted.filter(fits) {
                    Some(redacted) => (Some(Arc::new(redacted)), false),
                    None => (None, true),
                }
            }
        };
        RunEvent::RunEnded {
            run: self.tap.run(),
            status,
            cause,
            error: error.map(|error| RunError {
                name: scrubber.scrub_cut(&error.name, redact::MAX_ERROR_BYTES),
                message: scrubber.scrub_cut(&error.message, redact::MAX_ERROR_BYTES),
            }),
            value,
            value_truncated,
            logs: lines
                .iter()
                .map(|line| ConsoleLine {
                    offset_us: line.offset_us,
                    text: scrubber.scrub_cut(&line.text, redact::MAX_LINE_BYTES),
                })
                .collect(),
            script: self
                .script
                .as_deref()
                .map(|script| Arc::from(scrubber.scrub_cut(script, redact::MAX_SCRIPT_BYTES))),
            calls_made: self.trace.calls_made(),
            unreadable_calls: self.trace.unreadable_calls(),
            duration_us: self.tap.offset_us(),
        }
    }
}

impl Drop for RunGuard {
    /// Not ended means the caller dropped `run`'s future: no outcome will exist, so the run is
    /// reported here as cancelled, with the console lines written so far. The realm and the
    /// dispatcher may go before or after this; the calls they leave behind report themselves
    /// as they drop.
    fn drop(&mut self) {
        if self.ended || !self.tap.listening() {
            return;
        }
        let lines = self.logs.take();
        let ended = self.ended_event(RunStatus::Cancelled, EndCause::Dropped, None, None, &lines);
        self.tap.emit(&ended);
    }
}

/// How a stop during the tab's opening ended the run.
fn stopped_by(stop: Stop) -> EndCause {
    match stop {
        Stop::Timeout => EndCause::Deadline,
        Stop::Cancelled => EndCause::Cancelled,
    }
}
