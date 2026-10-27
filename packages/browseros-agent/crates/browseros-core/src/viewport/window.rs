//! Fits a tab's own window around its preview with `Browser.setContentsSize`, which sizes the
//! window from the browser's frame but reports success even when the OS clamps the result.
use super::{Dimension, LimitedBy, Size, WindowBounds, WindowFit, WindowLimit};
use crate::{BrowserView, CoreError, PageId, SessionId, TargetError, TargetId, WindowId};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::time::{Instant, sleep};

const SETTLE_LIMIT: Duration = Duration::from_millis(1_500);
const SETTLE_POLL: Duration = Duration::from_millis(25);
/// Consecutive equal reads before bounds count as applied; some window managers resize late.
const STABLE_READS: usize = 3;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowForTarget {
    window_id: i64,
    bounds: WindowBounds,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreenArea {
    avail_left: i64,
    avail_top: i64,
    avail_width: i64,
    avail_height: i64,
}

pub(super) struct Placement {
    pub(super) window: WindowId,
    pub(super) bounds: WindowBounds,
}

/// The window holding `target` now. It must still be the window the caller locked.
pub(super) async fn locate(
    view: &BrowserView,
    page: &PageId,
    target: &TargetId,
    expected: Option<WindowId>,
) -> Result<Placement, CoreError> {
    let reply = view
        .send_at_capture(
            "Browser.getWindowForTarget",
            json!({"targetId": target.0}),
            None,
        )
        .await?;
    let found: WindowForTarget = decode(reply)?;
    if expected.is_some_and(|expected| expected.0 != found.window_id) {
        return Err(TargetError::PageTargetChanged(page.clone()).into());
    }
    Ok(Placement {
        window: WindowId(found.window_id),
        bounds: found.bounds,
    })
}

/// Restores the window to normal, fits its contents to `contents`, and waits for the bounds.
/// With `classify`, a 1 px contents probe shows which dimensions really match: an exact one
/// follows the probe, a clamped one cannot.
pub(super) async fn fit(
    view: &BrowserView,
    session: &SessionId,
    placement: Placement,
    contents: Size,
    classify: bool,
) -> Result<WindowFit, CoreError> {
    let window = placement.window;
    let mut restored_from = None;
    if placement.bounds.window_state != "normal" {
        restored_from = Some(placement.bounds.window_state.clone());
        view.send_at_capture(
            "Browser.setWindowBounds",
            json!({"windowId": window.0, "bounds": {"windowState": "normal"}}),
            None,
        )
        .await?;
        settle(view, window).await?;
    }
    let fitted = resize(view, window, contents).await?;
    let mut fit = WindowFit {
        window_id: window.0,
        contents,
        bounds: fitted.clone(),
        exact: true,
        limited: Vec::new(),
        restored_from,
    };
    if !classify {
        return Ok(fit);
    }
    let probe = Size {
        width: contents.width - 1,
        height: contents.height - 1,
    };
    let probed = resize(view, window, probe).await?;
    fit.bounds = resize(view, window, contents).await?;
    let mut limited = Vec::new();
    if probed.width != fitted.width - 1 {
        limited.push(Dimension::Width);
    }
    if probed.height != fitted.height - 1 {
        limited.push(Dimension::Height);
    }
    if !limited.is_empty() {
        let area = work_area(view, session, &fit.bounds).await;
        fit.limited = limited
            .into_iter()
            .map(|dimension| WindowLimit {
                dimension,
                by: limited_by(dimension, &fit.bounds, area.as_ref()),
            })
            .collect();
    }
    fit.exact = fit.limited.is_empty();
    Ok(fit)
}

/// A window that still has the bounds of its last fit needs nothing.
pub(super) fn unchanged(placement: &Placement, last: Option<&WindowBounds>) -> bool {
    last.is_some_and(|last| *last == placement.bounds)
}

async fn resize(
    view: &BrowserView,
    window: WindowId,
    contents: Size,
) -> Result<WindowBounds, CoreError> {
    view.send_at_capture(
        "Browser.setContentsSize",
        json!({"windowId": window.0, "width": contents.width, "height": contents.height}),
        None,
    )
    .await?;
    settle(view, window).await
}

async fn settle(view: &BrowserView, window: WindowId) -> Result<WindowBounds, CoreError> {
    let deadline = Instant::now() + SETTLE_LIMIT;
    let mut last = read(view, window).await?;
    let mut equal = 1;
    while equal < STABLE_READS && Instant::now() < deadline {
        sleep(SETTLE_POLL).await;
        let next = read(view, window).await?;
        equal = if next == last { equal + 1 } else { 1 };
        last = next;
    }
    Ok(last)
}

async fn read(view: &BrowserView, window: WindowId) -> Result<WindowBounds, CoreError> {
    let reply = view
        .send_at_capture(
            "Browser.getWindowBounds",
            json!({"windowId": window.0}),
            None,
        )
        .await?;
    decode(reply["bounds"].clone())
}

async fn work_area(
    view: &BrowserView,
    session: &SessionId,
    bounds: &WindowBounds,
) -> Option<ScreenArea> {
    let reply = view
        .send_at_capture("Emulation.getScreenInfos", json!({}), Some(session))
        .await
        .ok()?;
    let screens: Vec<ScreenArea> = serde_json::from_value(reply["screenInfos"].clone()).ok()?;
    let contains = |area: &&ScreenArea| {
        (area.avail_left..area.avail_left + area.avail_width).contains(&bounds.left)
            && (area.avail_top..area.avail_top + area.avail_height).contains(&bounds.top)
    };
    let index = screens.iter().position(|area| contains(&area)).unwrap_or(0);
    screens.into_iter().nth(index)
}

/// A clamped dimension as large as the work area hit the screen; any other was held at the
/// browser's minimum window size.
fn limited_by(dimension: Dimension, bounds: &WindowBounds, area: Option<&ScreenArea>) -> LimitedBy {
    let Some(area) = area else {
        return LimitedBy::Unknown;
    };
    let (size, available) = match dimension {
        Dimension::Width => (bounds.width, area.avail_width),
        Dimension::Height => (bounds.height, area.avail_height),
    };
    if size >= available {
        LimitedBy::Screen
    } else {
        LimitedBy::Minimum
    }
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, CoreError> {
    serde_json::from_value(value).map_err(|err| CoreError::Message(format!("window reply: {err}")))
}
