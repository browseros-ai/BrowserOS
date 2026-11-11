//! Agent-supplied scripts run in a dedicated worker's default execution context. The same
//! `UtilityScript` transport as page.evaluate preserves Playwright's value serialization;
//! worker closure races both context startup and the CDP command, including pending promises.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::events::worker_table::WorkerContext;
use crate::op_scope::OpScope;
use crate::page::evaluate::ScriptInput;
use crate::page::evaluate_guard::{self, MAIN_WORLD_JS};
use crate::target::PageRef;
use crate::{ErrorCode, Playwright, PlaywrightError};

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct WorkerEvaluateInput {
    page: PageRef,
    worker: String,
    script: ScriptInput,
}

pub(crate) async fn evaluate(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: WorkerEvaluateInput,
) -> Result<Value, PlaywrightError> {
    scope.require(crate::Capability::MainWorld)?;
    let admitted = scope.admit(pw.browser(), input.page.tab()).await?;
    let closed = || {
        scope.fail(
            ErrorCode::TargetClosed,
            "Target page, context or browser has been closed",
        )
    };
    let worker = pw
        .events()
        .workers
        .get(&admitted.session.target_id, &input.worker)
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
    let script = match input.script {
        ScriptInput::Run(script) => script,
        ScriptInput::Refused(message) => return Err(scope.fail(ErrorCode::PageError, &message)),
    };
    if !script.handles.is_empty() {
        return Err(scope.fail(
            ErrorCode::Unsupported,
            "worker.evaluate does not accept element handles",
        ));
    }
    let params = json!({
        "functionDeclaration": MAIN_WORLD_JS,
        "executionContextId": id,
        "arguments": [{"value": "evaluate"}, {"value": script.is_function},
            {"value": script.expression}, {"value": 1}, {"value": script.arg}],
        "returnByValue": true,
        "awaitPromise": true,
        "userGesture": true,
    });
    let _sent = scope.send_effect()?;
    let call = worker.session.send_value("Runtime.callFunctionOn", params);
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
    Ok(json!({"value": value}))
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
