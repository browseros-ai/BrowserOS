//! `locator.evaluate`, `page.evalOnSelector` and `locator.evaluateAll`, with their element-handle,
//! page and frame forms (`elementHandle.evaluate`, `$eval`, `$$eval`): the agent's function called
//! with elements, in the main world of the frame they are in, through the main-world gateway
//! (`evaluate_guard`). All answer in Playwright's serialized format, which the prelude reads back
//! with the vendored `parseEvaluationResultValue`.
//!
//! ```text
//!  locator.evaluate ───── waitForSelector(attached, strict) ─▶ handle ─┐ upstream's `_withElement`
//!  page.evalOnSelector ── ($eval) the first match now, or the failure ─┤
//!  elementHandle.evaluate ─────────────────────────────────────────────┴▶ Subject::Element
//!  locator.evaluateAll, $$eval ── every match now, pinned ─▶ Subject::Elements in their frame
//!                                 nothing matched, or a frame on the way missing ─▶ fn([]) in
//!                                 the query's starting frame (measured, spike S-detached)
//!  locator.waitForFunction ── on the outer backoff: the first match now (strict) ─▶
//!                             Subject::Element, judged truthy in the page ─▶ done, or try again
//! ```
//!
//! Elements reach the main world by backend node id, never as isolated-world objects: an object
//! of one world cannot be an argument in another, and the main world resolves its own (the
//! guard's `run`).

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Deserialize;
use serde_json::Value;
use ts_rs::TS;

use crate::backoff::Backoff;
use crate::op_scope::OpScope;
use crate::page::evaluate::{self, ScriptArg};
use crate::page::evaluate_guard::{
    self, DOCUMENT_REPLACED, EvaluateGuard, MainWorldCall, NAVIGATED, Outcome, Script, Subject,
    TreeFrame,
};
use crate::page::serialized_value::SerializedValue;
use crate::page::{OpPage, world_call};
use crate::read::{self, Returning, SelectorState};
use crate::resolve::{self, HandleSite};
use crate::target::{ElementTarget, LocatorTarget, PageRef, RootHandle, Strictness, Target};
use crate::wire::{Nothing, options};
use crate::world::{FrameSeq, SeqLookup};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// `locator.evaluate(fn, arg)` and `elementHandle.evaluate`: upstream's
/// `JSHandle.evaluateExpression` params on the element a locator waits for, or on a handle's, and
/// `refused` (`evaluate::script`).
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct EvaluateOnElementInput {
    target: Target,
    strict: Strictness,
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
options!(EvaluateOnElementInput);

/// `page.$eval`, `frame.$eval` and `elementHandle.$eval`: upstream's `Frame.evalOnSelector` (and
/// `ElementHandle.evalOnSelector`, the same params) on the first match now, and `refused`.
/// `selector` is the selector as the script wrote it, which upstream's no-match error quotes;
/// `target` carries it parsed, since Rust never parses selectors.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct EvalOnSelectorInput {
    target: Target,
    strict: Strictness,
    selector: String,
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
options!(EvalOnSelectorInput);

/// `locator.evaluateAll(fn, arg)` and `$$eval`: upstream's `Frame.evalOnSelectorAll` params, and
/// `refused`.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct EvaluateOnAllInput {
    target: Target,
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
options!(EvaluateOnAllInput);

/// `locator.waitForFunction(fn, arg)`: upstream's `Frame.waitForFunction` with the locator for its
/// `selector` and `strict`, and `refused`.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct WaitForFunctionOnElementInput {
    target: Target,
    strict: Strictness,
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
options!(WaitForFunctionOnElementInput);

/// Runs `fn(element, arg)`. A locator waits for its element as upstream's `_withElement` does
/// (attached, strict, until the op's timeout), and the script itself has no timeout but the
/// run's. An element handle runs on its node, in its document or out of it, as upstream's does.
pub(crate) async fn evaluate_on_element(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: EvaluateOnElementInput,
) -> Result<SerializedValue, PlaywrightError> {
    let EvaluateOnElementInput {
        target,
        strict,
        expression,
        is_function,
        arg,
        refused,
    } = input;
    let script = evaluate::script(scope, expression, is_function, arg, refused)?;
    let element = match target {
        Target::Element(handle) => handle,
        Target::Locator(locator) => attached(pw, scope, &locator, strict).await?,
    };
    let page = element.page().clone();
    let call = MainWorldCall::once(script, Subject::Element(element));
    run_once(pw, scope, &page, call).await
}

/// Runs `$eval`: `fn(element, arg)` on the first match now, or upstream's failure quoting
/// `selector` as the script wrote it, as upstream's `evalOnSelector` does with no waiting.
pub(crate) async fn eval_on_selector(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: EvalOnSelectorInput,
) -> Result<SerializedValue, PlaywrightError> {
    let EvalOnSelectorInput {
        target,
        strict,
        selector,
        expression,
        is_function,
        arg,
        refused,
    } = input;
    let script = evaluate::script(scope, expression, is_function, arg, refused)?;
    let element = match target {
        Target::Element(handle) => handle,
        Target::Locator(locator) => {
            first_match(pw, scope, &locator, strict, Some(selector.as_str())).await?
        }
    };
    let page = element.page().clone();
    let call = MainWorldCall::once(script, Subject::Element(element));
    run_once(pw, scope, &page, call).await
}

/// Runs `fn(elements, arg)` on every match now, as upstream's `evalOnSelectorAll`: no waiting,
/// never strict, in the frame the matches are in. With no match, or a frame on the way not
/// there yet, `fn([])` runs in the frame the query starts from, as upstream's
/// `queryArrayInMainWorld` falls back to its own frame (measured on BrowserOS neo: a child frame
/// reached with no match runs in the starting frame too).
pub(crate) async fn evaluate_on_all(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: EvaluateOnAllInput,
) -> Result<SerializedValue, PlaywrightError> {
    let EvaluateOnAllInput {
        target,
        expression,
        is_function,
        arg,
        refused,
    } = input;
    let script = evaluate::script(scope, expression, is_function, arg, refused)?;
    let Target::Locator(locator) = target else {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            "target: an element handle is not a selector; query from it with root",
        ));
    };
    let page = OpPage::admit_handle(pw, scope, locator.page()).await?;
    let subject = match read::every_match(&page, scope, &locator).await? {
        Some((world, nodes)) if !nodes.is_empty() => Subject::Elements {
            frame: evaluate_guard::frame_endpoint(&page, world.session(), world.frame().clone()),
            seq: world.seq(),
            nodes,
        },
        Some(_) | None => {
            let (frame, seq) = starting_frame(&page, scope, &locator).await?;
            Subject::Elements {
                frame,
                seq,
                nodes: Vec::new(),
            }
        }
    };
    run_once(
        pw,
        scope,
        locator.page(),
        MainWorldCall::once(script, subject),
    )
    .await
}

