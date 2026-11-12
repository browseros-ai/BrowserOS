//! The main-world gateway: `page.evaluate`, `page.waitForFunction`, the element forms
//! (`locator.evaluate`, `evaluateAll`, `elementHandle.evaluate`, `$eval`, `$$eval`), their
//! `frame.*` forms, `page.setContent`'s document write, whose HTML's scripts run in the main
//! world (`document_write`), and the answer to a binding call (`bindingCall.resolve`). They run as Playwright's do, and their results come back as the page
//! answered them. One of the files in core's `tests/page_js_guard.rs` ALLOWED list: the only
//! code that sends script to a page's main world, and so the gateway of `Capability::MainWorld`.
//!
//! ```text
//!  EvaluateGuard::open  an op whose spec does not declare MainWorld is refused with nothing
//!                       admitted, and a script the prelude refused is raised; then the page is
//!                       admitted and every element the script receives (its subject, its handle
//!                       arguments) is bound in its document, within the op's timeout
//!  EvaluateGuard::run   name the main-world document (a frame form's frame found gone is found
//!                       again by its id first) and resolve every bound element in the main
//!                       world, then OpScope::send_effect: the host's rules on the op's tab
//!                       ──▶ Sent, then Runtime.callFunctionOn(assets/main-world.js, userGesture)
//!                       ──▶ Outcome; once per op, or once per waitForFunction poll
//!  Answer::finish       the answer read in Playwright's format (page_value), or the thrown text
//!                       as a page_error
//! ```
//!
//! The host's rules (the owner's takeover lock, a CAPTCHA solve's hold) run on the op's own tab
//! before every send, the document's names first so a takeover while they resolve is seen. A
//! script also reaches its opener and its popups, through `window.opener` and the windows
//! `window.open` returns, as Playwright's does; no rule here checks those tabs.
//!
//! The script runs on the main-world document node, never on an execution context id: context
//! ids are per-renderer counters, and after a cross-process navigation a cached one can name a
//! context of another world (measured, see `probe_call`). The isolated world names its document by
//! handle, and `DOM.resolveNode` without a context id resolves that node in its main world. The
//! elements a script receives reach that world the same way, by backend node id, so they are
//! objects of the document's own context, which a call on the document takes as arguments.
//!
//! Elements are where upstream runs them. One out of the document (a handle's node the page
//! removed) still runs, as upstream's does. A handle argument is adopted only into its own
//! document: the script frame's, on the op's page. Upstream also adopts one from a same-origin
//! frame of the same process, through a context id Neo never names, so that case is a decided
//! deviation with upstream's adoption error.

use std::slice;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use browseros_cdp::CdpError;
use browseros_core::{CoreError, FrameId, ProtocolSession};
use futures_util::future;
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use ts_rs::TS;

use crate::op_scope::{OpScope, Sent};
use crate::op_spec::Capability;
use crate::page::OpPage;
use crate::page::document_watch::decode;
use crate::page::page_value::{self, PageValue, TOO_DEEP, Unreadable};
use crate::page::world_call::{self, with_world};
use crate::probe::{NAVIGATED_OR_CLOSED, ProbeFailure, Ran};
use crate::resolve::{self, HandleSite};
use crate::target::{ElementTarget, PageRef};
use crate::wire::{Nothing, options};
use crate::world::{BackendNodeId, Endpoint, FrameSeq, IsolatedWorld, Place, SeqLookup};
use crate::{ErrorCode, Playwright, PlaywrightError};

mod document_write;

pub(crate) use document_write::DocumentWrite;

/// The function every agent script runs in, built by the JS package's `scripts/build.ts` from
/// the vendored `UtilityScript` and its serializers plus `src/main-world/index.ts`: one function
/// expression, `(mode, isFunction, expression, argCount, ...argTexts)`. `evaluate` runs
/// upstream's `UtilityScript.evaluate` and returns the result's serialized text; `poller` builds
/// upstream's `waitForFunction` predicate, runs its first poll and returns it by reference. It
/// travels with every call, so nothing of Neo stays in the page. `eval` works in it even under a
/// page CSP that forbids it, because Chromium allows code generation inside a call from the
/// inspector (measured on BrowserOS with `script-src 'self'`).
pub(crate) const MAIN_WORLD_JS: &str = include_str!("../../assets/main-world.js");

/// [`MAIN_WORLD_JS`] as one document's evaluator: an object whose `run(target, args)` applies
/// the main-world function to `target`, made by `Runtime.evaluate` as upstream makes its
/// `UtilityScript` ([`Evaluator`]). Making it runs no page code: it reads no global, so a page's
/// getter cannot run before the host's recheck; the call reads what the function reads, after.
static EVALUATOR_JS: LazyLock<String> = LazyLock::new(|| {
    format!(
        "(() => {{ const main = ({MAIN_WORLD_JS}); \
         return {{ run(target, args) {{ return main.apply(target, args); }} }}; }})()"
    )
});

/// A main-world call through the document's evaluator, as upstream's
/// `(utilityScript, ...args) => utilityScript.evaluate(...args)`: the document is `this`, and
/// the evaluator's own `run` property is read, which no page prototype can shadow.
const THROUGH_EVALUATOR: &str =
    "function (evaluator, ...args) { return evaluator.run(this, args); }";

/// Chromium's refusal of a call whose argument lives in another context than its target: the
/// evaluator was made in the session's root frame and the document is a same-process child's.
const OTHER_WORLD: &str = "Argument should belong to the same JavaScript world as target object";

/// A poller's next poll: the predicate called again, judged in the page.
const POLL: &str = "function () { return this(); }";

/// What a poller's first poll answered, which its creating call kept.
const FIRST_POLL: &str = "function () { return this.first; }";

/// The value a poller's predicate settled with, by reference, for upstream's preview of it.
const SETTLED: &str = "function () { return this.last; }";

/// The source of the function upstream's server reads a `{fn}` back as, in its bundle
/// (`parseEvaluationResultValue`, PW:20413), which its refusal to serialize it quotes.
const BINDING_CALLER: &str =
    "(...args) => globalThis[kBindingsControllerProperty].callBinding(name, ...args)";

/// Names the document from the isolated world, by handle, so `DOM.describeNode` can name its node.
const ISOLATED_DOCUMENT: &str = "function () { return document; }";

/// upstream's message for a handle argument from another document (`kUnableToAdoptErrorMessage`).
pub(crate) const UNABLE_TO_ADOPT: &str = "Unable to adopt element handle from a different document";

/// What upstream says of a handle argument from another page (measured, spike S-adopt): its
/// session lookup finds no frame of the script's page for it.
const OTHER_PAGE: &str = "Frame has been detached.";

