//! Neo: the calls that answer bytes, as dispatch hands them over: `Page.screenshot`,
//! `Page.pdf` and Neo's `neo:screenshot` frame command behind `locator.screenshot`. Each
//! checks its `path` before any page work, runs its port (`screenshot.rs`, `pdf.rs`), writes the
//! file into the run's artifact directory, and answers Playwright's `{binary}` / `{pdf}`.
//!
//! ```text
//!  JS page.screenshot({ path: 'a.png', mask: [loc] })
//!    {"guid":"page@T","method":"screenshot","params":{"type":"png","mask":[…],"path":"a.png"}}
//!    └─ method_router::route: PageCall::decode (Playwright's params; `path` is ignored there)
//!         + method_router::neo_path (Neo's `path` from the same params) → Capture::Page
//!  JS locator.screenshot()
//!    {"guid":"frame@T","method":"neo:screenshot","params":{"selector":"#a",…}} → Capture::Locator
//! ```
//!
//! Playwright's client writes `path` itself and its protocol has no such field, so the SDK sends
//! it next to Playwright's params and only this module reads it.

use std::fmt::Display;
use std::sync::Arc;

use browseros_cdp::CdpError;
use neo_runtime_spec::serve::{
    PagePdfParams, PagePdfResult, PageScreenshotParams, PageScreenshotParamsAnimations,
    PageScreenshotParamsCaret, PageScreenshotParamsMaskItem, PageScreenshotParamsScale,
    PageScreenshotParamsType, PageScreenshotResult,
};
use neo_runtime_spec::{Binary, MethodFlag, MethodMeta};
use serde::Deserialize;
use serde_json::Value;

use crate::artifacts::{self, ArtifactPath, Artifacts};
use crate::browser_representations::page::{FrameTab, PageSession};
use crate::browser_representations::tabs::{PageEntry, Tabs};
use crate::chromium_bridge::cdp::{self, Command};
use crate::chromium_bridge::frames::FrameScope;
use crate::chromium_bridge::playwright_injected_methods;
use crate::method_router::encode_result;
use crate::playwright_methods::pdf;
use crate::playwright_methods::screenshot::{self, Mask, ScreenshotOptions, ScreenshotParams};
use crate::playwright_methods::tab::PageError;
use crate::progress::{CallError, PlaywrightError, Progress};

/// `locator.screenshot`'s call, with Playwright's `ElementHandle.screenshot` title.
/// Neo: one call stands for Playwright's `waitForSelector` and `ElementHandle.screenshot`, under
/// one deadline and without minting a handle; its record names the method `neo:screenshot`.
pub(crate) static NEO_SCREENSHOT_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "neo:screenshot",
    title: Some("Screenshot"),
    subtitle: None,
    group: None,
    render_params: &["type"],
    internal: false,
    flags: &[],
};

/// Neo: `ElementHandle.screenshot`'s trace metadata, as `neo-runtime-spec/spec/playwright/handles.yml`
/// declares it. Handle calls are routed by hand (`evaluate.rs` `route_handle`), not by the
/// generated serve spec.
pub(crate) static ELEMENT_HANDLE_SCREENSHOT_META: MethodMeta = MethodMeta {
    interface: "ElementHandle",
    method: "screenshot",
    title: Some("Screenshot"),
    subtitle: None,
    group: None,
    render_params: &["type"],
    internal: false,
    flags: &[MethodFlag::Snapshot, MethodFlag::Pause],
};

/// The calls this module serves, routed by `method_router::route`.
#[derive(Debug)]
pub(crate) enum Capture {
    Page {
        params: Box<PageScreenshotParams>,
        path: Option<String>,
    },
    Pdf {
        params: Box<PagePdfParams>,
        path: Option<String>,
    },
    Locator(Box<NeoScreenshotParams>),
    /// `ElementHandle.screenshot` on the handle `guid`.
    Element {
        guid: String,
        params: Box<ElementScreenshotParams>,
    },
}

/// `neo:screenshot`: Playwright's `Locator.screenshot`, which its client serves as a strict
/// `waitForSelector` for an attached element and then `ElementHandle.screenshot` on it. One call
/// carries the selector beside `ElementHandle.screenshot`'s params and Neo's `path`
/// ([`ElementScreenshotParams`]). Keep this shape in sync with neo-quickjs-injected's
/// `NeoFrameCommands`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NeoScreenshotParams {
    pub(crate) selector: String,
    #[serde(flatten)]
    pub(crate) screenshot: ElementScreenshotParams,
}

