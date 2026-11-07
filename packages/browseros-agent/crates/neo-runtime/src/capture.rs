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

use std::fmt::Display;
use std::future::Future;
use std::sync::Arc;

use browseros_cdp::CdpError;
use neo_runtime_spec::serve::{
    PagePdfParams, PagePdfResult, PageScreenshotParams, PageScreenshotParamsAnimations,
    PageScreenshotParamsCaret, PageScreenshotParamsMaskItem, PageScreenshotParamsScale,
    PageScreenshotParamsType, PageScreenshotResult,
};
use neo_runtime_spec::{Binary, MethodMeta};
use serde::Deserialize;
use serde_json::Value;

use crate::artifacts::{self, ArtifactPath, Artifacts};
use crate::click::PageError;
use crate::dispatch::encode_result;
use crate::frames::FrameScope;
use crate::page::{FrameTab, PageSession};
use crate::pdf;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::screenshot::{self, Mask, ScreenshotOptions, ScreenshotParams};
use crate::tabs::{PageEntry, Tabs};
use crate::world;

/// `locator.screenshot`'s call, with Playwright's `ElementHandle.screenshot` title.
/// Neo: one call stands for Playwright's `waitForSelector` and `ElementHandle.screenshot` until
/// Neo has element handles; its record names the method `neo:screenshot`.
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
        .map_err(|error| protocol_error("Page.enable", &error))?;
    Ok(page)
}

/// Neo: runs a capture's background task (a restore, a reset, a print, a stream close) on the
/// run's tracker until it finishes or its tab closes or crashes. A tab that went has nothing left
/// to undo, and Chromium never answers a command in flight when its target goes (measured on
/// BrowserOS neo), so a task waiting for that answer would hold the tab's screenshot turn, and a
/// host's `Runtime::drain`, until browseros-cdp's request timeout. The watcher that sees the tab
/// go runs from [`session`] on; dropping `task` drops what it holds (its turn, its object group).
pub(crate) fn spawn_while_open(
    page: &PageSession,
    task: impl Future<Output = ()> + Send + 'static,
) {
    let closed = page.closed();
    page.spawn(async move {
        tokio::select! {
            biased;
            () = closed.cancelled() => {}
            () = task => {}
        }
    });
}

/// Playwright's wording for a failed CDP command; a closed tab is `TargetClosedError`.
/// Playwright: `server/protocolError.ts` `ProtocolError.setMessage`
pub(crate) fn protocol_error(method: &str, error: &CdpError) -> CallError {
    match error {
        CdpError::Protocol { message, .. } => {
            PlaywrightError::error(format!("Protocol error ({method}): {message}")).into()
        }
        error if world::classify(error) == PageError::Closed => {
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

#[cfg(test)]
mod tests {
    use std::future;
    use std::time::Duration;

    use tokio::runtime::Handle;
    use tokio::sync::{Mutex, oneshot};
    use tokio::time;
    use tokio_util::sync::CancellationToken;
    use tokio_util::task::TaskTracker;

    use super::*;
    use crate::events::EventHub;
    use crate::fake_browser::{FakeBrowser, PAGE, Replies};
    use crate::page::Session;
    use crate::run::PageId;

    async fn page_on(
        browser: &FakeBrowser,
        tasks: &TaskTracker,
        closed: &CancellationToken,
    ) -> Arc<PageSession> {
        let id = PageId::new(PAGE.to_string());
        let session = Session::attach(browser.cdp().await, id.clone(), Handle::current())
            .await
            .expect("attach");
        Arc::new(PageSession::new(
            session,
            &id,
            tasks.clone(),
            Arc::default(),
            Arc::new(Mutex::new(())),
            closed.clone(),
            EventHub::default(),
            false,
        ))
    }

    /// A cleanup waiting for an answer its gone tab will never send ends when the tab goes, and
    /// what it held (here a sender standing in for the tab's screenshot turn) goes with it, so
    /// the run's tracker drains.
    #[tokio::test]
    async fn a_background_task_ends_when_its_tab_goes() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let tasks = TaskTracker::new();
        let closed = CancellationToken::new();
        let page = page_on(&browser, &tasks, &closed).await;
        let (held, released) = oneshot::channel::<()>();
        spawn_while_open(&page, async move {
            future::pending::<()>().await;
            drop(held);
        });
        closed.cancel();
        assert!(
            time::timeout(Duration::from_secs(5), released)
                .await
                .is_ok(),
            "the task's hold went with the tab"
        );
        tasks.close();
        assert!(
            time::timeout(Duration::from_secs(5), tasks.wait())
                .await
                .is_ok()
        );
    }

    /// While the tab stays, the task runs to its end.
    #[tokio::test]
    async fn a_background_task_runs_to_its_end_while_its_tab_stays() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let tasks = TaskTracker::new();
        let closed = CancellationToken::new();
        let page = page_on(&browser, &tasks, &closed).await;
        let (done, finished) = oneshot::channel();
        spawn_while_open(&page, async move {
            time::sleep(Duration::from_millis(20)).await;
            let _sent = done.send("ran");
        });
        assert_eq!(
            time::timeout(Duration::from_secs(5), finished)
                .await
                .ok()
                .and_then(Result::ok),
            Some("ran")
        );
    }
}
