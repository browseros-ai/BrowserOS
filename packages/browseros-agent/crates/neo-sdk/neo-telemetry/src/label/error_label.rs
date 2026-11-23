//! One failure's labels, owner and code together.

use super::{ErrorCode, ErrorOwner};

/// One failure's labels: who must change to fix it and its code. Together, so a run or op
/// verdict cannot carry an owner without a code or a code without an owner; no failure is
/// `None` where an `Option<ErrorLabel>` is taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ErrorLabel {
    /// Who must change to fix it.
    pub owner: ErrorOwner,
    /// The code Neo serves for it on `/v1/runs`.
    pub code: ErrorCode,
}

impl ErrorLabel {
    /// The `neo.error.owner` value for an optional failure: its owner, or `none`.
    #[must_use]
    pub const fn owner_label(error: Option<Self>) -> &'static str {
        match error {
            Some(error) => error.owner.as_str(),
            None => ErrorOwner::NONE,
        }
    }
}
