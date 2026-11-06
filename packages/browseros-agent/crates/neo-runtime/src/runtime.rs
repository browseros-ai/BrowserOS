//! The entry point: owns the browser connection and runs one script per call against one tab.

use std::time::Duration;

use browseros_cdp::{CdpClient, CdpError, SessionId, browser, target};
use serde_json::Value;
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio::time;
use tracing::debug;

use crate::bridge::Getters;
use crate::deadline::Deadline;
use crate::getters::CLOSED_TAB_MESSAGE;
use crate::realm::Script;
use crate::run::{PageId, RunError, RunOutcome, RunRequest};
use crate::{bridge, getters, realm};

/// Detaching is best effort and must not hold up a finished run behind a hung browser.
const DETACH_TIMEOUT: Duration = Duration::from_secs(1);

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
    /// including a tab that is already closed, is a [`RunOutcome`] rather than an `Err`.
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

        // Bounded like the realm, but a run stopped here may leave the session it was
        // attaching behind; the browser drops it with the connection.
        let opened = tokio::select! {
            stop = deadline.reached() => {
                return RunOutcome::stopped(stop, deadline.timeout(), Vec::new());
            }
            opened = self.open(&page) => opened,
        };
        let (session, url) = match opened {
            Ok(opened) => opened,
            Err(error) => return RunOutcome::failed(error, Vec::new()),
        };

        let script = Script {
            code,
            page_guid: bridge::page_guid(&page),
            url,
        };
        let getters = Getters::new(
            self.cdp.clone(),
            page,
            session.clone(),
            deadline.clone(),
            tokio,
        );
        let outcome = realm::run(script, getters, deadline).await;
        self.detach(&session).await;
        outcome
    }

    /// Confirms the tab is open, attaches the run's session to it and reads its URL.
    async fn open(&self, page: &PageId) -> Result<(SessionId, String), RunError> {
        if getters::is_closed(&self.cdp, page)
            .await
            .map_err(|error| cdp_error(&error))?
        {
            return Err(RunError::target_closed());
        }
        let attached: target::AttachToTargetResult = self
            .cdp
            .send_typed(
                "Target.attachToTarget",
                target::AttachToTargetParams {
                    target_id: page.as_str().to_string(),
                    flatten: Some(true),
                },
                None,
            )
            .await
            .map_err(|error| cdp_error(&error))?;
        let session = SessionId::from(attached.session_id);
        match getters::url(&self.cdp, &session, page).await {
            Ok(Some(url)) => Ok((session, url)),
            Ok(None) => {
                self.detach(&session).await;
                Err(RunError::target_closed())
            }
            Err(error) => {
                self.detach(&session).await;
                Err(cdp_error(&error))
            }
        }
    }

    async fn detach(&self, session: &SessionId) {
        let detach = self.cdp.send_typed::<_, Value>(
            "Target.detachFromTarget",
            target::DetachFromTargetParams {
                session_id: Some(session.as_str().to_string()),
                target_id: None,
            },
            None,
        );
        match time::timeout(DETACH_TIMEOUT, detach).await {
            Ok(Ok(_)) => {}
            // Expected once the tab has closed: the session went with it.
            Ok(Err(error)) => debug!(%session, %error, "detaching a run's session failed"),
            Err(_) => debug!(%session, "detaching a run's session timed out"),
        }
    }
}

fn cdp_error(error: &CdpError) -> RunError {
    RunError::new("Error", error.to_string())
}
