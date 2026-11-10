//! Main-world scripts behind sdk-privacy's sweeps (design D14): `page.evaluate`,
//! `page.waitForFunction`, and the element forms (`locator.evaluate`, `evaluateAll`,
//! `elementHandle.evaluate`, `$eval`, `$$eval`), and `page.setContent`'s document write, whose
//! HTML's scripts run in the main world (`document_write`). One of the three files in core's
//! `tests/page_js_guard.rs` ALLOWED list: the only code that sends script to a page's main
//! world, and so the gateway of `Capability::MainWorld`.
//!
//! ```text
//!  EvaluateGuard::open    an op whose spec does not declare MainWorld is refused with nothing
//!                         admitted, and a script the prelude refused is raised; then every
//!                         element the script receives (its subject, its handle arguments) is
//!                         bound in its document and handed to the seal's watch, within the
//!                         op's timeout; then the first sweep: every frame in the script frame's
//!                         site, of the page and of each tab its opener links connect it to, on
//!                         every session (sweep_scope), each within FRAME_BUDGET; one that misses
//!                         it is swept again on a fresh budget while the op's window has room
//!                         (LateRetry), no try running past the op's deadline, and late on the
//!                         last try refuses with nothing sent. No timeout runs around a sweep,
//!                         so none drops what it found
//!  EvaluateGuard::run     name the main-world document and resolve every bound element in the
//!                         main world, then OpScope::send_effect_reaching(the group's other
//!                         tabs, from the last sweep): the host's rules on the op's tab, then on
//!                         those, ──▶ Sent, then
//!                         Runtime.callFunctionOn(assets/main-world.js, userGesture)
//!                         ──▶ Pending, which holds what the send produced
//!  Pending::sweep         the same sweep ──▶ (EvaluateGuard, Outcome); one late on its last
//!                         try withholds the result. A frame form's frame found gone, with no
//!                         answer held, may have moved to another renderer: it is found again
//!                         by its id and the site it is in now is swept
//!  EvaluateGuard::finish  the answer read in Playwright's format and masked with every sweep's
//!                         values (page_value), or the thrown text masked as a page_error
//!  every sweep            PlaywrightHooks::values_swept with every value found so far, before
//!                         the next send, and once more on drop when anything was sent
//! ```
//!
//! Typestate is the point. `run` consumes a swept guard and returns a [`Pending`], whose one
//! method is `sweep`, and only the swept [`EvaluateGuard`] has `finish`. What a send produced
//! stays inside the `Pending` until its sweep hands it out, and the page's text in it is an
//! opaque [`Answer`] that only `finish` reads. So no script goes out without a sweep since the
//! last one, and no answer leaves without a sweep after the script that produced it. A
//! `waitForFunction` that sweeps only before its first poll and after its last, and so returns
//! raw a field that appeared and left mid-wait, cannot be written against it.
//!
//! A script can also act on the tabs of its group (click in a popup, navigate its opener),
//! while the host's rules (the owner's takeover lock, a CAPTCHA solve's hold) are per tab. So
//! every send runs them on the group's other tabs too, as the sweep before it found them
//! (`OpEnv::recheck_reach`), after the op's own tab and before the op is marked sent. A page
//! with no opener link has none, and its host hears `recheck`, `mark_sent` as before. A tab
//! that joins the group between that sweep and the send (a popup the page opens then) is not
//! checked for that send; the sweep after it finds the tab, for the next.
//!
//! The host learns every swept value before the next send, not only when the op ends: an alert
//! or a download the script raises carries page text through the event stream while the op is
//! still pending, and the host masks those events with the values it was told. A value that
//! first appears while the script runs, and leaves in such an event before the sweep after the
//! send, is not known yet: that is the residual.
//!
//! A main-world script reads the page's own, unsealed getters, so whatever it returns may hold a
//! password the page shows. The sweeps hand Rust the raw value of every sensitive field the script
//! could reach: those of every frame of its site, which it touches through `frames[…]` across any
//! number of cross-site frames between them, in its own tab and in each tab its opener links
//! connect it to, through `window.opener` and the windows `window.open` returns. What the script
//! produced is an [`Answer`] whose only way out of the op is [`EvaluateGuard::finish`]. Results are
//! always masked; there is no refuse mode (build-design graft 3). A frame of the site that cannot
//! be swept fails the op closed: before the script nothing is sent, and after it the result is
//! withheld. A frame that comes back gone is not skipped on that alone: unless the browser no
//! longer lists its target, the target is read again and every frame it lists now is swept, and a
//! frame gone a second time fails the op too.
//!
//! The script runs on the main-world document node, never on an execution context id: context
//! ids are per-renderer counters, and after a cross-process navigation a cached one can name a
//! context of another world (measured, see `probe_call`). The sealed world names its document by
//! handle, and `DOM.resolveNode` without a context id resolves that node in its main world. The
//! elements a script receives reach that world the same way, by backend node id, so they are
//! objects of the document's own context, which a call on the document takes as arguments.
//!
//! Elements are where upstream runs them. One out of the document (a handle's node the page
//! removed) still runs, as upstream's does: the seal holds every bound element weakly and, at
//! every sweep from then on, sweeps the tree of each one that is out of the document then, so a
//! field removed with it is masked (core's `seal.js`, `watch`). A handle argument is adopted
//! only into its own document: the script frame's, on the op's page. Upstream also adopts one
//! from a same-origin frame of the same process, through a context id Neo never names, so that
//! case is a decided deviation with upstream's adoption error.

