//! Neo: page JS evaluation and the run's JS handles, the policy over `main_world.rs`.
//! Playwright's server side of `Frame.evaluateExpression(Handle)` and the `JSHandle` commands
//! (`server/javascript.ts` `evaluateExpression`, `JSHandle`; `dispatchers/jsHandleDispatcher.ts`;
//! `chromium/crExecutionContext.ts` `evaluateWithArguments`, `rewriteError`, `createHandle`).
//!
//! ```text
//!  SDK: serializeArgument → Playwright's client + server chain → {value (page format), handles}
//!    → Frame.evaluateExpression{expression, isFunction, arg}  (guid frame@…)
//!    → JSHandle.evaluateExpression / jsonValue / dispose     (guid handle@<n>)
//!  here: guids → objectIds of this frame document (refusing foreign, disposed or other-document
//!        handles with Playwright's errors), main_world::call, Playwright's rewriteError
//!  reply: {value} (the page's JSON, never read here) | {handle: {guid, initializer: {preview}}}
//! ```
//!
//! Rust never interprets a serialized value: `arg.value` and the page's answer pass through as
//! JSON. The one page-format value written here is `{h: n}`, a handle receiver's own reference.
//!
//! Handles live for one run. Every agent handle of a session is in one object group, the run's
//! handle group of that session ([`Handles`]), released when the run ends: by `Dispatcher::finish`,
//! or by the run's state dropping on cancel. `dispose()` releases one object and forgets its guid,
//! so a later call on it fails as on a guid Playwright's server never knew. Every guid carries its
//! run's random token, so a guid a script kept from an earlier run, in this process or another,
//! never names a handle of this one.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::mem;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use browseros_cdp::SessionId;
use browseros_cdp::runtime::RemoteObject;
use neo_runtime_spec::injected::InjectedPreviewNodeParams;
use neo_runtime_spec::{Element, Handle, HandleId, MethodFlag, MethodGroup, MethodMeta};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::click::{LoopAWaits, PageError};
use crate::frames::{FrameContext, FrameScope};
use crate::main_world::{Called, DocumentId, Installed, MainFunction};
use crate::page::{FrameTab, GroupGuard};
use crate::progress::{CallError, PlaywrightError, Progress, js_number};

/// Neo: the guid prefix of a run's handles (`handle@<run>.<n>`), opaque to the SDK.
pub(crate) const HANDLE_PREFIX: &str = "handle@";

/// Neo: the next handle number, process-wide, so a guid never names two handles in one process.
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

/// Neo: counts runs, salting each run's random token, so two runs never share one.
static NEXT_RUN: AtomicU64 = AtomicU64::new(1);

/// Playwright: `server/javascript.ts` `evaluateExpression`: `throw new JavaScriptErrorInEvaluate('JSHandles can be evaluated only in the context they were created!');`
const OTHER_CONTEXT: &str = "JSHandles can be evaluated only in the context they were created!";

/// Playwright: `server/chromium/crExecutionContext.ts` `rewriteError`: `throw new Error('Execution context was destroyed, most likely because of a navigation.');`
const CONTEXT_DESTROYED: &str =
    "Execution context was destroyed, most likely because of a navigation.";

/// Playwright: `server/chromium/crExecutionContext.ts` `rewriteError`: `throw new Error('Cannot serialize result: object reference chain is too long.');`
const CHAIN_TOO_LONG: &str = "Cannot serialize result: object reference chain is too long.";

/// Playwright: `server/chromium/crExecutionContext.ts` `rewriteError`: `throw new Error('Resulting promise was garbage collected.');`
const PROMISE_COLLECTED: &str = "Resulting promise was garbage collected.";

/// Playwright: `server/chromium/crExecutionContext.ts` `rewriteError`: `if (error.message.includes('Object reference chain is too long') || error.message.includes('CBOR: stack limit exceeded'))`
const CHAIN_TOO_LONG_CDP: [&str; 2] = [
    "Object reference chain is too long",
    "CBOR: stack limit exceeded",
];

/// Playwright: `server/chromium/crExecutionContext.ts` `rewriteError`: `if (error.message.includes('Object couldn\'t be returned by value'))`
const NOT_BY_VALUE_CDP: &str = "Object couldn't be returned by value";

/// Playwright: `server/chromium/crExecutionContext.ts` `rewriteError`: `if (error.message.includes('Promise was collected'))`
const PROMISE_COLLECTED_CDP: &str = "Promise was collected";

/// Playwright: `server/dom.ts` `ElementHandle._initializePreview`: `'JSHandle@' + injected.previewNode(e)`
const NODE_PREVIEW_PREFIX: &str = "JSHandle@";