/// What upstream says of a handle argument whose document was replaced (measured, spike
/// S-detached): describing the node of a context that is gone.
const RETIRED_HANDLE: &str =
    "Protocol error (DOM.describeNode): Cannot find context with specified id";

/// What upstream says when the document went away under a running script, or before an
/// element's script ran (measured, spike S-detached: no period on either path on BrowserOS).
pub(crate) const DOCUMENT_REPLACED: &str =
    "Execution context was destroyed, most likely because of a navigation";

/// What upstream says of a script a navigation cut off while it was pending: its `rewriteError`
/// reading Chromium's [`NAVIGATED_OR_CLOSED`], with a period of its own (measured on BrowserOS
/// with Playwright 1.64.1, spike S-cut-off: `page.evaluate` and `locator.evaluate`, under a
/// same-site and a cross-site navigation). [`DOCUMENT_REPLACED`] is what it says when the
/// context goes first.
pub(crate) const NAVIGATED: &str =
    "Execution context was destroyed, most likely because of a navigation.";

/// What upstream's server says of a result outside Playwright's format, which only a page that
/// replaced the serializer's globals produces.
const OUTSIDE_FORMAT: &str = "the page answered with a value outside Playwright's serialized \
                              format, as a page that replaced the globals its serializer uses \
                              would, so Neo withheld it";

/// upstream's answer to a binding call (`PageBinding.dispatch`): the controller settles the
/// call's promise with the agent's result or error.
const DELIVER_BINDING_RESULT: &str =
    "arg => globalThis['__playwright__binding__controller__'].deliverBindingResult(arg)";

/// `bindingCall.resolve`: `{ page, context, arg }`, the answer to one call of a binding the run
/// exposed (`page/init_script.rs`). `arg` is `{ name, seq, result }` or `{ name, seq, error }` in
/// Playwright's in-page format, as the prelude serialized the agent's function's outcome.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResolveBindingInput {
    page: PageRef,
    /// The execution context `Runtime.bindingCalled` named, on the tab's page session.
    context: i64,
    arg: String,
}
options!(ResolveBindingInput);

/// Answers a binding call where it was made: upstream's `deliverBindingResult` in the execution
/// context that called, as upstream's `PageBinding.dispatch` evaluates it there, after the door's
/// rules on the tab. A context gone by now (its document was replaced) has nobody waiting, so
/// that answer is dropped, as upstream's is; the page reads nothing back.
pub(crate) async fn resolve_binding(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ResolveBindingInput,
) -> Result<Nothing, PlaywrightError> {
    scope.require(Capability::MainWorld)?;
    let admitted = scope.admit(pw.browser(), input.page.tab()).await?;
    let session = pw
        .browser()
        .protocol_session(admitted.session.session_id.clone());
    let sent = scope.send_effect()?;
    deliver_binding_result(&session, input.context, &input.arg, &sent).await;
    Ok(())
}

/// The main-world function's `evaluate` with [`DELIVER_BINDING_RESULT`] and `arg`, in `context`.
async fn deliver_binding_result(
    session: &ProtocolSession,
    context: i64,
    arg: &str,
    _sent: &Sent<'_>,
) {
    let _delivered = session
        .send_value(
            "Runtime.callFunctionOn",
            json!({
                "executionContextId": context,
                "functionDeclaration": MAIN_WORLD_JS,
                "arguments": [
                    { "value": "evaluate" },
                    { "value": true },
                    { "value": DELIVER_BINDING_RESULT },
                    { "value": 1 },
                    { "value": arg },
                ],
                "returnByValue": true,
                "awaitPromise": true,
            }),
        )
        .await;
}

/// The agent's script as the prelude built it (`wire.ts` `PageScript`). No `Debug`: the
/// argument may hold a password the agent was given, and no log line may carry it (STYLE P4).
/// `Clone`, for `locator.waitForFunction`, which sends it again on every try.
#[derive(Clone)]
pub(crate) enum Script {
    /// Ready to run: upstream's normalized expression (`normalizeEvaluationExpression`, run in
    /// the prelude), whether it is a function, and the argument.
    Run {
        expression: String,
        is_function: bool,
        arg: ArgText,
        /// The element handles the argument's `{h: i}` names, in order.
        handles: Vec<ElementTarget>,
    },
    /// The first check upstream fails at the script's point, such as "Passed function is not
    /// well-serializable!", or for `locator.evaluate` a client check upstream runs after the
    /// wait. [`EvaluateGuard::open`] raises it as `<api>: <message>`, with nothing admitted or
    /// sent.
    Refused(String),
}

/// What the agent's function is called with before its argument, and so where it runs.
pub(crate) enum Subject {
    /// `fn(arg)` in a frame of the page: the main frame when `None` (`page.evaluate`,
    /// `waitForFunction`), else the child frame a `frame.*` form names, by its id from
    /// `page.frames`.
    Frame(Option<FrameId>),
    /// `fn(element, arg)` in the element's own frame: `locator.evaluate` once its wait found it,
    /// `$eval`'s first match, `elementHandle.evaluate`. The element may be out of its document:
    /// it still runs, as upstream's does.
    Element(ElementTarget),
    /// `fn(elements, arg)`: `evaluateAll` and `$$eval`, in the frame the matches are in, or the
    /// query's starting frame when nothing matched (upstream's `queryArrayInMainWorld`, PW:21919,
    /// measured). `nodes` may be empty.
    Elements {
        frame: TreeFrame,
        /// The frame's document, which a handle argument must be in.
        seq: FrameSeq,
        nodes: Vec<BackendNodeId>,
    },
}

/// Playwright's in-page serialized argument as JSON text: opaque here, parsed by the page. As
/// text it stays one string through the host's decode and the CDP request, however deep it is.
#[derive(Clone)]
pub(crate) struct ArgText(pub(crate) String);

/// How often one op runs its script.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallMode {
    /// `page.evaluate`: once, its result by value.
    Once,
    /// One try of `locator.waitForFunction`: once, as `Once`, but the page answers only whether
    /// the result is truthy, judged as upstream's `waitForFunctionExpressionOnElement` callback
    /// judges it (a promise is truthy, never awaited).
    Truthy,
    /// `waitForFunction`: upstream's predicate, built once in the page with its argument parsed
    /// once, then called on every poll. The user gesture goes with the call that builds it only:
    /// upstream's later polls run from page timers.
    Poll,
}

/// One agent script, as a leaf hands it to the gateway.
pub(crate) struct MainWorldCall {
    script: Script,
    subject: Subject,
    mode: CallMode,
}

impl MainWorldCall {
    pub(crate) fn once(script: Script, subject: Subject) -> Self {
        Self {
            script,
            subject,
            mode: CallMode::Once,
        }
    }

