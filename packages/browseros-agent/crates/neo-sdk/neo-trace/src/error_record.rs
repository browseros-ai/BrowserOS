//! Who caused a failure, as a record keeps it. The owner is the party whose behavior must change
//! to fix it. That is the whole point of claw-sdk's first goal: tell Neo bugs from script bugs,
//! host decisions and what the website did.

use crate::OperationId;
use serde::{Deserialize, Serialize};

/// Whose behavior must change to fix a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorOwner {
    /// Neo itself, the browser or the OS: a lost connection, a closed target, a limit.
    Neo,
    /// The agent's own code: a syntax error, `invalid_input`, `strict_mode_violation`,
    /// `no_match`, `method_unavailable`.
    Script,
    /// What the page did: a failed navigation, an element that never became actionable.
    Website,
    /// The host's rules, services or owner, including `owner_has_control`.
    Host,
}

impl ErrorOwner {
    /// The stored and wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Neo => "neo",
            Self::Script => "script",
            Self::Website => "website",
            Self::Host => "host",
        }
    }
}

/// A failure as it is stored: masked and bounded on the writer before it reaches disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorRecord {
    /// Who must change to fix it.
    pub owner: ErrorOwner,
    /// A stable snake_case code, e.g. `owner_has_control`.
    pub code: String,
    /// The human message.
    pub message: String,
    /// The host's own code when `code` is `host_error`, e.g. `needs_owner`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_code: Option<String>,
    /// True only when nothing was sent or the operation was a read.
    pub retryable: bool,
    /// On a run's error, the operation it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<OperationId>,
}
