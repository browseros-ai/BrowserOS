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
use std::future::Future;
use std::hash::{BuildHasher, Hasher};
use std::mem;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use browseros_cdp::SessionId;
use browseros_cdp::runtime::{GetPropertiesResult, RemoteObject};
use neo_runtime_spec::injected::{
    InjectedListPreviewsParams, InjectedPreviewNodeParams, InjectedQueryAllParams,
    InjectedScopedQueryAllParams, InjectedScopedSelectorElementParams,
    InjectedSelectorElementParams,
};
use neo_runtime_spec::{Element, Handle, HandleId, MethodFlag, MethodGroup, MethodMeta};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time;

use crate::click::{LoopAWaits, PageError};
use crate::frame_selector;
use crate::frames::{FrameContext, FrameScope};
use crate::main_world::{self, BuiltExpression, Called, DocumentId, Installed, MainFunction};
use crate::page::{FrameTab, GroupGuard};
use crate::progress::{CallError, PlaywrightError, Progress, js_number};
use crate::world::ObjectGroup;

/// Neo: the guid prefix of a run's handles (`handle@<run>.<n>`), opaque to the SDK.
pub(crate) const HANDLE_PREFIX: &str = "handle@";

/// Neo: the next handle number, process-wide, so a guid never names two handles in one process.
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

/// Neo: counts runs, salting each run's random token, so two runs never share one.
static NEXT_RUN: AtomicU64 = AtomicU64::new(1);

/// Playwright: `server/frames.ts` `Frame.waitForFunctionExpression`: `return this.retryWithProgressAndTimeouts(progress, [100], async () => {`
const WAIT_FOR_FUNCTION_RETRY: Duration = Duration::from_millis(100);

/// Neo: how long a `waitForFunction` reply waits for its page polling to stop before replying
/// anyway (a frozen renderer); Playwright waits without a bound.
const WAITER_ABORT_BOUND: Duration = Duration::from_secs(2);

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

/// `querySelector`'s params (`Frame` and `ElementHandle`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QueryParams {
    selector: String,
    #[serde(default)]
    strict: Option<bool>,
}

/// `querySelectorAll`'s params.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QueryAllParams {
    selector: String,
}

/// `evalOnSelector`'s params.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EvalOnSelectorParams {
    selector: String,
    #[serde(default)]
    strict: Option<bool>,
    expression: String,
    #[serde(default)]
    is_function: Option<bool>,
    arg: SerializedArgument,
}

/// `evalOnSelectorAll`'s params.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EvalOnSelectorAllParams {
    selector: String,
    expression: String,
    #[serde(default)]
    is_function: Option<bool>,
    arg: SerializedArgument,
}

/// `Frame.waitForFunction`'s params. `selector` and `strict` belong to `Locator.waitForFunction`,
/// which runs on an element in the main world's injected script: refused until that lands.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WaitForFunctionParams {
    expression: String,
    #[serde(default)]
    is_function: Option<bool>,
    arg: SerializedArgument,
    #[serde(default)]
    polling_interval: Option<f64>,
    #[serde(default)]
    selector: Option<String>,
    #[serde(default)]
    strict: Option<bool>,
}

/// `JSHandle.getProperty`'s params.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetPropertyParams {
    name: String,
}