/// Runs upstream's `waitForFunctionExpressionOnElement` (PW frames.ts, `_waitForFunctionOnSelector`
/// in the main world): `waiting for <locator>` in the call log, then tries on the outer backoff
/// until `fn(element, arg)` is truthy, within the op's timeout. Each try matches the locator
/// afresh, so an element the page re-rendered is found again, and the page judges the result
/// as it was returned (`MainWorldCall::truthy`). A strict violation or a throw in the page ends
/// the wait, as upstream's non-retriable errors do; no match yet, an element gone before its
/// script ran, or a document replaced under it, tries again. It resolves to nothing, as
/// upstream's does.
pub(crate) async fn wait_for_function_on_element(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: WaitForFunctionOnElementInput,
) -> Result<Nothing, PlaywrightError> {
    let WaitForFunctionOnElementInput {
        target,
        strict,
        expression,
        is_function,
        arg,
        refused,
    } = input;
    let script = evaluate::script(scope, expression, is_function, arg, refused)?;
    let Target::Locator(locator) = target else {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            "target: locator.waitForFunction takes a locator, not an element handle",
        ));
    };
    // Upstream's server normalizes the expression before it waits, so its refusal comes first.
    if let Script::Refused(message) = &script {
        return Err(scope.fail(ErrorCode::InvalidInput, message));
    }
    let matched = AtomicBool::new(false);
    let tries = Box::pin(truthy_tries(pw, scope, &locator, strict, &script, &matched));
    let done = scope.within(tries).await;
    done.unwrap_or_else(|_expired| Err(timed_out(scope, &matched)))
}

/// The timeout, as the reads' wait for an element words it when nothing ever matched (the
/// script's locator, with nothing sent), and as `page.waitForFunction`'s once the predicate ran.
fn timed_out(scope: &OpScope<'_>, matched: &AtomicBool) -> PlaywrightError {
    scope.timed_out(if matched.load(Ordering::Relaxed) {
        ErrorCode::WaitTimeout
    } else {
        ErrorCode::NoMatch
    })
}

/// [`wait_for_function_on_element`]'s tries, which its deadline drops wherever they are.
async fn truthy_tries(
    pw: &Playwright,
    scope: &OpScope<'_>,
    locator: &LocatorTarget,
    strict: Strictness,
    script: &Script,
    matched: &AtomicBool,
) -> Result<Nothing, PlaywrightError> {
    scope.log(format!("waiting for {}", locator.display));
    let backoff = Backoff::outer(scope.timeout());
    for attempt in 0.. {
        if scope.sleep(backoff.wait(attempt)).await.is_err() {
            return Err(timed_out(scope, matched));
        }
        if truthy_try(pw, scope, locator, strict, script, matched).await? {
            return Ok(());
        }
        // Nothing of a try is used by the next, which matches the locator afresh.
        scope.release_objects().await;
    }
    Err(timed_out(scope, matched))
}

