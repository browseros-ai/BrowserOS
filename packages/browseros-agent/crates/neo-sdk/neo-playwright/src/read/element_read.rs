//! The reads of one element: those that wait for it (`textContent`, `innerText`, `innerHTML`,
//! `getAttribute`, `inputValue`, `boundingBox`, `isEnabled`, `isDisabled`, `isEditable`,
//! `isChecked`) and the two that never do (`isVisible`, `isHidden`), with their page, frame and
//! element-handle forms. A locator is re-resolved on every attempt; an element handle's node
//! is read once, because it is there or it is gone for good.

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::backoff::Backoff;
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{ProbeReply, ProbeRequest, ProbeTarget, QueryState, Read};
use crate::resolve::{self, FramePath, HandleSite, Landed, Resolver};
use crate::target::{ElementTarget, LocatorTarget, Strictness, Target};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// Which of the two reads that answer at once, element or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Visibility {
    Visible,
    Hidden,
}

impl Visibility {
    /// The state the probe checks, with the frames' visibility.
    fn state(self) -> QueryState {
        match self {
            Self::Visible => QueryState::Visible,
            Self::Hidden => QueryState::Hidden,
        }
    }

    /// The answer when nothing matches: nothing is visible, so it is hidden.
    fn without_element(self) -> bool {
        self == Self::Hidden
    }
}

/// What a read got: the value, and the parent frames of the element it was read from, which a
/// box needs to reach the main viewport.
#[derive(Debug)]
pub(crate) struct Reading {
    pub(crate) value: Value,
    pub(crate) path: FramePath,
}

/// Reads `target`'s element with `read`, once it is there. A handle whose node is gone fails
/// `element_detached`.
pub(crate) async fn read_attached(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &Target,
    strict: Strictness,
    read: &Read,
) -> Result<Reading, PlaywrightError> {
    match target {
        Target::Locator(target) => read_locator(pw, scope, target, strict, read).await,
        Target::Element(target) => read_handle(pw, scope, target, read)
            .await?
            .ok_or_else(|| resolve::handle_detached(scope)),
    }
}

/// Upstream's `_waitForFunctionOnSelector`: `waiting for <locator>` in the call log, then
/// attempts on the outer backoff until the op's timeout (`no_match`). A strict violation, a
/// stale ref and an error the read throws in the page end it at once.
///
/// The timeout covers every await of the op, admission and each attempt's CDP calls included:
/// a busy renderer can hold one call for over a second, and the op still ends at its deadline.
async fn read_locator(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &LocatorTarget,
    strict: Strictness,
    read: &Read,
) -> Result<Reading, PlaywrightError> {
    // Boxed: every op's future is one variant of `Playwright::invoke`'s, which a host awaits.
    let attempts = Box::pin(poll_locator(pw, scope, target, strict, read));
    match scope.within(attempts).await {
        Ok(reading) => reading,
        Err(_expired) => Err(scope.timed_out(ErrorCode::NoMatch)),
    }
}

/// [`read_locator`]'s attempts, which its deadline drops wherever they are.
async fn poll_locator(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &LocatorTarget,
    strict: Strictness,
    read: &Read,
) -> Result<Reading, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, &target.page).await?;
    let resolver = Resolver::new(&page, scope, target, strict);
    scope.log(format!("waiting for {}", target.display));
    let backoff = Backoff::outer(scope.timeout());
    for attempt in 0.. {
        if scope.sleep(backoff.wait(attempt)).await.is_err() {
            return Err(scope.timed_out(ErrorCode::NoMatch));
        }
        let request = |last, frame_visible| ProbeRequest::Read {
            target: ProbeTarget::Chunk(last),
            strictness: strict,
            frame_visible,
            read: read.clone(),
        };
        let Some(Landed { reply, path, .. }) = resolver.attempt_at(request).await? else {
            continue;
        };
        match reply {
            ProbeReply::Read { value, .. } => {
                return Ok(Reading {
                    value: value.into_value(),
                    path,
                });
            }
            ProbeReply::None { stale_ref: None } | ProbeReply::Detached => {}
            other => return Err(resolve::refused(scope, other)),
        }
    }
    Err(scope.timed_out(ErrorCode::NoMatch))
}

/// One read of an element handle's node; `None` when the node is gone for good. Its frames'
/// visibility is not the handle's to know, so it counts as visible, as upstream's handle
/// queries start from `frameVisible: true`.
async fn read_handle(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &ElementTarget,
    read: &Read,
) -> Result<Option<Reading>, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, &target.page).await?;
    let Some(site) = HandleSite::find(&page, scope, target).await? else {
        return Ok(None);
    };
    let request = ProbeRequest::Read {
        target: ProbeTarget::Root,
        strictness: Strictness::Strict,
        frame_visible: true,
        read: read.clone(),
    };
    match site.call(&request).await? {
        ProbeReply::Read { value, .. } => Ok(Some(Reading {
            value: value.into_value(),
            path: site.path().clone(),
        })),
        ProbeReply::Detached => Ok(None),
        // A read of the handle's own node finds one element, so no strict violation.
        other => Err(resolve::refused(scope, other)),
    }
}

/// `isVisible` and `isHidden`, as upstream's `isVisibleInternal`: one attempt, no waiting and
/// no timeout of its own, so an element that is not there, or a frame on the way that is not,
/// reads as not visible. Visibility counts every iframe on the way (upstream's
/// `frameVisible`). Strict violations, stale refs and the page's own errors still fail it. A
/// handle whose node is gone is not visible either, as upstream answers for a detached node.
pub(crate) async fn read_visibility(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &Target,
    strict: Strictness,
    visibility: Visibility,
) -> Result<Value, PlaywrightError> {
    let read = Read::State {
        state: visibility.state(),
    };
    let without_element = Value::Bool(visibility.without_element());
    let target = match target {
        Target::Locator(target) => target,
        Target::Element(target) => {
            let reading = read_handle(pw, scope, target, &read).await?;
            return Ok(reading.map_or(without_element, |reading| reading.value));
        }
    };
    let page = OpPage::admit_handle(pw, scope, &target.page).await?;
    let resolver = Resolver::new(&page, scope, target, strict);
    scope.log(format!("  checking visibility of {}", target.display));
    let landed = resolver
        .attempt_at(|last, frame_visible| ProbeRequest::Read {
            target: ProbeTarget::Chunk(last),
            strictness: strict,
            frame_visible,
            read: read.clone(),
        })
        .await?;
    let Some(Landed { reply, .. }) = landed else {
        return Ok(without_element);
    };
    match reply {
        ProbeReply::None { stale_ref: None } | ProbeReply::Detached => Ok(without_element),
        ProbeReply::Read { value, .. } => Ok(value.into_value()),
        other => Err(resolve::refused(scope, other)),
    }
}

/// A read's value in the shape protocol.ts gives it; anything else is a stale asset.
pub(crate) fn read_value<T: DeserializeOwned>(
    scope: &OpScope<'_>,
    value: Value,
) -> Result<T, PlaywrightError> {
    serde_json::from_value(value).map_err(|error| {
        scope.fail(
            ErrorCode::Internal,
            &format!("the probe read a value this build cannot read: {error}"),
        )
    })
}
