//! Element handles: made from an element the probe pinned, and handed out by
//! `locator.elementHandle(s)`, `page.$`, `page.$$` and `page.waitForSelector`.

use serde::Deserialize;
use serde_json::json;
use ts_rs::TS;

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget, Read, ReadAll};
use crate::read::element_read;
use crate::resolve::{self, Landed, Resolver};
use crate::target::{ElementTarget, LocatorTarget, PageRef, RootHandle, Strictness};
use crate::world::{BackendNodeId, IsolatedWorld, Pin};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// A `located` read's value: the first match, pinned, and what `waitForSelector` checks and
/// logs on each poll.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct Located {
    pub(crate) pin: Pin,
    pub(crate) preview: String,
    pub(crate) visible: bool,
    pub(crate) count: u32,
}

impl Located {
    /// Upstream's `waitForSelector` call-log line for one poll.
    pub(crate) fn log_line(&self) -> String {
        let Self {
            preview,
            visible,
            count,
            ..
        } = self;
        if *count > 1 {
            format!(
                "  locator resolved to {count} elements. Proceeding with the first one: {preview}"
            )
        } else if *visible {
            format!("  locator resolved to visible {preview}")
        } else {
            format!("  locator resolved to hidden {preview}")
        }
    }
}

/// One value of a `readAll` `pins`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct PinnedMatch {
    pin: Pin,
    preview: String,
}

/// `DOM.describeNode`'s answer, for the node's backend id.
#[derive(Deserialize)]
struct Described {
    node: DescribedNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescribedNode {
    backend_node_id: BackendNodeId,
}

/// The handle for the element `pin` names in `world`: wire.ts's `ElementBrand`,
/// `{"$neo":"element","page":{"$neo":"page","tab":1234},"seq":4,"node":1234,
/// "display":"<button>Save</button>"}`, which `ElementTarget` reads back. It names the document
/// and the node's backend id, so it never depends on the pin staying in the probe's table, and
/// carries the op's page brand, so the host's door finds and checks the page of a later op on
/// it. `None` when the element went first.
pub(crate) async fn element_brand(
    scope: &OpScope<'_>,
    page: &PageRef,
    world: &IsolatedWorld,
    pin: Pin,
    preview: &str,
) -> Result<Option<RootHandle>, PlaywrightError> {
    let Some(node) = backend_node(scope, world, pin).await? else {
        return Ok(None);
    };
    Ok(Some(RootHandle::Element(ElementTarget {
        page: page.clone(),
        seq: world.seq(),
        node,
        display: preview.to_owned(),
    })))
}

/// The CDP backend node id of the element `pin` names in `world`, which names the node on
/// that world's session for good; `None` when the element or the world went first.
pub(crate) async fn backend_node(
    scope: &OpScope<'_>,
    world: &IsolatedWorld,
    pin: Pin,
) -> Result<Option<BackendNodeId>, PlaywrightError> {
    let isolated = match probe::pinned_object(scope, world, pin).await {
        Ok(Some(object)) => object,
        Ok(None) | Err(ProbeFailure::Gone(_)) => return Ok(None),
        Err(failure) => return Err(resolve::failed(scope, failure)),
    };
    let described = world
        .session()
        .send::<_, Described>("DOM.describeNode", json!({ "objectId": isolated }))
        .await;
    match described {
        Ok(described) => Ok(Some(described.node.backend_node_id)),
        Err(error) => match ProbeFailure::from_core(error) {
            ProbeFailure::Gone(_) => Ok(None),
            failure => Err(resolve::failed(scope, failure)),
        },
    }
}

/// Every match of `target` now, pinned in the world the query landed in, as upstream's
/// `queryArrayInMainWorld` takes them for `evaluateAll` and `$$eval`: no waiting, never strict.
/// `None` when a frame on the way is not there yet. Nothing of the matches is read: the
/// previews the probe sends with the pins stay here. A match that leaves before it is found
/// again fails the op, rather than hand the script fewer elements than matched.
pub(crate) async fn every_match(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    target: &LocatorTarget,
) -> Result<Option<(IsolatedWorld, Vec<BackendNodeId>)>, PlaywrightError> {
    let resolver = Resolver::new(page, scope, target, Strictness::FirstMatch);
    let landed = resolver
        .attempt_at(|last, _| ProbeRequest::ReadAll {
            target: ProbeTarget::Chunk(last),
            read: ReadAll::Pins,
        })
        .await?;
    let Some(Landed { reply, world, .. }) = landed else {
        return Ok(None);
    };
    let ProbeReply::Values { count, values, .. } = reply else {
        return Err(resolver.refused(reply));
    };
    let mut pins = Vec::with_capacity(values.len());
    for value in values {
        let pin = value.pin().ok_or_else(|| {
            scope.fail(
                ErrorCode::Internal,
                "a pins read sent a match without its pin",
            )
        })?;
        pins.push(pin);
    }
    let nodes = match probe::pinned_nodes(scope, &world, pins).await {
        Ok(Some(nodes)) => nodes,
        Ok(None) | Err(ProbeFailure::Gone(_)) => {
            return Err(scope.fail(
                ErrorCode::TargetClosed,
                &format!("{count} elements matched, but the page changed before the script ran; call it again"),
            ));
        }
        Err(failure) => return Err(resolve::failed(scope, failure)),
    };
    Ok(Some((world, nodes)))
}

/// `page.$` and `frame.$`: the first match now as a handle, or null, as upstream's
/// `querySelector` (no waiting; strict only when the script asked).
pub(crate) async fn find_one(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &LocatorTarget,
    strict: Strictness,
) -> Result<Option<RootHandle>, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, &target.page).await?;
    let resolver = Resolver::new(&page, scope, target, strict);
    let request = |last, frame_visible| ProbeRequest::Read {
        target: ProbeTarget::Chunk(last),
        strictness: strict,
        frame_visible,
        read: Read::Located,
    };
    // Its preview becomes the handle's display.
    let Some(Landed { reply, world, .. }) = resolver.attempt_at(request).await? else {
        return Ok(None);
    };
    match reply {
        ProbeReply::Read { value, .. } => {
            let located: Located = element_read::read_value(scope, value.into_value())?;
            let brand =
                element_brand(scope, &target.page, &world, located.pin, &located.preview).await?;
            Ok(brand)
        }
        ProbeReply::None { stale_ref: None } | ProbeReply::Detached => Ok(None),
        other => Err(resolver.refused(other)),
    }
}