    /// One try of `locator.waitForFunction` on the element its locator matched.
    pub(crate) fn truthy(script: Script, element: ElementTarget) -> Self {
        Self {
            script,
            subject: Subject::Element(element),
            mode: CallMode::Truthy,
        }
    }

    /// `waitForFunction`, on the main frame or the child frame `frame` names.
    pub(crate) fn poll(script: Script, frame: Option<FrameId>) -> Self {
        Self {
            script,
            subject: Subject::Frame(frame),
            mode: CallMode::Poll,
        }
    }
}

/// What one send produced.
pub(crate) enum Outcome {
    /// The script answered; [`Answer::finish`] turns it into the op's output.
    Answered(Answer),
    /// A poll whose predicate was falsy: keep waiting.
    Falsy,
    /// The document or the poller was gone before the call, so nothing ran.
    NotRun,
    /// The document was replaced while the script ran, by a navigation it started perhaps:
    /// Chromium destroyed the call's context under it, so it may have run.
    Destroyed,
    /// A navigation cut the call off while it was pending: Chromium fails a call into a
    /// document that left for another ([`NAVIGATED_OR_CLOSED`]), so it may have run. Upstream's
    /// `rewriteError` reads that failure as [`NAVIGATED`].
    Navigated,
}

/// An answer the page gave. No `Debug`, as for [`Script`]: it is page text, which no log line
/// carries.
pub(crate) enum Answer {
    /// `page.evaluate`'s result.
    Returned(Serialized),
    /// `waitForFunction`'s truthy value, and upstream's preview of it for `toString()`.
    Settled { value: Serialized, preview: String },
    /// What the page threw, as upstream's `getExceptionMessage` reads it, stack lines and all.
    Threw(String),
}

/// The page's serialized value as it answered it.
pub(crate) enum Serialized {
    /// Playwright's format, as JSON text.
    Text(String),
    /// No value: the script's `undefined`, which upstream's `jsonValue` reports as none.
    Undefined,
    /// Something `assets/main-world.js` never returns, from a page that tampered with it.
    Unreadable,
}

/// A finished script: what the leaves turn into their op's output.
pub(crate) struct Finished {
    /// The value in Playwright's format.
    pub(crate) value: Value,
    /// `waitForFunction`'s preview of it, upstream's `JSHandle.toString()`.
    pub(crate) preview: Option<String>,
}

/// An agent script bound in its frame's document, ready to send: the gateway's one way to the
/// main world. Only [`EvaluateGuard::open`] makes one, after the op's `MainWorld` declaration
/// was checked, and every [`EvaluateGuard::run`] runs the host's rules before its send.
#[must_use]
pub(crate) struct EvaluateGuard<'op> {
    page: OpPage<'op>,
    /// Where the script runs.
    frame: TreeFrame,
    /// The child frame a `frame.*` form names, by its id from `page.frames`. A navigation to
    /// another site moves that frame to another renderer, and so to another session, while it
    /// stays the same frame, so a run that finds `frame` gone finds it again by this id. `None`
    /// for the main frame, whose session outlives its navigations, and for an element's
    /// document, which its script cannot outlive.
    follow: Option<FrameId>,
    script: BoundScript,
}

/// An agent script, bound in its frame's document by `open`: what `run` sends, once or once per
/// poll.
struct BoundScript {
    expression: String,
    is_function: bool,
    arg: ArgText,
    mode: CallMode,
    /// What the function receives before its argument, by backend node id. Its elements and the
    /// handles are all of the guard's frame's document, which `open` checked, so one main world
    /// resolves them all for each run.
    subject: Bound,
    /// The argument's handles, by backend node id, in `{h: i}` order.
    handles: Vec<BackendNodeId>,
    /// The frame's main-world document by handle, in the op's object group. Dropped when a run
    /// finds it gone, so the next run names the new document's.
    document: Option<String>,
    /// `waitForFunction`'s predicate by handle, in the op's object group. Dropped with the
    /// document, so the next run builds it again in the new one, as upstream retries its wait
    /// across a navigation.
    poller: Option<String>,
    /// The document's evaluator. Dropped with the document, whose context it lives in.
    evaluator: Evaluator,
}

/// The main-world function compiled once for a document, by `Runtime.evaluate`, the way upstream
/// compiles its `UtilityScript`. Chromium compiles a `Runtime.callFunctionOn` declaration as an
/// inspector script it treats as cross-origin, and what such a script evaluates is muted too:
/// a callback the agent's script left behind (a timer, a promise) that throws later reaches the
/// page's `error` event and CDP's `Runtime.exceptionThrown` as a sanitized error, with the text
/// alone and no exception object, so `pageerror` lost the error's name and its value (measured on
/// BrowserOS; ER-004 through ER-008). Code `Runtime.evaluate` compiled is not muted, nor is
/// what it evaluates.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Evaluator {
    /// Not made for the document yet.
    Unmade,
    /// By handle, in the op's object group and the document's context.
    At(String),
    /// Unavailable for this document: `Runtime.evaluate` reaches only the session's root frame,
    /// so a same-process child frame's document ([`OTHER_WORLD`]), or the make failed. The
    /// main-world function then goes as each call's own declaration, muted as before.
    Absent,
}

impl BoundScript {
    /// Lets go of the document and the poller, so the next run names the new document's and
    /// builds the predicate there, as upstream's wait carries on in the frame's next context.
    fn leave_document(&mut self) {
        self.document = None;
        self.poller = None;
        self.evaluator = Evaluator::Unmade;
    }
}

impl<'op> EvaluateGuard<'op> {
    /// Opens the gateway: the declaration, the deferred refusal, the page, the binding. This is
    /// the only way to a guard, so it checks the op's declaration first: a host serves an op
    /// unheld unless its spec declares `MainWorld`, and such an op must not admit anything here.
    /// The page and the elements are found within the op's timeout.
    pub(crate) async fn open(
        pw: &'op Playwright,
        scope: &'op OpScope<'_>,
        page: &PageRef,
        call: MainWorldCall,
    ) -> Result<Self, PlaywrightError> {
        scope.require(Capability::MainWorld)?;
        let (expression, is_function, arg, handles) = match call.script {
            Script::Run {
                expression,
                is_function,
                arg,
                handles,
            } => (expression, is_function, arg, handles),
            Script::Refused(message) => return Err(scope.fail(ErrorCode::InvalidInput, &message)),
        };
        let found = async {
            let op_page = OpPage::admit_handle(pw, scope, page).await?;
            let binding = bind(&op_page, scope, page, call.subject, &handles).await?;
            Ok::<_, PlaywrightError>((op_page, binding))
        };
        let (op_page, binding) = match scope.within(found).await {
            Ok(found) => found?,
            Err(_expired) => return Err(scope.timed_out(ErrorCode::WaitTimeout)),
        };
        Ok(Self {
            page: op_page,
            frame: binding.frame,
            follow: binding.follow,
            script: BoundScript {
                expression,
                is_function,
                arg,
                mode: call.mode,
                subject: binding.subject,
                handles: binding.handles,
                document: None,
                poller: None,
                evaluator: Evaluator::Unmade,
            },
        })
    }

