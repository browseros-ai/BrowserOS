//! The page's persistent metrics override, shared with core so a takeover can restore it.

use browseros_core::viewport::{Size, ViewportRequest};
use serde::Deserialize;
use ts_rs::TS;

use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, PAGE};
use crate::target::PageRef;
use crate::wire::{Nothing, options};
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

pub(crate) async fn set(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ViewportSizeInput,
) -> Result<Nothing, PlaywrightError> {
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use crate::Op;
    use crate::op_output::to_output;
    use crate::op_scope::OpScope;
    use crate::target::CallHeader;
    use crate::test_env::TestEnv;

    #[tokio::test]
    async fn viewport_void_receipt_leaves_the_route_as_null() -> Result<(), Box<dyn Error>> {
        let env = TestEnv::new();
        let op = Op::from_name("page.setViewportSize").ok_or("viewport op exists")?;
        let scope = OpScope::new(&env, op, CallHeader::default());
        assert_eq!(serde_json::to_string(&to_output(&scope, &())?)?, "null");
        Ok(())
    }
}
