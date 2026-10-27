//! The one path BrowserOS takes to run JavaScript in a page.
//!
//! Every script goes through [`PageJs`], and its constructor names the world the script runs in:
//!
//! - [`PageJs::isolated`] is for scripts that only touch the DOM: settle probes, the agent
//!   pointer, element geometry, screenshot overlays. They run in BrowserOS's isolated world, which
//!   shares the page's DOM but not its JavaScript, so the page cannot see their globals or patch
//!   the built-ins they call.
//! - [`PageJs::page_world`] is for scripts that need the page's own JavaScript: the agent's
//!   `evaluate`, site globals, handlers the page assigned. They always run in the page's main world.
//!
//! An isolated script must behave the same in either world. With the rollout switch
//! ([`set_isolated_world_enabled`]) off every script runs in the page world, and with it on a
//! single call still falls back there when Chromium cannot give its frame a world or its node
//! lives in another document. `tests/page_js_guard.rs` fails on page JavaScript sent elsewhere.
//!
//! ```text
//! eval(expr)                          call(node, fn, args)
//!   isolated: Page.createIsolatedWorld  isolated: Page.createIsolatedWorld
//!             Runtime.evaluate            DOM.resolveNode {executionContextId}
//!               {contextId}               Runtime.callFunctionOn, Runtime.releaseObject
//!   page world: Runtime.evaluate        page world: the same calls without a context
//! ```

use crate::{CoreError, FrameId, ProtocolSession, SessionId};
use browseros_cdp::CdpError;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    future::Future,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// Chromium reuses an isolated world by name, so every script in a document shares this one and
/// state a script leaves there (the pointer, the settle counters) lasts as long as the document.
/// An empty name would create a fresh world on every call.
const WORLD_NAME: &str = "browseros";

fn no_isolated_world() -> CoreError {
    CoreError::Message("the page has no isolated world for BrowserOS's check".to_string())
}

/// Returned by a guarded isolated call whose node is not in the document of the world it was
/// resolved into. The call is then repeated in the page world. See [`guard_document`].
const OTHER_DOCUMENT: &str = "__browserosOtherDocument";

/// Top-frame ids are a property of a session (a page target's main frame id is its target id),
/// so they are looked up once per session. The cap only bounds a long-lived process.
const TOP_FRAME_CACHE_LIMIT: usize = 4096;

static ISOLATED_WORLD_ENABLED: AtomicBool = AtomicBool::new(false);

/// Process-wide rollout switch for [`PageJs::isolated`], set once at startup by the host. Off
/// runs every script in the page world, which is how BrowserOS behaved before isolated worlds.
pub fn set_isolated_world_enabled(enabled: bool) {
    ISOLATED_WORLD_ENABLED.store(enabled, Ordering::Relaxed);
}

#[must_use]
pub fn isolated_world_enabled() -> bool {
    ISOLATED_WORLD_ENABLED.load(Ordering::Relaxed)
}

/// A script's result as CDP reports it: `value` when the result serialized, and `description`
/// for what does not (functions, DOM nodes, symbols) or is `undefined`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteValue {
    pub value: Option<Value>,
    pub description: Option<String>,
}

/// Runs JavaScript in one page session. Cheap to build per call; see the module docs for the
/// contract each constructor carries.
#[derive(Debug, Clone, Copy)]
pub struct PageJs<'a> {
    session: &'a ProtocolSession,
    frame: Option<&'a FrameId>,
    isolated: bool,
    /// Never falls back to the page world: see [`PageJs::isolated_only`].
    strict: bool,
    user_gesture: bool,
    timeout: Option<Duration>,
}

impl<'a> PageJs<'a> {
    /// DOM-only scripts. Runs in BrowserOS's isolated world while the rollout switch is on, and in
    /// the page world otherwise or when the frame cannot get one.
    #[must_use]
    pub fn isolated(session: &'a ProtocolSession) -> Self {
        Self::new(session, isolated_world_enabled())
    }

    /// Scripts whose answer the page must not be able to forge, such as BrowserOS's own check of
    /// what the page reports. Always the isolated world, whatever the rollout switch; a frame that
    /// cannot get one is an error, never the page world.
    #[must_use]
    pub fn isolated_only(session: &'a ProtocolSession) -> Self {
        Self {
            strict: true,
            ..Self::new(session, true)
        }
    }

