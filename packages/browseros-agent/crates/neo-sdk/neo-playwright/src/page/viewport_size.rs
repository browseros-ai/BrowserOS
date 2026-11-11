//! The page's persistent metrics override, shared with core so a takeover can restore it.

use browseros_core::viewport::{Size, ViewportRequest};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::op_scope::OpScope;
use crate::target::PageRef;
use crate::{ErrorCode, Playwright, PlaywrightError};

/// `page.setViewportSize` uses no Playwright timeout; the run still bounds the operation.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ViewportSizeInput {
    page: PageRef,
    viewport_size: Dimensions,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Dimensions {
    width: i64,
    height: i64,
}

pub(crate) async fn set(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ViewportSizeInput,
) -> Result<Value, PlaywrightError> {
    let size = Size {
        width: input.viewport_size.width,
        height: input.viewport_size.height,
    };
    let request = ViewportRequest::playwright(size)
        .map_err(|error| scope.fail(ErrorCode::InvalidInput, &error))?;
    let admitted = scope.admit(pw.browser(), input.page.tab()).await?;
    // This is a tab write: the door's takeover recheck precedes the first CDP effect. Core
    // owns the request and the viewport lock, so a fit saves it and restores it unchanged.
    let _sent = scope.send_effect()?;
    scope
        .view()
        .set_viewport(
            admitted.core_page.clone(),
            request,
            &CancellationToken::new(),
        )
        .await
        .map_err(|error| scope.core_error(&error))?;
    Ok(json!({}))
}
