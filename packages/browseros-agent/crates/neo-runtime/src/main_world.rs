//! Neo: the only door from Rust into a frame's MAIN world, where Playwright runs every page
//! evaluation (`page.evaluate`, `locator.evaluate`, `JSHandle.evaluate`), as opposed to the
//! utility world (`world.rs`) where every Neo read and action runs. It runs exactly three built
//! functions of neo-runtime-injected, never anything else: `dist/utility.js` builds Playwright's
//! `UtilityScript`, and `dist/evaluate.js` / `dist/json-value.js` call it. The agent's code only
//! ever travels as an argument value of `evaluate.js`.
//!
//! ```text
//!  first call in a document (per session)
//!    World::call(document)                 utility world: the document node, as a handle
//!    DOM.describeNode{objectId}            → backendNodeId, the document's identity
//!    DOM.resolveNode{backendNodeId}        no executionContextId: the frame's main world
//!    Runtime.callFunctionOn{objectId: main document, UTILITY}   → the UtilityScript's object id
//!  every call
//!    Runtime.callFunctionOn{objectId: UtilityScript, EVALUATE | JSON_VALUE,
//!      arguments: [{objectId: UtilityScript}, …], awaitPromise, userGesture, returnByValue}
//!  a commit (the world's document generation moved) → install again on the next call
//!  a call whose UtilityScript or context is gone before it ran → `Stale`; the caller installs
//!    again and retries once
//! ```
//!
//! Measured on BrowserOS neo 155.0.8318.26 (probe M1): `DOM.resolveNode` without an execution
//! context resolves into the main world of the node's frame for the top frame, a same-process
//! child, a frame that never ran a script, and an OOPIF on its own session; with neither
//! `Runtime.enable` nor `DOM.enable`. The main-world object dies with its document ("Cannot find
//! context with specified id") and with its object group, and is unknown to other sessions. So
//! there is no `Runtime.enable`, no `Runtime.evaluate`, no execution-context event and no global
//! written: the `UtilityScript` is reachable from this session's object id alone, as Playwright
//! keeps it as a handle.
//!
//! `crates/browseros-core/tests/page_js_guard.rs` and `tests/page_js_guard.rs` allow this file
//! to send `Runtime.callFunctionOn` and nothing else that runs script, and require every
//! `functionDeclaration` here to be one of the built constants below.

use std::sync::Arc;

use browseros_cdp::CdpError;
use browseros_cdp::runtime::{CallFunctionOnResult, ExceptionDetails, RemoteObject};
use neo_runtime_spec::injected::InjectedDocumentParams;
use neo_runtime_spec::{Element, Handle, HandleId};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::click::PageError;
use crate::evaluate::js_string;
use crate::frames::FrameContext;
use crate::world::ObjectGroup;

/// `() => new UtilityScript(globalThis, false)`.
/// Playwright: `server/javascript.ts` `ExecutionContext._utilityScript`
const UTILITY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-runtime-injected/dist/utility.js"
));
/// `(utilityScript, ...args) => utilityScript.evaluate(...args)`.
/// Playwright: `server/javascript.ts` `evaluateExpression`
const EVALUATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-runtime-injected/dist/evaluate.js"
));
/// `(utilityScript, ...args) => utilityScript.jsonValue(...args)`.
/// Playwright: `server/javascript.ts` `JSHandle._jsonValue`
const JSON_VALUE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-runtime-injected/dist/json-value.js"
));

/// Playwright: `server/javascript.ts` `JSHandle._getProperty`
const GET_PROPERTY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-runtime-injected/dist/get-property.js"
));
/// Playwright: `server/frames.ts` `Frame.waitForFunctionExpression`
const WAIT_FOR_FUNCTION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-runtime-injected/dist/wait-for-function.js"
));
/// Playwright: `server/frames.ts` `Frame.waitForFunctionExpression`
const WAIT_RESULT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-runtime-injected/dist/wait-result.js"
));
/// Playwright: `server/frames.ts` `Frame.waitForFunctionExpression`
const WAIT_ABORT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-runtime-injected/dist/wait-abort.js"
));

