//! `page.screenshot()` through core's capture path.
//!
//! A per-tab guard prepares every frame, captures through core (which hides the agent pointer),
//! then restores temporary rendering state. Its queue remains held through cancellation cleanup.

use browseros_core::screenshot::{
    ScreenshotBackground, ScreenshotCapture, ScreenshotCaptureOptions, ScreenshotFormat,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::backoff::Backoff;
use crate::page::screenshot_geometry::{GeometryOptions, Scale, ScreenshotGeometry};
use crate::page::screenshot_masks::MaskLocator;
use crate::page::screenshot_preparation::ScreenshotPreparation;
use crate::page::{OpPage, screenshot_document, screenshot_png};
use crate::probe::Rect;

use crate::op_output::{Bytes, to_output};
use crate::op_scope::OpScope;
use crate::target::{PageRef, Strictness, Target};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// Screenshot options cross the prelude flat; temporary rendering changes are restored
/// before the bytes return, while core owns the pointer and pixel capture.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ScreenshotInput {
    page: PageRef,
    #[serde(default)]
    animations: Animations,
    #[serde(default)]
    caret: Caret,
    #[serde(default)]
    style: Option<String>,
    /// `locator.screenshot()` and `elementHandle.screenshot()`: the element to capture.
    #[serde(default)]
    target: Option<Target>,
    /// How `target` resolves; the prelude sends `strict`, as upstream's element screenshots are.
    #[serde(default, rename = "strict")]
    strict: Option<Strictness>,
    #[serde(default)]
    full_page: bool,
    #[serde(default)]
    clip: Option<Rect>,
    #[serde(default)]
    scale: Scale,
    #[serde(default, rename = "type")]
    image: ImageType,
    #[serde(default)]
    omit_background: bool,
    /// JPEG and WebP, 0 to 100.
    #[serde(default)]
    quality: Option<u8>,
    /// Only caller-selected locators; unrelated page captures never inherit these overlays.
    #[serde(default)]
    mask: Vec<MaskLocator>,
    #[serde(default)]
    mask_color: Option<String>,
}

impl ScreenshotInput {
    /// Validate before admission or any temporary rendering changes.
    fn validate(&self, scope: &OpScope<'_>) -> Result<(), PlaywrightError> {
        if self.image == ImageType::Png && self.quality.is_some() {
            return Err(scope.fail(
                ErrorCode::InvalidInput,
                "options.quality is unsupported for the png screenshots",
            ));
        }
        if let Some(quality) = self.quality.filter(|quality| *quality > 100) {
            return Err(scope.fail(
                ErrorCode::InvalidInput,
                &format!(
                    "Expected options.quality to be between 0 and 100 (inclusive), got {quality}"
                ),
            ));
        }
        if let Some(clip) = self.clip {
            if clip.width <= 0.0 {
                return Err(scope.fail(
                    ErrorCode::InvalidInput,
                    "Expected options.clip.width to be greater than 0.",
                ));
            }
            if clip.height <= 0.0 {
                return Err(scope.fail(
                    ErrorCode::InvalidInput,
                    "Expected options.clip.height to be greater than 0.",
                ));
            }
        }
        Ok(())
    }

    /// Chromium encodes WebP losslessly at quality 100, matching upstream's default.
    fn encoding_quality(&self) -> Option<i64> {
        match self.image {
            ImageType::Png => None,
            ImageType::Jpeg => Some(i64::from(self.quality.unwrap_or(80))),
            ImageType::Webp => Some(i64::from(self.quality.unwrap_or(100))),
        }
    }
}

/// How screenshot preparation treats the document's animations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Animations {
    #[default]
    Allow,
    Disabled,
}

/// Upstream hides the caret unless the caller explicitly asks for its initial behavior.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Caret {
    #[default]
    Hide,
    Initial,
}

/// Playwright's screenshot formats; PNG and WebP support transparency.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ImageType {
    #[default]
    Png,
    Jpeg,
    Webp,
}

