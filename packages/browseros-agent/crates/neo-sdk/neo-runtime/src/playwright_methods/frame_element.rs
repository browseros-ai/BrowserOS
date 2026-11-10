//! Neo: `Frame.frameElement`: the `<iframe>` or `<frame>` element that owns a child frame, as
//! an `ElementHandle` of the run in its parent's main world (eval's `element_handle_for`). Its
//! result requires an `ElementHandle`, which the generated spec cannot create, so dispatch
//! routes the method by [`FRAME_ELEMENT_META`] and the reply is the record the SDK builds its
//! `ElementHandle` from, as eval's handle-returning calls are.
//!
//! ```text
//!  Frame.frameElement {}   the main frame → "Frame has been detached."
//!    the frame's parent context and id   (Frames::owners, which refreshes the tree)
//!    DOM.getFrameOwner in the parent's session → the owner's backendNodeId
//!    → the owner in the parent's utility world → its previewNode → adopted into the parent's
//!      main world as a new handle of the run → { element }
//! ```

use browseros_cdp::CdpError;
use std::sync::Arc;

use neo_runtime_spec::injected::InjectedPreviewNodeParams;
use neo_runtime_spec::{MethodGroup, MethodMeta};

use crate::browser_representations::page::FrameTab;
use crate::chromium_bridge::cdp::{self, Command};
use crate::chromium_bridge::frames::{FrameContext, FrameScope};
use crate::playwright_methods::evaluate::{self, CreatedHandle};
use crate::playwright_methods::tab::PageError;
use crate::progress::{CallError, PlaywrightError, Progress};

/// Playwright: `server/chromium/crPage.ts` `CRPage.getFrameElement`: `throw new Error('Frame has been detached.');`
const DETACHED: &str = "Frame has been detached.";

/// Neo: `Frame.frameElement`'s trace metadata (`neo-runtime-spec/spec/playwright/frame.yml`).
pub(crate) static FRAME_ELEMENT_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "frameElement",
    title: Some("Get frame element"),
    subtitle: None,
    group: Some(MethodGroup::Getter),
    render_params: &[],
    internal: false,
    flags: &[],
};

/// Serves `Frame.frameElement`: the frame's owner element, adopted into its parent's main world.
/// A frame that is gone, or whose owner is gone, has been detached, as in Playwright.
///
/// Playwright: `server/chromium/crPage.ts` `CRPage.getFrameElement`: `return parentSession._adoptBackendNodeId(backendNodeId, await parent.mainContext());`
pub(crate) async fn frame_element(
    tab: &FrameTab,
    progress: &Progress,
) -> Result<CreatedHandle, CallError> {
    // Playwright: `server/chromium/crPage.ts` `CRPage.getFrameElement`: `if (!parent)`
    let FrameScope::Frame(frame) = &tab.scope else {
        return Err(PlaywrightError::error(DETACHED).into());
    };
    let owners = progress
        .race(tab.frames.owners(frame))
        .await?
        .map_err(detached)?;
    let Some((parent, child)) = owners.last() else {
        return Err(PlaywrightError::error(DETACHED).into());
    };
    progress
        .race(owner_handle(tab, parent, child, progress))
        .await?
        .map_err(detached)
}

/// The owner of `child` in `parent`, previewed in the parent's utility world, then adopted.
async fn owner_handle(
    tab: &FrameTab,
    parent: &Arc<FrameContext>,
    child: &str,
    progress: &Progress,
) -> Result<CreatedHandle, PageError> {
    // Playwright: `server/chromium/crPage.ts` `CRPage.getFrameElement`: `const { backendNodeId } = await parentSession._client.send('DOM.getFrameOwner', { frameId: frame._id }).catch(e => {`
    let owner = parent
        .host
        .send_command(cdp::dom::GetFrameOwner {
            frame_id: child.to_string(),
        })
        .await
        .map_err(|error| match error {
            CdpError::Json(message) => PageError::Contract(format!(
                "{} answered: {message}",
                cdp::dom::GetFrameOwner::METHOD
            )),
            error => parent.host.classify(&error),
        })?;
    let backend = owner.backend_node_id;
    let group = tab.group(parent, "frame-element");
    let element = parent.element(group.group(), backend).await?;
    // Playwright: `server/dom.ts` `ElementHandle._initializePreview`: `const utility = await this._context.injectedScript();`
    let preview = parent
        .call(
            None,
            &InjectedPreviewNodeParams {
                node: element.clone(),
            },
        )
        .await?
        .preview;
    evaluate::element_handle_for(tab, parent, &element, &preview, progress.handles()).await
}

/// Neo: a page error of the owner lookup. A frame or owner that went away has been detached; a
/// closed tab closes the call.
fn detached(error: PageError) -> CallError {
    match error {
        PageError::Closed => PlaywrightError::target_closed().into(),
        PageError::NotConnected | PageError::Threw(_) => PlaywrightError::error(DETACHED).into(),
        PageError::Contract(message) => {
            PlaywrightError::error(format!("neo-runtime: {message}")).into()
        }
    }
}
