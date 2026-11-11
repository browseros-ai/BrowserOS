//! The page ops, their registration metadata, and the small ones' handlers.

use browseros_core::{CoreError, ProtocolSession};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_output::to_output;
use crate::op_scope::{OpScope, Sent};
use crate::op_spec::{Capability, OpSpec, PAGE, TARGET};
use crate::page::document_watch::decode;
use crate::page::navigation::{GotoInput, NavigateInput, Navigation};
use crate::page::{
    PageHandle, add_tag, document_op, element_evaluate, evaluate, frame_tree, init_script,
    media_emulation, navigation, pdf, save_screenshot, screenshot, viewport_size,
    wait_for_navigation, wait_for_url,
};
use crate::probe::DocumentPart;
use crate::target::PageRef;
use crate::{Playwright, PlaywrightError};

listed_enum! {
    /// Page and frame leaves. A frame's form of a method (`frame.title`, `frame.evaluate`,
    /// `frame.waitForLoadState`) is the page op with a frame in its input.
    pub(crate) enum PageOp {
        Goto,
        Reload,
        GoBack,
        GoForward,
        WaitForNavigation,
        WaitForUrl,
        WaitForLoadState,
        Title,
        Content,
        SetContent,
        AddScriptTag,
        AddStyleTag,
        EmulateMedia,
        Close,
        Opener,
        Frames,
        Evaluate,
        EvaluateOnElement,
        EvaluateOnAll,
        WaitForFunction,
        WaitForFunctionOnElement,
        Screenshot,
        SaveScreenshot,
        Pdf,
        SetViewportSize,
        AddInitScript,
    }
}

impl PageOp {
    /// This op's registration metadata.
    #[expect(
        clippy::too_many_lines,
        reason = "one registration row per page operation keeps the dispatch contract together"
    )]
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::SaveScreenshot => save_screenshot::SPEC,
            Self::SetViewportSize => viewport_size::SPEC,
            Self::Goto => OpSpec::write(
                "page.goto",
                PAGE,
                "navigate and wait for the load state; the response or null",
            ),
            Self::Reload => {
                OpSpec::write("page.reload", PAGE, "reload and wait for the load state")
            }
            Self::GoBack => OpSpec::write("page.goBack", PAGE, "go back in the tab's history"),
            Self::GoForward => {
                OpSpec::write("page.goForward", PAGE, "go forward in the tab's history")
            }
            Self::WaitForNavigation => OpSpec::read(
                "page.waitForNavigation",
                PAGE,
                "wait for the next matching navigation and return its response or null",
            ),
            Self::WaitForUrl => OpSpec::read(
                "page.waitForURL",
                PAGE,
                "wait until the main frame's URL matches a string, glob or RegExp",
            ),
            Self::WaitForLoadState => OpSpec::read(
                "page.waitForLoadState",
                PAGE,
                "wait for a load state of the page or one of its frames",
            ),
            Self::Title => OpSpec::read("page.title", PAGE, "the document's title"),
            Self::Content => OpSpec::read(
                "page.content",
                PAGE,
                "the document's HTML, read in the sealed world so field values are masked",
            ),
            Self::SetContent => OpSpec::write(
                "page.setContent",
                PAGE,
                "replace the document's HTML and wait for the load state",
            )
            .using(&[Capability::MainWorld])
            .policed(),
            Self::AddScriptTag => OpSpec::write(
                "page.addScriptTag",
                PAGE,
                "insert a script and return its element",
            )
            .using(&[Capability::MainWorld, Capability::HostFiles])
            .policed(),
            Self::AddStyleTag => OpSpec::write(
                "page.addStyleTag",
                PAGE,
                "insert a stylesheet and return its element",
            )
            .using(&[Capability::MainWorld, Capability::HostFiles])
            .policed(),
            Self::EmulateMedia => OpSpec::write(
                "page.emulateMedia",
                PAGE,
                "set page media preferences across renderer and run lifetimes",
            ),
            Self::Close => OpSpec::write("page.close", PAGE, "close the tab"),
            Self::Opener => OpSpec::read("page.opener", PAGE, "the page that opened this one"),
            Self::Frames => OpSpec::read(
                "page.frames",
                PAGE,
                "the frame tree, for the prelude's synchronous frames() and mainFrame()",
            ),
            Self::Evaluate => OpSpec::write(
                "page.evaluate",
                PAGE,
                "run a function in the main world, as Playwright's page.evaluate does",
            )
            .using(&[Capability::MainWorld])
            .policed(),
            Self::EvaluateOnElement => OpSpec::write(
                "locator.evaluate",
                TARGET,
                "run a function on one element in the main world; also $eval",
            )
            .using(&[Capability::MainWorld])
            .policed(),
            Self::EvaluateOnAll => OpSpec::write(
                "locator.evaluateAll",
                TARGET,
                "run a function on every match in the main world; also $$eval",
            )
            .using(&[Capability::MainWorld])
            .policed(),
            Self::WaitForFunction => OpSpec::write(
                "page.waitForFunction",
                PAGE,
                "poll a main-world function from Rust until it returns truthy",
            )
            .using(&[Capability::MainWorld])
            .policed(),
            Self::WaitForFunctionOnElement => OpSpec::write(
                "locator.waitForFunction",
                TARGET,
                "call a main-world function with the locator's element until it returns truthy",
            )
            .using(&[Capability::MainWorld])
            .policed(),
            Self::Screenshot => {
                OpSpec::read("page.screenshot", PAGE, "capture the page or one element")
            }
            Self::Pdf => OpSpec::read("page.pdf", PAGE, "print the page to PDF"),
            Self::AddInitScript => ADD_INIT_SCRIPT,
        }
    }
}

