//! Neo: the only door from Rust into a frame's MAIN world, where Playwright runs every page
//! evaluation (`page.evaluate`, `locator.evaluate`, `JSHandle.evaluate`), as opposed to the
//! utility world (`playwright_injected_methods.rs`) where every Neo read and action runs. It runs exactly three built
//! functions of neo-playwright-injected, never anything else: `dist/utility.js` builds Playwright's
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
//!  a call whose UtilityScript or context is gone before it went out → `Stale`; the caller
//!    installs again and retries once
//!  a call whose document went while it was out (its frame's commit, seen by the page's
//!    watcher before the reply) → `Protocol`: the destroyed context's error, never run again
//!  a binding call's answer (bindings.rs): the same call, in the execution context that called
//!    {executionContextId, arguments: [{objectId: UtilityScript}, …]}: it runs only while that
//!    context lives and holds the UtilityScript, so it never reaches the next document
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
use neo_playwright_protocol::injected::{InjectedDocumentParams, InjectedInitializer};
use neo_playwright_protocol::{Element, Handle, HandleId};
use serde_json::{Value, json};
use tokio::sync::broadcast::error::TryRecvError;
use tokio::sync::{Mutex, OnceCell};

use crate::chromium_bridge::cdp::{self, Command, dom, page, runtime};
use crate::chromium_bridge::frames::FrameContext;
use crate::chromium_bridge::playwright_injected_methods::{self, ObjectGroup};
use crate::playwright_methods::evaluate::js_string;
use crate::playwright_methods::tab::PageError;

/// `() => new UtilityScript(globalThis, false)`.
/// Playwright: `server/javascript.ts` `ExecutionContext._utilityScript`
const UTILITY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/utility.js"
));
/// `(utilityScript, ...args) => utilityScript.evaluate(...args)`.
/// Playwright: `server/javascript.ts` `evaluateExpression`
const EVALUATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/evaluate.js"
));
/// `(utilityScript, ...args) => utilityScript.jsonValue(...args)`.
/// Playwright: `server/javascript.ts` `JSHandle._jsonValue`
const JSON_VALUE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/json-value.js"
));

/// Playwright: `server/javascript.ts` `JSHandle._getProperty`
const GET_PROPERTY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/get-property.js"
));
/// Playwright: `server/frames.ts` `Frame.waitForFunctionExpression`
const WAIT_FOR_FUNCTION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/wait-for-function.js"
));
/// Playwright: `server/frames.ts` `Frame.waitForFunctionExpression`
const WAIT_RESULT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/wait-result.js"
));
/// Playwright: `server/frames.ts` `Frame.waitForFunctionExpression`
const WAIT_ABORT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/wait-abort.js"
));

/// `(object) => object`, run on a page object an event named, with that object as its argument.
/// Neo: Playwright's handles of console arguments stay in the group CDP put them in; Neo's live
/// in the run's handle group, which Neo releases at run end (`evaluate::handle_for_remote_object`).
const RETAIN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/retain.js"
));

/// `(initializer) => InjectedScript`: the main world's own (`isUtilityWorld: false`), returned to
/// CDP and kept by id, never stored on the page's globals.
/// Playwright: `server/dom.ts` `FrameExecutionContext.injectedScript`
const MAIN_INJECTED: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/main-injected.js"
));
/// Playwright: `server/frames.ts` `Frame._waitForFunctionOnSelector`
const CALL_ON_ELEMENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/call-on-element.js"
));

/// Playwright: `server/frames.ts` `Frame._addScriptTag`
const ADD_TAG: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/add-tag.js"
));

/// Playwright: `server/page.ts` `PageBinding.createInitScript`
const BINDINGS_CONTROLLER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/bindings-controller.js"
));

/// Playwright: `server/page.ts` `PageBinding.dispatch`
const BINDINGS_CALL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/bindings-call.js"
));

/// Playwright: `server/clock.ts` `Clock._installIfNeeded`
const CLOCK_INSTALL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/clock-install.js"
));

/// Playwright: `server/clock.ts` `Clock.install`
const CLOCK_CALL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../neo-playwright-injected/dist/clock-call.js"
));

