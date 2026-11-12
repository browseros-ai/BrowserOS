//! Playwright screenshot geometry: resolve and scroll elements, trim page clips, then convert
//! CSS viewport coordinates into the document coordinates CDP captures. No viewport emulation
//! is needed for an element larger than the viewport; CDP captures its offscreen region.

use std::borrow::Cow;
use std::time::Duration;

use browseros_core::screenshot::Viewport;
use serde::Deserialize;
use serde_json::{Value, json};
use ts_rs::TS;

use crate::action::ElementRunner;
use crate::op_scope::OpScope;
use crate::page::{OpPage, world_call};
use crate::probe::{ProbeFailure, Read, Rect};
use crate::target::{Strictness, Target};
use crate::world::Place;
use crate::{ErrorCode, Playwright, PlaywrightError, read, resolve};

const METRICS: &str = r"function(requireBody) {
  const body = document.body, root = document.documentElement;
  if (!root || (requireBody && !body)) return null;
  return { width: innerWidth, height: innerHeight, scrollX, scrollY, deviceScaleFactor: devicePixelRatio,
    fullWidth: Math.max(body?.scrollWidth || 0, root.scrollWidth, body?.offsetWidth || 0, root.offsetWidth, body?.clientWidth || 0, root.clientWidth),
    fullHeight: Math.max(body?.scrollHeight || 0, root.scrollHeight, body?.offsetHeight || 0, root.offsetHeight, body?.clientHeight || 0, root.clientHeight) };
}";

/// The CSS document dimensions and scroll offset sampled after element scrolling or styles.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Metrics {
    width: f64,
    height: f64,
    scroll_x: f64,
    scroll_y: f64,
    device_scale_factor: f64,
    full_width: f64,
    full_height: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LayoutMetrics {
    visual_viewport: VisualViewport,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VisualViewport {
    page_x: f64,
    page_y: f64,
    scale: f64,
}

/// How image pixels relate to CSS pixels, independently of the captured rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub(super) enum Scale {
    Css,
    #[default]
    Device,
}

/// The geometry-related part of screenshot options, borrowed from the decoded request.
pub(super) struct GeometryOptions<'a> {
    pub(super) target: Option<&'a Target>,
    pub(super) strict: Strictness,
    pub(super) full_page: bool,
    pub(super) clip: Option<Rect>,
    pub(super) scale: Scale,
}

impl Default for GeometryOptions<'_> {
    fn default() -> Self {
        Self {
            target: None,
            strict: Strictness::Strict,
            full_page: false,
            clip: None,
            scale: Scale::Device,
        }
    }
}

/// The normalized document clip and whether Chromium must paint outside the viewport.
pub(super) struct ScreenshotGeometry {
    pub(super) clip: Viewport,
    pub(super) beyond_viewport: bool,
    /// Encoded pixels per CSS document pixel, including device density and clip scaling.
    pub(super) raster_scale: f64,
}

/// CSS document bounds before image-density scaling is applied.
struct DocumentRect {
    rect: Rect,
    fits_viewport: bool,
    page_scale: f64,
}

