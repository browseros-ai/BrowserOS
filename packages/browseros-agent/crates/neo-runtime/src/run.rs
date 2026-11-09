//! A script, its starting tab, and the outcome with every created or handed-out page.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures_util::future::BoxFuture;
use serde::{Serialize, Serializer};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::artifacts::RunArtifact;
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
    /// drain, and one more bounded wait to remove the run's highlights; cleanup tasks still
    /// pending then continue in the background, the run's input releases among them.
    pub timeout: Duration,
    /// Cancelling ends the run with [`RunStatus::Cancelled`] at the script's next instruction,
    /// timer or getter.
    pub cancel: CancellationToken,
    /// The directory the run's `path` options write into (`page.screenshot({ path })`,
    /// `page.pdf({ path })`), created on first write. A `path` must be relative to it and stay
    /// inside it; a runtime opted into `AbsolutePaths::Redirect` also takes absolute paths
    /// (`Runtime::with_absolute_paths`). `None` refuses every `path` with an Error naming the
    /// option; the bytes a script asked for are still returned. Files written are listed in
    /// [`RunOutcome::artifacts`].
    pub artifacts: Option<PathBuf>,
    /// The run's hook file, run in a sandbox of its own next to the script (`hooks.rs`). `None`
    /// creates no sandbox and costs nothing.
    pub hooks: Option<RunHooks>,
}

/// Neo: how long one hook invocation may take, its loading included, unless the host sets
/// another ([`RunHooks::with_budget`]). The hooks design's assumed per-invocation budget.
pub const HOOK_BUDGET: Duration = Duration::from_secs(5);

/// What a hook's `ctx.host(name, args)` asks the run's host. Hooks feature 5 calls the
/// callback; until then `ctx.host` rejects inside the hook and the callback is never called.
pub type HostCallback =
    Arc<dyn Fn(HostRequest) -> BoxFuture<'static, Result<Value, String>> + Send + Sync>;

/// One `ctx.host(name, args)` request from a hook.
#[derive(Debug, Clone, PartialEq)]
pub struct HostRequest {
    pub name: String,
    pub args: Value,
}

/// A run's hook file: the body of an async function that receives `neo` and registers handlers
/// with `neo.on(event, handler)`. Neo runs it once, in a second QuickJS realm of the run's own,
/// before the script starts. `beforeRun` may stop the script; `afterRun` may replace its result
/// or add a note. The script can neither see nor reach the hooks.
#[derive(Clone)]
pub struct RunHooks {
    pub code: String,
    /// The run's `meta`, handed to every handler as `ctx.meta` (`null` when the host sends none).
    pub meta: Value,
    /// Who answers `ctx.host`; `None` makes every `ctx.host` reject.
    pub host: Option<HostCallback>,
    /// Each invocation's budget, loading included ([`HOOK_BUDGET`] by default). A hook that
    /// spins or waits past it fails, and a failed hook fails closed.
    pub budget: Duration,
}

impl RunHooks {
    /// Hooks with no `meta`, no host and the default budget.
    #[must_use]
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            meta: Value::Null,
            host: None,
            budget: HOOK_BUDGET,
        }
    }

    #[must_use]
    pub fn with_meta(self, meta: Value) -> Self {
        Self { meta, ..self }
    }

    #[must_use]
    pub fn with_host(self, host: HostCallback) -> Self {
        Self {
            host: Some(host),
            ..self
        }
    }

    #[must_use]
    pub fn with_budget(self, budget: Duration) -> Self {
        Self { budget, ..self }
    }
}

