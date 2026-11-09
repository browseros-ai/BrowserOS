//! Playwright: `server/screenshotter.ts` `Screenshotter`
//! Playwright: `server/chromium/crPage.ts` `CRPage.takeScreenshot`
//! Playwright: `server/dom.ts` `ElementHandle._waitAndScrollIntoViewIfNeeded`
//!
//! `Page.screenshot` and the element screenshot behind `locator.screenshot`, over one tab's
//! session. The page-side steps are typed glue calls in each frame's utility world
//! (`spec/injected.yml`), as Playwright runs them in `'utility'`; no page code runs in the main
//! world and no `Runtime.*` method is sent from here.
//!
//! ```text
//!  screenshot_page / screenshot_element      one at a time per tab, across runs (TabScreenshots)
//!    viewportSize (main frame)
//!    Prepared: prepareForScreenshot in every frame, fontsReady       → restored on every exit
//!    [element] scroll-into-view loop, DOM.getBoxModel, scrollOffset
//!    capture: setDefaultBackgroundColorOverride (omitBackground)      → reset on every exit
//!             masks: hideHighlight everywhere, maskElements            → hidden on every exit
//!             Page.getLayoutMetrics, Page.captureScreenshot {clip, captureBeyondViewport}
//! ```
//!
//! Cleanup runs on the run's tracker whatever ends the call: a cancelled or timed-out call
//! still restores the page, removes its masks and resets the background (Playwright sends the
//! same steps without waiting for them when a screenshot fails).

use std::collections::HashMap;
use std::mem;
use std::sync::{Arc, Weak};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use browseros_cdp::CdpError;
use futures_util::future;
use neo_runtime_spec::Handle;
use neo_runtime_spec::injected::{
    ElementState, InjectedFontsReadyParams, InjectedFullPageSizeParams,
    InjectedHideHighlightParams, InjectedMaskElementsParams, InjectedPrepareForScreenshotParams,
    InjectedRestoreAfterScreenshotParams, InjectedScrollOffsetParams, InjectedViewportSizeParams,
    ScreenshotCleanup,
};
use neo_runtime_spec::serve::{
    FrameWaitForSelectorParamsState as SelectorState, PageScreenshotParams,
    PageScreenshotParamsAnimations, PageScreenshotParamsCaret, PageScreenshotParamsScale,
    PageScreenshotParamsType, Rect,
};
use tokio::sync::{Mutex, OwnedMutexGuard, oneshot};
use tracing::debug;

use crate::browser_representations::page::{FrameTab, GroupGuard, PageSession, ResolvedElement};
use crate::chromium_bridge::cdp::{self, Command};
use crate::chromium_bridge::frame_selector;
use crate::chromium_bridge::frames::{FrameContext, FrameScope};
use crate::playwright_methods::capture;
use crate::playwright_methods::click::{self, NotReady, PageError, Scroll, StatesCheck, Tab};
use crate::playwright_methods::evaluate::HandleEntry;
use crate::playwright_methods::waits::{self, WaitForSelectorOptions};
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::run::PageId;

/// Playwright: `server/screenshotter.ts` `Screenshotter._maskElements`: `options.maskColor || '#F0F'`
const DEFAULT_MASK_COLOR: &str = "#F0F";
/// Playwright: `server/screenshotter.ts` `Screenshotter._screenshot`: `options.quality ?? 80`
const DEFAULT_JPEG_QUALITY: i64 = 80;
/// Playwright: `server/screenshotter.ts` `Screenshotter._screenshot`: `options.quality ?? 100`
const DEFAULT_WEBP_QUALITY: i64 = 100;
/// Playwright: `server/helper.ts` `Helper.enclosingIntRect`: `const x = Math.floor(rect.x + 1e-3);`
const ENCLOSING_EPSILON: f64 = 1e-3;
/// Playwright: `server/screenshotter.ts` `validateScreenshotOptions`: `'options.quality is unsupported for the ' + format + ' screenshots'`
const QUALITY_UNSUPPORTED: [&str; 2] = ["options.quality is unsupported for the ", " screenshots"];
/// Playwright: `server/screenshotter.ts` `validateScreenshotOptions`: `'Expected options.quality to be between 0 and 100 (inclusive), got ' + options.quality`
const QUALITY_RANGE: &str = "Expected options.quality to be between 0 and 100 (inclusive), got ";

/// The image formats `Page.captureScreenshot` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Png,
    Jpeg,
    Webp,
}

/// Neo: the names `Page.captureScreenshot` takes, which are Playwright's.
impl Format {
    fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Webp => "webp",
        }
    }
}

/// One `mask` locator: its frame on this tab and its selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mask {
    pub(crate) scope: FrameScope,
    pub(crate) selector: String,
}

/// Playwright's server-side `ScreenshotOptions`, validated.
#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Playwright's boolean screenshot options, one field each"
)]
pub(crate) struct ScreenshotOptions {
    format: Format,
    quality: Option<i64>,
    full_page: bool,
    clip: Option<Rect>,
    omit_background: bool,
    hide_caret: bool,
    disable_animations: bool,
    css_scale: bool,
    masks: Vec<Mask>,
    mask_color: String,
    style: Option<String>,
}

/// The screenshot options `Page.screenshot` and `ElementHandle.screenshot` share, as the
/// protocol carries them; `full_page` and `clip` are the page's only.
pub(crate) struct ScreenshotParams {
    pub(crate) format: Option<PageScreenshotParamsType>,
    pub(crate) quality: Option<i64>,
    pub(crate) full_page: Option<bool>,
    pub(crate) clip: Option<Rect>,
    pub(crate) omit_background: Option<bool>,
    pub(crate) caret: Option<PageScreenshotParamsCaret>,
    pub(crate) animations: Option<PageScreenshotParamsAnimations>,
    pub(crate) scale: Option<PageScreenshotParamsScale>,
    pub(crate) mask_color: Option<String>,
    pub(crate) style: Option<String>,
}