    /// Scripts that need the page's own JavaScript. Always the page's main world.
    #[must_use]
    pub fn page_world(session: &'a ProtocolSession) -> Self {
        Self::new(session, false)
    }

    fn new(session: &'a ProtocolSession, isolated: bool) -> Self {
        Self {
            session,
            frame: None,
            isolated,
            strict: false,
            user_gesture: false,
            timeout: None,
        }
    }

    /// Targets a frame of this session other than its top document, such as a same-process
    /// iframe. Without it, scripts run in the session's top document.
    #[must_use]
    pub fn in_frame(mut self, frame: &'a FrameId) -> Self {
        self.frame = Some(frame);
        self
    }

    /// Runs as if the user had activated the page, so gesture-gated APIs work.
    #[must_use]
    pub fn user_gesture(mut self) -> Self {
        self.user_gesture = true;
        self
    }

    /// Chromium terminates an [`eval`](Self::eval) that runs longer. `call` has no such limit,
    /// and neither has a page-world `eval` in a same-process child frame: that frame's main-world
    /// context has no id to evaluate in, so it runs as a call on the frame's document.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Evaluates an expression, awaiting a returned promise, and decodes its value. A missing
    /// value (`undefined`) decodes as JSON `null`; a thrown exception is
    /// [`CoreError::PageScript`].
    pub async fn eval<T: DeserializeOwned>(self, expression: &str) -> Result<T, CoreError> {
        decode(self.eval_value(expression).await?.value)
    }

    /// [`eval`](Self::eval) for callers that report results: keeps CDP's description of values
    /// that do not serialize.
    pub async fn eval_value(self, expression: &str) -> Result<RemoteValue, CoreError> {
        if self.isolated
            && let Some(result) = self
                .with_isolated_context(|context| self.evaluate(Some(context), expression))
                .await
        {
            return result;
        }
        if self.strict {
            return Err(no_isolated_world());
        }
        match self.frame {
            Some(frame) if self.top_frame().await? != *frame => {
                // A child frame's main-world context has no id we can name, but its document's
                // wrapper does belong to it: a function called on the document runs there.
                let document = self.frame_document(frame).await?;
                let function = format!("function(){{return (\n{expression}\n);}}");
                self.call_node(None, document, &function, &[])
                    .await?
                    .ok_or_else(element_not_found)
            }
            _ => self.evaluate(None, expression).await,
        }
    }

    /// Calls `function` with the node as `this` and `args` as its arguments, awaiting a returned
    /// promise, and decodes its value.
    pub async fn call<T: DeserializeOwned>(
        self,
        backend_node_id: i64,
        function: &str,
        args: &[Value],
    ) -> Result<T, CoreError> {
        if self.isolated {
            // The node might sit in another document than the frame whose world resolves it (a
            // same-process child frame), where `document` and `window` would be the wrong ones.
            let guarded = guard_document(function);
            let function = guarded.as_str();
            let attempt = self
                .with_isolated_context(|context| {
                    self.call_node(Some(context), backend_node_id, function, args)
                })
                .await;
            match attempt {
                Some(Ok(Some(remote))) if !is_other_document(&remote) => {
                    return decode(remote.value);
                }
                Some(Err(error)) => return Err(error),
                Some(Ok(_)) | None if self.strict => return Err(no_isolated_world()),
                Some(Ok(_)) | None => {}
            }
        }
        let remote = self
            .call_node(None, backend_node_id, function, args)
            .await?
            .ok_or_else(element_not_found)?;
        decode(remote.value)
    }

    /// Calls `function` on an object this call already holds. It runs in the world that object
    /// belongs to, whichever constructor built this `PageJs`.
    async fn call_on_object(
        &self,
        object_id: &str,
        function: &str,
        args: &[Value],
    ) -> Result<RemoteValue, CoreError> {
        let mut params = json!({
            "functionDeclaration": function,
            "objectId": object_id,
            "awaitPromise": true,
            "returnByValue": true,
        });
        if !args.is_empty() {
            params["arguments"] = args.iter().map(|value| json!({ "value": value })).collect();
        }
        if self.user_gesture {
            params["userGesture"] = json!(true);
        }
        let reply: Reply = self.session.send("Runtime.callFunctionOn", params).await?;
        reply.into_result()
    }

