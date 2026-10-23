//! Agent pointer: Cua's animated cursor drawn on the live page before agent input.
//!
//! All pointer state lives in the page: `assets/agent-pointer.js` keeps one player
//! per document under `Symbol.for('browseros.agentPointer')` and removes it when
//! idle. This module holds no state; every call is one bounded `Runtime.evaluate`
//! through the caller's guarded session, and a drawing failure never fails input.

use crate::{ProtocolSession, timeouts};
use serde_json::{Value, json};
use tokio::time::timeout;

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
    let reply = timeout(
        timeouts::POINTER_EFFECT_DEADLINE,
        session.send::<_, Value>(
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true, "awaitPromise": false }),
        ),
    )
    .await
    .ok()?
    .ok()?;
    reply.get("result")?.get("value")?.as_bool()
}

#[cfg(test)]
mod tests {
    use super::{hide_for_capture, restore_after_capture};
    use crate::{ProtocolSession, test_support::TestConnection};
    use serde_json::json;

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
