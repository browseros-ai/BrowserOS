//! Agent pointer: Cua's animated cursor drawn on the live page before agent input.
//!
//! All pointer state lives in the page: `assets/agent-pointer.js` keeps one player
//! per document under `Symbol.for('browseros.agentPointer')` and removes it when
//! idle. This module holds no state; every call is one bounded isolated-world evaluation
//! (`PageJs::isolated`) through the caller's guarded session, and a drawing failure never
//! fails input. Move, ripple, hide and show must share that world: a pointer drawn in one world
//! is invisible to a hide sent to another, and would then show up in screenshots.

use crate::{PageJs, ProtocolSession, input::Point, timeouts};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::OnceLock;
use tokio::time::timeout;

/// Enables page-local pointer animation for one browser execution view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pointer {
    _private: (),
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PointerAction {
    Click,
    Hover,
    Focus,
    Fill,
}

impl Pointer {
    /// Only an explicit page-side skip preserves the original input sequence.
    /// A drawing error may follow partial travel, so the caller must recheck.
    pub(crate) async fn move_to(
        &self,
        session: &ProtocolSession,
        action: PointerAction,
        point: Point,
    ) -> bool {
        static TEMPLATE: OnceLock<String> = OnceLock::new();
        let template = TEMPLATE.get_or_init(|| {
            include_str!("assets/agent-pointer.js").replace(
                "__BROWSEROS_POINTER_CURSOR__",
                include_str!("assets/agent-cursor.js"),
            )
        });
        let command = json!({
            "op": "move",
            "kind": action,
            "x": point.x,
            "y": point.y,
            "travelMs": timeouts::POINTER_TRAVEL.as_millis(),
            "timeoutMs": (timeouts::POINTER_TRAVEL * 2).as_millis(),
            "idleMs": timeouts::POINTER_IDLE.as_millis(),
        });
        let expression = template.replace("__BROWSEROS_POINTER_CMD__", &command.to_string());
        match timeout(
            timeouts::POINTER_MOVE_DEADLINE,
            PageJs::isolated(session).eval::<Value>(&expression),
        )
        .await
        {
            Ok(Ok(reply)) => match reply["status"].as_str() {
                Some("skipped") => {
                    tracing::debug!("agent pointer move skipped");
                    false
                }
                Some("timeout") => {
                    tracing::debug!("agent pointer page move timed out");
                    true
                }
                _ => true,
            },
            Ok(Err(_)) => {
                tracing::debug!("agent pointer move failed");
                true
            }
            Err(_) => {
                tracing::debug!("agent pointer move deadline elapsed");
                true
            }
        }
    }

    pub(crate) async fn click_effect(&self, session: &ProtocolSession, point: Point) {
        let expression = format!(
            "(()=>{{try{{const p=window[Symbol.for('browseros.agentPointer')];return !!(p&&p.click({},{}));}}catch(_){{return false;}}}})()",
            point.x, point.y
        );
        let _ = evaluate_bool(session, &expression).await;
    }
}

const HIDE_JS: &str = "(()=>{try{const p=window[Symbol.for('browseros.agentPointer')];return !!(p&&p.hide());}catch(_){return false;}})()";
const SHOW_JS: &str = "(()=>{try{const p=window[Symbol.for('browseros.agentPointer')];if(p)p.show();}catch(_){}return true;})()";

/// Hides the pointer in the page's top document before a server screenshot, so
/// the agent never sees its own cursor. True when a pointer was hidden; a
/// failure or timeout reports false and the capture proceeds.
pub async fn hide_for_capture(session: &ProtocolSession) -> bool {
    evaluate_bool(session, HIDE_JS).await.unwrap_or(false)
}

/// Undoes `hide_for_capture`. Best effort.
pub async fn restore_after_capture(session: &ProtocolSession) {
    let _ = evaluate_bool(session, SHOW_JS).await;
}

async fn evaluate_bool(session: &ProtocolSession, expression: &str) -> Option<bool> {
    timeout(
        timeouts::POINTER_EFFECT_DEADLINE,
        PageJs::isolated(session).eval::<Option<bool>>(expression),
    )
    .await
    .ok()?
    .ok()?
}

#[cfg(test)]
mod tests {
    use super::{Pointer, PointerAction, hide_for_capture, restore_after_capture};
    use crate::{ProtocolSession, input::Point, test_support::TestConnection, timeouts};
    use serde_json::{Value, json};

    #[test]
    fn tests_that_the_embedded_cursor_keeps_its_binding_and_license_without_module_statements() {
        let script = include_str!("assets/agent-cursor.js");
        assert!(script.contains("var BrowserOSAgentCursor"));
        assert!(script.contains("MIT License"));
        assert!(script.contains("Copyright (c) 2025 Cua AI, Inc."));
        assert!(script.lines().all(|line| {
            let statement = line.trim_start();
            !statement.starts_with("export {") && !statement.starts_with("import ")
        }));
    }

