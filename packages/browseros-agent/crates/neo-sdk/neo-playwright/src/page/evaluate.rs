//! `page.evaluate` and `page.waitForFunction`, with their `frame.*` forms: the agent's scripts in
//! the main world of the page's main frame or of the child frame a `Frame` names, through the
//! main-world gateway (`evaluate_guard`). Both answer in Playwright's serialized format, which the
//! prelude's `wire.ts` reads back with the vendored `parseEvaluationResultValue`.

use std::time::Duration;

use browseros_core::FrameId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time;

use crate::op_scope::OpScope;
use crate::page::evaluate_guard::{
    ArgText, DOCUMENT_REPLACED, EvaluateGuard, MainWorldCall, NAVIGATED, Outcome, Script, Subject,
};
use crate::page::frame_tree::{self, FrameTree};
use crate::target::{ElementTarget, PageRef};
use crate::{ErrorCode, Playwright, PlaywrightError, op_output};

/// How often `waitForFunction` polls for `polling: 'raf'`, its default. Rust polls instead of
/// the page's animation frames, which never fire in a background tab (measured); 100 ms is
/// upstream's retry step.
const RAF_POLL: Duration = Duration::from_millis(100);

/// The script as the prelude sends it (`wire.ts` `PageScript`), decoded once.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) enum ScriptInput {
    /// upstream's client and server steps passed: the script to run.
    Run(RunInput),
    /// The message of the server-side check upstream fails first, raised at its point.
    Refused(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RunInput {
    /// `normalizeEvaluationExpression(String(pageFunction), isFunction)`.
    expression: String,
    /// `typeof pageFunction === 'function'`.
    is_function: bool,
    /// Playwright's in-page serialized argument, as JSON text.
    arg: String,
    /// The element handles the argument's `{h: i}` names, in order; left out when none.
    #[serde(default)]
    handles: Vec<HandleArg>,
}

/// A handle argument: an element brand, with its `$neo` tag.
#[derive(Deserialize)]
#[serde(tag = "$neo", rename_all = "lowercase")]
enum HandleArg {
    Element(ElementTarget),
}

impl From<ScriptInput> for Script {
    fn from(input: ScriptInput) -> Self {
        match input {
            ScriptInput::Run(RunInput {
                expression,
                is_function,
                arg,
                handles,
            }) => Self::Run {
                expression,
                is_function,
                arg: ArgText(arg),
                handles: handles
                    .into_iter()
                    .map(|HandleArg::Element(handle)| handle)
                    .collect(),
            },
            ScriptInput::Refused(message) => Self::Refused(message),
        }
    }
}

/// `page.evaluate(pageFunction, arg)`: `{ page, frame?, script }`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct EvaluateInput {
    page: PageRef,
    /// A child frame, for `frame.evaluate`; absent is the main frame.
    #[serde(default)]
    frame: Option<FrameId>,
    script: ScriptInput,
}

/// `page.waitForFunction(pageFunction, arg, { polling })`: the evaluate input and `polling`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct WaitForFunctionInput {
    page: PageRef,
    #[serde(default)]
    frame: Option<FrameId>,
    script: ScriptInput,
    /// An interval in milliseconds; absent is `'raf'`. The prelude refuses any other string,
    /// as upstream's client does.
    #[serde(default)]
    polling: Option<f64>,
}

/// Runs `page.evaluate`. It has no Playwright timeout, as upstream; the run's deadline still
/// bounds it.
pub(crate) async fn evaluate(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: EvaluateInput,
) -> Result<Value, PlaywrightError> {
    let call = MainWorldCall::once(input.script.into(), Subject::Frame(input.frame));
    let mut guard = EvaluateGuard::open(pw, scope, &input.page, call).await?;
    // A document that was gone before the script started ran nothing, so the script is tried
    // once more in the new one, as upstream waits for the new main context.
    for _attempt in 0..2 {
        match guard.run(scope).await? {
            Outcome::Answered(answer) => {
                return evaluated(pw, scope, &input.page, answer.finish(scope)?.value).await;
            }
            Outcome::NotRun => {}
            Outcome::Destroyed => break,
            Outcome::Navigated => return Err(scope.fail(ErrorCode::PageError, NAVIGATED)),
            Outcome::Falsy => {
                return Err(scope.fail(ErrorCode::Internal, "a script run once answered a poll"));
            }
        }
    }
    Err(scope.fail(ErrorCode::PageError, DOCUMENT_REPLACED))
}

/// A script can mutate history or attach frames before it answers. Return a bounded frame
/// snapshot alongside its value so synchronous accessors observe those effects. Metadata is
/// best-effort: a stalled replacement renderer must not turn a completed script into a wait
/// for navigation or discard its result.
async fn evaluated(
    pw: &Playwright,
    scope: &OpScope<'_>,
    page: &PageRef,
    value: Value,
) -> Result<Value, PlaywrightError> {
    #[derive(Serialize)]
    struct Evaluated {
        value: Value,
        #[serde(flatten)]
        frames: Option<FrameTree>,
    }
    let read = frame_tree::snapshot(pw, scope, page.tab());
    let frames = time::timeout(Duration::from_secs(2), read)
        .await
        .ok()
        .and_then(Result::ok)
        .flatten();
    op_output::to_output(scope, &Evaluated { value, frames })
}

/// Runs `page.waitForFunction`: polls from Rust until the predicate is truthy or throws, because
/// an agent's tab is a background tab, where the page's animation frames never fire. Every poll
/// is a send of its own, rechecked by the host; a navigation mid-wait builds the predicate again
/// in the new document, as upstream retries its wait. The answer is the value in Playwright's
/// format and upstream's preview of it, which the prelude's wait handle holds.
pub(crate) async fn wait_for_function(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: WaitForFunctionInput,
) -> Result<Value, PlaywrightError> {
    let interval = poll_interval(scope, input.polling)?;
    let call = MainWorldCall::poll(input.script.into(), input.frame);
    // Upstream's timeout covers the whole call: the guard finds the frame and installs its
    // world within it, and each poll runs within it here.
    let mut guard = EvaluateGuard::open(pw, scope, &input.page, call).await?;
    let answer = loop {
        // Boxed: a run's state holds a world's install and the main-world calls, and every
        // poll would hold it inside `Playwright::invoke`'s future.
        let outcome = match Box::pin(scope.within(guard.run(scope))).await {
            Ok(outcome) => outcome?,
            Err(_expired) => return Err(scope.timed_out(ErrorCode::WaitTimeout)),
        };
        if let Outcome::Answered(answer) = outcome {
            break answer;
        }
        if scope.sleep(interval).await.is_err() {
            return Err(scope.timed_out(ErrorCode::WaitTimeout));
        }
    };
    let finished = answer.finish(scope)?;
    Ok(json!({ "value": finished.value, "preview": finished.preview }))
}

/// The time between polls. Upstream's server refuses an interval that is not positive, before
/// it reads the script.
fn poll_interval(scope: &OpScope<'_>, polling: Option<f64>) -> Result<Duration, PlaywrightError> {
    let Some(millis) = polling else {
        return Ok(RAF_POLL);
    };
    (millis > 0.0)
        .then(|| Duration::try_from_secs_f64(millis / 1000.0).ok())
        .flatten()
        .ok_or_else(|| {
            scope.fail(
                ErrorCode::InvalidInput,
                &format!("Cannot poll with non-positive interval: {millis}"),
            )
        })
}