/// Neo: the page's params as the shared shape.
impl ScreenshotParams {
    pub(crate) fn from_page(params: &PageScreenshotParams) -> Self {
        Self {
            format: params.r#type,
            quality: params.quality,
            full_page: params.full_page,
            clip: params.clip.clone(),
            omit_background: params.omit_background,
            caret: params.caret,
            animations: params.animations,
            scale: params.scale,
            mask_color: params.mask_color.clone(),
            style: params.style.clone(),
        }
    }
}

/// Playwright: `server/screenshotter.ts` `validateScreenshotOptions`
impl ScreenshotOptions {
    /// Playwright's `validateScreenshotOptions`, with `masks` already resolved to this tab's
    /// frames.
    pub(crate) fn new(params: ScreenshotParams, masks: Vec<Mask>) -> Result<Self, PlaywrightError> {
        let format = match params.format {
            None | Some(PageScreenshotParamsType::Png) => Format::Png,
            Some(PageScreenshotParamsType::Jpeg) => Format::Jpeg,
            Some(PageScreenshotParamsType::Webp) => Format::Webp,
        };
        if let Some(quality) = params.quality {
            if format == Format::Png {
                let [before, after] = QUALITY_UNSUPPORTED;
                return Err(PlaywrightError::error(format!(
                    "{before}{}{after}",
                    format.as_str()
                )));
            }
            if !(0..=100).contains(&quality) {
                return Err(PlaywrightError::error(format!("{QUALITY_RANGE}{quality}")));
            }
        }
        if let Some(clip) = &params.clip {
            if clip.width <= 0.0 {
                // Playwright: `server/screenshotter.ts` `validateScreenshotOptions`: `'Expected options.clip.width to be greater than 0.'`
                return Err(PlaywrightError::error(
                    "Expected options.clip.width to be greater than 0.",
                ));
            }
            if clip.height <= 0.0 {
                // Playwright: `server/screenshotter.ts` `validateScreenshotOptions`: `'Expected options.clip.height to be greater than 0.'`
                return Err(PlaywrightError::error(
                    "Expected options.clip.height to be greater than 0.",
                ));
            }
        }
        Ok(Self {
            format,
            quality: params.quality,
            full_page: params.full_page.unwrap_or(false),
            clip: params.clip,
            omit_background: params.omit_background.unwrap_or(false),
            hide_caret: params.caret != Some(PageScreenshotParamsCaret::Initial),
            disable_animations: params.animations == Some(PageScreenshotParamsAnimations::Disabled),
            css_scale: params.scale == Some(PageScreenshotParamsScale::Css),
            masks,
            mask_color: params
                .mask_color
                .filter(|color| !color.is_empty())
                .unwrap_or_else(|| DEFAULT_MASK_COLOR.to_string()),
            style: params.style,
        })
    }
}

/// A screenshot's turn at its tab ([`PageSession::screenshots`]), shared by the call and every
/// cleanup task it started: the next screenshot on the tab starts only once the last of them
/// has undone its part of the page, so a call that stopped early never leaks its style, caret,
/// masks or transparent background into the next capture.
type Turn = Arc<OwnedMutexGuard<()>>;

/// One `Runtime`'s screenshot turns, one per tab, each shared by every run the `Runtime` makes on
/// that tab, as [`TabInputs`](crate::browser_representations::page::TabInputs) shares the tab's input. Every Neo run on a
/// tab captures the same page, so they share its queue: a run a host cancels or drops mid-capture
/// leaves cleanup tasks holding the turn (the background reset, the page restore), and the next
/// run's capture waits until they have run, instead of capturing over a transparent background
/// or a prepared page.
///
/// Entries are weak: a tab's turn lives while a run on the tab, or a cleanup still holding it,
/// does. Runs of another `Runtime`, and other CDP clients, are not coordinated.
#[derive(Clone, Default)]
pub(crate) struct TabScreenshots(Arc<Mutex<HashMap<PageId, Weak<Mutex<()>>>>>);

/// Playwright: `server/screenshotter.ts` `Screenshotter`: `private _queue = new TaskQueue();`
impl TabScreenshots {
    /// The screenshot turn of `page`, the one every live run on it shares.
    pub(crate) async fn turn_for(&self, page: &PageId) -> Arc<Mutex<()>> {
        let mut turns = self.0.lock().await;
        if let Some(turn) = turns.get(page).and_then(Weak::upgrade) {
            return turn;
        }
        // Tabs whose runs and cleanups are all gone go too, so the map holds only live tabs.
        turns.retain(|_, turn| turn.strong_count() > 0);
        let turn = Arc::default();
        turns.insert(page.clone(), Arc::downgrade(&turn));
        turn
    }
}

/// Waits for a screenshot's turn at its tab.
/// Neo: Playwright's `TaskQueue` per page, held past the call by its cleanup.
async fn turn(tab: &FrameTab, progress: &Progress) -> Result<Turn, CallError> {
    let guard = progress
        .race(Arc::clone(&tab.screenshots).lock_owned())
        .await?;
    Ok(Arc::new(guard))
}

/// A width and height in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Size {
    width: f64,
    height: f64,
}

/// Serves `Page.screenshot` on the tab's main frame: the image bytes.
/// Playwright: `server/screenshotter.ts` `Screenshotter.screenshotPage`
pub(crate) async fn screenshot_page(
    tab: &FrameTab,
    options: &ScreenshotOptions,
    progress: &Progress,
) -> Result<Vec<u8>, CallError> {
    let turn = turn(tab, progress).await?;
    progress.log("taking page screenshot");
    let viewport = original_viewport_size(tab, progress).await?;
    let main = tab.frames.main();
    let prepared = prepare(tab, &main, options, &turn, progress).await?;
    let result = async {
        if options.full_page {
            let full = full_page_size(tab, progress).await?;
            let fits_viewport = full.width <= viewport.width && full.height <= viewport.height;
            let mut document_rect = Rect {
                x: 0.0,
                y: 0.0,
                width: full.width,
                height: full.height,
            };
            if let Some(clip) = &options.clip {
                document_rect = trim_clip_to_size(clip, full)?;
            }
            return capture(
                tab,
                Some(document_rect),
                None,
                fits_viewport,
                options,
                &turn,
                progress,
            )
            .await;
        }
        let viewport_rect = match &options.clip {
            Some(clip) => trim_clip_to_size(clip, viewport)?,
            None => Rect {
                x: 0.0,
                y: 0.0,
                width: viewport.width,
                height: viewport.height,
            },
        };
        capture(
            tab,
            None,
            Some(viewport_rect),
            true,
            options,
            &turn,
            progress,
        )
        .await
    }
    .await;
    prepared.restore(progress).await;
    result
}

