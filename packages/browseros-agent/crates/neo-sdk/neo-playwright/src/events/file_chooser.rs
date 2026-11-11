//! Adopts a native chooser's input into Neo's existing element-handle contract. Uploads then
//! use `locator.setInputFiles`, including its host-file permission and native file validation.

use browseros_cdp::CdpError;
use browseros_core::CoreError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::events::page_event::FileChooserOpened;
use crate::op_scope::OpScope;
use crate::page::{OpPage, PageHandle, world_call};
use crate::probe::ProbeFailure;
use crate::world::BackendNodeId;
use crate::{ErrorCode, Playwright, PlaywrightError, resolve};

/// A chooser's input and its `multiple` property, read once like upstream's server does.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileChooser {
    element: Value,
    is_multiple: bool,
}

#[derive(Deserialize)]
struct Resolved {
    object: RemoteNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteNode {
    object_id: String,
}

#[derive(Deserialize)]
struct Multiple {
    result: MultipleValue,
}

#[derive(Deserialize)]
struct MultipleValue {
    value: bool,
}

impl FileChooser {
    /// A disappeared frame or input produces no chooser, as upstream's async adoption does.
    /// Nothing is reselected by CSS: a chooser always keeps the exact input Chromium named.
    pub(crate) async fn adopt(
        pw: &Playwright,
        scope: &OpScope<'_>,
        page: &PageHandle,
        opened: &FileChooserOpened,
    ) -> Result<Option<Self>, PlaywrightError> {
        let admitted = match scope.observe_page(pw.browser(), page.tab()).await {
            Ok(Some(admitted)) => admitted,
            Ok(None) => return Ok(None),
            Err(error) if error.error_code() == Some(ErrorCode::TargetClosed) => return Ok(None),
            Err(error) => return Err(error),
        };
        let op_page = OpPage::observed(pw, scope, &admitted).await;
        let world = match world_call::listed_frame_world(&op_page, scope, Some(&opened.frame)).await
        {
            Ok(Some(world)) if world.session().session_id() == Some(&opened.session) => world,
            // Navigation or detachment may win the race against event delivery.
            Ok(_) => return Ok(None),
            Err(error)
                if matches!(
                    error.error_code(),
                    Some(ErrorCode::TargetClosed | ErrorCode::ElementDetached)
                ) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let node: BackendNodeId = opened.node;
        let resolved = world
            .session()
            .send::<_, Resolved>(
                "DOM.resolveNode",
                json!({
                    "backendNodeId": node,
                    "executionContextId": world.context(),
                    "objectGroup": scope.object_group(world.session()),
                }),
            )
            .await;
        let resolved = match resolved {
            Ok(resolved) => resolved,
            Err(error) => return disappeared(scope, error),
        };
        // Only this boolean crosses the protocol boundary; the native node becomes the
        // same document/node brand used by page.$ and locator.elementHandle.
        let multiple = world
            .session()
            .send::<_, Multiple>(
                "Runtime.callFunctionOn",
                json!({
                    "objectId": resolved.object.object_id,
                    "functionDeclaration": "function() { return !!this.multiple; }",
                    "returnByValue": true,
                }),
            )
            .await;
        let multiple = match multiple {
            Ok(multiple) => multiple,
            Err(error) => return disappeared(scope, error),
        };
        Ok(Some(Self {
            element: json!({
                "$neo": "element", "page": page, "seq": world.seq(), "node": node,
                "display": "<input type=\"file\">",
            }),
            is_multiple: multiple.result.value,
        }))
    }
}

/// Context/frame/node disappearance is ordinary event loss during navigation. Transport,
/// schema and installation errors retain their source instead of becoming a missing event.
fn disappeared<T>(scope: &OpScope<'_>, error: CoreError) -> Result<Option<T>, PlaywrightError> {
    let missing_node = matches!(&error, CoreError::Cdp(CdpError::Protocol { message, .. })
        if message.contains("No node with given id found") || message.contains("Could not find node with given id"));
    match ProbeFailure::from_core(error) {
        ProbeFailure::Gone(_) => Ok(None),
        ProbeFailure::Cdp(_) if missing_node => Ok(None),
        failure => Err(resolve::failed(scope, failure)),
    }
}
