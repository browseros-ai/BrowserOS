//! The data a caller hands `Runtime::run` and gets back: one agent script, one tab, one outcome.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::deadline::Stop;

/// The CDP target id of a tab. In the script's realm the page is `page@<id>` and its main
/// frame is `frame@<id>`, because Chromium gives a page's main frame its target's id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PageId(String);

impl PageId {
    #[must_use]
    pub fn new(target_id: String) -> Self {
        Self(target_id)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One script to run against one tab.
#[derive(Debug, Clone)]
pub struct RunRequest {
    /// The body of an async function that receives `page` and `context`, as in a Playwright
    /// script: it may `await` and `return`.
    pub code: String,
    pub page: PageId,
    /// Wall-clock budget for the whole run, from the tab check to the script's return.
    pub timeout: Duration,
    /// Cancelling ends the run with [`RunStatus::Cancelled`] at the script's next instruction,
    /// timer or getter.
    pub cancel: CancellationToken,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Ok,
    Error,
    Timeout,
    Cancelled,
}

/// A script failure, named the way Playwright names it where Playwright has a name
/// (`TargetClosedError`, `TimeoutError`), and otherwise by the JS error's own `name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunError {
    pub name: String,
    pub message: String,
}

impl RunError {
    pub(crate) fn new(name: &str, message: String) -> Self {
        Self {
            name: name.to_string(),
            message,
        }
    }

    /// Playwright's error, and wording, for running out of time.
    pub(crate) fn timeout(timeout: Duration) -> Self {
        Self::new(
            "TimeoutError",
            format!("Timeout {}ms exceeded.", timeout.as_millis()),
        )
    }

    /// Playwright's error for a page that closed before or while it was used.
    pub(crate) fn target_closed() -> Self {
        Self::new(
            "TargetClosedError",
            "Target page, context or browser has been closed".to_string(),
        )
    }
}

/// The result of a run. `value` is set only for [`RunStatus::Ok`] when the script returned
/// something other than `undefined`; `error` is set for [`RunStatus::Error`] and
/// [`RunStatus::Timeout`]. `logs` holds the script's `console` lines, including those written
/// before a failure, up to 1000 lines and 1 MiB of text; past that, the run keeps no more lines
/// and the last entry is a note from neo-runtime saying how many it dropped.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunOutcome {
    pub status: RunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    pub logs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RunError>,
}

impl RunOutcome {
    pub(crate) fn returned(value: Option<Value>, logs: Vec<String>) -> Self {
        Self {
            status: RunStatus::Ok,
            value,
            logs,
            error: None,
        }
    }

    pub(crate) fn failed(error: RunError, logs: Vec<String>) -> Self {
        Self {
            status: RunStatus::Error,
            value: None,
            logs,
            error: Some(error),
        }
    }

    pub(crate) fn stopped(stop: Stop, timeout: Duration, logs: Vec<String>) -> Self {
        let (status, error) = match stop {
            Stop::Timeout => (RunStatus::Timeout, Some(RunError::timeout(timeout))),
            Stop::Cancelled => (RunStatus::Cancelled, None),
        };
        Self {
            status,
            value: None,
            logs,
            error,
        }
    }
}
