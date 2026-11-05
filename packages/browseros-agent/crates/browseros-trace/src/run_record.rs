//! What the trace hears about sessions and runs. Lifecycle events are never dropped, even when
//! the writer falls behind: a run's end is what turns its open operations into a verdict.

use crate::{EpochMillis, ErrorRecord, RunId, SessionId};
use serde_json::Value;

/// How a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    /// Still executing.
    Running,
    /// The script returned.
    Ok,
    /// The script threw.
    Error,
    /// The run was cancelled (MCP cancellation, the session stopped).
    Cancelled,
    /// The run hit its wall-clock budget.
    Timeout,
    /// Neo stopped before the run ended: set at the next open, never sent by the dispatcher.
    Interrupted,
}

impl RunStatus {
    /// The stored and wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
            Self::Interrupted => "interrupted",
        }
    }
}

/// Whether a run changed anything, rolled up from its write operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunEffects {
    /// No write was sent.
    None,
    /// Every write that was sent got a definite answer.
    Done,
    /// At least one write was sent and never confirmed. Check the page before repeating.
    Unknown,
}

impl RunEffects {
    /// The stored and wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Done => "done",
            Self::Unknown => "unknown",
        }
    }
}

/// A session started.
#[derive(Clone, Debug)]
pub struct SessionStarted {
    /// Its id.
    pub id: SessionId,
    /// The MCP client's `clientInfo.name`.
    pub client: Option<String>,
    /// When it started.
    pub at: EpochMillis,
}

/// A session ended.
#[derive(Clone, Debug)]
pub struct SessionEnded {
    /// Its id.
    pub id: SessionId,
    /// When it ended.
    pub at: EpochMillis,
}

/// A run was admitted.
#[derive(Clone, Debug)]
pub struct RunStarted {
    /// Its id.
    pub id: RunId,
    /// Its session.
    pub session: SessionId,
    /// The script, as given. The writer masks and bounds it.
    pub code: String,
    /// The host's reference for the call that started it (zuzu's tool-call id).
    pub host_ref: Option<String>,
    /// The host's own data for the run (`_meta["com.browseros.neo/run"].host`).
    pub host: Option<Value>,
    /// The key that makes a repeated `run` return this run's result instead of running again.
    pub idempotency_key: Option<String>,
    /// When it was admitted.
    pub at: EpochMillis,
}

/// A run ended, exactly once.
#[derive(Clone, Debug)]
pub struct RunEnded {
    /// Its id.
    pub id: RunId,
    /// How it ended. Never `Running` or `Interrupted`.
    pub status: RunStatus,
    /// The uncaught error, when the script threw.
    pub error: Option<ErrorRecord>,
    /// The script's return value. The writer masks and bounds it.
    pub result: Option<Value>,
    /// The exact MCP result handed to the client, already masked, kept for idempotent replay.
    pub replay: Option<Value>,
    /// When it ended.
    pub at: EpochMillis,
}