use std::future::Future;
use std::time::Duration;
use std::{mem, slice};

use browseros_cdp::CdpError;
use browseros_core::sensitive_fields::FieldValues;
use browseros_core::{CoreError, FrameId, ProtocolSession, TabId};
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
use crate::page::page_value::{self, MaskedValue, PageValue, TOO_DEEP, Unreadable};
use crate::page::sweep_scope::{self, LinkedTab, Site};
use crate::page::world_call::{self, unexpected, with_world};
use crate::probe::{self, NAVIGATED_OR_CLOSED, ProbeFailure, ProbeReply, ProbeRequest, Ran};
use crate::resolve::{self, HandleSite};
use crate::target::{ElementTarget, PageRef};
use crate::world::{BackendNodeId, Endpoint, FrameSeq, Place, Scan, SealedWorld, SeqLookup};
use crate::{ErrorCode, Playwright, PlaywrightError, op_env};

mod document_write;

/// How long one frame may take to be swept, from the start of the work that reaches it to the
/// end of its sweep: attaching its session, reading its session's frames, a second read after a
/// lost session, installing its world on first use, and the sweep, all under one deadline
/// (decisions.md, "Main-world sweep scope"). A healthy frame takes a few milliseconds. A frame
/// of the script's site that misses it fails that try closed, and the whole sweep goes again on
/// fresh budgets while the op's window has room (`LateRetry`), each budget ending at the
/// window's end at the latest (`TryCut`); late on the last try, it fails the op closed.
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
/// argument may hold a password the agent was given.
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
    /// wait. [`EvaluateGuard::open`] raises it as `<api>: <message>`, before any sweep, with
    /// nothing sent.
    Refused(String),
}

/// What the agent's function is called with before its argument, and so where it runs.
pub(crate) enum Subject {
    /// `fn(arg)` in a frame of the page: the main frame when `None` (`page.evaluate`,
    /// `waitForFunction`), else the child frame a `frame.*` form names, by its id from
    /// `page.frames`. Its site is the one every sweep covers.
    Frame(Option<FrameId>),
    /// `fn(element, arg)` in the element's own frame: `locator.evaluate` once its wait found it,
    /// `$eval`'s first match, `elementHandle.evaluate`. The element may be out of its document:
    /// it still runs, as upstream's does, and its tree is swept.
    Element(ElementTarget),
    /// `fn(elements, arg)`: `evaluateAll` and `$$eval`, in the frame the matches are in, or the
    /// query's starting frame when nothing matched (upstream's `queryArrayInMainWorld`, PW:21919,
    /// measured). `nodes` may be empty.
    Elements {
        frame: TreeFrame,
        /// The frame's document, which a handle argument must be in.
        seq: FrameSeq,
        nodes: Vec<BoundNode>,
    },
}

