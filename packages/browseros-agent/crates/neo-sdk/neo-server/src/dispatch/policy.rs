//! The rules every op must pass.

use browseros_core::TabId;
use neo_takeover::LockId;
use neo_trace::OperationId;

use super::{Caller, MethodSpec, OperationError};

/// One rule the door applies to every op: at admission, again when a leaf is about to send
/// (`OpCtx::recheck`), and once more before a read's result or a main-world script's answer is
/// released, as a read of its tab. This keeps a result taken while an owner acquired a
/// reads-blocking lock from reaching the script. A result that carries other tabs' data, such as
/// an event pull's, also goes past the rules once per tab it names (`OpCtx::check_tab`).
///
/// Rules read in-memory state only and never await. That is what lets overlapping calls
/// (`Promise.all`) pass through the door without waiting on each other (skeleton D7 rule 8).
/// The list is fixed at startup: the takeover lock, then the CAPTCHA busy rule.
pub(crate) trait Policy: Send + Sync {
    /// `Err` refuses the op with that error. Delivery is then `not_sent`, unless a child op
    /// already sent something.
    fn check(&self, check: &PolicyCheck<'_>) -> Result<(), OperationError>;
}

/// What a rule sees. A plain data bag.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PolicyCheck<'a> {
    /// The method being called.
    pub(crate) method: &'a MethodSpec,
    /// The script, or Neo's own Rust.
    pub(crate) caller: Caller,
    /// The target's tab, resolved by the door: the same tab the op's trace records, and
    /// already checked against the handle (agreed with sdk-traces: their `Locks::check(tab,
    /// effect)` keys by it). Tab ids survive a Neo restart and core's page ids do not, so no
    /// rule keys by a page id. `None` for a method without a page.
    pub(crate) tab: Option<TabId>,
    /// The op being checked. The CAPTCHA rule uses it to let a solve pass its own recheck.
    pub(crate) operation: OperationId,
    /// The one host lock this op's run may pass ([`RunCtx::pass_lock`](super::RunCtx::pass_lock)):
    /// only the host's private typing sets it, naming the lock the host holds on the tab it
    /// types into. Every other lock refuses as usual.
    pub(crate) lock_pass: Option<LockId>,
}
