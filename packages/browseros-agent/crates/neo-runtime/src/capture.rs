//! Neo: the calls that answer bytes, as dispatch hands them over: `Page.screenshot`,
//! `Page.pdf` and Neo's `neo:screenshot` frame command behind `locator.screenshot`. Each
//! checks its `path` before any page work, runs its port (`screenshot.rs`, `pdf.rs`), writes the
//! file into the run's artifact directory, and answers Playwright's `{binary}` / `{pdf}`.
//!
//! ```text
//!  JS page.screenshot({ path: 'a.png', mask: [loc] })
//!    {"guid":"page@T","method":"screenshot","params":{"type":"png","mask":[…],"path":"a.png"}}
//!    └─ dispatch::route: PageCall::decode (Playwright's params; `path` is ignored there)
//!         + dispatch::neo_path (Neo's `path` from the same params) → Capture::Page
//!  JS locator.screenshot()
//!    {"guid":"frame@T","method":"neo:screenshot","params":{"selector":"#a",…}} → Capture::Locator
//! ```
//!
//! Playwright's client writes `path` itself and its protocol has no such field, so the SDK sends
//! it next to Playwright's params and only this module reads it.

use std::sync::Arc;

use neo_runtime_spec::serve::{
    PagePdfParams, PagePdfResult, PageScreenshotParams, PageScreenshotParamsAnimations,
    PageScreenshotParamsCaret, PageScreenshotParamsMaskItem, PageScreenshotParamsScale,
    PageScreenshotParamsType, PageScreenshotResult,
};
use neo_runtime_spec::{Binary, MethodMeta};
use serde::Deserialize;
use serde_json::Value;

use crate::artifacts::{self, ArtifactPath, Artifacts};
use crate::dispatch::encode_result;
use crate::frames::FrameScope;
use crate::page::{FrameTab, PageSession};
use crate::pdf;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::screenshot::{self, Mask, ScreenshotOptions, ScreenshotParams};
use crate::tabs::{PageEntry, Tabs};

/// `locator.screenshot`'s call. Playwright's `ElementHandle.screenshot` title and flags.
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

/// The calls this module serves, routed by `dispatch::route`.
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
}

/// `neo:screenshot`: Playwright's `Locator.screenshot`, which its client serves as a strict
/// `waitForSelector` for an attached element and then `ElementHandle.screenshot` on it. Neo has
/// no element handles yet, so one call carries the selector and `ElementHandle.screenshot`'s
/// params (`spec/playwright/handles.yml`) plus Neo's `path`. Keep this shape in sync with
/// neo-sdk's `NeoFrameCommands`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NeoScreenshotParams {
    pub(crate) selector: String,
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

/// Serves one capture call on the tab `entry`, from `scope` for a locator.
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
            let path = check_path(artifacts, params.path.as_deref())?;
            let masks = masks(tabs, entry, params.mask.as_deref());
            let options = ScreenshotOptions::new(
                ScreenshotParams {
                    format: params.format,
                    quality: params.quality,
                    full_page: None,
                    clip: None,
                    omit_background: params.omit_background,
                    caret: params.caret,
                    animations: params.animations,
                    scale: params.scale,
                    mask_color: params.mask_color.clone(),
                    style: params.style.clone(),
                },
                masks,
            )?;
            let tab = FrameTab {
                page: session(tabs, entry, progress).await?,
                scope,
            };
            let bytes =
                screenshot::screenshot_locator(&tab, &params.selector, &options, progress).await?;
            let binary = saved("screenshot", bytes, path.as_ref(), artifacts).await?;
            serde_json::to_value(PageScreenshotResult { binary })
                .map(Some)
                .map_err(|error| PlaywrightError::error(error.to_string()).into())
        }
    }
}

/// `path` checked against the run's artifact directory. Playwright's `if (options.path)`: an
/// empty path writes nothing.
fn check_path(
    artifacts: &Artifacts,
    path: Option<&str>,
) -> Result<Option<ArtifactPath>, PlaywrightError> {
    path.filter(|path| !path.is_empty())
        .map(|path| artifacts.check("path", path))
        .transpose()
}

/// The bytes written to `path`, then as the call's `binary` result if they fit the realm.
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

/// The mask locators on this tab. Deviation: `server/screenshotter.ts`
/// `Screenshotter._maskElements`: `frame.selectors.callOnSelector({ selector, strict: false }` masks
/// a locator of another page in that page, where nothing hides it again; Neo skips it, which
/// leaves this page's screenshot the same.
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

/// The tab's session, attached once per run.
async fn session(
    tabs: &Tabs,
    entry: &PageEntry,
    progress: &Progress,
) -> Result<Arc<PageSession>, CallError> {
    if let Some(session) = entry.attached_session() {
        return Ok(session);
    }
    Ok(progress.race(tabs.session(entry)).await??)
}