/// Serves Neo's `locator.screenshot`: Playwright's `Locator._withElement` (one strict
/// `waitForSelector` for an attached element) then `Screenshotter.screenshotElement`, in one call
/// with one deadline.
/// Playwright: `client/locator.ts` `Locator._withElement`
pub(crate) async fn screenshot_locator(
    tab: &FrameTab,
    selector: &str,
    options: &ScreenshotOptions,
    progress: &Progress,
) -> Result<Vec<u8>, CallError> {
    // Playwright: `client/locator.ts` `Locator._withElement`: `this._frame._channel.waitForSelector({ selector: this._selector, strict: true, state: 'attached' }`
    let attached = WaitForSelectorOptions {
        selector: selector.to_string(),
        strict: true,
        state: SelectorState::Attached,
        // The element is resolved again below; no main-world handle is minted for it.
        return_element: false,
        // Playwright's dispatcher passes `true` for the client's `waitForSelector`, so the
        // locator handlers run first, as for every served wait.
        pre_checks: true,
    };
    let element = loop {
        waits::wait_for_selector_element(tab, &attached, progress).await?;
        // Deviation: `server/frames.ts` `Frame.waitForSelector`: `const element: Element | undefined  = elements[0];` is the element Playwright's wait returns; Neo resolves it again with click's `resolve`, which also marks it as a target (`markTargets: 'all'`), and waits again if it detached in between.
        match progress.race(tab.resolve(selector, true)).await? {
            Ok(Some(resolved)) => break resolved.element,
            // Gone, or its document replaced, since the wait: wait again, as the wait would.
            Ok(None) | Err(PageError::NotConnected) => {}
            Err(error) => return Err(page_error(error)),
        }
    };
    screenshot_element(tab, element, options, progress).await
}

/// Serves `ElementHandle.screenshot`: `Screenshotter.screenshotElement` on the handle's own
/// node, never a selector resolved again. Its steps run in the frame's utility world, where
/// Playwright's `ElementHandle.evaluateInUtility` adopts the node; a node gone with its document
/// is not attached, as there.
/// Playwright: `server/dom.ts` `ElementHandle.screenshot`
pub(crate) async fn screenshot_handle(
    tab: &FrameTab,
    handle: &HandleEntry,
    options: &ScreenshotOptions,
    progress: &Progress,
) -> Result<Vec<u8>, CallError> {
    let frame = Arc::clone(handle.frame());
    let group = tab.group(&frame, "screenshot");
    let element = progress
        .race(handle.utility_element(&group))
        .await?
        .map_err(element_error)?;
    // The iframe owners above the handle's frame, as a selector's resolution keeps them
    // (`frame_selector::locate`): the stability step scrolls an offscreen OOPIF's owners first.
    let owners = if matches!(tab.scope, FrameScope::Frame(_)) {
        progress
            .race(frame_selector::direct_owners(tab, &frame.id))
            .await?
            .map_err(element_error)?
    } else {
        Vec::new()
    };
    let element = ResolvedElement::adopted(element, Arc::new(group), frame, owners);
    screenshot_element(tab, element, options, progress).await
}

/// Playwright's `Screenshotter.screenshotElement` with `frameVisible: true`, as
/// `ElementHandle.screenshot` passes it.
/// Playwright: `server/screenshotter.ts` `Screenshotter.screenshotElement`
async fn screenshot_element(
    tab: &FrameTab,
    mut element: ResolvedElement,
    options: &ScreenshotOptions,
    progress: &Progress,
) -> Result<Vec<u8>, CallError> {
    // Playwright: `server/dom.ts` `ElementHandle.screenshot`: `screenshotElement(progress, this, options, true /* frameVisible */)`
    element.frame_visible = true;
    let turn = turn(tab, progress).await?;
    progress.log("taking element screenshot");
    let viewport = original_viewport_size(tab, progress).await?;
    let prepared = prepare(tab, &element.frame, options, &turn, progress).await?;
    let result = async {
        wait_and_scroll_into_view(tab, &element, progress).await?;
        let bounding_box = progress
            .race(bounding_box(tab, &element))
            .await?
            .map_err(page_error)?;
        // Playwright: `server/screenshotter.ts` `Screenshotter.screenshotElement`: `assert(boundingBox, 'Node is either not visible or not an HTMLElement');`
        let Some(bounding_box) = bounding_box else {
            return Err(
                PlaywrightError::error("Node is either not visible or not an HTMLElement").into(),
            );
        };
        if bounding_box.width == 0.0 {
            // Playwright: `server/screenshotter.ts` `Screenshotter.screenshotElement`: `assert(boundingBox.width !== 0, 'Node has 0 width.');`
            return Err(PlaywrightError::error("Node has 0 width.").into());
        }
        if bounding_box.height == 0.0 {
            // Playwright: `server/screenshotter.ts` `Screenshotter.screenshotElement`: `assert(boundingBox.height !== 0, 'Node has 0 height.');`
            return Err(PlaywrightError::error("Node has 0 height.").into());
        }
        let fits_viewport =
            bounding_box.width <= viewport.width && bounding_box.height <= viewport.height;
        let scroll = progress
            .race(tab.frames.main().call(None, &InjectedScrollOffsetParams {}))
            .await?
            .map_err(page_error)?;
        let document_rect = enclosing_int_rect(&Rect {
            x: bounding_box.x + scroll.x,
            y: bounding_box.y + scroll.y,
            width: bounding_box.width,
            height: bounding_box.height,
        });
        capture(
            tab,
            Some(document_rect),
            None,
            fits_viewport,
            options,
            &turn,
            progress,
        )
        .await
    }
    .await;
    prepared.restore(progress).await;
    result
}

