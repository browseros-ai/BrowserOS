//! The page ops, their registration metadata, and the small ones' handlers.

use std::time::Duration;

use browseros_core::{TabId, TargetId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::Instant;
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
        EvalOnSelector,
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
    pub(crate) fn wire(self) -> OpWire {
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
                "run a function on one element in the main world",
            )
            .using(&[Capability::MainWorld])
            .policed(),
            // The agent's function on an element in the page's main world, as `locator.evaluate`:
            // a write the door polices like every main-world script, and that it rechecks before
            // the script's value or error is released.
            Self::EvalOnSelector => OpSpec::write(
                "page.evalOnSelector",
                TARGET,
                "run a function on the first match now in the main world ($eval)",
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
const REFUSED: (&str, &str) = (
    "refused",
    "a check upstream runs only after its command starts (its server's normalization of the \
     expression, or its client's argument checks after a locator's wait), which the prelude \
     decides because only it runs JavaScript; Rust raises it at that point with nothing sent",
);

/// How the evaluating ops carry upstream's argument.
const ARG: (&str, &str) = (
    "arg",
    "the argument in the page's serialized form as JSON text, which the main world parses itself \
     so the page's own globals cannot alter it, with Neo's element brands for its handles; \
     absent beside refused",
);

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
        PageOp::Content => r.op(wire::command("Frame.content"), document_op::content),
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
                .adds(&[REFUSED])
                .retypes(&[ARG])
                .result(
                    "the frames as the script left them ride beside its value: a main-world \
                     script can navigate or add frames, and the prelude's synchronous frames() \
                     reads them",
                ),
            |pw, scope, input| Box::pin(evaluate::evaluate(pw, scope, input)),
        ),
        PageOp::WorkerEvaluate => r.op(
            wire::command("Worker.evaluateExpression")
                .adds(&[
                    REFUSED,
                    (
                        "worker",
                        "the host-owned worker's id, standing for upstream's worker channel",
                    ),
                ])
                .retypes(&[ARG]),
            super::worker_evaluate::evaluate,
        ),
        PageOp::EvaluateOnElement => r.op(
            wire::command("JSHandle.evaluateExpression")
                .adds(&[REFUSED])
                .retypes(&[ARG]),
            |pw, scope, input| Box::pin(element_evaluate::evaluate_on_element(pw, scope, input)),
        ),
        PageOp::EvalOnSelector => r.op(
            wire::command("Frame.evalOnSelector")
                .adds(&[
                    REFUSED,
                    (
                        "selector",
                        "the selector as the script wrote it, which upstream's no-match error \
                         quotes; target carries it parsed, since Rust never parses selectors",
                    ),
                ])
                .retypes(&[ARG]),
            |pw, scope, input| Box::pin(element_evaluate::eval_on_selector(pw, scope, input)),
        ),
        PageOp::EvaluateOnAll => r.op(
            wire::command("Frame.evalOnSelectorAll")
                .adds(&[REFUSED])
                .retypes(&[ARG]),
            |pw, scope, input| Box::pin(element_evaluate::evaluate_on_all(pw, scope, input)),
        ),
        PageOp::WaitForFunction => r.op(
            wire::command("Frame.waitForFunction")
                .adds(&[REFUSED])
                .retypes(&[ARG])
                .result(
                    "Neo keeps no page objects, so the handle holds the value Rust read and \
                     upstream's preview of it, which jsonValue() and toString() answer",
                ),
            |pw, scope, input| Box::pin(evaluate::wait_for_function(pw, scope, input)),
        ),
        // upstream's `Locator.waitForFunction` sends `Frame.waitForFunction` with the locator's
        // selector and `strict`, Neo's `target` and `strict` here.
        PageOp::WaitForFunctionOnElement => r.op(
            wire::command("Frame.waitForFunction")
                .adds(&[REFUSED])
                .omits(&[(
                    "pollingInterval",
                    "upstream's Locator.waitForFunction takes no polling option; its client never \
                     sends one",
                )])
                .retypes(&[ARG]),
            |pw, scope, input| {
                Box::pin(element_evaluate::wait_for_function_on_element(
                    pw, scope, input,
                ))
            },
        ),
        PageOp::AddInitScript => r.op(
            wire::command("Page.addInitScript")
                .adds(&[(
                    "path",
                    "the prelude has no filesystem, so Rust reads the file as upstream's client \
                     does (UTF-8, //# sourceURL appended)",
                )])
                .retypes(&[("source", "absent when path names the file Rust reads")])
                .result(init_script::REGISTRATION),
            init_script::add_to_page,
        ),
        PageOp::ExposeBinding => r.op(
            wire::command("Page.exposeBinding").result(init_script::REGISTRATION),
            |pw, scope, input| Box::pin(init_script::expose_on_page(pw, scope, input)),
        ),
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
                .omits(&[("result", "carried in arg")]),
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
    document_op::read(pw, scope, input, DocumentPart::Title, false).await
}

