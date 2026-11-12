//! `page.evaluate` and `page.waitForFunction`, with their `frame.*` forms: the agent's scripts in
//! the main world of the page's main frame or of the child frame a `Frame` names, through the
//! main-world gateway (`evaluate_guard`). Both answer in Playwright's serialized format, which the
//! prelude's `wire.ts` reads back with the vendored `parseEvaluationResultValue`.

use std::time::Duration;

use browseros_core::FrameId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time;
use ts_rs::TS;

use crate::op_scope::OpScope;
use crate::page::evaluate_guard::{
    ArgText, DOCUMENT_REPLACED, EvaluateGuard, MainWorldCall, NAVIGATED, Outcome, Script, Subject,
};
use crate::page::frame_tree::{self, FrameInfo, FrameTree};
use crate::page::serialized_value::SerializedValue;
use crate::target::{PageRef, RootHandle};
use crate::wire::options;
use crate::{ErrorCode, Playwright, PlaywrightError};

/// How often `waitForFunction` polls for `polling: 'raf'`, its default. Rust polls instead of
/// the page's animation frames, which never fire in a background tab (measured); 100 ms is
/// upstream's retry step.
const RAF_POLL: Duration = Duration::from_millis(100);

/// How long a timed-out wait waits for the page to answer the release of its poller
/// ([`timed_out_mid_poll`]).
const POLL_CANCEL: Duration = Duration::from_millis(500);

/// `arg` as every evaluating op carries it: upstream's `SerializedArgument` nesting with Neo's
/// contents. `value` is the argument in the page's serialized form as JSON text, which the main
/// world parses itself so the page's own globals cannot alter it (`main-world/json-text.ts`), and
/// `handles` are the element handles its `{h: i}` names, in order, as brands. No `Debug`: the
/// value may be a password the script passes.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScriptArg {
    value: String,
    handles: Vec<RootHandle>,
}

/// One evaluating op's script, from the four fields each op carries beside its addressing:
/// upstream's `expression`, `isFunction` and `arg`, or Neo's `refused`, the message of a check
/// upstream fails only after its command starts (its server's normalization, or its client's
/// argument checks after a locator's wait), which the gateway raises at that point with nothing
/// sent. The prelude leaves `arg` out only beside `refused`.
pub(crate) fn script(
    scope: &OpScope<'_>,
    expression: String,
    is_function: bool,
    arg: Option<ScriptArg>,
    refused: Option<String>,
) -> Result<Script, PlaywrightError> {
    match (refused, arg) {
        (Some(message), _) => Ok(Script::Refused(message)),
        (None, Some(ScriptArg { value, handles })) => Ok(Script::Run {
            expression,
            is_function,
            arg: ArgText(value),
            handles: handles
                .into_iter()
                .map(|RootHandle::Element(handle)| handle)
                .collect(),
        }),
        (None, None) => Err(scope.fail(ErrorCode::InvalidInput, "missing field `arg`")),
    }
}