/// `Screenshotter._originalViewportSize` without an emulated viewport (Neo never emulates
/// one): the main frame's `innerWidth` and `innerHeight`.
/// Playwright: `server/screenshotter.ts` `Screenshotter._originalViewportSize`
async fn original_viewport_size(tab: &FrameTab, progress: &Progress) -> Result<Size, CallError> {
    let size = progress
        .race(tab.frames.main().call(None, &InjectedViewportSizeParams {}))
        .await?
        .map_err(page_error)?;
    Ok(Size {
        width: pixels(size.width),
        height: pixels(size.height),
    })
}

/// `Screenshotter._fullPageSize`: Playwright's `waitForFunctionValueInUtility` polls until the
/// document has a `body` and a `documentElement`; so does this.
/// Playwright: `server/screenshotter.ts` `Screenshotter._fullPageSize`
/// Neo: it polls on loop A's backoff rather than on animation frames.
async fn full_page_size(tab: &FrameTab, progress: &Progress) -> Result<Size, CallError> {
    let mut backoff = click::LoopAWaits::new(progress.timeout_ms());
    loop {
        progress.wait(backoff.next_wait()).await?;
        let read = progress
            .race(tab.frames.main().call(None, &InjectedFullPageSizeParams {}))
            .await?;
        match read {
            Ok(read) => {
                if let Some(size) = read.size {
                    return Ok(Size {
                        width: pixels(size.width),
                        height: pixels(size.height),
                    });
                }
            }
            // Playwright's poll survives a navigation; so does this one.
            Err(PageError::NotConnected) => {}
            Err(error) => return Err(page_error(error)),
        }
    }
}

/// Playwright's `_retryAction(progress, 'scroll into view', …)`: wait until the element is
/// visible and stable, then `DOM.scrollIntoViewIfNeeded`, retrying with loop B's waits.
/// Playwright: `server/dom.ts` `ElementHandle._retryAction`
/// Playwright: `server/dom.ts` `ElementHandle._waitAndScrollIntoViewIfNeeded`
async fn wait_and_scroll_into_view(
    tab: &FrameTab,
    element: &ResolvedElement,
    progress: &Progress,
) -> Result<(), CallError> {
    let mut retry = 0;
    loop {
        if retry == 0 {
            progress.log("attempting scroll into view action");
        } else {
            progress.log("retrying scroll into view action");
            let wait = click::loop_b_wait(retry);
            if !wait.is_zero() {
                progress.log(format!("  waiting {}ms", wait.as_millis()));
                progress.wait(wait).await?;
            }
        }
        // Playwright: `server/page.ts` `Page._performWaitForNavigationCheck`: `if (!mainFrame || !mainFrame.pendingDocument())`
        if let Some(url) = Tab::pending_navigation(tab) {
            progress.log(format!("  waiting for \"{url}\" navigation to finish..."));
            progress
                .race(Tab::navigation_settled(tab))
                .await?
                .map_err(page_error)?;
        }
        retry += 1;
        progress.log("  waiting for element to be stable");
        let states = progress
            // A screenshot always scrolls into view (no `scroll: 'none'`), frame owners first.
            .race(tab.check_states(
                element,
                &[ElementState::Visible, ElementState::Stable],
                true,
            ))
            .await?;
        let not_ready = match states {
            Ok(StatesCheck::Ready) => {
                let scrolled = progress
                    .race(tab.scroll(element, Scroll::Protocol, None))
                    .await?;
                match scrolled {
                    Ok(None) => return Ok(()),
                    Ok(Some(reason)) => reason,
                    Err(error) => return Err(element_error(error)),
                }
            }
            Ok(StatesCheck::Missing(state)) => NotReady::Missing(state),
            Ok(StatesCheck::NotConnected) => NotReady::NotConnected,
            Err(error) => return Err(element_error(error)),
        };
        match click::not_ready_line(&not_ready) {
            Some(line) => progress.log(line),
            // Playwright: `server/dom.ts` `throwElementIsNotAttached`: `throw new Error('Element is not attached to the DOM');`
            None => return Err(PlaywrightError::error("Element is not attached to the DOM").into()),
        }
    }
}

/// `CRPage._getBoundingBox`: the border box from `DOM.getBoxModel`, moved by the offset of the
/// element's out-of-process frame. `None` where Playwright's `_sendMayFail` gives `null`.
/// Playwright: `server/chromium/crPage.ts` `FrameSession._getBoundingBox`
async fn bounding_box(
    tab: &FrameTab,
    element: &ResolvedElement,
) -> Result<Option<Rect>, PageError> {
    let model = element
        .frame
        .host
        .send_command(cdp::dom::GetBoxModel {
            object_id: Some(element.handle().id().object_id().to_string()),
            ..cdp::dom::GetBoxModel::default()
        })
        .await;
    let model = match model {
        Ok(model) => model,
        Err(error) => {
            return match element.frame.host.classify(&error) {
                PageError::Closed => Err(PageError::Closed),
                _ => Ok(None),
            };
        }
    };
    let values = model.model.border;
    if values.len() != 8 {
        return Ok(None);
    }
    let xs = [values[0], values[2], values[4], values[6]];
    let ys = [values[1], values[3], values[5], values[7]];
    let x = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let y = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let width = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max) - x;
    let height = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max) - y;
    let offset = tab
        .frames
        .offset(&element.frame.host)
        .await?
        .unwrap_or_default();
    Ok(Some(Rect {
        x: x + offset.0,
        y: y + offset.1,
        width,
        height,
    }))
}

