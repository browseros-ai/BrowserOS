//! `elementHandle.waitForElementState`: upstream's `_retryAction` around `checkElementStates`
//! (server/dom.ts at v1.64.1), on one element handle's node.

use std::time::Duration;

use crate::backoff::Backoff;
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{ProbeReply, ProbeRequest, ProbeTarget, QueryState, Read, Rect};
use crate::read::element_read;
use crate::resolve::{self, HandleSite};
use crate::target::{ElementTarget, Strictness};
use crate::wire::Nothing;
use crate::{ErrorCode, Playwright, PlaywrightError};

/// The gap between the two measurements of the `stable` check, on Rust's clock: about one
/// frame, as upstream's two animation frames at `stableRafCount: 1`, without the page's rAF,
/// which stalls in a background tab.
const STABLE_GAP: Duration = Duration::from_millis(16);

/// The states a handle waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ElementWait {
    Visible,
    Hidden,
    Stable,
    Enabled,
    Disabled,
    Editable,
}

impl ElementWait {
    /// Upstream's name for the state, in the call log.
    fn name(self) -> &'static str {
        match self {
            Self::Visible => "visible",
            Self::Hidden => "hidden",
            Self::Stable => "stable",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Editable => "editable",
        }
    }

    /// The state the probe checks; `stable` is two measurements in time instead.
    fn query(self) -> Option<QueryState> {
        match self {
            Self::Visible => Some(QueryState::Visible),
            Self::Hidden => Some(QueryState::Hidden),
            Self::Enabled => Some(QueryState::Enabled),
            Self::Disabled => Some(QueryState::Disabled),
            Self::Editable => Some(QueryState::Editable),
            Self::Stable => None,
        }
    }

    /// The error a wait that ran out of time ends with: the element stayed hidden, disabled,
    /// unstable or not editable (`not_actionable`), or it never hid or got disabled
    /// (`wait_timeout`).
    fn timed_out(self) -> ErrorCode {
        match self {
            Self::Visible | Self::Stable | Self::Enabled | Self::Editable => {
                ErrorCode::NotActionable
            }
            Self::Hidden | Self::Disabled => ErrorCode::WaitTimeout,
        }
    }
}

/// Waits until the handle's node is in `state`, as upstream's `waitForElementState`: an
/// attempt at once, then retries on the inner backoff, logging `attempting wait for <state>
/// action`, `retrying wait for <state> action`, `waiting <n>ms` and `element is not <state>`.
/// A node that is gone meets `hidden` and fails every other state at once
/// ("Element is not attached to the DOM"), as upstream's `elementState` answers for it.
///
/// The timeout covers every await of the op: finding the node again (`DOM.resolveNode`) waits
/// on the renderer like any probe call, and a busy one can hold it for over a second.
pub(crate) async fn wait_for_element_state(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &ElementTarget,
    state: ElementWait,
) -> Result<Nothing, PlaywrightError> {
    // Boxed: every op's future is one variant of `Playwright::invoke`'s, which a host awaits.
    match scope
        .within(Box::pin(poll_element(pw, scope, target, state)))
        .await
    {
        Ok(waited) => waited,
        Err(_expired) => Err(scope.timed_out(state.timed_out())),
    }
}

/// [`wait_for_element_state`]'s attempts, which its deadline drops wherever they are.
async fn poll_element(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &ElementTarget,
    state: ElementWait,
) -> Result<Nothing, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, &target.page).await?;
    let gone = || {
        if state == ElementWait::Hidden {
            Ok(())
        } else {
            Err(resolve::handle_detached(scope))
        }
    };
    let Some(site) = HandleSite::find(&page, scope, target).await? else {
        return gone();
    };
    let backoff = Backoff::inner();
    for attempt in 0.. {
        let wait = backoff.wait(attempt);
        if attempt == 0 {
            scope.log(format!("attempting wait for {} action", state.name()));
        } else {
            scope.log(format!("retrying wait for {} action", state.name()));
            if !wait.is_zero() {
                scope.log(format!("  waiting {}ms", wait.as_millis()));
            }
        }
        if scope.sleep(wait).await.is_err() {
            return Err(scope.timed_out(state.timed_out()));
        }
        match meets(scope, &site, state).await? {
            Some(true) => return Ok(()),
            Some(false) => scope.log(format!("  element is not {}", state.name())),
            None => return gone(),
        }
    }
    Err(scope.timed_out(state.timed_out()))
}

/// Whether the node is in `state` now; `None` when it is gone.
async fn meets(
    scope: &OpScope<'_>,
    site: &HandleSite<'_>,
    state: ElementWait,
) -> Result<Option<bool>, PlaywrightError> {
    let Some(query) = state.query() else {
        let Some(before) = border_box(scope, site).await? else {
            return Ok(None);
        };
        // The sleep is cut short by the op's timeout, which `wait_for_element_state` reports.
        let _expired = scope.sleep(STABLE_GAP).await;
        let Some(after) = border_box(scope, site).await? else {
            return Ok(None);
        };
        return Ok(Some(before == after));
    };
    let reply = site.call(&root_read(Read::State { state: query })).await?;
    match reply {
        ProbeReply::Read { value, .. } => {
            element_read::read_value(scope, value.into_value()).map(Some)
        }
        ProbeReply::Detached => Ok(None),
        other => Err(resolve::refused(scope, other)),
    }
}

/// The node's border box in its own frame (`None` when the node is gone; a box of `None` when
/// it has no layout box, which is as stable as any).
async fn border_box(
    scope: &OpScope<'_>,
    site: &HandleSite<'_>,
) -> Result<Option<Option<Rect>>, PlaywrightError> {
    match site.call(&root_read(Read::BoundingBox)).await? {
        ProbeReply::Read { value, .. } => {
            element_read::read_value(scope, value.into_value()).map(Some)
        }
        ProbeReply::Detached => Ok(None),
        other => Err(resolve::refused(scope, other)),
    }
}

/// A read of the handle's own node.
fn root_read(read: Read) -> ProbeRequest {
    ProbeRequest::Read {
        target: ProbeTarget::Root,
        strictness: Strictness::Strict,
        frame_visible: true,
        read,
    }
}