    /// Sends the script once: one poll for `waitForFunction`. It names the main-world document
    /// first (three CDP calls on a new document) and resolves the elements there, and only then
    /// re-runs the host's policies and marks the op sent, with no await before the script goes
    /// out (STYLE P6). Minted before the document, a takeover during those calls would go
    /// unseen, and a cancel there would leave the op marked sent although no script ran.
    pub(crate) async fn run(&mut self, scope: &OpScope<'_>) -> Result<Outcome, PlaywrightError> {
        // A poll after the first needs no elements, its predicate holds its argument. A document
        // or an element gone on the way means nothing ran.
        let Some(document) = self.document(scope).await? else {
            self.script.leave_document();
            return Ok(Outcome::NotRun);
        };
        let session = self.frame.endpoint.session.clone();
        let group = scope.object_group(&session).to_owned();
        let script = &mut self.script;
        // Beside the document's naming, before the host's recheck: making it runs no page code,
        // and nothing may await between the recheck and the script's send (STYLE P6).
        if script.evaluator == Evaluator::Unmade {
            script.evaluator = make_evaluator(&session, &group).await;
        }
        let objects = if script.mode == CallMode::Poll && script.poller.is_some() {
            Some(Vec::new())
        } else {
            main_world_nodes(&session, &group, &script.handles, &script.subject)
                .await
                .map_err(|error| scope.core_error(&error))?
        };
        let Some(objects) = objects else {
            script.leave_document();
            return Ok(Outcome::NotRun);
        };
        // At most twice: Chromium refuses an evaluator of another context before anything runs,
        // and the direct call then goes out after a recheck of its own (STYLE P6).
        let called = loop {
            let evaluator = match &script.evaluator {
                Evaluator::At(handle) => Some(handle.clone()),
                Evaluator::Unmade | Evaluator::Absent => None,
            };
            let sent = scope.send_effect()?;
            let call = MainCall {
                session: &session,
                group: &group,
                evaluator: evaluator.as_deref(),
                other_world: AtomicBool::new(false),
                _sent: &sent,
            };
            let called = match script.mode {
                CallMode::Once | CallMode::Truthy => {
                    call.evaluate(&document, script, &objects).await
                }
                CallMode::Poll => script.poll(&call, &document, &objects).await,
            };
            if !call.other_world.load(Ordering::Relaxed) {
                break called;
            }
            script.evaluator = Evaluator::Absent;
        };
        let outcome = called.map_err(|error| scope.core_error(&error))?;
        if matches!(
            outcome,
            Outcome::NotRun | Outcome::Destroyed | Outcome::Navigated
        ) {
            script.leave_document();
        }
        Ok(outcome)
    }

    /// The script frame's main-world document, named once per document. A frame form's frame
    /// whose world came back gone may only have moved to another renderer ([`Self::follow`]):
    /// it is found again by its id through the page's tree, as `open` found it, and named there.
    /// A frame the page no longer lists fails `Frame was detached`, as upstream's frame forms
    /// do. `None` when the world is still gone, as the main frame's is mid-navigation, so
    /// nothing runs.
    async fn document(&mut self, scope: &OpScope<'_>) -> Result<Option<String>, PlaywrightError> {
        if let Some(document) = &self.script.document {
            return Ok(Some(document.clone()));
        }
        let mut named = self.name_document(scope).await;
        if matches!(named, Err(ProbeFailure::Gone(_)))
            && let Some(frame) = self.follow.clone()
        {
            let found = world_call::frame_world(&self.page, scope, Some(&frame));
            let world = match scope.within(found).await {
                Ok(world) => world?,
                Err(_expired) => return Err(scope.timed_out(ErrorCode::WaitTimeout)),
            };
            self.frame = frame_endpoint(&self.page, world.session(), world.frame().clone());
            named = self.name_document(scope).await;
        }
        match named {
            Ok(named) => {
                self.script.document = Some(named.clone());
                Ok(Some(named))
            }
            Err(ProbeFailure::Gone(_)) => Ok(None),
            Err(failure) => Err(resolve::failed(scope, failure)),
        }
    }

    /// The main-world document of the guard's frame as it stands now, its world installed on
    /// first use and once more after a replaced document.
    async fn name_document(&self, scope: &OpScope<'_>) -> Result<String, ProbeFailure> {
        with_world(
            &self.page,
            &self.frame.endpoint,
            &self.frame.place,
            |world| main_world_document(scope, world),
        )
        .await
    }
}

impl Answer {
    /// The answer as the op returns it: a value in Playwright's format, or a throw as
    /// `page_error` with its stack lines, as upstream prints it. A value `PageValue::parse`
    /// cannot read fails: too deep is upstream's own failure, and any other shape is the page's.
    pub(crate) fn finish(self, scope: &OpScope<'_>) -> Result<Finished, PlaywrightError> {
        match self {
            Self::Returned(value) => Ok(Finished {
                value: returned(scope, parsed(scope, value)?)?,
                preview: None,
            }),
            // A wait hands back a handle, which upstream builds without serializing its value.
            Self::Settled { value, preview } => Ok(Finished {
                value: parsed(scope, value)?.into_json(),
                preview: Some(preview),
            }),
            Self::Threw(thrown) => Err(scope.fail(ErrorCode::PageError, &thrown)),
        }
    }
}

/// `serialized` in Playwright's format, or why it cannot leave.
fn parsed(scope: &OpScope<'_>, serialized: Serialized) -> Result<PageValue, PlaywrightError> {
    let value = match serialized {
        Serialized::Text(text) => PageValue::parse(&text),
        Serialized::Undefined => Ok(PageValue::undefined()),
        Serialized::Unreadable => Err(Unreadable::Shape),
    };
    value.map_err(|unreadable| match unreadable {
        Unreadable::TooDeep => scope.fail(ErrorCode::PageError, TOO_DEEP),
        Unreadable::Shape => scope.fail(ErrorCode::PageError, OUTSIDE_FORMAT),
    })
}

/// The same result boundary for worker scripts: enforce the depth and tagged-value
/// contract before the host records the value or the prelude revives it.
pub(crate) fn evaluation_result(
    scope: &OpScope<'_>,
    text: Option<String>,
) -> Result<Value, PlaywrightError> {
    returned(
        scope,
        parsed(scope, text.map_or(Serialized::Undefined, Serialized::Text))?,
    )
}