/// Neo: `Frame.querySelector`'s trace metadata, as `neo-runtime-spec/spec/playwright/frame.yml`
/// declares it.
pub(crate) static FRAME_QUERY_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "querySelector",
    title: Some("Query selector"),
    subtitle: Some("{selector}"),
    group: None,
    render_params: &[],
    internal: false,
    flags: &[MethodFlag::Snapshot],
};
/// Neo: `Frame.querySelectorAll`'s trace metadata (`neo-runtime-spec/spec/playwright/frame.yml`).
pub(crate) static FRAME_QUERY_ALL_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "querySelectorAll",
    title: Some("Query selector all"),
    subtitle: Some("{selector}"),
    group: None,
    render_params: &[],
    internal: false,
    flags: &[MethodFlag::Snapshot],
};
/// Neo: `Frame.evalOnSelector`'s trace metadata (`neo-runtime-spec/spec/playwright/frame.yml`).
pub(crate) static FRAME_EVAL_ON_SELECTOR_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "evalOnSelector",
    title: Some("Evaluate"),
    subtitle: Some("{selector}"),
    group: None,
    render_params: &[],
    internal: false,
    flags: &[MethodFlag::Snapshot, MethodFlag::Pause],
};
/// Neo: `Frame.evalOnSelectorAll`'s trace metadata (`neo-runtime-spec/spec/playwright/frame.yml`).
pub(crate) static FRAME_EVAL_ON_SELECTOR_ALL_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "evalOnSelectorAll",
    title: Some("Evaluate"),
    subtitle: Some("{selector}"),
    group: None,
    render_params: &[],
    internal: false,
    flags: &[MethodFlag::Snapshot, MethodFlag::Pause],
};
/// Neo: `Frame.waitForFunction`'s trace metadata (`neo-runtime-spec/spec/playwright/frame.yml`).
pub(crate) static FRAME_WAIT_FOR_FUNCTION_META: MethodMeta = MethodMeta {
    interface: "Frame",
    method: "waitForFunction",
    title: Some("Wait for function"),
    subtitle: Some("{selector}"),
    group: None,
    render_params: &[],
    internal: false,
    flags: &[MethodFlag::Snapshot, MethodFlag::Pause],
};
/// Neo: `ElementHandle.querySelector`'s trace metadata (`neo-runtime-spec/spec/playwright/handles.yml`).
pub(crate) static ELEMENT_QUERY_META: MethodMeta = MethodMeta {
    interface: "ElementHandle",
    method: "querySelector",
    title: Some("Query selector"),
    subtitle: Some("{selector}"),
    group: None,
    render_params: &[],
    internal: false,
    flags: &[MethodFlag::Snapshot],
};
/// Neo: `ElementHandle.querySelectorAll`'s trace metadata (`neo-runtime-spec/spec/playwright/handles.yml`).
pub(crate) static ELEMENT_QUERY_ALL_META: MethodMeta = MethodMeta {
    interface: "ElementHandle",
    method: "querySelectorAll",
    title: Some("Query selector all"),
    subtitle: Some("{selector}"),
    group: None,
    render_params: &[],
    internal: false,
    flags: &[MethodFlag::Snapshot],
};
/// Neo: `ElementHandle.evalOnSelector`'s trace metadata (`neo-runtime-spec/spec/playwright/handles.yml`).
pub(crate) static ELEMENT_EVAL_ON_SELECTOR_META: MethodMeta = MethodMeta {
    interface: "ElementHandle",
    method: "evalOnSelector",
    title: Some("Evaluate"),
    subtitle: Some("{selector}"),
    group: None,
    render_params: &[],
    internal: false,
    flags: &[MethodFlag::Snapshot, MethodFlag::Pause],
};
/// Neo: `ElementHandle.evalOnSelectorAll`'s trace metadata (`neo-runtime-spec/spec/playwright/handles.yml`).
pub(crate) static ELEMENT_EVAL_ON_SELECTOR_ALL_META: MethodMeta = MethodMeta {
    interface: "ElementHandle",
    method: "evalOnSelectorAll",
    title: Some("Evaluate"),
    subtitle: Some("{selector}"),
    group: None,
    render_params: &[],
    internal: false,
    flags: &[MethodFlag::Snapshot, MethodFlag::Pause],
};
/// Neo: `JSHandle.getProperty`'s trace metadata (`neo-runtime-spec/spec/playwright/handles.yml`).
pub(crate) static HANDLE_GET_PROPERTY_META: MethodMeta = MethodMeta {
    interface: "JSHandle",
    method: "getProperty",
    title: Some("Get JS property"),
    subtitle: None,
    group: Some(MethodGroup::Getter),
    render_params: &[],
    internal: false,
    flags: &[],
};
/// Neo: `JSHandle.getPropertyList`'s trace metadata (`neo-runtime-spec/spec/playwright/handles.yml`).
pub(crate) static HANDLE_GET_PROPERTY_LIST_META: MethodMeta = MethodMeta {
    interface: "JSHandle",
    method: "getPropertyList",
    title: Some("Get property list"),
    subtitle: None,
    group: Some(MethodGroup::Getter),
    render_params: &[],
    internal: false,
    flags: &[],
};

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
    /// `Frame.querySelector` / `ElementHandle.querySelector` (`scope` names the element).
    Query {
        scope: Option<String>,
        selector: String,
        strict: bool,
    },
    /// `Frame.querySelectorAll` / `ElementHandle.querySelectorAll`.
    QueryAll {
        scope: Option<String>,
        selector: String,
    },
    /// `Frame.evalOnSelector` / `ElementHandle.evalOnSelector` (`$eval`).
    EvalOnSelector {
        scope: Option<String>,
        selector: String,
        strict: bool,
        evaluation: Evaluation,
    },
    /// `Frame.evalOnSelectorAll` / `ElementHandle.evalOnSelectorAll` (`$$eval`, `evaluateAll`).
    EvalOnSelectorAll {
        scope: Option<String>,
        selector: String,
        evaluation: Evaluation,
    },
    /// `Frame.waitForFunction`.
    WaitForFunction {
        evaluation: Evaluation,
        polling: Option<f64>,
    },
    /// `JSHandle.getProperty`.
    GetProperty {
        guid: String,
        name: String,
    },
    /// `JSHandle.getPropertyList`.
    GetPropertyList {
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
    if let Some(routed) = route_selector_call(None, method, params) {
        return routed.0.map(|meta| (meta, routed.1));
    }
    if method == "waitForFunction" {
        let meta = &FRAME_WAIT_FOR_FUNCTION_META;
        return Some((
            meta,
            decode::<WaitForFunctionParams>(params, meta).and_then(|params| {
                if params.selector.is_some() || params.strict.is_some() {
                    // Deviation: `server/frames.ts` `Frame.waitForFunctionExpressionOnElement`: `expression = js.normalizeEvaluationExpression(expression, isFunction);` Neo has no main-world injected script yet to run the predicate on an element, so it refuses the selector form.
                    return Err(PlaywrightError::error(
                        "Neo does not support waitForFunction on a selector yet",
                    ));
                }
                Ok(EvalCall::WaitForFunction {
                    evaluation: Evaluation {
                        expression: params.expression,
                        is_function: params.is_function,
                        arg: params.arg,
                        return_by_value: false,
                    },
                    polling: params.polling_interval,
                })
            }),
        ));
    }
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
    if let Some(routed) = route_selector_call(Some(guid), method, params) {
        return routed;
    }
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
        "getProperty" => (
            &HANDLE_GET_PROPERTY_META,
            decode::<GetPropertyParams>(params, &HANDLE_GET_PROPERTY_META).map(|params| {
                EvalCall::GetProperty {
                    guid,
                    name: params.name,
                }
            }),
        ),
        "getPropertyList" => (
            &HANDLE_GET_PROPERTY_LIST_META,
            decode::<NoParams>(params, &HANDLE_GET_PROPERTY_LIST_META)
                .map(|NoParams {}| EvalCall::GetPropertyList { guid }),
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

/// The selector commands `Frame` and `ElementHandle` share (`scope` is the element handle's guid):
/// `querySelector`, `querySelectorAll`, `evalOnSelector`, `evalOnSelectorAll`; `None` for any
/// other method.
fn route_selector_call(
    scope: Option<&str>,
    method: &str,
    params: Option<&Value>,
) -> Option<(
    Option<&'static MethodMeta>,
    Result<EvalCall, PlaywrightError>,
)> {
    let scope = scope.map(str::to_string);
    let element = scope.is_some();
    let pick = |frame: &'static MethodMeta, handle: &'static MethodMeta| {
        if element { handle } else { frame }
    };
    let (meta, call) = match method {
        "querySelector" => {
            let meta = pick(&FRAME_QUERY_META, &ELEMENT_QUERY_META);
            (
                meta,
                decode::<QueryParams>(params, meta).map(|params| EvalCall::Query {
                    scope,
                    selector: params.selector,
                    strict: params.strict.unwrap_or(false),
                }),
            )
        }
        "querySelectorAll" => {
            let meta = pick(&FRAME_QUERY_ALL_META, &ELEMENT_QUERY_ALL_META);
            (
                meta,
                decode::<QueryAllParams>(params, meta).map(|params| EvalCall::QueryAll {
                    scope,
                    selector: params.selector,
                }),
            )
        }
        "evalOnSelector" => {
            let meta = pick(&FRAME_EVAL_ON_SELECTOR_META, &ELEMENT_EVAL_ON_SELECTOR_META);
            (
                meta,
                decode::<EvalOnSelectorParams>(params, meta).map(|params| {
                    EvalCall::EvalOnSelector {
                        scope,
                        selector: params.selector,
                        strict: params.strict.unwrap_or(false),
                        evaluation: Evaluation {
                            expression: params.expression,
                            is_function: params.is_function,
                            arg: params.arg,
                            return_by_value: true,
                        },
                    }
                }),
            )
        }
        "evalOnSelectorAll" => {
            let meta = pick(
                &FRAME_EVAL_ON_SELECTOR_ALL_META,
                &ELEMENT_EVAL_ON_SELECTOR_ALL_META,
            );
            (
                meta,
                decode::<EvalOnSelectorAllParams>(params, meta).map(|params| {
                    EvalCall::EvalOnSelectorAll {
                        scope,
                        selector: params.selector,
                        evaluation: Evaluation {
                            expression: params.expression,
                            is_function: params.is_function,
                            arg: params.arg,
                            return_by_value: true,
                        },
                    }
                }),
            )
        }
        _ => return None,
    };
    Some((Some(meta), call))
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
    /// An `ElementHandle`: only these serve the selector commands, as in Playwright's dispatchers.
    element: bool,
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

    /// Playwright: `server/dispatchers/dispatcher.ts` `DispatcherConnection._tChannelImplFromWire`: `no object with guid ${guid}`
    ///
    /// `selectOption`'s `elements[index]`: the element handle `guid` names, checked as
    /// Playwright's dispatcher checks an `ElementHandle` channel. A disposed handle's guid is
    /// gone, so it names nothing.
    ///
    /// # Errors
    ///
    /// No handle has `guid`, or it is a `JSHandle` that is not an element.
    pub(crate) fn select_element(
        &self,
        index: usize,
        guid: &str,
    ) -> Result<Arc<HandleEntry>, PlaywrightError> {
        let entry = self.entry(guid).ok_or_else(|| {
            PlaywrightError::error(format!("elements[{index}]: no object with guid {guid}"))
        })?;
        if !entry.element {
            // Playwright: `server/dispatchers/dispatcher.ts` `DispatcherConnection._tChannelImplFromWire`: `has type ${dispatcher._type}, expected ${names.toString()}`
            return Err(PlaywrightError::error(format!(
                "elements[{index}]: object with guid {guid} has type JSHandle, expected ElementHandle"
            )));
        }
        Ok(entry)
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
                element,
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
        call @ (EvalCall::Query { .. }
        | EvalCall::QueryAll { .. }
        | EvalCall::EvalOnSelector { .. }
        | EvalCall::EvalOnSelectorAll { .. }) => {
            return serve_selector(tab, call, handles, progress).await;
        }
        EvalCall::WaitForFunction {
            evaluation,
            polling,
        } => {
            let frame = tab.context().await.map_err(page_error)?;
            wait_for_function(tab, &frame, &evaluation, polling, handles, progress).await?
        }
        EvalCall::GetProperty { guid, name } => {
            let receiver = handles
                .entry(&guid)
                .ok_or_else(PlaywrightError::target_closed)?;
            get_property(tab, &receiver, &name, handles, progress).await?
        }
        EvalCall::GetPropertyList { guid } => {
            let receiver = handles
                .entry(&guid)
                .ok_or_else(PlaywrightError::target_closed)?;
            let properties = get_property_list(tab, &receiver, handles, progress).await?;
            return Ok(Some(json!({ "properties": properties })));
        }
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
    let extras: Vec<Extra<'_>> = receiver.map(Extra::Entry).into_iter().collect();
    let (document, called) = invoke(
        tab,
        &Invocation {
            frame,
            receiver,
            expression: &evaluation.expression,
            is_function: evaluation.is_function,
            return_by_value: evaluation.return_by_value,
            // `serializeAsCallArgument(this)`, then the arg; the receiver is the last handle, so
            // the SDK's `{h: i}` indices hold as they are.
            leading: &|base| {
                if receiver.is_some() {
                    vec![json!({ "h": base })]
                } else {
                    Vec::new()
                }
            },
            extras: &extras,
            arg: &evaluation.arg,
        },
        handles,
        progress,
    )
    .await?;
    answer(tab, frame, document, called, handles, progress).await
}

/// A handle `invoke` passes after the agent argument's own handles.
enum Extra<'a> {
    /// A handle of the run, checked against the document as an argument is.
    Entry(&'a HandleEntry),
    /// An object of this call's own, by id: a waiter between its start and its result.
    Object(&'a str),
    /// The document's `UtilityScript`.
    Utility,
}

/// One `UtilityScript.evaluate` call: `leading` page-format arguments before the agent's `arg`
/// (given the agent handles' count, so they can name `extras` as `{h: base + i}`), then `extras`
/// after the agent's handles.
struct Invocation<'a> {
    frame: &'a Arc<FrameContext>,
    /// The handle whose document must still be the frame's (Playwright's destroyed context).
    receiver: Option<&'a HandleEntry>,
    expression: &'a str,
    is_function: Option<bool>,
    return_by_value: bool,
    leading: &'a (dyn Fn(usize) -> Vec<Value> + Sync),
    extras: &'a [Extra<'a>],
    arg: &'a SerializedArgument,
}

/// Runs one invocation in its frame's main world: Playwright's `evaluateWithArguments` over
/// `dist/evaluate.js`, installing the `UtilityScript` again once if it went stale before the call
/// started. By-handle results land in the run's handle group of the frame's session.
async fn invoke(
    tab: &FrameTab,
    invocation: &Invocation<'_>,
    handles: &Handles,
    progress: &Progress,
) -> Result<(DocumentId, Called), CallError> {
    invoke_with(tab, invocation, handles, progress, LateAnswer::Drop).await
}

/// What becomes of a main-world call's answer when the call stops before it arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LateAnswer {
    /// Dropped: the call leaves nothing running in the page.
    Drop,
    /// A waiter (`dist/wait-for-function.js`) that may already poll: awaited in the background
    /// and aborted once it arrives, so nothing polls after the call returned.
    AbortWaiter,
}

/// [`invoke`], with `late` deciding what becomes of an answer that arrives after the call
/// stopped (a timeout, the run's end).
async fn invoke_with(
    tab: &FrameTab,
    invocation: &Invocation<'_>,
    handles: &Handles,
    progress: &Progress,
    late: LateAnswer,
) -> Result<(DocumentId, Called), CallError> {
    let frame = invocation.frame;
    let args = resolve_args(&invocation.arg.handles, handles)?;
    let group = if invocation.return_by_value {
        None
    } else {
        Some(handles.group(tab, frame).map_err(page_error)?)
    };
    let mut attempt = 0;
    loop {
        attempt += 1;
        let installed = installed(tab, frame, invocation.receiver, progress).await?;
        let mut handle_args = Vec::with_capacity(args.len() + invocation.extras.len());
        for arg in &args {
            handle_args.push(call_argument(arg, frame, &installed)?);
        }
        let leading = (invocation.leading)(handle_args.len());
        for extra in invocation.extras {
            handle_args.push(match extra {
                Extra::Entry(entry) => call_argument(entry, frame, &installed)?,
                Extra::Object(id) => json!({ "objectId": id }),
                Extra::Utility => json!({ "objectId": installed.utility_id() }),
            });
        }
        let mut values = vec![
            match invocation.is_function {
                Some(flag) => json!({ "value": flag }),
                None => json!({}),
            },
            json!({ "value": invocation.return_by_value }),
            json!({ "value": invocation.expression }),
            json!({ "value": leading.len() + 1 }),
        ];
        values.extend(leading.into_iter().map(|value| json!({ "value": value })));
        values.push(json!({ "value": invocation.arg.value }));
        values.extend(handle_args);
        let call = {
            let frame = Arc::clone(frame);
            let installed = Arc::clone(&installed);
            let group = group.clone();
            let return_by_value = invocation.return_by_value;
            Box::pin(async move {
                frame
                    .main
                    .call(
                        &frame,
                        &installed,
                        MainFunction::Evaluate,
                        values,
                        return_by_value,
                        group.as_deref().map(GroupGuard::group),
                    )
                    .await
            })
        };
        let called = match late {
            LateAnswer::Drop => progress.race(call).await?,
            LateAnswer::AbortWaiter => race_with_cleanup(tab, frame, progress, call).await?,
        }
        .map_err(page_error)?;
        match called {
            Called::Stale if attempt == 1 => frame.main.invalidate(&installed).await,
            called => return Ok((installed.document, called)),
        }
    }
}

/// Playwright's `raceUncancellableOperationWithCleanup` for a waiter's setup: a call that stops
/// first still receives the waiter in the background, then aborts it.
/// Playwright: `server/frames.ts` `Frame.waitForFunctionExpression`: `await handle.evaluate(h => h.abort()).catch(() => {});`
async fn race_with_cleanup<F>(
    tab: &FrameTab,
    frame: &Arc<FrameContext>,
    progress: &Progress,
    mut call: Pin<Box<F>>,
) -> Result<Result<Called, PageError>, CallError>
where
    F: Future<Output = Result<Called, PageError>> + Send + 'static,
{
    match progress.race(&mut call).await {
        Ok(called) => Ok(called),
        Err(stopped) => {
            let page = tab.clone();
            let frame = Arc::clone(frame);
            tab.spawn(async move {
                if let Ok(Called::Object(waiter)) = call.await
                    && let Some(waiter) = waiter.object_id
                {
                    abort_waiter(page, frame, waiter).await;
                }
            });
            Err(stopped)
        }
    }
}

/// The selector commands' share of [`serve`].
async fn serve_selector(
    tab: &FrameTab,
    call: EvalCall,
    handles: &Handles,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let result = match call {
        EvalCall::Query {
            scope,
            selector,
            strict,
        } => {
            let scope = element_scope(handles, scope.as_deref(), "querySelector")?;
            let created =
                query_one(tab, scope.as_deref(), &selector, strict, handles, progress).await?;
            return Ok(Some(match created {
                Some(created) => json!({ "element": created }),
                None => json!({}),
            }));
        }
        EvalCall::QueryAll { scope, selector } => {
            let scope = element_scope(handles, scope.as_deref(), "querySelectorAll")?;
            let created = query_all(tab, scope.as_deref(), &selector, handles, progress)
                .await?
                .map_or_else(Vec::new, |(_, created)| created);
            return Ok(Some(json!({ "elements": created })));
        }
        EvalCall::EvalOnSelector {
            scope,
            selector,
            strict,
            evaluation,
        } => {
            let scope = element_scope(handles, scope.as_deref(), "evalOnSelector")?;
            eval_on_selector(
                tab,
                scope.as_deref(),
                &selector,
                strict,
                &evaluation,
                handles,
                progress,
            )
            .await?
        }
        EvalCall::EvalOnSelectorAll {
            scope,
            selector,
            evaluation,
        } => {
            let scope = element_scope(handles, scope.as_deref(), "evalOnSelectorAll")?;
            eval_on_selector_all(
                tab,
                scope.as_deref(),
                &selector,
                &evaluation,
                handles,
                progress,
            )
            .await?
        }
        _ => return Err(PlaywrightError::error("neo-runtime: not a selector command").into()),
    };
    Ok(Some(match result {
        EvalResult::Value(Some(value)) => json!({ "value": value }),
        EvalResult::Value(None) => json!({}),
        EvalResult::Handle(created) => json!({ "handle": created }),
    }))
}

/// The element handle a selector command is scoped to, refusing a `JSHandle` as Playwright's
/// dispatcher does.
fn element_scope(
    handles: &Handles,
    scope: Option<&str>,
    method: &str,
) -> Result<Option<Arc<HandleEntry>>, CallError> {
    let Some(guid) = scope else {
        return Ok(None);
    };
    let entry = handles
        .entry(guid)
        .ok_or_else(PlaywrightError::target_closed)?;
    if !entry.element {
        // Playwright: `server/dispatchers/dispatcher.ts` `DispatcherConnection.dispatch`: `Mismatching dispatcher: "${dispatcher._type}" does not implement "${method}"`
        return Err(PlaywrightError::error(format!(
            "Mismatching dispatcher: \"JSHandle\" does not implement \"{method}\""
        ))
        .into());
    }
    Ok(Some(entry))
}

/// Playwright: `server/dom.ts` `FrameExecutionContext.adoptIfNeeded`: `return this.frame._page.delegate.adoptElementHandle(handle, this);`
///
/// `entry`'s element adopted into `frame`'s utility world in `group`, as Playwright adopts
/// `selectOption`'s `ElementHandle` values into the select's utility context for each attempt:
/// the LIVE node, wherever it now is (`DOM.describeNode` where the handle's object lives, then
/// `DOM.resolveNode` in the select's world), so an option moved into the select from another
/// document of the same process is still the one its handle names. `None` when the node cannot
/// be resolved there: another process's (another session's), or one whose object is gone, which
/// Playwright's adoption refuses (`kUnableToAdoptErrorMessage`).
///
/// # Errors
///
/// `Closed` when the tab is gone.
pub(crate) async fn adopt_element(
    entry: &HandleEntry,
    frame: &Arc<FrameContext>,
    group: &ObjectGroup,
) -> Result<Option<Handle<Element>>, PageError> {
    let Remote::Object(id) = &entry.remote else {
        return Ok(None);
    };
    // A backend node id names a node in its own process only.
    // Deviation: `server/dom.ts` `FrameExecutionContext.adoptIfNeeded`: `return this.frame._page.delegate.adoptElementHandle(handle, this);` A handle of another session (an out-of-process frame's, another tab's) fails Playwright's adoption, which `_retryWithProgressIfNotConnected` retries until the call times out (TimeoutError); Neo refuses it at once with Playwright's adoption Error, since a script gains nothing from waiting its whole timeout to learn the handle is unusable. Neither selects anything.
    if entry.frame.host.session_id() != frame.host.session_id() {
        return Ok(None);
    }
    let backend = match entry
        .frame
        .main
        .backend_of(&entry.frame, id.object_id())
        .await
    {
        Ok(backend) => backend,
        Err(PageError::Closed) => return Err(PageError::Closed),
        // The handle's object went with its document.
        Err(_) => return Ok(None),
    };
    match frame.world.element(group, backend).await {
        Ok(node) => Ok(Some(node)),
        Err(PageError::Closed) => Err(PageError::Closed),
        Err(_) => Ok(None),
    }
}

/// An element handle adopted into its frame's utility world, as the root of a scoped query, in
/// `group`. Frame-crossing selectors from an element are not served.
async fn utility_root(
    root: &HandleEntry,
    selector: &str,
    group: &GroupGuard,
) -> Result<Handle<Element>, CallError> {
    let chunks = frame_selector::split(selector).map_err(PlaywrightError::error)?;
    if !chunks.hops.is_empty() || chunks.from_main {
        // Deviation: `server/frameSelectors.ts` `FrameSelectors._resolveChainedSelector`: `const frames = await this._queryFrames(frame, info, i === 0 ? scope : undefined, noStall, frameVisible);` Playwright resolves frame hops from a scope too; Neo serves a scoped query within the element's own document only.
        return Err(PlaywrightError::error(
            "Neo does not support selectors that enter frames from an element handle yet",
        )
        .into());
    }
    let Remote::Object(id) = &root.remote else {
        return Err(PlaywrightError::error(CONTEXT_DESTROYED).into());
    };
    let frame = &root.frame;
    let backend = frame
        .main
        .backend_of(frame, id.object_id())
        .await
        .map_err(page_error)?;
    frame
        .element(group.group(), backend)
        .await
        .map_err(|error| page_error(error).into())
}

/// The first match of `selector` (strict when asked), in the frame the selector ends in, adopted
/// into that frame's main world; `None` when nothing matched. With `scope`, within that element.
/// Playwright: `server/frameSelectors.ts` `FrameSelectors.query`
async fn query_one(
    tab: &FrameTab,
    scope: Option<&HandleEntry>,
    selector: &str,
    strict: bool,
    handles: &Handles,
    progress: &Progress,
) -> Result<Option<CreatedHandle>, CallError> {
    let (frame, element, preview, _group) = if let Some(root) = scope {
        let group = tab.group(&root.frame, "query");
        let utility_root = utility_root(root, selector, &group).await?;
        let found = progress
            .race(root.frame.call(
                Some(group.group()),
                &InjectedScopedSelectorElementParams {
                    selector: selector.to_string(),
                    strict,
                    root: utility_root,
                },
            ))
            .await?
            .map_err(page_error)?;
        (Arc::clone(&root.frame), found.element, found.preview, group)
    } else {
        let Some(located) = progress
            .race(frame_selector::locate(tab, selector, strict))
            .await?
            .map_err(page_error)?
        else {
            return Ok(None);
        };
        let group = tab.group(&located.frame, "query");
        let found = progress
            .race(located.frame.call(
                Some(group.group()),
                &InjectedSelectorElementParams {
                    selector: located.selector,
                    strict,
                },
            ))
            .await?
            .map_err(page_error)?;
        (located.frame, found.element, found.preview, group)
    };
    let (Some(element), Some(preview)) = (element, preview) else {
        return Ok(None);
    };
    progress
        .race(element_handle_for(tab, &frame, &element, &preview, handles))
        .await?
        .map(Some)
        .map_err(|error| page_error(error).into())
}

/// Every match, in document order, each adopted into the main world of the frame the selector
/// ends in; `None` when the selector's frame is not there. With `scope`, within that element.
/// Playwright: `server/frameSelectors.ts` `FrameSelectors.queryAll`
async fn query_all(
    tab: &FrameTab,
    scope: Option<&HandleEntry>,
    selector: &str,
    handles: &Handles,
    progress: &Progress,
) -> Result<Option<(Arc<FrameContext>, Vec<CreatedHandle>)>, CallError> {
    let (frame, list, group) = if let Some(root) = scope {
        let group = tab.group(&root.frame, "query-all");
        let utility_root = utility_root(root, selector, &group).await?;
        let found = progress
            .race(root.frame.call(
                Some(group.group()),
                &InjectedScopedQueryAllParams {
                    selector: selector.to_string(),
                    root: utility_root,
                },
            ))
            .await?
            .map_err(page_error)?;
        (Arc::clone(&root.frame), found.elements, group)
    } else {
        let Some(located) = progress
            .race(frame_selector::locate(tab, selector, false))
            .await?
            .map_err(page_error)?
        else {
            return Ok(None);
        };
        let group = tab.group(&located.frame, "query-all");
        let found = progress
            .race(located.frame.call(
                Some(group.group()),
                &InjectedQueryAllParams {
                    selector: located.selector,
                },
            ))
            .await?
            .map_err(page_error)?;
        (located.frame, found.elements, group)
    };
    let previews = progress
        .race(frame.call(None, &InjectedListPreviewsParams { list: list.clone() }))
        .await?
        .map_err(page_error)?
        .previews;
    let elements = progress
        .race(own_properties(&frame, list.id().object_id()))
        .await?
        .map_err(page_error)?;
    let mut created = Vec::with_capacity(elements.len());
    for ((_, element), preview) in elements.into_iter().zip(previews) {
        let Some(element) = element
            .object_id
            .and_then(|id| Handle::<Element>::new(HandleId::new(id), element.subtype.as_deref()))
        else {
            continue;
        };
        created.push(
            progress
                .race(element_handle_for(tab, &frame, &element, &preview, handles))
                .await?
                .map_err(page_error)?,
        );
    }
    drop(group);
    Ok(Some((frame, created)))
}

/// `Runtime.getProperties {ownProperties: true}`: each enumerable own property that has a value,
/// in the order CDP lists them (Playwright's `CRExecutionContext.getProperties`).
async fn own_properties(
    frame: &FrameContext,
    object_id: &str,
) -> Result<Vec<(String, RemoteObject)>, PageError> {
    let reply = frame
        .host
        .send(
            "Runtime.getProperties",
            json!({ "objectId": object_id, "ownProperties": true }),
        )
        .await
        .map_err(|error| frame.host.classify(&error))?;
    let properties: GetPropertiesResult = serde_json::from_value(reply)
        .map_err(|error| PageError::Contract(format!("Runtime.getProperties answered: {error}")))?;
    // Playwright: `server/chromium/crExecutionContext.ts` `CRExecutionContext.getProperties`: `if (!property.enumerable || !property.value)`
    Ok(properties
        .result
        .into_iter()
        .filter(|property| property.enumerable)
        .filter_map(|property| Some((property.name, property.value?)))
        .collect())
}

/// Forgets the run's handles of one call's temporaries and releases their objects.
async fn release(handles: &Handles, created: &[CreatedHandle]) {
    for created in created {
        if let Some(entry) = handles.forget(&created.guid)
            && let Remote::Object(id) = &entry.remote
        {
            let _released = entry
                .frame
                .host
                .send(
                    "Runtime.releaseObject",
                    json!({ "objectId": id.object_id() }),
                )
                .await;
        }
    }
}

/// Playwright's `Frame._evalOnSelector`: the first match (strict when asked), then the
/// function on it, by value; no match fails with Playwright's error.
async fn eval_on_selector(
    tab: &FrameTab,
    scope: Option<&HandleEntry>,
    selector: &str,
    strict: bool,
    evaluation: &Evaluation,
    handles: &Handles,
    progress: &Progress,
) -> Result<EvalResult, CallError> {
    let Some(created) = query_one(tab, scope, selector, strict, handles, progress).await? else {
        // Playwright: `server/frames.ts` `Frame._evalOnSelector`: `Failed to find element matching selector "${target.selector}"`
        return Err(PlaywrightError::error(format!(
            "Failed to find element matching selector \"{selector}\""
        ))
        .into());
    };
    let result = match handles.entry(&created.guid) {
        Some(receiver) => {
            let frame = Arc::clone(&receiver.frame);
            evaluate(tab, &frame, Some(&receiver), evaluation, handles, progress).await
        }
        None => Err(PlaywrightError::target_closed().into()),
    };
    release(handles, &[created]).await;
    result
}

/// Playwright's `Frame._evalOnSelectorAll`: every match as one array, the function's first
/// argument, by value. The array is built in the page from the adopted elements:
/// `{a: [{h: k}, …]}` is `UtilityScript`'s own encoding of an array of handles.
async fn eval_on_selector_all(
    tab: &FrameTab,
    scope: Option<&HandleEntry>,
    selector: &str,
    evaluation: &Evaluation,
    handles: &Handles,
    progress: &Progress,
) -> Result<EvalResult, CallError> {
    let (frame, created) = match query_all(tab, scope, selector, handles, progress).await? {
        Some(found) => found,
        // Playwright: `server/frameSelectors.ts` `FrameSelectors.queryArrayInMainWorld`: `return await context.evaluateHandle(() => []);`
        None => (tab.context().await.map_err(page_error)?, Vec::new()),
    };
    let entries: Vec<Arc<HandleEntry>> = created
        .iter()
        .filter_map(|created| handles.entry(&created.guid))
        .collect();
    let extras: Vec<Extra<'_>> = entries.iter().map(|entry| Extra::Entry(entry)).collect();
    let count = extras.len();
    let invoked = invoke(
        tab,
        &Invocation {
            frame: &frame,
            receiver: None,
            expression: &evaluation.expression,
            is_function: evaluation.is_function,
            return_by_value: true,
            leading: &|base| {
                vec![json!({
                    "a": (base..base + count).map(|h| json!({ "h": h })).collect::<Vec<_>>(),
                    "id": 1,
                })]
            },
            extras: &extras,
            arg: &evaluation.arg,
        },
        handles,
        progress,
    )
    .await;
    release(handles, &created).await;
    let (document, called) = invoked?;
    answer(tab, &frame, document, called, handles, progress).await
}

/// Playwright's `JSHandle._getProperty`: the property, read in the page (own, inherited or a
/// getter's value) onto a null-prototype object, then that object's one property as a handle.
async fn get_property(
    tab: &FrameTab,
    receiver: &HandleEntry,
    name: &str,
    handles: &Handles,
    progress: &Progress,
) -> Result<EvalResult, CallError> {
    let frame = Arc::clone(&receiver.frame);
    let arg = SerializedArgument {
        value: json!(name),
        handles: Vec::new(),
    };
    let extras = [Extra::Entry(receiver)];
    let (document, called) = invoke(
        tab,
        &Invocation {
            frame: &frame,
            receiver: Some(receiver),
            expression: BuiltExpression::GetProperty.source(),
            is_function: Some(true),
            return_by_value: false,
            leading: &|base| vec![json!({ "h": base })],
            extras: &extras,
            arg: &arg,
        },
        handles,
        progress,
    )
    .await?;
    let holder = match called {
        Called::Object(object) => object,
        called => return answer(tab, &frame, document, called, handles, progress).await,
    };
    let holder_id = holder.object_id.clone().unwrap_or_default();
    let properties = progress
        .race(own_properties(&frame, &holder_id))
        .await?
        .map_err(page_error)?;
    let _released = frame
        .host
        .send("Runtime.releaseObject", json!({ "objectId": holder_id }))
        .await;
    let value = properties
        .into_iter()
        .find_map(|(property, value)| (property == name).then_some(value))
        .unwrap_or_else(undefined_object);
    create_handle(tab, &frame, document, value, handles, progress)
        .await
        .map(EvalResult::Handle)
}

/// Playwright's `JSHandle.getProperties`: the enumerable own properties as handles, none for a
/// primitive.
async fn get_property_list(
    tab: &FrameTab,
    receiver: &HandleEntry,
    handles: &Handles,
    progress: &Progress,
) -> Result<Vec<Value>, CallError> {
    let Remote::Object(id) = &receiver.remote else {
        // Playwright: `server/javascript.ts` `JSHandle.internalGetProperties`: `return new Map();`
        return Ok(Vec::new());
    };
    let frame = Arc::clone(&receiver.frame);
    let installed = installed(tab, &frame, Some(receiver), progress).await?;
    let properties = progress
        .race(own_properties(&frame, id.object_id()))
        .await?
        .map_err(page_error)?;
    let mut list = Vec::with_capacity(properties.len());
    for (name, value) in properties {
        let created =
            create_handle(tab, &frame, installed.document, value, handles, progress).await?;
        list.push(json!({ "name": name, "value": created }));
    }
    Ok(list)
}

/// A CDP `RemoteObject` for `undefined`, as Playwright reads a property the holder lacks.
fn undefined_object() -> RemoteObject {
    RemoteObject {
        r#type: "undefined".to_string(),
        ..RemoteObject::default()
    }
}

/// Playwright's `Frame.waitForFunctionExpression`: polls the predicate in the page (every
/// animation frame, or every `polling` ms) until it returns a truthy value, under the call's
/// deadline, and answers a handle to that value. A document that goes away under the wait
/// starts it again after 100 ms, as Playwright's retry does, on the frame's context as it is
/// then: an iframe that changed process has another one (`frames.rs`). A frame that detached
/// ends the wait, as does a predicate that throws. Leaving for any reason stops the page's
/// polling first.
async fn wait_for_function(
    tab: &FrameTab,
    frame: &Arc<FrameContext>,
    evaluation: &Evaluation,
    polling: Option<f64>,
    handles: &Handles,
    progress: &Progress,
) -> Result<EvalResult, CallError> {
    if let Some(polling) = polling
        && polling <= 0.0
    {
        // Playwright: `server/frames.ts` `Frame.waitForFunctionExpression`: `'Cannot poll with non-positive interval: ' + options.pollingInterval`
        return Err(PlaywrightError::error(format!(
            "Cannot poll with non-positive interval: {}",
            js_number(polling)
        ))
        .into());
    }
    let mut frame = Arc::clone(frame);
    loop {
        if let Some(result) = wait_once(tab, &frame, evaluation, polling, handles, progress).await?
        {
            return Ok(result);
        }
        frame = current_context(tab, progress).await?;
    }
}

/// The frame's context for the next attempt of a wait, after the retry interval: the same
/// deadline and signal, on whatever context the frame has now. A frame between contexts (an
/// iframe mid-way to another process) is looked up again after another interval; a frame that
/// detached, or a tab that closed, ends the wait.
async fn current_context(
    tab: &FrameTab,
    progress: &Progress,
) -> Result<Arc<FrameContext>, CallError> {
    loop {
        progress.wait(WAIT_FOR_FUNCTION_RETRY).await?;
        match progress.race(tab.context()).await? {
            Ok(frame) => return Ok(frame),
            Err(PageError::NotConnected) => {}
            Err(error) => return Err(page_error(error).into()),
        }
    }
}

/// One attempt of [`wait_for_function`] in the frame's current document: `None` when that
/// document went away and the wait must start again.
async fn wait_once(
    tab: &FrameTab,
    frame: &Arc<FrameContext>,
    evaluation: &Evaluation,
    polling: Option<f64>,
    handles: &Handles,
    progress: &Progress,
) -> Result<Option<EvalResult>, CallError> {
    let extras = [Extra::Utility];
    let is_function = match evaluation.is_function {
        Some(flag) => json!(flag),
        None => json!({ "v": "undefined" }),
    };
    let polling_value =
        polling.map_or_else(|| json!({ "v": "undefined" }), |polling| json!(polling));
    let started = invoke_with(
        tab,
        &Invocation {
            frame,
            receiver: None,
            expression: BuiltExpression::WaitForFunction.source(),
            is_function: Some(true),
            return_by_value: false,
            // `(utilityScript, expression, isFunction, polling, arg)`.
            leading: &|base| {
                vec![
                    json!({ "h": base }),
                    json!(evaluation.expression),
                    is_function.clone(),
                    polling_value.clone(),
                ]
            },
            extras: &extras,
            arg: &evaluation.arg,
        },
        handles,
        progress,
        LateAnswer::AbortWaiter,
    )
    .await;
    let waiter = match started {
        Ok((_, Called::Object(waiter))) => waiter.object_id.unwrap_or_default(),
        Ok((_, Called::Threw(message))) => return Err(PlaywrightError::error(message).into()),
        Ok((_, Called::Stale | Called::Protocol(_))) => return Ok(None),
        Ok((_, Called::Value(_))) => {
            return Err(PlaywrightError::error("neo-runtime: the waiter answered by value").into());
        }
        Err(CallError::Playwright(error)) if error.message == CONTEXT_DESTROYED => return Ok(None),
        Err(error) => return Err(error),
    };
    let no_arg = SerializedArgument {
        value: json!({ "v": "undefined" }),
        handles: Vec::new(),
    };
    let waiter_extras = [Extra::Object(&waiter)];
    let answered = invoke(
        tab,
        &Invocation {
            frame,
            receiver: None,
            expression: BuiltExpression::WaitResult.source(),
            is_function: Some(true),
            return_by_value: false,
            leading: &|base| vec![json!({ "h": base })],
            extras: &waiter_extras,
            arg: &no_arg,
        },
        handles,
        progress,
    )
    .await;
    let outcome = match answered {
        Ok((document, Called::Object(object))) => Ok(Some(
            create_handle(tab, frame, document, *object, handles, progress).await?,
        )),
        Ok((_, Called::Threw(message))) => Err(PlaywrightError::error(message).into()),
        Ok((_, Called::Stale | Called::Protocol(_))) => Ok(None),
        // The frame's context went while the waiter ran: the wait starts again on its next one.
        Err(CallError::Playwright(error)) if error.message == CONTEXT_DESTROYED => Ok(None),
        Ok((_, Called::Value(_))) => {
            Err(PlaywrightError::error("neo-runtime: the waiter's result answered by value").into())
        }
        Err(error) => Err(error),
    };
    // Playwright awaits the abort, so no poll runs after the call returns, a timed-out or failed
    // one included (`await handle.evaluate(h => h.abort()).catch(() => {})`), bounded here so a
    // frozen page cannot hold the reply; only a run that ended leaves it to the page's tasks.
    let abort = abort_waiter(tab.clone(), Arc::clone(frame), waiter);
    if matches!(outcome, Err(CallError::RunEnded(_))) {
        tab.spawn(abort);
    } else {
        let _stopped = time::timeout(WAITER_ABORT_BOUND, abort).await;
    }
    outcome.map(|created| created.map(EvalResult::Handle))
}

/// Stops a waiter's polling and releases it; errors are ignored, as Playwright's `.catch(() => {})`.
async fn abort_waiter(tab: FrameTab, frame: Arc<FrameContext>, waiter: String) {
    let Ok(installed) = frame.main.installed(&frame).await else {
        return;
    };
    let arguments = vec![
        json!({ "value": true }),
        json!({ "value": true }),
        json!({ "value": BuiltExpression::WaitAbort.source() }),
        json!({ "value": 2 }),
        json!({ "value": { "h": 0 } }),
        json!({ "value": { "v": "undefined" } }),
        json!({ "objectId": waiter }),
    ];
    let _aborted = frame
        .main
        .call(
            &frame,
            &installed,
            MainFunction::Evaluate,
            arguments,
            true,
            None,
        )
        .await;
    let _released = frame
        .host
        .send("Runtime.releaseObject", json!({ "objectId": waiter }))
        .await;
    drop(tab);
}

/// The seam of med-pw-net's `ConsoleMessage.args()`: `object`, a value an event of `frame`'s
/// session carried (a console message's argument), as a handle of the run. An object is first
/// re-wrapped into the run's handle group of that session (`main_world::retain`), so the handle
/// outlives the event's own group (`Runtime.discardConsoleEntries`) and is released with the
/// run's other handles; a node is an `ElementHandle`, and a value without an object keeps it.
/// Playwright: `server/chromium/crExecutionContext.ts` `createHandle`: `if (remoteObject.subtype === 'node') {`
///
/// # Errors
///
/// `NotConnected` when the object or its frame's document is gone; `Closed` once the run ended.
pub(crate) async fn handle_for_remote_object(
    tab: &FrameTab,
    frame: &Arc<FrameContext>,
    object: RemoteObject,
    handles: &Handles,
) -> Result<CreatedHandle, PageError> {
    let installed = frame.main.installed(frame).await?;
    let object = match object.object_id.as_deref() {
        Some(object_id) => {
            let group = handles.group(tab, frame)?;
            main_world::retain(frame, object_id, group.group()).await?
        }
        None => object,
    };
    created_handle(tab, frame, installed.document, object, handles).await
}

/// [`handle_for_remote_object`] for an argument whose page closed before the script read it: the
/// same handle, made without the browser. A value keeps it, so `jsonValue()` still answers; a
/// call through an object's handle fails as on a closed page, as Playwright's handle does then.
pub(crate) fn handle_for_gone_object(
    tab: &FrameTab,
    frame: &Arc<FrameContext>,
    object: &RemoteObject,
    handles: &Handles,
) -> Result<CreatedHandle, PageError> {
    let Some(object_id) = object.object_id.clone() else {
        let (argument, preview) = primitive(object);
        return handles.register(
            tab,
            frame,
            DocumentId::GONE,
            Remote::Value(argument),
            false,
            preview,
        );
    };
    let element = object.subtype.as_deref() == Some(NODE_TYPE);
    let preview = if element {
        format!("{NODE_PREVIEW_PREFIX}{NODE_TYPE}")
    } else {
        render_preview(object)
    };
    handles.register(
        tab,
        frame,
        DocumentId::GONE,
        Remote::Object(HandleId::new(object_id)),
        element,
        preview,
    )
}

/// Playwright's `JSHandle.jsonValue`: the page's `UtilityScript.jsonValue(true, handle)`, or for a
/// primitive the value the handle keeps, without its frame: it answers after the frame is gone.
async fn json_value(
    tab: &FrameTab,
    receiver: &HandleEntry,
    handles: &Handles,
    progress: &Progress,
) -> Result<EvalResult, CallError> {
    // Playwright: `server/javascript.ts` `JSHandle._jsonValue`: `if (!this._objectId)`
    if let Remote::Value(argument) = &receiver.remote {
        return Ok(EvalResult::Value(Some(page_value(argument))));
    }
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

/// A primitive handle's kept `CallArgument` (`{value}`, `{unserializableValue}` or `{}`) in the
/// page's own encoding, as `UtilityScript.jsonValue` answers the same value: `null`, `undefined`
/// and the numbers JSON cannot carry as `{v}`, everything else as itself.
fn page_value(argument: &Value) -> Value {
    match (argument.get("value"), argument.get("unserializableValue")) {
        (Some(Value::Null), _) => json!({ "v": "null" }),
        (Some(value), _) => value.clone(),
        (None, Some(Value::String(special))) => json!({ "v": special }),
        _ => json!({ "v": "undefined" }),
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
                // A child frame may have been removed meanwhile, or changed process, which gives
                // it another context: re-resolve it. Another context ends this call as its
                // destroyed one would, so a wait starts again on the frame's current one rather
                // than poll a context that will never come back.
                if !matches!(tab.scope, FrameScope::Main) {
                    let current = tab.context().await.map_err(page_error)?;
                    if !Arc::ptr_eq(&current, frame) {
                        return Err(PlaywrightError::error(CONTEXT_DESTROYED).into());
                    }
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
    progress
        .race(created_handle(tab, frame, document, object, handles))
        .await?
        .map_err(|error| page_error(error).into())
}

/// [`create_handle`] outside a call, for [`handle_for_remote_object`].
async fn created_handle(
    tab: &FrameTab,
    frame: &Arc<FrameContext>,
    document: DocumentId,
    object: RemoteObject,
    handles: &Handles,
) -> Result<CreatedHandle, PageError> {
    let Some(object_id) = object.object_id.clone() else {
        let (argument, preview) = primitive(&object);
        return handles.register(
            tab,
            frame,
            document,
            Remote::Value(argument),
            false,
            preview,
        );
    };
    let element = object.subtype.as_deref() == Some(NODE_TYPE);
    let preview = if element {
        // Deviation: `server/dom.ts` `ElementHandle._initializePreview`: `this._setPreview(await utility.evaluate((injected, e) => 'JSHandle@' + injected.previewNode(e), this));` Playwright starts at `JSHandle@node` and updates the preview by event; Neo has no events, so the handle starts with the final preview.
        match node_preview(tab, frame, &object_id).await {
            Some(preview) => format!("{NODE_PREVIEW_PREFIX}{preview}"),
            None => format!("{NODE_PREVIEW_PREFIX}{NODE_TYPE}"),
        }
    } else {
        render_preview(&object)
    };
    handles.register(
        tab,
        frame,
        document,
        Remote::Object(HandleId::new(object_id)),
        element,
        preview,
    )
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
    use crate::progress::ErrorName;

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
    fn a_primitive_handle_answers_its_value_in_the_page_encoding() {
        for (argument, value) in [
            (json!({"value": 42}), json!(42)),
            (json!({"value": "a"}), json!("a")),
            (json!({"value": false}), json!(false)),
            (json!({"value": null}), json!({"v": "null"})),
            (json!({"unserializableValue": "-0"}), json!({"v": "-0"})),
            (json!({"unserializableValue": "NaN"}), json!({"v": "NaN"})),
            (json!({}), json!({"v": "undefined"})),
        ] {
            assert_eq!(page_value(&argument), value, "{argument}");
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
        assert!(meta.is_some());
        assert!(matches!(call, Ok(EvalCall::GetProperty { .. })));
        let (meta, call) = route_handle("handle@1", "boundingBox", Some(&json!({})));
        assert!(meta.is_none());
        assert!(call.is_err());
    }

    #[test]
    fn selector_and_wait_commands_decode_their_params_and_refuse_the_selector_wait() {
        let arg = json!({"value": {"v": "undefined"}, "handles": []});
        let (meta, call) = route_frame(
            "querySelector",
            Some(&json!({"selector": "#a", "strict": true})),
        )
        .expect("a frame query");
        assert_eq!(meta.method, "querySelector");
        assert!(matches!(
            call,
            Ok(EvalCall::Query { scope: None, ref selector, strict: true }) if selector == "#a"
        ));
        let (_, call) = route_handle(
            "handle@1",
            "querySelectorAll",
            Some(&json!({"selector": "li"})),
        );
        assert!(matches!(
            call,
            Ok(EvalCall::QueryAll { scope: Some(ref scope), .. }) if scope == "handle@1"
        ));
        let (_, call) = route_handle(
            "handle@1",
            "evalOnSelector",
            Some(
                &json!({"selector": "td", "expression": "e => e.id", "isFunction": true, "arg": arg}),
            ),
        );
        assert!(matches!(
            call,
            Ok(EvalCall::EvalOnSelector { scope: Some(_), strict: false, ref evaluation, .. })
                if evaluation.return_by_value
        ));
        let (_, call) = route_frame(
            "waitForFunction",
            Some(&json!({"expression": "() => 1", "isFunction": true, "arg": arg, "pollingInterval": 50})),
        )
        .expect("a wait");
        assert!(matches!(
            call,
            Ok(EvalCall::WaitForFunction { polling: Some(polling), ref evaluation })
                if polling.to_bits() == 50.0_f64.to_bits() && !evaluation.return_by_value
        ));
        // `Locator.waitForFunction`'s selector form needs the element in the main world's
        // injected script, which Neo does not have yet.
        let (_, call) = route_frame(
            "waitForFunction",
            Some(&json!({"expression": "() => 1", "arg": arg, "selector": "#a", "strict": true})),
        )
        .expect("a wait");
        assert_eq!(call.expect_err("refused").name, ErrorName::Error);
        let (_, call) = route_handle("handle@1", "getPropertyList", None);
        assert!(matches!(call, Ok(EvalCall::GetPropertyList { ref guid }) if guid == "handle@1"));
        let (_, call) = route_frame("querySelector", Some(&json!({"selector": "#a", "x": 1})))
            .expect("a frame query");
        assert!(call.is_err(), "unknown params are refused");
    }

    #[test]
    fn a_closed_table_names_nothing() {
        let handles = Handles::default();
        handles.close();
        assert!(handles.frame_guid("handle@1").is_none());
    }
}