/// Playwright: `server/dom.ts` `ElementHandle.constructor`: `super(context, 'node', undefined, objectId);`
const NODE_TYPE: &str = "node";

/// Neo: the trace metadata of Playwright's `Frame.evaluateExpression`, as
/// `neo-runtime-spec/spec/playwright/frame.yml` declares it; written here because the spec does not
/// generate this command (its `arg` arrives in the page's format).
pub(crate) static FRAME_EVALUATE_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "evaluateExpression",
    title: Some("Evaluate"),
    subtitle: None,
    group: None,
    render_params: &[],
    internal: false,
    flags: &[MethodFlag::Snapshot, MethodFlag::Pause],
};
/// Neo: `Frame.evaluateExpressionHandle`'s trace metadata, as
/// `neo-runtime-spec/spec/playwright/frame.yml` declares it.
pub(crate) static FRAME_EVALUATE_HANDLE_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "evaluateExpressionHandle",
    title: Some("Evaluate"),
    subtitle: None,
    group: None,
    render_params: &[],
    internal: false,
    flags: &[MethodFlag::Snapshot, MethodFlag::Pause],
};
/// Neo: `JSHandle.evaluateExpression`'s trace metadata (also an `ElementHandle`'s), as
/// `neo-runtime-spec/spec/playwright/handles.yml` declares it.
pub(crate) static HANDLE_EVALUATE_META: MethodMeta = MethodMeta {
    interface: "JSHandle",
    method: "evaluateExpression",
    title: Some("Evaluate"),
    subtitle: None,
    group: None,
    render_params: &[],
    internal: false,
    flags: &[MethodFlag::Snapshot, MethodFlag::Pause],
};
/// Neo: `JSHandle.evaluateExpressionHandle`'s trace metadata, as
/// `neo-runtime-spec/spec/playwright/handles.yml` declares it.
pub(crate) static HANDLE_EVALUATE_HANDLE_META: MethodMeta = MethodMeta {
    interface: "JSHandle",
    method: "evaluateExpressionHandle",
    title: Some("Evaluate"),
    subtitle: None,
    group: None,
    render_params: &[],
    internal: false,
    flags: &[MethodFlag::Snapshot, MethodFlag::Pause],
};
/// Neo: `JSHandle.jsonValue`'s trace metadata, as `neo-runtime-spec/spec/playwright/handles.yml`
/// declares it.
pub(crate) static HANDLE_JSON_VALUE_META: MethodMeta = MethodMeta {
    interface: "JSHandle",
    method: "jsonValue",
    title: Some("Get JSON value"),
    subtitle: None,
    group: Some(MethodGroup::Getter),
    render_params: &[],
    internal: false,
    flags: &[],
};
/// Neo: `JSHandle.dispose`'s trace metadata, an internal command in
/// `neo-runtime-spec/spec/playwright/handles.yml`.
pub(crate) static HANDLE_DISPOSE_META: MethodMeta = MethodMeta {
    interface: "JSHandle",
    method: "dispose",
    title: None,
    subtitle: None,
    group: None,
    render_params: &[],
    internal: true,
    flags: &[],
};

/// Playwright's `SerializedArgument` after neo-sdk ran Playwright's client and server steps on
/// it (`neo-sdk/src/js-handle.ts` `serializeArgument`): `value` is in the page's format and
/// stays opaque here; `handles` are the guids its `{h: i}` references name, in order.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SerializedArgument {
    pub(crate) value: Value,
    pub(crate) handles: Vec<String>,
}

/// The params of every evaluation command: Playwright's, with `arg` as above.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EvaluateParams {
    expression: String,
    #[serde(default)]
    is_function: Option<bool>,
    arg: SerializedArgument,
}

/// `JSHandle.jsonValue` and `JSHandle.dispose` take no params.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoParams {}

/// One evaluation as the page runs it.
#[derive(Debug, Clone)]
pub(crate) struct Evaluation {
    /// Already normalized by the SDK (`normalizeEvaluationExpression`).
    pub(crate) expression: String,
    pub(crate) is_function: Option<bool>,
    pub(crate) arg: SerializedArgument,
    pub(crate) return_by_value: bool,
}

/// A routed evaluation command.
#[derive(Debug)]
pub(crate) enum EvalCall {
    Frame(Evaluation),
    Handle {
        guid: String,
        evaluation: Evaluation,
    },
    JsonValue {
        guid: String,
    },
    Dispose {
        guid: String,
    },
}

/// What a page evaluation answers: the page's JSON value (absent for `undefined`), or a new
/// handle.
#[derive(Debug)]
pub(crate) enum EvalResult {
    Value(Option<Value>),
    Handle(CreatedHandle),
}

