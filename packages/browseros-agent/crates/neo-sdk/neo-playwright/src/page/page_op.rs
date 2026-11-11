//! The page ops, their registration metadata, and the small ones' handlers.

use browseros_core::{CoreError, ProtocolSession};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::{OpScope, Sent};
use crate::op_spec::{Capability, OpSpec, PAGE, TARGET};
use crate::page::document_op::DocumentInput;
use crate::page::document_watch::decode;
use crate::page::navigation::{GotoInput, NavigateInput, Navigation, NavigationResult};
use crate::page::{
    PageHandle, add_tag, document_op, element_evaluate, evaluate, evaluate_guard, frame_tree,
    init_script, media_emulation, navigation, pdf, save_screenshot, screenshot, viewport_size,
    wait_for_navigation, wait_for_url, web_storage,
};
use crate::probe::DocumentPart;
use crate::target::PageRef;
use crate::wire::{self, Describe, Nothing, OpWire, Route, Run, options};
use crate::{ErrorCode, Playwright, PlaywrightError};

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
        RequestGc,
        Close,
        Opener,
        Frames,
        Evaluate,
        WorkerEvaluate,
        EvaluateOnElement,
        EvaluateOnAll,
        WaitForFunction,
        WaitForFunctionOnElement,
        Screenshot,
        SaveScreenshot,
        Pdf,
        SetViewportSize,
        AddInitScript,
        ExposeBinding,
        ResolveBinding,
        WebStorageRead,
        WebStorageWrite,
    }
}

impl PageOp {
    /// Describes the ops on typed route rows from the same types the runtime route decodes and
    /// encodes; `None` for an op still on the untyped row.
    pub(crate) fn wire(self) -> Option<OpWire> {
        route(self, Describe)
    }

