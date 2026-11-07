//! Playwright: `server/page.ts` `Worker.evaluateExpression`
//! Playwright: `server/page.ts` `Worker.evaluateExpressionHandle`
//! Playwright: `server/javascript.ts` `JSHandle.getProperty`
//!
//! `worker.evaluate`, `worker.evaluateHandle` and the calls on a worker's handles, run in the
//! worker's own execution context through `worker_world.rs`, with the argument layout
//! `evaluate.rs` gives a frame's evaluations. A worker has no frame and no DOM: its handles are
//! `JSHandle`s of the run's table (`evaluate.rs` `WorkerHandle`), closed once the worker is.
//!
//! ```text
//!  JS worker.evaluate(fn, arg)
//!    {"guid":"worker@T:1","method":"evaluateExpression","params":{expression, isFunction, arg}}
//!    └─ dispatch::route → route → serve: the worker ready → WorkerWorld::call(Evaluate)
//! ```

use std::sync::Arc;

use neo_runtime_spec::MethodMeta;
use serde_json::{Value, json};

use crate::click::PageError;
use crate::evaluate::{
    self, EvalCall, EvalResult, Evaluation, Handles, SerializedArgument, WorkerHandle,
};
use crate::main_world::{BuiltExpression, Called};
use crate::page::PageSession;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::worker_world::{WorkerFunction, WorkerWorld};
use crate::workers::{WORKER_PREFIX, Worker};

/// Neo: `Worker.evaluateExpression`'s trace metadata, as `neo-runtime-spec/spec/playwright/worker.yml`
/// declares it.
pub(crate) static WORKER_EVALUATE_META: MethodMeta = MethodMeta {
    interface: "Worker",
    method: "evaluateExpression",
    title: Some("Evaluate"),
    subtitle: None,
    group: None,
    render_params: &[],
    internal: false,
    flags: &[],
};

/// Neo: `Worker.evaluateExpressionHandle`'s trace metadata, as `spec/playwright/worker.yml`
/// declares it.
pub(crate) static WORKER_EVALUATE_HANDLE_META: MethodMeta = MethodMeta {
    interface: "Worker",
    method: "evaluateExpressionHandle",
    title: Some("Evaluate"),
    subtitle: None,
    group: None,
    render_params: &[],
    internal: false,
    flags: &[],
};

/// Playwright: `server/page.ts` `Worker.didClose`: `this.existingExecutionContext.contextDestroyed(error?.message ?? 'Worker was closed');`
const WORKER_CLOSED: &str = "Worker was closed";

/// A message on a worker guid: Playwright's two worker evaluations.
/// Neo: the spec's `Worker` commands are routed by hand (`neo-runtime-spec/spec/serve.yml`).
pub(crate) fn route(
    method: &str,
    params: Option<&Value>,
) -> (
    Option<&'static MethodMeta>,
    Result<Evaluation, PlaywrightError>,
) {
    match method {
        "evaluateExpression" => (
            Some(&WORKER_EVALUATE_META),
            evaluate::evaluation(params, &WORKER_EVALUATE_META, true),
        ),
        "evaluateExpressionHandle" => (
            Some(&WORKER_EVALUATE_HANDLE_META),
            evaluate::evaluation(params, &WORKER_EVALUATE_HANDLE_META, false),
        ),
        // Playwright: `server/dispatchers/dispatcher.ts` `DispatcherConnection.dispatch`: `Mismatching dispatcher: "${dispatcher._type}" does not implement "${method}"`
        _ => (
            None,
            Err(PlaywrightError::error(format!(
                "Mismatching dispatcher: \"Worker\" does not implement \"{method}\""
            ))),
        ),
    }
}

