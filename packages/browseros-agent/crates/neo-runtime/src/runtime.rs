//! The entry point: owns the browser connection and runs one script per call against one tab.
//!
//! A run's order, after the tab check and the attach:
//!
//! ```text
//!  Session ─▶ Arc<PageSession> ─▶ Dispatcher ─┬─▶ realm::run(…, dispatcher)   the deadline decides, as before
//!                                             ├─▶ dispatcher.finish(reason)  cancel calls, wait ≤ 1 s, records
//!                                             └─▶ last Arc drops ─▶ Session::drop ─▶ detach in background
//! ```
//!
//! No step here detaches anything; ownership does (`page.rs`). The dispatcher holds the page,
//! so the session detaches once the realm and the dispatcher are gone, also when a caller drops
//! the run's future.

use std::sync::Arc;

use browseros_cdp::{CdpClient, CdpError, browser};
use serde_json::Value;
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio_util::task::TaskTracker;

use crate::bridge::Getters;
use crate::deadline::Deadline;
use crate::dispatch::Dispatcher;
use crate::getters::CLOSED_TAB_MESSAGE;
use crate::page::{PageSession, Session};
use crate::progress::RunScope;
use crate::realm::Script;
use crate::run::{PageId, RunEnd, RunError, RunOutcome, RunRequest, RunStatus};
use crate::{bridge, getters, realm};

/// A target id no tab can have, used to see how the browser reports a missing tab.
const PROBE_TARGET_ID: &str = "neo-runtime-connect-probe";

/// Runs agent scripts against tabs of one BrowserOS browser. Runs are independent: each gets
/// its own CDP session and its own QuickJS realm, so one `Runtime` serves any number of
/// concurrent runs.
#[derive(Clone)]
pub struct Runtime {
    cdp: CdpClient,
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
                Ok(Self { cdp })
            }
            Err(CdpError::Protocol { message, .. }) => {
                Err(RuntimeError::UnsupportedBrowser(message))
            }
            Err(error) => Err(RuntimeError::Cdp(error)),
            Ok(reply) => Err(RuntimeError::UnsupportedBrowser(reply.to_string())),
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

        let opened = tokio::select! {
            stop = deadline.reached() => {
                return RunOutcome::stopped(stop, deadline.timeout(), Vec::new());
            }
            opened = self.open(&page, &tokio) => opened,
        };
        let (session, url) = match opened {
            Ok(opened) => opened,
            Err(error) => return RunOutcome::failed(error, Vec::new()),
        };

        let script = Script {
            code,
            page_guid: bridge::page_guid(&page),
            frame_guid: bridge::frame_guid(&page),
            url,
        };
        // One tracker for the run: call tasks, and the writer, interceptor-stop and
        // group-release tasks the page spawns, so `finish` waits for them all.
        let tasks = TaskTracker::new();
        let tab = Arc::new(PageSession::new(session, &page, tasks.clone()));
        let getters = Getters::new(
            self.cdp.clone(),
            page.clone(),
            tab.session_id().clone(),
            deadline.clone(),
            tokio,
        );
        let run = RunScope::new(deadline.clone(), tasks);
        let dispatcher = Dispatcher::new(tab, run, &page);
        let outcome = realm::run(script, getters, dispatcher.clone(), deadline).await;
        let (calls, dropped) = dispatcher.finish(run_end_of(&outcome)).await;
        outcome.with_calls(calls, dropped)
    }

    /// Confirms the tab is open, attaches the run's session to it and reads its URL.
    async fn open(&self, page: &PageId, tokio: &Handle) -> Result<(Session, String), RunError> {
        if getters::is_closed(&self.cdp, page)
            .await
            .map_err(|error| cdp_error(&error))?
        {
            return Err(RunError::target_closed());
        }
        // The attach runs as its own task so that its answer is read even when the run stops
        // first and drops this future. The browser creates the session either way; awaiting
        // the send here would throw that answer away with the future and leak the session.
        // When nobody awaits the task any more, tokio drops its output, the `Session`, and
        // dropping it detaches.
        let session = tokio
            .spawn(Session::attach(
                self.cdp.clone(),
                page.clone(),
                tokio.clone(),
            ))
            .await
            .map_err(|error| RunError::new("Error", format!("attaching to the tab: {error}")))?
            .map_err(|error| cdp_error(&error))?;
        match getters::url(&self.cdp, &session.id, page).await {
            Ok(Some(url)) => Ok((session, url)),
            Ok(None) => Err(RunError::target_closed()),
            Err(error) => Err(cdp_error(&error)),
        }
    }
}

fn cdp_error(error: &CdpError) -> RunError {
    RunError::new("Error", error.to_string())
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