/// `Screenshotter._screenshot`: the background override, the masks, the capture, and their
/// cleanup in Playwright's order.
/// Playwright: `server/screenshotter.ts` `Screenshotter._screenshot`
async fn capture(
    tab: &FrameTab,
    document_rect: Option<Rect>,
    viewport_rect: Option<Rect>,
    fits_viewport: bool,
    options: &ScreenshotOptions,
    turn: &Turn,
    progress: &Progress,
) -> Result<Vec<u8>, CallError> {
    // Playwright: `server/screenshotter.ts` `Screenshotter._screenshot`: `const shouldSetDefaultBackground = options.omitBackground && format !== 'jpeg';`
    let background = if options.omit_background && options.format != Format::Jpeg {
        // Armed before the override is sent: from here on, whatever ends the call resets it.
        let (background, answered) = Background::set(&tab.page, Arc::clone(turn));
        progress
            .race(answered)
            .await?
            .map_err(|_| PlaywrightError::target_closed())?
            .map_err(|error| {
                capture::protocol_error(
                    cdp::emulation::SetDefaultBackgroundColorOverride::METHOD,
                    &error,
                )
            })?;
        Some(background)
    } else {
        None
    };
    let masks = mask_elements(tab, options, turn, progress).await?;
    let quality = match options.format {
        Format::Jpeg => Some(options.quality.unwrap_or(DEFAULT_JPEG_QUALITY)),
        Format::Webp => Some(options.quality.unwrap_or(DEFAULT_WEBP_QUALITY)),
        Format::Png => None,
    };
    let bytes = take_screenshot(
        &tab.page,
        options.format,
        document_rect,
        viewport_rect,
        quality,
        fits_viewport,
        options.css_scale,
        progress,
    )
    .await?;
    if let Some(masks) = masks {
        masks.hide(progress).await;
    }
    if let Some(background) = background {
        background.reset(progress).await;
    }
    Ok(bytes)
}

/// `CRPage.takeScreenshot`: the clip in document coordinates, scaled for `scale: 'css'`, and
/// `captureBeyondViewport` when the area does not fit the viewport. Neo never emulates a
/// device scale or touch, so the device scale is the content's ratio and no touch emulation
/// needs restoring after a capture beyond the viewport.
/// Playwright: `server/chromium/crPage.ts` `CRPage.takeScreenshot`
#[expect(
    clippy::too_many_arguments,
    reason = "CRPage.takeScreenshot's own parameters, in its order"
)]
async fn take_screenshot(
    page: &PageSession,
    format: Format,
    document_rect: Option<Rect>,
    viewport_rect: Option<Rect>,
    quality: Option<i64>,
    fits_viewport: bool,
    css_scale: bool,
    progress: &Progress,
) -> Result<Vec<u8>, CallError> {
    // The deprecated device-pixel fields of the answer are the ones Playwright reads.
    let metrics = progress
        .race(page.send_command(cdp::page::GetLayoutMetrics {}))
        .await?
        .map_err(|error| capture::protocol_error(cdp::page::GetLayoutMetrics::METHOD, &error))?;
    let (visual_viewport, content_size, css_content_size) = (
        metrics.visual_viewport,
        metrics.content_size,
        metrics.css_content_size,
    );
    let document_rect = match (document_rect, &viewport_rect) {
        (Some(rect), _) => rect,
        (None, Some(viewport)) => Rect {
            x: visual_viewport.page_x + viewport.x,
            y: visual_viewport.page_y + viewport.y,
            width: enclosing_int(viewport.width / visual_viewport.scale),
            height: enclosing_int(viewport.height / visual_viewport.scale),
        },
        (None, None) => {
            return Err(capture::malformed(
                "screenshot",
                "no document or viewport area",
            ));
        }
    };
    // When taking screenshots with documentRect (based on the page content, not viewport),
    // ignore current page scale.
    let mut scale = if viewport_rect.is_some() {
        visual_viewport.scale
    } else {
        1.0
    };
    if css_scale {
        // Playwright: `server/chromium/crPage.ts` `CRPage.takeScreenshot`: `contentSize.width / cssContentSize.width || 1`
        let ratio = content_size.width / css_content_size.width;
        let device_scale_factor = if ratio.is_nan() || ratio == 0.0 {
            1.0
        } else {
            ratio
        };
        scale /= device_scale_factor;
    }
    let params = cdp::page::CaptureScreenshot {
        format: Some(format.as_str().to_string()),
        quality,
        clip: Some(cdp::page::Viewport {
            x: document_rect.x,
            y: document_rect.y,
            width: document_rect.width,
            height: document_rect.height,
            scale,
        }),
        capture_beyond_viewport: Some(!fits_viewport),
        ..cdp::page::CaptureScreenshot::default()
    };
    let shot = progress
        .race(page.send_command(params))
        .await?
        .map_err(|error| capture::protocol_error(cdp::page::CaptureScreenshot::METHOD, &error))?;
    let data = shot.data.as_str().unwrap_or_default();
    STANDARD
        .decode(data.as_bytes())
        .map_err(|error| capture::malformed(cdp::page::CaptureScreenshot::METHOD, error))
}

/// The frames of this tab with a utility world Neo can reach, the main frame first. A frame
/// the tree read cannot reach is left out, as Playwright's evaluation in all frames skips one
/// that fails.
/// Neo: Playwright's `this.frames()`, read from the tab's frame tree.
async fn all_frames(tab: &FrameTab) -> Vec<Arc<FrameContext>> {
    let main = tab.frames.main();
    let mut frames = vec![Arc::clone(&main)];
    match tab.frames.tree().await {
        Ok(tree) => frames.extend(
            tree.iter()
                .filter(|info| info.id != main.id)
                .filter_map(|info| tab.frames.cached(&info.id)),
        ),
        Err(error) => debug!(?error, "reading the frame tree for a screenshot failed"),
    }
    frames
}

