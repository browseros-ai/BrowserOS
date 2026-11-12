//! The page's persistent metrics override, shared with core so a takeover can restore it.

use browseros_core::viewport::{Size, ViewportRequest};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::wire::options;

use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, PAGE};
use crate::target::PageRef;
use crate::{ErrorCode, Playwright, PlaywrightError};

/// A tab write, so the dispatcher's takeover lock guards resizing just like navigation.
pub(crate) const SPEC: OpSpec = OpSpec::write(
    "page.setViewportSize",
    PAGE,
    "set this tab's viewport and screen size until the tab closes",
);

/// `page.setViewportSize` uses no Playwright timeout; the run still bounds the operation.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ViewportSizeInput {
    page: PageRef,
    viewport_size: Dimensions,
}

#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
struct Dimensions {
    width: i64,
    height: i64,
}

options!(ViewportSizeInput);

/// The existing viewport write receipt; capture and frame state remain owned by the page.
#[derive(Debug, Serialize, TS)]
pub(crate) struct ViewportChanged {}

pub(crate) async fn set(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ViewportSizeInput,
) -> Result<ViewportChanged, PlaywrightError> {
    let size = Size {
        width: input.viewport_size.width,
        height: input.viewport_size.height,
    };
    let request = ViewportRequest::playwright(size)
        .map_err(|error| scope.fail(ErrorCode::InvalidInput, &error))?;
    let admitted = scope.admit(pw.browser(), input.page.tab()).await?;
    let change = scope
        .view()
        .prepare_playwright_viewport(admitted.core_page.clone(), request)
        .await
        .map_err(|error| scope.core_error(&error))?;
    // Admission and the viewport queue may wait while the owner takes the tab. Check after
    // those waits, immediately before CDP; dropping an unauthorized change sends nothing.
    let _sent = scope.send_effect()?;
    change
        .apply()
        .await
        .map_err(|error| scope.core_error(&error))?;
    Ok(ViewportChanged {})
}
