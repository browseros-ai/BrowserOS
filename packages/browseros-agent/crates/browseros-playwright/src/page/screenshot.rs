//! `page.screenshot()`, after the host allows the capture.
//!
//! The capture goes through core's screenshot path, which serializes captures per session and
//! hides the agent pointer for the shot. The host's privacy layer is asked inside that path,
//! immediately before `Page.captureScreenshot`, because a page can turn private while the op
//! waits for admission, the capture lock or the pointer.

use browseros_core::screenshot::{ScreenshotCaptureOptions, ScreenshotFormat, Viewport};
use serde::Deserialize;
use serde_json::Value;

use crate::op_output::{Bytes, to_output};
use crate::op_scope::OpScope;
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

/// Captures the page as `input` asks, once the host allows it.
pub(crate) async fn capture(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ScreenshotInput,
) -> Result<Value, PlaywrightError> {
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
    let page = input.page.checked(pw.browser(), scope).await?;
    // Core asks the host inside its capture lock, after admission and the pointer hide, with
    // no await before `Page.captureScreenshot`. Asked here instead, the verdict would be stale
    // by the time the pixels are read.
    let shot = scope
        .view()
        .screenshot_checked(page.clone(), options, || pw.hooks().capture_allowed(page))
        .await
        .map_err(|error| scope.core_error(&error))??;
    to_output(scope, &Bytes::from_base64(shot.data))
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;

    use browseros_core::sensitive_fields::{FieldValues, JournalEntry};
    use browseros_core::testing::{TestCall, TestConnection};
    use browseros_core::{BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, PageId};
    use browseros_trace::ErrorOwner;
    use serde_json::json;

    use crate::test_env::TestEnv;
    use crate::{Op, Playwright, PlaywrightHooks, Refusal, TypedBy};

    /// Core's pointer hide, the last thing it does before it captures.
    fn hides_pointer(call: &TestCall) -> bool {
        call.method == "Runtime.evaluate"
            && call.params["expression"]
                .as_str()
                .is_some_and(|expression| expression.contains("p.hide()"))
    }

    /// A host whose page turns private once core has hidden the agent pointer, as if the owner
    /// marked it private while the op waited for the capture.
    struct PrivateOncePrepared {
        connection: Arc<TestConnection>,
    }

    impl PlaywrightHooks for PrivateOncePrepared {
        fn field_entered(&self, _page: PageId, _entry: JournalEntry, _by: TypedBy) {}

        fn values_swept(&self, _page: PageId, _values: &FieldValues) {}

        fn capture_allowed(&self, _page: PageId) -> Result<(), Refusal> {
            let prepared = self
                .connection
                .calls()
                .is_ok_and(|calls| calls.iter().any(hides_pointer));
            if prepared {
                Err(Refusal {
                    owner: ErrorOwner::Host,
                    code: "page_private".to_owned(),
                    message: "page.screenshot: the owner marked this page private".to_owned(),
                    host_code: None,
                })
            } else {
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn tests_that_a_page_turning_private_during_preparation_is_never_captured()
    -> Result<(), Box<dyn Error>> {
        let connection = TestConnection::new([]);
        connection.always(
            "Browser.getTabs",
            json!({ "tabs": [{ "tabId": 7, "targetId": "target-7", "windowId": 1,
                "url": "https://example.test/", "title": "Account", "isActive": true,
                "isLoading": false, "loadProgress": 1.0, "isPinned": false, "isHidden": false }] }),
        )?;
        for setup in [
            "Page.enable",
            "DOM.enable",
            "Runtime.enable",
            "Accessibility.enable",
        ] {
            connection.always(setup, json!({}))?;
        }
        connection.always("Runtime.evaluate", json!({ "result": { "value": true } }))?;
        connection.always("Page.captureScreenshot", json!({ "data": "cG5n" }))?;
        let cdp: Arc<dyn CdpConnection> = Arc::clone(&connection) as Arc<dyn CdpConnection>;
        let browser = BrowserSession::new(cdp, BrowserSessionHooks::default());
        let hooks = PrivateOncePrepared {
            connection: Arc::clone(&connection),
        };
        let playwright = Playwright::new(Arc::clone(&browser), Arc::new(hooks));
        let page = browser
            .pages
            .list()
            .await?
            .first()
            .ok_or("the tab is listed")?
            .page_id
            .clone();
        let env = TestEnv::over(BrowserView::unbound(Arc::clone(&browser), None));
        let op = Op::from_name("page.screenshot").ok_or("page.screenshot is an op")?;
        let error = playwright
            .invoke(
                &env,
                op,
                json!({ "page": { "$neo": "page", "id": page.0, "tab": 7 } }),
            )
            .await
            .err()
            .ok_or("a page that turned private is not captured")?;
        assert_eq!(
            (error.owner(), error.code()),
            (ErrorOwner::Host, "page_private"),
            "the host's refusal reaches the script unchanged: {error}"
        );
        let calls = connection.calls()?;
        assert!(calls.iter().any(hides_pointer), "core prepared the capture");
        assert!(
            calls
                .iter()
                .all(|call| call.method != "Page.captureScreenshot"),
            "no capture is sent once the host refuses: {calls:?}"
        );
        Ok(())
    }
}