/// An element a script receives, found in its document's sealed world: its backend node id,
/// which the main world resolves, and its sealed-world object, which the seal watches.
pub(crate) struct BoundNode {
    pub(crate) backend: BackendNodeId,
    pub(crate) sealed: String,
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

/// What one send produced, handed out by the sweep after it.
pub(crate) enum Outcome {
    /// The script answered; only [`EvaluateGuard::finish`] reads it.
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

/// An answer the page gave, unmasked. Opaque outside this file, with no `Serialize` and no
/// `Debug`: it may hold a password, and [`EvaluateGuard::finish`] is the one reader of its text.
pub(crate) struct Answer(Answered);

/// What [`Answer`] holds.
enum Answered {
    /// `page.evaluate`'s result.
    Returned(Unmasked),
    /// `waitForFunction`'s truthy value, and upstream's preview of it for `toString()`.
    Settled {
        value: Unmasked,
        preview: UnmaskedText,
    },
    /// What the page threw, as upstream's `getExceptionMessage` reads it, stack lines and all.
    Threw(UnmaskedText),
}

/// The page's serialized value as it answered it. Its only reader is [`EvaluateGuard::finish`].
enum Unmasked {
    /// Playwright's format, as JSON text.
    Text(String),
    /// No value: the script's `undefined`, which upstream's `jsonValue` reports as none.
    Undefined,
    /// Something `assets/main-world.js` never returns, from a page that tampered with it.
    Unreadable,
}

/// Page text: a thrown message or a preview. Its only reader is [`EvaluateGuard::finish`].
struct UnmaskedText(String);

/// A finished script, masked: what the leaves turn into their op's output.
pub(crate) struct Finished {
    /// The value in Playwright's format.
    pub(crate) value: Value,
    /// `waitForFunction`'s preview of it, upstream's `JSHandle.toString()`.
    pub(crate) preview: Option<String>,
}

/// What a guard sends to the main world, chosen when it opens. It decides the guard's one send
/// and its one exit, what a send leaves inside [`Pending`] until the sweep after it, and how a
/// refusal says what became of the send. Only the swept state has the send and the exit, so
/// every kind is bracketed by sweeps alike.
pub(crate) trait Guarded {
    /// What one send produced, unmasked; only this kind's exit reads it.
    type Outcome;
    /// How a refusal ends when the sweep before the send failed: nothing was sent.
    const NOT_SENT: &'static str;
    /// How a refusal ends when the sweep after the send failed: the send happened.
    const SENT: &'static str;

    /// Lets go of what this kind holds in the script frame's document, for a sweep after
    /// `outcome` that found the frame gone and is about to find it again in its new renderer
    /// ([`GuardCore::sweep`]). `false` holds on instead, when `outcome` holds what the page
    /// answered there: that leaves only masked by a sweep of the site it came from, so the op
    /// fails closed.
    fn leave_document(&mut self, outcome: &Self::Outcome) -> bool;
}

/// The swept state: the guard may send (`run`), and only it lets an answer out (`finish`).
#[must_use]
pub(crate) struct EvaluateGuard<'op, G: Guarded = BoundScript> {
    core: GuardCore<'op>,
    guarded: G,
}

/// The state after a send and before the next sweep: its one method is `sweep`, which alone
/// hands out what the send produced. Dropping it, on an error or a cancelled op, still reports
/// every value swept so far.
#[must_use]
pub(crate) struct Pending<'op, G: Guarded = BoundScript> {
    core: GuardCore<'op>,
    guarded: G,
    outcome: G::Outcome,
}

/// What both states carry, whatever they guard. One `Drop`, so the final report happens once,
/// whichever state ends the op; every sweep reports before it too.
struct GuardCore<'op> {
    pw: &'op Playwright,
    page: OpPage<'op>,
    /// Where the script runs: its site is the site every sweep covers.
    frame: TreeFrame,
    /// The child frame a `frame.*` form names, by its id from `page.frames`. A navigation to
    /// another site moves that frame to another renderer, and so to another session, while it
    /// stays the same frame, so a sweep that finds `frame` gone finds it again by this id. `None`
    /// for the main frame, whose session outlives its navigations, and for an element's
    /// document, which its script cannot outlive.
    follow: Option<FrameId>,
    /// Every value a sweep found so far.
    swept: FieldValues,
    /// The other tabs of the page's opener-linked group, by page target, as the latest sweep
    /// found them, which the host's rules cover before the next send ([`GuardCore::reach`]).
    /// Empty for a page with no opener link, whose sends ask the host nothing more.
    group: Vec<FrameId>,
    /// Something was sent to the main world.
    sent: bool,
}

/// An agent script, bound in its frame's document by `open`: what `run` sends, once or once per
/// poll.
pub(crate) struct BoundScript {
    expression: String,
    is_function: bool,
    arg: ArgText,
    mode: CallMode,
    /// What the function receives before its argument, by backend node id. Its elements and the
    /// handles are all of the guard's frame's document, which `open` checked, so one main world
    /// resolves them all for each run, and the seal of that document watches them.
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

impl Guarded for BoundScript {
    type Outcome = Outcome;
    const NOT_SENT: &'static str = "so the script was not run";
    const SENT: &'static str = "so the result of the script, which ran, is withheld";

    /// The document and the poller go, so the next run names the new document's and builds the
    /// predicate there, as upstream's wait carries on in the frame's next context. An answer
    /// stays, and fails the op with the sweep.
    fn leave_document(&mut self, outcome: &Outcome) -> bool {
        if matches!(outcome, Outcome::Answered(_)) {
            return false;
        }
        self.document = None;
        self.poller = None;
        true
    }
}

/// `setContent`'s HTML, which the probe writes into the guard's frame from the sealed world and
/// whose scripts the main world runs: what `write` sends (`document_write`).
pub(crate) struct DocumentWrite;

impl Guarded for DocumentWrite {
    type Outcome = Written;
    const NOT_SENT: &'static str = "so the content was not set";
    const SENT: &'static str = "though the content was set";

