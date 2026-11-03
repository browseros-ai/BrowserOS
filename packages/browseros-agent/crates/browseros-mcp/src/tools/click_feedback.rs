//! Shared MCP feedback for a click's delivery path, including modal/picker
//! interruptions. Handoff is reported as uncertain until the core receives its ack.

use browseros_core::input::DomFallbackStatus;
use serde_json::{Value, json};

pub(super) fn fallback_feedback(
    status: Option<DomFallbackStatus>,
) -> Option<(Value, &'static str)> {
    Some(match status? {
        DomFallbackStatus::Started => (
            json!({"status":"unknown", "domFallback":true, "retryable":false}),
            "DOM fallback may have been sent; inspect application state before another attempt. Do not automatically repeat this click.",
        ),
        DomFallbackStatus::Dispatched => (
            json!({"status":"dispatched", "domFallback":true}),
            "DOM fallback dispatched once; verify application state.",
        ),
    })
}