impl ScreenshotGeometry {
    /// Binds a locator to one node before preparation. This ensures its owning frame exists
    /// when styles are applied, and scrolling and bounds cannot silently target replacements.
    pub(super) async fn bind<'a>(
        pw: &Playwright,
        scope: &OpScope<'_>,
        target: Option<&'a Target>,
        strict: Strictness,
    ) -> Result<Option<Cow<'a, Target>>, PlaywrightError> {
        match target {
            None => Ok(None),
            Some(target @ Target::Element(_)) => Ok(Some(Cow::Borrowed(target))),
            Some(Target::Locator(locator)) => {
                let value = read::wait_for_selector(
                    pw,
                    scope,
                    locator,
                    strict,
                    read::SelectorState::Attached,
                    read::Returning::Handle,
                )
                .await?;
                let handle: Target = serde_json::from_value(value)
                    .map_err(|error| scope.fail(ErrorCode::Internal, &error.to_string()))?;
                Ok(Some(Cow::Owned(handle)))
            }
        }
    }

    /// Implements Screenshotter's page/element rectangles and Chromium's viewport-to-document
    /// translation. Element actionability is shared with scrollIntoViewIfNeeded, with the
    /// additional visible requirement Playwright uses for screenshots.
    pub(super) async fn measure(
        pw: &Playwright,
        page: &OpPage<'_>,
        scope: &OpScope<'_>,
        options: GeometryOptions<'_>,
    ) -> Result<Self, PlaywrightError> {
        let GeometryOptions {
            target,
            strict,
            full_page,
            clip,
            scale,
        } = options;
        if let Some(target) = target {
            ElementRunner::new(scope, page, target, strict)
                .scroll_for_screenshot()
                .await?;
        }
        let metrics_target = target.map_or(MetricsTarget::Page { full_page }, |_| {
            MetricsTarget::Element
        });
        let metrics = metrics(page, scope, metrics_target).await?;
        // Metrics can cross a page navigation. Read the bound node afterwards, so a stale
        // handle fails instead of combining an old element rectangle with a new document.
        let element = if let Some(target) = target {
            let reading =
                read::read_attached(pw, scope, target, strict, &Read::BoundingBox).await?;
            let value = read::for_screenshot(scope, &reading.path, reading.value).await?;
            let rect: Option<Rect> = serde_json::from_value(value)
                .map_err(|error| scope.fail(ErrorCode::Internal, &error.to_string()))?;
            Some(rect.ok_or_else(|| {
                scope.fail(
                    ErrorCode::InvalidInput,
                    "Node is either not visible or not an HTMLElement",
                )
            })?)
        } else {
            None
        };
        let layout: LayoutMetrics = page
            .root_session()
            .send("Page.getLayoutMetrics", json!({}))
            .await
            .map_err(|error| scope.core_error(&error))?;
        let document = if let Some(mut rect) = element {
            let fits = rect.width <= metrics.width && rect.height <= metrics.height;
            rect.x += metrics.scroll_x;
            rect.y += metrics.scroll_y;
            DocumentRect {
                rect: enclosing(rect),
                fits_viewport: fits,
                page_scale: 1.0,
            }
        } else if full_page {
            let rect = Rect {
                x: 0.0,
                y: 0.0,
                width: metrics.full_width,
                height: metrics.full_height,
            };
            let fits = rect.width <= metrics.width && rect.height <= metrics.height;
            DocumentRect {
                rect: trim(scope, clip, rect)?,
                fits_viewport: fits,
                page_scale: 1.0,
            }
        } else {
            let rect = trim(
                scope,
                clip,
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: metrics.width,
                    height: metrics.height,
                },
            )?;
            let viewport = layout.visual_viewport;
            DocumentRect {
                rect: Rect {
                    x: viewport.page_x + rect.x,
                    y: viewport.page_y + rect.y,
                    width: (rect.width / viewport.scale + 1e-3).floor(),
                    height: (rect.height / viewport.scale + 1e-3).floor(),
                },
                fits_viewport: true,
                page_scale: viewport.scale,
            }
        };
        let image_scale = match scale {
            Scale::Device => document.page_scale,
            Scale::Css => document.page_scale / metrics.device_scale_factor,
        };
        let rect = document.rect;
        Ok(Self {
            clip: Viewport {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                scale: image_scale,
            },
            beyond_viewport: !document.fits_viewport,
            raster_scale: image_scale * metrics.device_scale_factor,
        })
    }
}

fn trim(scope: &OpScope<'_>, clip: Option<Rect>, size: Rect) -> Result<Rect, PlaywrightError> {
    let Some(clip) = clip else {
        return Ok(size);
    };
    let x = clip.x.clamp(0.0, size.width);
    let y = clip.y.clamp(0.0, size.height);
    let width = (clip.x + clip.width).clamp(0.0, size.width) - x;
    let height = (clip.y + clip.height).clamp(0.0, size.height) - y;
    if width <= 0.0 || height <= 0.0 {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            "Clipped area is either empty or outside the resulting image",
        ));
    }
    Ok(Rect {
        x,
        y,
        width,
        height,
    })
}

fn enclosing(rect: Rect) -> Rect {
    let x = (rect.x + 1e-3).floor();
    let y = (rect.y + 1e-3).floor();
    Rect {
        x,
        y,
        width: (rect.x + rect.width - 1e-3).ceil() - x,
        height: (rect.y + rect.height - 1e-3).ceil() - y,
    }
}

/// Page images can use a replacement document; an element capture is bound to one node and
/// must restart locator resolution if its document disappears during geometry measurement.
#[derive(Clone, Copy)]
enum MetricsTarget {
    Page { full_page: bool },
    Element,
}