    /// This op's registration metadata.
    #[expect(
        clippy::too_many_lines,
        reason = "one registration row per page operation keeps the dispatch contract together"
    )]
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::WorkerEvaluate => OpSpec::write(
                "worker.evaluate",
                PAGE,
                "evaluate a script in a dedicated worker",
            )
            .using(&[Capability::MainWorld])
            .policed(),
            Self::SaveScreenshot => save_screenshot::SPEC,
            Self::WebStorageRead => web_storage::READ_SPEC,
            Self::WebStorageWrite => web_storage::WRITE_SPEC,
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
                "the document's HTML, read in the engine's isolated world",
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
            Self::RequestGc => OpSpec::write(
                "page.requestGC",
                PAGE,
                "request garbage collection in the page's main renderer",
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
            Self::ExposeBinding => EXPOSE_BINDING,
            Self::ResolveBinding => RESOLVE_BINDING,
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

/// `page.exposeBinding`'s registration (also `exposeFunction`): its stub and the controller run
/// in the page's main world (`page/init_script.rs`).
const EXPOSE_BINDING: OpSpec = OpSpec::write(
    "page.exposeBinding",
    PAGE,
    "let the page call a function of the script, until the run ends",
)
.using(&[Capability::MainWorld])
.policed();

/// The answer to one binding call, run in the main-world context that made it.
const RESOLVE_BINDING: OpSpec = OpSpec::write(
    "bindingCall.resolve",
    PAGE,
    "answer a page's call of an exposed binding with the script's result or error",
)
.using(&[Capability::MainWorld])
.policed();

/// The navigations' one field upstream's commands lack.
const PUBLICATION: &[(&str, &str)] = &[(
    "publication",
    "the run-local receipt the prelude stamps, correlating the host's navigation facts with \
     the call that caused them",
)];

/// One table binds each page op on a typed handler to Playwright 1.64.1's command; `Run` and
/// `Describe` walk the same rows, and the policy in `spec()` stays independent of the shapes.
/// The family moves in three changes: the ops not moved yet go through the untyped row to
/// `run_untyped`, and keep their hand-written TypeScript in `wire.ts` until theirs lands.
fn route<'a, R: Route<'a>>(op: PageOp, r: R) -> R::Out {
    match op {
        PageOp::Goto => r.op(wire::command("Frame.goto").adds(PUBLICATION), goto),
        PageOp::Reload => r.op(wire::command("Page.reload").adds(PUBLICATION), reload),
        PageOp::GoBack => r.op(wire::command("Page.goBack").adds(PUBLICATION), go_back),
        PageOp::GoForward => r.op(
            wire::command("Page.goForward").adds(PUBLICATION),
            go_forward,
        ),
        PageOp::WaitForNavigation => r.op(
            wire::neo_only(
                "a client-side wait upstream; Neo waits on its navigation journal in Rust",
            ),
            wait_for_navigation::wait_for_navigation,
        ),
        PageOp::WaitForUrl => r.op(
            wire::neo_only(
                "a client-side wait upstream; Neo matches the URL as part of a navigation wait",
            ),
            wait_for_url::wait_for_url,
        ),
        PageOp::WaitForLoadState => r.op(
            wire::neo_only("a client-side wait upstream; Neo waits on its lifecycle journal"),
            navigation::wait_for_load_state,
        ),
        PageOp::Title => r.op(wire::command("Frame.title"), title),
        PageOp::Content => r.op(
            wire::command("Frame.content").omits(&[(
                "includeShadow",
                "Neo returns the document's HTML; shadow-root serialization is not offered",
            )]),
            content,
        ),
        PageOp::SetContent => r.op(wire::command("Frame.setContent"), document_op::set_content),
        PageOp::Close => r.op(
            wire::command("Page.close")
                .adds(&[(
                    "runBeforeUnload",
                    "upstream sends Page.runBeforeUnload as a command of its own; Neo folds it \
                     into close, where the host's policy reads it",
                )])
                .omits(&[(
                    "reason",
                    "the script's close reason is not sent; a closed page's errors do not \
                     carry it",
                )]),
            close,
        ),
        PageOp::Opener => r.op(
            wire::neo_only("upstream reads its initializer state; Neo asks its tab bookkeeping"),
            opener,
        ),
        PageOp::Frames => r.op(
            wire::neo_only(
                "upstream keeps client-side frame objects; Neo reads the tree for the \
                 prelude's synchronous frames()",
            ),
            frame_tree::frames,
        ),
        op @ (PageOp::AddScriptTag
        | PageOp::AddStyleTag
        | PageOp::EmulateMedia
        | PageOp::RequestGc
        | PageOp::Evaluate
        | PageOp::WorkerEvaluate
        | PageOp::EvaluateOnElement
        | PageOp::EvaluateOnAll
        | PageOp::WaitForFunction
        | PageOp::WaitForFunctionOnElement
        | PageOp::Screenshot
        | PageOp::SaveScreenshot
        | PageOp::Pdf
        | PageOp::SetViewportSize
        | PageOp::AddInitScript
        | PageOp::ExposeBinding
        | PageOp::ResolveBinding
        | PageOp::WebStorageRead
        | PageOp::WebStorageWrite) => {
            r.untyped(move |pw, scope, args| run_untyped(pw, scope, op, args))
        }
    }
}

/// Runs one page op.
pub(crate) async fn invoke(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: PageOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    route(op, Run { pw, scope, args }).await
}

/// The page ops not on typed rows yet, decoded and answered as JSON here. The ops that run in
/// the page's main world (evaluate, waitForFunction, the element scripts) are built with it.
async fn run_untyped(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: PageOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    match op {
        PageOp::SetViewportSize => viewport_size::set(pw, scope, args.decode(scope)?).await,
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
        PageOp::RequestGc => request_gc(pw, scope, args.decode(scope)?).await,
        PageOp::Screenshot => screenshot::capture(pw, scope, args.decode(scope)?).await,
        PageOp::SaveScreenshot => save_screenshot::save(scope, args.decode(scope)?).await,
        PageOp::WebStorageRead => web_storage::call(pw, scope, args.decode(scope)?, false).await,
        PageOp::WebStorageWrite => web_storage::call(pw, scope, args.decode(scope)?, true).await,
        PageOp::Pdf => pdf::print(pw, scope, args.decode(scope)?).await,
        PageOp::AddInitScript => init_script::add_to_page(pw, scope, args.decode(scope)?).await,
        PageOp::ExposeBinding => {
            Box::pin(init_script::expose_on_page(pw, scope, args.decode(scope)?)).await
        }
        PageOp::ResolveBinding => {
            evaluate_guard::resolve_binding(pw, scope, args.decode(scope)?).await
        }
        // Boxed, as the other families box their leaves: a script's guard and its polls are the
        // largest state a page op holds, and every op's future is as large as its largest leaf.
        PageOp::Evaluate => Box::pin(evaluate::evaluate(pw, scope, args.decode(scope)?)).await,
        PageOp::WaitForFunction => {
            Box::pin(evaluate::wait_for_function(pw, scope, args.decode(scope)?)).await
        }
        PageOp::WorkerEvaluate => {
            super::worker_evaluate::evaluate(pw, scope, args.decode(scope)?).await
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
        // The typed rows never reach the untyped arm (`route`).
        PageOp::Goto
        | PageOp::Reload
        | PageOp::GoBack
        | PageOp::GoForward
        | PageOp::WaitForNavigation
        | PageOp::WaitForUrl
        | PageOp::WaitForLoadState
        | PageOp::Title
        | PageOp::Content
        | PageOp::SetContent
        | PageOp::Close
        | PageOp::Opener
        | PageOp::Frames => Err(scope.not_built()),
    }
}

/// `page.goto` and `frame.goto`.
async fn goto(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: GotoInput,
) -> Result<NavigationResult, PlaywrightError> {
    let (page, navigation, wait_until, publication) = input.into_parts();
    navigation::navigate(pw, scope, page.tab(), navigation, wait_until, publication).await
}

async fn reload(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: NavigateInput,
) -> Result<NavigationResult, PlaywrightError> {
    renavigate(pw, scope, input, Navigation::Reload).await
}

async fn go_back(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: NavigateInput,
) -> Result<NavigationResult, PlaywrightError> {
    renavigate(pw, scope, input, Navigation::History(-1)).await
}

async fn go_forward(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: NavigateInput,
) -> Result<NavigationResult, PlaywrightError> {
    renavigate(pw, scope, input, Navigation::History(1)).await
}

/// `reload`, `goBack` and `goForward`, which take only `waitUntil`.
async fn renavigate(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: NavigateInput,
    navigation: Navigation,
) -> Result<NavigationResult, PlaywrightError> {
    let (page, wait_until, publication) = input.into_parts();
    navigation::navigate(pw, scope, page.tab(), navigation, wait_until, publication).await
}

async fn title(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: DocumentInput,
) -> Result<String, PlaywrightError> {
    document_op::read(pw, scope, input, DocumentPart::Title).await
}

async fn content(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: DocumentInput,
) -> Result<String, PlaywrightError> {
    document_op::read(pw, scope, input, DocumentPart::Content).await
}

/// `page.close({ runBeforeUnload })`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CloseInput {
    page: PageRef,
    /// Run the page's `beforeunload` handlers first. Their dialog follows the dialog rules
    /// (a listener's answer, or accepted after the grace), and the close returns at once,
    /// as upstream does. The prelude sends it only when set, so it prints as optional.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    run_before_unload: bool,
}
options!(CloseInput);

async fn close(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: CloseInput,
) -> Result<Nothing, PlaywrightError> {
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
        pw.workers_closed(admitted.session.target_id.clone()).await;
        scope.run_state().frames.forget(page);
    }
    Ok(())
}