/// A page function of Playwright's server, built in neo-runtime-injected, that runs as the
/// `expression` of `UtilityScript.evaluate` (`EVALUATE`), so its arguments arrive parsed with
/// their handles, primitives included. Agent code never takes this path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuiltExpression {
    /// `(object, name) => ({ __proto__: null, [name]: object[name] })`.
    GetProperty,
    /// `(utilityScript, expression, isFunction, polling, arg) => { result, abort }`.
    WaitForFunction,
    /// `(waiter) => waiter.result`.
    WaitResult,
    /// `(waiter) => waiter.abort()`.
    WaitAbort,
}

impl BuiltExpression {
    /// The built source, as `UtilityScript.evaluate`'s `expression`.
    pub(crate) fn source(self) -> &'static str {
        match self {
            Self::GetProperty => GET_PROPERTY,
            Self::WaitForFunction => WAIT_FOR_FUNCTION,
            Self::WaitResult => WAIT_RESULT,
            Self::WaitAbort => WAIT_ABORT,
        }
    }
}

/// Neo: the object group of every document's `UtilityScript` and its anchor document. Never
/// released by Neo, as `world.rs`'s glue group: Chromium frees it with the run's session, and a
/// new document's install replaces the old one's dead objects.
const MAIN_WORLD_GROUP: &str = "neo-main-world";

/// Platform: V8 inspector's answer when `Runtime.callFunctionOn` names an object whose
/// execution context is gone (Chromium `v8/src/inspector/injected-script.cc`), measured in
/// probe M1. The call never started.
const CONTEXT_GONE: &str = "Cannot find context with specified id";
/// Platform: V8 inspector's answer for a released or unknown object id (Chromium
/// `v8/src/inspector/injected-script.cc`), measured in probe M1. The call never started.
const OBJECT_GONE: &str = "Could not find object with given id";

/// One frame document's main world as one session sees it: the `UtilityScript` of the
/// document that this frame context showed last. Lives in its [`FrameContext`].
#[derive(Default)]
pub(crate) struct MainWorld {
    /// A mutex, not a lock-free cache, so concurrent first calls install once.
    installed: Mutex<Option<Arc<Installed>>>,
}

/// The identity of a main-world document on one session: its document node's `backendNodeId`,
/// which Chromium never reuses for another node. Handles remember it; a handle evaluated
/// against another document is refused as Playwright refuses a handle of another context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DocumentId(i64);

/// A document's installed `UtilityScript`.
pub(crate) struct Installed {
    /// The utility world's document generation at install (`World::generation`).
    generation: u64,
    pub(crate) document: DocumentId,
    /// In [`MAIN_WORLD_GROUP`].
    utility: HandleId,
}

/// The built function a main-world call runs on the `UtilityScript`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MainFunction {
    /// `UtilityScript.evaluate(isFunction, returnByValue, expression, argCount, ...args)`.
    Evaluate,
    /// `UtilityScript.jsonValue(returnByValue, value)`.
    JsonValue,
}

/// How a main-world call answered.
#[derive(Debug)]
pub(crate) enum Called {
    /// `returnByValue: true`: the page's JSON value, `None` for `undefined`.
    Value(Option<Value>),
    /// `returnByValue: false`: the result object, in the caller's group when it has an id.
    Object(Box<RemoteObject>),
    /// The page threw: Playwright's `getExceptionMessage` of the exception details.
    Threw(String),
    /// A protocol error: Playwright's `rewriteError` decides what it means.
    Protocol(String),
    /// The `UtilityScript`, the document or an argument's object was gone before the call
    /// started: nothing ran, so the caller may install again and retry.
    Stale,
}

impl Installed {
    /// The `UtilityScript`'s object id, for an evaluation that hands it to a built page function.
    pub(crate) fn utility_id(&self) -> &str {
        self.utility.object_id()
    }
}

