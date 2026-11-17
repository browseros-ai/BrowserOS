//! Exercise the framework's failure response after an adapter has already
//! recorded a DOM handoff. Lost acknowledgements must retain that delivery path.

use super::{harness_ctx, tool_by_name};
use crate::{
    framework::{ToolCtx, ToolExecResult, ToolHandler, ToolResult, execute_tool},
    response::ToolResponse,
};
use browseros_core::CoreError;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

fn lost_ack<'a>(
    args: Value,
    _ctx: &'a ToolCtx,
    response: &'a mut ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        if args["domFallback"] == true {
            response
                .data(json!({"click":{"domFallback":true,"status":"unknown","retryable":false}}));
        }
        Err(CoreError::ClickOutcomeUnknown("lost activation acknowledgement".into()).into())
    })
}

#[tokio::test]
async fn unknown_click_preserves_the_adapters_delivery_path() {
    for dom in [false, true] {
        let (ctx, _connection, page) = harness_ctx().await;
        let mut definition = tool_by_name("act");
        definition.handler = ToolHandler::Browser(lost_ack);
        let result = execute_tool(&definition, json!({"page":page,"domFallback":dom}), &ctx)
            .await
            .unwrap_or_else(|error| panic!("framework failed: {error}"));
        assert!(result.is_error);
        let data = result
            .structured_content
            .unwrap_or_else(|| panic!("missing click feedback"));
        assert_eq!(data["click"]["status"], "unknown");
        assert_eq!(data["click"]["retryable"], false);
        assert_eq!(
            data["click"].get("domFallback"),
            dom.then_some(&json!(true))
        );
    }
}