/// A handle Rust created, as the SDK builds its `JSHandle` or `ElementHandle` from it:
/// Playwright's `__create__` record for the object, `{type, guid, initializer: {preview}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct CreatedHandle {
    /// `ElementHandle` for a node, `JSHandle` otherwise.
    /// Playwright: `server/dispatchers/jsHandleDispatcher.ts` `JSHandleDispatcher.constructor`: `jsHandle.asElement() ? 'ElementHandle' : 'JSHandle'`
    #[serde(rename = "type")]
    pub(crate) kind: HandleType,
    pub(crate) guid: String,
    pub(crate) initializer: HandleInitializer,
}

/// The two handle interfaces of Playwright's protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum HandleType {
    JSHandle,
    ElementHandle,
}

/// Playwright's `JSHandleInitializer`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct HandleInitializer {
    pub(crate) preview: String,
}

/// Routes a frame command that is an evaluation; `None` for any other method.
pub(crate) fn route_frame(
    method: &str,
    params: Option<&Value>,
) -> Option<(&'static MethodMeta, Result<EvalCall, PlaywrightError>)> {
    let (meta, return_by_value) = match method {
        "evaluateExpression" => (&FRAME_EVALUATE_META, true),
        "evaluateExpressionHandle" => (&FRAME_EVALUATE_HANDLE_META, false),
        _ => return None,
    };
    Some((
        meta,
        decode::<EvaluateParams>(params, meta).map(|params| {
            EvalCall::Frame(Evaluation {
                expression: params.expression,
                is_function: params.is_function,
                arg: params.arg,
                return_by_value,
            })
        }),
    ))
}

/// Routes a command on a handle guid. An unknown method replies as Playwright's dispatcher
/// does for a method the object lacks.
pub(crate) fn route_handle(
    guid: &str,
    method: &str,
    params: Option<&Value>,
) -> (
    Option<&'static MethodMeta>,
    Result<EvalCall, PlaywrightError>,
) {
    let guid = guid.to_string();
    let (meta, call) = match method {
        "evaluateExpression" | "evaluateExpressionHandle" => {
            let (meta, return_by_value) = if method == "evaluateExpression" {
                (&HANDLE_EVALUATE_META, true)
            } else {
                (&HANDLE_EVALUATE_HANDLE_META, false)
            };
            let call = decode::<EvaluateParams>(params, meta).map(|params| EvalCall::Handle {
                guid,
                evaluation: Evaluation {
                    expression: params.expression,
                    is_function: params.is_function,
                    arg: params.arg,
                    return_by_value,
                },
            });
            (meta, call)
        }
        "jsonValue" => (
            &HANDLE_JSON_VALUE_META,
            decode::<NoParams>(params, &HANDLE_JSON_VALUE_META)
                .map(|NoParams {}| EvalCall::JsonValue { guid }),
        ),
        "dispose" => (
            &HANDLE_DISPOSE_META,
            decode::<NoParams>(params, &HANDLE_DISPOSE_META)
                .map(|NoParams {}| EvalCall::Dispose { guid }),
        ),
        // Playwright: `server/dispatchers/dispatcher.ts` `DispatcherConnection.dispatch`: `Mismatching dispatcher: "${dispatcher._type}" does not implement "${method}"`
        _ => {
            return (
                None,
                Err(PlaywrightError::error(format!(
                    "Mismatching dispatcher: \"JSHandle\" does not implement \"{method}\""
                ))),
            );
        }
    };
    (Some(meta), call)
}

fn decode<T: for<'de> Deserialize<'de>>(
    params: Option<&Value>,
    meta: &MethodMeta,
) -> Result<T, PlaywrightError> {
    serde_json::from_value(params.cloned().unwrap_or_else(|| json!({}))).map_err(|error| {
        PlaywrightError::error(format!(
            "invalid params for {}.{}: {error}",
            meta.interface, meta.method
        ))
    })
}

/// One run's handles: guid → what it names, and the run's handle group of each session.
pub(crate) struct Handles {
    /// This run's random token in every guid it mints: runs in other processes (each `neo_run`)
    /// count handles from 1 too, and a guid kept from one must never name another's handle.
    run: String,
    entries: Mutex<HashMap<String, Arc<HandleEntry>>>,
    /// Keyed by the host session. Dropping a guard releases its group (`page.rs`).
    groups: Mutex<HashMap<SessionId, Arc<GroupGuard>>>,
    /// Set at run end: no handle is created after the run's groups are released.
    closed: AtomicBool,
}

