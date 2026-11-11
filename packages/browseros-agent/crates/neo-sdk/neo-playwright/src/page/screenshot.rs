//! `page.screenshot()` through core's capture path.
//!
//! A per-tab guard prepares every frame, captures through core (which hides the agent pointer),
//! then restores temporary rendering state. Its queue remains held through cancellation cleanup.

use browseros_core::screenshot::{
    ScreenshotBackground, ScreenshotCaptureOptions, ScreenshotFormat,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::backoff::Backoff;
use crate::page::OpPage;
use crate::page::screenshot_geometry::{GeometryOptions, Scale, ScreenshotGeometry};
use crate::page::screenshot_preparation::ScreenshotPreparation;
use crate::probe::Rect;

use crate::op_output::{Bytes, to_output};
use crate::op_scope::OpScope;
use crate::op_spec::Capability;
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
    // Check the op's declared capability before admitting or preparing its page.
    scope.require(Capability::Capture)?;
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
                capture_pixels(pw, &page, scope, &input, target.as_deref(), format).await
            }
            .await;
            preparation.restore().await;
            match captured {
                Err(error)
                    if error.error_code() == Some(ErrorCode::ElementDetached)
                        && matches!(input.target, Some(Target::Locator(_))) =>
                {
                    scope.log("element was detached from the DOM, retrying".to_owned());
                    scope
                        .sleep(backoff.wait(attempt + 1))
                        .await
                        .map_err(|_expired| scope.timed_out(ErrorCode::WaitTimeout))?;
                }
                done => return done,
            }
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
    if default_viewport && background == ScreenshotBackground::Default {
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
    if let Some(geometry) = geometry {
        let clip = geometry.clip;
        params.insert("clip".to_owned(), json!({ "x": clip.x, "y": clip.y, "width": clip.width, "height": clip.height, "scale": clip.scale }));
    }
    if let Some(quality) = input.encoding_quality() {
        params.insert("quality".to_owned(), json!(quality));
    }
    scope
        .view()
        .screenshot_cdp_with_background(page.core_page().clone(), Value::Object(params), background)
        .await
        .map_err(|error| scope.core_error(&error))
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use browseros_core::BrowserView;
    use browseros_core::testing::TestConnection;
    use neo_trace::ErrorOwner;
    use serde_json::json;

    use super::{ScreenshotInput, capture};
    use crate::op_scope::OpScope;
    use crate::target::CallHeader;
    use crate::test_env::{self, TestEnv};
    use crate::test_support::{self, RecordingHooks};
    use crate::{Op, Playwright};

    /// The capture path only serves ops that declare `Capture`; undeclared calls fail before
    /// the page is admitted or prepared.
    #[tokio::test]
    async fn tests_that_only_an_op_declaring_capture_is_captured() -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new([]);
        let test_support::OneTab { browser } = test_support::one_tab(&connection).await?;
        connection.always("Page.captureScreenshot", json!({ "data": "cG5n" }))?;
        let playwright = Playwright::new(Arc::clone(&browser), Arc::new(RecordingHooks));
        let env = TestEnv::over(BrowserView::unbound(Arc::clone(&browser), None));
        let input = || {
            serde_json::from_value::<ScreenshotInput>(
                json!({ "page": { "$neo": "page", "tab": 7 } }),
            )
        };
        let screenshot = Op::from_name("page.screenshot").ok_or("page.screenshot is an op")?;
        let declared = OpScope::new(&env, screenshot, CallHeader::default());
        capture(&playwright, &declared, input()?).await?;
        let evaluate = Op::from_name("page.evaluate").ok_or("page.evaluate is an op")?;
        let undeclared = OpScope::new(&env, evaluate, CallHeader::default());
        let sent_before = connection.calls()?.len();
        let error = capture(&playwright, &undeclared, input()?)
            .await
            .err()
            .ok_or("an op that does not declare Capture must not be captured")?;
        assert_eq!(
            (error.owner(), error.code()),
            (ErrorOwner::Neo, "internal"),
            "{error}"
        );
        assert_eq!(
            connection.calls()?.len(),
            sent_before,
            "the undeclared op admitted and prepared nothing"
        );
        let captures = connection
            .calls()?
            .iter()
            .filter(|call| call.method == "Page.captureScreenshot")
            .count();
        assert_eq!(
            captures, 1,
            "only the op that declares Capture was captured"
        );
        Ok(())
    }

    /// The declaration is checked before anything else: an op that does not declare `Capture`
    /// is refused as `neo / internal` even when its browser is gone, so it never reached the
    /// page's lookup, admission or the pointer hide.
    #[tokio::test]
    async fn tests_that_an_undeclared_op_is_refused_before_its_page_is_looked_up()
    -> Result<(), Box<dyn Error>> {
        let playwright = Playwright::new(test_env::offline_session(), Arc::new(RecordingHooks));
        let env = TestEnv::new();
        let evaluate = Op::from_name("page.evaluate").ok_or("page.evaluate is an op")?;
        let undeclared = OpScope::new(&env, evaluate, CallHeader::default());
        let input: ScreenshotInput =
            serde_json::from_value(json!({ "page": { "$neo": "page", "tab": 7 } }))?;
        let error = capture(&playwright, &undeclared, input)
            .await
            .err()
            .ok_or("an undeclared op is refused")?;
        assert_eq!(
            (error.owner(), error.code()),
            (ErrorOwner::Neo, "internal"),
            "{error}"
        );
        Ok(())
    }
}
