use anyhow::Result;
use serde::Serialize;

use crate::engine::conflict::{self, AbortOutcome};
use crate::engine::lock::CheckoutLock;
use crate::engine::state::StateContext;

/// Serializable abort result for a conflict session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum AbortReport {
    /// A conflict session was removed.
    Aborted {
        /// Process exit code for this result.
        exit: i32,
    },
    /// There was no session to abort.
    NoSession {
        /// Process exit code for this result.
        exit: i32,
    },
    /// Abort could not acquire the lock or remove session state.
    Error {
        /// Human-readable failure reason.
        reason: String,
        /// Process exit code for this result.
        exit: i32,
    },
}

impl AbortReport {
    /// Returns the process exit code represented by the report.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Aborted { exit } | Self::NoSession { exit } | Self::Error { exit, .. } => *exit,
        }
    }
}

/// Runs abort with the checkout lock held.
pub fn run(ctx: &StateContext) -> AbortReport {
    let _lock = match CheckoutLock::acquire(&ctx.checkout) {
        Ok(lock) => lock,
        Err(err) => {
            return AbortReport::Error {
                reason: err.to_string(),
                exit: 1,
            };
        }
    };

    match conflict::abort(&ctx.checkout) {
        Ok(AbortOutcome::Aborted) => AbortReport::Aborted { exit: 0 },
        Ok(AbortOutcome::NoSession) => AbortReport::NoSession { exit: 1 },
        Err(err) => AbortReport::Error {
            reason: err.to_string(),
            exit: 1,
        },
    }
}

/// Renders a human abort report.
pub fn render_human(report: &AbortReport) -> String {
    match report {
        AbortReport::Aborted { .. } => "conflict session aborted.\n".to_string(),
        AbortReport::NoSession { .. } => "no conflict session to abort.\n".to_string(),
        AbortReport::Error { reason, .. } => format!("error: {reason}\n"),
    }
}

/// Renders a JSON abort report.
pub fn render_json(report: &AbortReport) -> Result<String> {
    Ok(serde_json::to_string(report)?)
}