/// What a guid names.
pub(crate) struct HandleEntry {
    /// The handle's frame, as a frame guid (`frame@T` or `frame@T/F`), which routes its calls.
    frame_guid: String,
    /// The frame document the handle was created in, and the session that owns its object.
    frame: Arc<FrameContext>,
    document: DocumentId,
    remote: Remote,
}

/// The page object, or for a primitive the value itself, as Playwright keeps `JSHandle._value`
/// for a result without an `objectId`.
#[derive(Debug, Clone)]
enum Remote {
    Object(HandleId),
    /// A CDP `CallArgument` carrying the value: `{value}`, `{unserializableValue}` or `{}` for
    /// `undefined`.
    Value(Value),
}

impl Default for Handles {
    fn default() -> Self {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(NEXT_RUN.fetch_add(1, Ordering::Relaxed));
        Self {
            run: format!("{:016x}", hasher.finish()),
            entries: Mutex::default(),
            groups: Mutex::default(),
            closed: AtomicBool::default(),
        }
    }
}

impl Handles {
    /// The frame guid that routes calls on `guid`, `None` when the run has no such handle.
    pub(crate) fn frame_guid(&self, guid: &str) -> Option<String> {
        self.entry(guid).map(|entry| entry.frame_guid.clone())
    }

    fn entry(&self, guid: &str) -> Option<Arc<HandleEntry>> {
        self.entries.lock().ok()?.get(guid).cloned()
    }

    /// Releases every handle group the run created and forgets every guid. Called at run end;
    /// the releases run on the pages' tracked tasks (`GroupGuard`).
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
        let groups = self
            .groups
            .lock()
            .map(|mut groups| mem::take(&mut *groups))
            .unwrap_or_default();
        drop(groups);
    }

    /// The run's handle group of `frame`'s session, created on first use.
    fn group(
        &self,
        tab: &FrameTab,
        frame: &Arc<FrameContext>,
    ) -> Result<Arc<GroupGuard>, PageError> {
        let mut groups = self.groups.lock().map_err(|_| table_error())?;
        if self.closed.load(Ordering::SeqCst) {
            return Err(PageError::Closed);
        }
        Ok(Arc::clone(
            groups
                .entry(frame.host.session_id().clone())
                .or_insert_with(|| Arc::new(tab.group(frame, "handles"))),
        ))
    }

    fn register(
        &self,
        tab: &FrameTab,
        frame: &Arc<FrameContext>,
        document: DocumentId,
        remote: Remote,
        element: bool,
        preview: String,
    ) -> Result<CreatedHandle, PageError> {
        let guid = format!(
            "{HANDLE_PREFIX}{}.{}",
            self.run,
            NEXT_HANDLE.fetch_add(1, Ordering::Relaxed)
        );
        let main = tab.frames.main();
        let frame_guid = if frame.id == main.id {
            format!("frame@{}", main.id)
        } else {
            format!("frame@{}/{}", main.id, frame.id)
        };
        let mut entries = self.entries.lock().map_err(|_| table_error())?;
        if self.closed.load(Ordering::SeqCst) {
            return Err(PageError::Closed);
        }
        entries.insert(
            guid.clone(),
            Arc::new(HandleEntry {
                frame_guid,
                frame: Arc::clone(frame),
                document,
                remote,
            }),
        );
        Ok(CreatedHandle {
            kind: if element {
                HandleType::ElementHandle
            } else {
                HandleType::JSHandle
            },
            guid,
            initializer: HandleInitializer { preview },
        })
    }

    fn forget(&self, guid: &str) -> Option<Arc<HandleEntry>> {
        self.entries.lock().ok()?.remove(guid)
    }
}

fn table_error() -> PageError {
    PageError::Contract("neo-runtime: the run's handles cannot be accessed".to_string())
}