/// How long a close that sets no timeout of its own waits for the browser to drop the tab.
/// Chromium drops a tab it was told to close within a second or so, unload handlers included.
const CLOSE_CONFIRMATION: Duration = Duration::from_secs(10);
/// How often a close lists the browser's tabs while it waits for its tab to go.
const CLOSE_POLL: Duration = Duration::from_millis(50);

/// The default close, as upstream's: the browser closes the tab with `Target.closeTarget`, and
/// the page is asked nothing, neither by an attach ([`OpScope::admit_target`]) nor through its
/// beforeunload. Closing is how an agent, a host or the idle-tab janitor gets rid of a tab, so a
/// page that answers nothing (its navigation never commits) or asks to stay ("Leave site?")
/// closes too. `page.runBeforeUnload` is the close that asks.
///
/// Core's own close does not serve here: BrowserOS's `Browser.closeTab` runs beforeunload, so a
/// page that prevents leaving keeps its tab open behind a dialog, and core forgets the page
/// whatever the browser then did (measured on BrowserOS, and on the published image with a
/// payment page). So the op answers closed only once the browser's own tab list no longer
/// shows the tab, and that listing is what makes core's registry forget the page, with the
/// hooks core's own close fires. A tab still listed when the op's timeout passes, or
/// [`CLOSE_CONFIRMATION`] without one, fails the op as `wait_timeout`: nothing was reported
/// closed, so the tab stays a candidate for whoever closes it next (the janitor's next sweep).
async fn close(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PageInput,
) -> Result<Nothing, PlaywrightError> {
    let page = input.page.tab();
    let admitted = scope.admit_target(pw.browser(), page).await?;
    let _sent = scope.send_effect()?;
    let closed = close_target(scope, page, admitted).await?;
    gone(scope, page).await?;
    pw.workers_closed(closed).await;
    scope.run_state().frames.forget(page);
    Ok(())
}

/// Sends `Target.closeTarget` for the tab's page target, through the run's view, whose gate
/// still decides. The target admission named can have been replaced since (Chromium swaps a
/// tab's contents), and the browser then knows no such target: the tab's current target, from
/// the browser's tab list, is closed instead. Without that, the miss would read as a page
/// already gone, which the prelude takes for a successful close. Returns the target closed.
async fn close_target(
    scope: &OpScope<'_>,
    tab: TabId,
    target: TargetId,
) -> Result<TargetId, PlaywrightError> {
    let send = |target: &TargetId| {
        scope.view().cdp(
            "Target.closeTarget",
            json!({ "targetId": target.as_str() }),
            None,
        )
    };
    match send(&target).await {
        Ok(_) => return Ok(target),
        Err(error) if ErrorCode::from_core(&error) == ErrorCode::TargetClosed => {}
        Err(error) => return Err(scope.core_error(&error)),
    }
    match listed_target(scope, tab).await? {
        Some(current) if current != target => {
            send(&current)
                .await
                .map_err(|error| scope.core_error(&error))?;
            Ok(current)
        }
        // Not listed, or listed under the very target the browser no longer knows (a tab whose
        // close is under way): never an error here, which the prelude would read as closed.
        // The wait for the listing decides.
        Some(_) | None => Ok(target),
    }
}