/// A result upstream's server cannot send back: one holding a `{fn}`, which it reads as a
/// binding's caller and then refuses to serialize, with the caller's own source in the message
/// (measured on BrowserOS neo, spike `__pw_fn_`).
fn returned(scope: &OpScope<'_>, value: PageValue) -> Result<Value, PlaywrightError> {
    match value.binding_position() {
        None => Ok(value.into_json()),
        Some(at) => Err(scope.fail(
            ErrorCode::PageError,
            &format!("Attempting to serialize unexpected value{at}: {BINDING_CALLER}"),
        )),
    }
}

/// A frame of the page as an op reaches it: the endpoint its world lives on, and the frame's
/// place in the page, which the world's install takes with it.
pub(crate) struct TreeFrame {
    pub(crate) endpoint: Endpoint,
    pub(crate) place: Place,
}

/// Where `frame` of `session`'s tree is reached: the page's main frame, whose first document
/// takes the page's first seq, through the main endpoint; any other as a frame found in a tree
/// rather than through a hop.
pub(crate) fn frame_endpoint(
    page: &OpPage<'_>,
    session: &ProtocolSession,
    frame: FrameId,
) -> TreeFrame {
    let main = page.main_endpoint();
    if frame == main.frame {
        TreeFrame {
            endpoint: main,
            place: Place::Main,
        }
    } else {
        TreeFrame {
            endpoint: page.child_endpoint(session.clone(), frame),
            place: Place::Descendant,
        }
    }
}

/// A remote object by handle.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Handle {
    #[serde(default)]
    object_id: Option<String>,
}

#[derive(Deserialize)]
struct Named {
    result: Handle,
}

#[derive(Deserialize)]
struct Described {
    node: DescribedNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescribedNode {
    backend_node_id: i64,
}

#[derive(Deserialize)]
struct Resolved {
    object: Handle,
}

/// The main-world document of `world`'s frame, by handle in the op's object group.
pub(super) async fn main_world_document(
    scope: &OpScope<'_>,
    world: IsolatedWorld,
) -> Result<String, ProbeFailure> {
    let session = world.session();
    let group = scope.object_group(session).to_owned();
    let named: Named = session
        .send(
            "Runtime.callFunctionOn",
            json!({ "objectId": world.global(), "functionDeclaration": ISOLATED_DOCUMENT,
                    "objectGroup": group }),
        )
        .await
        .map_err(ProbeFailure::from_core)?;
    let isolated = named.result.object_id.ok_or_else(|| {
        ProbeFailure::Broken("the isolated world returned no document".to_owned())
    })?;
    let described: Described = session
        .send("DOM.describeNode", json!({ "objectId": isolated }))
        .await
        .map_err(ProbeFailure::from_core)?;
    // No `executionContextId`: the node resolves in its own document's main world.
    let resolved: Resolved = session
        .send(
            "DOM.resolveNode",
            json!({ "backendNodeId": described.node.backend_node_id, "objectGroup": group }),
        )
        .await
        .map_err(ProbeFailure::from_core)?;
    resolved
        .object
        .object_id
        .ok_or_else(|| ProbeFailure::Broken("the main world returned no document".to_owned()))
}

/// The main-world calls of one send, all on the script frame's session and in the op's object
/// group. Each takes the send's [`Sent`]: none can go out without the host's recheck.
struct MainCall<'a> {
    session: &'a ProtocolSession,
    group: &'a str,
    /// The document's evaluator, when it has one.
    evaluator: Option<&'a str>,
    /// Set when the evaluator turned out to live in another context than the document: the
    /// call ran nothing, and `run` sends it again without the evaluator.
    other_world: AtomicBool,
    _sent: &'a Sent<'a>,
}

/// A main-world call's reply, or what became of the script when there was none.
enum Called<T> {
    Value(T),
    /// The call did not run to a value: nothing ran, the document was replaced, or the poller
    /// threw as it was built.
    Ended(Outcome),
}

/// What one poll answered.
enum Polled {
    /// The predicate was falsy.
    Waiting,
    /// It settled truthy: its value's serialized text.
    Settled(Serialized),
    Threw(String),
    /// Not what a poll answers, from a page that tampered with the poller.
    Unreadable,
}

/// How one main-world call returns, and whether it carries a user gesture.
#[derive(Clone, Copy)]
enum Shape {
    /// `page.evaluate`'s script: by value, with the user gesture upstream sends on every
    /// evaluation (PW:36960-36975).
    Evaluate,
    /// Building `waitForFunction`'s predicate, which runs its first poll: the poller by
    /// reference, with the gesture.
    BuildPoller,
    /// A later poll, or the first poll's answer: by value, with no gesture, as upstream's later
    /// polls run from page timers.
    Poll,
    /// The settled value by reference, for upstream's preview of it.
    Settled,
}

impl Shape {
    const fn by_value(self) -> bool {
        matches!(self, Self::Evaluate | Self::Poll)
    }

    const fn gesture(self) -> bool {
        matches!(self, Self::Evaluate | Self::BuildPoller)
    }
}

