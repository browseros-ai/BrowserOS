//! What the dispatcher tells the trace about one SDK operation: once when it is admitted
//! (before policy, so refusals are recorded too), once when it ends, and in between whenever its
//! script reaches a tab besides its target.
//!
//! The start event carries the raw input; it is bounded on the writer task, never on the
//! dispatcher's path. Delivery arrives already derived by the dispatcher's `OperationGuard`.

use crate::{EpochMillis, ErrorRecord, OperationId};
use browseros_core::TabId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

/// Whether an operation's side effect happened, as far as Neo can know. It serializes as its
/// stored name, `as_str`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
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
    /// The page's URL; clipped before storage.
    pub url: Option<String>,
    /// The page's title; clipped before storage.
    pub title: Option<String>,
}

/// The agent script location that submitted an operation, kept with its start record so
/// running, refused and interrupted operations can all be mapped back to the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationSite {
    /// The script line, starting at 1.
    pub line: u32,
    /// The script column, starting at 1.
    pub column: u32,
}

/// An operation was admitted, before policy runs.
#[derive(Clone, Debug)]
pub struct OperationStarted {
    /// The script call site; absent for Neo and prelude work without a script frame.
    pub site: Option<OperationSite>,
    /// Its id.
    pub id: OperationId,
    /// The operation it runs inside: a wrap's method, a defined method, a host call.
    pub parent: Option<OperationId>,
    /// The method name as called, e.g. `locator.click` or `zuzu.auth.fill`, clipped when the
    /// start is stored. Arguments remain in memory until the operation ends.
    pub name: String,
    /// Its place in the tree.
    pub kind: OperationKind,
    /// Read or write.
    pub effect: Effect,
    /// The tab it targets, when it has one. It lists the run under that tab, and after a
    /// restart it is how a task finds its last tab again.
    pub tab: Option<TabId>,
    /// The raw input. The writer bounds it at the operation's end.
    pub args: Value,
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
    /// Its result, when it returned one. The writer bounds it.
    pub result: Option<Value>,
    /// Its error, when it failed.
    pub error: Option<ErrorRecord>,
    /// The page after it, from the browser's cached target info.
    pub page_after: Option<PageState>,
    /// Whether it sent script into a page's main world, the page's own JavaScript world, rather
    /// than Neo's isolated world: the work a site can notice. The host sets it where the op
    /// reaches that world, so an op refused or failed before its script went out stays false.
    pub main_world: bool,
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

#[cfg(test)]
mod tests {
    use super::Delivery;

    #[test]
    fn delivery_serializes_as_its_stored_name() -> Result<(), serde_json::Error> {
        for delivery in [Delivery::NotSent, Delivery::Done, Delivery::Unknown] {
            assert_eq!(serde_json::to_value(delivery)?, delivery.as_str());
        }
        Ok(())
    }
}