/// What `prepareForScreenshot` changed, frame by frame. Each [`Cleanup`] restores its own
/// frame exactly once: through [`Self::restore`] after the capture, or from its `Drop`.
struct Prepared {
    page: Arc<PageSession>,
    cleanups: Vec<Cleanup>,
}

/// One frame's cleanup handle and the object group that keeps it alive until it ran. Dropped
/// unrestored (the call stopped, the run ended, the run's future was dropped), it restores its
/// frame on the run's tracker, so no path out of a screenshot leaves Playwright's styles,
/// caret colours or cancelled animations in the user's page.
struct Cleanup {
    page: Arc<PageSession>,
    frame: Arc<FrameContext>,
    handle: Handle<ScreenshotCleanup>,
    /// `None` once the restore has been sent.
    group: Option<GroupGuard>,
    /// Held until the restore has run.
    turn: Turn,
}

/// Playwright: `server/screenshotter.ts` `Screenshotter._restorePageAfterScreenshot`
impl Cleanup {
    async fn restore(mut self) {
        let group = self.group.take();
        restore_frame(&self.frame, &self.handle).await;
        // The group releases only now, after the restore has used its handle.
        drop(group);
    }
}

/// Neo: an unrestored cleanup restores its frame in the background.
impl Drop for Cleanup {
    fn drop(&mut self) {
        let Some(group) = self.group.take() else {
            return;
        };
        let frame = Arc::clone(&self.frame);
        let handle = self.handle.clone();
        let turn = Arc::clone(&self.turn);
        self.page.spawn_while_open(async move {
            restore_frame(&frame, &handle).await;
            drop(group);
            drop(turn);
        });
    }
}

/// `Screenshotter._preparePageForScreenshot`: the page function in every frame, errors ignored
/// (Playwright's `safeNonStallingEvaluateInAllFrames`), then the fonts of `frame`.
/// Playwright: `server/screenshotter.ts` `Screenshotter._preparePageForScreenshot`
async fn prepare(
    tab: &FrameTab,
    frame: &Arc<FrameContext>,
    options: &ScreenshotOptions,
    turn: &Turn,
    progress: &Progress,
) -> Result<Prepared, CallError> {
    if options.disable_animations {
        progress.log("  disabled all CSS animations");
    }
    let params = InjectedPrepareForScreenshotParams {
        style: options.style.clone(),
        hide_caret: options.hide_caret,
        disable_animations: options.disable_animations,
    };
    let mut prepared = Prepared {
        page: Arc::clone(&tab.page),
        cleanups: Vec::new(),
    };
    let frames = progress.race(all_frames(tab)).await?;
    // Each frame's request runs on the run's tracker and owns its answer: once sent, it changes
    // the page whether or not this call is still waiting, so the cleanup it answers with must
    // reach a `Cleanup` whatever happens here. A call that stops drops its receivers, and each
    // task's undelivered (or delivered and then dropped) `Cleanup` restores its frame.
    let mut answers = Vec::new();
    for frame in frames {
        let group = tab.group(&frame, "screenshot");
        let params = params.clone();
        let page = Arc::clone(&tab.page);
        let turn = Arc::clone(turn);
        let (sent, answer) = oneshot::channel();
        tab.page.spawn_while_open(async move {
            match frame.call(Some(group.group()), &params).await {
                Ok(result) => {
                    if let Some(handle) = result.cleanup {
                        let _undelivered = sent.send(Cleanup {
                            page,
                            frame,
                            handle,
                            group: Some(group),
                            turn,
                        });
                    }
                }
                Err(error) => {
                    debug!(?error, frame = %frame.id, "preparing a frame for a screenshot failed");
                }
            }
        });
        answers.push(answer);
    }
    for answer in answers {
        // A closed channel is a frame with nothing to restore, or one whose request failed.
        if let Ok(cleanup) = progress.race(answer).await? {
            prepared.cleanups.push(cleanup);
        }
    }
    progress.log("waiting for fonts to load...");
    // Playwright's `.catch(() => {})`: a document without fonts, or one that went away, still
    // takes its screenshot.
    if let Err(error) = progress
        .race(frame.call(None, &InjectedFontsReadyParams {}))
        .await?
    {
        debug!(?error, "waiting for fonts failed");
    }
    progress.log("fonts loaded");
    Ok(prepared)
}

/// Playwright: `server/screenshotter.ts` `Screenshotter._restorePageAfterScreenshot`
impl Prepared {
    /// `Screenshotter._restorePageAfterScreenshot`, awaited unless the call has to stop; the
    /// restore itself always runs to its end on the run's tracker.
    async fn restore(self, progress: &Progress) {
        if self.cleanups.is_empty() {
            return;
        }
        let Prepared { page, cleanups } = self;
        let (done, restored) = oneshot::channel();
        page.spawn_while_open(async move {
            future::join_all(cleanups.into_iter().map(Cleanup::restore)).await;
            let _sent = done.send(());
        });
        let _restored = progress.race(restored).await;
    }
}

/// Playwright: `server/screenshotter.ts` `Screenshotter._restorePageAfterScreenshot`: `window.__pwCleanupScreenshot && window.__pwCleanupScreenshot()`
async fn restore_frame(frame: &FrameContext, handle: &Handle<ScreenshotCleanup>) {
    let params = InjectedRestoreAfterScreenshotParams {
        cleanup: handle.clone(),
    };
    if let Err(error) = frame.call(None, &params).await {
        debug!(?error, frame = %frame.id, "restoring a frame after a screenshot failed");
    }
}

/// Masks drawn for one capture. Hidden once: by [`Self::hide`] after the capture, or from
/// `Drop` on the run's tracker when the capture failed or the call stopped.
struct Masks {
    page: Arc<PageSession>,
    frames: Vec<Arc<FrameContext>>,
    /// Held until the masks are hidden.
    turn: Option<Turn>,
}

