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

#[expect(
    clippy::disallowed_types,
    reason = "imports the host's client type for Runtime::connect, which hands it into chromium_bridge/cdp.rs"
)]
use browseros_cdp::{CdpClient, CdpError};
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio_util::task::TaskTracker;

use crate::artifacts::{AbsolutePaths, Artifacts};
use crate::browser_representations::cookies_and_storage::Permissions;
use crate::browser_representations::page::TabInputs;
use crate::browser_representations::tabs::Tabs;
use crate::chromium_bridge::browser_connection_settings::Switches;
use crate::chromium_bridge::cdp::browser::{GetTabInfo, TabExists};
use crate::chromium_bridge::cdp::{Browser, Command};
use crate::deadline::Deadline;
use crate::method_router::Dispatcher;
use crate::playwright_methods::getters::CLOSED_TAB_MESSAGE;
use crate::playwright_methods::screenshot::TabScreenshots;
use crate::progress::{ErrorName, RunScope};
use crate::quickjs;
use crate::quickjs::Script;
use crate::quickjs_bridge;
use crate::quickjs_bridge::Getters;
use crate::run::{RunEnd, RunError, RunOutcome, RunRequest, RunStatus};
#[cfg(feature = "test-host")]
use crate::test_host::TestHost;

/// A target id no tab can have, used to see how the browser reports a missing tab.
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
    /// The door over the client the host connected: the one place neo-runtime holds it.
    browser: Browser,
    inputs: TabInputs,
    screenshots: TabScreenshots,
    /// Target discovery and download reports on the connection, each on while any run listens for them.
    switches: Switches,
    /// The runs' permission grants on the connection, reset once no run holds one (`cookies_and_storage.rs`).
    permissions: Permissions,
    /// How every run's `path` options treat an absolute path (`artifacts.rs`).
    absolute_paths: AbsolutePaths,
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
        "{method} does not report a missing tab as {CLOSED_TAB_MESSAGE:?}; is this BrowserOS? It answered: {0}",
        method = TabExists::METHOD
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
    #[expect(
        clippy::disallowed_types,
        reason = "the runtime hands its host's connected client straight into chromium_bridge/cdp.rs"
    )]
    pub async fn connect(cdp: CdpClient) -> Result<Self, RuntimeError> {
        let browser = Browser::new(cdp);
        let probe = browser
            .send(TabExists(GetTabInfo {
                target_id: Some(PROBE_TARGET_ID.to_string()),
                tab_id: None,
            }))
            .await;
        match probe {
            Err(CdpError::Protocol { message, .. }) if message.contains(CLOSED_TAB_MESSAGE) => {
                let cleanup = TaskTracker::new();
                Ok(Self {
                    browser,
                    inputs: TabInputs::default(),
                    screenshots: TabScreenshots::default(),
                    switches: Switches::new(cleanup.clone()),
                    permissions: Permissions::new(cleanup.clone()),
                    absolute_paths: AbsolutePaths::default(),
                    cleanup,
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
        let RunRequest {
            code,
            page,
            timeout,
            cancel,
            artifacts,
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
            opened = Tabs::open(self.browser.clone(), tokio.clone(), self.inputs.clone(), self.screenshots.clone(), self.switches.clone(), self.permissions.clone(), tasks.clone(), &page) => opened,
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
        let run = RunScope::new(deadline.clone(), tasks).with_artifacts(Arc::clone(&artifacts));
        let getters = Getters::new(
            tabs.clone(),
            deadline.clone(),
            tokio,
            Arc::clone(&run.handles),
        );
        let dispatcher = Dispatcher::new(tabs.clone(), run);
        let events = tabs.events().clone();
        // A run whose future is dropped lets go of its permission grants on the cleanup tracker.
        let grants = Grants {
            permissions: self.permissions.clone(),
            browser: self.browser.clone(),
            run: tabs.run_id(),
            released: false,
        };
        let outcome = quickjs::run(script, getters, dispatcher.clone(), deadline, events).await;
        let (calls, dropped) = dispatcher.finish(run_end_of(&outcome)).await;
        // `Dispatcher::finish` let go of the grants (`Tabs::release_run_scoped`).
        grants.disarm();
        outcome
            .with_calls(calls, dropped)
            .with_pages(tabs.outcome())
            .with_artifacts(artifacts.written(), artifacts.notes())
    }
}

/// A run's hold on its permission grants (`cookies_and_storage.rs` `Permissions`), for a run whose
/// future is dropped before `Dispatcher::finish` let go of them (`Tabs::release_run_scoped`,
/// before the outcome returns, so the next run finds the browser's permissions as they were):
/// its grants are then released on the cleanup tracker.
struct Grants {
    permissions: Permissions,
    browser: Browser,
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
            self.permissions
                .release_later(self.browser.clone(), self.run);
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
