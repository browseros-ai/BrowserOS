//! `page.evaluate` and `page.waitForFunction`: the agent's scripts in the page's main world,
//! between the evaluate guard's two sweeps. Both return the masked value itself, as the
//! prelude's `wire.ts` reads it.

use std::time::Duration;

use browseros_core::FrameId;
use serde::Deserialize;
use serde_json::Value;

use crate::op_scope::OpScope;
use crate::page::evaluate_guard::{EvaluateGuard, Ran, Script, present};
use crate::page::world_call::main_frame_only;
use crate::target::PageRef;
use crate::{ErrorCode, Playwright, PlaywrightError};

/// What upstream says when the document went away under a running script.
const DOCUMENT_REPLACED: &str =
    "Execution context was destroyed, most likely because of a navigation";

/// How often `waitForFunction` polls for `polling: 'raf'`, its default. Rust polls instead of
/// the page's animation frames, which never fire in a background tab (measured); 100 ms is
/// upstream's retry step.
const RAF_POLL: Duration = Duration::from_millis(100);

/// `page.evaluate(pageFunction, arg)`: `{ page, frame?, expression, isFunction, arg? }`,
/// upstream's `evaluateExpression` parameters with the argument as JSON.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct EvaluateInput {
    page: PageRef,
    /// A child frame, for `frame.evaluate`; absent is the main frame.
    #[serde(default)]
    frame: Option<FrameId>,
    /// `String(pageFunction)`.
    expression: String,
    /// `typeof pageFunction === 'function'`; absent detects it in the page, as upstream does.
    #[serde(default)]
    is_function: Option<bool>,
    /// The argument as JSON; absent is `undefined`, and `null` stays `null`.
    #[serde(default, deserialize_with = "present")]
    arg: Option<Value>,
}

/// `page.waitForFunction(pageFunction, arg, { polling })`: the evaluate input and `polling`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct WaitForFunctionInput {
    page: PageRef,
    #[serde(default)]
    frame: Option<FrameId>,
    expression: String,
    #[serde(default)]
    is_function: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    arg: Option<Value>,
    /// `'raf'` or an interval in milliseconds; absent is `'raf'`.
    #[serde(default)]
    polling: Option<Polling>,
}

/// `waitForFunction`'s `polling` option.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(untagged)]
enum Polling {
    Raf(RafPolling),
    Interval(f64),
}

/// The string `"raf"`.
#[derive(Clone, Copy, Debug, Deserialize)]
enum RafPolling {
    #[serde(rename = "raf")]
    Raf,
}

/// Runs `page.evaluate`. It has no Playwright timeout, as upstream; the run's deadline still
/// bounds it.
pub(crate) async fn evaluate(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: EvaluateInput,
) -> Result<Value, PlaywrightError> {
    main_frame_only(scope, input.frame.as_ref())?;
    let script = Script::new(input.expression, input.is_function, input.arg);
    let mut guard =
        EvaluateGuard::sweep_before(pw, scope, &input.page.checked(pw.browser(), scope).await?)
            .await?;
    // A document that was gone before the script started ran nothing, so the script is tried
    // once more in the new one.
    for _attempt in 0..2 {
        match guard.run(scope, &script).await? {
            Ran::Finished(unmasked) => return guard.finish(scope, unmasked).await,
            Ran::NotRun => {}
            Ran::Destroyed => break,
        }
    }
    Err(scope.fail(ErrorCode::PageError, DOCUMENT_REPLACED))
}

/// Runs `page.waitForFunction`: polls from Rust until the script returns a truthy value or
/// throws. Each poll re-runs the host's policies right before its script (inside
/// `EvaluateGuard::run`), because each runs script in the page.
/// Only the last poll's result leaves the page, through the guard's second sweep.
pub(crate) async fn wait_for_function(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: WaitForFunctionInput,
) -> Result<Value, PlaywrightError> {
    main_frame_only(scope, input.frame.as_ref())?;
    let interval = poll_interval(scope, input.polling)?;
    let script = Script::new(input.expression, input.is_function, input.arg);
    let mut guard =
        EvaluateGuard::sweep_before(pw, scope, &input.page.checked(pw.browser(), scope).await?)
            .await?;
    let polls = async {
        loop {
            // A falsy result polls again, and so does a document a navigation replaced:
            // upstream retries the wait in the new document.
            if let Ran::Finished(unmasked) = guard.run(scope, &script).await?
                && unmasked.ends_wait()
            {
                return Ok(unmasked);
            }
            if scope.sleep(interval).await.is_err() {
                return Err(scope.timed_out(ErrorCode::WaitTimeout));
            }
        }
    };
    let last = match scope.within(polls).await {
        Ok(polled) => polled?,
        Err(_expired) => return Err(scope.timed_out(ErrorCode::WaitTimeout)),
    };
    guard.finish(scope, last).await
}

/// The time between polls. Upstream refuses an interval that is not positive.
fn poll_interval(
    scope: &OpScope<'_>,
    polling: Option<Polling>,
) -> Result<Duration, PlaywrightError> {
    match polling {
        None | Some(Polling::Raf(RafPolling::Raf)) => Ok(RAF_POLL),
        Some(Polling::Interval(millis)) => (millis > 0.0)
            .then(|| Duration::try_from_secs_f64(millis / 1000.0).ok())
            .flatten()
            .ok_or_else(|| {
                scope.fail(
                    ErrorCode::InvalidInput,
                    &format!("Cannot poll with non-positive interval: {millis}"),
                )
            }),
    }
}