impl MainCall<'_> {
    /// `page.evaluate`'s one call: upstream's `UtilityScript.evaluate` on the main-world document,
    /// or for a `Truthy` script the truthiness of its result, as serialized text all the same.
    async fn evaluate(
        &self,
        document: &str,
        script: &BoundScript,
        objects: &[String],
    ) -> Result<Outcome, CoreError> {
        let called = self
            .main_world(
                document,
                arguments(
                    if script.mode == CallMode::Truthy {
                        "truthy"
                    } else {
                        "evaluate"
                    },
                    script,
                    objects,
                ),
                Shape::Evaluate,
            )
            .await?;
        let reply = match called {
            Called::Value(reply) => reply,
            Called::Ended(outcome) => return Ok(outcome),
        };
        Ok(Outcome::Answered(match reply.thrown() {
            Some(thrown) => Answer::Threw(thrown),
            None => Answer::Returned(reply.result.text()),
        }))
    }

    /// Builds upstream's predicate in the page and runs its first poll: the poller by reference,
    /// or how its first poll ended when it threw.
    async fn build_poller(
        &self,
        document: &str,
        script: &BoundScript,
        objects: &[String],
    ) -> Result<Called<String>, CoreError> {
        let called = self
            .main_world(
                document,
                arguments("poller", script, objects),
                Shape::BuildPoller,
            )
            .await?;
        let reply = match called {
            Called::Value(reply) => reply,
            Called::Ended(outcome) => return Ok(Called::Ended(outcome)),
        };
        if let Some(thrown) = reply.thrown() {
            return Ok(Called::Ended(Outcome::Answered(Answer::Threw(thrown))));
        }
        Ok(match reply.result.object_id {
            Some(poller) => Called::Value(poller),
            None => Called::Ended(Outcome::Answered(Answer::Returned(Serialized::Unreadable))),
        })
    }

    /// The main-world function on `document` with `arguments`: through the document's evaluator
    /// when it has one, else as the call's own declaration. Chromium refuses an evaluator of
    /// another context before anything runs: that sets [`Self::other_world`] and answers that
    /// nothing ran, and `EvaluateGuard::run` sends again the direct way after a fresh recheck.
    async fn main_world(
        &self,
        document: &str,
        arguments: Value,
        shape: Shape,
    ) -> Result<Called<Reply>, CoreError> {
        let Some(evaluator) = self.evaluator else {
            return self.send(document, MAIN_WORLD_JS, arguments, shape).await;
        };
        let through: Vec<Value> = [json!({ "objectId": evaluator })]
            .into_iter()
            .chain(arguments.as_array().cloned().unwrap_or_default())
            .collect();
        match self
            .send(document, THROUGH_EVALUATOR, Value::Array(through), shape)
            .await
        {
            Err(CoreError::Cdp(CdpError::Protocol { message, .. }))
                if message.contains(OTHER_WORLD) =>
            {
                self.other_world.store(true, Ordering::Relaxed);
                Ok(Called::Ended(Outcome::NotRun))
            }
            sent => sent,
        }
    }

    /// A poll's answer: `function` called on the poller, its result read by value.
    async fn poll(&self, poller: &str, function: &str) -> Result<Called<Polled>, CoreError> {
        let reply = match self.send(poller, function, json!([]), Shape::Poll).await? {
            Called::Value(reply) => reply,
            Called::Ended(outcome) => return Ok(Called::Ended(outcome)),
        };
        if let Some(thrown) = reply.thrown() {
            return Ok(Called::Value(Polled::Threw(thrown)));
        }
        Ok(Called::Value(match reply.result.value {
            Some(Value::Array(answer)) => match answer.as_slice() {
                [Value::Bool(false)] => Polled::Waiting,
                [Value::Bool(true), Value::String(text)] => {
                    Polled::Settled(Serialized::Text(text.clone()))
                }
                [Value::Bool(true), Value::Null] => Polled::Settled(Serialized::Undefined),
                _ => Polled::Unreadable,
            },
            _ => Polled::Unreadable,
        }))
    }

    /// The poller's settled value by reference: CDP's description of it is upstream's preview.
    async fn settled(&self, poller: &str) -> Result<Called<RemoteObject>, CoreError> {
        Ok(
            match self
                .send(poller, SETTLED, json!([]), Shape::Settled)
                .await?
            {
                Called::Value(reply) => Called::Value(reply.result),
                Called::Ended(outcome) => Called::Ended(outcome),
            },
        )
    }

    /// One `Runtime.callFunctionOn` into the main world, its reply decoded, or the outcome its
    /// protocol error stands for. Never logs its params or result: either may hold a password
    /// the agent passed or the page held (STYLE P4).
    async fn send(
        &self,
        object: &str,
        function: &str,
        arguments: Value,
        shape: Shape,
    ) -> Result<Called<Reply>, CoreError> {
        let sent = self
            .session
            .send_value(
                "Runtime.callFunctionOn",
                json!({
                    "objectId": object,
                    "functionDeclaration": function,
                    "arguments": arguments,
                    "returnByValue": shape.by_value(),
                    "awaitPromise": true,
                    "userGesture": shape.gesture(),
                    "objectGroup": self.group,
                }),
            )
            .await;
        match sent {
            Ok(reply) => Ok(Called::Value(decode::<Reply>(reply)?)),
            Err(error) => not_run(error),
        }
    }
}

impl BoundScript {
    /// One poll. The first builds upstream's predicate and runs it in the same call, so the
    /// agent's code runs right after the host's recheck; later polls call the predicate the page
    /// kept, each after a recheck of its own. A settled poll reads its value again by reference,
    /// for upstream's preview of it.
    async fn poll(
        &mut self,
        call: &MainCall<'_>,
        document: &str,
        objects: &[String],
    ) -> Result<Outcome, CoreError> {
        let polled = if let Some(poller) = &self.poller {
            call.poll(poller, POLL).await?
        } else {
            let poller = match call.build_poller(document, self, objects).await? {
                Called::Value(poller) => poller,
                Called::Ended(outcome) => return Ok(outcome),
            };
            let first = call.poll(&poller, FIRST_POLL).await?;
            self.poller = Some(poller);
            first
        };
        let (settled, poller) = match (polled, &self.poller) {
            (Called::Ended(outcome), _) => return Ok(outcome),
            (Called::Value(Polled::Waiting), _) => return Ok(Outcome::Falsy),
            (Called::Value(Polled::Threw(thrown)), _) => {
                return Ok(Outcome::Answered(Answer::Threw(thrown)));
            }
            (Called::Value(Polled::Settled(value)), Some(poller)) => (value, poller),
            (Called::Value(Polled::Unreadable | Polled::Settled(_)), _) => {
                return Ok(Outcome::Answered(Answer::Returned(Serialized::Unreadable)));
            }
        };
        Ok(match call.settled(poller).await? {
            Called::Value(object) => Outcome::Answered(Answer::Settled {
                value: object.json_value(settled),
                preview: object.preview(),
            }),
            Called::Ended(outcome) => outcome,
        })
    }
}

/// The main-world function's parameters for `mode`: `(mode, isFunction, expression, argCount,
/// ...argTexts, ...handles)`, as upstream's `UtilityScript.evaluate` takes them. The texts are
/// the subject's, when there is one, then the argument's; `objects` are the argument's handles,
/// then the subject's elements, so the argument's `{h: i}` from the prelude names its own and
/// the subject's text, written here, counts on from them: `{h: k}` for one element, an array of
/// them for several (Playwright's format, its id private to that text).
fn arguments(mode: &str, script: &BoundScript, objects: &[String]) -> Value {
    let first = script.handles.len();
    let subject = match &script.subject {
        Bound::Frame => None,
        Bound::Element(_) => Some(json!({ "h": first }).to_string()),
        Bound::Elements(nodes) => {
            let items: Vec<Value> = (first..first + nodes.len())
                .map(|index| json!({ "h": index }))
                .collect();
            Some(json!({ "a": items, "id": 1 }).to_string())
        }
    };
    let texts: Vec<&str> = subject
        .as_deref()
        .into_iter()
        .chain([script.arg.0.as_str()])
        .collect();
    let mut parameters = vec![
        json!({ "value": mode }),
        json!({ "value": script.is_function }),
        json!({ "value": script.expression }),
        json!({ "value": texts.len() }),
    ];
    parameters.extend(texts.into_iter().map(|text| json!({ "value": text })));
    parameters.extend(objects.iter().map(|object| json!({ "objectId": object })));
    Value::Array(parameters)
}

