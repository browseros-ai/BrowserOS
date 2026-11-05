//! What the dispatcher tells the trace about one SDK operation: once when it is admitted
//! (before policy, so refusals are recorded too) and once when it ends.
//!
//! The start event carries the raw input; it is redacted on the writer task, never on the
//! dispatcher's path. Delivery arrives already derived by the dispatcher's `OperationGuard`.

use crate::{EpochMillis, ErrorRecord, JsonPointer, OperationId, SessionId};
use serde_json::Value;
use std::sync::Arc;

/// Where an operation sits in the run's tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    /// A Playwright leaf or a `neo.*` method.
    Builtin,
    /// A method the host's extension defined (`neo.define`).
    Defined,
    /// One call of `next()` inside a wrap; `attempt` counts from 1.
    Next {
        /// Which call of `next()` this was.
        attempt: u32,
    },
    /// One `ctx.host.call` to the host.
    HostCall,
    /// The host's protected fill, arriving while its host call waits.
    Protected,
}

impl OperationKind {
    /// The stored and wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Defined => "defined",
            Self::Next { .. } => "next",
            Self::HostCall => "host_call",
            Self::Protected => "protected",
        }
    }

    /// The `next()` attempt, for `Next`.
    #[must_use]
    pub const fn attempt(self) -> Option<u32> {
        match self {
            Self::Next { attempt } => Some(attempt),
            _ => None,
        }
    }
}

/// Whether the operation can change the page or the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Reads only; never marks itself sent.
    Read,
    /// May change something. Host-defined methods default to this.
    Write,
}

impl Effect {
    /// The stored and wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

/// How an operation ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It returned a value.
    Ok,
    /// It failed with an error record.
    Error,
    /// It was cut short: run cancelled, timed out, or ended while it ran.
    Cancelled,
}

impl Outcome {
    /// The stored and wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Whether an operation's side effect happened, as far as Neo can know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// No side effect was attempted: safe to retry.
    NotSent,
    /// Neo has a definite answer, success or failure.
    Done,
    /// Sent and never confirmed. Never repeat it blindly.
    Unknown,
}

impl Delivery {
    /// The stored and wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotSent => "not_sent",
            Self::Done => "done",
            Self::Unknown => "unknown",
        }
    }
}

/// Whether the field an operation typed into was a password, card or one-time-code field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TargetSensitivity {
    /// An ordinary target, or none.
    #[default]
    Ordinary,
    /// A sensitive field: the method's `sensitive` input paths are redacted, and the run's
    /// stored code is withheld.
    Sensitive,
}

/// Which input fields a record redacts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RedactionPolicy {
    /// Always stored as `[redacted]`.
    pub secret: Vec<JsonPointer>,
    /// Stored as `[redacted]` when the target turned out sensitive (e.g. `fill`'s `/value`).
    pub sensitive: Vec<JsonPointer>,
}

/// The tab an operation acted on, as it was after the operation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageState {
    /// The session's page id.
    pub page_id: Option<u32>,
    /// The browser's tab id.
    pub tab_id: Option<i64>,
    /// The page's URL; masked and clipped before storage.
    pub url: Option<String>,
    /// The page's title; masked and clipped before storage.
    pub title: Option<String>,
}

/// An operation was admitted, before policy runs.
#[derive(Clone, Debug)]
pub struct OperationStarted {
    /// Its id.
    pub id: OperationId,
    /// The operation it runs inside: a wrap's method, a defined method, a host call.
    pub parent: Option<OperationId>,
    /// The session it belongs to.
    pub session: SessionId,
    /// The method name, e.g. `locator.click` or `zuzu.auth.fill`.
    pub name: String,
    /// Its place in the tree.
    pub kind: OperationKind,
    /// Read or write.
    pub effect: Effect,
    /// The page it targets, when it has one.
    pub page: Option<u32>,
    /// The raw input. The writer redacts it; it is never stored as given.
    pub args: Value,
    /// Which input fields to redact.
    pub policy: Arc<RedactionPolicy>,
    /// When it was admitted.
    pub at: EpochMillis,
}

/// An operation ended, exactly once.
#[derive(Clone, Debug)]
pub struct OperationEnded {
    /// Its id.
    pub id: OperationId,
    /// How it ended.
    pub outcome: Outcome,
    /// What the dispatcher derived from its `sent` flag and the answer.
    pub delivery: Delivery,
    /// When it marked itself sent, if it did.
    pub sent_at: Option<EpochMillis>,
    /// Its result, when it returned one. The writer masks and bounds it.
    pub result: Option<Value>,
    /// Its error, when it failed.
    pub error: Option<ErrorRecord>,
    /// The page after it, from the browser's cached target info.
    pub page_after: Option<PageState>,
    /// Whether the field it typed into was sensitive.
    pub target: TargetSensitivity,
    /// When it ended.
    pub at: EpochMillis,
}