impl fmt::Debug for RunHooks {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunHooks")
            .field("code", &self.code)
            .field("meta", &self.meta)
            .field("host", &self.host.is_some())
            .field("budget", &self.budget)
            .finish()
    }
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Ok,
    Error,
    Timeout,
    Cancelled,
    /// The run's hooks stopped the script before it started: a `beforeRun` handler blocked it,
    /// or the hooks failed to load or to decide (they fail closed).
    Blocked,
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
/// something other than `undefined`; `error` is set for [`RunStatus::Error`],
/// [`RunStatus::Timeout`] and [`RunStatus::Blocked`]. `logs` holds the script's `console` lines, including those written
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
    /// Every `Uint8Array` (a `Buffer` included) in `value`, in the order JSON writes them. Its
    /// place in `value` holds `{"type":"Buffer"|"Uint8Array","bytes":<length>}`; its bytes are
    /// here, never spread one number per byte as Node's `Buffer.toJSON` would. Absent from the
    /// JSON when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub binaries: Vec<RunBinary>,
    /// The files the run wrote into its artifact directory ([`RunRequest::artifacts`]), in the
    /// order first written, each once with its last size. Absent from the JSON when empty. A
    /// file written somewhere other than the absolute path the script named also leaves a
    /// `neo:` line in `logs` saying where it went.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<RunArtifact>,
    /// The notes the run's `afterRun` hooks added, in the order they were added, for the model
    /// to read. Absent from the JSON when empty, so a run without hooks reads as before.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// The hook trace, for the host only and never in the outcome's JSON: every `ctx.trace` and
    /// hook `console` line, and why a hook failed (a hook's error text may hold a secret, so the
    /// model-visible `error` names no detail). Capped as `logs` is.
    #[serde(skip)]
    pub hook_trace: Vec<String>,
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
            binaries: Vec::new(),
            artifacts: Vec::new(),
            notes: Vec::new(),
            hook_trace: Vec::new(),
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
            binaries: Vec::new(),
            artifacts: Vec::new(),
            notes: Vec::new(),
            hook_trace: Vec::new(),
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
            binaries: Vec::new(),
            artifacts: Vec::new(),
            notes: Vec::new(),
            hook_trace: Vec::new(),
        }
    }

    /// The run's hooks stopped the script before it started, with `error` saying why.
    pub(crate) fn blocked(error: RunError) -> Self {
        Self {
            status: RunStatus::Blocked,
            error: Some(error),
            ..Self::returned(None, Vec::new())
        }
    }

    /// Adds the notes the run's `afterRun` hooks left for the model.
    pub(crate) fn with_notes(self, notes: Vec<String>) -> Self {
        let mut all = self.notes;
        all.extend(notes);
        Self { notes: all, ..self }
    }

    /// Attaches the run's host-only hook trace.
    pub(crate) fn with_hook_trace(self, hook_trace: Vec<String>) -> Self {
        Self { hook_trace, ..self }
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

    /// Attaches the bytes of the binaries the returned `value` holds (`quickjs.rs` `to_json`).
    pub(crate) fn with_binaries(self, binaries: Vec<RunBinary>) -> Self {
        Self { binaries, ..self }
    }

    pub(crate) fn with_pages(self, pages: Vec<RunPage>) -> Self {
        Self { pages, ..self }
    }

    /// Attaches the files the run wrote, and appends the notes for redirected ones to `logs`.
    pub(crate) fn with_artifacts(self, artifacts: Vec<RunArtifact>, notes: Vec<String>) -> Self {
        let mut logs = self.logs;
        logs.extend(notes);
        Self {
            logs,
            artifacts,
            ..self
        }
    }
}

/// One binary of a run's returned value ([`RunOutcome::binaries`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunBinary {
    /// Its place in [`RunOutcome::value`] as an RFC 6901 JSON pointer: `""` when the value
    /// itself is the binary, `/shot` or `/0` for a property or element.
    pub pointer: String,
    /// Its bytes, as standard base64 in the outcome's JSON.
    #[serde(serialize_with = "serialize_base64")]
    pub bytes: Vec<u8>,
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
    /// Who made the call: the agent's script, or one of the run's hooks. Absent from the JSON
    /// for the script's calls, so a run without hooks reads as before.
    #[serde(skip_serializing_if = "CallOrigin::is_agent")]
    pub origin: CallOrigin,
    /// The params exactly as the SDK sent them; `null` for a hook's call, which may type a
    /// secret the model must never read back.
    pub params: Value,
    /// Where the agent's script made the call; `None` for a hook's call.
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
    /// too; empty for a hook's call, whose log may echo what it typed.
    pub log: Vec<String>,
}

/// Which realm sent a browser call. Decided by the realm's bridge the call came through (the
/// hook realm's holds a dispatcher of its own, `Dispatcher::for_hooks`), never by a field of the
/// message: the agent's script can call its bridge directly with any message it likes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CallOrigin {
    Agent,
    Hook,
}

impl CallOrigin {
    /// Whether the agent's script made the call.
    #[must_use]
    pub fn is_agent(&self) -> bool {
        matches!(self, Self::Agent)
    }
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
    /// `TargetClosedError`, `AbortError`, `Error`) and message, without the client's `apiName`
    /// prefix or the call log.
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

/// Bytes as standard base64, the encoding Playwright's protocol gives `binary`.
fn serialize_base64<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&STANDARD.encode(bytes))
}

/// A duration as whole milliseconds, as the rest of the outcome's JSON counts time.
fn serialize_millis<S: Serializer>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_u128(duration.as_millis())
}