/// What a guard binds in `open`: where the script runs and the elements it receives.
struct Binding {
    frame: TreeFrame,
    /// The child frame a frame form named, which the guard follows ([`EvaluateGuard::follow`]).
    follow: Option<FrameId>,
    subject: Bound,
    handles: Vec<BackendNodeId>,
}

/// The subject once bound, by backend node id.
enum Bound {
    /// No element: the script's frame alone.
    Frame,
    Element(BackendNodeId),
    Elements(Vec<BackendNodeId>),
}

/// Where a bound subject runs, before its handle arguments are bound against it.
struct Placed {
    /// The frame the script runs in.
    frame: TreeFrame,
    /// The child frame a frame form named, by its id.
    follow: Option<FrameId>,
    /// That frame's document, which a handle argument must be of; `None` for the page's main
    /// frame when no handle needs it named.
    document: Option<FrameSeq>,
    /// The subject, by backend node id.
    subject: Bound,
}

/// Binds the subject and every handle argument in the script frame's document. Upstream's
/// errors, in its order: the subject first (its document gone is the navigation's message, as
/// upstream's evaluate on a handle of a replaced document; its node gone is Neo's detached
/// message), then each handle as upstream's adoption fails it.
async fn bind(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    page_ref: &PageRef,
    subject: Subject,
    handles: &[ElementTarget],
) -> Result<Binding, PlaywrightError> {
    let Placed {
        frame,
        follow,
        document,
        subject,
    } = place(page, scope, subject, !handles.is_empty()).await?;
    let mut bound = Vec::with_capacity(handles.len());
    for handle in handles {
        if handle.page() != page_ref {
            return Err(scope.fail(ErrorCode::InvalidInput, OTHER_PAGE));
        }
        let retired = |lookup| match lookup {
            SeqLookup::Retired => scope.fail(ErrorCode::ElementDetached, RETIRED_HANDLE),
            SeqLookup::Live { .. } | SeqLookup::Unknown => {
                scope.fail(ErrorCode::InvalidInput, UNABLE_TO_ADOPT)
            }
        };
        if let lookup @ (SeqLookup::Retired | SeqLookup::Unknown) = page.frames().by_seq(handle.seq)
        {
            return Err(retired(lookup));
        }
        // Found first: a world found gone on the way retires its seq, a replaced document.
        if HandleSite::find(page, scope, handle).await?.is_none() {
            return Err(retired(page.frames().by_seq(handle.seq)));
        }
        if document != Some(handle.seq) {
            return Err(scope.fail(ErrorCode::InvalidInput, UNABLE_TO_ADOPT));
        }
        bound.push(handle.node);
    }
    Ok(Binding {
        frame,
        follow,
        subject,
        handles: bound,
    })
}

/// The subject's frame and document, found again: a handle's document gone is the navigation's
/// message, its node gone Neo's detached one.
async fn place(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    subject: Subject,
    with_handles: bool,
) -> Result<Placed, PlaywrightError> {
    Ok(match subject {
        Subject::Frame(frame) => {
            let main = page.main_endpoint();
            let follow = frame.filter(|frame| *frame != main.frame);
            let child = follow.is_some();
            // A child frame is entered as `frame.locator` enters it, which installs its world, or
            // fails as upstream's frame forms do once it is detached. The main frame's world is
            // installed only for a handle argument to be checked against its document.
            let world = if child || with_handles {
                Some(world_call::frame_world(page, scope, follow.as_ref()).await?)
            } else {
                None
            };
            let frame = world.as_ref().map_or(
                TreeFrame {
                    endpoint: main,
                    place: Place::Main,
                },
                |world| frame_endpoint(page, world.session(), world.frame().clone()),
            );
            Placed {
                frame,
                follow,
                document: world.as_ref().map(IsolatedWorld::seq),
                subject: Bound::Frame,
            }
        }
        Subject::Element(element) => {
            if matches!(page.frames().by_seq(element.seq), SeqLookup::Retired) {
                return Err(scope.fail(ErrorCode::PageError, DOCUMENT_REPLACED));
            }
            let Some(site) = HandleSite::find(page, scope, &element).await? else {
                // A world found gone on the way retires its seq: the document was replaced.
                return Err(match page.frames().by_seq(element.seq) {
                    SeqLookup::Retired => scope.fail(ErrorCode::PageError, DOCUMENT_REPLACED),
                    SeqLookup::Live { .. } | SeqLookup::Unknown => resolve::handle_detached(scope),
                });
            };
            let world = site.world();
            Placed {
                frame: frame_endpoint(page, world.session(), world.frame().clone()),
                follow: None,
                document: Some(element.seq),
                subject: Bound::Element(element.node),
            }
        }
        Subject::Elements { frame, seq, nodes } => Placed {
            frame,
            follow: None,
            document: Some(seq),
            subject: Bound::Elements(nodes),
        },
    })
}

