//! The host's side of one operation.
//!
//! claw-sdk implements [`OpEnv`] over its dispatcher's `OpCtx` (adapter/playwright.rs), and
//! tests over a plain struct. The crate borrows it for one `Playwright::invoke` and never
//! stores it.
//!
//! Deliberately absent: a cancel token and the run's deadline. The dispatcher races every
//! handler with both and drops the op's future (skeleton D7, steps 4 and 5), and everything
//! an op leaves behind in the page is cleaned up by `Drop` guards, which run on that drop too.
//! A second way to stop would be a second cleanup path to get right.

use std::sync::Arc;

use browseros_core::BrowserView;
use browseros_trace::ErrorOwner;

use crate::RunState;

/// What one operation needs from the host that runs it.
pub trait OpEnv: Send + Sync {
    /// The run's admitted browser view. An op admits its page through it once (the profile
    /// and window scope check) and then sends its own calls on that page's session.
    fn view(&self) -> &BrowserView;

    /// This run's Playwright state. The host keeps exactly one per run (claw-sdk: the run's
    /// `RunCtx::slot::<RunState>()`) and drops it with the run.
    fn run_state(&self) -> &Arc<RunState>;

    /// Re-runs the host's attempt-stage policies (the owner's takeover lock, page privacy),
    /// right before the op's first write to the page, because the owner may have taken over
    /// while the op waited.
    fn recheck(&self) -> Result<(), Refusal>;

    /// The op is about to change the page. Write-once on the host side; delivery is derived
    /// from it. The crate calls it only from `OpScope::send_effect`, right after `recheck`.
    fn mark_sent(&self);

    /// The op's target is a password, card or one-time-code field, so the trace redacts the
    /// op's sensitive arguments.
    fn mark_sensitive_target(&self);

    /// One line of Playwright's call log, mirrored into the op's trace record. Lines never
    /// carry a typed value.
    fn note(&self, line: String);
}

/// A host policy refused the op (`owner_has_control`, `page_private`, ...). It reaches the
/// agent unchanged, so the host's owner and code are what the script sees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// Who must act, as the policy decided: `Host` for a takeover, `Script` for a busy page.
    pub owner: ErrorOwner,
    /// The policy's stable code, such as `owner_has_control`.
    pub code: String,
    /// The policy's message.
    pub message: String,
    /// The host's own code, when `code` is `host_error`.
    pub host_code: Option<String>,
}
