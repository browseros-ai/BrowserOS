//! Working overlay: the "neo is working on this page" layer drawn over a tab neo acts on.
//!
//! All overlay state lives in the page: `assets/agent-overlay.js` keeps one layer per document
//! under `Symbol.for('browseros.agentOverlay')`, holds the person's input with a shield while neo
//! works, and disposes itself when the server stops sending beats. This module holds no state;
//! every call is one bounded evaluation in BrowserOS's isolated world (`PageJs::isolated_only`),
//! so the page can neither see the controls nor forge a take over, and a failure never fails
//! the action it decorates.
//!
//! neo's own input arrives as trusted events too, so the shield would block it. Every input
//! neo sends is wrapped in [`yield_input`] / [`resume_input`], which lower the shield for just
//! that action.

use crate::{CoreError, PageJs, ProtocolSession, timeouts};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    future::Future,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::time::timeout;

/// Name of the CDP binding the overlay calls for take over, hand back and reach in. It is
/// added with `executionContextName` set to the isolated world, so page scripts never see it.
pub const BINDING: &str = "__browserosOverlay";

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Process-wide rollout switch, set once at startup by the host. Off skips every overlay
/// evaluation, including the per-input yield.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

#[must_use]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Working,
    Between,
    Person,
    Finished,
    Quiet,
}

/// The element neo is about to act on, in the top document's viewport CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OverlayState {
    pub phase: Phase,
    pub label: String,
    pub rect: Option<Rect>,
    /// Set by the server once it has handled a hand back, so the page leaves its local
    /// "you have control" state. Without it a held phase never overrides a local take over.
    pub resume: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    Button,
    Esc,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum OverlayEvent {
    ReachIn,
    TakeOver { via: Via },
    HandBack,
    LetFinish,
}

impl OverlayEvent {
    /// Parses a binding payload or a drained queue entry; anything else is ignored.
    #[must_use]
    pub fn parse(payload: &str) -> Option<Self> {
        serde_json::from_str(payload).ok()
    }
}

/// What the page answered. `status` is the page's phase after the call, `person` when a local
/// take over is waiting for the server, or `absent` / `skipped` when nothing is drawn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverlayReply {
    pub status: String,
    pub events: Vec<OverlayEvent>,
}

impl OverlayReply {
    fn from_value(value: &Value) -> Self {
        let events = value["events"]
            .as_array()
            .map(|events| {
                events
                    .iter()
                    .filter_map(|event| serde_json::from_value(event.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();
        Self {
            status: value["status"].as_str().unwrap_or("skipped").to_string(),
            events,
        }
    }
}

fn template() -> &'static str {
    static TEMPLATE: OnceLock<String> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        include_str!("assets/agent-overlay.js").replace(
            "__BROWSEROS_OVERLAY_MARK__",
            include_str!("assets/neo-mark.svg").trim(),
        )
    })
}

async fn run(session: &ProtocolSession, command: Value) -> OverlayReply {
    let expression = template().replace("__BROWSEROS_OVERLAY_CMD__", &command.to_string());
    match timeout(
        timeouts::OVERLAY_DEADLINE,
        PageJs::isolated_only(session).eval::<Value>(&expression),
    )
    .await
    {
        Ok(Ok(value)) => OverlayReply::from_value(&value),
        Ok(Err(error)) => {
            tracing::debug!(%error, "working overlay call failed");
            OverlayReply::default()
        }
        Err(_) => {
            tracing::debug!("working overlay call deadline elapsed");
            OverlayReply::default()
        }
    }
}

/// Draws or updates the overlay. Mounts it on first use.
pub async fn show(session: &ProtocolSession, state: &OverlayState) -> OverlayReply {
    let mut command = serde_json::to_value(state).unwrap_or_else(|_| json!({}));
    command["op"] = json!("show");
    command["expiryMs"] = json!(timeouts::OVERLAY_EXPIRY.as_millis());
    command["finishMs"] = json!(timeouts::OVERLAY_FINISHED.as_millis());
    run(session, command).await
}

/// Keeps a drawn overlay alive and drains the events it queued. Never mounts one.
pub async fn beat(session: &ProtocolSession) -> OverlayReply {
    run(
        session,
        json!({ "op": "beat", "expiryMs": timeouts::OVERLAY_EXPIRY.as_millis() }),
    )
    .await
}

pub async fn dispose(session: &ProtocolSession) {
    let _ = run(session, json!({ "op": "dispose" })).await;
}

/// Exposes [`BINDING`] to the overlay's world. Needs `Runtime.enable` on the session for the
/// `Runtime.bindingCalled` events to arrive.
pub async fn install_binding(session: &ProtocolSession) -> Result<(), CoreError> {
    let _: Value = session
        .send(
            "Runtime.addBinding",
            json!({ "name": BINDING, "executionContextName": crate::page_js::WORLD_NAME }),
        )
        .await?;
    Ok(())
}

/// Whether neo's input may go ahead on this page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Yield {
    /// No overlay is drawn, or it could not be reached: send input as usual.
    Absent,
    /// The shield is down until [`resume_input`] with this id, or the page lets it lapse.
    Yielded(u64),
    /// The person took over on the page: neo must not send input.
    Person,
}

