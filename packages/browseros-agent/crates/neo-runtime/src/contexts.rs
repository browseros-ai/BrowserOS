//! Neo: the frame each main-world JavaScript context of a session belongs to, from the
//! execution-context events Chromium sends once `Runtime.enable` is on. Neo's main world never
//! needs it for its own calls (`main_world.rs` reaches a frame's world by its document node);
//! `Runtime.bindingCalled` and `Runtime.consoleAPICalled` name only an `executionContextId`.
//!
//! One map per producer, fed in socket order by the task that reads that producer's events:
//! `PageEvents` (net's watcher) for a tab's session and its frame hosts, `bindings.rs` for the
//! sessions `init_scripts.rs` auto-attached.

use std::collections::HashMap;
use std::sync::Mutex;

use browseros_cdp::{CdpEvent, SessionId};

/// `(session, executionContextId)` → frame id, for each session's default (main world) contexts.
#[derive(Debug, Default)]
pub(crate) struct ContextFrames(Mutex<HashMap<(String, i64), String>>);

impl ContextFrames {
    /// Applies one event of `Runtime.executionContextCreated` (a default context only),
    /// `executionContextDestroyed` or `executionContextsCleared`; any other event is ignored.
    pub(crate) fn apply(&self, event: &CdpEvent) {
        let Some(session) = event.session_id.as_ref().map(SessionId::as_str) else {
            return;
        };
        let Ok(mut contexts) = self.0.lock() else {
            return;
        };
        match event.method.as_str() {
            "Runtime.executionContextCreated" => {
                let context = &event.params["context"];
                let aux = &context["auxData"];
                if let (Some(id), Some(frame), Some(true)) = (
                    context["id"].as_i64(),
                    aux["frameId"].as_str(),
                    aux["isDefault"].as_bool(),
                ) {
                    contexts.insert((session.to_string(), id), frame.to_string());
                }
            }
            "Runtime.executionContextDestroyed" => {
                if let Some(id) = event.params["executionContextId"].as_i64() {
                    contexts.remove(&(session.to_string(), id));
                }
            }
            "Runtime.executionContextsCleared" => {
                contexts.retain(|(owner, _), _| owner != session);
            }
            _ => {}
        }
    }

    /// The frame of `context` on `session`, when its creation was applied.
    pub(crate) fn frame_of(&self, session: &str, context: i64) -> Option<String> {
        self.0
            .lock()
            .ok()?
            .get(&(session.to_string(), context))
            .cloned()
    }

    /// Forgets every context of `session`, which detached.
    pub(crate) fn forget(&self, session: &str) {
        if let Ok(mut contexts) = self.0.lock() {
            contexts.retain(|(owner, _), _| owner != session);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn event(method: &str, session: &str, params: Value) -> CdpEvent {
        CdpEvent {
            method: method.to_string(),
            params,
            session_id: Some(SessionId::from(session)),
        }
    }

    #[test]
    fn a_default_context_names_its_frame_until_destroyed_or_cleared() {
        let contexts = ContextFrames::default();
        let created = |session: &str, id: i64, frame: &str, default: bool| {
            event(
                "Runtime.executionContextCreated",
                session,
                json!({"context": {"id": id, "auxData": {"frameId": frame, "isDefault": default}}}),
            )
        };
        contexts.apply(&created("tab", 1, "main", true));
        contexts.apply(&created("tab", 2, "child", true));
        contexts.apply(&created("tab", 3, "main", false));
        contexts.apply(&created("iframe", 1, "oopif", true));
        assert_eq!(contexts.frame_of("tab", 1).as_deref(), Some("main"));
        assert_eq!(
            contexts.frame_of("tab", 3),
            None,
            "an isolated world is no main world"
        );
        assert_eq!(contexts.frame_of("iframe", 1).as_deref(), Some("oopif"));
        contexts.apply(&event(
            "Runtime.executionContextDestroyed",
            "tab",
            json!({"executionContextId": 2}),
        ));
        assert_eq!(contexts.frame_of("tab", 2), None);
        contexts.apply(&event("Runtime.executionContextsCleared", "tab", json!({})));
        assert_eq!(contexts.frame_of("tab", 1), None);
        assert_eq!(contexts.frame_of("iframe", 1).as_deref(), Some("oopif"));
        contexts.forget("iframe");
        assert_eq!(contexts.frame_of("iframe", 1), None);
    }
}
