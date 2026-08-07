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
Defaults to JPEG quality 80 around 1024x768; prefer snapshot for structure/actions.";

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum ScreenshotFormat {
    #[default]
    Jpeg,
    Png,
    Webp,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ScreenshotSize {
    #[serde(default = "default_width")]
    #[schemars(range(min = 1, max = 4096))]
    width: i64,
    #[serde(default = "default_height")]
    #[schemars(range(min = 1, max = 4096))]
    height: i64,
}

impl Default for ScreenshotSize {
    fn default() -> Self {
        Self {
            width: default_width(),
            height: default_height(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ScreenshotArgs {
    page: u32,
    #[serde(default)]
    format: ScreenshotFormat,
    #[schemars(range(min = 0, max = 100))]
    quality: Option<i64>,
    /// Max viewport capture size. Defaults to 1024x768.
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
            let page = ctx.session.pages.get_session(PageId(args.page)).await?;
            options.clip =
                Some(build_screenshot_clip(&page.session, &args.size.unwrap_or_default()).await?);
        }
        let result = ctx.session.screenshot(PageId(args.page), options).await?;
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
    target: &ScreenshotSize,
) -> ToolExecResult<Viewport> {
    let metrics: LayoutMetrics = session.send("Page.getLayoutMetrics", json!({})).await?;
    let viewport = metrics
        .css_layout_viewport
        .unwrap_or(metrics.layout_viewport);
    let scale = if viewport.client_width > 0.0 && viewport.client_height > 0.0 {
        1.0_f64
            .min(target.width as f64 / viewport.client_width)
            .min(target.height as f64 / viewport.client_height)
    } else {
        1.0
    };
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
        && (!(1..=4096).contains(&size.width) || !(1..=4096).contains(&size.height))
    {
        return Err(crate::framework::ToolError::InvalidArguments(vec![
            crate::framework::ArgIssue {
                path: "size".to_string(),
                message: "width and height must be between 1 and 4096".to_string(),
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

fn default_width() -> i64 {
    1024
}

fn default_height() -> i64 {
    768
}
