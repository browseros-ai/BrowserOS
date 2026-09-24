//! Isolated-world ownership. Handles belong to a connection epoch, target session, and
//! frame document; neither stable frame ids nor reconnects preserve execution contexts.

use crate::{CoreError, FrameId, ProtocolSession, SessionId};
use browseros_cdp::CdpEvent;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use tokio::sync::{Mutex, broadcast};

const INJECTED: &str = include_str!("assets/injected_script.js");
type Key = (u64, Option<SessionId>, FrameId);

#[derive(Clone)]
pub(super) struct World {
    pub context: i64,
    pub object: String,
}

/// No detached listener task: drain lifecycle events before each cache access. A lagged
/// receiver invalidates everything, and a navigation racing a command gets one retry.
#[derive(Default)]
pub(super) struct Worlds {
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<Key, World>,
    prepared: HashSet<(u64, Option<SessionId>)>,
    events: Option<broadcast::Receiver<CdpEvent>>,
    sequence: u64,
}

impl Cache {
    fn drain(&mut self, epoch: u64) {
        self.entries.retain(|(e, _, _), _| *e == epoch);
        self.prepared.retain(|(e, _)| *e == epoch);
        let Some(events) = &mut self.events else {
            return;
        };
        loop {
            match events.try_recv() {
                Ok(event) => match event.method.as_str() {
                    "Runtime.executionContextsCleared" => {
                        self.entries
                            .retain(|(_, session, _), _| *session != event.session_id);
                    }
                    "Runtime.executionContextDestroyed" => {
                        let context = event.params["executionContextId"].as_i64();
                        self.entries.retain(|(_, session, _), world| {
                            *session != event.session_id || Some(world.context) != context
                        });
                    }
                    "Target.detachedFromTarget" => {
                        let id = event.params["sessionId"].as_str().map(SessionId::from);
                        self.entries.retain(|(_, session, _), _| *session != id);
                        self.prepared.retain(|(_, session)| *session != id);
                    }
                    _ => {}
                },
                Err(broadcast::error::TryRecvError::Lagged(_)) => self.entries.clear(),
                Err(_) => break,
            }
        }
    }
}

impl Worlds {
    pub async fn get(
        &self,
        session: &ProtocolSession,
        frame: &FrameId,
    ) -> Result<World, CoreError> {
        let epoch = session.connection().connection_epoch();
        let key = (epoch, session.session_id().cloned(), frame.clone());
        // Serialize bootstrap, including focus emulation: concurrent callers cannot inject
        // duplicate engines or lose context-destruction events while one is being created.
        let mut cache = self.cache.lock().await;
        if cache.events.is_none() {
            cache.events = Some(session.connection().events());
        }
        cache.drain(epoch);
        if let Some(world) = cache.entries.get(&key) {
            return Ok(world.clone());
        }
        let session_key = (epoch, session.session_id().cloned());
        if !cache.prepared.contains(&session_key) {
            session.send_value("Runtime.enable", json!({})).await?;
            // Background tabs otherwise suspend RAF, including Playwright's stability
            // check. This changes page-visible focus/visibility, not the selected tab.
            session
                .send_value(
                    "Emulation.setFocusEmulationEnabled",
                    json!({"enabled": true}),
                )
                .await?;
            cache.prepared.insert(session_key);
        }
        let created = session
            .send_value(
                "Page.createIsolatedWorld",
                json!({
                    "frameId": frame.0, "worldName": "__browseros_pw", "grantUniveralAccess": false
                }),
            )
            .await?;
        let context = created["executionContextId"]
            .as_i64()
            .ok_or_else(|| CoreError::from("Isolated world has no executionContextId"))?;
        cache.sequence += 1;
        let options = json!({
            "isUnderTest": false, "sdkLanguage": "javascript", "frameSeq": cache.sequence,
            "testIdAttributeName": "data-testid", "stableRafCount": 1,
            "browserName": "chromium", "shouldPrependErrorPrefix": false,
            "isUtilityWorld": true, "customEngines": []
        });
        // 1.63's export is a getter returning the class. Keep the module stub local;
        // no page global or binding is required, and the main world cannot see it.
        let expression = format!(
            "(() => {{ const module = {{}};\n{INJECTED}\nreturn new (module.exports.InjectedScript())(globalThis, {options}); }})()"
        );
        let result = runtime_result(
            session
                .send_value(
                    "Runtime.evaluate",
                    json!({
                        "expression": expression, "contextId": context, "returnByValue": false
                    }),
                )
                .await?,
        )?;
        let world = World {
            context,
            object: object_id(&result)?,
        };
        // Drain again before inserting: queued events from a prior document must not
        // evict this new engine at the next access. A concurrent navigation after the
        // evaluate is still detected by the next drain or the stale-handle retry.
        cache.drain(epoch);
        cache.entries.insert(key, world.clone());
        Ok(world)
    }

    pub async fn invalidate(&self, session: &ProtocolSession, frame: &FrameId) {
        self.cache.lock().await.entries.remove(&(
            session.connection().connection_epoch(),
            session.session_id().cloned(),
            frame.clone(),
        ));
    }

    /// Arguments here are JSON values, never handles from a previous world. This is
    /// what makes replay after context destruction safe for selector/matcher reads.
    pub async fn call(
        &self,
        session: &ProtocolSession,
        frame: &FrameId,
        function: &str,
        args: Vec<Value>,
        by_value: bool,
    ) -> Result<Value, CoreError> {
        for attempt in 0..2 {
            let result = async {
                let world = self.get(session, frame).await?;
                runtime_result(session.send_value("Runtime.callFunctionOn", json!({
                    "objectId": world.object, "functionDeclaration": function,
                    "arguments": args.iter().map(|v| json!({"value": v})).collect::<Vec<_>>(),
                    "returnByValue": by_value, "awaitPromise": true
                })).await?)
            }
            .await;
            match result {
                Err(error) if attempt == 0 && stale(&error) => {
                    self.invalidate(session, frame).await
                }
                result => return result,
            }
        }
        Err(CoreError::DocumentChanged)
    }
}

pub(super) fn stale(error: &CoreError) -> bool {
    let message = error.to_string();
    message.contains("Cannot find context")
        || message.contains("Execution context was destroyed")
        || message.contains("Cannot find object with given id")
}

/// Runtime exceptions arrive in successful protocol replies, not as CdpError.
pub(super) fn runtime_result(reply: Value) -> Result<Value, CoreError> {
    if let Some(exception) = reply.get("exceptionDetails") {
        let message = exception["exception"]["description"]
            .as_str()
            .or_else(|| exception["text"].as_str())
            .unwrap_or("JavaScript evaluation failed");
        return Err(CoreError::Message(message.to_owned()));
    }
    reply
        .get("result")
        .cloned()
        .ok_or_else(|| CoreError::from("Runtime reply has no result"))
}

pub(super) fn object_id(remote: &Value) -> Result<String, CoreError> {
    remote["objectId"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| CoreError::from("Runtime reply has no objectId"))
}

pub(super) async fn release(session: &ProtocolSession, object: &str) {
    let _ = session
        .send_value("Runtime.releaseObject", json!({"objectId": object}))
        .await;
}