    #[tokio::test]
    async fn tests_that_moves_send_the_cursor_and_timed_command()
    -> Result<(), Box<dyn std::error::Error>> {
        let connection = TestConnection::new((0..4).map(|_| {
            (
                "Runtime.evaluate",
                json!({ "result": { "value": { "status": "arrived" } } }),
            )
        }));
        let session = ProtocolSession::root(connection.clone());
        let pointer = Pointer::default();
        for action in [
            PointerAction::Click,
            PointerAction::Hover,
            PointerAction::Focus,
            PointerAction::Fill,
        ] {
            assert!(
                pointer
                    .move_to(&session, action, Point { x: 12.5, y: 35.0 })
                    .await
            );
        }
        let calls = connection.calls()?;
        assert_eq!(calls.len(), 4);
        for (call, kind) in calls.iter().zip(["click", "hover", "focus", "fill"]) {
            assert_eq!(call.method, "Runtime.evaluate");
            assert_eq!(call.params["awaitPromise"], true);
            assert_eq!(call.params["returnByValue"], true);
            let expression = call.params["expression"]
                .as_str()
                .ok_or("missing expression")?;
            let command = json!({
                "op": "move", "kind": kind, "x": 12.5, "y": 35.0,
                "travelMs": timeouts::POINTER_TRAVEL.as_millis(),
                "timeoutMs": (timeouts::POINTER_TRAVEL * 2).as_millis(),
                "idleMs": timeouts::POINTER_IDLE.as_millis(),
            });
            assert!(expression.contains(&serde_json::to_string(&command)?));
            assert!(expression.contains(include_str!("assets/agent-cursor.js")));
            assert!(!expression.contains("__BROWSEROS_POINTER_CURSOR__"));
            assert!(!expression.contains("__BROWSEROS_POINTER_CMD__"));
        }
        Ok(())
    }

    #[tokio::test]
    async fn tests_that_only_an_explicit_skip_disables_the_recheck() {
        let connection = TestConnection::new([
            (
                "Runtime.evaluate",
                json!({ "result": { "value": { "status": "skipped", "reason": "hidden" } } }),
            ),
            (
                "Runtime.evaluate",
                json!({ "result": { "value": { "status": "timeout" } } }),
            ),
            (
                "Runtime.evaluate",
                json!({ "exceptionDetails": { "text": "drawing failed" } }),
            ),
            (
                "Runtime.evaluate",
                json!({ "result": { "value": Value::Null } }),
            ),
        ]);
        let session = ProtocolSession::root(connection);
        let point = Point { x: 2.0, y: 4.0 };
        let pointer = Pointer::default();
        assert!(!pointer.move_to(&session, PointerAction::Click, point).await);
        for _ in 0..4 {
            assert!(pointer.move_to(&session, PointerAction::Click, point).await);
        }
    }

    #[tokio::test]
    async fn tests_that_click_effects_use_a_small_no_wait_expression()
    -> Result<(), Box<dyn std::error::Error>> {
        let connection =
            TestConnection::new([("Runtime.evaluate", json!({ "result": { "value": true } }))]);
        let session = ProtocolSession::root(connection.clone());
        Pointer::default()
            .click_effect(&session, Point { x: 72.5, y: 91.0 })
            .await;
        let calls = connection.calls()?;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].params["returnByValue"], true);
        let expression = calls[0].params["expression"]
            .as_str()
            .ok_or("missing expression")?;
        assert!(expression.contains("Symbol.for('browseros.agentPointer')"));
        assert!(expression.contains(".click(72.5,91)"));
        assert!(expression.len() < 250);
        // A missing/erroring page API is best effort too.
        Pointer::default()
            .click_effect(&session, Point { x: 0.0, y: 0.0 })
            .await;
        Ok(())
    }

    #[tokio::test]
    async fn hide_reports_whether_the_page_hid_a_pointer() -> Result<(), Box<dyn std::error::Error>>
    {
        let connection = TestConnection::new([
            ("Runtime.evaluate", json!({ "result": { "value": true } })),
            ("Runtime.evaluate", json!({ "result": { "value": false } })),
        ]);
        let session = ProtocolSession::root(connection.clone());
        assert!(hide_for_capture(&session).await);
        assert!(!hide_for_capture(&session).await);
        let calls = connection.calls()?;
        assert!(calls.iter().all(|call| {
            call.params["expression"]
                .as_str()
                .is_some_and(|expression| expression.contains("browseros.agentPointer"))
        }));
        Ok(())
    }

    #[tokio::test]
    async fn capture_helpers_never_fail_when_evaluate_fails() {
        // No queued replies: every evaluate errors, which must read as "nothing hidden".
        let session = ProtocolSession::root(TestConnection::new([]));
        assert!(!hide_for_capture(&session).await);
        restore_after_capture(&session).await;
    }
}
