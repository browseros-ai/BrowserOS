//! `setViewport`, offered only by hosts that attach viewport state to their views (hosted
//! BrowserOS neo). It is not part of [`super::catalog`]; the host adds [`definition`].
use crate::framework::{
    ArgIssue, ToolCtx, ToolError, ToolExecResult, ToolResult, parse_args, text_result,
};
use browseros_core::{
    PageId,
    viewport::{Size, ViewportMode, ViewportReport, ViewportRequest},
};
use futures_util::future::BoxFuture;
use rmcp::model::ToolAnnotations;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

pub const NAME: &str = "setViewport";

const DESCRIPTION: &str = "Set how one tab renders. mobile: a 400x906 CSS px phone at device scale 2 with touch and a mobile user agent and client hints; the mouse acts as a finger, so a drag scrolls or draws as a touch would, and a double tap does not zoom. desktop: 1920x1080 at device scale 1 with the browser's own identity. width and height (together, CSS px) give mobile or desktop another device size. default: the tab's natural size; removes only what setViewport set. Tabs start in default. The mode stays with the tab through reloads, navigation and reconnects. The browser window is never moved, resized or taken out of fullscreen: the page renders at the given size inside whatever window it has. Size or fullscreen the window first; after the window is resized, call setViewport again. zoom (mobile only, percent 25-400, default 100) scales the on-screen preview, not the page's CSS size. The page is not reloaded: servers that vary by user agent see the new identity from the next navigation. The result reports what the page actually shows.";

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Mobile,
    Desktop,
    Default,
}

impl From<Mode> for ViewportMode {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::Mobile => Self::Mobile,
            Mode::Desktop => Self::Desktop,
            Mode::Default => Self::Default,
        }
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetViewportArgs {
    page: u32,
    mode: Mode,
    /// Mobile preview zoom in percent, 25-400. Defaults to 100 on every call; desktop and
    /// default take none.
    zoom: Option<f64>,
    /// Device viewport width in CSS px, 100-4096, with height. Mobile and desktop only;
    /// defaults to 400 for mobile and 1920 for desktop on every call.
    #[schemars(range(min = 100, max = 4096))]
    width: Option<i64>,
    /// Device viewport height in CSS px, 100-4096, with width. Defaults to 906 for mobile and
    /// 1080 for desktop on every call.
    #[schemars(range(min = 100, max = 4096))]
    height: Option<i64>,
}

#[must_use]
pub fn definition() -> crate::framework::ToolDef {
    super::def::<SetViewportArgs>(
        NAME,
        DESCRIPTION,
        Some(
            ToolAnnotations::new()
                .read_only(false)
                .destructive(false)
                .idempotent(true),
        ),
        handler,
    )
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let (page, request) = parse(raw)?;
        let report = ctx.browser.set_viewport(page, request, &ctx.cancel).await?;
        Ok(Some(text_result(
            summary(&report),
            Some(report_json(&report)),
        )))
    })
}

fn parse(raw: Value) -> ToolExecResult<(PageId, ViewportRequest)> {
    let args: SetViewportArgs = parse_args(raw)?;
    let issue = |path: &str| {
        let path = path.to_string();
        move |message| ToolError::InvalidArguments(vec![ArgIssue { path, message }])
    };
    let request = ViewportRequest::new(args.mode.into(), args.zoom).map_err(issue("zoom"))?;
    let request = match (args.width, args.height) {
        (Some(width), Some(height)) => request
            .with_size(Size { width, height })
            .map_err(issue("width"))?,
        (None, None) => request,
        (Some(_), None) => return Err(issue("height")("give height with width".to_string())),
        (None, Some(_)) => return Err(issue("width")("give width with height".to_string())),
    };
    Ok((PageId(args.page), request))
}

/// The run SDK's `page.setViewport(opts)`: the same arguments with the page from the handle.
pub(crate) fn parse_for_script(raw: Value) -> Result<(PageId, ViewportRequest), String> {
    parse(raw).map_err(|err| match err {
        ToolError::InvalidArguments(issues) => format!(
            "setViewport: {}",
            issues
                .iter()
                .map(|issue| format!("{}: {}", issue.path, issue.message))
                .collect::<Vec<_>>()
                .join("; ")
        ),
        other => format!("setViewport: {other}"),
    })
}

pub(crate) fn report_json(report: &ViewportReport) -> Value {
    serde_json::to_value(report).unwrap_or(Value::Null)
}

fn summary(report: &ViewportReport) -> String {
    let page = report.page;
    let metrics = &report.viewport;
    let mut size = match report.size {
        Some(device) => format!(
            "{}x{} CSS px at device scale {}",
            device.width, device.height, metrics.device_scale_factor
        ),
        None => format!(
            "{}x{} CSS px at device scale {}",
            metrics.width, metrics.height, metrics.device_scale_factor
        ),
    };
    // Wide pages lay out wider than a mobile device, as Chromium does on phones.
    if let Some(device) = report.size
        && (metrics.width, metrics.height) != (device.width as f64, device.height as f64)
    {
        size.push_str(&format!(
            " (this page lays out at {}x{})",
            metrics.width, metrics.height
        ));
    }
    let mut lines = vec![match (report.mode, report.zoom) {
        (ViewportMode::Mobile, zoom) => format!(
            "Page {page} is mobile: {size}{}, preview zoom {}%.",
            if metrics.touch_points > 0 {
                " with touch"
            } else {
                ""
            },
            zoom.unwrap_or(browseros_core::viewport::DEFAULT_ZOOM)
        ),
        (ViewportMode::Desktop, _) => format!("Page {page} is desktop: {size}."),
        (ViewportMode::Default, _) => format!(
            "Page {page} is default: its natural size, now {size}. setViewport's overrides, if any, are removed."
        ),
    }];
    if report.mode != ViewportMode::Default {
        lines.push(
            "The page was not reloaded; servers that vary by user agent see the new identity from the next navigation."
                .to_string(),
        );
    }
    lines.join(" ")
}
