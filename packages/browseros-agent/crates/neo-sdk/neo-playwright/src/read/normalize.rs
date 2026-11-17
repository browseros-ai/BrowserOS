//! `locator.normalize()`: upstream's `Frame.resolveSelector` (server/frames.ts at v1.64.1). It
//! queries the locator once, without waiting and without strict mode, has Playwright's selector
//! generator write a selector for the element it found, then one for each `<iframe>` between the
//! main frame and that element, and joins them top down:
//!
//! ```text
//!  resolver.attempt_at ── selector { chunk } in the element's world ── generateSelectorSimple
//!  each hop of the path ── selector { iframe } in the parent's world  (pin or ref key)
//!  "<iframe>" >> internal:control=enter-frame >> ... >> "<element>"
//! ```
//!
//! A ref jump's path is the parent links its frame was reached through, so `getByRef('f4e2')`
//! comes back as the chain of frame locators upstream would print.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget};
use crate::resolve::{self, FramePath, Landed, Resolver};
use crate::target::{Strictness, Target};
use crate::wire::options;
use crate::{ErrorCode, Playwright, PlaywrightError};

/// What joins the parts, as upstream's `resolveSelector` joins them.
const ENTER_FRAME: &str = " >> internal:control=enter-frame >> ";

/// `locator.normalize()`: `{ target, selector }`. `selector` is the locator's own selector as
/// the script built it, which upstream's two messages quote.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "ReadNormalizeInput")]
pub(crate) struct NormalizeInput {
    target: Target,
    selector: String,
}
options!(NormalizeInput);

/// The generator's selector for the first match, framed from the main document down.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "ReadNormalized")]
pub(crate) struct Normalized {
    resolved_selector: String,
}

/// Runs `locator.normalize`; its output is `{ resolvedSelector }`. Nothing matching now fails at once,
/// as upstream's query does: `element_detached`, because the page holds no such element and no
/// wait is involved (upstream throws a plain error, not a timeout).
pub(crate) async fn normalize(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: NormalizeInput,
) -> Result<Normalized, PlaywrightError> {
    let Target::Locator(target) = &input.target else {
        return Err(scope.not_built());
    };
    let page = OpPage::admit_handle(pw, scope, &target.page).await?;
    let resolver = Resolver::new(&page, scope, target, Strictness::FirstMatch);
    let request = |last, _| ProbeRequest::Selector {
        target: ProbeTarget::Chunk(last),
    };
    let no_element = || {
        scope.fail(
            ErrorCode::ElementDetached,
            &format!("No element matching {}", input.selector),
        )
    };
    let landed = match resolver.attempt_at(request).await {
        Ok(landed) => landed,
        // A ref whose document is gone (`element_detached` from a ref jump): upstream's query
        // finds nothing there either, and says so as for any other miss.
        Err(error) if error.error_code() == Some(ErrorCode::ElementDetached) => {
            return Err(no_element());
        }
        Err(error) => return Err(error),
    };
    let Some(Landed { reply, path, .. }) = landed else {
        return Err(no_element());
    };
    let element = match reply {
        ProbeReply::Selector { selector } => selector,
        ProbeReply::None { .. } | ProbeReply::Detached => return Err(no_element()),
        other => return Err(resolver.refused(other)),
    };
    let mut parts = frame_selectors(scope, &page, &path, &input.selector).await?;
    parts.push(element);
    Ok(Normalized {
        // Translate only the op reply; the probe's `selector` protocol stays unchanged.
        resolved_selector: parts.join(ENTER_FRAME),
    })
}

/// The selector of each `<iframe>` on `path`, top down, each written in its parent's world.
/// An iframe that is gone, or a parent world that is, fails as upstream's does when a frame
/// element cannot be found again.
async fn frame_selectors(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    path: &FramePath,
    selector: &str,
) -> Result<Vec<String>, PlaywrightError> {
    let unable = || {
        scope.fail(
            ErrorCode::ElementDetached,
            &format!("Unable to generate locator for {selector}"),
        )
    };
    let mut parts = Vec::with_capacity(path.hops().len());
    for hop in path.hops() {
        let target = hop
            .iframe
            .target()
            .map_err(|error| scope.fail(ErrorCode::Internal, &error.to_string()))?;
        match probe::call(scope, page, &hop.parent, &ProbeRequest::Selector { target }).await {
            Ok(ProbeReply::Selector { selector }) => parts.push(selector),
            Ok(ProbeReply::None { .. } | ProbeReply::Detached) | Err(ProbeFailure::Gone(_)) => {
                return Err(unable());
            }
            Ok(other) => return Err(resolve::refused(scope, other)),
            Err(failure) => return Err(resolve::failed(scope, failure)),
        }
    }
    Ok(parts)
}