/// Serves one routed evaluation command on `tab` and encodes its reply.
///
/// # Errors
///
/// The call's [`CallError`]: Playwright's errors for page exceptions, destroyed contexts,
/// foreign handles; `TargetClosedError` for a handle the run does not have.
pub(crate) async fn serve(
    tab: &FrameTab,
    call: EvalCall,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let handles = progress.handles();
    let result = match call {
        EvalCall::Frame(evaluation) => {
            let frame = tab.context().await.map_err(page_error)?;
            evaluate(tab, &frame, None, &evaluation, handles, progress).await?
        }
        EvalCall::Handle { guid, evaluation } => {
            let receiver = handles
                .entry(&guid)
                .ok_or_else(PlaywrightError::target_closed)?;
            let frame = Arc::clone(&receiver.frame);
            evaluate(tab, &frame, Some(&receiver), &evaluation, handles, progress).await?
        }
        EvalCall::JsonValue { guid } => {
            let receiver = handles
                .entry(&guid)
                .ok_or_else(PlaywrightError::target_closed)?;
            json_value(tab, &receiver, handles, progress).await?
        }
        EvalCall::Dispose { guid } => {
            // Playwright's server disposes and forgets the dispatcher; the client treats a
            // second dispose's TargetClosedError as done.
            let Some(entry) = handles.forget(&guid) else {
                return Err(PlaywrightError::target_closed().into());
            };
            if let Remote::Object(id) = &entry.remote {
                // Playwright: `server/chromium/crProtocolHelper.ts` `releaseObject`: `await client.send('Runtime.releaseObject', { objectId }).catch(error => { });`
                let _released = entry
                    .frame
                    .host
                    .send(
                        "Runtime.releaseObject",
                        json!({ "objectId": id.object_id() }),
                    )
                    .await;
            }
            return Ok(None);
        }
    };
    Ok(Some(match result {
        EvalResult::Value(Some(value)) => json!({ "value": value }),
        EvalResult::Value(None) => json!({}),
        EvalResult::Handle(created) => json!({ "handle": created }),
    }))
}

/// Playwright's evaluation in `frame`'s main world, with an optional handle receiver
/// (`JSHandle.evaluateExpression` passes `this` first).
/// Playwright: `server/javascript.ts` `evaluateExpression`
///
/// # Errors
///
/// See [`serve`].
pub(crate) async fn evaluate(
    tab: &FrameTab,
    frame: &Arc<FrameContext>,
    receiver: Option<&HandleEntry>,
    evaluation: &Evaluation,
    handles: &Handles,
    progress: &Progress,
) -> Result<EvalResult, CallError> {
    let args = resolve_args(&evaluation.arg.handles, handles)?;
    let group = if evaluation.return_by_value {
        None
    } else {
        Some(handles.group(tab, frame).map_err(page_error)?)
    };
    let mut attempt = 0;
    loop {
        attempt += 1;
        let installed = installed(tab, frame, receiver, progress).await?;
        let mut handle_args = Vec::with_capacity(args.len() + 1);
        for arg in &args {
            handle_args.push(call_argument(arg, frame, &installed)?);
        }
        // `serializeAsCallArgument(this)` and the arg, in Playwright's order; the receiver is the
        // last handle, so the SDK's `{h: i}` indices hold as they are.
        let mut values = vec![
            match evaluation.is_function {
                Some(flag) => json!({ "value": flag }),
                None => json!({}),
            },
            json!({ "value": evaluation.return_by_value }),
            json!({ "value": evaluation.expression }),
        ];
        if let Some(receiver) = receiver {
            values.push(json!({ "value": 2 }));
            values.push(json!({ "value": { "h": handle_args.len() } }));
            values.push(json!({ "value": evaluation.arg.value }));
            handle_args.push(call_argument(receiver, frame, &installed)?);
        } else {
            values.push(json!({ "value": 1 }));
            values.push(json!({ "value": evaluation.arg.value }));
        }
        values.extend(handle_args);
        let called = progress
            .race(frame.main.call(
                frame,
                &installed,
                MainFunction::Evaluate,
                values,
                evaluation.return_by_value,
                group.as_deref().map(GroupGuard::group),
            ))
            .await?
            .map_err(page_error)?;
        match called {
            Called::Stale if attempt == 1 => frame.main.invalidate(&installed).await,
            called => {
                return answer(tab, frame, installed.document, called, handles, progress).await;
            }
        }
    }
}

/// Playwright's `JSHandle.jsonValue`: the page's `UtilityScript.jsonValue(true, handle)`.
async fn json_value(
    tab: &FrameTab,
    receiver: &HandleEntry,
    handles: &Handles,
    progress: &Progress,
) -> Result<EvalResult, CallError> {
    let frame = Arc::clone(&receiver.frame);
    let mut attempt = 0;
    loop {
        attempt += 1;
        let installed = installed(tab, &frame, Some(receiver), progress).await?;
        let values = vec![
            json!({ "value": true }),
            call_argument(receiver, &frame, &installed)?,
        ];
        let called = progress
            .race(frame.main.call(
                &frame,
                &installed,
                MainFunction::JsonValue,
                values,
                true,
                None,
            ))
            .await?
            .map_err(page_error)?;
        match called {
            Called::Stale if attempt == 1 => frame.main.invalidate(&installed).await,
            called => {
                return answer(tab, &frame, installed.document, called, handles, progress).await;
            }
        }
    }
}

