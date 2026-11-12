//! The page ops, their registration metadata, and the small ones' handlers.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
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
        RunBeforeUnload,
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
        WebStorageItems,
        WebStorageGetItem,
        WebStorageSetItem,
        WebStorageRemoveItem,
        WebStorageClear,
    }
}

impl PageOp {
    /// Describes the op from the same types the runtime route decodes and encodes: `Some` for
    /// every page op, since all of them are on typed route rows.
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
            Self::WebStorageItems => web_storage::ITEMS_SPEC,
            Self::WebStorageGetItem => web_storage::GET_ITEM_SPEC,
            Self::WebStorageSetItem => web_storage::SET_ITEM_SPEC,
            Self::WebStorageRemoveItem => web_storage::REMOVE_ITEM_SPEC,
            Self::WebStorageClear => web_storage::CLEAR_SPEC,
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
            Self::RunBeforeUnload => OpSpec::write(
                "page.runBeforeUnload",
                PAGE,
                "run beforeunload handlers and request a close without waiting for it",
            ),
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

/// The evaluating ops' one field upstream's commands lack.
const SCRIPT: (&str, &str) = (
    "script",
    "the script as one value: upstream's expression, isFunction and argument after its client \
     normalized them, or the refusal of upstream's first server-side check, which Rust raises \
     at its point with nothing sent",
);

/// What `script` carries in upstream's commands.
const EVALUATION: [(&str, &str); 3] = [
    ("arg", "carried in script"),
    ("expression", "carried in script"),
    ("isFunction", "carried in script"),
];

/// The navigations' one field upstream's commands lack.
const PUBLICATION: &[(&str, &str)] = &[(
    "publication",
    "the run-local receipt the prelude stamps, correlating the host's navigation facts with \
     the call that caused them",
)];

/// Navigation replies carry the facts behind the prelude's synchronous page methods.
const NAVIGATION_RESULT: &str = "page.url(), page.frames() and the Response are synchronous or \
    object-less in Neo, so the navigation answers the url, the frames and the response facts \
    the prelude caches";

/// One table binds each page op on a typed handler to Playwright 1.64.1's command; `Run` and
/// `Describe` walk the same rows, and the policy in `spec()` stays independent of the shapes.
#[expect(
    clippy::too_many_lines,
    reason = "one exhaustive row per page operation keeps the wire contract together"
)]
fn route<'a, R: Route<'a>>(op: PageOp, r: R) -> R::Out {
    match op {
        PageOp::Goto => r.op(
            wire::command("Frame.goto")
                .adds(PUBLICATION)
                .result(NAVIGATION_RESULT),
            goto,
        ),
        PageOp::Reload => r.op(
            wire::command("Page.reload")
                .adds(PUBLICATION)
                .result(NAVIGATION_RESULT),
            reload,
        ),
        PageOp::GoBack => r.op(
            wire::command("Page.goBack")
                .adds(PUBLICATION)
                .result(NAVIGATION_RESULT),
            go_back,
        ),
        PageOp::GoForward => r.op(
            wire::command("Page.goForward")
                .adds(PUBLICATION)
                .result(NAVIGATION_RESULT),
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
                "shadow-root serialization is not offered (parity gap PA-025)",
            )]),
            content,
        ),
        PageOp::SetContent => r.op(wire::command("Frame.setContent"), document_op::set_content),
        PageOp::Close => r.op(
            wire::command("Page.close").omits(&[(
                "reason",
                "Neo reports the browser's own close and attaches no reason text to later errors",
            )]),
            close,
        ),
        PageOp::RunBeforeUnload => r.op(wire::command("Page.runBeforeUnload"), run_before_unload),
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
        PageOp::AddScriptTag => r.op(
            wire::command("Frame.addScriptTag").adds(&[("path", "the Neo host reads the source file; upstream's client reads it and sends content")])
                .result("the prelude consumes element and frame facts to refresh synchronous frames() after a tag adds frames"),
            add_tag::script,
        ),
        PageOp::AddStyleTag => r.op(
            wire::command("Frame.addStyleTag").adds(&[("path", "the Neo host reads the stylesheet; upstream's client reads it and sends content")])
                .result("the prelude consumes element and frame facts to refresh synchronous frames() after insertion"),
            add_tag::style,
        ),
        PageOp::EmulateMedia => r.op(wire::command("Page.emulateMedia"), media_emulation::emulate),
        PageOp::RequestGc => r.op(wire::command("Page.requestGC"), request_gc),
        PageOp::SetViewportSize => r.op(
            wire::command("Page.setViewportSize"),
            viewport_size::set,
        ),
        PageOp::Screenshot => r.op(
            wire::command("Page.screenshot").retypes(&[("mask", "mask locators travel as Neo brands instead of frame channels and selectors")]),
            screenshot::capture,
        ),
        PageOp::SaveScreenshot => r.op(
            wire::neo_only("the screenshot client's path step writes captured bytes on Neo's disk"),
            save_screenshot,
        ),
        PageOp::Pdf => r.op(wire::command("Page.pdf"), pdf::print),
        PageOp::WebStorageItems => r.op(wire::command("Page.webStorageItems"), web_storage::items),
        PageOp::WebStorageGetItem => r.op(wire::command("Page.webStorageGetItem"), web_storage::get_item),
        PageOp::WebStorageSetItem => r.op(wire::command("Page.webStorageSetItem"), web_storage::set_item),
        PageOp::WebStorageRemoveItem => r.op(wire::command("Page.webStorageRemoveItem"), web_storage::remove_item),
        PageOp::WebStorageClear => r.op(wire::command("Page.webStorageClear"), web_storage::clear),
        // The script ops box their handlers, as the other families box their leaves: a script's
        // guard and its polls are the largest state a page op holds, and every op's future is as
        // large as its largest leaf.
        PageOp::Evaluate => r.op(
            wire::command("Frame.evaluateExpression")
                .adds(&[SCRIPT])
                .omits(&EVALUATION),
            |pw, scope, input| Box::pin(evaluate::evaluate(pw, scope, input)),
        ),
        PageOp::WorkerEvaluate => r.op(
            wire::command("Worker.evaluateExpression")
                .adds(&[
                    SCRIPT,
                    (
                        "worker",
                        "the host-owned worker's id, standing for upstream's worker channel",
                    ),
                ])
                .omits(&EVALUATION),
            super::worker_evaluate::evaluate,
        ),
        PageOp::EvaluateOnElement => r.op(
            wire::command("Frame.evalOnSelector")
                .adds(&[
                    (
                        "noWait",
                        "`$eval`: the first match now instead of waiting for one",
                    ),
                    SCRIPT,
                    (
                        "selector",
                        "`$eval`'s selector as the script wrote it, for upstream's no-match \
                         message",
                    ),
                ])
                .omits(&EVALUATION),
            |pw, scope, input| Box::pin(element_evaluate::evaluate_on_element(pw, scope, input)),
        ),
        PageOp::EvaluateOnAll => r.op(
            wire::command("Frame.evalOnSelectorAll")
                .adds(&[SCRIPT])
                .omits(&EVALUATION),
            |pw, scope, input| Box::pin(element_evaluate::evaluate_on_all(pw, scope, input)),
        ),
        PageOp::WaitForFunction => r.op(
            wire::command("Frame.waitForFunction")
                .adds(&[
                    (
                        "polling",
                        "milliseconds between polls, absent for 'raf', in place of \
                         pollingInterval",
                    ),
                    SCRIPT,
                ])
                .omits(&[
                    ("arg", "carried in script"),
                    ("expression", "carried in script"),
                    ("isFunction", "carried in script"),
                    ("pollingInterval", "sent as polling"),
                ]),
            |pw, scope, input| Box::pin(evaluate::wait_for_function(pw, scope, input)),
        ),
        PageOp::WaitForFunctionOnElement => r.op(
            wire::neo_only(
                "Neo probes the element predicate; upstream composes evaluation and waiting",
            ),
            |pw, scope, input| {
                Box::pin(element_evaluate::wait_for_function_on_element(
                    pw, scope, input,
                ))
            },
        ),
        PageOp::AddInitScript => r.op(
            wire::command("Page.addInitScript")
                .adds(&[(
                    "script",
                    "the source, or a path Rust reads on Neo's disk as upstream's client reads it",
                )])
                .omits(&[("source", "folded into script")]),
            init_script::add_to_page,
        ),
        PageOp::ExposeBinding => r.op(wire::command("Page.exposeBinding"), |pw, scope, input| {
            Box::pin(init_script::expose_on_page(pw, scope, input))
        }),
        PageOp::ResolveBinding => r.op(
            wire::command("BindingCall.resolve")
                .adds(&[
                    (
                        "arg",
                        "the binding's JSON completion upstream's server builds for the page's \
                         stub, as text, which serves resolve and reject",
                    ),
                    (
                        "context",
                        "Neo keeps no BindingCall object, so the execution context that called \
                         is named",
                    ),
                ])
                .omits(&[("result", "carried in arg")])
                .result("answers {} as it always has; the prelude reads nothing from it"),
            evaluate_guard::resolve_binding,
        ),
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

