//! Forwards a failed `run` as a redacted report, when every gate allows it.
//!
//! The observer holds no policy of its own. It gathers the inputs the reporter needs and
//! lets the reporter decide, so there is one place to read to know what can leave.

use crate::{
    analytics::AnalyticsService, clock::now_epoch_ms, services::run_failures::RunFailureReporter,
};
use browseros_mcp_host::{
    mcp::dispatch::{ToolObserverContext, dispatch_error_text},
    seams::DispatchObserver,
};
use futures_util::future::BoxFuture;
use std::sync::Arc;

/// The only tool this reports on. `evaluate` shares the sandbox and the scrubber but runs
/// at a far lower error rate, and would draw down the same daily budget.
const REPORTED_TOOL: &str = "run";

pub struct RunFailureObserver {
    pub run_failures: Arc<RunFailureReporter>,
    pub analytics: Arc<AnalyticsService>,
}

impl DispatchObserver for RunFailureObserver {
    fn name(&self) -> &'static str {
        "run_failure"
    }

    fn observe<'a>(
        &'a self,
        context: ToolObserverContext<'a>,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            if context.call.tool().name != REPORTED_TOOL {
                return Ok(());
            }
            if !context.result.is_error || context.cancelled {
                return Ok(());
            }

            // Cheap refusal before touching consent state or the database.
            if !self.run_failures.is_configured() {
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

            let consent = self.analytics.get_state().await.consent;
            self.run_failures
                .report(consent, script, &error, context.duration_ms, now_epoch_ms())
                .await;
            Ok(())
        })
    }
}