impl MainWorld {
    /// The `UtilityScript` of the frame's current document, installed on first use.
    ///
    /// # Errors
    ///
    /// [`PageError`] when the utility world, the document node or the install fails; a gone
    /// document reads `NotConnected`, as everywhere in the page layer.
    pub(crate) async fn installed(
        &self,
        frame: &FrameContext,
    ) -> Result<Arc<Installed>, PageError> {
        let mut slot = self.installed.lock().await;
        let generation = frame.world.generation();
        if let Some(installed) = slot.as_ref()
            && installed.generation == generation
        {
            return Ok(Arc::clone(installed));
        }
        *slot = None;
        let group = ObjectGroup(MAIN_WORLD_GROUP.to_string());
        let anchor = frame.call(Some(&group), &InjectedDocumentParams {}).await?;
        let document = self.backend_node(frame, &anchor.node).await?;
        let main_document = self.resolve(frame, document.0, &group).await?;
        let object_id = main_document.object_id.ok_or_else(|| {
            PageError::Contract("DOM.resolveNode returned no objectId".to_string())
        })?;
        let reply = frame
            .host
            .send(
                "Runtime.callFunctionOn",
                json!({
                    "functionDeclaration": UTILITY,
                    "objectId": object_id,
                    "returnByValue": false,
                    "objectGroup": MAIN_WORLD_GROUP,
                }),
            )
            .await
            .map_err(|error| frame.host.classify(&error))?;
        let answer: CallFunctionOnResult = serde_json::from_value(reply).map_err(|error| {
            PageError::Contract(format!("installing UtilityScript answered: {error}"))
        })?;
        if let Some(details) = answer.exception_details {
            // Only a page that broke `globalThis` itself could make the constructor throw.
            return Err(PageError::Threw(exception_message(details)));
        }
        let utility = answer.result.object_id.ok_or_else(|| {
            PageError::Contract("utility.js returned no UtilityScript".to_string())
        })?;
        if frame.world.generation() != generation {
            return Err(PageError::NotConnected);
        }
        let installed = Arc::new(Installed {
            generation,
            document,
            utility: HandleId::new(utility),
        });
        *slot = Some(Arc::clone(&installed));
        Ok(installed)
    }

    /// Forgets `stale` if it is still the cached install, so a concurrent call's newer install
    /// survives.
    pub(crate) async fn invalidate(&self, stale: &Arc<Installed>) {
        let mut slot = self.installed.lock().await;
        if slot
            .as_ref()
            .is_some_and(|installed| Arc::ptr_eq(installed, stale))
        {
            *slot = None;
        }
    }

    /// Runs `function` on `installed`'s `UtilityScript` with `arguments` (CDP `CallArgument`s,
    /// after the `{objectId: UtilityScript}` Playwright passes first): `awaitPromise` and
    /// `userGesture`, by value or into `group`. Cancel-safe: dropping it loses only the answer.
    /// Playwright: `server/chromium/crExecutionContext.ts` `CRExecutionContext.evaluateWithArguments`
    ///
    /// # Errors
    ///
    /// `Closed` when the session or the tab is gone; every other outcome is a [`Called`].
    pub(crate) async fn call(
        &self,
        frame: &FrameContext,
        installed: &Installed,
        function: MainFunction,
        arguments: Vec<Value>,
        return_by_value: bool,
        group: Option<&ObjectGroup>,
    ) -> Result<Called, PageError> {
        let mut all = Vec::with_capacity(arguments.len() + 1);
        all.push(json!({ "objectId": installed.utility.object_id() }));
        all.extend(arguments);
        // Each declaration is named at its key, so the page JS guard can see it is built.
        let mut request = match function {
            MainFunction::Evaluate => json!({ "functionDeclaration": EVALUATE }),
            MainFunction::JsonValue => json!({ "functionDeclaration": JSON_VALUE }),
        };
        request["objectId"] = json!(installed.utility.object_id());
        request["arguments"] = json!(all);
        request["returnByValue"] = json!(return_by_value);
        request["awaitPromise"] = json!(true);
        request["userGesture"] = json!(true);
        if let Some(ObjectGroup(group)) = group {
            request["objectGroup"] = json!(group);
        }
        let reply = match frame.host.send("Runtime.callFunctionOn", request).await {
            Ok(reply) => reply,
            Err(CdpError::Protocol { message, .. }) => {
                return Ok(
                    if message.contains(CONTEXT_GONE) || message.contains(OBJECT_GONE) {
                        Called::Stale
                    } else {
                        Called::Protocol(message)
                    },
                );
            }
            Err(error) => {
                return match frame.host.classify(&error) {
                    // A request timeout or a vanished OOPIF: the document is gone for this call.
                    PageError::NotConnected => Ok(Called::Protocol(error.to_string())),
                    other => Err(other),
                };
            }
        };
        let answer: CallFunctionOnResult = serde_json::from_value(reply).map_err(|error| {
            PageError::Contract(format!("Runtime.callFunctionOn answered: {error}"))
        })?;
        if let Some(details) = answer.exception_details {
            return Ok(Called::Threw(exception_message(details)));
        }
        Ok(if return_by_value {
            Called::Value(answer.result.value)
        } else {
            Called::Object(Box::new(answer.result))
        })
    }

