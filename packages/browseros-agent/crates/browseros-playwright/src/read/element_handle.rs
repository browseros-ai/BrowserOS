//! Element handles: made from an element the probe pinned, and handed out by
//! `locator.elementHandle(s)`, `page.$`, `page.$$` and `page.waitForSelector`.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget, Read, ReadAll};
use crate::read::element_read;
use crate::resolve::{self, Landed, Resolver};
use crate::target::{LocatorTarget, PageRef, Strictness};
use crate::world::{BackendNodeId, Pin, SealedWorld};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// A `located` read's value: the first match, pinned, and what `waitForSelector` checks and
/// logs on each poll.
#[derive(Debug, Deserialize)]
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
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinnedMatch {
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
/// `{"$neo":"element","page":{"$neo":"page","id":7,"tab":1234},"seq":4,"node":1234,
/// "display":"<button>Save</button>"}`, which `ElementTarget` reads back. It names the document
/// and the node's backend id, so it never depends on the pin staying in the probe's table, and
/// carries the op's page brand, so the host's door finds and checks the page of a later op on
/// it. `None` when the element went first.
pub(crate) async fn element_brand(
    scope: &OpScope<'_>,
    page: &PageRef,
    world: &SealedWorld,
    pin: Pin,
    preview: &str,
) -> Result<Option<Value>, PlaywrightError> {
    let object = match probe::pinned_object(scope, world, pin).await {
        Ok(Some(object)) => object,
        Ok(None) | Err(ProbeFailure::Gone) => return Ok(None),
        Err(failure) => return Err(resolve::failed(scope, failure)),
    };
    let described = world
        .session()
        .send::<_, Described>("DOM.describeNode", json!({ "objectId": object }))
        .await;
    let node = match described {
        Ok(described) => described.node.backend_node_id,
        Err(error) => {
            return match ProbeFailure::from_core(error) {
                ProbeFailure::Gone => Ok(None),
                failure => Err(resolve::failed(scope, failure)),
            };
        }
    };
    Ok(Some(json!({
        "$neo": "element",
        "page": page,
        "seq": world.seq(),
        "node": node,
        "display": preview,
    })))
}

/// `page.$` and `frame.$`: the first match now as a handle, or null, as upstream's
/// `querySelector` (no waiting; strict only when the script asked).
pub(crate) async fn query_one(
    pw: &Playwright,
    scope: &OpScope<'_>,
    target: &LocatorTarget,
    strict: Strictness,
) -> Result<Value, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, &target.page).await?;
    let resolver = Resolver::new(&page, scope, target, strict);
    let landed = resolver
        .attempt_at(|last, frame_visible| ProbeRequest::Read {
            target: ProbeTarget::Chunk(last),
            strictness: strict,
            frame_visible,
            read: Read::Located,
        })
        .await?;
    let Some(Landed { reply, world, .. }) = landed else {
        return Ok(Value::Null);
    };
    match reply {
        ProbeReply::Read {
            value,
            sensitive,
            sweep,
        } => {
            element_read::mark_sensitive(scope, sensitive);
            element_read::report_swept(pw, page.id(), &sweep);
            let located: Located = element_read::read_value(scope, value)?;
            let brand =
                element_brand(scope, &target.page, &world, located.pin, &located.preview).await?;
            Ok(brand.unwrap_or(Value::Null))
        }
        ProbeReply::None { stale_ref: None } | ProbeReply::Detached => Ok(Value::Null),
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
) -> Result<Value, PlaywrightError> {
    let page = OpPage::admit_handle(pw, scope, &target.page).await?;
    let resolver = Resolver::new(&page, scope, target, Strictness::FirstMatch);
    let landed = resolver
        .attempt_at(|last, _| ProbeRequest::ReadAll {
            target: ProbeTarget::Chunk(last),
            read: ReadAll::Pins,
        })
        .await?;
    let Some(Landed { reply, world, .. }) = landed else {
        return Ok(json!([]));
    };
    let ProbeReply::Values {
        count,
        values,
        sweep,
    } = reply
    else {
        return Err(resolver.refused(reply));
    };
    element_read::report_swept(pw, page.id(), &sweep);
    let mut handles = Vec::with_capacity(values.len());
    for value in values {
        let PinnedMatch { pin, preview } = element_read::read_value(scope, value)?;
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
    Ok(Value::Array(handles))
}
