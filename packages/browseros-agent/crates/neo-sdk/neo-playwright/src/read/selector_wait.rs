//! Upstream's `waitForSelector` (server/frames.ts at v1.64.1), which `locator.waitFor`,
//! `page.waitForSelector` and `locator.elementHandle` wait on, and which the locator-handler
//! checkpoint waits on for a handler's locator to be hidden.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::backoff::Backoff;
use crate::locator_handler::{self, Prechecks};
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{ProbeReply, ProbeRequest, ProbeTarget, Read};
use crate::read::element_handle::{self, Located};
use crate::read::element_read;
use crate::resolve::{Landed, Resolver};
use crate::target::{LocatorTarget, RootHandle, Strictness};
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
pub(crate) async fn wait_for_match(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &LocatorTarget,
    strict: Strictness,
    state: SelectorState,
    returning: Returning,
) -> Result<Option<RootHandle>, PlaywrightError> {
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

/// The locator-handler checkpoint's wait once a handler ran: upstream's
/// `waitForSelector(progress, { selector }, false, { state: 'hidden' })`. The first match,
/// with no `waiting for` line and no checkpoint of its own, but each poll's `locator resolved
/// to visible <preview>` line. Unbounded, as the checkpoint is: its action's timeout ends it.
pub(crate) async fn wait_hidden(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    target: &LocatorTarget,
) -> Result<(), PlaywrightError> {
    let polls = Polls {
        strict: Strictness::FirstMatch,
        state: SelectorState::Hidden,
        returning: Returning::Nothing,
        prechecks: Prechecks::Skip,
    };
    polls
        .run(scope, page, target, &AtomicBool::new(false))
        .await
        .map(drop)
}

/// [`wait_for_match`]'s polls, which its deadline drops wherever they are. `matched` is set
/// once a poll found an element.
async fn poll_selector(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &LocatorTarget,
    strict: Strictness,
    state: SelectorState,
    returning: Returning,
    matched: &AtomicBool,
) -> Result<Option<RootHandle>, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, &target.page).await?;
    scope.log(format!("waiting for {}{}", target.display, state.suffix()));
    let polls = Polls {
        strict,
        state,
        returning,
        prechecks: Prechecks::Run,
    };
    polls.run(scope, &page, target, matched).await
}

/// What one selector wait polls for.
struct Polls {
    strict: Strictness,
    state: SelectorState,
    returning: Returning,
    /// Upstream's `performActionPreChecksAndLog`: the script's own waits run the checkpoint
    /// before every poll; the checkpoint's wait does not.
    prechecks: Prechecks,
}

impl Polls {
    async fn run(
        &self,
        scope: &OpScope<'_>,
        page: &OpPage<'_>,
        target: &LocatorTarget,
        matched: &AtomicBool,
    ) -> Result<Option<RootHandle>, PlaywrightError> {
        let Self {
            strict,
            state,
            returning,
            prechecks,
        } = *self;
        let resolver = Resolver::new(page, scope, target, strict);
        let backoff = Backoff::outer(scope.timeout());
        for attempt in 0.. {
            if scope.sleep(backoff.wait(attempt)).await.is_err() {
                return Err(scope.timed_out(state.timed_out(matched.load(Ordering::Relaxed))));
            }
            if prechecks == Prechecks::Run {
                Box::pin(locator_handler::checkpoint(scope, page)).await?;
            }
            let request = |last, frame_visible| ProbeRequest::Read {
                target: ProbeTarget::Chunk(last),
                strictness: strict,
                frame_visible,
                read: Read::Located,
            };
            // Its preview reaches the call log and the handle.
            let landed = match resolver.attempt_at(request).await {
                Ok(landed) => landed,
                Err(error)
                    if state.met_without_element()
                        && error.error_code() == Some(ErrorCode::ElementDetached) =>
                {
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };
            let Some(Landed { reply, world, .. }) = landed else {
                if state.met_without_element() {
                    return Ok(None);
                }
                continue;
            };
            let located: Located = match reply {
                ProbeReply::Read { value, .. } => {
                    element_read::read_value(scope, value.into_value())?
                }
                ProbeReply::None { stale_ref: None } | ProbeReply::Detached => {
                    if state.met_without_element() {
                        return Ok(None);
                    }
                    continue;
                }
                // A ref that can never match again: gone is what these two wait for.
                ProbeReply::None { stale_ref: Some(_) } if state.met_without_element() => {
                    return Ok(None);
                }
                other => return Err(resolver.refused(other)),
            };
            scope.log(located.log_line());
            matched.store(true, Ordering::Relaxed);
            if !state.met_by(&located) {
                continue;
            }
            let hands_out = matches!(state, SelectorState::Attached | SelectorState::Visible);
            if returning == Returning::Nothing || !hands_out {
                return Ok(None);
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
                return Ok(Some(brand));
            }
        }
        Err(scope.timed_out(state.timed_out(matched.load(Ordering::Relaxed))))
    }
}
