//! `page.screenshot()` through core's capture path.
//!
//! The capture goes through core's screenshot path, which serializes captures per session and
//! hides the agent pointer for the shot.

use browseros_core::screenshot::{ScreenshotCaptureOptions, ScreenshotFormat, Viewport};
use serde::Deserialize;
use serde_json::Value;

use crate::op_output::{Bytes, to_output};
use crate::op_scope::OpScope;
use crate::op_spec::Capability;
use crate::target::{PageRef, Strictness, Target};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// `page.screenshot({ fullPage, clip, type, quality })`. The prelude writes `path` itself and
/// refuses the options Neo does not have (`mask`, `style`, `omitBackground`, `scale: 'css'`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ScreenshotInput {
    page: PageRef,
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
    let page = input.page.tab();
    let shot = scope
        .view()
        .screenshot(
            scope.admit(pw.browser(), page).await?.core_page.clone(),
            options,
        )
        .await
        .map_err(|error| scope.core_error(&error))?;
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
        let playwright = Playwright::new(Arc::clone(&browser), Arc::new(RecordingHooks::default()));
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
        let playwright = Playwright::new(
            test_env::offline_session(),
            Arc::new(RecordingHooks::default()),
        );
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
