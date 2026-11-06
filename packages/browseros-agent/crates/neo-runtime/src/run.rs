//! A script, its starting tab, and the outcome with every created or handed-out page.

use std::time::Duration;

use serde::{Serialize, Serializer};
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

/// One script to start from a tab; its context may hand it additional pages.
#[derive(Debug, Clone)]
pub struct RunRequest {
    /// The body of an async function that receives `page` and `context`, as in a Playwright
    /// script: it may `await` and `return`.
    pub code: String,
    pub page: PageId,
    /// Wall-clock budget from the tab check to the script's return. Finish adds one bounded
    /// drain; cleanup tasks still pending then continue in the background.
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
///
/// `calls` holds one record per browser call the script made, in arrival order, including calls
/// it never awaited and calls still running when the run ended. It keeps the first 1000 calls
/// and up to 1 MiB of records (params plus call log). Past either cap, `calls_dropped` counts
/// the rest and keeps the least safe delivery among them, so a host deciding whether a run may
/// be repeated never misses a write. The caps exist for the reason `logs` has one: records live
/// in Rust, outside the realm's memory limit. For the same reason a run may have at most 64
/// browser calls, holding at most 8 MiB of requests, unfinished at once; a call past either
/// cap fails at once with an `Error` naming it, and is recorded like any other.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunOutcome {
    pub status: RunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    pub logs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RunError>,
    pub calls: Vec<CallRecord>,
    /// Pages handed out or acknowledged before finish's bounded drain ends, in first-seen
    /// order with the run page first. Creations acknowledged later are closed in the
    /// background. IDs can be passed to a later `RunRequest`; successful tabs outlive runs.
    pub pages: Vec<RunPage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calls_dropped: Option<DroppedCalls>,
}

impl RunOutcome {
    pub(crate) fn returned(value: Option<Value>, logs: Vec<String>) -> Self {
        Self {
            status: RunStatus::Ok,
            value,
            logs,
            error: None,
            calls: Vec::new(),
            pages: Vec::new(),
            calls_dropped: None,
        }
    }

    pub(crate) fn failed(error: RunError, logs: Vec<String>) -> Self {
        Self {
            status: RunStatus::Error,
            value: None,
            logs,
            error: Some(error),
            calls: Vec::new(),
            pages: Vec::new(),
            calls_dropped: None,
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
            calls: Vec::new(),
            pages: Vec::new(),
            calls_dropped: None,
        }
    }

    /// Attaches the run's call records. `Runtime::run` calls it last, after the realm is gone
    /// and `Dispatcher::finish` has ended every call.
    pub(crate) fn with_calls(
        self,
        calls: Vec<CallRecord>,
        calls_dropped: Option<DroppedCalls>,
    ) -> Self {
        Self {
            calls,
            calls_dropped,
            ..self
        }
    }

    pub(crate) fn with_pages(self, pages: Vec<RunPage>) -> Self {
        Self { pages, ..self }
    }
}

/// A page identity handed out or created by the run, and the lifecycle changes it made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPage {
    pub id: String,
    pub created: bool,
    pub closed: bool,
}

/// One browser call (`__neo_runtime.call`), as the dispatcher saw it. Derived when the run ends
/// from state kept outside the realm, so a call cut off by the run's deadline or by the script
/// returning still has its record, its call log and its true delivery.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallRecord {
    /// Arrival order within the run, from 1. The wire `id` is the SDK's and is not kept.
    pub call_id: u64,
    /// The object the call named, such as `frame@<targetId>`.
    pub guid: String,
    /// Playwright's wire method, such as `click`.
    pub method: String,
    /// Playwright's trace title for the method, such as `Click`; `None` when the call named no
    /// method Neo serves on that object. Neo records no `apiName`: it never crosses the wire.
    pub title: Option<String>,
    /// The params exactly as the SDK sent them.
    pub params: Value,
    /// Where the agent's script made the call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<CallLocation>,
    /// Wall clock at arrival, in Unix milliseconds.
    pub started_at_ms: u64,
    #[serde(serialize_with = "serialize_millis")]
    pub duration: Duration,
    pub result: CallResult,
    /// Set for methods Playwright's spec flags `input`, `None` for reads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<Delivery>,
    /// The call log folded as Playwright's error messages show it, kept for successful calls
    /// too.
    pub log: Vec<String>,
}

/// `metadata.location` of a call, without its file, which is always the agent's `run.js`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CallLocation {
    /// 1-based, counted in the agent's script.
    pub line: u32,
    /// 1-based.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

/// How a browser call ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CallResult {
    Ok,
    /// The Playwright error the call replied with: its wire `name` (`TimeoutError`,
    /// `TargetClosedError`, `Error`) and message, without the client's `apiName` prefix or the
    /// call log.
    Error {
        name: String,
        message: String,
    },
    /// The run ended while the call was still running, so it never replied.
    Unfinished {
        reason: RunEnd,
    },
}

/// Why a run ended under a call that was still running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RunEnd {
    /// The script returned or threw without awaiting the call.
    Settled,
    Timeout,
    /// The host cancelled the run, or dropped its future (then no outcome is returned).
    Cancelled,
}

/// Whether an input call's events reached the page. Derived only from what the call recorded
/// as it wrote input, never from how the call ended: a click that timed out after every mouse
/// event was acknowledged is `Done`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Delivery {
    /// No input reached the page's handlers: none was written, or Neo verifiably blocked it.
    /// A press Neo's hit-target interceptor blocked still moved the pointer, and listeners the
    /// page added to `window` before Neo's script saw its events; nothing past `window` did.
    NotSent,
    /// Every input event was written and acknowledged.
    Done,
    /// Some input event was written and not every acknowledgement came back. Never repeat a
    /// call in this state blindly. Ordered last so `max` is the least safe delivery.
    Unknown,
}

/// The calls a run made past the record caps. They ran normally; only their records were not
/// kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DroppedCalls {
    pub count: u64,
    /// The least safe delivery among the dropped input calls; `None` if none was an input call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<Delivery>,
}

/// A duration as whole milliseconds, as the rest of the outcome's JSON counts time.
fn serialize_millis<S: Serializer>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_u128(duration.as_millis())
}