/// The frame's main-world install, waiting out a document that is not there yet as Playwright's
/// `frame._context('main')` waits for the context, with loop A's backoff. A receiver whose
/// document is gone fails as Playwright's destroyed context does.
async fn installed(
    tab: &FrameTab,
    frame: &Arc<FrameContext>,
    receiver: Option<&HandleEntry>,
    progress: &Progress,
) -> Result<Arc<Installed>, CallError> {
    let mut backoff = LoopAWaits::new(progress.timeout_ms());
    loop {
        progress.wait(backoff.next_wait()).await?;
        match progress.race(frame.main.installed(frame)).await? {
            Ok(installed) => {
                if let Some(receiver) = receiver
                    && matches!(receiver.remote, Remote::Object(_))
                    && receiver.document != installed.document
                {
                    // Playwright: `server/frames.ts` `Frame.contextDestroyed`: `context.contextDestroyed('Execution context was destroyed, most likely because of a navigation');`
                    return Err(PlaywrightError::error(
                        "Execution context was destroyed, most likely because of a navigation",
                    )
                    .into());
                }
                return Ok(installed);
            }
            Err(PageError::NotConnected) if receiver.is_none() => {
                // A child frame may have been removed meanwhile: re-resolve its context.
                if !matches!(tab.scope, FrameScope::Main) {
                    tab.context().await.map_err(page_error)?;
                }
            }
            Err(PageError::NotConnected) => {
                return Err(PlaywrightError::error(
                    "Execution context was destroyed, most likely because of a navigation",
                )
                .into());
            }
            Err(error) => return Err(page_error(error).into()),
        }
    }
}

/// The run's entries for `guids`, refusing one the run does not have as Playwright's validator
/// does.
fn resolve_args(guids: &[String], handles: &Handles) -> Result<Vec<Arc<HandleEntry>>, CallError> {
    guids
        .iter()
        .enumerate()
        .map(|(index, guid)| {
            handles.entry(guid).ok_or_else(|| {
                // Playwright: `server/dispatchers/dispatcher.ts` `DispatcherConnection._tChannelImplFromWire`: `${path}: no object with guid ${guid}`
                PlaywrightError::error(format!("arg.handles[{index}]: no object with guid {guid}"))
                    .into()
            })
        })
        .collect()
}

/// A handle as a CDP `CallArgument` in `frame`'s current document: its object, or, for a
/// primitive, its value, which Playwright passes by value from any context
/// (`serializeAsCallArgument`'s `{ fallThrough: handle._value }`).
fn call_argument(
    handle: &HandleEntry,
    frame: &FrameContext,
    installed: &Installed,
) -> Result<Value, CallError> {
    match &handle.remote {
        Remote::Value(argument) => Ok(argument.clone()),
        Remote::Object(id) => {
            if handle.frame.host.session_id() != frame.host.session_id()
                || handle.document != installed.document
            {
                return Err(PlaywrightError::error(OTHER_CONTEXT).into());
            }
            Ok(json!({ "objectId": id.object_id() }))
        }
    }
}

/// Playwright's reading of an evaluation's answer: `rewriteError` for a protocol error,
/// `JavaScriptErrorInEvaluate` for a page exception, `createHandle` for a by-handle result.
async fn answer(
    tab: &FrameTab,
    frame: &Arc<FrameContext>,
    document: DocumentId,
    called: Called,
    handles: &Handles,
    progress: &Progress,
) -> Result<EvalResult, CallError> {
    match called {
        Called::Value(value) => Ok(EvalResult::Value(value)),
        Called::Object(object) => create_handle(tab, frame, document, *object, handles, progress)
            .await
            .map(EvalResult::Handle),
        Called::Threw(message) => Err(PlaywrightError::error(message).into()),
        Called::Protocol(message) => {
            if CHAIN_TOO_LONG_CDP.iter().any(|text| message.contains(text)) {
                Err(PlaywrightError::error(CHAIN_TOO_LONG).into())
            } else if message.contains(NOT_BY_VALUE_CDP) {
                // Playwright answers `{ result: { type: 'undefined' } }`.
                Ok(EvalResult::Value(None))
            } else if message.contains(PROMISE_COLLECTED_CDP) {
                Err(PlaywrightError::error(PROMISE_COLLECTED).into())
            } else {
                Err(PlaywrightError::error(CONTEXT_DESTROYED).into())
            }
        }
        Called::Stale => Err(PlaywrightError::error(CONTEXT_DESTROYED).into()),
    }
}

