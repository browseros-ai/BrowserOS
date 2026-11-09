//! Neo: the frame each main-world JavaScript context of a session belongs to, from the
//! execution-context events Chromium sends once `Runtime.enable` is on. Neo's main world never
//! needs it for its own calls (`main_world.rs` reaches a frame's world by its document node);
//! `Runtime.bindingCalled` and `Runtime.consoleAPICalled` name only an `executionContextId`.
//!
//! One map per producer, fed in socket order by the task that reads that producer's events:
//! `PageEvents` (net's watcher) for a tab's session and its frame hosts, `bindings.rs` for the
//! sessions `init_scripts.rs` auto-attached.
//!
//! A destroyed context keeps naming its frame: a call made in it can be read after the context
//! went, as when a binding's iframe is removed in the evaluation that called it, and Playwright's
//! binding call keeps that frame as its source. Only the newest [`DEAD_KEPT`] of a map's
//! destroyed contexts are kept.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use browseros_cdp::{CdpEvent, SessionId};

use crate::chromium_bridge::cdp::{self, runtime};

/// Neo: how many destroyed contexts a map keeps naming their frames, oldest dropped first: a
/// session's every navigation destroys a few, and a map lives as long as its producer.
const DEAD_KEPT: usize = 1024;

/// `(session, executionContextId)` → frame id, for each session's default (main world) contexts.
#[derive(Debug, Default)]
pub(crate) struct ContextFrames(Mutex<Contexts>);

#[derive(Debug, Default)]
struct Contexts {
    frames: HashMap<(String, i64), Context>,
    /// Each death, oldest first, by its number: a key created again since is no longer that
    /// death's, and outlives it.
    dead: VecDeque<((String, i64), u64)>,
    deaths: u64,
}

#[derive(Debug, Clone)]
struct Context {
    frame: String,
    /// Chromium's `uniqueId`, which no other process reuses, unlike the numeric id a call names.
    unique: Option<String>,
    /// The number of its death; `None` while the context lives.
    died: Option<u64>,
}

impl Contexts {
    fn kill(&mut self, key: (String, i64)) {
        let Some(context) = self.frames.get_mut(&key) else {
            return;
        };
        if context.died.is_some() {
            return;
        }
        self.deaths += 1;
        context.died = Some(self.deaths);
        self.dead.push_back((key, self.deaths));
        while self.dead.len() > DEAD_KEPT {
            let Some((oldest, died)) = self.dead.pop_front() else {
                break;
            };
            if self
                .frames
                .get(&oldest)
                .is_some_and(|context| context.died == Some(died))
            {
                self.frames.remove(&oldest);
            }
        }
    }
}

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
        if let Some(created) = cdp::decode::<runtime::ExecutionContextCreated>(event) {
            let context = created.context;
            if let (Some(id), Some(frame), Some(true)) = (
                context.id,
                context.aux_data.frame_id,
                context.aux_data.is_default,
            ) {
                contexts.frames.insert(
                    (session.to_string(), id),
                    Context {
                        frame,
                        unique: context.unique_id,
                        died: None,
                    },
                );
            }
        } else if let Some(destroyed) = cdp::decode::<runtime::ExecutionContextDestroyed>(event) {
            if let Some(id) = destroyed.execution_context_id {
                contexts.kill((session.to_string(), id));
            }
        } else if cdp::is::<runtime::ExecutionContextsCleared>(event) {
            let owned: Vec<(String, i64)> = contexts
                .frames
                .iter()
                .filter(|((owner, _), context)| owner == session && context.died.is_none())
                .map(|(key, _)| key.clone())
                .collect();
            for key in owned {
                contexts.kill(key);
            }
        }
    }

    /// The frame of `context` on `session` and its system-unique id, once its creation was
    /// applied, destroyed or not.
    pub(crate) fn context_of(
        &self,
        session: &str,
        context: i64,
    ) -> Option<(String, Option<String>)> {
        self.0
            .lock()
            .ok()?
            .frames
            .get(&(session.to_string(), context))
            .map(|context| (context.frame.clone(), context.unique.clone()))
    }

    /// Takes `from`'s live contexts of `session` that this map has not seen: a reader that starts
    /// on a session after its contexts were created learns them from the producer that saw them.
    pub(crate) fn seed(&self, from: &Self, session: &str) {
        let live: Vec<((String, i64), Context)> = match from.0.lock() {
            Ok(from) => from
                .frames
                .iter()
                .filter(|((owner, _), context)| owner == session && context.died.is_none())
                .map(|(key, context)| (key.clone(), context.clone()))
                .collect(),
            Err(_) => return,
        };
        if let Ok(mut contexts) = self.0.lock() {
            for (key, context) in live {
                contexts.frames.entry(key).or_insert(context);
            }
        }
    }

    /// Forgets every context of `session`, which detached.
    pub(crate) fn forget(&self, session: &str) {
        if let Ok(mut contexts) = self.0.lock() {
            contexts.frames.retain(|(owner, _), _| owner != session);
            contexts.dead.retain(|((owner, _), _)| owner != session);
        }
    }
}