/// `locator.elementHandles()`, `page.$$` and `frame.$$`: every match now, each made a handle,
/// as upstream's `querySelectorAll` (no waiting, never strict).
///
/// The probe pins the matches as one batch, which counts once toward its pin limit, and a
/// pinned element answers by handle even after it left the page, so each match gets a handle
/// (one that left reads as detached, as upstream's would). A handle that still cannot be made
/// (512 newer pin entries, or the document went away meanwhile) fails the op rather than hand
/// back fewer handles than there were matches.
pub(crate) async fn query_all(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &LocatorTarget,
) -> Result<Vec<RootHandle>, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, &target.page).await?;
    let resolver = Resolver::new(&page, scope, target, Strictness::FirstMatch);
    let request = |last, _| ProbeRequest::ReadAll {
        target: ProbeTarget::Chunk(last),
        read: ReadAll::Pins,
    };
    // Each match's preview becomes its handle's display.
    let Some(Landed { reply, world, .. }) = resolver.attempt_at(request).await? else {
        return Ok(Vec::new());
    };
    let ProbeReply::Values { count, values } = reply else {
        return Err(resolver.refused(reply));
    };
    let mut handles = Vec::with_capacity(values.len());
    for value in values {
        let PinnedMatch { pin, preview } = element_read::read_value(scope, value.into_value())?;
        let Some(brand) = element_brand(scope, &target.page, &world, pin, &preview).await? else {
            return Err(scope.fail(
                ErrorCode::TargetClosed,
                &format!(
                    "{count} elements matched, but the page changed before the handle of match \
                     {} was made; call it again",
                    handles.len() + 1
                ),
            ));
        };
        handles.push(brand);
    }
    Ok(handles)
}
