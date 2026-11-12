//! Agent-supplied scripts run in a dedicated worker's default execution context. The same
//! `UtilityScript` transport as page.evaluate preserves Playwright's value serialization;
//! worker closure races both script startup and the CDP commands, including pending promises.
//!
//! The call goes through a handle to the worker's global, taken from the session's default
//! context, not by an execution context id: a worker has one context, and learning its id would
//! need `Runtime` enabled on the session, which a site can detect by timing its console calls
//! (`browseros_core::runtime_lease`). So `worker.evaluate` leaves `Runtime` off.

use browseros_core::ProtocolSession;
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
    loop {
        let state = *context.borrow_and_update();
        match state {
            WorkerContext::Ready => break,
            WorkerContext::Closed => return Err(closed()),
            WorkerContext::Starting => {
                context.changed().await.map_err(|_disconnected| closed())?;
            }
        }
    }
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
    // After the readiness wait: a recovered endpoint may have been rebound meanwhile.
    let session = worker.session();
    let call = call_in_worker(scope, &session, &expression, is_function, &arg);
    let reply = tokio::select! {
        biased;
        _ = context.wait_for(|state| *state == WorkerContext::Closed) => return Err(closed()),
        reply = call => reply?,
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

/// Upstream's `UtilityScript.evaluate` of the agent's script, through a handle to the worker's
/// global: `Runtime.callFunctionOn`'s raw reply.
async fn call_in_worker(
    scope: &OpScope<'_>,
    session: &ProtocolSession,
    expression: &str,
    is_function: bool,
    arg: &str,
) -> Result<Value, PlaywrightError> {
    // A script's top-level `this` is the global itself: unlike `globalThis`, a property the
    // worker can replace with a getter, reading it runs no worker code, so it may come before
    // the effect is marked. It is still Neo's script in the worker's own realm, which a refusal
    // at the send below would leave unreported.
    scope.sending_main_world();
    let global: Global = session
        .send(
            "Runtime.evaluate",
            json!({
                "expression": "this",
                "objectGroup": scope.object_group(session),
                "silent": true,
            }),
        )
        .await
        .map_err(|error| scope.core_error(&error))?;
    let global = global.result.object_id.ok_or_else(|| {
        scope.fail(
            ErrorCode::Internal,
            "the worker returned no handle to its global",
        )
    })?;
    let _sent = scope.send_effect()?;
    session
        .send_value(
            "Runtime.callFunctionOn",
            json!({
                "objectId": global,
                "functionDeclaration": MAIN_WORLD_JS,
                "arguments": [{"value": "evaluate"}, {"value": is_function},
                    {"value": expression}, {"value": 1}, {"value": arg}],
                "returnByValue": true,
                "awaitPromise": true,
                "userGesture": true,
            }),
        )
        .await
        .map_err(|error| scope.core_error(&error))
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

/// `Runtime.evaluate`'s reply for the worker's global, by handle.
#[derive(Deserialize)]
struct Global {
    result: GlobalHandle,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GlobalHandle {
    object_id: Option<String>,
}

#[derive(Deserialize)]
struct ExceptionDetails {
    text: String,
    exception: Option<RemoteObject>,
}

#[cfg(test)]
mod tests {
    use browseros_core::{SessionId, TargetId};
    use neo_trace::ErrorOwner;
    use serde_json::{Value, json};

    use crate::events::worker_table::WorkerInfo;
    use crate::test_env::EnvCall;
    use crate::test_support::{
        ProbeAnswer, SCRIPTED_MAIN, ScriptedRun, ScriptedTab, TabScript, TestResult,
    };
    use crate::{Op, Refusal};

    struct NoProbes;
    impl TabScript for NoProbes {
        fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
            ProbeAnswer::unexpected()
        }
    }

    /// A worker evaluate takes the worker's global with a script in the worker's own realm,
    /// before the host's recheck, so one the host refuses at its send reports the main world,
    /// though the agent's script never went out.
    #[tokio::test]
    async fn tests_that_a_refused_worker_evaluate_reports_its_global_lookup() -> TestResult {
        let tab = ScriptedTab::new(NoProbes);
        let run = ScriptedRun::over(&tab).await?;
        let refusal = Refusal {
            owner: ErrorOwner::Host,
            code: "owner_has_control".to_owned(),
            message: "The owner is using this tab.".to_owned(),
            host_code: None,
        };
        let run = ScriptedRun {
            env: run.env.with_refusal(refusal),
            ..run
        };
        let session = run
            .playwright
            .browser()
            .protocol_session(SessionId::from("worker-session"));
        assert!(
            run.playwright.events().workers.insert(
                TargetId(SCRIPTED_MAIN.to_owned()),
                WorkerInfo {
                    id: "worker".to_owned(),
                    url: "https://a.test/worker.js".to_owned(),
                },
                session,
                false,
            ),
            "the worker is new"
        );
        let op = Op::from_name("worker.evaluate").ok_or("worker.evaluate is an op")?;
        let input = json!({ "page": serde_json::to_value(&run.page)?, "worker": "worker",
            "expression": "() => 1", "isFunction": true,
            "arg": { "value": r#"{"v":"undefined"}"#, "handles": [] } });
        let refused = run
            .playwright
            .invoke(&run.env, op, input)
            .await
            .err()
            .ok_or("the host refuses the script")?;
        assert_eq!(refused.code(), "owner_has_control", "{refused}");
        assert_eq!(
            tab.sent("Runtime.callFunctionOn"),
            0,
            "the agent's script never went out"
        );
        assert!(
            run.env.calls().contains(&EnvCall::MainWorld),
            "the global's lookup ran in the worker: {:?}",
            run.env.calls()
        );
        Ok(())
    }
}
