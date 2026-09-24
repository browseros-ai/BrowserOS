//! Forwards a failed `run` or `playwright` as a redacted report, when every gate allows it.
//!
//! The observer holds no policy of its own. It gathers the inputs the reporter needs and
//! lets the reporter decide, so there is one place to read to know what can leave.

use crate::{
    api::mcp::dispatch::{ToolObserverContext, dispatch_error_text},
    clock::now_epoch_ms,
};
use futures_util::future::BoxFuture;

/// The script tools this reports on. `evaluate` shares the sandbox and the scrubber but runs
/// at a far lower error rate, and would draw down the same daily budget.
const REPORTED_TOOLS: &[&str] = &["run", "playwright"];

pub fn apply(context: ToolObserverContext<'_>) -> BoxFuture<'_, anyhow::Result<()>> {
    Box::pin(async move {
        if !REPORTED_TOOLS.contains(&context.call.tool().name) {
            return Ok(());
        }
        if !context.result.is_error || context.cancelled {
            return Ok(());
        }

        let state = &context.call.state;
        // Cheap refusal before touching consent state or the database.
        if !state.run_failures.is_configured() {
            return Ok(());
        }

        let Some(script) = context
            .call
            .raw_args
            .get("code")
            .and_then(serde_json::Value::as_str)
        else {
            return Ok(());
        };
        let Some(error) = dispatch_error_text(context.result) else {
            return Ok(());
        };

        let consent = state.analytics.get_state().await.consent;
        state
            .run_failures
            .report(consent, script, &error, context.duration_ms, now_epoch_ms())
            .await;
        Ok(())
    })
}