/// Waits until the browser's own tab list no longer shows `tab`; see [`close`] for the limit.
async fn gone(scope: &OpScope<'_>, tab: TabId) -> Result<(), PlaywrightError> {
    let limit = scope.timeout().unwrap_or(CLOSE_CONFIRMATION);
    let started = Instant::now();
    let mut logged = false;
    while listed_target(scope, tab).await?.is_some() {
        if !logged {
            scope.log("waiting for the browser to remove the tab from its tab list".to_owned());
            logged = true;
        }
        if started.elapsed() >= limit {
            return Err(scope.fail(
                ErrorCode::WaitTimeout,
                &format!("Timeout {}ms exceeded.", limit.as_millis()),
            ));
        }
        scope
            .sleep(CLOSE_POLL)
            .await
            .map_err(|_expired| scope.timed_out(ErrorCode::WaitTimeout))?;
    }
    Ok(())
}

/// The tab's page target in a fresh listing of the browser's tabs, `None` once it is not
/// listed. Core reconciles its registry from each such listing.
async fn listed_target(
    scope: &OpScope<'_>,
    tab: TabId,
) -> Result<Option<TargetId>, PlaywrightError> {
    let listing = scope
        .view()
        .list_pages()
        .await
        .map_err(|error| scope.core_error(&error))?;
    Ok(listing
        .pages
        .into_iter()
        .find(|listed| listed.tab_id == tab)
        .map(|listed| listed.target_id))
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
    // Target facts are the browser's own, so a page that answers nothing still has an opener.
    let target = scope.admit_target(pw.browser(), input.page.tab()).await?;
    let core = |error| scope.core_error(&error);
    let info: TargetInfoReply = pw
        .browser()
        .cdp(
            "Target.getTargetInfo",
            json!({ "targetId": target.as_str() }),
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

/// `page.close` and `page.opener` on a tab whose renderer answers nothing, as while its
/// navigation never commits or while a "Leave site?" dialog holds it: both are the browser's
/// business, so neither sends the page a command, and the close is upstream's
/// `Target.closeTarget`, confirmed by the browser's tab list.
#[cfg(test)]
mod browser_only_tests {
    use std::error::Error;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use serde_json::{Value, json};
    use tokio::time;

    use crate::test_env::EnvCall;
    use crate::test_support::{ProbeAnswer, SCRIPTED_MAIN, ScriptedRun, ScriptedTab, TabScript};
    use crate::{ErrorCode, Op, PlaywrightError};

    /// Far under the CDP client's 60 s, which a page command would wait for.
    const BROWSER_ONLY: Duration = Duration::from_secs(5);
    /// What core's attach and a write's admission send a page's renderer.
    const RENDERER: [&str; 4] = [
        "Page.enable",
        "DOM.enable",
        "Accessibility.enable",
        "Runtime.evaluate",
    ];

    /// A tab whose renderer outlasts the client on every command. The browser drops it from
    /// its tab list once told to close it, unless it `stays` listed.
    #[derive(Default)]
    struct Stuck {
        stays: bool,
        closed: AtomicBool,
    }

    impl TabScript for Stuck {
        fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
            ProbeAnswer::unexpected()
        }

        fn outlasts_client(&self, method: &str) -> bool {
            RENDERER.contains(&method)
        }

        fn cdp_reply(&self, method: &str, _params: &Value) -> Option<Value> {
            match method {
                "Target.getTargetInfo" => {
                    Some(json!({ "targetInfo": { "targetId": SCRIPTED_MAIN } }))
                }
                "Target.closeTarget" => {
                    self.closed.store(true, Ordering::SeqCst);
                    Some(json!({ "success": true }))
                }
                "Browser.getTabs" if self.closed.load(Ordering::SeqCst) && !self.stays => {
                    Some(json!({ "tabs": [] }))
                }
                _ => None,
            }
        }
    }

    async fn invoke(
        run: &ScriptedRun,
        name: &'static str,
        input: Value,
    ) -> Result<Result<Value, PlaywrightError>, Box<dyn Error>> {
        let op = Op::from_name(name).ok_or("an op")?;
        time::timeout(BROWSER_ONLY, run.playwright.invoke(&run.env, op, input))
            .await
            .map_err(|_elapsed| format!("{name} waited on the page").into())
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_close_and_opener_send_nothing_to_a_page_that_answers_nothing()
    -> Result<(), Box<dyn Error>> {
        let tab = ScriptedTab::new(Stuck::default());
        let run = ScriptedRun::over(&tab).await?;
        let input = json!({ "call": {}, "page": run.page });

        assert_eq!(
            invoke(&run, "page.opener", input.clone()).await??,
            json!({ "opener": null })
        );
        invoke(&run, "page.close", input).await??;

        for renderer in RENDERER {
            assert_eq!(tab.sent(renderer), 0, "the page was sent {renderer}");
        }
        assert_eq!(
            (tab.sent("Target.closeTarget"), tab.sent("Browser.closeTab")),
            (1, 0),
            "upstream's close, which runs no beforeunload"
        );
        assert!(
            run.env.calls().contains(&EnvCall::MarkSent),
            "the close was sent"
        );
        Ok(())
    }

    /// A tab whose contents Chromium swapped since admission: the browser knows the admitted
    /// target no more, which must not pass for a page already gone. The tab's current target,
    /// from the browser's tab list, is closed instead.
    #[derive(Default)]
    struct Swapped {
        /// The browser refused the admitted target: its list shows the current one from then on.
        refused: AtomicBool,
        closed: AtomicBool,
    }

    /// The target the tab shows after the swap.
    const CURRENT: &str = "swapped-in";

    impl TabScript for Swapped {
        fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
            ProbeAnswer::unexpected()
        }

        fn cdp_error(&self, method: &str, params: &Value) -> Option<&'static str> {
            let refused = method == "Target.closeTarget" && params["targetId"] == SCRIPTED_MAIN;
            if refused {
                self.refused.store(true, Ordering::SeqCst);
            }
            refused.then_some("No target with given id found")
        }

        fn cdp_reply(&self, method: &str, _params: &Value) -> Option<Value> {
            match method {
                "Target.closeTarget" => {
                    self.closed.store(true, Ordering::SeqCst);
                    Some(json!({ "success": true }))
                }
                "Browser.getTabs" if self.closed.load(Ordering::SeqCst) => {
                    Some(json!({ "tabs": [] }))
                }
                "Browser.getTabs" if self.refused.load(Ordering::SeqCst) => {
                    Some(json!({ "tabs": [{ "tabId": 7, "targetId": CURRENT,
                    "windowId": 1, "url": "https://a.test/", "title": "A", "isActive": true,
                    "isLoading": false, "loadProgress": 1.0, "isPinned": false,
                    "isHidden": false }] }))
                }
                _ => None,
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_close_after_a_swap_closes_the_tabs_current_target()
    -> Result<(), Box<dyn Error>> {
        let tab = ScriptedTab::new(Swapped::default());
        // Core lists the tab before the swap is visible, so admission names the old target.
        let run = ScriptedRun::over(&tab).await?;
        let input = json!({ "call": {}, "page": run.page });
        invoke(&run, "page.close", input).await??;
        assert_eq!(
            tab.sent("Target.closeTarget"),
            2,
            "the admitted target, which the browser no longer knows, then the current one"
        );
        assert!(
            tab.script().closed.load(Ordering::SeqCst),
            "{CURRENT} closed"
        );
        Ok(())
    }

    /// A tab the browser keeps listing is never answered closed: the close fails as a timeout
    /// once its own timeout passes, so whoever closes it can try again.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_tab_the_browser_keeps_listing_fails_the_close()
    -> Result<(), Box<dyn Error>> {
        let tab = ScriptedTab::new(Stuck {
            stays: true,
            ..Stuck::default()
        });
        let run = ScriptedRun::over(&tab).await?;
        let input = json!({ "call": { "timeoutMs": 300 }, "page": run.page });

        let error = invoke(&run, "page.close", input)
            .await?
            .err()
            .ok_or("a tab still listed is not closed")?;
        assert_eq!(error.error_code(), Some(ErrorCode::WaitTimeout), "{error}");
        assert!(
            error
                .message()
                .starts_with("page.close: Timeout 300ms exceeded."),
            "{error}"
        );
        assert!(
            error
                .message()
                .contains("waiting for the browser to remove the tab from its tab list"),
            "{error}"
        );
        assert_eq!(tab.sent("Target.closeTarget"), 1);
        Ok(())
    }
}