/// `page.addInitScript`'s registration: it reaches the main world of every later document, and
/// its `{ path }` form reads a file from Neo's disk (`page/init_script.rs`).
const ADD_INIT_SCRIPT: OpSpec = OpSpec::write(
    "page.addInitScript",
    PAGE,
    "run a script in the main world of every new document of the page, until the run ends",
)
.using(&[Capability::MainWorld, Capability::HostFiles])
.policed();

/// Runs one page op. The ops that run in the page's sealed world (title, content, setContent,
/// waitForURL) or its main world (evaluate, waitForFunction, the element scripts) are built with
/// those worlds.
pub(crate) async fn invoke(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: PageOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    match op {
        PageOp::SetViewportSize => viewport_size::set(pw, scope, args.decode(scope)?).await,
        PageOp::Goto => {
            let (page, navigation, wait_until) = args.decode::<GotoInput>(scope)?.into_parts();
            let page = page.tab();
            navigation::navigate(pw, scope, page, navigation, wait_until).await
        }
        PageOp::Reload => renavigate(pw, scope, args, Navigation::Reload).await,
        PageOp::GoBack => renavigate(pw, scope, args, Navigation::History(-1)).await,
        PageOp::GoForward => renavigate(pw, scope, args, Navigation::History(1)).await,
        PageOp::WaitForLoadState => {
            navigation::wait_for_load_state(pw, scope, args.decode(scope)?).await
        }
        PageOp::AddScriptTag => {
            Box::pin(add_tag::insert(
                pw,
                scope,
                args.decode(scope)?,
                add_tag::TagKind::Script,
            ))
            .await
        }
        PageOp::AddStyleTag => {
            Box::pin(add_tag::insert(
                pw,
                scope,
                args.decode(scope)?,
                add_tag::TagKind::Style,
            ))
            .await
        }
        PageOp::EmulateMedia => {
            Box::pin(media_emulation::emulate(pw, scope, args.decode(scope)?)).await
        }
        PageOp::Close => close(pw, scope, args.decode(scope)?).await,
        PageOp::Opener => opener(pw, scope, args.decode(scope)?).await,
        PageOp::Frames => frame_tree::frames(pw, scope, args.decode(scope)?).await,
        PageOp::Screenshot => screenshot::capture(pw, scope, args.decode(scope)?).await,
        PageOp::SaveScreenshot => save_screenshot::save(scope, args.decode(scope)?).await,
        PageOp::Pdf => pdf::print(pw, scope, args.decode(scope)?).await,
        PageOp::AddInitScript => init_script::add_to_page(pw, scope, args.decode(scope)?).await,
        // Boxed, as the other families box their leaves: a script's guard and its polls are the
        // largest state a page op holds, and every op's future is as large as its largest leaf.
        PageOp::Evaluate => Box::pin(evaluate::evaluate(pw, scope, args.decode(scope)?)).await,
        PageOp::WaitForFunction => {
            Box::pin(evaluate::wait_for_function(pw, scope, args.decode(scope)?)).await
        }
        PageOp::Title => {
            document_op::read(pw, scope, args.decode(scope)?, DocumentPart::Title).await
        }
        PageOp::Content => {
            document_op::read(pw, scope, args.decode(scope)?, DocumentPart::Content).await
        }
        // Boxed for the same reason: each holds its frame's world install inside its timeout.
        PageOp::SetContent => {
            Box::pin(document_op::set_content(pw, scope, args.decode(scope)?)).await
        }
        PageOp::WaitForNavigation => {
            Box::pin(wait_for_navigation::wait_for_navigation(
                pw,
                scope,
                args.decode(scope)?,
            ))
            .await
        }
        PageOp::WaitForUrl => {
            Box::pin(wait_for_url::wait_for_url(pw, scope, args.decode(scope)?)).await
        }
        PageOp::EvaluateOnElement => {
            Box::pin(element_evaluate::evaluate_on_element(
                pw,
                scope,
                args.decode(scope)?,
            ))
            .await
        }
        PageOp::EvaluateOnAll => {
            Box::pin(element_evaluate::evaluate_on_all(
                pw,
                scope,
                args.decode(scope)?,
            ))
            .await
        }
        PageOp::WaitForFunctionOnElement => {
            Box::pin(element_evaluate::wait_for_function_on_element(
                pw,
                scope,
                args.decode(scope)?,
            ))
            .await
        }
    }
}