/// `ElementHandle.screenshot`'s params (`spec/playwright/handles.yml`) and Neo's `path`: what
/// `elementHandle.screenshot` sends, and what `neo:screenshot` carries beside its selector. Keep
/// this shape in sync with neo-quickjs-injected's `ElementScreenshotParams`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ElementScreenshotParams {
    #[serde(rename = "type")]
    pub(crate) format: Option<PageScreenshotParamsType>,
    pub(crate) quality: Option<i64>,
    pub(crate) omit_background: Option<bool>,
    pub(crate) caret: Option<PageScreenshotParamsCaret>,
    pub(crate) animations: Option<PageScreenshotParamsAnimations>,
    pub(crate) scale: Option<PageScreenshotParamsScale>,
    pub(crate) mask: Option<Vec<PageScreenshotParamsMaskItem>>,
    pub(crate) mask_color: Option<String>,
    pub(crate) style: Option<String>,
    pub(crate) path: Option<String>,
}

/// Playwright: `server/screenshotter.ts` `Screenshotter.screenshotElement`
impl ElementScreenshotParams {
    /// The element capture's options: `ElementHandle.screenshot` has no `fullPage` or `clip`.
    fn options(&self, masks: Vec<Mask>) -> Result<ScreenshotOptions, PlaywrightError> {
        ScreenshotOptions::new(
            ScreenshotParams {
                format: self.format,
                quality: self.quality,
                full_page: None,
                clip: None,
                omit_background: self.omit_background,
                caret: self.caret,
                animations: self.animations,
                scale: self.scale,
                mask_color: self.mask_color.clone(),
                style: self.style.clone(),
            },
            masks,
        )
    }
}

/// Serves one capture call on the tab `entry`, from `scope` for a locator.
/// Neo: the dispatcher's arm for the three calls.
pub(crate) async fn serve(
    tabs: &Tabs,
    entry: &Arc<PageEntry>,
    scope: FrameScope,
    capture: Capture,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let artifacts = progress.artifacts();
    match capture {
        Capture::Page { params, path } => {
            let path = check_path(artifacts, path.as_deref())?;
            let masks = masks(tabs, entry, params.mask.as_deref());
            let options = ScreenshotOptions::new(ScreenshotParams::from_page(&params), masks)?;
            let tab = FrameTab {
                page: session(tabs, entry, progress).await?,
                scope: FrameScope::Main,
            };
            let bytes = screenshot::screenshot_page(&tab, &options, progress).await?;
            let binary = saved("screenshot", bytes, path.as_ref(), artifacts).await?;
            encode_result::<PageScreenshotParams>(&PageScreenshotResult { binary })
        }
        Capture::Pdf { params, path } => {
            let path = check_path(artifacts, path.as_deref())?;
            let page = session(tabs, entry, progress).await?;
            let bytes = pdf::pdf(&page, &params, progress).await?;
            let pdf = saved("pdf", bytes, path.as_ref(), artifacts).await?;
            encode_result::<PagePdfParams>(&PagePdfResult { pdf })
        }
        Capture::Locator(params) => {
            let path = check_path(artifacts, params.screenshot.path.as_deref())?;
            let masks = masks(tabs, entry, params.screenshot.mask.as_deref());
            let options = params.screenshot.options(masks)?;
            let tab = FrameTab {
                page: session(tabs, entry, progress).await?,
                scope,
            };
            let bytes =
                screenshot::screenshot_locator(&tab, &params.selector, &options, progress).await?;
            let binary = saved("screenshot", bytes, path.as_ref(), artifacts).await?;
            screenshot_result(binary)
        }
        Capture::Element { guid, params } => {
            let path = check_path(artifacts, params.path.as_deref())?;
            let masks = masks(tabs, entry, params.mask.as_deref());
            let options = params.options(masks)?;
            let handle = progress
                .handles()
                .element(&guid, ELEMENT_HANDLE_SCREENSHOT_META.method)?;
            let tab = FrameTab {
                page: session(tabs, entry, progress).await?,
                scope,
            };
            let bytes = screenshot::screenshot_handle(&tab, &handle, &options, progress).await?;
            let binary = saved("screenshot", bytes, path.as_ref(), artifacts).await?;
            screenshot_result(binary)
        }
    }
}

