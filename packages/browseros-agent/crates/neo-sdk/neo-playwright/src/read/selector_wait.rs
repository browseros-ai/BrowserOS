//! Upstream's `waitForSelector` (server/frames.ts at v1.64.1), which `locator.waitFor`,
//! `page.waitForSelector` and `locator.elementHandle` wait on.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use crate::backoff::Backoff;
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{ProbeReply, ProbeRequest, ProbeTarget, Read};
use crate::read::element_handle::{self, Located};
use crate::read::element_read;
use crate::resolve::{Landed, Resolver};
use crate::target::{LocatorTarget, Strictness};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// What a selector wait waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectorState {
    Attached,
    Detached,
    Visible,
    Hidden,
}

/// What a wait that succeeds hands back. Only `attached` and `visible` have an element to hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Returning {
    /// `page.waitForSelector` and `locator.elementHandle`: the element, as a handle.
    Handle,
    /// `locator.waitFor` and `omitReturnValue`.
    Nothing,
}

impl SelectorState {
    /// Upstream's first call-log line names the state, except `attached`.
    fn suffix(self) -> &'static str {
        match self {
            Self::Attached => "",
            Self::Detached => " to be detached",
            Self::Visible => " to be visible",
            Self::Hidden => " to be hidden",
        }
    }

    /// Whether no element at all is what the wait waits for.
    fn met_without_element(self) -> bool {
        matches!(self, Self::Detached | Self::Hidden)
    }

    /// Whether the first match meets the state.
    fn met_by(self, located: &Located) -> bool {
        match self {
            Self::Attached => true,
            Self::Detached => false,
            Self::Visible => located.visible,
            Self::Hidden => !located.visible,
        }
    }

    /// The error a wait that ran out of time ends with: nothing matched (`no_match`), the match
    /// stayed hidden (`not_actionable`), or it never went away (`wait_timeout`).
    fn timed_out(self, matched: bool) -> ErrorCode {
        match self {
            Self::Visible if matched => ErrorCode::NotActionable,
            Self::Attached | Self::Visible => ErrorCode::NoMatch,
            Self::Detached | Self::Hidden => ErrorCode::WaitTimeout,
        }
    }
}

/// Waits until `target`'s first match (the only one under `strict`) is in `state`, on the outer
/// backoff, logging `waiting for <locator> to be <state>` and each poll's `locator resolved to
/// visible <preview>`. No element, or a frame on the way that is not there, meets `detached`
/// and `hidden` at once, and so does a ref whose document or element is gone for good. A wait
/// that meets `attached` or `visible` with [`Returning::Handle`] answers the element's handle,
/// every other success null.
///
/// The timeout covers every await of the op, admission and making the handle after a poll
/// matched included, so a busy renderer cannot keep the op past its deadline.
pub(crate) async fn wait_for_selector(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &LocatorTarget,
    strict: Strictness,
    state: SelectorState,
    returning: Returning,
) -> Result<Value, PlaywrightError> {
    // Outside the polls, so the timeout error still knows whether any poll matched.
    let matched = AtomicBool::new(false);
    // Boxed: every op's future is one variant of `Playwright::invoke`'s, which a host awaits.
    let polls = Box::pin(poll_selector(
        pw, scope, target, strict, state, returning, &matched,
    ));
    match scope.within(polls).await {
        Ok(waited) => waited,
        Err(_expired) => Err(scope.timed_out(state.timed_out(matched.load(Ordering::Relaxed)))),
    }
}

/// [`wait_for_selector`]'s polls, which its deadline drops wherever they are. `matched` is set
/// once a poll found an element.
async fn poll_selector(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &LocatorTarget,
    strict: Strictness,
    state: SelectorState,
    returning: Returning,
    matched: &AtomicBool,
) -> Result<Value, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, &target.page).await?;
    let resolver = Resolver::new(&page, scope, target, strict);
    scope.log(format!("waiting for {}{}", target.display, state.suffix()));
    let backoff = Backoff::outer(scope.timeout());
    for attempt in 0.. {
        if scope.sleep(backoff.wait(attempt)).await.is_err() {
            return Err(scope.timed_out(state.timed_out(matched.load(Ordering::Relaxed))));
        }
        let request = |last, frame_visible| ProbeRequest::Read {
            target: ProbeTarget::Chunk(last),
            strictness: strict,
            frame_visible,
            read: Read::Located,
        };
        // Its preview reaches the call log and the handle: the site's values go with it.
        let landed = match resolver.attempt_masked(request).await {
            Ok(landed) => landed,
            Err(error)
                if state.met_without_element()
                    && error.error_code() == Some(ErrorCode::ElementDetached) =>
            {
                return Ok(Value::Null);
            }
            Err(error) => return Err(error),
        };
        let Some(Landed {
            reply,
            world,
            alongside: site,
            ..
        }) = landed
        else {
            if state.met_without_element() {
                return Ok(Value::Null);
            }
            continue;
        };
        let located: Located = match reply {
            ProbeReply::Read { value, sweep, .. } => {
                let value = site.with_own(pw, page.tab(), sweep).mask(value);
                element_read::read_value(scope, value)?
            }
            ProbeReply::None { stale_ref: None } | ProbeReply::Detached => {
                if state.met_without_element() {
                    return Ok(Value::Null);
                }
                continue;
            }
            // A ref that can never match again: gone is what these two wait for.
            ProbeReply::None { stale_ref: Some(_) } if state.met_without_element() => {
                return Ok(Value::Null);
            }
            other => return Err(site.refused(scope, other)),
        };
        scope.log(located.log_line());
        matched.store(true, Ordering::Relaxed);
        if !state.met_by(&located) {
            continue;
        }
        let hands_out = matches!(state, SelectorState::Attached | SelectorState::Visible);
        if returning == Returning::Nothing || !hands_out {
            return Ok(Value::Null);
        }
        // Upstream polls again when the element went before its handle was made.
        if let Some(brand) = element_handle::element_brand(
            scope,
            &target.page,
            &world,
            located.pin,
            &located.preview,
        )
        .await?
        {
            return Ok(brand);
        }
    }
    Err(scope.timed_out(state.timed_out(matched.load(Ordering::Relaxed))))
}