/// A page function of Playwright's server, built in neo-playwright-injected, that runs as the
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
    /// `(injected, selector, strict, scope, callback, ...data) => 0 | 1 | preview`: a
    /// `mainWorld: true` element callback on the first match.
    CallOnElement,
    /// `(utilityScript, kind, value, type) => element`: `addScriptTag` / `addStyleTag`.
    AddTag,
    /// `() => void`: installs Playwright's `BindingsController` (`bindings.rs`), also the source
    /// of the run's bindings init script.
    BindingsController,
    /// `(op, arg) => void`: a call on that controller, by name (`bindings.rs`).
    BindingsCall,
    /// `(browserName) => void`: installs Playwright's fake clock (`clock.rs`), also the source of
    /// the run's clock init script.
    ClockInstall,
    /// `(op, ...args) => unknown`: a call on that clock's controller, by name (`clock.rs`).
    ClockCall,
}

impl BuiltExpression {
    /// The built source, as `UtilityScript.evaluate`'s `expression`.
    pub(crate) fn source(self) -> &'static str {
        match self {
            Self::GetProperty => GET_PROPERTY,
            Self::WaitForFunction => WAIT_FOR_FUNCTION,
            Self::WaitResult => WAIT_RESULT,
            Self::WaitAbort => WAIT_ABORT,
            Self::CallOnElement => CALL_ON_ELEMENT,
            Self::AddTag => ADD_TAG,
            Self::BindingsController => BINDINGS_CONTROLLER,
            Self::BindingsCall => BINDINGS_CALL,
            Self::ClockInstall => CLOCK_INSTALL,
            Self::ClockCall => CLOCK_CALL,
        }
    }
}

/// Neo: the object group of every document's `UtilityScript` and its anchor document. Never
/// released by Neo, as `playwright_injected_methods.rs`'s glue group: Chromium frees it with the run's session, and a
/// new document's install replaces the old one's dead objects.
const MAIN_WORLD_GROUP: &str = "neo-main-world";

/// Platform: V8 inspector's answer when `Runtime.callFunctionOn` names an object whose
/// execution context is gone (Chromium `v8/src/inspector/injected-script.cc`), measured in
/// probe M1. The call never started.
const CONTEXT_GONE: &str = "Cannot find context with specified id";
/// Platform: V8 inspector's answer for a released or unknown object id (Chromium
/// `v8/src/inspector/injected-script.cc`), measured in probe M1. The call never started.
const OBJECT_GONE: &str = "Could not find object with given id";

/// Whether `frame`'s document went while a call was out: among the events its host's session
/// reported since the call was sent, a commit of the frame or of the session's main frame
/// (which takes its child frames along), or the frame's removal. Events a lagging reader missed
/// count as gone, so a call that may have run is never sent again.
///
/// Playwright: `server/frames.ts` `Frame.contextDestroyed`: `context.contextDestroyed('Execution context was destroyed, most likely because of a navigation');`
pub(crate) fn replaced_during(events: &mut cdp::Events, frame: &FrameContext) -> bool {
    let session = frame.host.session_id();
    loop {
        let event = match events.try_recv() {
            Ok(event) => event,
            Err(TryRecvError::Lagged(_)) => return true,
            Err(TryRecvError::Empty | TryRecvError::Closed) => return false,
        };
        if event.session() != Some(session) {
            continue;
        }
        let navigated = event
            .decode::<page::FrameNavigated>()
            .is_some_and(|navigated| {
                navigated.frame.id == frame.id || navigated.frame.parent_id.is_none()
            });
        let detached = event
            .decode::<page::FrameDetached>()
            .is_some_and(|detached| detached.frame_id == frame.id);
        if navigated || detached {
            return true;
        }
    }
}

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

impl DocumentId {
    /// Neo: the document of a handle made after its page closed (`evaluate.rs`
    /// `handle_for_gone_object`): no live document has it, since Chromium's backend node ids are
    /// positive.
    pub(crate) const GONE: Self = Self(0);
}