    /// A write's end leaves only through `close`, after a sweep of the frame it was written in,
    /// so a write never follows its frame elsewhere (its guard follows none).
    fn leave_document(&mut self, _written: &Written) -> bool {
        false
    }
}

/// How a document write ended: its load state reached, or the op's failure after the write.
/// Opaque outside the guard, because such a failure may carry text the page's scripts shaped,
/// and only `close` lets it out, masked.
pub(crate) struct Written(Result<(), PlaywrightError>);

impl<'op> EvaluateGuard<'op> {
    /// Opens the gateway: the declaration, the deferred refusal, the page, the first sweep.
    /// This is the only way to a guard, so it checks the op's declaration first: a host serves
    /// an op unheld unless its spec declares `MainWorld`, and such an op must not admit or sweep
    /// anything here.
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
        // The page and the elements are found within the op's timeout; the sweep after them
        // runs outside it, to the deadline at the latest (`GuardCore::sweep_once`).
        let found = async {
            let op_page = OpPage::admit_handle(pw, scope, page).await?;
            let binding = bind(&op_page, scope, page, call.subject, &handles).await?;
            Ok::<_, PlaywrightError>((op_page, binding))
        };
        let (op_page, binding) = match scope.within(found).await {
            Ok(found) => found?,
            Err(_expired) => return Err(scope.timed_out(ErrorCode::WaitTimeout)),
        };
        let mut core = GuardCore {
            pw,
            page: op_page,
            frame: binding.frame,
            follow: binding.follow,
            swept: FieldValues::default(),
            group: Vec::new(),
            sent: false,
        };
        // Nothing of a document is held yet, so the frame may be followed.
        core.sweep(scope, BoundScript::NOT_SENT, || true).await?;
        let guarded = BoundScript {
            expression,
            is_function,
            arg,
            mode: call.mode,
            subject: binding.subject,
            handles: binding.handles,
            document: None,
            poller: None,
        };
        Ok(Self { core, guarded })
    }

    /// Sends the script once: one poll for `waitForFunction`. It names the main-world document
    /// first (three CDP calls on a new document), and only then re-runs the host's policies, on
    /// the op's tab and on the group's other tabs the last sweep found, and marks the op sent,
    /// with no await before the script goes out (STYLE P6). Minted before the document, a
    /// takeover during those calls would go unseen, and a cancel there would leave the op
    /// marked sent although no script ran.
    pub(crate) async fn run(
        mut self,
        scope: &OpScope<'_>,
    ) -> Result<Pending<'op>, PlaywrightError> {
        let core = &mut self.core;
        let script = &mut self.guarded;
        let session = core.frame.endpoint.session.clone();
        let group = scope.object_group(&session).to_owned();
        // The document and the elements in the main world, before the recheck: a takeover while
        // they resolve is seen by it. A poll after the first needs no elements, its predicate
        // holds its argument. A document or an element gone on the way means nothing ran, as
        // when the frame moved to another renderer and left this session behind; the sweep
        // after finds the frame again.
        let named = match script.document(core, scope).await? {
            None => None,
            Some(document) if script.mode == CallMode::Poll && script.poller.is_some() => {
                Some((document, Vec::new()))
            }
            Some(document) => main_world_nodes(&session, &group, &script.handles, &script.subject)
                .await
                .map_err(|error| scope.core_error(&error))?
                .map(|objects| (document, objects)),
        };
        let Some((document, objects)) = named else {
            script.document = None;
            script.poller = None;
            return Ok(Pending {
                core: self.core,
                guarded: self.guarded,
                outcome: Outcome::NotRun,
            });
        };
        let reach = core.reach(scope).await?;
        let sent = scope.send_effect_reaching(&reach)?;
        core.sent = true;
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
            script.document = None;
            script.poller = None;
        }
        Ok(Pending {
            core: self.core,
            guarded: self.guarded,
            outcome,
        })
    }

    /// The answer, masked with every sweep's values: a value in Playwright's format, or a throw
    /// as `page_error` with its stack lines, as upstream prints it. Answers are read here and
    /// nowhere else. A value `PageValue::parse` cannot read fails closed: too deep is upstream's
    /// own failure, and any other shape is the page's.
    pub(crate) fn finish(
        self,
        scope: &OpScope<'_>,
        answer: Answer,
    ) -> Result<Finished, PlaywrightError> {
        let swept = &self.core.swept;
        match answer.0 {
            Answered::Returned(value) => Ok(Finished {
                value: returned(scope, masked(scope, swept, value)?)?,
                preview: None,
            }),
            // A wait hands back a handle, which upstream builds without serializing its value.
            Answered::Settled { value, preview } => Ok(Finished {
                value: masked(scope, swept, value)?.into_json(),
                preview: Some(page_value::mask_text(swept, preview.0)),
            }),
            Answered::Threw(thrown) => Err(scope.fail(
                ErrorCode::PageError,
                &page_value::mask_text(swept, thrown.0),
            )),
        }
    }
}