/// `page.evaluate(pageFunction, arg)`: upstream's `Frame.evaluateExpression` params on the page
/// or one of its frames, and `refused` (`script`).
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct EvaluateInput {
    page: PageRef,
    /// A child frame, for `frame.evaluate`; absent is the main frame.
    #[serde(default)]
    #[ts(optional)]
    frame: Option<FrameId>,
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
options!(EvaluateInput);

/// `page.waitForFunction(pageFunction, arg, { polling })`: upstream's `Frame.waitForFunction`
/// params, and `refused` (`script`).
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct WaitForFunctionInput {
    page: PageRef,
    #[serde(default)]
    #[ts(optional)]
    frame: Option<FrameId>,
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
    /// An interval in milliseconds; absent is `'raf'`, as upstream's. The prelude refuses any
    /// other string, as upstream's client does.
    #[serde(default)]
    #[ts(optional)]
    polling_interval: Option<f64>,
}
options!(WaitForFunctionInput);

/// `page.evaluate`'s answer: the script's value, and the page's frames as the script left them,
/// for the prelude's synchronous accessors; absent when that bounded read did not finish.
#[derive(Debug, Serialize, TS)]
pub(crate) struct Evaluated {
    value: SerializedValue,
    // A plain optional field where a flattened `Option<FrameTree>` was (ts-rs cannot print a
    // flattened `Option`): the same JSON, pinned by `tests::tests_that_the_answers_keep_their_bytes`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    frames: Option<Vec<FrameInfo>>,
}

/// `page.waitForFunction`'s answer: the predicate's truthy value and upstream's preview of it
/// (`toString()`), which the prelude's wait handle holds.
#[derive(Debug, Serialize, TS)]
pub(crate) struct WaitedFunction {
    value: SerializedValue,
    preview: String,
}

/// Runs `page.evaluate`. It has no Playwright timeout, as upstream; the run's deadline still
/// bounds it.
pub(crate) async fn evaluate(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: EvaluateInput,
) -> Result<Evaluated, PlaywrightError> {
    let EvaluateInput {
        page,
        frame,
        expression,
        is_function,
        arg,
        refused,
    } = input;
    let script = script(scope, expression, is_function, arg, refused)?;
    let call = MainWorldCall::once(script, Subject::Frame(frame));
    let mut guard = EvaluateGuard::open(pw, scope, &page, call).await?;
    // A document that was gone before the script started ran nothing, so the script is tried
    // once more in the new one, as upstream waits for the new main context.
    for _attempt in 0..2 {
        match guard.run(scope).await? {
            Outcome::Answered(answer) => {
                return evaluated(pw, scope, &page, answer.finish(scope)?.value).await;
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
) -> Result<Evaluated, PlaywrightError> {
    let read = frame_tree::snapshot(pw, scope, page.tab());
    let frames: Option<FrameTree> = time::timeout(Duration::from_secs(2), read)
        .await
        .ok()
        .and_then(Result::ok)
        .flatten();
    Ok(Evaluated {
        value: SerializedValue(value),
        frames: frames.map(FrameTree::into_frames),
    })
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
) -> Result<WaitedFunction, PlaywrightError> {
    let WaitForFunctionInput {
        page,
        frame,
        expression,
        is_function,
        arg,
        refused,
        polling_interval,
    } = input;
    let interval = poll_interval(scope, polling_interval)?;
    let call = MainWorldCall::poll(script(scope, expression, is_function, arg, refused)?, frame);
    // Upstream's timeout covers the whole call: the guard finds the frame and installs its
    // world within it, and each poll runs within it here.
    let mut guard = EvaluateGuard::open(pw, scope, &page, call).await?;
    let answer = loop {
        // Boxed: a run's state holds a world's install and the main-world calls, and every
        // poll would hold it inside `Playwright::invoke`'s future.
        let outcome = match Box::pin(scope.within(guard.run(scope))).await {
            Ok(outcome) => outcome?,
            Err(_expired) => return Err(timed_out_mid_poll(scope).await),
        };
        if let Outcome::Answered(answer) = outcome {
            break answer;
        }
        if scope.sleep(interval).await.is_err() {
            return Err(scope.timed_out(ErrorCode::WaitTimeout));
        }
    };
    let finished = answer.finish(scope)?;
    // A wait's answer settles with its preview (`Answer::Settled`); the one other answer a poll
    // gives, an unreadable value, fails in `finish` first.
    let preview = finished.preview.ok_or_else(|| {
        scope.fail(
            ErrorCode::Internal,
            "a waitForFunction answer came back without its preview",
        )
    })?;
    Ok(WaitedFunction {
        value: SerializedValue(finished.value),
        preview,
    })
}

/// The timeout of a wait whose deadline cut a poll short. The poll may already be in the page
/// and still run there, and upstream's timeout awaits its poller's `cancel` before it throws, so
/// whatever such a poll logs reaches the listeners first (WA-005). Releasing the op's objects is
/// Neo's cancel: a round trip on the session the poll went out on, which the renderer answers
/// only after that poll ran, so its events come before this op's reply
/// (`Playwright::events_before_reply`), and the poller is gone. Bounded, for a renderer that no
/// longer answers.
async fn timed_out_mid_poll(scope: &OpScope<'_>) -> PlaywrightError {
    let _released = time::timeout(POLL_CANCEL, scope.release_objects()).await;
    scope.timed_out(ErrorCode::WaitTimeout)
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

#[cfg(test)]
mod tests {
    use serde::Serialize;
    use serde_json::{Error, Value, json};

    use super::{Evaluated, WaitedFunction};
    use crate::page::frame_tree::{FrameTree, sample_tree};
    use crate::page::serialized_value::SerializedValue;
    use crate::test_support::TestResult;

    /// An answer as the route sends it: through `serde_json::to_value`, whose maps sort the keys
    /// (`op_output::to_output`). Written straight to text a struct keeps its declaration order,
    /// which is not what goes over the wire.
    fn sent(answer: &impl Serialize) -> Result<String, Error> {
        serde_json::to_string(&serde_json::to_value(answer)?)
    }

    /// `page.evaluate` sends the bytes the struct with a flattened `Option<FrameTree>` it
    /// replaced did, key order included, with frames and without; `waitForFunction` sends the
    /// bytes its `json!` did. A later edit cannot drift unseen.
    #[test]
    fn tests_that_the_answers_keep_their_bytes() -> TestResult {
        #[derive(Serialize)]
        struct Before {
            value: Value,
            #[serde(flatten)]
            frames: Option<FrameTree>,
        }
        let value = || json!({ "v": "undefined" });
        let with = sent(&Evaluated {
            value: SerializedValue(value()),
            frames: Some(sample_tree()?.into_frames()),
        })?;
        let before = Before {
            value: value(),
            frames: Some(sample_tree()?),
        };
        assert_eq!(with, sent(&before)?);
        assert_eq!(
            with,
            r#"{"frames":[{"id":"main","name":"","parentId":null,"url":"https://main.test/"},{"id":"child","name":"ad","parentId":"main","url":"https://child.test/"}],"value":{"v":"undefined"}}"#
        );
        let without = sent(&Evaluated {
            value: SerializedValue(value()),
            frames: None,
        })?;
        let before = Before {
            value: value(),
            frames: None,
        };
        assert_eq!(without, sent(&before)?);
        assert_eq!(without, r#"{"value":{"v":"undefined"}}"#);
        let waited = sent(&WaitedFunction {
            value: SerializedValue(value()),
            preview: "undefined".to_owned(),
        })?;
        assert_eq!(
            waited,
            sent(&json!({ "value": value(), "preview": "undefined" }))?
        );
        assert_eq!(
            waited,
            r#"{"preview":"undefined","value":{"v":"undefined"}}"#
        );
        Ok(())
    }
}
