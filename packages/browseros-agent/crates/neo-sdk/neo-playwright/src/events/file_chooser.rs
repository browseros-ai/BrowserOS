//! Adopts a native chooser's input into Neo's existing element-handle contract. Uploads then
//! use `locator.setInputFiles`, including its host-file permission and native file validation.

use browseros_cdp::CdpError;
use browseros_core::{CoreError, PageJs};
use serde::Serialize;
use serde_json::{Value, json};
use ts_rs::TS;

use crate::events::page_event::FileChooserOpened;
use crate::op_scope::OpScope;
use crate::page::{OpPage, PageHandle, world_call};
use crate::probe::ProbeFailure;
use crate::world::{BackendNodeId, FrameSeq};
use crate::{ErrorCode, Playwright, PlaywrightError, resolve};

/// A chooser's input and its `multiple` property, read once like upstream's server does.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "FileChooserInfo")]
pub(crate) struct FileChooser {
    // Prints as the element brand every element handle shares (target.rs), which the prelude's
    // `page._handle` revives; this struct is only the output side of that same shape.
    #[ts(as = "crate::target::RootHandle")]
    element: ChooserElement,
    is_multiple: bool,
}

/// The chooser's exact `<input>`, as the element brand `page.$` hands a script, rather than a
/// selector that could resolve to a replacement: its page, document and node.
// Fields in sorted key order, the order the `json!` this replaced printed them in.
#[derive(Debug, Serialize)]
struct ChooserElement {
    #[serde(rename = "$neo")]
    brand: ElementBrand,
    /// Playwright's preview of the element, for messages.
    display: &'static str,
    node: BackendNodeId,
    page: PageHandle,
    seq: FrameSeq,
}

/// The only brand an element handle carries.
#[derive(Debug, Serialize)]
enum ElementBrand {
    #[serde(rename = "element")]
    Element,
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
        let op_page = OpPage::observed(pw, &admitted).await;
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
            .send::<_, Value>(
                "DOM.resolveNode",
                json!({
                    "backendNodeId": node,
                    "executionContextId": world.context(),
                    "objectGroup": scope.run_state().objects.on(world.session()),
                }),
            )
            .await;
        if let Err(error) = resolved {
            return disappeared(scope, error);
        }
        // Only this boolean crosses the protocol boundary; the native node becomes the
        // same document/node brand used by page.$ and locator.elementHandle.
        let multiple = PageJs::isolated_only(world.session())
            .in_frame(&opened.frame)
            .call::<bool>(node.0, "function() { return !!this.multiple; }", &[])
            .await;
        let multiple = match multiple {
            Ok(multiple) => multiple,
            Err(error) => return disappeared(scope, error),
        };
        Ok(Some(Self {
            element: ChooserElement {
                brand: ElementBrand::Element,
                display: "<input type=\"file\">",
                node,
                page: page.clone(),
                seq: world.seq(),
            },
            is_multiple: multiple,
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

#[cfg(test)]
mod tests {
    use browseros_core::TabId;
    use serde_json::json;

    use super::{ChooserElement, ElementBrand, FileChooser};
    use crate::page::PageHandle;
    use crate::world::{BackendNodeId, FrameSeq};

    /// The typed element is the brand `adopt` built with `json!` before: same keys and values,
    /// printed in the same order once `to_output` makes its JSON value of the reply.
    #[test]
    fn tests_that_the_chooser_keeps_its_element_brand_bytes() -> Result<(), serde_json::Error> {
        let page = PageHandle::new(TabId(1007));
        let chooser = FileChooser {
            element: ChooserElement {
                brand: ElementBrand::Element,
                display: "<input type=\"file\">",
                node: BackendNodeId(1234),
                page: page.clone(),
                seq: FrameSeq(4),
            },
            is_multiple: true,
        };
        let before = json!({
            "element": {
                "$neo": "element", "page": page, "seq": FrameSeq(4), "node": BackendNodeId(1234),
                "display": "<input type=\"file\">",
            },
            "isMultiple": true,
        });
        let element = r#"{"$neo":"element","display":"<input type=\"file\">","node":1234,"page":{"$neo":"page","tab":1007},"seq":4}"#;
        let after = serde_json::to_value(&chooser)?;
        assert_eq!(serde_json::to_string(&before["element"])?, element);
        assert_eq!(serde_json::to_string(&after["element"])?, element);
        assert_eq!(after, before);
        Ok(())
    }
}