/// The argument's handles, then the subject's elements, resolved in the main world of their
/// document on `session` by backend node id, in the op's object group; `None` when one is gone,
/// so nothing runs.
async fn main_world_nodes(
    session: &ProtocolSession,
    group: &str,
    handles: &[BackendNodeId],
    subject: &Bound,
) -> Result<Option<Vec<String>>, CoreError> {
    let subject: &[BackendNodeId] = match subject {
        Bound::Frame => &[],
        Bound::Element(node) => slice::from_ref(node),
        Bound::Elements(nodes) => nodes,
    };
    let resolves = handles.iter().chain(subject).map(|node| {
        session.send::<_, Resolved>(
            "DOM.resolveNode",
            json!({ "backendNodeId": node, "objectGroup": group }),
        )
    });
    let mut objects = Vec::with_capacity(handles.len() + subject.len());
    for resolved in future::join_all(resolves).await {
        match resolved {
            Ok(Resolved {
                object: Handle {
                    object_id: Some(object),
                },
            }) => objects.push(object),
            // No object, or a node the backend id no longer names, or a document gone: nothing
            // ran.
            Ok(_) | Err(CoreError::Cdp(CdpError::Protocol { .. })) => return Ok(None),
            Err(error)
                if matches!(
                    ProbeFailure::from_core(error.clone()),
                    ProbeFailure::Gone(_)
                ) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(Some(objects))
}

/// A protocol error that is an outcome of the script rather than a failure, as upstream's
/// `rewriteError` reads them; any other error is the op's failure. The call's world gone is
/// read as every probe call's is ([`Ran::when_gone`]). A call Chromium cut off while it was
/// pending may have run: its context destroyed under it (`Destroyed`), or its document left for
/// another (`Navigated`). `page.evaluate` never sends it again and fails as upstream's does,
/// and a poll goes on in the next document, as upstream's wait does, its predicate being meant
/// to be free of side effects. A call whose context, session or frame was gone before it
/// reached the page ran nothing, as one on an object the page no longer holds.
fn not_run<T>(error: CoreError) -> Result<Called<T>, CoreError> {
    const OBJECT_GONE: &str = "Could not find object with given id";
    let message = match &error {
        CoreError::Cdp(CdpError::Protocol { message, .. }) => message.as_str(),
        _ => "",
    };
    Ok(Called::Ended(match Ran::when_gone(&error) {
        Some(Ran::Maybe) if message.contains(NAVIGATED_OR_CLOSED) => Outcome::Navigated,
        Some(Ran::Maybe) => Outcome::Destroyed,
        Some(Ran::No) => Outcome::NotRun,
        None if message.contains(OBJECT_GONE) => Outcome::NotRun,
        None => return Err(error),
    }))
}

/// Makes the document's [`Evaluator`] in the session's default context, the main world of the
/// session's root frame. A same-process child frame's document lives in another context, which
/// the first call through it finds ([`MainCall::main_world`]).
async fn make_evaluator(session: &ProtocolSession, group: &str) -> Evaluator {
    let made = session
        .send_value(
            "Runtime.evaluate",
            json!({ "expression": &*EVALUATOR_JS, "objectGroup": group, "silent": true }),
        )
        .await;
    match made
        .ok()
        .and_then(|reply| decode::<Named>(reply).ok())
        .and_then(|named| named.result.object_id)
    {
        Some(handle) => Evaluator::At(handle),
        None => Evaluator::Absent,
    }
}

/// `Runtime.callFunctionOn`'s reply.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reply {
    result: RemoteObject,
    #[serde(default)]
    exception_details: Option<ExceptionDetails>,
}

impl Reply {
    /// What the page threw, as upstream's `getExceptionMessage` reads it: the exception's
    /// description, stack lines and all, or a thrown primitive as text.
    fn thrown(&self) -> Option<String> {
        let details = self.exception_details.as_ref()?;
        Some(match &details.exception {
            Some(exception) => exception
                .description
                .clone()
                .unwrap_or_else(|| js_string(exception.value.as_ref())),
            None => details.text.clone(),
        })
    }
}

/// A remote object, by value or by reference. `null` arrives as a `value` of `null`, and
/// `undefined` as no value at all.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteObject {
    r#type: String,
    #[serde(default)]
    subtype: Option<String>,
    #[serde(default, deserialize_with = "present")]
    value: Option<Value>,
    #[serde(default)]
    unserializable_value: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    object_id: Option<String>,
}

#[derive(Deserialize)]
struct ExceptionDetails {
    text: String,
    #[serde(default)]
    exception: Option<RemoteObject>,
}

impl RemoteObject {
    /// `assets/main-world.js`'s evaluate answer: the serialized text, or no value for
    /// `undefined`.
    fn text(self) -> Serialized {
        match (self.r#type.as_str(), self.value) {
            ("string", Some(Value::String(text))) => Serialized::Text(text),
            ("undefined", None) => Serialized::Undefined,
            _ => Serialized::Unreadable,
        }
    }

    /// The value upstream's `JSHandle.jsonValue()` gives for this settled value: an object's is
    /// the page's serialized `text`, and a primitive's is CDP's own, where a `BigInt` is lost
    /// (`parseUnserializableValue`, PW:20628), so it reads `undefined`.
    fn json_value(&self, text: Serialized) -> Serialized {
        if self.object_id.is_none() && self.r#type == "bigint" {
            Serialized::Undefined
        } else {
            text
        }
    }

    /// upstream's preview of this value, its `JSHandle.toString()` (PW:20753, `renderPreview`
    /// PW:36897, no `generatePreview`): `String(value)` for a primitive, CDP's description of an
    /// object, else `JSHandle@<subtype or type>`.
    fn preview(self) -> String {
        if self.object_id.is_none() {
            return match self.unserializable_value.as_deref() {
                // `parseUnserializableValue` knows these four and nothing else.
                Some("-0") => "0".to_owned(),
                Some(special @ ("NaN" | "Infinity" | "-Infinity")) => special.to_owned(),
                Some(_) => "undefined".to_owned(),
                None => js_string(self.value.as_ref()),
            };
        }
        self.description
            .filter(|description| !description.is_empty())
            .unwrap_or_else(|| format!("JSHandle@{}", self.subtype.unwrap_or(self.r#type)))
    }
}

/// `String(value)` for a primitive CDP reported by value; `undefined` for none.
fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_owned(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number
            .as_f64()
            .map_or_else(|| number.to_string(), page_value::js_number),
        Some(other) => other.to_string(),
    }
}

/// Decodes a field that is present, `null` included, as `Some`: absent stays `None` through
/// `#[serde(default)]`. A plain `Option<Value>` would read an explicit `null` as absent.
pub(crate) fn present<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests;

/// Runs tag insertion against a document handle resolved in its own frame's main world.
/// The operation owns every temporary object, and the host recheck's `Sent` gates the write.
/// A final browser round trip orders asynchronously reported inline CSP errors before success.
pub(super) async fn insert_tag(
    scope: &OpScope<'_>,
    world: &IsolatedWorld,
    document: &str,
    params: Value,
    _sent: &Sent<'_>,
) -> Result<String, PlaywrightError> {
    let session = world.session();
    let reply: Reply = session
        .send(
            "Runtime.callFunctionOn",
            json!({
                "objectId": document,
                "functionDeclaration": include_str!("add_tag.js"),
                "arguments": [{ "value": params }],
                "awaitPromise": true,
                "userGesture": true,
                // A tag can remove itself while its script executes. Keep the returned node
                // alive until the run ends so its ElementHandle survives browser GC.
                "objectGroup": scope.run_state().objects.on(session),
            }),
        )
        .await
        .map_err(|error| scope.core_error(&error))?;
    session
        .send_value(
            "Runtime.callFunctionOn",
            json!({
                "objectId": document, "functionDeclaration": "function () { return true; }",
                "returnByValue": true,
            }),
        )
        .await
        .map_err(|error| scope.core_error(&error))?;
    if let Some(thrown) = reply.thrown() {
        return Err(scope.fail(ErrorCode::PageError, &thrown));
    }
    reply
        .result
        .object_id
        .ok_or_else(|| scope.fail(ErrorCode::Internal, "The inserted tag returned no element"))
}
