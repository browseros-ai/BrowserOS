//! Neo: A script, its starting tab, and the outcome with every created or handed-out page.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Serialize, Serializer};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use ulid::{Generator, Ulid};

use crate::artifacts::RunArtifact;
use crate::deadline::Stop;
use crate::trace::RunOrigin;

/// The last millisecond a ULID's 48-bit clock holds, in the year 10889.
/// Platform: ULID timestamps have 48 bits (`ulid::Ulid::TIME_BITS`).
const MAX_ULID_MS: u64 = (1 << Ulid::TIME_BITS) - 1;

/// The process's one source of run ids. Monotonic: an id minted in the same millisecond as the
/// one before it, or after the clock stepped back, is that id plus one, so mint order is sort
/// order across every `Runtime` in the process.
/// Neo: one process-wide generator keeps run mint order equal to trace listing order.
static RUN_IDS: Mutex<Generator> = Mutex::new(Generator::new());

/// The CDP target id of a tab. In the script's realm the page is `page@<id>` and its main
/// frame is `frame@<id>`, because Chromium gives a page's main frame its target's id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
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

/// A run's id: a ULID minted first thing in `Runtime::run`, before anything can fail, so every
/// outcome and every event of the run carries it, early failures included. Ids come from one
/// process-wide monotonic generator, so they sort by when their runs started, within one
/// millisecond too, and so does their text: a store can use the id as its run order, its cursor
/// and its retention cut ([`RunId::floor`]). Its text, also when serialized, is the ULID's 26
/// characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunId(Ulid);

impl RunId {
    /// A fresh id. Should the generator fail (2^80 ids in one millisecond, or a panic while it
    /// was locked), the id is still unique, only no longer ordered within its millisecond.
    #[expect(
        clippy::unwrap_or_default,
        reason = "Ulid's default is the nil id; `Ulid::new` mints a fresh one"
    )]
    pub(crate) fn mint() -> Self {
        let next = RUN_IDS.lock().ok().and_then(|mut ids| ids.generate().ok());
        Self(next.unwrap_or_else(Ulid::new))
    }

    /// The smallest id of the millisecond `unix_ms`: runs minted at or after it sort at or
    /// after this id, earlier ones before it, so "started before T" is `id < RunId::floor(T)`.
    /// A millisecond past the ULID clock's end (year 10889) reads as that end.
    #[must_use]
    pub fn floor(unix_ms: u64) -> Self {
        Self(Ulid::from_parts(unix_ms.min(MAX_ULID_MS), 0))
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl FromStr for RunId {
    type Err = InvalidRunId;

    /// Reads an id's 26 characters. A first character past `7` is refused: 26 characters hold
    /// 130 bits, and decoding would silently drop the top two, naming some other run.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match Ulid::from_string(text) {
            Ok(id) if text.starts_with(|first: char| first <= '7') => Ok(Self(id)),
            _ => Err(InvalidRunId),
        }
    }
}

impl Serialize for RunId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Text that is not a [`RunId`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a run id is 26 Crockford base32 characters, the first one 0 to 7")]
pub struct InvalidRunId;

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
    /// Host labels for trace sinks; default when the host supplies no labels.
    pub origin: RunOrigin,
    /// The directory the run's `path` options write into (`page.screenshot({ path })`,
    /// `page.pdf({ path })`), created on first write. A `path` must be relative to it and stay
    /// inside it; a runtime opted into `AbsolutePaths::Redirect` also takes absolute paths
    /// (`Runtime::with_absolute_paths`). `None` refuses every `path` with an Error naming the
    /// option; the bytes a script asked for are still returned. Files written are listed in
    /// [`RunOutcome::artifacts`].
    pub artifacts: Option<PathBuf>,
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
    /// Minted before anything can fail, including before the starting tab opens.
    pub run_id: RunId,
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
}

impl RunOutcome {
    pub(crate) fn returned(run_id: RunId, value: Option<Value>, logs: Vec<String>) -> Self {
        Self {
            run_id,
            status: RunStatus::Ok,
            value,
            logs,
            error: None,
            calls: Vec::new(),
            pages: Vec::new(),
            calls_dropped: None,
            binaries: Vec::new(),
            artifacts: Vec::new(),
        }
    }

    pub(crate) fn failed(run_id: RunId, error: RunError, logs: Vec<String>) -> Self {
        Self {
            run_id,
            status: RunStatus::Error,
            value: None,
            logs,
            error: Some(error),
            calls: Vec::new(),
            pages: Vec::new(),
            calls_dropped: None,
            binaries: Vec::new(),
            artifacts: Vec::new(),
        }
    }

    pub(crate) fn stopped(run_id: RunId, stop: Stop, timeout: Duration, logs: Vec<String>) -> Self {
        let (status, error) = match stop {
            Stop::Timeout => (RunStatus::Timeout, Some(RunError::timeout(timeout))),
            Stop::Cancelled => (RunStatus::Cancelled, None),
        };
        Self {
            run_id,
            status,
            value: None,
            logs,
            error,
            calls: Vec::new(),
            pages: Vec::new(),
            calls_dropped: None,
            binaries: Vec::new(),
            artifacts: Vec::new(),
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

    /// Attaches console lines read once by the run guard, shared with the trace's copy.
    pub(crate) fn with_logs(self, logs: Vec<String>) -> Self {
        Self { logs, ..self }
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
    /// The params exactly as the SDK sent them.
    pub params: Value,
    /// Where the agent's script made the call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<CallLocation>,
    /// The SDK metadata timeout, in milliseconds; zero means no call timeout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<f64>,
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

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use serde_json::json;

    use super::*;
    use crate::trace::unix_ms;

    #[test]
    fn run_ids_minted_back_to_back_increase_and_read_back_from_their_text() {
        let ids: Vec<RunId> = (0..10_000).map(|_| RunId::mint()).collect();
        for pair in ids.windows(2) {
            let (first, next) = (pair[0], pair[1]);
            assert!(first < next, "minted in order: {first} then {next}");
            assert!(
                first.to_string() < next.to_string(),
                "their text sorts as they do: {first} then {next}"
            );
        }
        for id in &ids {
            let text = id.to_string();
            assert_eq!(text.len(), 26, "a ULID's 26 characters: {text}");
            assert_eq!(text.parse(), Ok(*id), "{text} reads back");
            assert_eq!(
                serde_json::to_value(id).unwrap(),
                json!(text),
                "serialized as its text"
            );
        }
        for bad in [
            "",
            "01JC8Z3K",
            "80000000000000000000000000",
            "0000000000000000000000000U",
        ] {
            assert_eq!(
                bad.parse::<RunId>(),
                Err(InvalidRunId),
                "{bad:?} names no run"
            );
        }
    }

    #[test]
    fn floor_splits_runs_at_its_millisecond() {
        let before = unix_ms(SystemTime::now());
        let id = RunId::mint();
        let after = unix_ms(SystemTime::now());
        assert!(
            RunId::floor(before) <= id,
            "{id} was minted at or after {before}"
        );
        assert!(
            id < RunId::floor(after + 1),
            "{id} was minted before {}",
            after + 1
        );
        assert!(
            id < RunId::floor(1 << 48),
            "a millisecond past the ULID clock's end sorts after every run"
        );
    }
}