/// Serves `Worker.evaluateExpression` / `evaluateExpressionHandle` on `guid`, a worker of `page`.
/// Playwright: `server/page.ts` `Worker.evaluateExpression`: `return progress.race(js.evaluateExpression(await this._executionContext(progress), expression, { returnByValue: true, isFunction }, arg));`
///
/// # Errors
///
/// TargetClosedError when the worker closed before the call or before it was ready; Playwright's
/// `Worker was closed` when it closes during the call; the evaluation's own error otherwise.
pub(crate) async fn serve(
    page: &PageSession,
    guid: &str,
    evaluation: Evaluation,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let worker = page
        .events
        .workers
        .get(guid)
        .ok_or_else(PlaywrightError::target_closed)?;
    let handles = progress.handles();
    // Playwright: `server/page.ts` `Worker._executionContext`: `return await progress.race(this.openScope.race(this._executionContextPromise));`
    let world = progress
        .race(worker.world())
        .await?
        .map_err(evaluate::page_error)?;
    let called = call(&world, &worker, None, &evaluation, handles, progress).await?;
    let result = answer(&worker, called, handles)?;
    Ok(Some(encode(result)))
}

/// A call on `receiver`, a worker's handle (`evaluate::serve` sends it here): Playwright's
/// `JSHandle` methods in the worker, run in the context the handle was made in. A worker's handle
/// is never an element, so a selector or element call is refused as Playwright's dispatcher
/// refuses it on a `JSHandle`.
/// Playwright: `server/javascript.ts` `JSHandle.evaluateExpression`
///
/// # Errors
///
/// `Worker was closed` once the worker closes; the call's own error otherwise.
pub(crate) async fn serve_handle(
    receiver: &Arc<WorkerHandle>,
    call: EvalCall,
    handles: &Handles,
    progress: &Progress,
) -> Result<Option<Value>, CallError> {
    let worker = &receiver.worker;
    // A handle exists only once its worker's context did.
    let world = worker
        .current_world()
        .ok_or_else(PlaywrightError::target_closed)?;
    let result = match call {
        EvalCall::Handle { evaluation, .. } => {
            let called = self::call(
                &world,
                worker,
                Some(receiver),
                &evaluation,
                handles,
                progress,
            )
            .await?;
            answer(worker, called, handles)?
        }
        EvalCall::JsonValue { .. } => json_value(&world, receiver, handles, progress).await?,
        EvalCall::GetProperty { name, .. } => {
            get_property(&world, receiver, &name, handles, progress).await?
        }
        EvalCall::GetPropertyList { .. } => {
            let properties = get_property_list(&world, receiver, handles, progress).await?;
            return Ok(Some(json!({ "properties": properties })));
        }
        EvalCall::Dispose { guid } => {
            if let Some(handle) = handles.forget_worker(&guid)
                && let Some(object) = handle.object_id()
            {
                // Playwright: `server/chromium/crProtocolHelper.ts` `releaseObject`: `await client.send('Runtime.releaseObject', { objectId }).catch(error => { });`
                world.release_object(object).await;
            }
            return Ok(None);
        }
        other => {
            // Playwright: `server/dispatchers/dispatcher.ts` `DispatcherConnection.dispatch`: `Mismatching dispatcher: "${dispatcher._type}" does not implement "${method}"`
            return Err(PlaywrightError::error(format!(
                "Mismatching dispatcher: \"JSHandle\" does not implement \"{}\"",
                element_method(&other)
            ))
            .into());
        }
    };
    Ok(Some(encode(result)))
}