/// One try: the locator's match now, and whether `fn(element, arg)` was truthy on it. `false`
/// tries again: nothing matched, the match left the page before its script ran, or a document
/// replaced it.
async fn truthy_try(
    pw: &Playwright,
    scope: &OpScope<'_>,
    locator: &LocatorTarget,
    strict: Strictness,
    script: &Script,
    matched: &AtomicBool,
) -> Result<bool, PlaywrightError> {
    let Some(RootHandle::Element(element)) = read::find_one(pw, scope, locator, strict).await?
    else {
        return Ok(false);
    };
    matched.store(true, Ordering::Relaxed);
    let page = element.page().clone();
    let call = MainWorldCall::truthy(script.clone(), element);
    let mut guard = match EvaluateGuard::open(pw, scope, &page, call).await {
        Ok(guard) => guard,
        // The match went away between the query and its binding, as a re-render removes it:
        // upstream's query and call are one page call, so its next try finds the new one.
        Err(error)
            if matches!(
                error.error_code(),
                Some(ErrorCode::ElementDetached | ErrorCode::PageError)
            ) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    match guard.run(scope).await? {
        Outcome::Answered(answer) => match answer.finish(scope)?.value {
            Value::Bool(truthy) => Ok(truthy),
            _ => Err(scope.fail(
                ErrorCode::Internal,
                "a truthiness try answered something other than a boolean",
            )),
        },
        Outcome::NotRun | Outcome::Destroyed | Outcome::Navigated => Ok(false),
        Outcome::Falsy => Err(scope.fail(ErrorCode::Internal, "a script run once answered a poll")),
    }
}

/// One run through the guard. An element's script that found its document or a node gone
/// before it ran fails as upstream's evaluate on a replaced document does: its handle belongs
/// to that document, so there is nothing to try again.
async fn run_once(
    pw: &Playwright,
    scope: &OpScope<'_>,
    page: &PageRef,
    call: MainWorldCall,
) -> Result<SerializedValue, PlaywrightError> {
    let mut guard = EvaluateGuard::open(pw, scope, page, call).await?;
    match guard.run(scope).await? {
        Outcome::Answered(answer) => Ok(SerializedValue(answer.finish(scope)?.value)),
        Outcome::NotRun | Outcome::Destroyed => {
            Err(scope.fail(ErrorCode::PageError, DOCUMENT_REPLACED))
        }
        Outcome::Navigated => Err(scope.fail(ErrorCode::PageError, NAVIGATED)),
        Outcome::Falsy => Err(scope.fail(ErrorCode::Internal, "a script run once answered a poll")),
    }
}

/// `$eval`'s element: the first match now (strict only when the script asked), as upstream's
/// `querySelector`; nothing is upstream's `Failed to find element` error, which quotes the
/// selector as the script wrote it.
async fn first_match(
    pw: &Playwright,
    scope: &OpScope<'_>,
    locator: &LocatorTarget,
    strict: Strictness,
    selector: Option<&str>,
) -> Result<ElementTarget, PlaywrightError> {
    let Some(RootHandle::Element(element)) = read::find_one(pw, scope, locator, strict).await?
    else {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            &format!(
                "Failed to find element matching selector \"{}\"",
                selector.unwrap_or(&locator.display)
            ),
        ));
    };
    Ok(element)
}

/// `locator.evaluate`'s element: `waitForSelector` until a match is attached, its call log and
/// timeout included.
async fn attached(
    pw: &Playwright,
    scope: &OpScope<'_>,
    locator: &LocatorTarget,
    strict: Strictness,
) -> Result<ElementTarget, PlaywrightError> {
    let found = read::wait_for_match(
        pw,
        scope,
        locator,
        strict,
        SelectorState::Attached,
        Returning::Handle,
    )
    .await?;
    // A wait that meets `attached` with `Returning::Handle` answers the match's handle.
    let Some(RootHandle::Element(element)) = found else {
        return Err(scope.fail(
            ErrorCode::Internal,
            "a match did not make an element handle",
        ));
    };
    Ok(element)
}

/// The frame a query starts from, and its document: the root handle's frame for a query rooted
/// at one (`elementHandle.$$eval`), the frame the locator was built from (`frame.$$eval`,
/// `frame.locator`), else the page's main frame.
async fn starting_frame(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    locator: &LocatorTarget,
) -> Result<(TreeFrame, FrameSeq), PlaywrightError> {
    if let Some(RootHandle::Element(root)) = &locator.root {
        if matches!(page.frames().by_seq(root.seq), SeqLookup::Retired) {
            return Err(scope.fail(ErrorCode::PageError, DOCUMENT_REPLACED));
        }
        let site = HandleSite::find(page, scope, root)
            .await?
            .ok_or_else(|| resolve::handle_detached(scope))?;
        let world = site.world();
        return Ok((
            evaluate_guard::frame_endpoint(page, world.session(), world.frame().clone()),
            world.seq(),
        ));
    }
    let world = world_call::frame_world(page, scope, locator.frame.as_ref()).await?;
    Ok((
        evaluate_guard::frame_endpoint(page, world.session(), world.frame().clone()),
        world.seq(),
    ))
}