async fn close(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PageInput,
) -> Result<Nothing, PlaywrightError> {
    let page = input.page.tab();
    let admitted = scope.admit(pw.browser(), page).await?;
    let core = |error| scope.core_error(&error);
    let _sent = scope.send_effect()?;
    // Through core, so its tab registry forgets the page and the run's view still decides.
    scope
        .view()
        .close_page(admitted.core_page.clone())
        .await
        .map_err(core)?;
    pw.workers_closed(admitted.session.target_id.clone()).await;
    scope.run_state().frames.forget(page);
    Ok(())
}

/// CDP's `Page.close` runs beforeunload and returns before the tab necessarily closes. Leave
/// workers and frame facts alive: a dialog can veto it, and the tab watcher owns later cleanup.
async fn run_before_unload(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PageInput,
) -> Result<Nothing, PlaywrightError> {
    let admitted = scope.admit(pw.browser(), input.page.tab()).await?;
    let session = pw
        .browser()
        .protocol_session(admitted.session.session_id.clone());
    let _sent = scope.send_effect()?;
    session
        .send_value("Page.close", json!({}))
        .await
        .map_err(|error| scope.core_error(&error))?;
    Ok(())
}

/// Page methods without options (`close`, `runBeforeUnload`, `opener`, `requestGC`): `{ page }`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "PageOnlyInput")]
struct PageInput {
    page: PageRef,
}
options!(PageInput);