/// `Screenshotter._maskElements`: hide any overlay in every frame, then mask each locator's
/// matches in its own frame. `None` without masks.
/// Playwright: `server/screenshotter.ts` `Screenshotter._maskElements`
async fn mask_elements(
    tab: &FrameTab,
    options: &ScreenshotOptions,
    turn: &Turn,
    progress: &Progress,
) -> Result<Option<Masks>, CallError> {
    if options.masks.is_empty() {
        return Ok(None);
    }
    let frames = progress.race(all_frames(tab)).await?;
    let masks = Masks {
        page: Arc::clone(&tab.page),
        frames,
        turn: Some(Arc::clone(turn)),
    };
    progress.race(hide_highlights(&masks.frames)).await?;
    for mask in &options.masks {
        let mask_tab = FrameTab {
            page: Arc::clone(&tab.page),
            scope: mask.scope.clone(),
        };
        // Playwright: `server/screenshotter.ts` `Screenshotter._maskElements`: `frame.selectors.callOnSelector({ selector, strict: false }`
        let located = progress
            .race(frame_selector::locate(&mask_tab, &mask.selector, false))
            .await?
            .map_err(page_error)?;
        let Some(located) = located else {
            continue;
        };
        progress
            .race(located.frame.call(
                None,
                &InjectedMaskElementsParams {
                    selector: located.selector,
                    color: options.mask_color.clone(),
                },
            ))
            .await?
            .map_err(page_error)?;
    }
    Ok(Some(masks))
}

/// Playwright: `server/screenshotter.ts` `Screenshotter._screenshot`: `await progress.race(cleanupHighlight());`
impl Masks {
    /// Playwright's `await progress.race(cleanupHighlight())`.
    async fn hide(mut self, progress: &Progress) {
        let turn = self.turn.take();
        let frames = mem::take(&mut self.frames);
        let (done, hidden) = oneshot::channel();
        self.page.spawn_while_open(async move {
            hide_highlights(&frames).await;
            let _sent = done.send(());
            drop(turn);
        });
        let _hidden = progress.race(hidden).await;
    }
}

/// Playwright: `server/screenshotter.ts` `Screenshotter._screenshot`: `cleanupHighlight().catch(() => {});`
impl Drop for Masks {
    fn drop(&mut self) {
        let Some(turn) = self.turn.take() else {
            return;
        };
        let frames = mem::take(&mut self.frames);
        self.page.spawn_while_open(async move {
            hide_highlights(&frames).await;
            drop(turn);
        });
    }
}

/// `HighlightController.hideHighlights` over `frames`, errors ignored.
/// Playwright: `server/highlightController.ts` `HighlightController.hideHighlights`
async fn hide_highlights(frames: &[Arc<FrameContext>]) {
    let hides = frames
        .iter()
        .map(|frame| frame.call(None, &InjectedHideHighlightParams {}));
    for result in future::join_all(hides).await {
        if let Err(error) = result {
            debug!(?error, "hiding screenshot masks failed");
        }
    }
}

/// The transparent default background of one capture, owned by one task on the run's tracker
/// that sets the override, holds it until the capture is done, and then resets it. The reset
/// goes out only after the override's own request has settled, so a call that stops while the
/// override is still on its way can never leave it applied to later screenshots.
struct Background {
    /// Sent (or dropped) when the capture no longer needs the override.
    release: Option<oneshot::Sender<()>>,
    /// Answers once the reset has been sent and answered.
    reset: Option<oneshot::Receiver<()>>,
}

/// Playwright: `server/screenshotter.ts` `Screenshotter._screenshot`: `await progress.race(this._page.delegate.setBackgroundColor({ r: 0, g: 0, b: 0, a: 0 }));`
impl Background {
    /// Starts the override's task, and returns its guard with the override's answer.
    fn set(page: &Arc<PageSession>, turn: Turn) -> (Self, oneshot::Receiver<Result<(), CdpError>>) {
        let (answer, answered) = oneshot::channel();
        let (release, released) = oneshot::channel::<()>();
        let (reset, was_reset) = oneshot::channel();
        let owner = Arc::clone(page);
        page.spawn_while_open(async move {
            let set = owner
                .send_command(cdp::emulation::SetDefaultBackgroundColorOverride {
                    color: Some(cdp::emulation::Rgba {
                        r: 0,
                        g: 0,
                        b: 0,
                        a: Some(0.0),
                    }),
                })
                .await
                .map(drop);
            let _unread = answer.send(set);
            // A dropped guard is a release too: the call stopped.
            let _released = released.await;
            reset_background(&owner).await;
            let _sent = reset.send(());
            // The next screenshot may start only now.
            drop(turn);
        });
        let guard = Self {
            release: Some(release),
            reset: Some(was_reset),
        };
        (guard, answered)
    }

    /// Playwright's `setBackgroundColor()` after a capture: the reset, awaited unless the call
    /// has to stop.
    async fn reset(mut self, progress: &Progress) {
        if let Some(release) = self.release.take() {
            let _sent = release.send(());
        }
        if let Some(reset) = self.reset.take() {
            let _reset = progress.race(reset).await;
        }
    }
}

/// Playwright: `server/chromium/crPage.ts` `CRPage.setBackgroundColor`
async fn reset_background(page: &PageSession) {
    if let Err(error) = page
        .send_command(cdp::emulation::SetDefaultBackgroundColorOverride { color: None })
        .await
    {
        debug!(%error, "resetting the screenshot background failed");
    }
}

/// Playwright: `server/screenshotter.ts` `trimClipToSize`: `assert(result.width > 0 && result.height > 0, 'Clipped area is either empty or outside the resulting image');`
fn trim_clip_to_size(clip: &Rect, size: Size) -> Result<Rect, PlaywrightError> {
    let x1 = clip.x.min(size.width).max(0.0);
    let y1 = clip.y.min(size.height).max(0.0);
    let x2 = (clip.x + clip.width).min(size.width).max(0.0);
    let y2 = (clip.y + clip.height).min(size.height).max(0.0);
    let result = Rect {
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    };
    if result.width > 0.0 && result.height > 0.0 {
        Ok(result)
    } else {
        Err(PlaywrightError::error(
            "Clipped area is either empty or outside the resulting image",
        ))
    }
}