/// One evaluation in the worker: `evaluate.rs`'s argument layout, with the receiver as the last
/// handle when the call is on a handle, so the SDK's `{h: i}` indices hold as they are.
/// Playwright: `server/javascript.ts` `evaluateExpression`
async fn call(
    world: &WorkerWorld,
    worker: &Arc<Worker>,
    receiver: Option<&WorkerHandle>,
    evaluation: &Evaluation,
    handles: &Handles,
    progress: &Progress,
) -> Result<Called, CallError> {
    let mut handle_args = Vec::with_capacity(evaluation.arg.handles.len() + 1);
    for (index, guid) in evaluation.arg.handles.iter().enumerate() {
        handle_args.push(argument(handles, worker, guid, index)?);
    }
    let leading = match receiver {
        Some(_) => vec![json!({ "h": handle_args.len() })],
        None => Vec::new(),
    };
    if let Some(receiver) = receiver {
        handle_args.push(receiver.call_argument());
    }
    let mut values = vec![
        match evaluation.is_function {
            Some(flag) => json!({ "value": flag }),
            None => json!({}),
        },
        json!({ "value": evaluation.return_by_value }),
        json!({ "value": evaluation.expression }),
        json!({ "value": leading.len() + 1 }),
    ];
    values.extend(leading.into_iter().map(|value| json!({ "value": value })));
    values.push(json!({ "value": evaluation.arg.value }));
    values.extend(handle_args);
    let group = (!evaluation.return_by_value).then(|| handles.worker_group(worker));
    progress
        .race(world.call(
            WorkerFunction::Evaluate,
            values,
            evaluation.return_by_value,
            group.as_ref(),
        ))
        .await?
        .map_err(in_flight)
}

/// Playwright's `JSHandle.jsonValue` on a worker's handle.
/// Playwright: `server/javascript.ts` `JSHandle._jsonValue`: `if (!this._objectId)`
async fn json_value(
    world: &WorkerWorld,
    receiver: &WorkerHandle,
    handles: &Handles,
    progress: &Progress,
) -> Result<EvalResult, CallError> {
    if let Some(value) = receiver.primitive_value() {
        return Ok(EvalResult::Value(Some(value)));
    }
    let values = vec![json!({ "value": true }), receiver.call_argument()];
    let called = progress
        .race(world.call(WorkerFunction::JsonValue, values, true, None))
        .await?
        .map_err(in_flight)?;
    answer(&receiver.worker, called, handles)
}

/// Playwright's `JSHandle.getProperty` on a worker's handle: the property in a holder object,
/// read back as the worker's own handle.
/// Playwright: `server/javascript.ts` `JSHandle.getProperty`
async fn get_property(
    world: &WorkerWorld,
    receiver: &WorkerHandle,
    name: &str,
    handles: &Handles,
    progress: &Progress,
) -> Result<EvalResult, CallError> {
    let worker = &receiver.worker;
    let evaluation = Evaluation {
        expression: BuiltExpression::GetProperty.source().to_string(),
        is_function: Some(true),
        arg: SerializedArgument {
            value: json!(name),
            handles: Vec::new(),
        },
        return_by_value: false,
    };
    let holder = match call(
        world,
        worker,
        Some(receiver),
        &evaluation,
        handles,
        progress,
    )
    .await?
    {
        Called::Object(object) => object,
        called => return answer(worker, called, handles),
    };
    let holder_id = holder.object_id.clone().unwrap_or_default();
    let properties = progress
        .race(world.own_properties(&holder_id))
        .await?
        .map_err(in_flight)?;
    world.release_object(&holder_id).await;
    let value = properties
        .into_iter()
        .find_map(|(property, value)| (property == name).then_some(value))
        .unwrap_or_else(evaluate::undefined_object);
    handles
        .register_worker(worker, page_frame_guid(worker), &value)
        .map(EvalResult::Handle)
        .map_err(|error| evaluate::page_error(error).into())
}

/// Playwright's `JSHandle.getProperties` on a worker's handle, each property the worker's handle.
/// Playwright: `server/javascript.ts` `JSHandle.internalGetProperties`: `return new Map();`
async fn get_property_list(
    world: &WorkerWorld,
    receiver: &WorkerHandle,
    handles: &Handles,
    progress: &Progress,
) -> Result<Vec<Value>, CallError> {
    let Some(object) = receiver.object_id() else {
        return Ok(Vec::new());
    };
    let properties = progress
        .race(world.own_properties(object))
        .await?
        .map_err(in_flight)?;
    let mut list = Vec::with_capacity(properties.len());
    for (name, value) in properties {
        let created = handles
            .register_worker(&receiver.worker, page_frame_guid(&receiver.worker), &value)
            .map_err(evaluate::page_error)?;
        list.push(json!({ "name": name, "value": created }));
    }
    Ok(list)
}