/// Wait for page dimensions across navigation, like upstream's utility-world poller.
/// An element's loss is returned to the screenshot loop rather than retargeting its capture.
async fn metrics(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    target: MetricsTarget,
) -> Result<Metrics, PlaywrightError> {
    let require_body = matches!(target, MetricsTarget::Page { full_page: true });
    let endpoint = page.main_endpoint();
    loop {
        let measured = world_call::with_world(page, &endpoint, &Place::Main, |world| async move {
            world
                .call_for_screenshot(METRICS, &json!([require_body]))
                .await
                .map_err(ProbeFailure::from_core)
        })
        .await;
        let delay = match measured {
            Ok(value) if !value.is_null() => {
                return serde_json::from_value(value)
                    .map_err(|error| scope.fail(ErrorCode::Internal, &error.to_string()));
            }
            Ok(_) => Duration::from_millis(16),
            Err(ProbeFailure::Gone(_)) => {
                // A world can disappear when its tab closes as well. This context-independent
                // read keeps a closed session or lost transport fatal, even with timeout: 0.
                let _: Value = page
                    .root_session()
                    .send("Page.getLayoutMetrics", json!({}))
                    .await
                    .map_err(|error| scope.core_error(&error))?;
                if matches!(target, MetricsTarget::Element) {
                    return Err(scope.fail(
                        ErrorCode::ElementDetached,
                        "Element is not attached to the DOM",
                    ));
                }
                scope.log("document changed while measuring screenshot, retrying".to_owned());
                Duration::from_millis(100)
            }
            Err(failure) => return Err(resolve::failed(scope, failure)),
        };
        scope
            .sleep(delay)
            .await
            .map_err(|_expired| scope.timed_out(ErrorCode::WaitTimeout))?;
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::{Value, json};

    use super::{METRICS, MetricsTarget};
    use crate::op_scope::OpScope;
    use crate::page::OpPage;
    use crate::target::CallHeader;
    use crate::test_support::{ProbeAnswer, ScriptedRun, ScriptedTab, TabScript};
    use crate::{ErrorCode, Op};

    struct ReplacedDocuments {
        reads: AtomicUsize,
        error: &'static str,
        layout_times_out: bool,
    }

    impl TabScript for ReplacedDocuments {
        fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
            ProbeAnswer::unexpected()
        }

        fn outlasts_client(&self, method: &str) -> bool {
            self.layout_times_out && method == "Page.getLayoutMetrics"
        }

        fn main_world(&self, _object: &str, function: &str) -> Option<Result<Value, &'static str>> {
            if function != METRICS {
                return None;
            }
            Some(if self.reads.fetch_add(1, Ordering::SeqCst) < 3 {
                Err(self.error)
            } else {
                Ok(json!({ "result": { "type": "object", "value": {
                    "width": 500, "height": 400, "scrollX": 0, "scrollY": 0,
                    "deviceScaleFactor": 1, "fullWidth": 500, "fullHeight": 900,
                } } }))
            })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn document_measurement_survives_repeated_navigation_but_preserves_fatal_errors()
    -> Result<(), Box<dyn Error>> {
        for (message, layout_times_out, expected, reads) in [
            ("Execution context was destroyed.", false, None, 4),
            (
                "No target with given id found",
                false,
                Some(ErrorCode::TargetClosed),
                1,
            ),
            ("capture denied", false, Some(ErrorCode::Internal), 1),
            (
                "Execution context was destroyed.",
                true,
                Some(ErrorCode::BrowserDisconnected),
                2,
            ),
        ] {
            let tab = ScriptedTab::new(ReplacedDocuments {
                reads: AtomicUsize::new(0),
                error: message,
                layout_times_out,
            });
            let run = ScriptedRun::over(&tab).await?;
            let op = Op::from_name("page.screenshot").ok_or("screenshot is registered")?;
            let scope = OpScope::new(&run.env, op, CallHeader::default());
            let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
            let result =
                super::metrics(&page, &scope, MetricsTarget::Page { full_page: true }).await;
            if let Some(code) = expected {
                let error = result.err().ok_or("fatal measurement must fail")?;
                assert_eq!(error.error_code(), Some(code), "{error}");
                assert_eq!(
                    tab.script().reads.load(Ordering::SeqCst),
                    reads,
                    "fatal errors stop measurement, including a failed session read after the generic context retry"
                );
            } else {
                let measured = result?;
                assert_eq!(
                    (measured.width, measured.full_height),
                    (500.0, 900.0),
                    "measurement returns dimensions from the surviving document"
                );
                assert_eq!(
                    tab.script().reads.load(Ordering::SeqCst),
                    reads,
                    "three document replacements precede the successful fourth read"
                );
            }
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn element_measurement_does_not_continue_into_a_replacement_document()
    -> Result<(), Box<dyn Error>> {
        let tab = ScriptedTab::new(ReplacedDocuments {
            reads: AtomicUsize::new(0),
            error: "Execution context was destroyed.",
            layout_times_out: false,
        });
        let run = ScriptedRun::over(&tab).await?;
        let op = Op::from_name("page.screenshot").ok_or("screenshot is registered")?;
        let scope = OpScope::new(&run.env, op, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let error = super::metrics(&page, &scope, MetricsTarget::Element)
            .await
            .err()
            .ok_or("an element's replaced document must fail its measurement")?;
        assert_eq!(
            error.error_code(),
            Some(ErrorCode::ElementDetached),
            "locator captures must re-resolve after document loss; handles must stay detached"
        );
        assert_eq!(
            tab.script().reads.load(Ordering::SeqCst),
            2,
            "element measurement must not enter the page-level retry in a replacement document"
        );
        Ok(())
    }
}