    async fn evaluate(
        &self,
        context: Option<i64>,
        expression: &str,
    ) -> Result<RemoteValue, CoreError> {
        let mut params = json!({
            "expression": expression,
            "awaitPromise": true,
            "returnByValue": true,
        });
        if let Some(context) = context {
            params["contextId"] = json!(context);
        }
        if self.user_gesture {
            params["userGesture"] = json!(true);
        }
        if let Some(timeout) = self.timeout {
            params["timeout"] = json!(u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX));
        }
        let reply: Reply = self.session.send("Runtime.evaluate", params).await?;
        reply.into_result()
    }

    /// `None` when the node has no wrapper in `context`'s world, which Chromium reports for a node
    /// from a document that world cannot reach.
    async fn call_node(
        &self,
        context: Option<i64>,
        backend_node_id: i64,
        function: &str,
        args: &[Value],
    ) -> Result<Option<RemoteValue>, CoreError> {
        let mut params = json!({ "backendNodeId": backend_node_id });
        if let Some(context) = context {
            params["executionContextId"] = json!(context);
        }
        let resolved: Resolved = self.session.send("DOM.resolveNode", params).await?;
        let Some(object_id) = resolved.object.and_then(|object| object.object_id) else {
            return Ok(None);
        };
        let wrapper = NodeWrapper {
            session: self.session,
            object_id: Some(object_id),
        };
        let result = self.call_on_object(wrapper.id(), function, args).await;
        wrapper.release().await;
        result.map(Some)
    }

    /// Runs `attempt` in this frame's isolated world. `None` means the page world should run the
    /// script instead: Chromium refused the world, or its context kept going stale (the document
    /// was replaced twice in a row). A context goes stale when its document navigates away
    /// between creating the world and using it.
    async fn with_isolated_context<T, F, Fut>(&self, attempt: F) -> Option<Result<T, CoreError>>
    where
        F: Fn(i64) -> Fut,
        Fut: Future<Output = Result<T, CoreError>>,
    {
        for _ in 0..2 {
            let context = match self.isolated_context().await {
                Ok(context) => context,
                Err(error) if is_protocol_refusal(&error) => {
                    tracing::debug!(%error, "isolated world unavailable; running in the page world");
                    return None;
                }
                Err(error) => return Some(Err(error)),
            };
            match attempt(context).await {
                Err(error) if is_stale_context(&error) => {}
                result => return Some(result),
            }
        }
        tracing::debug!("isolated world context kept going stale; running in the page world");
        None
    }

    async fn isolated_context(&self) -> Result<i64, CoreError> {
        let frame = match self.frame {
            Some(frame) => frame.clone(),
            None => self.top_frame().await?,
        };
        let world: CreatedWorld = self
            .session
            .send(
                "Page.createIsolatedWorld",
                json!({ "frameId": frame.as_str(), "worldName": WORLD_NAME }),
            )
            .await?;
        Ok(world.execution_context_id)
    }

    async fn top_frame(&self) -> Result<FrameId, CoreError> {
        let key = self.session.session_id();
        if let Some(frame) = key.and_then(cached_top_frame) {
            return Ok(frame);
        }
        let tree: FrameTree = self.session.send("Page.getFrameTree", json!({})).await?;
        let frame = FrameId(tree.frame_tree.frame.id);
        if let Some(key) = key {
            remember_top_frame(key, &frame);
        }
        Ok(frame)
    }

    async fn frame_document(&self, frame: &FrameId) -> Result<i64, CoreError> {
        let owner: FrameOwner = self
            .session
            .send("DOM.getFrameOwner", json!({ "frameId": frame.as_str() }))
            .await?;
        let described: Described = self
            .session
            .send(
                "DOM.describeNode",
                json!({ "backendNodeId": owner.backend_node_id }),
            )
            .await?;
        described
            .node
            .content_document
            .and_then(|document| document.backend_node_id)
            .ok_or_else(|| {
                CoreError::Message(format!("Frame {frame} has no document in this session"))
            })
    }
}

/// A node wrapper resolved for one call. It would otherwise live until its document goes away, so
/// it is released after the call, and also when a caller's timeout drops the call mid-script, which
/// skips the awaited release.
struct NodeWrapper<'a> {
    session: &'a ProtocolSession,
    object_id: Option<String>,
}

