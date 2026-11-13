//! The agent's tab in front before Neo sends it input or captures it, so Linux draws it, and so
//! the tab has the page focus its cross-site frames read (`focus()` takes the input hold too).
//!
//! ```text
//!  neo-playwright op ─ input lock / capture ─▶ OpEnv::bring_forward(tab) ─▶ AgentFront
//!    armed (the host API runs: an owner, views)  ─▶ neo_takeover::Front::bring_forward(tab, task)
//!        owner holds the screen, or a view watches another tab or task ─▶ stays behind
//!        else, under the front turn: already active, or Browser.activateTab (a move followers read)
//!    the app before its host API serves ─▶ stays behind (an owner may arrive at once)
//!    without_host (no host API in this process: no owner screen, no views) ─▶ the run's view:
//!        Browser.getTabInfo ─ not active ─▶ Browser.activateTab
//! ```
//!
//! Only the SDK reaches this: desktop's server never runs neo-playwright, and its rule that an
//! agent never switches the user's tab stands. Nothing here fails or refuses an op: a tab that
//! stays behind is as slow as before on Linux, and its cross-site frames stay unfocused after
//! `focus()` (`toBeFocused()` fails there), which is what real Playwright sees on such a tab too.

use std::sync::Arc;

use browseros_core::{BrowserView, TabId};
use neo_takeover::{Forward, Front};
use neo_trace::TaskId;
use serde_json::{Value, json};
use tokio::sync::SetOnce;
use tracing::debug;

/// Brings an agent tab forward for one op (`OpEnv::bring_forward`). In the app it is armed with
/// the service's [`Front`] when the host API starts, together with the takeover lock, so with an
/// owner around every move goes through the one front turn and the screen rule; until then it
/// moves nothing, since an owner may arrive the moment the API serves. A process with no host
/// API at all ([`AgentFront::without_host`]) activates directly.
#[derive(Default)]
pub(crate) struct AgentFront {
    front: SetOnce<Front>,
    /// No host API will ever arm this process: there is no owner's screen or view to respect.
    without_host: bool,
}

impl AgentFront {
    /// For a process that serves no host API (the hosted sidecar, the spec harness).
    #[cfg(feature = "hosted-run")]
    pub(crate) fn without_host() -> Self {
        Self {
            front: SetOnce::new(),
            without_host: true,
        }
    }

    /// The host API started: from now on its front decides. Called once per process; a second
    /// call changes nothing.
    pub(crate) fn arm(&self, front: Front) {
        let _ = self.front.set(front);
    }

    /// Makes `tab` its window's active tab if nothing of the owner's says otherwise. Never fails:
    /// what happened is logged.
    // TODO(remove): without this a background agent tab breaks three things, two of them on
    // Linux only, where no display draws such a tab (SS-054, LH-001):
    // - Input and frames (Linux): the focus emulation core gives the tab keeps it rendering
    //   frames that viz throttles to one BeginFrame a second (ThrottleUndrawnFrames), so every
    //   mouse move waits ~1 s for its ack.
    // - Captures (Linux): a screenshot waits for the presentation of a forced frame, ~1.2 s
    //   headed and possibly forever headless. This is not the throttle (1.2 s with zero
    //   throttled BeginFrames); its cause is not yet found. The first lead is Chromium's
    //   CDPScreenshotNewSurface feature, not yet measured.
    // - Focus in cross-site frames (every platform): each renderer process keeps its own focus
    //   state, and core's focus emulation is on the page session only (as upstream Playwright
    //   sets it), so it reaches only the main frame's process. A cross-site frame's process reads
    //   the tab's real page focus, which a background tab lacks until it is activated or input
    //   reaches it: there, after `focus()`, `document.hasFocus()` stays false and Playwright's
    //   `toBeFocused()` fails. Measured on a background tab with no activation:
    //   `Emulation.setFocusEmulationEnabled` sent on the frame's own session makes it true
    //   (claw-sdk `lead/verify/oopif-focus/NOTES.md`).
    // Remove it only when ALL of these hold for a background agent tab:
    // 1. Input and rAF are fast on Linux: the focus-emulation browser patch
    //    (claw-sdk `lead/verify/ss054/BROWSER-PATCH-DESIGN.md`), which lifts the throttle only.
    // 2. Captures are fast on Linux: `neo-playwright/tests/background_tab_live.rs` passes there.
    // 3. Cross-site frames take focus: Neo sends `Emulation.setFocusEmulationEnabled` on every
    //    out-of-process frame session of an agent tab (at attach, after a frame's cross-process
    //    navigation, nested frames too), and `tests/cross_site_focus_browseros.rs` passes with
    //    this activation gone. The browser patch needs the same per-frame command to unthrottle
    //    those frames, so one change can serve 1 and 3.
    pub(crate) async fn bring_forward(
        self: Arc<Self>,
        view: BrowserView,
        tab: TabId,
        task: Option<TaskId>,
    ) {
        let outcome = match self.front.get() {
            Some(front) => front
                .bring_forward(tab, task)
                .await
                .map(|forward| describe(&forward))
                .map_err(|error| error.to_string()),
            None if self.without_host => activate(&view, tab).await,
            None => Ok("stays behind: the host API is not serving yet"),
        };
        match outcome {
            Ok(what) => debug!(tab = tab.0, what, "agent tab forward"),
            Err(error) => debug!(tab = tab.0, error, "agent tab not brought forward"),
        }
    }
}

/// With no host API there is no owner's screen or view to respect: activate the tab unless it
/// already is its window's active tab.
async fn activate(view: &BrowserView, tab: TabId) -> Result<&'static str, String> {
    let info = view
        .cdp("Browser.getTabInfo", json!({ "tabId": tab.0 }), None)
        .await
        .map_err(|error| error.to_string())?;
    let active = info
        .get("tab")
        .and_then(|tab| tab.get("isActive"))
        .and_then(Value::as_bool);
    if active == Some(true) {
        return Ok("already in front");
    }
    view.cdp("Browser.activateTab", json!({ "tabId": tab.0 }), None)
        .await
        .map_err(|error| error.to_string())?;
    Ok("brought")
}

fn describe(forward: &Forward) -> &'static str {
    match forward {
        Forward::AlreadyInFront => "already in front",
        Forward::Brought => "brought",
        Forward::OwnerHasScreen(_) => "stays behind: the owner holds the screen",
        Forward::ViewerElsewhere => "stays behind: a viewer watches another tab",
    }
}