/// A document's installed `UtilityScript`.
pub(crate) struct Installed {
    /// The utility world's document generation at install (`World::generation`).
    generation: u64,
    pub(crate) document: DocumentId,
    /// In [`MAIN_WORLD_GROUP`].
    utility: HandleId,
    /// The document's main-world `InjectedScript` once a call needed it, in [`MAIN_WORLD_GROUP`].
    injected: OnceCell<HandleId>,
}

/// The built function a main-world call runs on the `UtilityScript`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MainFunction {
    /// `UtilityScript.evaluate(isFunction, returnByValue, expression, argCount, ...args)`.
    Evaluate,
    /// `UtilityScript.jsonValue(returnByValue, value)`.
    JsonValue,
}

/// The execution context a pinned call runs in ([`MainWorld::call_in`]): its system-unique id
/// when Chromium reported one, which a context of another process never reuses (a frame that
/// changed process may number its new contexts as its old ones), else its numeric id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContextPin {
    pub(crate) id: i64,
    pub(crate) unique: Option<String>,
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
    /// went out (an install this session kept from a document replaced since): nothing ran, so
    /// the caller may install again and retry. A document replaced while the call was out is a
    /// [`Called::Protocol`] error instead: the call may have run there.
    Stale,
}

impl Installed {
    /// The `UtilityScript`'s object id, for an evaluation that hands it to a built page function.
    pub(crate) fn utility_id(&self) -> &str {
        self.utility.object_id()
    }
}

/// The main world's `InjectedScript` options: the utility world's but `isUtilityWorld: false`, as
/// Playwright builds each world's injected script from the same settings.
/// Playwright: `server/dom.ts` `FrameExecutionContext.injectedScript`: `isUtilityWorld: this.world === 'utility',`
fn main_initializer() -> InjectedInitializer {
    InjectedInitializer {
        is_utility_world: false,
        ..playwright_injected_methods::initializer()
    }
}