impl<'op, G: Guarded> Pending<'op, G> {
    /// The sweep after a send, then what the send produced. A frame that misses it withholds
    /// that.
    pub(crate) async fn sweep(
        self,
        scope: &OpScope<'_>,
    ) -> Result<(EvaluateGuard<'op, G>, G::Outcome), PlaywrightError> {
        let Self {
            mut core,
            mut guarded,
            outcome,
        } = self;
        core.sweep(scope, G::SENT, || guarded.leave_document(&outcome))
            .await?;
        Ok((EvaluateGuard { core, guarded }, outcome))
    }
}

impl GuardCore<'_> {
    /// Sweeps every frame in the script frame's site, of the page and of its opener-linked tabs,
    /// and adds what it finds, also when a frame could not be swept, so the host still learns
    /// those values. That is wider than the page's session: a script reaches a same-origin
    /// frame on another session through `frames[…]`, and one in its opener or its popup
    /// through `window.opener` or the window `window.open` returned, as easily as one on its
    /// own.
    ///
    /// The host hears the union at once, before the next send can go out: a dialog or a
    /// download the script raises while it runs carries page text to the host's event stream,
    /// which the host masks with the values it was told. The host's side takes a repeated report
    /// as a union, so a report per sweep costs it nothing new.
    ///
    /// A frame form's frame that comes back gone may only have moved to another renderer
    /// ([`GuardCore::follow`]). Once `leave` lets go of the old document, the frame is found
    /// again by its id through the page's tree, as `open` found it, and the sweep runs once more
    /// there, of the site the frame is in now, so the next send goes out only after a sweep of
    /// that site. A frame the page no longer lists fails `Frame was detached`, as upstream's
    /// frame forms do. `consequence` says what became of the script when a frame stays unswept.
    async fn sweep(
        &mut self,
        scope: &OpScope<'_>,
        consequence: &str,
        leave: impl FnOnce() -> bool,
    ) -> Result<(), PlaywrightError> {
        let mut swept = self.sweep_once().await;
        if matches!(swept, Err(Unswept::Gone))
            && let Some(frame) = self.follow.clone()
            && leave()
        {
            let found = world_call::frame_world(&self.page, scope, Some(&frame));
            let world = match scope.within(found).await {
                Ok(world) => world?,
                Err(_expired) => return Err(scope.timed_out(ErrorCode::WaitTimeout)),
            };
            self.frame = frame_endpoint(&self.page, world.session(), world.frame().clone());
            swept = self.sweep_once().await;
        }
        swept.map_err(|unswept| unswept.into_error(scope, consequence))
    }

    /// One sweep of the script frame's site, its values added and reported, and the group's
    /// other tabs kept for the next send.
    async fn sweep_once(&mut self) -> Result<(), Unswept> {
        // Boxed, as every other caller of `sweep_site` boxes it: its state is large, and a
        // `waitForFunction` holds it at every poll. Its first try ends at the op's deadline, and
        // no timeout of the op's runs around it, so what the frames that answered found always
        // reaches the host.
        let found = Box::pin(sweep_site_from(
            &self.page,
            &self.frame.endpoint,
            &self.frame.place,
            self.page.deadline_cut(),
        ))
        .await;
        self.swept = mem::take(&mut self.swept).union(found.values);
        self.report();
        self.group = found.group;
        found.unswept.map_or(Ok(()), Err)
    }

    /// The group the latest sweep found, as the tab ids the host's rules take, for the send
    /// right after (`OpScope::send_effect_reaching`). Looked up only for a page with opener
    /// links, and only before a send, so the sweep after a script's last send asks nothing
    /// more. A tab of the group the browser lists no tab for cannot be named to the host, so
    /// the send is refused with nothing sent, with the refusal a host that checks no other tab
    /// gives (`op_env::unchecked_reach`).
    async fn reach(&self, scope: &OpScope<'_>) -> Result<Vec<TabId>, PlaywrightError> {
        if self.group.is_empty() {
            return Ok(Vec::new());
        }
        match self.page.tabs_on(&self.group).await {
            Ok(Some(tabs)) => Ok(tabs),
            Ok(None) => Err(op_env::unchecked_reach(self.group.len()).into()),
            Err(error) => Err(scope.core_error(&error)),
        }
    }

    /// Tells the host every value swept so far (`PlaywrightHooks::values_swept`).
    fn report(&self) {
        if !self.swept.is_empty() {
            self.pw
                .hooks()
                .values_swept(self.page.id().clone(), &self.swept);
        }
    }
}

