//! Agent-supplied scripts run in a dedicated worker's default execution context. The same
//! `UtilityScript` transport as page.evaluate preserves Playwright's value serialization;
//! worker closure races both context startup and the CDP command, including pending promises.

use serde::Deserialize;
use serde_json::{Value, json};
use ts_rs::TS;

use crate::events::worker_table::WorkerContext;
use crate::op_scope::OpScope;
use crate::page::evaluate::{self, ScriptArg};
use crate::page::evaluate_guard::{self, ArgText, MAIN_WORLD_JS, Script};
use crate::page::serialized_value::SerializedValue;
use crate::target::PageRef;
use crate::wire::options;
use crate::{ErrorCode, Playwright, PlaywrightError};

/// `worker.evaluate(fn, arg)`: the page, the worker the host's table names, and upstream's
/// `Worker.evaluateExpression` params with `refused` (`evaluate::script`).
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct WorkerEvaluateInput {
    page: PageRef,
    worker: String,
    expression: String,
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    is_function: bool,
    #[serde(default)]
    #[ts(optional)]
    arg: Option<ScriptArg>,
    #[serde(default)]
    #[ts(optional)]
    refused: Option<String>,
}
options!(WorkerEvaluateInput);

pub(crate) async fn evaluate(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: WorkerEvaluateInput,
) -> Result<SerializedValue, PlaywrightError> {
    let WorkerEvaluateInput {
        page,
        worker,
        expression,
        is_function,
        arg,
        refused,
    } = input;
    let script = evaluate::script(scope, expression, is_function, arg, refused)?;
    scope.require(crate::Capability::MainWorld)?;
    let admitted = scope.admit(pw.browser(), page.tab()).await?;
    let closed = || {
        scope.fail(
            ErrorCode::TargetClosed,
            "Target page, context or browser has been closed",
        )
    };
    let worker = pw
        .events()
        .workers
        .get(&admitted.session.target_id, &worker)
        .ok_or_else(closed)?;
    let mut context = worker.context.subscribe();
    let id = loop {
        let state = *context.borrow_and_update();
        match state {
            WorkerContext::Ready(id) => break id,
            WorkerContext::Closed => return Err(closed()),
            WorkerContext::Starting { .. } => {
                context.changed().await.map_err(|_disconnected| closed())?;
            }
        }
    };
    let (expression, is_function, ArgText(arg)) = match script {
        Script::Run { handles, .. } if !handles.is_empty() => {
            return Err(scope.fail(
                ErrorCode::Unsupported,
                "worker.evaluate does not accept element handles",
            ));
        }
        Script::Run {
            expression,
            is_function,
            arg,
            ..
        } => (expression, is_function, arg),
        Script::Refused(message) => return Err(scope.fail(ErrorCode::PageError, &message)),
    };
    let params = json!({
        "functionDeclaration": MAIN_WORLD_JS,
        "executionContextId": id,
        "arguments": [{"value": "evaluate"}, {"value": is_function},
            {"value": expression}, {"value": 1}, {"value": arg}],
        "returnByValue": true,
        "awaitPromise": true,
        "userGesture": true,
    });
    let _sent = scope.send_effect()?;
    let session = worker.session();
    let call = session.send_value("Runtime.callFunctionOn", params);
    let reply = tokio::select! {
        biased;
        _ = context.wait_for(|state| *state == WorkerContext::Closed) => return Err(closed()),
        reply = call => reply.map_err(|error| scope.core_error(&error))?,
    };
    let reply: Reply = serde_json::from_value(reply)
        .map_err(|error| scope.fail(ErrorCode::Internal, &error.to_string()))?;
    if let Some(details) = reply.exception_details {
        let message = details
            .exception
            .and_then(|exception| exception.description)
            .unwrap_or(details.text);
        return Err(scope.fail(ErrorCode::PageError, &message));
    }
    let text = match reply.result.value {
        Some(Value::String(text)) => Some(text),
        _ => None,
    };
    let value = evaluate_guard::evaluation_result(scope, text)?;
    Ok(SerializedValue(value))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reply {
    result: RemoteObject,
    exception_details: Option<ExceptionDetails>,
}

#[derive(Deserialize)]
struct RemoteObject {
    value: Option<Value>,
    description: Option<String>,
}

#[derive(Deserialize)]
struct ExceptionDetails {
    text: String,
    exception: Option<RemoteObject>,
}