const API: &str = "window[Symbol.for('browseros.agentOverlay')]";

async fn call<T: serde::de::DeserializeOwned>(
    session: &ProtocolSession,
    expression: &str,
) -> Option<T> {
    timeout(
        timeouts::OVERLAY_EFFECT_DEADLINE,
        PageJs::isolated_only(session).eval::<Option<T>>(expression),
    )
    .await
    .ok()?
    .ok()?
}

/// Lowers the shield for neo's next input. A page that does not answer reads as no overlay,
/// so neo is never blocked by a shield it cannot lower; the page's own safety timer raises
/// it again.
pub async fn yield_input(session: &ProtocolSession) -> Yield {
    let expiry = timeouts::OVERLAY_YIELD_EXPIRY.as_millis();
    let expression = format!(
        "(()=>{{try{{const o={API};return o?o.yieldInput({expiry}):'absent';}}catch(_){{return 'absent';}}}})()"
    );
    match call::<Value>(session, &expression).await {
        Some(Value::Number(id)) => id.as_u64().map_or(Yield::Absent, Yield::Yielded),
        Some(Value::String(status)) if status == "person" => Yield::Person,
        _ => Yield::Absent,
    }
}

pub async fn resume_input(session: &ProtocolSession, id: u64) {
    let expression = format!(
        "(()=>{{try{{const o={API};return !!(o&&o.resumeInput({id}));}}catch(_){{return false;}}}})()"
    );
    let _ = call::<bool>(session, &expression).await;
}

/// Runs one input action with the shield lowered, and raises it again whatever the outcome.
/// Refuses when the person has taken the page over.
pub async fn with_yield<T, F>(session: &ProtocolSession, action: F) -> Result<T, CoreError>
where
    F: Future<Output = Result<T, CoreError>>,
{
    match yield_input(session).await {
        Yield::Person => Err(CoreError::PersonControl),
        Yield::Absent => action.await,
        Yield::Yielded(id) => {
            let result = action.await;
            resume_input(session, id).await;
            result
        }
    }
}

/// Hides the overlay in the page's top document before a server screenshot, so the agent
/// never sees it. True when an overlay was hidden; a failure reports false.
pub async fn hide_for_capture(session: &ProtocolSession) -> bool {
    if !enabled() {
        return false;
    }
    let expression = format!(
        "(()=>{{try{{const o={API};return !!(o&&o.hide());}}catch(_){{return false;}}}})()"
    );
    call::<bool>(session, &expression).await.unwrap_or(false)
}

