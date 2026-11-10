//! The host's side of one operation.
//!
//! claw-sdk implements [`OpEnv`] over its dispatcher's `OpCtx`
//! (`crates/neo-sdk/neo-sdk/src/builtins/playwright_ops.rs`), and tests over a plain struct. The crate borrows it for one `Playwright::invoke` and never
//! stores it.
//!
//! Deliberately absent: a cancel token and the run's deadline. The dispatcher races every
//! handler with both and drops the op's future (skeleton D7, steps 4 and 5), and everything
//! an op leaves behind in the page is cleaned up by `Drop` guards, which run on that drop too.
//! A second way to stop would be a second cleanup path to get right.

use std::sync::Arc;

use browseros_core::{BrowserView, PageId, TabId};
use neo_trace::ErrorOwner;
use serde_json::Value;

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

    /// Re-runs the host's attempt-stage write rules (the owner's takeover lock, a CAPTCHA
    /// solve's hold) on each of `tabs`, right before a script goes out to the op page's main
    /// world. `tabs` are the other tabs of that page's opener-linked group: its opener, the
    /// popups it opened and theirs, followed both ways and through chains, `noopener` popups
    /// included, whatever their site. The page's script reaches each of them through
    /// `window.opener`, the windows `window.open` returned and named windows, so it can click
    /// in, type into and navigate them, while [`OpEnv::recheck`] runs the rules on the op's own
    /// tab only. A host that implements this runs, for each tab, the rules it runs for a write
    /// on that tab, and returns the first refusal unchanged.
    ///
    /// The crate calls it only from the one place it marks an op sent, right after `recheck`
    /// and right before `mark_sent`, synchronously, for every main-world send: each script,
    /// each `waitForFunction` poll, and `setContent`'s write, with the group the privacy sweep
    /// before that send found. It is never called with the op's own tab, and never with an
    /// empty list, so an op on a page with no opener link records `recheck`, `mark_sent` as
    /// before. A refusal reaches the agent unchanged, nothing is marked sent, and the script
    /// does not run.
    ///
    /// The default refuses any tab, as `neo / unsupported`, because a host that has not
    /// implemented this has not said its rules allow a script to act on tabs they never saw.
    /// So every main-world script on a page with an opener link is refused until the host
    /// implements it; one on a page with no link runs. An empty `tabs` reaches no other tab and
    /// passes.
    fn recheck_reach(&self, tabs: &[TabId]) -> Result<(), Refusal> {
        if tabs.is_empty() {
            return Ok(());
        }
        Err(unchecked_reach(tabs.len()))
    }

    /// The op is about to change the page. Write-once on the host side; delivery is derived
    /// from it. The crate calls it only from the one place it marks an op sent, right after
    /// `recheck` and, for a main-world send on a linked page, `recheck_reach`.
    fn mark_sent(&self);

    /// The op's target is a password, card or one-time-code field, so the trace redacts the
    /// op's sensitive arguments.
    fn mark_sensitive_target(&self);

    /// One line of Playwright's call log, mirrored into the op's trace record. Lines never
    /// carry a typed value.
    fn note(&self, line: String);

    /// `page`'s frame tree (the page is on tab `tab`), for the host to hand the run's prelude in
    /// its page facts, so `page.frames()` stays live after any op (FR-2; sdk-skeleton's C6).
    /// `frames` is `{"seq": n, "frames": [{"id", "parentId", "name", "url"}, ...]}`: the main
    /// frame first, every frame's `parentId` naming one listed before it (`null` for the main
    /// frame), and `seq` growing with each report of the run, so the prelude applies only a
    /// newer tree. Frame names and URLs are the page's own text. The default ignores it, and
    /// the crate does not report yet.
    fn report_frames(&self, _page: &PageId, _tab: TabId, _frames: Value) {}
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

/// The refusal of a main-world send that reaches `tabs` other tabs no host rule checked:
/// [`OpEnv::recheck_reach`]'s default, and the crate's own when a tab of the group has no tab id
/// to name it to the host by (`GuardCore::reach`), so a script on a linked page meets the one
/// refusal however its host fell short.
pub(crate) fn unchecked_reach(tabs: usize) -> Refusal {
    Refusal {
        owner: ErrorOwner::Neo,
        code: "unsupported".to_owned(),
        message: format!(
            "Neo refused this call: the page is linked to {tabs} other tab(s) through \
             window.opener (its opener, or popups it opened), which a script in the page can act \
             on, and Neo's host does not check its rules on those tabs yet, so nothing was sent \
             to the page. Run it in a tab with no opener and no popups."
        ),
        host_code: None,
    }
}
