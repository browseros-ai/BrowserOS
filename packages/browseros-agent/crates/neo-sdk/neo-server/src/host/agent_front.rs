//! The agent's tab in front before Neo sends it input or captures it, so Linux draws it.
//!
//! ```text
//!  neo-playwright op ─ input lock / capture ─▶ OpEnv::bring_forward(tab) ─▶ AgentFront
//!    armed (the host API runs: an owner, views)  ─▶ neo_takeover::Front::bring_forward(tab, task)
//!        owner holds the screen, or a view watches another tab or task ─▶ stays behind
//!        else, under the front turn: already active, or Browser.activateTab (a move followers read)
//!    not armed (no host API in this process: no owner screen, no views) ─▶ the run's view:
//!        Browser.getTabInfo ─ not active ─▶ Browser.activateTab
//! ```
//!
//! Only the SDK reaches this: desktop's server never runs neo-playwright, and its rule that an
//! agent never switches the user's tab stands. A tab that stays behind is as slow as before on
//! Linux, never wrong, so nothing here fails or refuses an op.

use std::sync::Arc;

use browseros_core::{BrowserView, TabId};
use neo_takeover::{Forward, Front};
use neo_trace::TaskId;
use serde_json::{Value, json};
use tokio::sync::SetOnce;
use tracing::debug;

/// Brings an agent tab forward for one op (`OpEnv::bring_forward`). Armed with the service's
/// [`Front`] when the host API starts, together with the takeover lock, so with an owner around
/// every move goes through the one front turn and the screen rule. `default()` is unarmed.
#[derive(Default)]
pub(crate) struct AgentFront {
    front: SetOnce<Front>,
}

impl AgentFront {
    /// The host API started: from now on its front decides. Called once per process; a second
    /// call changes nothing.
    pub(crate) fn arm(&self, front: Front) {
        let _ = self.front.set(front);
    }

    /// Makes `tab` its window's active tab if nothing of the owner's says otherwise. Never fails:
    /// what happened is logged.
    // TODO(remove): on Linux a background agent tab is never drawn, and the focus emulation core
    // gives it keeps it rendering frames that viz then throttles to one BeginFrame a second
    // (ThrottleUndrawnFrames): without this, every mouse move waits ~1 s for its ack and a
    // screenshot waits for a frame that may never be presented (SS-054, LH-001). Remove it once
    // BrowserOS's focus emulation clears that throttle for the page (claw-sdk
    // `lead/verify/ss054/BROWSER-PATCH-DESIGN.md`).
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
            None => activate(&view, tab).await,
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
