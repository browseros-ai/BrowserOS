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
//!
//! Neo: an optional host emitter surrounds the run lifetime so failed admission checks and dropped futures still report a matched start and end.
//! Around all of it, a runtime with a host side reports the run through its `RunEmitter`
//! (`host/emit.rs`): `RunStarted` before the tab check, `RunEnded` after the outcome, or from
//! the emitter's guard when the run's future is dropped.

use std::sync::Arc;

use browseros_cdp::{CdpClient, CdpError, browser};
use serde_json::Value;
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio_util::sync::DropGuard;
use tokio_util::task::TaskTracker;

use crate::artifacts::{AbsolutePaths, Artifacts};
use crate::browser_representations::cookies_and_storage::Permissions;
use crate::browser_representations::page::TabInputs;
use crate::browser_representations::tabs::Tabs;
use crate::chromium_bridge::browser_connection_settings::Switches;
use crate::deadline::Deadline;
use crate::host::{Host, HostError, HostOptions, RunEmitter};
use crate::method_router::Dispatcher;
use crate::playwright_methods::getters::CLOSED_TAB_MESSAGE;
use crate::playwright_methods::screenshot::TabScreenshots;
use crate::progress::{ErrorName, RunScope};
use crate::quickjs;
use crate::quickjs::Script;
use crate::quickjs_bridge;
use crate::quickjs_bridge::Getters;
use crate::run::{RunError, RunId, RunOutcome, RunRequest};

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
    // Neo: a separate runtime lifetime guard cancels the host actor when the last Runtime clone drops, breaking the actor/Host ownership cycle.
    _host_lifetime: Option<Arc<DropGuard>>,
}

/// Close the run's tracker on every exit, including a dropped future or failed tab open.
/// The runtime's cleanup waiter must be able to finish on those paths too.
struct CloseRunTasks(TaskTracker);

