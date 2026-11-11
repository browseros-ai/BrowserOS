//! `page.screenshot()` through core's capture path.
//!
//! A per-tab guard prepares every frame, captures through core (which hides the agent pointer),
//! then restores temporary rendering state. Its queue remains held through cancellation cleanup.

use browseros_core::screenshot::{ScreenshotCaptureOptions, ScreenshotFormat, Viewport};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::page::OpPage;
use crate::page::screenshot_preparation::ScreenshotPreparation;

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
    _strict: Option<Strictness>,
    #[serde(default)]
    full_page: bool,
    #[serde(default)]
    clip: Option<Clip>,
    #[serde(default, rename = "type")]
    image: ImageType,
    /// JPEG only, 0 to 100.
    #[serde(default)]
    quality: Option<u8>,
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

/// A rectangle of the page in CSS pixels.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Clip {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// Playwright's two screenshot formats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ImageType {
    #[default]
    Png,
    Jpeg,
}

/// Captures the page as `input` asks.
pub(crate) async fn capture(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ScreenshotInput,
) -> Result<Value, PlaywrightError> {
    // Check the op's declared capability before admitting or preparing its page.
    scope.require(Capability::Capture)?;
    if input.target.is_some() {
        return Err(scope.not_built());
    }
    if input.image == ImageType::Png && input.quality.is_some() {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            "options.quality is unsupported for the png screenshots",
        ));
    }
    if input.quality.is_some_and(|quality| quality > 100) {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            "options.quality is out of the range 0..100",
        ));
    }
    let options = ScreenshotCaptureOptions {
        format: Some(match input.image {
            ImageType::Png => ScreenshotFormat::Png,
            ImageType::Jpeg => ScreenshotFormat::Jpeg,
        }),
        quality: input.quality.map(i64::from),
        full_page: Some(input.full_page),
        annotate: None,
        clip: input.clip.map(|clip| Viewport {
            x: clip.x,
            y: clip.y,
            width: clip.width,
            height: clip.height,
            scale: 1.0,
        }),
    };
    let page = OpPage::admit_handle(pw, scope, &input.page).await?;
    let capture = async {
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
        let shot = scope
            .view()
            .screenshot(page.core_page().clone(), options)
            .await;
        preparation.restore().await;
        shot.map_err(|error| scope.core_error(&error))
    };
    // Frame installation and preparation have large futures. Keep this leaf boxed so every
    // caller of Playwright::invoke does not inherit their stack allocation.
    let shot = scope
        .within(Box::pin(capture))
        .await
        .map_err(|_expired| scope.timed_out(ErrorCode::WaitTimeout))??;
    to_output(scope, &Bytes::from_base64(shot.data))
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
