//! The entry point: owns the browser connection and runs each script from its requested tab.
//!
//! A run's order, after the tab check and the attach:
//!
//! ```text
//!  Tabs ─▶ PageEntry ─▶ Arc<PageSession> ─┬─▶ realm::run(…, dispatcher)   the deadline decides, as before
//!                                             ├─▶ dispatcher.finish(reason)  cancel calls, wait ≤ 1 s, records
//!                                             └─▶ last Arc drops ─▶ Session::drop ─▶ detach in background
//! ```
//!
//! No step here detaches anything; ownership does (`page.rs`). The registry holds each page,
//! so sessions detach once the realm, dispatcher and their writers are gone, also when a
//! caller drops the run's future. Browser tabs themselves survive the run.

use browseros_cdp::{CdpClient, CdpError, browser};
use serde_json::Value;
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio_util::task::TaskTracker;

use crate::bridge;
use crate::bridge::Getters;
use crate::deadline::Deadline;
use crate::dispatch::Dispatcher;
use crate::getters::CLOSED_TAB_MESSAGE;
use crate::page::TabInputs;
use crate::progress::{ErrorName, RunScope};
use crate::realm;
use crate::realm::Script;
use crate::run::{RunEnd, RunError, RunOutcome, RunRequest, RunStatus};
use crate::tabs::Tabs;
#[cfg(feature = "test-host")]
use crate::test_host::TestHost;

/// A target id no tab can have, used to see how the browser reports a missing tab.
const PROBE_TARGET_ID: &str = "neo-runtime-connect-probe";

/// Runs agent scripts against tabs of one BrowserOS browser. Runs are independent: each gets
/// its own CDP session and its own QuickJS realm, so one `Runtime` serves any number of
/// concurrent runs. Runs on one tab share only its input: their clicks take turns at it, from
/// interceptor setup to stop (`page.rs` `TabInputs`). Clones share those turns too.
#[derive(Clone)]
pub struct Runtime {
    cdp: CdpClient,
    inputs: TabInputs,
    cleanup: TaskTracker,
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
                Ok(Self {
                    cdp,
                    inputs: TabInputs::default(),
                    cleanup: TaskTracker::new(),
                })
            }
            Err(CdpError::Protocol { message, .. }) => {
                Err(RuntimeError::UnsupportedBrowser(message))
            }
            Err(error) => Err(RuntimeError::Cdp(error)),
            Ok(reply) => Err(RuntimeError::UnsupportedBrowser(reply.to_string())),
        }
    }

    /// Waits for every run's remaining browser work, including late creation replies and
    /// their tab cleanup. Hosts must stop admitting runs and cancel/await active runs first,
    /// then drain before disconnecting CDP; a returned outcome alone does not settle writes.
    pub async fn drain(&self) {
        self.cleanup.close();
        self.cleanup.wait().await;
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

    /// Runs the same native script lifecycle as [`Self::run`], with an opaque test transport
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
        let RunRequest {
            code,
            page,
            timeout,
            cancel,
        } = request;
        let deadline = Deadline::new(timeout, cancel);
        let tokio = match Handle::try_current() {
            Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => handle,
            _ => {
                let message = "neo-runtime runs need tokio's multi-threaded runtime".to_string();
                return RunOutcome::failed(RunError::new("Error", message), Vec::new());
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
                return RunOutcome::stopped(stop, deadline.timeout(), Vec::new());
            }
            opened = Tabs::open(self.cdp.clone(), tokio.clone(), self.inputs.clone(), tasks.clone(), &page) => opened,
        };
        let (tabs, url) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                let error = if error.name == ErrorName::TargetClosedError {
                    RunError::target_closed()
                } else {
                    RunError::new(error.name.as_str(), error.message)
                };
                return RunOutcome::failed(error, Vec::new());
            }
        };

        let script = Script {
            code,
            page_guid: bridge::page_guid(&page),
            frame_guid: bridge::frame_guid(&page),
            url,
            #[cfg(feature = "test-host")]
            test_host,
        };
        // One tracker for the run: call tasks, and the writer, interceptor-stop and
        // group-release tasks the page spawns, so `finish` waits for them all.
        let getters = Getters::new(tabs.clone(), deadline.clone(), tokio);
        let run = RunScope::new(deadline.clone(), tasks);
        let dispatcher = Dispatcher::new(tabs.clone(), run);
        let outcome = realm::run(script, getters, dispatcher.clone(), deadline).await;
        let (calls, dropped) = dispatcher.finish(run_end_of(&outcome)).await;
        outcome
            .with_calls(calls, dropped)
            .with_pages(tabs.outcome())
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
