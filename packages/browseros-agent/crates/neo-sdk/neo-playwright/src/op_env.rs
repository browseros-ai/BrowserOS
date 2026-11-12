//! The host's side of one operation.
//!
//! claw-sdk implements [`OpEnv`] over its dispatcher's `OpCtx`
//! (`crates/neo-sdk/neo-server/src/builtins/playwright_ops.rs`), and tests over a plain struct. The crate borrows it for one `Playwright::invoke` and never
//! stores it.
//!
//! Deliberately absent: a cancel token and the run's deadline. The dispatcher races every
//! handler with both and drops the op's future (skeleton D7, steps 4 and 5), and everything
//! an op leaves behind in the page is cleaned up by `Drop` guards, which run on that drop too.
//! A second way to stop would be a second cleanup path to get right.
//!
//! An abort is not a stop. Playwright's `signal` option fails one op as its own timeout does,
//! with the op's call log and an `operation was aborted: <reason>` line (upstream's
//! `Progress.abort`), so it reaches the op through [`OpEnv::aborted`], which `OpScope`'s waits
//! race exactly as they race the op's deadline: the op returns its error through the timeout
//! path it already has, and nothing is dropped from outside.

use std::future;
use std::sync::Arc;

use browseros_core::{BrowserView, TabId, TargetId};
use futures_util::future::BoxFuture;
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

    /// Whether this browser runs in hosted mode. Hosted BrowserOS suppresses console and
    /// page-error events by design (`events/event_op.rs`, `lease_runtime`), so listeners for
    /// them stay silent and a one-shot wait is refused. Hosts supply their startup mode; the
    /// library never reads process environment. Existing hosts default to non-hosted.
    fn hosted_mode(&self) -> bool {
        false
    }

    /// Re-runs the host's attempt-stage policies (the owner's takeover lock on the op's tab),
    /// right before the op's first write to the page, because the owner may have taken over
    /// while the op waited.
    fn recheck(&self) -> Result<(), Refusal>;

    /// The op is about to change the page. Write-once on the host side; delivery is derived
    /// from it. The crate calls it only from the one place it marks an op sent, right after
    /// `recheck`.
    fn mark_sent(&self);

    /// The op is about to send script into a page's main world, the page's own JavaScript world,
    /// where a site can notice it, rather than Neo's isolated world. The host records it as a
    /// write-once fact of the op, as it records `mark_sent`. The crate calls it right after the
    /// host's recheck passed for such a send, so an op refused or failed before its script went
    /// out never reports it: at a `Capability::MainWorld` gateway's `send_effect`, and when a
    /// write op's admission adds the run's context scripts or bindings to its tab, which that op
    /// does without declaring `MainWorld`. The default ignores it.
    fn main_world(&self) {}

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
    fn report_frames(&self, _tab: TabId, _frames: Value) {}

    /// A delivered close and the page target the watcher saw destroyed. The host compares
    /// that target with its current tab binding even after the event leaves the bounded log.
    /// The record sequence lets the host approve its precise callback position in a batch.
    /// Hosts without cached page facts may leave this callback empty.
    fn report_close(&self, _tab: TabId, _target: &TargetId, _seq: u64) {}

    /// The target this operation admitted for its own page. The host compares this with
    /// the later cache observation before allowing a result payload to refresh page state.
    fn report_target(&self, _tab: TabId, _target: &TargetId) {}

    /// A page event may observe a navigation performed by the page rather than an SDK goto.
    /// Refresh the host's cached URL before delivering that event's reply. Hosts without
    /// cached page facts can ignore this observation.
    fn report_navigation(&self, _tab: TabId) {}

    /// The reason the script gave when it aborted this op through Playwright's `signal` option,
    /// as upstream prints it (an `Error` reason's message, else the reason as a string), once it
    /// has. `None` while the op runs unaborted, and always for a host that offers no abort.
    fn abort_reason(&self) -> Option<String> {
        None
    }

    /// Resolves once the script aborts this op, when [`OpEnv::abort_reason`] reads `Some`.
    /// Cheap to call again: each wait of the op races a fresh one. The default never resolves.
    fn aborted(&self) -> BoxFuture<'_, ()> {
        Box::pin(future::pending())
    }
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
