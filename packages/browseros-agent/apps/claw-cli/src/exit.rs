//! The process exit codes, in one place, because scripts depend on them.

/// What a shell caller can branch on.
///
/// `Stopped` exists because the server already distinguishes the two
/// cancellation paths: an MCP `notifications/cancelled` from this client, and a
/// human pressing Stop in the cockpit, which comes back with
/// `cancellationKind: "cockpit.operator-cancelled"`. Only one of those is worth
/// retrying, and a script cannot tell them apart from exit 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    Ok = 0,
    ToolFailed = 1,
    Usage = 2,
    NoBrowser = 3,
    Timeout = 4,
    Stopped = 5,
    Interrupted = 130,
}

impl Code {
    #[must_use]
    pub fn value(self) -> i32 {
        self as i32
    }
}

/// An error that already knows which exit code it deserves.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct Failure {
    pub code: Code,
    pub message: String,
}

impl Failure {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(Code::Usage, message)
    }

    pub fn no_browser(message: impl Into<String>) -> Self {
        Self::new(Code::NoBrowser, message)
    }

    pub fn tool(message: impl Into<String>) -> Self {
        Self::new(Code::ToolFailed, message)
    }
}

/// Maps anything that is not already a [`Failure`] onto a code.
///
/// Anything unclassified is a tool failure rather than a usage error: the
/// command parsed, so the fault is on the far side of the wire.
#[must_use]
pub fn code_of(error: &anyhow::Error) -> Code {
    error
        .downcast_ref::<Failure>()
        .map_or(Code::ToolFailed, |failure| failure.code)
}