    /// Adopts `node`, a utility-world node of `frame`'s current document, into the main world
    /// in `group`, as Playwright's `ElementHandle._adoptTo(mainContext)` does with
    /// `DOM.describeNode` and `DOM.resolveNode`. Returns the document the node belongs to.
    ///
    /// # Errors
    ///
    /// `NotConnected` when the node or its document is gone, so a wait keeps polling.
    pub(crate) async fn adopt(
        &self,
        frame: &FrameContext,
        node: &Handle<Element>,
        group: &ObjectGroup,
    ) -> Result<(DocumentId, RemoteObject), PageError> {
        let installed = self.installed(frame).await?;
        let backend = self.backend_node(frame, node).await?;
        let object = self.resolve(frame, backend.0, group).await?;
        if frame.world.generation() != installed.generation {
            return Err(PageError::NotConnected);
        }
        Ok((installed.document, object))
    }

    /// The main-world object's `backendNodeId`, for adopting it into the utility world.
    ///
    /// # Errors
    ///
    /// `NotConnected` when the object or its document is gone.
    pub(crate) async fn backend_of(
        &self,
        frame: &FrameContext,
        object_id: &str,
    ) -> Result<i64, PageError> {
        describe(frame, object_id).await
    }

    async fn backend_node(
        &self,
        frame: &FrameContext,
        node: &Handle<Element>,
    ) -> Result<DocumentId, PageError> {
        describe(frame, node.id().object_id()).await.map(DocumentId)
    }

    async fn resolve(
        &self,
        frame: &FrameContext,
        backend: i64,
        group: &ObjectGroup,
    ) -> Result<RemoteObject, PageError> {
        // No `executionContextId`: Chromium resolves into the main world of the node's frame.
        let reply = frame
            .host
            .send(
                "DOM.resolveNode",
                json!({ "backendNodeId": backend, "objectGroup": group.0 }),
            )
            .await
            .map_err(|error| frame.host.classify(&error))?;
        serde_json::from_value(reply["object"].clone())
            .map_err(|error| PageError::Contract(format!("DOM.resolveNode answered: {error}")))
    }
}

async fn describe(frame: &FrameContext, object_id: &str) -> Result<i64, PageError> {
    let reply = frame
        .host
        .send("DOM.describeNode", json!({ "objectId": object_id }))
        .await
        .map_err(|error| frame.host.classify(&error))?;
    reply["node"]["backendNodeId"]
        .as_i64()
        .ok_or_else(|| PageError::Contract("DOM.describeNode returned no backendNodeId".into()))
}

/// Playwright: `server/chromium/crProtocolHelper.ts` `getExceptionMessage`: `return exceptionDetails.exception.description || String(exceptionDetails.exception.value);`
fn exception_message(details: ExceptionDetails) -> String {
    if let Some(exception) = details.exception {
        if let Some(description) = exception.description {
            return description;
        }
        // `String(value)`; serde reads a thrown `null` as no value, so its subtype tells.
        if exception.subtype.as_deref() == Some("null") {
            return "null".to_string();
        }
        return match exception.value {
            Some(Value::String(text)) => text,
            Some(value) => js_string(&value),
            None => "undefined".to_string(),
        };
    }
    let mut message = details.text;
    if let Some(trace) = details.stack_trace {
        for frame in trace.call_frames {
            let name = if frame.function_name.is_empty() {
                "<anonymous>"
            } else {
                frame.function_name.as_str()
            };
            let location = [
                frame.url,
                (frame.line_number + 1).to_string(),
                (frame.column_number + 1).to_string(),
            ]
            .join(":");
            message.push_str("\n    at ");
            message.push_str(name);
            message.push_str(" (");
            message.push_str(&location);
            message.push(')');
        }
    }
    message
}