/// Playwright: `server/helper.ts` `Helper.enclosingIntRect`: `const x2 = Math.ceil(rect.x + rect.width - 1e-3);`
fn enclosing_int_rect(rect: &Rect) -> Rect {
    let x = (rect.x + ENCLOSING_EPSILON).floor();
    let y = (rect.y + ENCLOSING_EPSILON).floor();
    let x2 = (rect.x + rect.width - ENCLOSING_EPSILON).ceil();
    let y2 = (rect.y + rect.height - ENCLOSING_EPSILON).ceil();
    Rect {
        x,
        y,
        width: x2 - x,
        height: y2 - y,
    }
}

/// Playwright: `server/helper.ts` `Helper.enclosingIntSize`: `return { width: Math.floor(size.width + 1e-3), height: Math.floor(size.height + 1e-3) };`
fn enclosing_int(value: f64) -> f64 {
    (value + ENCLOSING_EPSILON).floor()
}

/// A glue size in whole pixels as a float.
/// Neo: the glue answers whole pixels; the arithmetic is Playwright's in floats.
#[expect(
    clippy::cast_precision_loss,
    reason = "CSS pixel sizes are far below 2^52"
)]
fn pixels(value: i64) -> f64 {
    value as f64
}

/// Neo: a page step's failure as the call's error.
fn page_error(error: PageError) -> CallError {
    match error {
        PageError::Closed => PlaywrightError::target_closed().into(),
        PageError::Threw(message) | PageError::Contract(message) => {
            PlaywrightError::error(message).into()
        }
        // Playwright: `server/frames.ts` `Frame.contextDestroyed`: `context.contextDestroyed('Execution context was destroyed, most likely because of a navigation');`
        PageError::NotConnected => PlaywrightError::error(
            "Execution context was destroyed, most likely because of a navigation",
        )
        .into(),
    }
}

/// An element handle's page step failing: its document is gone, so is the element.
/// Playwright: `server/dom.ts` `throwElementIsNotAttached`
fn element_error(error: PageError) -> CallError {
    match error {
        PageError::NotConnected => {
            PlaywrightError::error("Element is not attached to the DOM").into()
        }
        error => page_error(error),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A run's session is gone but its cleanup still holds the tab's turn: the next run's session
    /// gets the same turn and waits; once nothing holds it, the tab starts afresh.
    #[tokio::test]
    async fn a_tabs_screenshot_turn_outlives_its_run_while_cleanup_holds_it() {
        let turns = TabScreenshots::default();
        let page = PageId::new("T".to_string());
        let first_run = turns.turn_for(&page).await;
        let cleanup = Arc::clone(&first_run).lock_owned().await;
        drop(first_run);
        let next_run = turns.turn_for(&page).await;
        assert!(
            next_run.try_lock().is_err(),
            "the next run waits for the cleanup"
        );
        drop(cleanup);
        assert!(next_run.try_lock().is_ok());
        let other = turns.turn_for(&PageId::new("U".to_string())).await;
        assert!(
            !Arc::ptr_eq(&next_run, &other),
            "another tab has its own turn"
        );
        drop((next_run, other));
        assert!(turns.turn_for(&page).await.try_lock().is_ok());
        assert_eq!(turns.0.lock().await.len(), 1, "gone tabs leave the map");
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn params() -> ScreenshotParams {
        ScreenshotParams::from_page(
            &serde_json::from_value::<PageScreenshotParams>(json!({})).expect("no options"),
        )
    }

    #[test]
    fn tests_that_defaults_are_playwrights() {
        let options = ScreenshotOptions::new(params(), Vec::new()).expect("defaults");
        assert_eq!(options.format, Format::Png);
        assert!(options.hide_caret, "caret defaults to hide");
        assert!(!options.disable_animations, "animations default to allow");
        assert!(!options.css_scale, "scale defaults to device");
        assert_eq!(options.mask_color, "#F0F");
    }

    #[test]
    fn tests_that_quality_and_clip_are_validated_as_playwright_does() {
        let png = ScreenshotOptions::new(
            ScreenshotParams {
                quality: Some(50),
                ..params()
            },
            Vec::new(),
        )
        .expect_err("png has no quality");
        assert_eq!(
            png.message,
            "options.quality is unsupported for the png screenshots"
        );
        let range = ScreenshotOptions::new(
            ScreenshotParams {
                format: Some(PageScreenshotParamsType::Jpeg),
                quality: Some(101),
                ..params()
            },
            Vec::new(),
        )
        .expect_err("out of range");
        assert!(range.message.ends_with("got 101"), "{range:?}");
        let clip = ScreenshotOptions::new(
            ScreenshotParams {
                clip: Some(rect(0.0, 0.0, 0.0, 10.0)),
                ..params()
            },
            Vec::new(),
        )
        .expect_err("empty clip");
        assert_eq!(
            clip.message,
            "Expected options.clip.width to be greater than 0."
        );
    }

    #[test]
    fn tests_that_clips_are_trimmed_to_the_area() {
        let size = Size {
            width: 100.0,
            height: 50.0,
        };
        assert_eq!(
            trim_clip_to_size(&rect(-10.0, 10.0, 50.0, 100.0), size).expect("inside"),
            rect(0.0, 10.0, 40.0, 40.0)
        );
        let error = trim_clip_to_size(&rect(200.0, 0.0, 10.0, 10.0), size).expect_err("outside");
        assert_eq!(
            error.message,
            "Clipped area is either empty or outside the resulting image"
        );
    }

    #[test]
    fn tests_that_element_boxes_round_outwards_as_playwright_does() {
        assert_eq!(
            enclosing_int_rect(&rect(10.4, 20.0005, 100.2, 30.0)),
            rect(10.0, 20.0, 101.0, 30.0)
        );
        assert_eq!(enclosing_int(474.9995).to_bits(), 475.0_f64.to_bits());
    }
}