/// `Page.close`: the page runs its `beforeunload` handlers and closes unless one cancels.
async fn close_with_unload(session: &ProtocolSession, _sent: &Sent<'_>) -> Result<(), CoreError> {
    session.send_value("Page.close", json!({})).await?;
    Ok(())
}

/// Page methods without options (`opener`, `requestGC`): `{ page }`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "PageOnlyInput")]
struct PageInput {
    page: PageRef,
}
options!(PageInput);

/// Upstream collects on the main renderer session. Treat collection as a write because
/// finalizers can execute page code; recheck the host immediately before the CDP command.
async fn request_gc(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PageInput,
) -> Result<Value, PlaywrightError> {
    let admitted = scope.admit(pw.browser(), input.page.tab()).await?;
    let session = pw
        .browser()
        .protocol_session(admitted.session.session_id.clone());
    let _sent = scope.send_effect()?;
    scope
        .within(session.send_value("HeapProfiler.collectGarbage", json!({})))
        .await
        .map_err(|_expired| scope.timed_out(ErrorCode::BrowserDisconnected))?
        .map_err(|error| scope.core_error(&error))?;
    Ok(Value::Null)
}

/// The tab that opened this one while it is still open, as upstream: `{ opener: page | null }`.
async fn opener(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PageInput,
) -> Result<OpenerResult, PlaywrightError> {
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
    Ok(OpenerResult { opener })
}

/// `page.opener()`'s result.
#[derive(Debug, Serialize, TS)]
struct OpenerResult {
    opener: Option<PageHandle>,
}