impl BoundScript {
    /// The script frame's main-world document, named once per document; `None` when its world
    /// came back gone a second time, as a frame's does on the session it left.
    async fn document(
        &mut self,
        core: &GuardCore<'_>,
        scope: &OpScope<'_>,
    ) -> Result<Option<String>, PlaywrightError> {
        if let Some(document) = &self.document {
            return Ok(Some(document.clone()));
        }
        let named = with_world(
            &core.page,
            &core.frame.endpoint,
            &core.frame.place,
            |world| main_world_document(scope, world),
        )
        .await;
        match named {
            Ok(named) => {
                self.document = Some(named.clone());
                Ok(Some(named))
            }
            Err(ProbeFailure::Gone(_)) => Ok(None),
            Err(failure) => Err(resolve::failed(scope, failure)),
        }
    }
}

/// The final report, once anything was sent, whichever state ends the op: every sweep has
/// reported already, so this only repeats the union for an op that ends between a send and its
/// sweep (an error, a cancel).
impl Drop for GuardCore<'_> {
    fn drop(&mut self) {
        if self.sent {
            self.report();
        }
    }
}

/// `unmasked` in Playwright's format, masked with `swept`, or why it cannot leave.
fn masked(
    scope: &OpScope<'_>,
    swept: &FieldValues,
    unmasked: Unmasked,
) -> Result<MaskedValue, PlaywrightError> {
    let value = match unmasked {
        Unmasked::Text(text) => PageValue::parse(&text),
        Unmasked::Undefined => Ok(PageValue::undefined()),
        Unmasked::Unreadable => Err(Unreadable::Shape),
    };
    match value {
        Ok(value) => Ok(value.masked(swept)),
        Err(Unreadable::TooDeep) => Err(scope.fail(ErrorCode::PageError, TOO_DEEP)),
        Err(Unreadable::Shape) => Err(scope.fail(ErrorCode::PageError, OUTSIDE_FORMAT)),
    }
}