/// An element capture's `{binary}` answer.
fn screenshot_result(binary: Binary) -> Result<Option<Value>, CallError> {
    serde_json::to_value(PageScreenshotResult { binary })
        .map(Some)
        .map_err(|error| PlaywrightError::error(error.to_string()).into())
}

/// `path` checked against the run's artifact directory before any page work.
/// Playwright: `client/page.ts` `Page.screenshot`: `if (options.path) {`
fn check_path(
    artifacts: &Artifacts,
    path: Option<&str>,
) -> Result<Option<ArtifactPath>, PlaywrightError> {
    path.filter(|path| !path.is_empty())
        .map(|path| artifacts.check("path", path))
        .transpose()
}

/// The bytes written to `path`, then as the call's `binary` result if they fit the realm.
/// Playwright: `client/page.ts` `Page.screenshot`: `await fs.promises.writeFile(options.path, result.binary);`
async fn saved(
    what: &str,
    bytes: Vec<u8>,
    path: Option<&ArtifactPath>,
    artifacts: &Artifacts,
) -> Result<Binary, CallError> {
    if let Some(path) = path {
        artifacts.write(path, &bytes).await?;
    }
    Ok(artifacts::returnable(what, bytes, path)?)
}

/// The mask locators on this tab.
/// Deviation: `server/screenshotter.ts` `Screenshotter._maskElements`: `frame.selectors.callOnSelector({ selector, strict: false }` masks a locator of another page in that page, where nothing hides the mask again; Neo skips it, which leaves this page's screenshot the same.
fn masks(
    tabs: &Tabs,
    entry: &PageEntry,
    mask: Option<&[PageScreenshotParamsMaskItem]>,
) -> Vec<Mask> {
    mask.unwrap_or_default()
        .iter()
        .filter_map(|item| {
            let (page, scope) = tabs.resolve(&item.frame.guid)?;
            (page.id == entry.id).then(|| Mask {
                scope,
                selector: item.selector.clone(),
            })
        })
        .collect()
}

/// The tab's session for a capture, with its watcher running (`PageSession::prepare`). Only the
/// watcher tells a call that its tab closed or crashed, and a capture may wait for the tab's
/// screenshot turn behind another run's cleanup (`screenshot.rs` `TabScreenshots`): that wait
/// races the call's `Progress`, whose closed token the watcher cancels, so it ends with
/// `TargetClosedError` when the tab goes, even on a session nothing else of this run prepared.
/// Neo: the call's own lazy attach, as dispatch's frame calls do, watched before it waits.
async fn session(
    tabs: &Tabs,
    entry: &PageEntry,
    progress: &Progress,
) -> Result<Arc<PageSession>, CallError> {
    let page = match entry.attached_session() {
        Some(session) => session,
        None => progress.race(tabs.session(entry)).await??,
    };
    progress
        .race(page.prepare())
        .await?
        .map_err(|error| protocol_error(cdp::page::Enable::METHOD, &error))?;
    Ok(page)
}

/// Playwright's wording for a failed CDP command; a closed tab is `TargetClosedError`, and an
/// answer the door could not decode as the protocol declares it is [`malformed`].
/// Playwright: `server/protocolError.ts` `ProtocolError.setMessage`
pub(crate) fn protocol_error(method: &str, error: &CdpError) -> CallError {
    match error {
        CdpError::Protocol { message, .. } => {
            PlaywrightError::error(format!("Protocol error ({method}): {message}")).into()
        }
        CdpError::Json(message) => malformed(method, message),
        error if playwright_injected_methods::classify(error) == PageError::Closed => {
            PlaywrightError::target_closed().into()
        }
        error => PlaywrightError::error(error.to_string()).into(),
    }
}

/// An answer outside CDP's documented shape: a broken browser or transport, never a page.
/// Neo: Playwright's typed CDP client would reject such an answer on its own terms.
pub(crate) fn malformed(method: &str, error: impl Display) -> CallError {
    PlaywrightError::error(format!("{method} answered: {error}")).into()
}