/// `reload`, `goBack` and `goForward`, which take only `waitUntil`.
async fn renavigate(
    pw: &Playwright,
    scope: &OpScope<'_>,
    args: OpArgs,
    navigation: Navigation,
) -> Result<Value, PlaywrightError> {
    let (page, wait_until) = args.decode::<NavigateInput>(scope)?.into_parts();
    let page = page.tab();
    navigation::navigate(pw, scope, page, navigation, wait_until).await
}

/// `page.close({ runBeforeUnload })`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CloseInput {
    page: PageRef,
    /// Run the page's `beforeunload` handlers first. Their dialog follows the dialog rules
    /// (a listener's answer, or accepted after the grace), and the close returns at once,
    /// as upstream does.
    #[serde(default)]
    run_before_unload: bool,
}

async fn close(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: CloseInput,
) -> Result<Value, PlaywrightError> {
    let page = input.page.tab();
    let admitted = scope.admit(pw.browser(), page).await?;
    let core = |error| scope.core_error(&error);
    let sent = scope.send_effect()?;
    if input.run_before_unload {
        let session = pw
            .browser()
            .protocol_session(admitted.session.session_id.clone());
        close_with_unload(&session, &sent).await.map_err(core)?;
    } else {
        // Through core, so its tab registry forgets the page and the run's view still decides.
        scope
            .view()
            .close_page(admitted.core_page.clone())
            .await
            .map_err(core)?;
    }
    if !input.run_before_unload {
        scope.run_state().frames.forget(page);
    }
    Ok(json!({}))
}

/// `Page.close`: the page runs its `beforeunload` handlers and closes unless one cancels.
async fn close_with_unload(session: &ProtocolSession, _sent: &Sent<'_>) -> Result<(), CoreError> {
    session.send_value("Page.close", json!({})).await?;
    Ok(())
}

/// `page.opener()`: `{ page }`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenerInput {
    page: PageRef,
}

/// The tab that opened this one while it is still open, as upstream: `{ opener: page | null }`.
async fn opener(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: OpenerInput,
) -> Result<Value, PlaywrightError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TargetInfoReply {
        target_info: TargetFields,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct TargetFields {
        #[serde(default)]
        opener_id: Option<String>,
    }
    let admitted = scope.admit(pw.browser(), input.page.tab()).await?;
    let core = |error| scope.core_error(&error);
    let info: TargetInfoReply = pw
        .browser()
        .cdp(
            "Target.getTargetInfo",
            json!({ "targetId": admitted.session.target_id.as_str() }),
            None,
        )
        .await
        .and_then(decode)
        .map_err(core)?;
    let opener = match info.target_info.opener_id {
        Some(opener) => scope
            .view()
            .list_pages()
            .await
            .map_err(core)?
            .pages
            .into_iter()
            .find(|page| page.target_id.as_str() == opener)
            .map(|page| PageHandle::new(page.tab_id)),
        None => None,
    };
    to_output(scope, &OpenerResult { opener })
}

/// `page.opener()`'s result.
#[derive(Debug, Serialize)]
struct OpenerResult {
    opener: Option<PageHandle>,
}