/// A by-handle result as a new handle: a node is an `ElementHandle`, whose preview is `JSHandle@`
/// plus the utility world's `previewNode` (Playwright fills it in after creation; Neo before
/// replying); anything else a `JSHandle` with `renderPreview`'s text.
/// Playwright: `server/chromium/crExecutionContext.ts` `createHandle`
async fn create_handle(
    tab: &FrameTab,
    frame: &Arc<FrameContext>,
    document: DocumentId,
    object: RemoteObject,
    handles: &Handles,
    progress: &Progress,
) -> Result<CreatedHandle, CallError> {
    let Some(object_id) = object.object_id.clone() else {
        let (argument, preview) = primitive(&object);
        return handles
            .register(
                tab,
                frame,
                document,
                Remote::Value(argument),
                false,
                preview,
            )
            .map_err(|error| page_error(error).into());
    };
    let element = object.subtype.as_deref() == Some(NODE_TYPE);
    let preview = if element {
        // Deviation: `server/dom.ts` `ElementHandle._initializePreview`: `this._setPreview(await utility.evaluate((injected, e) => 'JSHandle@' + injected.previewNode(e), this));` Playwright starts at `JSHandle@node` and updates the preview by event; Neo has no events, so the handle starts with the final preview.
        match progress.race(node_preview(tab, frame, &object_id)).await? {
            Some(preview) => format!("{NODE_PREVIEW_PREFIX}{preview}"),
            None => format!("{NODE_PREVIEW_PREFIX}{NODE_TYPE}"),
        }
    } else {
        render_preview(&object)
    };
    handles
        .register(
            tab,
            frame,
            document,
            Remote::Object(HandleId::new(object_id)),
            element,
            preview,
        )
        .map_err(|error| page_error(error).into())
}

/// The utility world's `previewNode` of a main-world node, `None` when the node cannot be
/// adopted (Playwright keeps `JSHandle@node` when `_initializePreview` fails).
async fn node_preview(
    tab: &FrameTab,
    frame: &Arc<FrameContext>,
    object_id: &str,
) -> Option<String> {
    let backend = frame.main.backend_of(frame, object_id).await.ok()?;
    let group = tab.group(frame, "preview");
    let node = frame.element(group.group(), backend).await.ok()?;
    frame
        .call(None, &InjectedPreviewNodeParams { node })
        .await
        .ok()
        .map(|result| result.preview)
}

/// A primitive result as Playwright's `JSHandle` keeps it (`potentiallyUnserializableValue`:
/// `-0`, `NaN` and `±Infinity` parsed, any other unserializable value such as a `BigInt` read as
/// `undefined`), as a CDP `CallArgument`, and its preview, JavaScript's `String(value)`.
fn primitive(object: &RemoteObject) -> (Value, String) {
    // `null` arrives as `value: null`, which serde reads as no value.
    if object.subtype.as_deref() == Some("null") {
        return (json!({ "value": null }), "null".to_string());
    }
    if let Some(unserializable) = object.unserializable_value.as_deref() {
        return match unserializable {
            // `String(-0)` is "0".
            "-0" => (json!({ "unserializableValue": "-0" }), "0".to_string()),
            "NaN" | "Infinity" | "-Infinity" => (
                json!({ "unserializableValue": unserializable }),
                unserializable.to_string(),
            ),
            _ => (json!({}), "undefined".to_string()),
        };
    }
    match &object.value {
        None => (json!({}), "undefined".to_string()),
        Some(value) => (json!({ "value": value }), js_string(value)),
    }
}

/// Playwright: `server/chromium/crExecutionContext.ts` `renderPreview`: `return object.description;`
fn render_preview(object: &RemoteObject) -> String {
    let preview = if object.r#type == "undefined" {
        Some("undefined".to_string())
    } else if let Some(value) = &object.value {
        Some(js_string(value))
    } else if let Some(unserializable) = &object.unserializable_value {
        Some(unserializable.clone())
    } else {
        object.description.clone()
    };
    // Playwright: `server/javascript.ts` `JSHandle.constructor`: `JSHandle@${this._objectType}`
    preview
        .filter(|preview| !preview.is_empty())
        .unwrap_or_else(|| {
            format!(
                "JSHandle@{}",
                object.subtype.as_deref().unwrap_or(object.r#type.as_str())
            )
        })
}

/// JavaScript's `String(value)` for a value CDP returned by value.
pub(crate) fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number
            .as_f64()
            .map_or_else(|| number.to_string(), js_number),
        Value::String(text) => text.clone(),
        Value::Array(_) | Value::Object(_) => "[object Object]".to_string(),
    }
}