impl NodeWrapper<'_> {
    fn id(&self) -> &str {
        self.object_id.as_deref().unwrap_or_default()
    }

    async fn release(mut self) {
        if let Some(object_id) = self.object_id.take() {
            release_object(self.session, object_id).await;
        }
    }
}

impl Drop for NodeWrapper<'_> {
    fn drop(&mut self) {
        let Some(object_id) = self.object_id.take() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let session = self.session.clone();
        drop(handle.spawn(async move { release_object(&session, object_id).await }));
    }
}

async fn release_object(session: &ProtocolSession, object_id: String) {
    let _ = session
        .send::<_, Value>("Runtime.releaseObject", json!({ "objectId": object_id }))
        .await;
}

/// Wraps `function` so it reports [`OTHER_DOCUMENT`] instead of running when its node belongs to
/// another document than the one whose world is calling it. The trailing newline keeps a line
/// comment at the end of `function` from swallowing the call.
fn guard_document(function: &str) -> String {
    format!(
        "function(...args){{if((this.ownerDocument||this)!==document)return {{{OTHER_DOCUMENT}:true}};return (\n{function}\n).apply(this,args);}}"
    )
}

fn is_other_document(remote: &RemoteValue) -> bool {
    remote
        .value
        .as_ref()
        .and_then(|value| value.get(OTHER_DOCUMENT))
        .is_some()
}

/// Chromium answered but would not do it (no such frame, a frame mid-navigation, a node outside
/// the world's reach). Transport loss and target admission errors are not refusals: the page
/// world would fail the same way, so they surface instead.
fn is_protocol_refusal(error: &CoreError) -> bool {
    matches!(error, CoreError::Cdp(CdpError::Protocol { .. }))
}

fn is_stale_context(error: &CoreError) -> bool {
    match error {
        CoreError::Cdp(CdpError::Protocol { message, .. }) => {
            message.contains("Cannot find context with specified id")
                || message.contains("does not belong to the document")
                || message.contains("Execution context was destroyed")
        }
        _ => false,
    }
}

fn element_not_found() -> CoreError {
    CoreError::Message("Element not found in DOM. Take a new snapshot.".to_string())
}

fn decode<T: DeserializeOwned>(value: Option<Value>) -> Result<T, CoreError> {
    serde_json::from_value(value.unwrap_or(Value::Null)).map_err(|error| {
        CoreError::Message(format!("page script returned an unexpected value: {error}"))
    })
}

fn top_frames() -> &'static Mutex<HashMap<SessionId, FrameId>> {
    static TOP_FRAMES: OnceLock<Mutex<HashMap<SessionId, FrameId>>> = OnceLock::new();
    TOP_FRAMES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_top_frame(session: &SessionId) -> Option<FrameId> {
    top_frames().lock().ok()?.get(session).cloned()
}

fn remember_top_frame(session: &SessionId, frame: &FrameId) {
    if let Ok(mut frames) = top_frames().lock() {
        if frames.len() >= TOP_FRAME_CACHE_LIMIT {
            frames.clear();
        }
        frames.insert(session.clone(), frame.clone());
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reply {
    result: Option<RemoteValue>,
    exception_details: Option<ExceptionDetails>,
}

impl Reply {
    fn into_result(self) -> Result<RemoteValue, CoreError> {
        if let Some(details) = self.exception_details {
            return Err(CoreError::PageScript(
                details
                    .exception
                    .and_then(|exception| exception.description)
                    .unwrap_or(details.text),
            ));
        }
        Ok(self.result.unwrap_or_default())
    }
}

#[derive(Deserialize)]
struct ExceptionDetails {
    #[serde(default)]
    text: String,
    exception: Option<ExceptionObject>,
}

#[derive(Deserialize)]
struct ExceptionObject {
    description: Option<String>,
}

#[derive(Deserialize)]
struct Resolved {
    object: Option<ResolvedObject>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolvedObject {
    object_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatedWorld {
    execution_context_id: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameTree {
    frame_tree: FrameTreeNode,
}

#[derive(Deserialize)]
struct FrameTreeNode {
    frame: FrameInfo,
}

#[derive(Deserialize)]
struct FrameInfo {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameOwner {
    backend_node_id: i64,
}

#[derive(Deserialize)]
struct Described {
    node: DescribedNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescribedNode {
    content_document: Option<DescribedDocument>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescribedDocument {
    backend_node_id: Option<i64>,
}

#[cfg(test)]
mod tests;
