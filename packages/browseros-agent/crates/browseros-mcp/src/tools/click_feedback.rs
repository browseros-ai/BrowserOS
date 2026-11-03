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
        // `trusted` tells an agent whether a page that ignores synthetic events
        // saw the press; `inert` that it went through something, usually a dialog.
        DomFallbackStatus::Dispatched { trusted, inert } => (
            json!({"status":"dispatched", "domFallback":true, "trusted":trusted, "inert":inert}),
            if inert {
                "DOM fallback pressed the element once through an inert cover, usually an open dialog; verify application state."
            } else {
                "DOM fallback dispatched once; verify application state."
            },
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatched_feedback_reports_trust_and_inert_cover() {
        for (trusted, inert) in [(true, false), (true, true), (false, false)] {
            let Some((delivery, notice)) =
                fallback_feedback(Some(DomFallbackStatus::Dispatched { trusted, inert }))
            else {
                panic!("a dispatched fallback has feedback");
            };
            assert_eq!(
                delivery,
                json!({"status":"dispatched", "domFallback":true, "trusted":trusted, "inert":inert})
            );
            assert_eq!(notice.contains("open dialog"), inert);
        }
        assert_eq!(fallback_feedback(None), None);
    }
}