/// Adopts a utility-world element of `frame` into the main world as a new handle of the run, as
/// Playwright's `waitForSelector` adopts its element (`element._adoptTo(mainContext)`). The
/// preview is the one the utility world computed for the match.
///
/// # Errors
///
/// `NotConnected` when the element or its document went away, so a wait keeps polling.
pub(crate) async fn element_handle_for(
    tab: &FrameTab,
    frame: &Arc<FrameContext>,
    element: &Handle<Element>,
    preview: &str,
    handles: &Handles,
) -> Result<CreatedHandle, PageError> {
    let group = handles.group(tab, frame)?;
    let (document, object) = frame.main.adopt(frame, element, group.group()).await?;
    let object_id = object.object_id.ok_or(PageError::NotConnected)?;
    handles.register(
        tab,
        frame,
        document,
        Remote::Object(HandleId::new(object_id)),
        true,
        format!("{NODE_PREVIEW_PREFIX}{preview}"),
    )
}

fn page_error(error: PageError) -> PlaywrightError {
    match error {
        PageError::Threw(message) => PlaywrightError::error(message),
        PageError::Closed => PlaywrightError::target_closed(),
        PageError::NotConnected => PlaywrightError::error(CONTEXT_DESTROYED),
        PageError::Contract(message) => PlaywrightError::error(format!("neo-runtime: {message}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(value: Value) -> RemoteObject {
        serde_json::from_value(value).expect("a recorded RemoteObject")
    }

    #[test]
    fn primitives_keep_playwrights_value_and_preview() {
        let cases = [
            (
                json!({"type":"number","value":2,"description":"2"}),
                json!({"value":2}),
                "2",
            ),
            (
                json!({"type":"string","value":"a"}),
                json!({"value":"a"}),
                "a",
            ),
            (
                json!({"type":"boolean","value":true}),
                json!({"value":true}),
                "true",
            ),
            (json!({"type":"undefined"}), json!({}), "undefined"),
            (
                json!({"type":"object","subtype":"null","value":null}),
                json!({"value":null}),
                "null",
            ),
            (
                json!({"type":"number","unserializableValue":"NaN","description":"NaN"}),
                json!({"unserializableValue":"NaN"}),
                "NaN",
            ),
            (
                json!({"type":"number","unserializableValue":"-0","description":"-0"}),
                json!({"unserializableValue":"-0"}),
                "0",
            ),
            (
                json!({"type":"bigint","unserializableValue":"5n","description":"5n"}),
                json!({}),
                "undefined",
            ),
        ];
        for (object, argument, preview) in cases {
            assert_eq!(
                primitive(&remote(object.clone())),
                (argument, preview.to_string()),
                "{object}"
            );
        }
    }

    #[test]
    fn objects_preview_their_description_or_their_type() {
        assert_eq!(
            render_preview(&remote(
                json!({"type":"object","className":"Window","description":"Window","objectId":"1"})
            )),
            "Window"
        );
        assert_eq!(
            render_preview(&remote(
                json!({"type":"object","subtype":"proxy","objectId":"1"})
            )),
            "JSHandle@proxy"
        );
    }

    #[test]
    fn evaluation_params_decode_with_an_opaque_argument() {
        let routed = route_frame(
            "evaluateExpression",
            Some(&json!({"expression":"(a) => a","isFunction":true,"arg":{"value":{"a":[1],"id":1},"handles":["handle@7"]}})),
        )
        .expect("an evaluation");
        let (meta, Ok(EvalCall::Frame(evaluation))) = routed else {
            panic!("expected a frame evaluation");
        };
        assert_eq!(meta.method, "evaluateExpression");
        assert!(evaluation.return_by_value);
        assert_eq!(evaluation.arg.value, json!({"a":[1],"id":1}));
        assert_eq!(evaluation.arg.handles, ["handle@7"]);
        assert!(route_frame("click", None).is_none());
        let (_, refused) =
            route_frame("evaluateExpressionHandle", Some(&json!({"expression":"1"})))
                .expect("an evaluation");
        assert!(
            refused.is_err(),
            "arg is required, as in Playwright's protocol"
        );
    }

    #[test]
    fn handle_commands_route_and_unknown_methods_are_refused() {
        let (meta, call) = route_handle("handle@1", "jsonValue", None);
        assert_eq!(meta.map(|meta| meta.title), Some(Some("Get JSON value")));
        assert!(matches!(call, Ok(EvalCall::JsonValue { .. })));
        let (meta, call) = route_handle("handle@1", "dispose", Some(&json!({})));
        assert!(meta.is_some_and(|meta| meta.internal));
        assert!(matches!(call, Ok(EvalCall::Dispose { .. })));
        let (meta, call) = route_handle("handle@1", "getProperty", Some(&json!({"name":"a"})));
        assert!(meta.is_none());
        assert!(call.is_err());
    }

    #[test]
    fn a_closed_table_names_nothing() {
        let handles = Handles::default();
        handles.close();
        assert!(handles.frame_guid("handle@1").is_none());
    }
}