/// Undoes `hide_for_capture`. Best effort.
pub async fn restore_after_capture(session: &ProtocolSession) {
    let expression = format!(
        "(()=>{{try{{const o={API};return !!(o&&o.reveal());}}catch(_){{return false;}}}})()"
    );
    let _ = call::<bool>(session, &expression).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{TestCall, TestConnection};

    fn expression(call: &TestCall) -> &str {
        call.params["expression"].as_str().unwrap_or_default()
    }

    /// Replies for isolated evaluations on a root session, which looks its top frame up each time.
    fn isolated(values: &[Value]) -> Vec<(&'static str, Value)> {
        let mut replies = Vec::new();
        for value in values {
            replies.push((
                "Page.getFrameTree",
                json!({ "frameTree": { "frame": { "id": "main" } } }),
            ));
            replies.push((
                "Page.createIsolatedWorld",
                json!({ "executionContextId": 7 }),
            ));
            replies.push(("Runtime.evaluate", json!({ "result": { "value": value } })));
        }
        replies
    }

    #[test]
    fn the_asset_has_every_placeholder_filled_but_the_command() {
        let script = template();
        assert!(!script.contains("__BROWSEROS_OVERLAY_MARK__"));
        assert!(script.contains("__BROWSEROS_OVERLAY_CMD__"));
        assert!(script.contains("<svg"));
        assert!(script.contains("Phosphor Icons"));
    }

    #[tokio::test]
    async fn show_sends_the_state_to_the_isolated_world_and_reads_back_events()
    -> Result<(), Box<dyn std::error::Error>> {
        let connection = TestConnection::new(isolated(&[json!({
            "status": "shown",
            "events": [{ "type": "take-over", "via": "esc", "at": 1 }, { "type": "bogus" }],
        })]));
        let session = ProtocolSession::root(connection.clone());
        let reply = show(
            &session,
            &OverlayState {
                phase: Phase::Working,
                label: "Clicking \"Add to cart\"".into(),
                rect: Some(Rect {
                    x: 1.0,
                    y: 2.0,
                    width: 30.0,
                    height: 40.0,
                }),
                resume: false,
            },
        )
        .await;
        assert_eq!(reply.status, "shown");
        assert_eq!(reply.events, vec![OverlayEvent::TakeOver { via: Via::Esc }]);
        let calls = connection.calls()?;
        let evaluate = calls
            .iter()
            .find(|call| call.method == "Runtime.evaluate")
            .ok_or("missing evaluate")?;
        assert_eq!(evaluate.params["contextId"], 7);
        let script = expression(evaluate);
        assert!(script.contains("\"op\":\"show\""));
        assert!(script.contains("\"phase\":\"working\""));
        assert!(script.contains("\"width\":30.0"));
        assert!(!script.contains("__BROWSEROS_OVERLAY_CMD__"));
        Ok(())
    }

    #[tokio::test]
    async fn a_failed_call_reads_as_nothing_drawn() {
        let session = ProtocolSession::root(TestConnection::new([]));
        assert_eq!(beat(&session).await, OverlayReply::default());
    }

    #[tokio::test]
    async fn input_is_wrapped_in_a_yield_and_a_resume_even_when_it_fails()
    -> Result<(), Box<dyn std::error::Error>> {
        let connection = TestConnection::new(isolated(&[json!(7), json!(true)]));
        let session = ProtocolSession::root(connection.clone());
        let result: Result<(), CoreError> =
            with_yield(&session, async { Err(CoreError::CrossFrameDrag) }).await;
        assert_eq!(result, Err(CoreError::CrossFrameDrag));
        let scripts: Vec<String> = connection
            .calls()?
            .iter()
            .filter(|call| call.method == "Runtime.evaluate")
            .map(|call| expression(call).to_string())
            .collect();
        assert_eq!(scripts.len(), 2);
        assert!(scripts[0].contains("yieldInput(30000)"));
        assert!(scripts[1].contains("resumeInput(7)"));
        Ok(())
    }

    #[tokio::test]
    async fn a_person_in_control_refuses_input_and_an_unreachable_page_allows_it()
    -> Result<(), Box<dyn std::error::Error>> {
        let connection = TestConnection::new(isolated(&[json!("person")]));
        let session = ProtocolSession::root(connection);
        let mut sent = false;
        let refused = with_yield(&session, async {
            sent = true;
            Ok(())
        })
        .await;
        assert_eq!(refused, Err(CoreError::PersonControl));
        assert!(!sent);

        let unreachable = ProtocolSession::root(TestConnection::new([]));
        assert_eq!(yield_input(&unreachable).await, Yield::Absent);
        Ok(())
    }

    #[tokio::test]
    async fn capture_hiding_is_skipped_while_the_switch_is_off()
    -> Result<(), Box<dyn std::error::Error>> {
        let connection = TestConnection::new([]);
        let session = ProtocolSession::root(connection.clone());
        // Nothing in this test binary turns the switch on.
        assert!(!hide_for_capture(&session).await);
        assert!(connection.calls()?.is_empty());
        Ok(())
    }

    #[test]
    fn binding_payloads_parse_into_events() {
        assert_eq!(
            OverlayEvent::parse(r#"{"type":"take-over","via":"button","at":5}"#),
            Some(OverlayEvent::TakeOver { via: Via::Button })
        );
        assert_eq!(
            OverlayEvent::parse(r#"{"type":"hand-back"}"#),
            Some(OverlayEvent::HandBack)
        );
        assert_eq!(OverlayEvent::parse("not json"), None);
    }
}
