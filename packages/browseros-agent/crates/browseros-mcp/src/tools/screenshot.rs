use crate::framework::{ToolCtx, ToolExecResult, ToolResult, parse_args, pending_dialog_result};
use base64::Engine;
use browseros_core::{
    PageId,
    screenshot::{ScreenshotCaptureOptions, ScreenshotFormat as CoreScreenshotFormat, Viewport},
};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const DEFAULT_SCREENSHOT_QUALITY: i64 = 80;
const DESCRIPTION: &str = "\
Capture a screenshot of the page, returned inline. \
Defaults to JPEG quality 80 at the current CSS viewport, with native device pixel density; \
prefer snapshot for structure/actions.";

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum ScreenshotFormat {
    #[default]
    Jpeg,
    Png,
    Webp,
}

/// Optional CSS viewport bounds; only supplied axes constrain the capture scale.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScreenshotSize {
    #[schemars(range(min = 1, max = 4096))]
    width: Option<i64>,
    #[schemars(range(min = 1, max = 4096))]
    height: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScreenshotArgs {
    page: u32,
    #[serde(default)]
    format: ScreenshotFormat,
    #[schemars(range(min = 0, max = 100))]
    quality: Option<i64>,
    /// Max viewport size in CSS pixels, preserving aspect ratio without upscaling.
    /// No cap by default; omitted width or height is unconstrained. Ignored for fullPage.
    size: Option<ScreenshotSize>,
    /// Capture beyond the viewport.
    #[serde(rename = "fullPage")]
    full_page: Option<bool>,
    /// Overlay numbered refs from a fresh snapshot. Defaults false.
    annotate: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LayoutMetrics {
    css_layout_viewport: Option<LayoutViewport>,
    layout_viewport: LayoutViewport,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LayoutViewport {
    page_x: f64,
    page_y: f64,
    client_width: f64,
    client_height: f64,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<ScreenshotArgs>(
        "screenshot",
        DESCRIPTION,
        Some(super::read_only_annotations()),
        handler,
    )
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: ScreenshotArgs = parse_args(raw)?;
        validate_args(&args)?;
        if args.annotate.unwrap_or(false)
            && let Some(result) = pending_dialog_result(ctx, PageId(args.page))
        {
            return Ok(Some(result));
        }
        let full_page = args.full_page.unwrap_or(false);
        let mut options = ScreenshotCaptureOptions {
            format: Some(core_format(&args.format)),
            quality: screenshot_quality(&args.format, args.quality),
            full_page: Some(full_page),
            annotate: Some(args.annotate.unwrap_or(false)),
            clip: None,
        };
        if !full_page {
            // A bound view's page session brackets Page.getLayoutMetrics with scope and
            // target-incarnation checks; the raw registry session would bypass the profile.
            let page = ctx.browser.page_session(PageId(args.page)).await?;
            options.clip = Some(build_screenshot_clip(&page.session, args.size.as_ref()).await?);
        }
        let result = ctx.browser.screenshot(PageId(args.page), options).await?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&result.data)
            .map(|bytes| bytes.len())
            .unwrap_or(0);
        let mut structured = json!({
            "page": args.page,
            "format": format_name(&args.format),
            "bytes": bytes
        });
        if !result.annotations.is_empty()
            && let Value::Object(object) = &mut structured
        {
            object.insert(
                "annotations".to_string(),
                Value::Array(
                    result
                        .annotations
                        .iter()
                        .map(|annotation| {
                            let mut value = json!({
                                "ref": annotation.ref_id,
                                "number": annotation.number,
                                "role": annotation.role,
                                "box": {
                                    "x": annotation.box_.x,
                                    "y": annotation.box_.y,
                                    "width": annotation.box_.width,
                                    "height": annotation.box_.height
                                }
                            });
                            if let (Value::Object(object), Some(name)) =
                                (&mut value, &annotation.name)
                            {
                                object.insert("name".to_string(), json!(name));
                            }
                            value
                        })
                        .collect(),
                ),
            );
        }
        Ok(Some(ToolResult::image(
            result.data,
            result.mime_type,
            structured,
        )))
    })
}

async fn build_screenshot_clip(
    session: &browseros_core::ProtocolSession,
    target: Option<&ScreenshotSize>,
) -> ToolExecResult<Viewport> {
    let metrics: LayoutMetrics = session.send("Page.getLayoutMetrics", json!({})).await?;
    let viewport = metrics
        .css_layout_viewport
        .unwrap_or(metrics.layout_viewport);
    // The clip covers the user's CSS viewport without resizing the page. There
    // is no default cap: scale 1 preserves native device density (2x on Retina).
    let mut scale = 1.0_f64;
    if let Some(target) = target {
        if let Some(width) = target.width
            && viewport.client_width > 0.0
        {
            scale = scale.min(width as f64 / viewport.client_width);
        }
        if let Some(height) = target.height
            && viewport.client_height > 0.0
        {
            scale = scale.min(height as f64 / viewport.client_height);
        }
    }
    Ok(Viewport {
        x: viewport.page_x,
        y: viewport.page_y,
        width: viewport.client_width,
        height: viewport.client_height,
        scale,
    })
}

fn validate_args(args: &ScreenshotArgs) -> ToolExecResult<()> {
    if let Some(quality) = args.quality
        && !(0..=100).contains(&quality)
    {
        return Err(crate::framework::ToolError::InvalidArguments(vec![
            crate::framework::ArgIssue {
                path: "quality".to_string(),
                message: "Number must be greater than or equal to 0 and less than or equal to 100"
                    .to_string(),
            },
        ]));
    }
    if let Some(size) = &args.size
        && [size.width, size.height]
            .into_iter()
            .flatten()
            .any(|dimension| !(1..=4096).contains(&dimension))
    {
        return Err(crate::framework::ToolError::InvalidArguments(vec![
            crate::framework::ArgIssue {
                path: "size".to_string(),
                message: "supplied width and height must be between 1 and 4096".to_string(),
            },
        ]));
    }
    Ok(())
}

fn screenshot_quality(format: &ScreenshotFormat, quality: Option<i64>) -> Option<i64> {
    if matches!(format, ScreenshotFormat::Jpeg) {
        Some(quality.unwrap_or(DEFAULT_SCREENSHOT_QUALITY))
    } else {
        None
    }
}

fn core_format(format: &ScreenshotFormat) -> CoreScreenshotFormat {
    match format {
        ScreenshotFormat::Jpeg => CoreScreenshotFormat::Jpeg,
        ScreenshotFormat::Png => CoreScreenshotFormat::Png,
        ScreenshotFormat::Webp => CoreScreenshotFormat::Webp,
    }
}

fn format_name(format: &ScreenshotFormat) -> &'static str {
    match format {
        ScreenshotFormat::Jpeg => "jpeg",
        ScreenshotFormat::Png => "png",
        ScreenshotFormat::Webp => "webp",
    }
}