/// A handle argument as a CDP `CallArgument` in `worker`: its own handle's object, any
/// handle's primitive by value (`serializeAsCallArgument`'s `fallThrough`), and nothing of
/// another context.
/// Playwright: `server/javascript.ts` `evaluateExpression`: `throw new JavaScriptErrorInEvaluate('JSHandles can be evaluated only in the context they were created!');`
fn argument(
    handles: &Handles,
    worker: &Arc<Worker>,
    guid: &str,
    index: usize,
) -> Result<Value, CallError> {
    if let Some(handle) = handles.worker_entry(guid) {
        if handle.object_id().is_some() && !Arc::ptr_eq(&handle.worker, worker) {
            return Err(PlaywrightError::error(evaluate::OTHER_CONTEXT).into());
        }
        return Ok(handle.call_argument());
    }
    match handles.primitive_argument(guid) {
        Some(Some(argument)) => Ok(argument),
        Some(None) => Err(PlaywrightError::error(evaluate::OTHER_CONTEXT).into()),
        // Playwright: `server/dispatchers/dispatcher.ts` `DispatcherConnection._tChannelImplFromWire`: `${path}: no object with guid ${guid}`
        None => Err(PlaywrightError::error(format!(
            "arg.handles[{index}]: no object with guid {guid}"
        ))
        .into()),
    }
}

/// Playwright's reading of a worker evaluation's answer, as `evaluate.rs`'s `answer` reads a
/// frame's: a by-handle result becomes the worker's handle, and a protocol error is
/// Playwright's `rewriteError`'s.
/// Playwright: `server/chromium/crExecutionContext.ts` `rewriteError`
fn answer(
    worker: &Arc<Worker>,
    called: Called,
    handles: &Handles,
) -> Result<EvalResult, CallError> {
    match called {
        Called::Value(value) => Ok(EvalResult::Value(value)),
        Called::Object(object) => handles
            .register_worker(worker, page_frame_guid(worker), &object)
            .map(EvalResult::Handle)
            .map_err(|error| evaluate::page_error(error).into()),
        Called::Threw(message) => Err(PlaywrightError::error(message).into()),
        Called::Protocol(message) => evaluate::protocol_answer(&message),
        Called::Stale => Err(PlaywrightError::error(evaluate::CONTEXT_DESTROYED).into()),
    }
}

/// A worker call's page error once it was sent: the worker closing under it ends it as
/// Playwright's `Worker.didClose` ends its context's calls.
/// Playwright: `server/page.ts` `Worker.didClose`: `this.existingExecutionContext.contextDestroyed(error?.message ?? 'Worker was closed');`
fn in_flight(error: PageError) -> CallError {
    match error {
        PageError::Closed => PlaywrightError::error(WORKER_CLOSED).into(),
        error => evaluate::page_error(error).into(),
    }
}

/// Neo: the page's main frame guid of `worker`, `frame@<T>`: what routes its handles' calls.
fn page_frame_guid(worker: &Worker) -> String {
    let target = worker
        .guid()
        .strip_prefix(WORKER_PREFIX)
        .and_then(|rest| rest.split_once(':'))
        .map_or("", |(target, _)| target);
    format!("frame@{target}")
}

/// Neo: the protocol method a refused element call names.
fn element_method(call: &EvalCall) -> &'static str {
    match call {
        EvalCall::Query { .. } => "querySelector",
        EvalCall::QueryAll { .. } => "querySelectorAll",
        EvalCall::EvalOnSelector { .. } => "evalOnSelector",
        EvalCall::EvalOnSelectorAll { .. } => "evalOnSelectorAll",
        EvalCall::DispatchEvent { .. } => "dispatchEvent",
        _ => "evaluateExpression",
    }
}

/// Neo: an evaluation's result as its reply, as `evaluate::serve` encodes a frame's.
fn encode(result: EvalResult) -> Value {
    match result {
        EvalResult::Value(Some(value)) => json!({ "value": value }),
        EvalResult::Value(None) => json!({}),
        EvalResult::Handle(created) => json!({ "handle": created }),
    }
}
