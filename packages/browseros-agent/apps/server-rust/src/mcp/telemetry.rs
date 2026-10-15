//! `tool_executed` for every catalog tool the shared host dispatches: TS's
//! `observeToolTelemetry` (the observer on `dispatchBrowserTool`), plugged into the host's
//! `DispatchObserver` seam. `Metrics` only counts this event into the 30-minute usage
//! rollup, so what must match TS is the tool name, the success flag and the `source`:
//! the lease's engine (`chat` | `acp`) or `mcp` for an external client without a lease.

use super::leases::LeaseScope;
use crate::telemetry::metrics::Metrics;
use browseros_mcp_host::{mcp::dispatch::ToolObserverContext, seams::DispatchObserver};
use futures_util::future::BoxFuture;
use serde_json::json;

pub(super) struct ToolTelemetry(pub Metrics);

impl DispatchObserver for ToolTelemetry {
    fn name(&self) -> &'static str {
        "tool-telemetry"
    }

    fn observe<'a>(
        &'a self,
        context: ToolObserverContext<'a>,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        let source = context
            .call
            .caller
            .as_ref()
            .and_then(|caller| caller.host_data::<LeaseScope>())
            .map_or("mcp", |scope| scope.source().as_str());
        // TS: `success: error === undefined && !result?.isError`; a cancelled dispatch is the
        // thrown abort.
        self.0.capture(
            "tool_executed",
            json!({
                "tool_name": context.call.tool().name,
                "source": source,
                "duration_ms": context.duration_ms,
                "success": !context.cancelled && !context.result.is_error,
            }),
        );
        Box::pin(async { Ok(()) })
    }
}