/// The disk write has no page target; capture has already passed the tab's read policies.
async fn save_screenshot(
    _pw: &Playwright,
    scope: &OpScope<'_>,
    input: save_screenshot::SaveScreenshotInput,
) -> Result<Nothing, PlaywrightError> {
    save_screenshot::save(scope, input).await
}

/// Upstream collects on the main renderer session. Treat collection as a write because
/// finalizers can execute page code; recheck the host immediately before the CDP command.
async fn request_gc(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PageInput,
) -> Result<Nothing, PlaywrightError> {
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
    Ok(())
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

#[cfg(test)]
mod wire_tests {
    use std::error::Error;

    use serde::Deserialize;

    use super::PageInput;
    use crate::Op;
    use crate::op_input::OpInput;
    use crate::op_scope::OpScope;
    use crate::page::{
        add_tag, media_emulation, pdf, save_screenshot, screenshot, viewport_size, web_storage,
    };
    use crate::test_env::TestEnv;

    #[derive(Deserialize)]
    struct Fixture {
        scenarios: Vec<Scenario>,
    }
    #[derive(Deserialize)]
    struct Scenario {
        exchanges: Vec<Exchange>,
    }
    #[derive(Deserialize)]
    struct Exchange {
        op: String,
        args: String,
    }

    #[tokio::test]
    async fn aligned_inputs_decode_every_recording() -> Result<(), Box<dyn Error>> {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../tests/data/page-c-aligned-wire.json"))?;
        let env = TestEnv::new();
        for exchange in fixture
            .scenarios
            .into_iter()
            .flat_map(|scenario| scenario.exchanges)
        {
            let op = Op::from_name(&exchange.op).ok_or("recorded op exists")?;
            let OpInput { call, args } =
                OpInput::decode(op, serde_json::from_str(&exchange.args)?)?;
            let scope = OpScope::new(&env, op, call);
            macro_rules! decodes {
                ($ty:ty) => {{
                    let _: $ty = args.decode(&scope)?;
                }};
            }
            match exchange.op.as_str() {
                "page.requestGC" => decodes!(PageInput),
                "page.emulateMedia" => decodes!(media_emulation::MediaInput),
                "page.setViewportSize" => decodes!(viewport_size::ViewportSizeInput),
                "page.addScriptTag" => decodes!(add_tag::AddTagInput),
                "page.addStyleTag" => decodes!(add_tag::AddStyleTagInput),
                "page.screenshot" => decodes!(screenshot::ScreenshotInput),
                "page.saveScreenshot" => decodes!(save_screenshot::SaveScreenshotInput),
                "page.pdf" => decodes!(pdf::PdfInput),
                "webStorage.items" | "webStorage.clear" => decodes!(web_storage::StorageInput),
                "webStorage.getItem" | "webStorage.removeItem" => {
                    decodes!(web_storage::StorageKeyInput);
                }
                "webStorage.setItem" => decodes!(web_storage::StorageSetInput),
                _ => {}
            }
        }
        Ok(())
    }
}