impl Drop for CloseRunTasks {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// Neo: opt-in host wiring lets servers add the ledger and capture socket without changing script-only runtimes.
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
    Host(#[from] HostError),
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

    /// Neo: start the host socket and actor only when the caller supplies host options, after validating the run client's browser.
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
                let host = match options.host {
                    Some(options) => Some(Host::connect(options).await?),
                    None => None,
                };
                let lifetime = host.as_ref().map(|host| Arc::new(host.lifetime()));
                let cleanup = TaskTracker::new();
                Ok(Self {
                    cdp,
                    inputs: TabInputs::default(),
                    screenshots: TabScreenshots::default(),
                    switches: Switches::new(cleanup.clone()),
                    permissions: Permissions::new(cleanup.clone()),
                    absolute_paths: AbsolutePaths::default(),
                    cleanup,
                    host,
                    _host_lifetime: lifetime,
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

    /// Neo: server shutdown drains runs before detaching presences, preserving focus through outstanding input cleanup.
    /// Cancels the host side and waits for its ledger writer to stop. For server shutdown,
    /// first stop admission, cancel and await runs, then call [`Runtime::drain`], and finally
    /// this method. Draining before releasing host presences prevents their detach from
    /// clearing focus under outstanding input cleanup.
    ///
    /// The run client belongs to the caller; this method does not disconnect it or cancel runs.
    pub async fn shutdown(&self) {
        if let Some(host) = &self.host {
            host.shutdown();
            host.stopped().await;
        }
    }

    /// Emitter tests deliberately leave the FIFO undrained to prove overflow cannot affect a
    /// run. Production startup always connects the host link and starts its consumer instead.
    #[cfg(test)]
    pub(crate) fn with_undrained_host(mut self, options: HostOptions) -> Self {
        self.host = Some(Host::new(options));
        self
    }

    /// Runs against the same integrated host actor a fake-browser test deliberately stalls.
    #[cfg(test)]
    pub(crate) fn with_host_for_test(mut self, host: Host) -> Self {
        self.host = Some(host);
        self
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
    ///
    /// With a host side and a `request.session`, the run reports to the host as it goes:
    /// `RunStarted` before anything else, `RunEnded` once it is over, also when the caller drops
    /// this future, which then ends the run as `Cancelled`.
    pub async fn run(&self, request: RunRequest) -> RunOutcome {
        let run_id = RunId::mint();
        let emitter = RunEmitter::new(self.host.as_ref(), request.session.as_ref(), &run_id);
        let ending = emitter.run_started(&request.page);
        let outcome = self.run_script(run_id, request, emitter).await;
        ending.ended(outcome.status);
        outcome
    }

    /// Neo: run execution carries the same id and emitter minted before tab validation, so every early outcome remains attributable.
    async fn run_script(
        &self,
        run_id: RunId,
        request: RunRequest,
        emitter: RunEmitter,
    ) -> RunOutcome {
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
        let run =
            RunScope::new(deadline.clone(), tasks, emitter).with_artifacts(Arc::clone(&artifacts));
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
        let (calls, dropped) = dispatcher.finish(outcome.status.run_end()).await;
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

#[cfg(test)]
mod tests {
    use crate::fake_host_cdp::FakeHostCdp;
    use crate::host::MemoryStore;
    use std::time::Duration;

    use crate::host::HostStore;
    use tokio::task;
    use tokio::time;
    use tokio::time::Instant;
    use tokio_util::sync::CancellationToken;
    use ulid::Ulid;

    use super::*;
    use crate::fake_browser::{self, FakeBrowser, Replies};
    use crate::run::{PageId, RunStatus};

    #[tokio::test(flavor = "multi_thread")]
    async fn only_a_runtime_given_host_options_has_a_host_side() {
        let browser = FakeBrowser::start(Replies::default()).await;
        assert!(browser.runtime().await.host().is_none());
        let host_browser = FakeHostCdp::start().await;
        let options = RuntimeOptions {
            host: Some(HostOptions::new(
                host_browser.port(),
                1,
                Arc::new(MemoryStore::default()),
            )),
        };
        let hosted = Runtime::connect_with(browser.cdp().await, options)
            .await
            .unwrap();
        assert!(hosted.host().is_some());
        host_browser.wait_for("Browser.getTabs", 1).await;
        hosted.shutdown().await;
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
    #[tokio::test(flavor = "multi_thread")]
    async fn a_dropped_run_completes_through_the_host_task_and_after_call_worker() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let host_browser = FakeHostCdp::start().await;
        host_browser.set_tabs(vec![FakeHostCdp::tab(
            fake_browser::PAGE,
            41,
            fake_browser::PAGE_URL,
        )]);
        let store = Arc::new(MemoryStore::default());
        let runtime = Runtime::connect_with(
            browser.cdp().await,
            RuntimeOptions {
                host: Some(HostOptions::new(
                    host_browser.port(),
                    1,
                    Arc::clone(&store) as Arc<dyn HostStore>,
                )),
            },
        )
        .await
        .unwrap();
        let host = runtime.host().unwrap();
        let pass = host.admit_run(None).await;
        let key = pass.session().clone();
        host.withhold(&key).await.unwrap();
        let running = tokio::spawn({
            let runtime = runtime.clone();
            let key = key.clone();
            async move {
                let mut request = fake_browser::run_request(
                    "await page.click('#go', {delay:5000})",
                    Duration::from_secs(10),
                    CancellationToken::new(),
                );
                request.session = Some(key);
                runtime.run(request).await
            }
        });
        time::timeout(Duration::from_secs(5), async {
            while !browser
                .exchanges()
                .iter()
                .any(|exchange| exchange.command["params"]["type"] == "mousePressed")
            {
                task::yield_now().await;
            }
        })
        .await
        .unwrap();
        host.end_session(&key);
        assert!(!host.view(&key).unwrap().complete);
        running.abort();
        assert!(running.await.unwrap_err().is_cancelled());
        drop(pass);
        let view = host
            .wait_complete(&key, Instant::now() + Duration::from_secs(10))
            .await
            .unwrap();
        assert!(
            view.complete,
            "cancelled run and call jobs settled before the store fence"
        );
        assert!(!view.live);
        assert!(!host_browser.sent("Browser.getTabInfo").is_empty());
        runtime.shutdown().await;
        assert!(
            runtime.cdp.is_connected(),
            "host shutdown leaves the run client alone"
        );
    }
}
