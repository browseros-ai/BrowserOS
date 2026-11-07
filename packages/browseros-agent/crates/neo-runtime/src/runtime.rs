//! The entry point: owns the browser connection and runs each script from its requested tab.
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
use crate::deadline::Deadline;
use crate::host::{Host, HostOptions};
use crate::method_router::Dispatcher;
use crate::playwright_methods::getters::CLOSED_TAB_MESSAGE;
use crate::playwright_methods::screenshot::TabScreenshots;
use crate::progress::{ErrorName, RunScope};
use crate::quickjs;
use crate::quickjs::Script;
use crate::quickjs_bridge;
use crate::quickjs_bridge::Getters;
use crate::run::{RunEnd, RunError, RunId, RunOutcome, RunRequest, RunStatus};

/// A target id no tab can have, used to see how the browser reports a missing tab.
const PROBE_TARGET_ID: &str = "neo-runtime-connect-probe";

/// Runs agent scripts against tabs of one BrowserOS browser. Runs are independent: each gets
/// its own CDP session and its own QuickJS realm, so one `Runtime` serves any number of
/// concurrent runs. Runs on one tab share only its input and its screenshots: their clicks take
/// turns at its input, from interceptor setup to stop (`page.rs` `TabInputs`), and their
/// screenshots at its capture, cleanup included (`screenshot.rs` `TabScreenshots`). Clones share
/// those turns and the optional host side too.
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
    host: Option<Host>,
}

/// Close the run's tracker on every exit, including a dropped future or failed tab open.
/// The runtime's cleanup waiter must be able to finish on those paths too.
struct CloseRunTasks(TaskTracker);

impl Drop for CloseRunTasks {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// How [`Runtime::connect_with`] builds a runtime. The default runs scripts and nothing else.
#[derive(Clone, Debug, Default)]
pub struct RuntimeOptions {
    /// Gives the runtime a host side ([`Runtime::host`]). `None` adds no host state, no events
    /// and no connection.
    pub host: Option<HostOptions>,
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
    /// `page.isClosed()` expects. The runtime has no host side.
    ///
    /// # Errors
    ///
    /// As [`Self::connect_with`].
    pub async fn connect(cdp: CdpClient) -> Result<Self, RuntimeError> {
        Self::connect_with(cdp, RuntimeOptions::default()).await
    }

    /// Like [`Self::connect`], with a host side when `options.host` asks for one.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::UnsupportedBrowser`] when `Browser.getTabInfo` is missing or reports
    /// an unknown tab differently, and [`RuntimeError::Cdp`] when the check itself fails.
    pub async fn connect_with(
        cdp: CdpClient,
        options: RuntimeOptions,
    ) -> Result<Self, RuntimeError> {
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
                    host: options.host.map(Host::new),
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

    /// The host side, when the runtime was built with one.
    #[must_use]
    pub fn host(&self) -> Option<&Host> {
        self.host.as_ref()
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
        let run_id = RunId::mint();
        let RunRequest {
            code,
            page,
            timeout,
            cancel,
            artifacts,
            session: _,
        } = request;
        let deadline = Deadline::new(timeout, cancel);
        let tokio = match Handle::try_current() {
            Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => handle,
            _ => {
                let message = "neo-runtime runs need tokio's multi-threaded runtime".to_string();
                return RunOutcome::failed(run_id, RunError::new("Error", message), Vec::new());
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
                return RunOutcome::stopped(run_id, stop, deadline.timeout(), Vec::new());
            }
            opened = Tabs::open(self.cdp.clone(), tokio.clone(), self.inputs.clone(), self.screenshots.clone(), self.switches.clone(), self.permissions.clone(), tasks.clone(), &page) => opened,
        };
        let (tabs, url) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                let error = if error.name == ErrorName::TargetClosedError {
                    RunError::target_closed()
                } else {
                    RunError::new(error.name.as_str(), error.message)
                };
                return RunOutcome::failed(run_id, error, Vec::new());
            }
        };

        let script = Script {
            code,
            page_guid: quickjs_bridge::page_guid(&page),
            frame_guid: quickjs_bridge::frame_guid(&page),
            url,
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
        )
        .await;
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio_util::sync::CancellationToken;
    use ulid::Ulid;

    use super::*;
    use crate::fake_browser::{self, FakeBrowser, Replies};
    use crate::run::PageId;

    #[tokio::test(flavor = "multi_thread")]
    async fn only_a_runtime_given_host_options_has_a_host_side() {
        let browser = FakeBrowser::start(Replies::default()).await;
        assert!(browser.runtime().await.host().is_none());
        let options = RuntimeOptions {
            host: Some(HostOptions::new(9222, 1)),
        };
        let hosted = Runtime::connect_with(browser.cdp().await, options)
            .await
            .unwrap();
        assert!(hosted.host().is_some());
    }

    /// A run that never reaches its tab is a run too: it gets an id of its own, and the outcome's
    /// JSON carries the id as `runId`.
    #[tokio::test(flavor = "multi_thread")]
    async fn each_run_is_named_by_a_fresh_ulid() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let runtime = browser.runtime().await;
        let request = || {
            fake_browser::run_request("return 1", Duration::from_secs(5), CancellationToken::new())
        };
        let ran = runtime.run(request()).await;
        let mut closed = request();
        closed.page = PageId::new("closed-tab".to_string());
        let refused = runtime.run(closed).await;

        assert_eq!(ran.status, RunStatus::Ok, "{ran:?}");
        assert_eq!(refused.status, RunStatus::Error, "{refused:?}");
        assert_ne!(ran.run_id, refused.run_id);
        for outcome in [&ran, &refused] {
            let id = outcome.run_id.as_str();
            assert!(Ulid::from_string(id).is_ok(), "{id} is a ULID");
            assert_eq!(serde_json::to_value(outcome).unwrap()["runId"], id);
        }
    }
}