#[cfg(test)]
impl ContextFrames {
    fn frame_of(&self, session: &str, context: i64) -> Option<String> {
        self.context_of(session, context).map(|(frame, _)| frame)
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
    fn a_default_context_names_its_frame_after_it_is_destroyed_or_cleared() {
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
        assert_eq!(
            contexts.frame_of("tab", 2).as_deref(),
            Some("child"),
            "a call made before its context went still finds its frame"
        );
        contexts.apply(&event("Runtime.executionContextsCleared", "tab", json!({})));
        assert_eq!(contexts.frame_of("tab", 1).as_deref(), Some("main"));
        contexts.apply(&created("tab", 1, "next", true));
        assert_eq!(
            contexts.frame_of("tab", 1).as_deref(),
            Some("next"),
            "a context created again names its new frame"
        );
        assert_eq!(contexts.frame_of("iframe", 1).as_deref(), Some("oopif"));
        contexts.forget("iframe");
        assert_eq!(contexts.frame_of("iframe", 1), None);
    }

    #[test]
    fn only_the_newest_destroyed_contexts_are_kept() {
        let contexts = ContextFrames::default();
        let kept = i64::try_from(DEAD_KEPT).expect("a small bound");
        for id in 0..=kept {
            contexts.apply(&event(
                "Runtime.executionContextCreated",
                "tab",
                json!({"context": {"id": id, "auxData": {"frameId": "frame", "isDefault": true}}}),
            ));
            contexts.apply(&event(
                "Runtime.executionContextDestroyed",
                "tab",
                json!({"executionContextId": id}),
            ));
        }
        assert_eq!(contexts.frame_of("tab", 0), None, "the oldest went first");
        assert_eq!(contexts.frame_of("tab", 1).as_deref(), Some("frame"));
        assert_eq!(contexts.frame_of("tab", kept).as_deref(), Some("frame"));
    }
}

#[cfg(test)]
mod unique_tests {
    use serde_json::json;

    use super::*;

    fn created(session: &str, id: i64, frame: &str, unique: &str) -> CdpEvent {
        CdpEvent {
            method: "Runtime.executionContextCreated".to_string(),
            params: json!({"context": {
                "id": id,
                "uniqueId": unique,
                "auxData": {"frameId": frame, "isDefault": true},
            }}),
            session_id: Some(SessionId::from(session)),
        }
    }

    #[test]
    fn a_context_keeps_its_unique_id_and_a_reused_number_takes_the_new_one() {
        let contexts = ContextFrames::default();
        contexts.apply(&created("tab", 1, "main", "old.1"));
        assert_eq!(
            contexts.context_of("tab", 1),
            Some(("main".to_string(), Some("old.1".to_string())))
        );
        // Another process numbers its first context 1 again: only its unique id tells them apart.
        contexts.apply(&CdpEvent {
            method: "Runtime.executionContextsCleared".to_string(),
            params: json!({}),
            session_id: Some(SessionId::from("tab")),
        });
        contexts.apply(&created("tab", 1, "main", "new.1"));
        assert_eq!(
            contexts.context_of("tab", 1),
            Some(("main".to_string(), Some("new.1".to_string())))
        );
    }

    #[test]
    fn a_seed_takes_the_live_contexts_the_map_has_not_seen() {
        let producer = ContextFrames::default();
        producer.apply(&created("tab", 1, "main", "seen.1"));
        producer.apply(&created("tab", 2, "child", "seen.2"));
        producer.apply(&created("other", 1, "elsewhere", "other.1"));
        let reader = ContextFrames::default();
        reader.apply(&created("tab", 2, "child", "own.2"));
        reader.seed(&producer, "tab");
        assert_eq!(
            reader.context_of("tab", 1),
            Some(("main".to_string(), Some("seen.1".to_string())))
        );
        assert_eq!(
            reader.context_of("tab", 2),
            Some(("child".to_string(), Some("own.2".to_string()))),
            "its own reading wins"
        );
        assert_eq!(reader.context_of("other", 1), None, "only that session");
    }
}