/// Captures the page as `input` asks.
pub(crate) async fn capture(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ScreenshotInput,
) -> Result<Value, PlaywrightError> {
    input.validate(scope)?;
    let format = match input.image {
        ImageType::Png => ScreenshotFormat::Png,
        ImageType::Jpeg => ScreenshotFormat::Jpeg,
        ImageType::Webp => ScreenshotFormat::Webp,
    };
    let capture = async {
        let backoff = Backoff::outer(scope.timeout());
        for attempt in 0.. {
            let target = ScreenshotGeometry::bind(
                pw,
                scope,
                input.target.as_ref(),
                input.strict.unwrap_or(Strictness::Strict),
            )
            .await?;
            let page = OpPage::admit_handle(pw, scope, &input.page).await?;
            let mut preparation = ScreenshotPreparation::acquire(&page).await;
            preparation
                .prepare(
                    &page,
                    scope,
                    json!([
                        input.style.as_deref().unwrap_or_default(),
                        input.caret == Caret::Hide,
                        input.animations == Animations::Disabled,
                        false,
                    ]),
                )
                .await;
            let captured = async {
                preparation
                    .wait_for_fonts(&page, scope, target.as_deref())
                    .await?;
                let pixels = capture_pixels(
                    pw,
                    &page,
                    scope,
                    &input,
                    target.as_deref(),
                    format,
                    &mut preparation,
                );
                screenshot_document::capture(&page, scope, target.as_deref(), pixels).await
            }
            .await;
            preparation.restore().await;
            match captured {
                Ok(screenshot_document::Attempt::Captured(data)) => return Ok(data),
                Ok(screenshot_document::Attempt::Navigated) => {
                    scope.log("document changed during screenshot, retrying".to_owned());
                }
                Err(error)
                    if error.error_code() == Some(ErrorCode::ElementDetached)
                        && matches!(input.target, Some(Target::Locator(_))) =>
                {
                    scope.log("element was detached from the DOM, retrying".to_owned());
                }
                Err(error) => return Err(error),
            }
            scope
                .sleep(backoff.wait(attempt + 1))
                .await
                .map_err(|_expired| scope.timed_out(ErrorCode::WaitTimeout))?;
        }
        Err(scope.timed_out(ErrorCode::WaitTimeout))
    };
    // Frame resolution and geometry carry large futures; keep their storage at this leaf.
    let data = scope
        .within(Box::pin(capture))
        .await
        .map_err(|_expired| scope.timed_out(ErrorCode::WaitTimeout))??;
    to_output(scope, &Bytes::from_base64(data))
}

/// Core owns admission and capture serialization; this layer supplies Playwright's geometry.
async fn capture_pixels(
    pw: &Playwright,
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    input: &ScreenshotInput,
    target: Option<&Target>,
    format: ScreenshotFormat,
    preparation: &mut ScreenshotPreparation,
) -> Result<String, PlaywrightError> {
    let default_viewport = input.target.is_none()
        && !input.full_page
        && input.clip.is_none()
        && input.scale == Scale::Device;
    let background = if input.omit_background && input.image != ImageType::Jpeg {
        ScreenshotBackground::Transparent
    } else {
        ScreenshotBackground::Default
    };
    // The default opaque path needs no layout or utility-world support. Transparent captures
    // use the normalized core path, which owns background isolation from audit captures.
    if default_viewport && background == ScreenshotBackground::Default && input.mask.is_empty() {
        return scope
            .view()
            .screenshot(
                page.core_page().clone(),
                ScreenshotCaptureOptions {
                    format: Some(format),
                    quality: input.encoding_quality(),
                    full_page: Some(false),
                    annotate: None,
                    clip: None,
                },
            )
            .await
            .map(|shot| shot.data)
            .map_err(|error| scope.core_error(&error));
    }
    let geometry = if default_viewport {
        None
    } else {
        Some(
            Box::pin(ScreenshotGeometry::measure(
                pw,
                page,
                scope,
                GeometryOptions {
                    target,
                    strict: input.strict.unwrap_or(Strictness::Strict),
                    full_page: input.full_page,
                    clip: input.clip,
                    scale: input.scale,
                },
            ))
            .await?,
        )
    };
    let mut params = Map::new();
    params.insert("format".to_owned(), json!(format.as_str()));
    params.insert(
        "captureBeyondViewport".to_owned(),
        json!(
            geometry
                .as_ref()
                .is_some_and(|geometry| geometry.beyond_viewport)
        ),
    );
    if let Some(geometry) = &geometry {
        let clip = geometry.clip;
        params.insert("clip".to_owned(), json!({ "x": clip.x, "y": clip.y, "width": clip.width, "height": clip.height, "scale": clip.scale }));
    }
    if let Some(quality) = input.encoding_quality() {
        params.insert("quality".to_owned(), json!(quality));
    }
    if !input.mask.is_empty() {
        let capture = scope
            .view()
            .begin_screenshot(page.core_page().clone(), background)
            .await
            .map_err(|error| scope.core_error(&error))?;
        preparation.hold_capture(capture);
        preparation
            .mask(
                page,
                scope,
                &input.mask,
                input
                    .mask_color
                    .as_deref()
                    .filter(|color| !color.is_empty())
                    .unwrap_or("#F0F"),
            )
            .await?;
        let capture = preparation.capture().ok_or_else(|| {
            scope.fail(ErrorCode::Internal, "Screenshot capture lease is missing")
        })?;
        return capture_normalized(capture, params, geometry.as_ref(), format, scope).await;
    }
    let capture = scope
        .view()
        .begin_screenshot(page.core_page().clone(), background)
        .await
        .map_err(|error| scope.core_error(&error))?;
    let data = capture_normalized(&capture, params, geometry.as_ref(), format, scope).await;
    capture.finish().await;
    data
}

async fn capture_normalized(
    capture: &ScreenshotCapture,
    params: Map<String, Value>,
    geometry: Option<&ScreenshotGeometry>,
    format: ScreenshotFormat,
    scope: &OpScope<'_>,
) -> Result<String, PlaywrightError> {
    if format == ScreenshotFormat::Png
        && let Some(geometry) = geometry
        && screenshot_png::needs_tiles(geometry)
    {
        return screenshot_png::capture(capture, params, geometry, scope).await;
    }
    capture
        .capture(Value::Object(params))
        .await
        .map_err(|error| scope.core_error(&error))
}

#[cfg(test)]
mod tests;
