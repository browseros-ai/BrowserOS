//! The main-world gateway: `page.evaluate`, `page.waitForFunction`, the element forms
//! (`locator.evaluate`, `evaluateAll`, `elementHandle.evaluate`, `$eval`, `$$eval`), their
//! `frame.*` forms, and `page.setContent`'s document write, whose HTML's scripts run in the main
//! world (`document_write`). They run as Playwright's do, and their results come back as the page
//! answered them. One of the three files in core's `tests/page_js_guard.rs` ALLOWED list: the only
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
//! context of another world (measured, see `probe_call`). The sealed world names its document by
//! handle, and `DOM.resolveNode` without a context id resolves that node in its main world. The
//! elements a script receives reach that world the same way, by backend node id, so they are
//! objects of the document's own context, which a call on the document takes as arguments.
//!
//! Elements are where upstream runs them. One out of the document (a handle's node the page
//! removed) still runs, as upstream's does. A handle argument is adopted only into its own
//! document: the script frame's, on the op's page. Upstream also adopts one from a same-origin
//! frame of the same process, through a context id Neo never names, so that case is a decided
//! deviation with upstream's adoption error.
//!
//! The site sweep at the end of this file (`sweep_site` and what it calls) is not the scripts':
//! read masking (`site_sweep`) sweeps a read's site with it, and the journal sweeps a reported
//! world with `sweep_reported`. No main-world send here sweeps or masks anything.

use std::future::Future;
use std::time::Duration;
use std::{mem, slice};

use browseros_cdp::CdpError;
use browseros_core::sensitive_fields::FieldValues;
use browseros_core::{CoreError, FrameId, ProtocolSession};
use futures_util::future;
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use tokio::time::{self, Instant};

use crate::op_scope::{OpScope, Sent};
use crate::op_spec::Capability;
use crate::page::OpPage;
use crate::page::document_watch::{FrameFields, decode};
use crate::page::frame_tree::frame_tree;
use crate::page::op_page::TryCut;
use crate::page::page_value::{self, PageValue, TOO_DEEP, Unreadable};
use crate::page::sweep_scope::{self, LinkedTab, Site};
use crate::page::world_call::{self, unexpected, with_world};
use crate::probe::{self, NAVIGATED_OR_CLOSED, ProbeFailure, ProbeReply, ProbeRequest, Ran};
use crate::resolve::{self, HandleSite};
use crate::target::{ElementTarget, PageRef};
use crate::world::{BackendNodeId, Endpoint, FrameSeq, Place, Scan, SealedWorld, SeqLookup};
use crate::{ErrorCode, Playwright, PlaywrightError};

mod document_write;

pub(crate) use document_write::DocumentWrite;

/// How long one frame may take to be swept, from the start of the work that reaches it to the
/// end of its sweep: attaching its session, reading its session's frames, a second read after a
/// lost session, installing its world on first use, and the sweep, all under one deadline
/// (decisions.md, "Main-world sweep scope"). A healthy frame takes a few milliseconds. A frame
/// of a read's site that misses it fails that try closed, and the whole sweep goes again on
/// fresh budgets while the op's window has room (`LateRetry`), each budget ending at the
/// window's end at the latest (`TryCut`); late on the last try, it fails the read closed.
pub(crate) const FRAME_BUDGET: Duration = Duration::from_secs(1);

/// The function every agent script runs in, built by the JS package's `scripts/build.ts` from
/// the vendored `UtilityScript` and its serializers plus `src/main-world/index.ts`: one function
/// expression, `(mode, isFunction, expression, argCount, ...argTexts)`. `evaluate` runs
/// upstream's `UtilityScript.evaluate` and returns the result's serialized text; `poller` builds
/// upstream's `waitForFunction` predicate, runs its first poll and returns it by reference. It
/// travels with every call, so nothing of Neo stays in the page. `eval` works in it even under a
/// page CSP that forbids it, because Chromium allows code generation inside a call from the
/// inspector (measured on BrowserOS with `script-src 'self'`).
const MAIN_WORLD_JS: &str = include_str!("../../assets/main-world.js");

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

/// Names the document from the sealed world, by handle, so `DOM.describeNode` can name its node.
const SEALED_DOCUMENT: &str = "function () { return document; }";

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

/// The agent's script as the prelude built it (`wire.ts` `PageScript`). No `Debug`: the
/// argument may hold a password the agent was given, and no log line may carry it (STYLE P4).
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
pub(crate) struct ArgText(pub(crate) String);

