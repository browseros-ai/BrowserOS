//! What the dispatcher tells the trace about one SDK operation: once when it is admitted
//! (before policy, so refusals are recorded too), once when it ends, and in between whenever its
//! script reaches a tab besides its target.
//!
//! The start event carries the raw input; it is masked and bounded on the writer task, never on the
//! dispatcher's path. Delivery arrives already derived by the dispatcher's `OperationGuard`.

use crate::{EpochMillis, ErrorRecord, OperationId, SecretSet};
use browseros_core::TabId;
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

/// The tab an operation acted on, as it was after the operation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageState {
    /// The browser's tab id.
    pub tab_id: Option<TabId>,
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
    /// The method name as called, e.g. `locator.click` or `zuzu.auth.fill`. A script can call
    /// any name, so the writer masks it, with `also_mask` too, as it stores the start.
    pub name: String,
    /// Its place in the tree.
    pub kind: OperationKind,
    /// Read or write.
    pub effect: Effect,
    /// The tab it targets, when it has one. It lists the run under that tab, and after a
    /// restart it is how a task finds its last tab again.
    pub tab: Option<TabId>,
    /// The raw input. The writer masks and bounds it at the operation's end.
    pub args: Value,
    /// Values the process set does not mask yet that this record must not keep: what claw-sdk
    /// still holds unregistered (sdk-traces decision 32). The writer masks the name, the stored
    /// input and the operation's end record with it, together with the process set. It
    /// registers nothing from it. `None` masks with the process set alone.
    pub also_mask: Option<Arc<SecretSet>>,
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
    /// Values this record must not keep beyond the process set's, as on [`OperationStarted`].
    /// The writer masks the input, notes, result, error message and page with it.
    pub also_mask: Option<Arc<SecretSet>>,
    /// When it ended.
    pub at: EpochMillis,
}

/// Tabs an operation's script can act on besides its target, as a main-world script in a popup
/// does on its opener through `window.opener`. Tab ids only, never a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationReached {
    /// The operation, whose start was recorded before.
    pub id: OperationId,
    /// The tabs it reached for the first time, never its own target tab.
    pub tabs: Vec<TabId>,
    /// When the door saw the reach.
    pub at: EpochMillis,
}