/// A result upstream's server cannot send back: one holding a `{fn}`, which it reads as a
/// binding's caller and then refuses to serialize, with the caller's own source in the message
/// (measured on BrowserOS neo, spike `__pw_fn_`).
fn returned(scope: &OpScope<'_>, value: MaskedValue) -> Result<Value, PlaywrightError> {
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

/// Why a sweep left a frame of the script's site unswept.
pub(crate) enum Unswept {
    /// It missed its [`FRAME_BUDGET`] on the last try the op's window had room for, or that try
    /// reached the window's end first, as an unresponsive renderer's calls do.
    Late,
    /// It came back gone on a second read of its target too (a lost session, a session of
    /// another target, a missing frame), while the browser still lists that target.
    Lost,
    /// The frame the sweep was taken for came back gone, on a fresh world too: its document,
    /// its frame or its session went away, as mid-navigation. A script there fails, unless a
    /// frame form's frame only moved to another renderer, where the guard finds it again
    /// ([`GuardCore::sweep`]); a read there lands nowhere and polls again, as its own call
    /// coming back gone would (`SiteSweep::values_or_gone`).
    Gone,
    /// The op's own deadline cut the first try of a guard's sweep before a frame of the site
    /// answered ([`OpPage::deadline_cut`]): the frame never had its whole budget, so the op
    /// times out, as upstream's would, rather than refuse.
    Expired,
    /// It failed for another reason.
    Failed(ProbeFailure),
}

impl Unswept {
    /// The op's failure, `consequence` saying what became of the script. A frame that missed
    /// its budget, or that Neo could not reach again, is `neo / unsupported`: Neo cannot keep
    /// its privacy rule for this script now and refuses it, as it refuses a `RegExp` value
    /// matcher on a password field. Neither the script's code nor a Playwright timeout caused
    /// it, and the answer is definite: before the script nothing was sent, and after it the
    /// script is known to have finished.
    fn into_error(self, scope: &OpScope<'_>, consequence: &str) -> PlaywrightError {
        match self {
            Self::Late => scope.fail(
                ErrorCode::Unsupported,
                &format!(
                    "a frame of the script's site did not answer Neo's privacy sweep within \
                     {}ms, {consequence}; try again once the frame responds",
                    FRAME_BUDGET.as_millis()
                ),
            ),
            Self::Lost => scope.fail(
                ErrorCode::Unsupported,
                &format!(
                    "Neo lost its connection to a frame of the script's site and could not reach \
                     it again for its privacy sweep, {consequence}; try again"
                ),
            ),
            Self::Gone => resolve::failed(scope, ProbeFailure::Gone(Ran::No)),
            Self::Expired => scope.timed_out(ErrorCode::WaitTimeout),
            Self::Failed(failure) => resolve::failed(scope, failure),
        }
    }
}

/// What a sweep found so far, and the first frame of the script's site it could not sweep.
#[derive(Default)]
pub(crate) struct Found {
    pub(crate) values: FieldValues,
    pub(crate) unswept: Option<Unswept>,
    /// The other tabs of the page's opener-linked group, by page target, from the target list
    /// the sweep read: every one of them, whatever its site, because a script can act on any
    /// (navigate an opener, click in a popup) even where it cannot read. Reads ignore it.
    pub(crate) group: Vec<FrameId>,
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

/// Every frame in the site of the script's frame at `script`, of the page and of every tab its
/// opener links connect it to, and that group's other tabs, swept again while a frame missed its
/// budget and the op's window has room for another try as long as the last
/// (`OpPage::late_retry`). Each try is a whole sweep on fresh budgets, its frames, targets and
/// links read anew, so a frame that left meanwhile is not waited for, and the group is the last
/// try's. A try after the first is cut at the window's end (`LateRetry::cut`): a try can outlast
/// the one before it, and run past the op's deadline, where the op's own timeout would replace
/// Neo's refusal, or, outside `OpScope::within`, the op would end late. Every try's values are
/// kept, a cut one's too, a linked tab's among them, and a late try's go to the host before the
/// next starts, as every sweep's do: a dialog or a download the script raised can carry page
/// text while the sweep after it is still trying. A frame still late when the window closes
/// leaves the sweep `Unswept::Late`, as one late try did before.
pub(crate) async fn sweep_site(page: &OpPage<'_>, script: &Endpoint, place: &Place) -> Found {
    sweep_site_from(page, script, place, TryCut::default()).await
}

/// [`sweep_site`] with its first try cut at `first`: the guard's, at the op's deadline, so a
/// timeout never drops a sweep with what the frames that answered found. A first try the cut
/// ended before a frame of the site answered is [`Unswept::Expired`], the op's own timeout, as
/// that frame never had its whole budget.
async fn sweep_site_from(
    page: &OpPage<'_>,
    script: &Endpoint,
    place: &Place,
    first: TryCut,
) -> Found {
    let retry = page.late_retry();
    let mut found = Found::default();
    let mut cut = first;
    let mut first_try = true;
    loop {
        let started = Instant::now();
        let tried = sweep_site_once(page, script, place, cut).await;
        found.add(tried.values);
        found.group = tried.group;
        match tried.unswept {
            Some(Unswept::Late) if first_try && first.passed() => {
                found.unswept = Some(Unswept::Expired);
                return found;
            }
            Some(Unswept::Late) if retry.has_room(started.elapsed()) => {
                first_try = false;
                cut = retry.cut();
                if !found.values.is_empty() {
                    page.playwright()
                        .hooks()
                        .values_swept(page.id().clone(), &found.values);
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
/// The script's frame is swept while its session's frames and the browser's list of targets
/// are read; then the session's other frames, every other target of the page that may be of
/// the site, and each linked tab's such targets are swept, all at once. The targets come from
/// the browser, so one of another site, or of a tab no link connects, is never asked anything,
/// hung or not (`sweep_scope`). The list is read on every sweep: no renderer-side count finds
/// every out-of-process frame, and links change as tabs open and close (decisions.md,
/// "Main-world sweep scope"). A page with no opener link reaches its own targets only, from
/// the same reply, so it sends nothing more than before the group was swept. The list's read
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
    found.group = reach.linked.iter().map(|tab| tab.main.clone()).collect();
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
            // A tab's first page state waits on browser-side reads (`PageRegistry::page`), and
            // no timeout of the op's runs around a guard's sweep, so the tab's deadline ends
            // that wait as late too. A cut setup leaves no state behind, and the try's other
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
                .resolve_frame_target(page.id().clone(), Some(target.clone()), None, true)
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
    Settled(Unmasked),
    Threw(UnmaskedText),
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
        Ok(Outcome::Answered(Answer(match reply.thrown() {
            Some(thrown) => Answered::Threw(thrown),
            None => Answered::Returned(reply.result.text()),
        })))
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
            return Ok(Called::Ended(Outcome::Answered(Answer(Answered::Threw(
                thrown,
            )))));
        }
        Ok(match reply.result.object_id {
            Some(poller) => Called::Value(poller),
            None => Called::Ended(Outcome::Answered(Answer(Answered::Returned(
                Unmasked::Unreadable,
            )))),
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
                    Polled::Settled(Unmasked::Text(text.clone()))
                }
                [Value::Bool(true), Value::Null] => Polled::Settled(Unmasked::Undefined),
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
    /// (STYLE P4).
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
    /// agent's code runs right after the host's recheck on every poll; later polls call the
    /// predicate the page kept. A settled poll reads its value again by reference, for
    /// upstream's preview of it.
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
                return Ok(Outcome::Answered(Answer(Answered::Threw(thrown))));
            }
            (Called::Value(Polled::Settled(value)), Some(poller)) => (value, poller),
            (Called::Value(Polled::Unreadable | Polled::Settled(_)), _) => {
                return Ok(Outcome::Answered(Answer(Answered::Returned(
                    Unmasked::Unreadable,
                ))));
            }
        };
        Ok(match call.settled(poller).await? {
            Called::Value(object) => Outcome::Answered(Answer(Answered::Settled {
                value: object.json_value(settled),
                preview: UnmaskedText(object.preview()),
            })),
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
    /// The child frame a frame form named, which the guard follows ([`GuardCore::follow`]).
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
    /// The document's sealed world, when it was found on the way.
    world: Option<SealedWorld>,
    /// The subject, by backend node id.
    subject: Bound,
    /// The subject's elements as sealed-world objects, for the seal to watch.
    watched: Vec<String>,
}

/// Binds the subject and every handle argument in the script frame's document, and hands each
/// element to that world's seal to watch, before the first sweep. Upstream's errors, in its
/// order: the subject first (its document gone is the navigation's message, as upstream's
/// evaluate on a handle of a replaced document; its node gone is Neo's detached message), then
/// each handle as upstream's adoption fails it.
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
        mut world,
        subject,
        mut watched,
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
        let Some(site) = HandleSite::find(page, scope, handle).await? else {
            return Err(retired(page.frames().by_seq(handle.seq)));
        };
        if document != Some(handle.seq) {
            return Err(scope.fail(ErrorCode::InvalidInput, UNABLE_TO_ADOPT));
        }
        world.get_or_insert_with(|| site.world().clone());
        watched.push(site.node().to_owned());
        bound.push(handle.node);
    }
    if let Some(world) = world.filter(|_| !watched.is_empty()) {
        match world.watch_trees(&watched).await {
            // The document went with its world: nothing the script was given can run now, and
            // `run` finds its elements gone.
            Ok(()) | Err(ProbeFailure::Gone(_)) => {}
            Err(failure) => return Err(resolve::failed(scope, failure)),
        }
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
                world,
                subject: Bound::Frame,
                watched: Vec::new(),
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
            let world = site.world().clone();
            let frame = frame_endpoint(page, world.session(), world.frame().clone());
            Placed {
                frame,
                follow: None,
                document: Some(element.seq),
                world: Some(world),
                subject: Bound::Element(element.node),
                watched: vec![site.node().to_owned()],
            }
        }
        Subject::Elements { frame, seq, nodes } => {
            let (backends, sealed): (Vec<_>, Vec<_>) = nodes
                .into_iter()
                .map(|node| (node.backend, node.sealed))
                .unzip();
            let world = if sealed.is_empty() && !with_handles {
                None
            } else {
                Some(
                    page.world(&frame.endpoint, frame.place.clone())
                        .await
                        .map_err(|failure| resolve::failed(scope, failure))?,
                )
            };
            Placed {
                frame,
                follow: None,
                document: Some(seq),
                world,
                subject: Bound::Elements(backends),
                watched: sealed,
            }
        }
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
/// and a poll goes on in the next document after a sweep, as upstream's wait does, its
/// predicate being meant to be free of side effects. A call whose context, session or frame
/// was gone before it reached the page ran nothing, as one on an object the page no longer
/// holds.
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
    fn thrown(&self) -> Option<UnmaskedText> {
        let details = self.exception_details.as_ref()?;
        Some(UnmaskedText(match &details.exception {
            Some(exception) => exception
                .description
                .clone()
                .unwrap_or_else(|| js_string(exception.value.as_ref())),
            None => details.text.clone(),
        }))
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
    fn text(self) -> Unmasked {
        match (self.r#type.as_str(), self.value) {
            ("string", Some(Value::String(text))) => Unmasked::Text(text),
            ("undefined", None) => Unmasked::Undefined,
            _ => Unmasked::Unreadable,
        }
    }

    /// The value upstream's `JSHandle.jsonValue()` gives for this settled value: an object's is
    /// the page's serialized `text`, and a primitive's is CDP's own, where a `BigInt` is lost
    /// (`parseUnserializableValue`, PW:20628), so it reads `undefined`.
    fn json_value(&self, text: Unmasked) -> Unmasked {
        if self.object_id.is_none() && self.r#type == "bigint" {
            Unmasked::Undefined
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