/// How often one op runs its script.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallMode {
    /// `page.evaluate`: once, its result by value.
    Once,
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
}

impl BoundScript {
    /// Lets go of the document and the poller, so the next run names the new document's and
    /// builds the predicate there, as upstream's wait carries on in the frame's next context.
    fn leave_document(&mut self) {
        self.document = None;
        self.poller = None;
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
        let sent = scope.send_effect()?;
        let call = MainCall {
            session: &session,
            group: &group,
            _sent: &sent,
        };
        let called = match script.mode {
            CallMode::Once => call.evaluate(&document, script, &objects).await,
            CallMode::Poll => script.poll(&call, &document, &objects).await,
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

/// One world's sweep: the raw value of every sensitive field in its document, those inside its
/// closed shadow roots included. The seal finds only the closed roots it was handed, and the page
/// can attach one at any time without a trace in the page, so each sweep first scans the
/// document and has the seal watch every root it does not watch yet (`world::hidden_roots`).
/// The scan shares the frame's budget, so a frame whose roots cannot be watched in time is
/// `Unswept::Late` like any other late frame, and the op is refused.
async fn sweep_world(world: SealedWorld) -> Result<FieldValues, ProbeFailure> {
    sweep_watched(&world, Scan::Document).await
}

/// The sweep a journal report of `world` starts (`HIDDEN_ROOT_REPORT`): the seal watches the
/// roots of the hosts it reported, and the world is swept while their fields still exist, so
/// the host learns a value typed into a closed root the page removes before any op sweeps.
pub(crate) async fn sweep_reported(world: &SealedWorld) -> Result<FieldValues, ProbeFailure> {
    sweep_watched(world, Scan::Reported).await
}

/// The watch's hold lives until the sweep has read, so a root whose host the page removed is
/// still alive for it.
async fn sweep_watched(world: &SealedWorld, scan: Scan) -> Result<FieldValues, ProbeFailure> {
    let _held = world.watch_hidden_roots(scan).await?;
    match probe::call(world, &ProbeRequest::Sweep).await? {
        ProbeReply::Sweep { values } => Ok(values),
        other => Err(unexpected("sweep", other)),
    }
}

/// Why a sweep left a frame of the read's site unswept.
pub(crate) enum Unswept {
    /// It missed its [`FRAME_BUDGET`] on the last try the op's window had room for, or that try
    /// reached the window's end first, as an unresponsive renderer's calls do.
    Late,
    /// It came back gone on a second read of its target too (a lost session, a session of
    /// another target, a missing frame), while the browser still lists that target.
    Lost,
    /// The frame the sweep was taken for came back gone, on a fresh world too: its document,
    /// its frame or its session went away, as mid-navigation. A read there lands nowhere and
    /// polls again, as its own call coming back gone would (`SiteSweep::values_or_gone`).
    Gone,
    /// It failed for another reason.
    Failed(ProbeFailure),
}

/// What a sweep found so far, and the first frame of the read's site it could not sweep.
#[derive(Default)]
pub(crate) struct Found {
    pub(crate) values: FieldValues,
    pub(crate) unswept: Option<Unswept>,
}

impl Found {
    fn add(&mut self, values: FieldValues) {
        self.values = mem::take(&mut self.values).union(values);
    }

    fn fail(&mut self, unswept: Unswept) {
        self.unswept = self.unswept.take().or(Some(unswept));
    }

    fn merge(&mut self, other: Self) {
        self.values = mem::take(&mut self.values).union(other.values);
        self.unswept = self.unswept.take().or(other.unswept);
    }
}

/// Every frame in the site of the read's frame at `script`, of the page and of every tab its
/// opener links connect it to, swept again while a frame missed its budget and the op's window
/// has room for another try as long as the last (`OpPage::late_retry`). Each try is a whole
/// sweep on fresh budgets, its frames, targets and links read anew, so a frame that left
/// meanwhile is not waited for. A try after the first is cut at the window's end
/// (`LateRetry::cut`): a try can outlast the one before it, and run past the op's deadline,
/// where the op's own timeout would replace Neo's refusal. Every try's values are kept, a cut
/// one's too, a linked tab's among them, and a late try's go to the host before the next
/// starts. A frame still late when the window closes leaves the sweep `Unswept::Late`, as one
/// late try did before.
pub(crate) async fn sweep_site(page: &OpPage<'_>, script: &Endpoint, place: &Place) -> Found {
    let retry = page.late_retry();
    let mut found = Found::default();
    let mut cut = TryCut::default();
    loop {
        let started = Instant::now();
        let tried = sweep_site_once(page, script, place, cut).await;
        found.add(tried.values);
        match tried.unswept {
            Some(Unswept::Late) if retry.has_room(started.elapsed()) => {
                cut = retry.cut();
                if !found.values.is_empty() {
                    page.playwright()
                        .hooks()
                        .values_swept(page.tab(), &found.values);
                }
            }
            unswept => {
                found.unswept = unswept;
                return found;
            }
        }
    }
}

/// One try of [`sweep_site`], every budget in it ending at `cut` at the latest, a linked tab's
/// too.
///
/// The read's frame is swept while its session's frames and the browser's list of targets are
/// read; then the session's other frames, every other target of the page that may be of the
/// site, and each linked tab's such targets are swept, all at once. The targets come from
/// the browser, so one of another site, or of a tab no link connects, is never asked anything,
/// hung or not (`sweep_scope`). The list is read on every sweep: no renderer-side count finds
/// every out-of-process frame, and links change as tabs open and close (decisions.md,
/// "Main-world sweep scope"). A page with no opener link reaches its own targets only, from
/// the same reply. The list's read
/// has no frame budget, being the browser's, but `cut` ends it too: a try cut before the
/// browser listed the targets is late.
async fn sweep_site_once(
    page: &OpPage<'_>,
    script: &Endpoint,
    place: &Place,
    cut: TryCut,
) -> Found {
    // The script's session is read once for all of its frames, so they share one deadline.
    let deadline = cut.budget();
    let (own, tree, reach) = future::join3(
        sweep_frame(page, script.clone(), place.clone(), deadline),
        read_within_budget(deadline, frame_tree(&script.session)),
        cut.before(sweep_scope::reach(
            page.browser(),
            &page.main_endpoint().frame,
        )),
    )
    .await;
    let mut found = Found::default();
    match own {
        Ok(Some(values)) => found.add(values),
        Ok(None) => found.fail(Unswept::Gone),
        Err(unswept) => found.fail(unswept),
    }
    let tree = match tree {
        Ok(Some(tree)) => tree.frame_tree,
        Ok(None) => {
            found.fail(Unswept::Gone);
            return found;
        }
        Err(unswept) => {
            found.fail(unswept);
            return found;
        }
    };
    let reach = match reach {
        Some(Ok(reach)) => reach,
        Some(Err(error)) => {
            found.fail(Unswept::Failed(ProbeFailure::Cdp(error)));
            return found;
        }
        None => {
            found.fail(Unswept::Late);
            return found;
        }
    };
    let site = Site::of(&tree.frame.url, tree.frame.domain_and_registry.as_deref());
    // A session's root frame is its target: the page's main frame, or the OOPIF.
    let own_target = FrameId(tree.frame.id.clone());
    let own_frames = TargetFrames {
        session: script.session.clone(),
        frames: tree.into_frames().collect(),
    };
    let in_targets = reach
        .own
        .into_iter()
        .filter(|target| target.frame != own_target && site.may_hold(&target.url))
        .map(|target| async move {
            let deadline = cut.budget();
            sweep_target(page, &target.frame, None, None, deadline).await
        });
    let in_tabs = reach
        .linked
        .into_iter()
        .map(|tab| sweep_linked(page, tab, &site, cut));
    let (own, targets, tabs) = future::join3(
        sweep_target(
            page,
            &own_target,
            Some(own_frames),
            Some(&script.frame),
            deadline,
        ),
        future::join_all(in_targets),
        future::join_all(in_tabs),
    )
    .await;
    found.merge(own);
    for swept in targets.into_iter().chain(tabs) {
        found.merge(swept);
    }
    found
}

/// Every target of `tab` that may be of `site`, in a tab opener links connect to the page. A
/// tab with none is never asked anything. Otherwise the tab is found in core, its page state is
/// set up, and its targets are swept as the page's are, all by the tab's one deadline from when
/// finding it starts, a frame budget that ends at the try's `cut` at the latest, so the group's
/// sweeps are the try's and never outlast it. A tab core cannot reach is skipped only once the
/// browser no longer lists it.
async fn sweep_linked(page: &OpPage<'_>, tab: LinkedTab, site: &Site, cut: TryCut) -> Found {
    let mut found = Found::default();
    let targets: Vec<_> = tab
        .targets
        .into_iter()
        .filter(|target| site.may_hold(&target.url))
        .collect();
    if targets.is_empty() {
        return found;
    }
    let deadline = cut.budget();
    let missing = match read_within_budget(deadline, page.linked_tab(&tab.main)).await {
        Ok(Some(Some(reached))) => {
            // A tab's first page state waits on browser-side reads (`PageRegistry::page`), so
            // the tab's deadline ends that wait as late too. A cut setup leaves no state behind, and the try's other
            // sweeps keep what they found.
            let Ok(linked) = time::timeout_at(deadline, page.linked(&reached)).await else {
                found.fail(Unswept::Late);
                return found;
            };
            let sweeps = targets
                .iter()
                .map(|target| sweep_target(&linked, &target.frame, None, None, deadline));
            for swept in future::join_all(sweeps).await {
                found.merge(swept);
            }
            return found;
        }
        Err(Unswept::Late) => {
            found.fail(Unswept::Late);
            return found;
        }
        Ok(Some(None) | None) => Unswept::Lost,
        Err(unswept) => unswept,
    };
    let listed = read_within_budget(deadline, sweep_scope::is_listed(page.browser(), &tab.main));
    match listed.await {
        Ok(Some(false)) => {}
        Ok(Some(true) | None) => found.fail(missing),
        Err(unswept) => found.fail(unswept),
    }
    found
}

/// One target's frames as one read found them: the session that reaches them, and every
/// frame its tree lists, the target's root first.
struct TargetFrames {
    session: ProtocolSession,
    frames: Vec<FrameFields>,
}

/// How one attempt at a target's frames ended.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Attempt {
    /// Every frame its tree listed was swept.
    Swept,
    /// Its session was gone or answered for another target, or a frame came back gone. None
    /// of that shows the frame left the page, so one of the target's frames may be unswept.
    Lost,
    /// A read or a sweep failed otherwise, recorded in the [`Found`]; another attempt would
    /// not change that.
    Failed,
}

/// Every frame of `target` but `except`, from `first` when the caller already read the target
/// (the script's own session), else from a read here.
///
/// A lost debugger session proves nothing about its frames: another client's detach, or a
/// `Target.detachedFromTarget` core missed, ends a session while the frame lives on, and core
/// forgets a session the browser refused as gone, so its next resolve attaches anew
/// (`EvictingConnection`). So an attempt that comes back gone is followed by one look at the
/// browser's target list. A target the browser no longer lists left the page, and with it
/// every frame no script can reach any more; otherwise the target is read and swept again,
/// and a frame gone a second time leaves it [`Unswept::Lost`]. All of it runs by the frames'
/// one `deadline`, so a second attempt never extends their budget.
async fn sweep_target(
    page: &OpPage<'_>,
    target: &FrameId,
    first: Option<TargetFrames>,
    except: Option<&FrameId>,
    deadline: Instant,
) -> Found {
    let mut found = Found::default();
    let first = match first {
        Some(read) => Ok(Some(read)),
        None => read_target(page, target, deadline).await,
    };
    if sweep_attempt(page, first, except, deadline, &mut found).await != Attempt::Lost {
        return found;
    }
    let listed = read_within_budget(deadline, sweep_scope::is_listed(page.browser(), target)).await;
    match listed {
        Ok(Some(false)) => return found,
        Ok(Some(true) | None) => {}
        Err(unswept) => {
            found.fail(unswept);
            return found;
        }
    }
    let again = read_target(page, target, deadline).await;
    if sweep_attempt(page, again, except, deadline, &mut found).await == Attempt::Lost {
        found.fail(Unswept::Lost);
    }
    found
}

/// `target`'s session and frames: the page's own session for the page's target, else the
/// OOPIF's from core's frame registry, which attaches one when it holds none. `None` when the
/// session was gone, or when core answered with another target's session, as it does for a
/// frame it cannot attach.
async fn read_target(
    page: &OpPage<'_>,
    target: &FrameId,
    deadline: Instant,
) -> Result<Option<TargetFrames>, Unswept> {
    let main = page.main_endpoint();
    let read = read_within_budget(deadline, async {
        let session = if *target == main.frame {
            main.session.clone()
        } else {
            page.browser()
                .frames()
                .resolve_frame_target(page.core_page().clone(), Some(target.clone()), None, true)
                .await?
                .session
        };
        let tree = frame_tree(&session).await?;
        Ok((session, tree.frame_tree))
    })
    .await?;
    Ok(read
        .filter(|(_, tree)| tree.frame.id == target.as_str())
        .map(|(session, tree)| TargetFrames {
            session,
            frames: tree.into_frames().collect(),
        }))
}

/// Sweeps every frame of `read` but `except` at once, into `found`.
async fn sweep_attempt(
    page: &OpPage<'_>,
    read: Result<Option<TargetFrames>, Unswept>,
    except: Option<&FrameId>,
    deadline: Instant,
    found: &mut Found,
) -> Attempt {
    let read = match read {
        Ok(Some(read)) => read,
        Ok(None) => return Attempt::Lost,
        Err(unswept) => {
            found.fail(unswept);
            return Attempt::Failed;
        }
    };
    let sweeps = read
        .frames
        .into_iter()
        .filter(|frame| except.is_none_or(|except| frame.id != except.as_str()))
        .map(|frame| {
            let TreeFrame { endpoint, place } =
                frame_endpoint(page, &read.session, FrameId(frame.id));
            sweep_frame(page, endpoint, place, deadline)
        });
    let mut attempt = Attempt::Swept;
    for frame in future::join_all(sweeps).await {
        match frame {
            Ok(Some(values)) => found.add(values),
            Ok(None) if attempt == Attempt::Swept => attempt = Attempt::Lost,
            Ok(None) => {}
            Err(unswept) => {
                found.fail(unswept);
                attempt = Attempt::Failed;
            }
        }
    }
    attempt
}

/// A frame of a session's tree as its sweep reaches it: the endpoint its world lives on, and
/// the frame's place in the page, which the world's install takes with it.
pub(crate) struct TreeFrame {
    pub(crate) endpoint: Endpoint,
    pub(crate) place: Place,
}

/// Where `frame` of `session`'s tree is swept: the page's main frame, whose first document
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

/// One frame's install and sweep by its `deadline`; `None` when it came back gone: its
/// document, its frame or its session.
async fn sweep_frame(
    page: &OpPage<'_>,
    endpoint: Endpoint,
    place: Place,
    deadline: Instant,
) -> Result<Option<FieldValues>, Unswept> {
    let swept = with_world(page, &endpoint, &place, sweep_world);
    match time::timeout_at(deadline, swept).await {
        Err(_) | Ok(Err(ProbeFailure::Late)) => Err(Unswept::Late),
        Ok(Ok(swept)) => Ok(Some(swept)),
        Ok(Err(ProbeFailure::Gone(_))) => Ok(None),
        Ok(Err(failure)) => Err(Unswept::Failed(failure)),
    }
}

/// `read`, a session's tree or the browser's targets, by the `deadline` of the frames it is
/// read for; `None` when it came back gone.
async fn read_within_budget<T>(
    deadline: Instant,
    read: impl Future<Output = Result<T, CoreError>>,
) -> Result<Option<T>, Unswept> {
    match time::timeout_at(deadline, read).await {
        Err(_late) => Err(Unswept::Late),
        Ok(Ok(value)) => Ok(Some(value)),
        Ok(Err(error)) => match ProbeFailure::from_core(error) {
            ProbeFailure::Gone(_) => Ok(None),
            failure => Err(Unswept::Failed(failure)),
        },
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
async fn main_world_document(
    scope: &OpScope<'_>,
    world: SealedWorld,
) -> Result<String, ProbeFailure> {
    let session = world.session();
    let group = scope.object_group(session).to_owned();
    let named: Named = session
        .send(
            "Runtime.callFunctionOn",
            json!({ "objectId": world.global(), "functionDeclaration": SEALED_DOCUMENT,
                    "objectGroup": group }),
        )
        .await
        .map_err(ProbeFailure::from_core)?;
    let sealed = named
        .result
        .object_id
        .ok_or_else(|| ProbeFailure::Broken("the sealed world returned no document".to_owned()))?;
    let described: Described = session
        .send("DOM.describeNode", json!({ "objectId": sealed }))
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
    /// `page.evaluate`'s one call: upstream's `UtilityScript.evaluate` on the main-world document.
    async fn evaluate(
        &self,
        document: &str,
        script: &BoundScript,
        objects: &[String],
    ) -> Result<Outcome, CoreError> {
        let called = self
            .send(
                document,
                MAIN_WORLD_JS,
                arguments("evaluate", script, objects),
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
            .send(
                document,
                MAIN_WORLD_JS,
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
                document: world.as_ref().map(SealedWorld::seq),
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
