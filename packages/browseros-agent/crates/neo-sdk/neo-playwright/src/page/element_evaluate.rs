//! `locator.evaluate` and `locator.evaluateAll`, with their element-handle, page and frame forms
//! (`elementHandle.evaluate`, `$eval`, `$$eval`): the agent's function called with elements, in
//! the main world of the frame they are in, through the main-world gateway (`evaluate_guard`).
//! Both answer in Playwright's serialized format, which the prelude reads back with the vendored
//! `parseEvaluationResultValue`.
//!
//! ```text
//!  locator.evaluate ── waitForSelector(attached, strict) ─▶ handle ─┐  upstream's `_withElement`
//!  $eval (noWait) ──── the first match now, or upstream's failure ──┤
//!  elementHandle.evaluate ──────────────────────────────────────────┴▶ Subject::Element
//!  locator.evaluateAll, $$eval ── every match now, pinned ─▶ Subject::Elements in their frame
//!                                 nothing matched, or a frame on the way missing ─▶ fn([]) in
//!                                 the query's starting frame (measured, spike S-detached)
//! ```
//!
//! Elements reach the main world by backend node id, never as sealed-world objects: an object
//! of one world cannot be an argument in another, and the main world resolves its own (the
//! guard's `run`).

use serde::Deserialize;
use serde_json::Value;

use crate::op_scope::OpScope;
use crate::page::evaluate::ScriptInput;
use crate::page::evaluate_guard::{
    self, DOCUMENT_REPLACED, EvaluateGuard, MainWorldCall, NAVIGATED, Outcome, Subject, TreeFrame,
};
use crate::page::{OpPage, world_call};
use crate::read::{self, Returning, SelectorState};
use crate::resolve::{self, HandleSite};
use crate::target::{ElementTarget, LocatorTarget, PageRef, RootHandle, Strictness, Target};
use crate::world::{FrameSeq, SeqLookup};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// `locator.evaluate(fn, arg)`, `elementHandle.evaluate`, and `$eval` with `noWait`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct EvaluateOnElementInput {
    target: Target,
    strict: Strictness,
    script: ScriptInput,
    /// `$eval`: the first match now, or upstream's failure, instead of waiting for an element.
    #[serde(default)]
    no_wait: bool,
    /// `$eval`'s selector as the script wrote it, which upstream's no-match message quotes.
    #[serde(default)]
    selector: Option<String>,
}

/// `locator.evaluateAll(fn, arg)` and `$$eval`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct EvaluateOnAllInput {
    target: Target,
    script: ScriptInput,
}

/// Runs `fn(element, arg)`. A locator waits for its element as upstream's `_withElement` does
/// (attached, strict, until the op's timeout), and the script itself has no timeout but the
/// run's; `$eval` takes the first match now. An element handle runs on its node, in its
/// document or out of it, as upstream's does.
pub(crate) async fn evaluate_on_element(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: EvaluateOnElementInput,
) -> Result<Value, PlaywrightError> {
    let EvaluateOnElementInput {
        target,
        strict,
        script,
        no_wait,
        selector,
    } = input;
    let element = match target {
        Target::Element(handle) => handle,
        Target::Locator(locator) if no_wait => {
            first_match(pw, scope, &locator, strict, selector.as_deref()).await?
        }
        Target::Locator(locator) => attached(pw, scope, &locator, strict).await?,
    };
    let page = element.page().clone();
    let call = MainWorldCall::once(script.into(), Subject::Element(element));
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
) -> Result<Value, PlaywrightError> {
    let EvaluateOnAllInput { target, script } = input;
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
        MainWorldCall::once(script.into(), subject),
    )
    .await
}

/// One run through the guard. An element's script that found its document or a node gone
/// before it ran fails as upstream's evaluate on a replaced document does: its handle belongs
/// to that document, so there is nothing to try again.
async fn run_once(
    pw: &Playwright,
    scope: &OpScope<'_>,
    page: &PageRef,
    call: MainWorldCall,
) -> Result<Value, PlaywrightError> {
    let mut guard = EvaluateGuard::open(pw, scope, page, call).await?;
    match guard.run(scope).await? {
        Outcome::Answered(answer) => Ok(answer.finish(scope)?.value),
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
    let found = read::query_one(pw, scope, locator, strict).await?;
    if found.is_null() {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            &format!(
                "Failed to find element matching selector \"{}\"",
                selector.unwrap_or(&locator.display)
            ),
        ));
    }
    handle_of(scope, found)
}

/// `locator.evaluate`'s element: `waitForSelector` until a match is attached, its call log and
/// timeout included.
async fn attached(
    pw: &Playwright,
    scope: &OpScope<'_>,
    locator: &LocatorTarget,
    strict: Strictness,
) -> Result<ElementTarget, PlaywrightError> {
    let found = read::wait_for_selector(
        pw,
        scope,
        locator,
        strict,
        SelectorState::Attached,
        Returning::Handle,
    )
    .await?;
    handle_of(scope, found)
}

/// The handle a match was made into, read back as the target a script would send it as.
fn handle_of(scope: &OpScope<'_>, brand: Value) -> Result<ElementTarget, PlaywrightError> {
    match serde_json::from_value(brand) {
        Ok(Target::Element(handle)) => Ok(handle),
        Ok(Target::Locator(_)) | Err(_) => Err(scope.fail(
            ErrorCode::Internal,
            "a match did not make an element handle",
        )),
    }
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