impl MainWorld {
    /// `installed`'s main-world `InjectedScript`, built from `MAIN_INJECTED` on the first call that
    /// needs it and kept by CDP id with the `UtilityScript`; a new document gets a new one.
    ///
    /// # Errors
    ///
    /// `NotConnected` when the document went away meanwhile; `Closed` when the session did.
    pub(crate) async fn injected(
        &self,
        frame: &FrameContext,
        installed: &Installed,
    ) -> Result<String, PageError> {
        let id = installed
            .injected
            .get_or_try_init(|| async {
                let reply = frame
                    .host
                    .send_command(runtime::CallFunctionOn {
                        function_declaration: MAIN_INJECTED.to_string(),
                        object_id: Some(installed.utility.object_id().to_string()),
                        arguments: Some(vec![playwright_injected_methods::argument(json!(
                            main_initializer()
                        ))]),
                        return_by_value: Some(false),
                        object_group: Some(MAIN_WORLD_GROUP.to_string()),
                        ..runtime::CallFunctionOn::default()
                    })
                    .await
                    .map_err(|error| frame.host.classify(&error))?;
                let answer: CallFunctionOnResult =
                    serde_json::from_value(reply).map_err(|error| {
                        PageError::Contract(format!(
                            "{} answered: {error}",
                            runtime::CallFunctionOn::METHOD
                        ))
                    })?;
                if let Some(details) = answer.exception_details {
                    return Err(PageError::Contract(exception_message(details)));
                }
                answer.result.object_id.map(HandleId::new).ok_or_else(|| {
                    PageError::Contract("the main world's InjectedScript has no objectId".into())
                })
            })
            .await?;
        Ok(id.object_id().to_string())
    }

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
            PageError::Contract(format!("{} returned no objectId", dom::ResolveNode::METHOD))
        })?;
        let reply = frame
            .host
            .send_command(runtime::CallFunctionOn {
                function_declaration: UTILITY.to_string(),
                object_id: Some(object_id),
                return_by_value: Some(false),
                object_group: Some(MAIN_WORLD_GROUP.to_string()),
                ..runtime::CallFunctionOn::default()
            })
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
            injected: OnceCell::new(),
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
        self.call_in(
            frame,
            installed,
            None,
            function,
            arguments,
            return_by_value,
            group,
        )
        .await
    }

    /// [`Self::call`], in the execution context `context` names when given: the call runs only
    /// while that context lives and holds `installed`'s `UtilityScript`, which it is passed, so
    /// an answer meant for a document that is gone never reaches the next one (`Called::Stale`
    /// or `Called::Protocol`).
    /// Playwright: `server/page.ts` `PageBinding.dispatch`: `{ isFunction: true }, { name, seq, result }).catch(e => debugLogger.log('error', e));`
    ///
    /// # Errors
    ///
    /// As [`Self::call`].
    #[expect(
        clippy::too_many_arguments,
        reason = "one CDP call's every field, as `call` takes them, and the context it is pinned to"
    )]
    pub(crate) async fn call_in(
        &self,
        frame: &FrameContext,
        installed: &Installed,
        context: Option<&ContextPin>,
        function: MainFunction,
        arguments: Vec<Value>,
        return_by_value: bool,
        group: Option<&ObjectGroup>,
    ) -> Result<Called, PageError> {
        let request = call_request(
            installed.utility.object_id(),
            context,
            function,
            arguments,
            return_by_value,
            group,
        );
        // Subscribed before the call goes out: what arrives here came after it, in the socket's
        // order, so a commit that precedes the reply came while the call was in the page.
        let mut events = frame.host.events();
        let reply = match frame.host.send_command(request).await {
            Ok(reply) => reply,
            Err(CdpError::Protocol { message, .. }) => {
                let gone = message.contains(CONTEXT_GONE) || message.contains(OBJECT_GONE);
                return Ok(if gone && !replaced_during(&mut events, frame) {
                    Called::Stale
                } else {
                    // Gone while the call was in the page (Chromium holds a call to a document
                    // with a navigation pending, then answers once the next one commits):
                    // Playwright's call fails with the context it was sent to
                    // (`rewriteError`) and never runs again in the next document.
                    Called::Protocol(message)
                });
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
            PageError::Contract(format!(
                "{} answered: {error}",
                runtime::CallFunctionOn::METHOD
            ))
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
            .send_command(dom::ResolveNode {
                backend_node_id: Some(backend),
                object_group: Some(group.0.clone()),
                node_id: None,
                execution_context_id: None,
            })
            .await
            .map_err(|error| frame.host.classify(&error))?;
        Ok(reply.object)
    }
}

/// `object_id`, a page object an event named (a console message's argument), as the same object
/// under a new id in `group`: CDP's answer to `RETAIN` run on it with the object as its argument.
/// The new id lives until `group` is released, whatever becomes of the event's own group.
///
/// # Errors
///
/// `NotConnected` when the object or its context is gone; `Closed` when the session is.
/// One `Runtime.callFunctionOn` of a built function on `utility` (the `UtilityScript`'s object
/// id, always the first argument): on that object, or in the execution context `context`.
fn call_request(
    utility: &str,
    context: Option<&ContextPin>,
    function: MainFunction,
    arguments: Vec<Value>,
    return_by_value: bool,
    group: Option<&ObjectGroup>,
) -> runtime::CallFunctionOn {
    let mut all = Vec::with_capacity(arguments.len() + 1);
    all.push(runtime::CallArgument {
        object_id: Some(utility.to_string()),
        ..runtime::CallArgument::default()
    });
    all.extend(arguments.into_iter().map(call_argument));
    // Each declaration is named at its key, so the page JS guard can see it is built.
    let mut request = match function {
        MainFunction::Evaluate => runtime::CallFunctionOn {
            function_declaration: EVALUATE.to_string(),
            ..runtime::CallFunctionOn::default()
        },
        MainFunction::JsonValue => runtime::CallFunctionOn {
            function_declaration: JSON_VALUE.to_string(),
            ..runtime::CallFunctionOn::default()
        },
    };
    match context {
        Some(ContextPin {
            unique: Some(unique),
            ..
        }) => request.unique_context_id = Some(unique.clone()),
        Some(pin) => request.execution_context_id = Some(pin.id),
        None => request.object_id = Some(utility.to_string()),
    }
    request.arguments = Some(all);
    request.return_by_value = Some(return_by_value);
    request.await_promise = Some(true);
    request.user_gesture = Some(true);
    if let Some(ObjectGroup(group)) = group {
        request.object_group = Some(group.clone());
    }
    request
}

/// An argument a caller built as CDP's JSON (`{"value": …}`, `{"objectId": …}` or
/// `{"unserializableValue": …}`), as the generated type. A `"value": null` stays a value, as CDP
/// reads it; only an absent one is `undefined`.
fn call_argument(argument: Value) -> runtime::CallArgument {
    let Value::Object(mut fields) = argument else {
        return runtime::CallArgument::default();
    };
    let text = |value: Option<Value>| match value {
        Some(Value::String(text)) => Some(text),
        _ => None,
    };
    runtime::CallArgument {
        value: fields.remove("value"),
        unserializable_value: text(fields.remove("unserializableValue")),
        object_id: text(fields.remove("objectId")),
    }
}

pub(crate) async fn retain(
    frame: &FrameContext,
    object_id: &str,
    group: &ObjectGroup,
) -> Result<RemoteObject, PageError> {
    let reply = frame
        .host
        .send_command(runtime::CallFunctionOn {
            function_declaration: RETAIN.to_string(),
            object_id: Some(object_id.to_string()),
            arguments: Some(vec![runtime::CallArgument {
                object_id: Some(object_id.to_string()),
                ..runtime::CallArgument::default()
            }]),
            object_group: Some(group.0.clone()),
            return_by_value: Some(false),
            ..runtime::CallFunctionOn::default()
        })
        .await
        .map_err(|error| frame.host.classify(&error))?;
    let answer: CallFunctionOnResult = serde_json::from_value(reply).map_err(|error| {
        PageError::Contract(format!(
            "{} answered: {error}",
            runtime::CallFunctionOn::METHOD
        ))
    })?;
    match answer.exception_details {
        Some(details) => Err(PageError::Contract(exception_message(details))),
        None => Ok(answer.result),
    }
}

async fn describe(frame: &FrameContext, object_id: &str) -> Result<i64, PageError> {
    let reply = frame
        .host
        .send_command(dom::DescribeNode {
            object_id: Some(object_id.to_string()),
            ..dom::DescribeNode::default()
        })
        .await
        .map_err(|error| frame.host.classify(&error))?;
    Ok(reply.node.backend_node_id)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_pinned_to_a_context_names_it_and_passes_the_utility_script() {
        let numbered = ContextPin {
            id: 7,
            unique: None,
        };
        let pinned = serde_json::to_value(call_request(
            "utility-1",
            Some(&numbered),
            MainFunction::Evaluate,
            vec![json!({ "value": 1 })],
            true,
            None,
        ))
        .expect("the request serializes");
        assert_eq!(pinned["executionContextId"], json!(7));
        assert!(
            pinned.get("objectId").is_none(),
            "a pinned call names its context, not an object: {pinned}"
        );
        assert_eq!(
            pinned["arguments"],
            json!([{ "objectId": "utility-1" }, { "value": 1 }]),
            "the UtilityScript argument ties the call to its document"
        );
        // A context of another process may reuse 7: its unique id names this one alone.
        let unique = ContextPin {
            id: 7,
            unique: Some("-123.456".to_string()),
        };
        let pinned = serde_json::to_value(call_request(
            "utility-1",
            Some(&unique),
            MainFunction::Evaluate,
            Vec::new(),
            true,
            None,
        ))
        .expect("the request serializes");
        assert_eq!(pinned["uniqueContextId"], json!("-123.456"));
        assert!(pinned.get("executionContextId").is_none());
        let group = ObjectGroup("group".to_string());
        let unpinned = serde_json::to_value(call_request(
            "utility-1",
            None,
            MainFunction::JsonValue,
            Vec::new(),
            false,
            Some(&group),
        ))
        .expect("the request serializes");
        assert_eq!(unpinned["objectId"], json!("utility-1"));
        assert!(unpinned.get("executionContextId").is_none());
        assert_eq!(unpinned["objectGroup"], json!("group"));
        assert_eq!(unpinned["functionDeclaration"], json!(JSON_VALUE));
    }
}
