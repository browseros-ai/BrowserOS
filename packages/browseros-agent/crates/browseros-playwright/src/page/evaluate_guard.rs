//! Main-world scripts behind sdk-privacy's sweep (design D14), for `page.evaluate` and
//! `page.waitForFunction`. One of the three files in core's `tests/page_js_guard.rs` ALLOWED
//! list: the only code that sends script to a page's main world.
//!
//! ```text
//!  EvaluateGuard::sweep_before   sealed `sweep` in every frame of the page in the main frame's
//!                                site, on every session (sweep_scope), each within
//!                                FRAME_BUDGET; one that misses it refuses with nothing sent
//!  EvaluateGuard::run            name the main-world document, then
//!                                OpScope::send_effect ──▶ Sent, then
//!                                Runtime.callFunctionOn on that document ──▶ Unmasked
//!  EvaluateGuard::finish         sweep again; mask the result, or the thrown message, with both
//!  drop                          PlaywrightHooks::values_swept with every value either sweep found
//! ```
//!
//! A main-world script reads the page's own, unsealed getters, so whatever it returns may hold a
//! password the page shows. The sweeps hand Rust the raw value of every sensitive field the
//! script could reach: those of every frame of its site, which it touches through `frames[…]`
//! across any number of cross-site frames between them. What the script produced is an
//! [`Unmasked`], whose only way out of the op is [`EvaluateGuard::finish`]. Results are always
//! masked; there is no refuse mode (build-design graft 3). A frame of the site that cannot be
//! swept fails the op closed: before the script nothing is sent, and after it the result is
//! withheld. A frame that comes back gone is not skipped on that alone: unless the browser no
//! longer lists its target, the target is read again and every frame it lists now is swept,
//! and a frame gone a second time fails the op too.
//!
//! The script runs on the main-world document node, never on an execution context id: context
//! ids are per-renderer counters, and after a cross-process navigation a cached one can name a
//! context of another world (measured, see `probe_call`). The sealed world names its document by
//! handle, and `DOM.resolveNode` without a context id resolves that node in its main world.

use std::future::Future;
use std::mem;
use std::num::FpCategory;
use std::time::Duration;

use browseros_cdp::CdpError;
use browseros_core::sensitive_fields::FieldValues;
use browseros_core::{CoreError, FrameId, PageId, ProtocolSession};
use futures_util::future;
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use tokio::time::{self, Instant};

use crate::op_scope::{OpScope, Sent};
use crate::page::OpPage;
use crate::page::document_watch::{FrameFields, decode};
use crate::page::frame_tree::frame_tree;
use crate::page::sweep_scope::{self, Site};
use crate::page::world_call::{unexpected, with_world};
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest};
use crate::resolve::failed;
use crate::world::{Endpoint, Place, SealedWorld};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// How long one frame may take to be swept, from the start of the work that reaches it to the
/// end of its sweep: attaching its session, reading its session's frames, a second read after a
/// lost session, installing its world on first use, and the sweep, all under one deadline
/// (decisions.md, "Main-world sweep scope"). A healthy frame takes a few milliseconds; a frame
/// of the script's site that misses it fails the op closed.
pub(crate) const FRAME_BUDGET: Duration = Duration::from_secs(1);

/// The function every main-world script runs in: upstream's `normalizeEvaluationExpression`
/// (a method shorthand becomes a function) and `UtilityScript.evaluate` (a function's source is
/// called with the argument; an expression's value is the result). `eval` and `new Function` work
/// here even under a page CSP that forbids them, because Chromium allows code generation inside
/// a call from the inspector (measured on BrowserOS with `script-src 'self'`).
const MAIN_WORLD_FUNCTION: &str = r"function (expression, isFunction, arg) {
  expression = expression.trim();
  if (isFunction) {
    try {
      new Function('(' + expression + ')');
    } catch (_) {
      expression = expression.startsWith('async ') ? 'async function ' + expression.substring(6) : 'function ' + expression;
      try {
        new Function('(' + expression + ')');
      } catch (_) {
        throw new Error('Passed function is not well-serializable!');
      }
    }
  }
  if (/^(async)?\s*function(\s|\()/.test(expression)) expression = '(' + expression + ')';
  let result = globalThis.eval(expression);
  if (isFunction === true || (isFunction === undefined && typeof result === 'function')) result = result(arg);
  return result;
}";

/// Names the document from the sealed world, by handle, so `DOM.describeNode` can name its node.
const SEALED_DOCUMENT: &str = "function () { return document; }";

/// A main-world script as the prelude sends it: upstream's `evaluateExpression` parameters, the
/// argument as JSON. v1 passes JSON only, so `Date`, `RegExp`, `Map` and `undefined` inside an
/// argument are listed deviations. No `Debug`: the argument may be a password.
pub(crate) struct Script {
    /// `String(pageFunction)`: a function's source, or an expression.
    expression: String,
    /// `typeof pageFunction === 'function'`; `None` detects it in the page, as upstream does.
    is_function: Option<bool>,
    /// The argument; `None` is `undefined`.
    arg: Option<Value>,
}

impl Script {
    pub(crate) fn new(expression: String, is_function: Option<bool>, arg: Option<Value>) -> Self {
        Self {
            expression,
            is_function,
            arg,
        }
    }

    /// The three `CallArgument`s of [`MAIN_WORLD_FUNCTION`]; `{}` is `undefined`.
    fn arguments(&self) -> Value {
        let given = |value: Option<Value>| {
            value.map_or_else(|| json!({}), |value| json!({ "value": value }))
        };
        json!([
            { "value": self.expression },
            given(self.is_function.map(Value::Bool)),
            given(self.arg.clone()),
        ])
    }
}

/// What one run of the script produced, before any masking. Its only way out of the op is
/// [`EvaluateGuard::finish`]: it has no `Serialize` and no `Debug`.
pub(crate) enum Unmasked {
    /// The script's completion value.
    Returned(Completion),
    /// What it threw, as upstream prints it, without the page's stack frames.
    Threw(String),
}

/// A completion value as `returnByValue` reports it.
pub(crate) enum Completion {
    Undefined,
    /// JSON. A DOM node or a `Date` comes back as `{}`.
    Json(Value),
    /// `NaN`, `Infinity`, `-Infinity`, `-0` or a `BigInt` (`10n`), which JSON cannot hold.
    Unserializable(String),
}

impl Unmasked {
    /// Whether `waitForFunction` stops at this run: a truthy value ends it, and so does a throw.
    pub(crate) fn ends_wait(&self) -> bool {
        match self {
            Self::Threw(_) => true,
            Self::Returned(Completion::Undefined) => false,
            Self::Returned(Completion::Json(value)) => truthy(value),
            Self::Returned(Completion::Unserializable(text)) => {
                !matches!(text.as_str(), "NaN" | "-0" | "0n")
            }
        }
    }
}

/// JavaScript's truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number
            .as_f64()
            .is_some_and(|number| number.classify() != FpCategory::Zero),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// How one run of the script ended.
pub(crate) enum Ran {
    /// It returned or threw.
    Finished(Unmasked),
    /// The document was gone before the script started, so nothing ran.
    NotRun,
    /// The document was replaced while the script ran, by a navigation it started perhaps.
    Destroyed,
}

/// One op's main-world scripts on one page. Built by the first sweep; runs the script as often
/// as the op needs (once for `evaluate`, once per poll for `waitForFunction`); lets a result out
/// only through [`EvaluateGuard::finish`], after the second sweep.
///
/// Dropping it, on every exit and on cancellation, reports the values both sweeps found through
/// `PlaywrightHooks::values_swept` once a script was sent, so the host registers them as
/// secrets even when no result left the op.
pub(crate) struct EvaluateGuard<'op> {
    pw: &'op Playwright,
    page: OpPage<'op>,
    /// Every value a sweep found so far.
    swept: FieldValues,
    /// The main-world document by handle, in the op's object group. Dropped when a run finds it
    /// gone, so the next run resolves the new document's.
    document: Option<String>,
    /// A script was sent.
    sent: bool,
}

impl<'op> EvaluateGuard<'op> {
    /// Admits `page` and sweeps it before any script runs.
    pub(crate) async fn sweep_before(
        pw: &'op Playwright,
        scope: &'op OpScope<'_>,
        page: &PageId,
    ) -> Result<Self, PlaywrightError> {
        let page = OpPage::admit(pw, scope, page).await?;
        let mut guard = Self {
            pw,
            page,
            swept: FieldValues::default(),
            document: None,
            sent: false,
        };
        guard
            .sweep()
            .await
            .map_err(|unswept| unswept.into_error(scope, "so the script was not run"))?;
        Ok(guard)
    }

    /// Runs `script` once in the main frame's main world. It names the main-world document
    /// first (three CDP calls on a new document), and only then re-runs the host's policies and
    /// marks the op sent, with no await before the script goes out (STYLE P6). Minted before the
    /// document, a takeover during those calls would go unseen, and a cancel there would leave
    /// the op marked sent although no script ran.
    pub(crate) async fn run(
        &mut self,
        scope: &OpScope<'_>,
        script: &Script,
    ) -> Result<Ran, PlaywrightError> {
        let main = self.page.main_endpoint();
        let document = if let Some(document) = &self.document {
            document.clone()
        } else {
            let named = with_world(&self.page, &main, &Place::Main, |world| {
                main_world_document(scope, world)
            })
            .await
            .map_err(|failure| failed(scope, failure))?;
            self.document = Some(named.clone());
            named
        };
        let group = scope.object_group(&main.session).to_owned();
        let sent = scope.send_effect()?;
        self.sent = true;
        let called = call_main_world(&main.session, &document, &group, script, &sent).await;
        let ran = ran(called).map_err(|error| scope.core_error(&error))?;
        if !matches!(ran, Ran::Finished(_)) {
            self.document = None;
        }
        Ok(ran)
    }

    /// Sweeps again, then lets `unmasked` out with every swept value masked in it: a returned
    /// value as the op's result, a thrown message as its `page_error`.
    pub(crate) async fn finish(
        mut self,
        scope: &OpScope<'_>,
        unmasked: Unmasked,
    ) -> Result<Value, PlaywrightError> {
        self.sweep().await.map_err(|unswept| {
            unswept.into_error(scope, "so the result of the script, which ran, is withheld")
        })?;
        match unmasked {
            Unmasked::Returned(completion) => Ok(mask_completion(&self.swept, completion)),
            Unmasked::Threw(message) => {
                Err(scope.fail(ErrorCode::PageError, &mask_text(&self.swept, message)))
            }
        }
    }

    /// Sweeps every frame of the page in the main frame's site and adds what it finds, also
    /// when a frame could not be swept, so the host still learns those values. That is wider
    /// than the page's session: a script reaches a same-origin frame on another session through
    /// `frames[…]` as easily as one on its own.
    async fn sweep(&mut self) -> Result<(), Unswept> {
        let found = sweep_site(&self.page, &self.page.main_endpoint(), &Place::Main).await;
        self.swept = mem::take(&mut self.swept).union(found.values);
        found.unswept.map_or(Ok(()), Err)
    }
}

impl Drop for EvaluateGuard<'_> {
    fn drop(&mut self) {
        if self.sent && !self.swept.is_empty() {
            self.pw
                .hooks()
                .values_swept(self.page.id().clone(), &self.swept);
        }
    }
}

/// One world's sweep: the raw value of every sensitive field in its document.
async fn sweep_world(world: SealedWorld) -> Result<FieldValues, ProbeFailure> {
    match probe::call(&world, &ProbeRequest::Sweep).await? {
        ProbeReply::Sweep { values } => Ok(values),
        other => Err(unexpected("sweep", other)),
    }
}

/// Why a sweep left a frame of the script's site unswept.
pub(crate) enum Unswept {
    /// It missed its [`FRAME_BUDGET`], as an unresponsive renderer's calls do.
    Late,
    /// It came back gone on a second read of its target too (a lost session, a session of
    /// another target, a missing frame), while the browser still lists that target.
    Lost,
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
            Self::Failed(failure) => failed(scope, failure),
        }
    }
}

/// What a sweep found so far, and the first frame of the script's site it could not sweep.
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

/// Every frame of the page in the site of the script's frame at `script`.
///
/// The script's frame is swept while its session's frames and the browser's list of the page's
/// targets are read; then the session's other frames and every other target that may be of
/// the site are swept, all at once. The targets come from the browser, so one of another site,
/// hung or not, is never asked anything (`sweep_scope`). The list is read on every sweep: no
/// renderer-side count finds every out-of-process frame (decisions.md, "Main-world sweep
/// scope").
pub(crate) async fn sweep_site(page: &OpPage<'_>, script: &Endpoint, place: &Place) -> Found {
    // The script's session is read once for all of its frames, so they share one deadline.
    let deadline = Instant::now() + FRAME_BUDGET;
    let (own, tree, targets) = future::join3(
        sweep_frame(page, script.clone(), place.clone(), deadline),
        read_within_budget(deadline, frame_tree(&script.session)),
        sweep_scope::page_targets(page.browser(), &page.main_endpoint().frame),
    )
    .await;
    let mut found = Found::default();
    match own {
        Ok(Some(values)) => found.add(values),
        // The script's own document going away fails the op.
        Ok(None) => found.fail(Unswept::Failed(ProbeFailure::Gone)),
        Err(unswept) => found.fail(unswept),
    }
    let tree = match tree {
        Ok(Some(tree)) => tree.frame_tree,
        Ok(None) => {
            found.fail(Unswept::Failed(ProbeFailure::Gone));
            return found;
        }
        Err(unswept) => {
            found.fail(unswept);
            return found;
        }
    };
    let targets = match targets {
        Ok(targets) => targets,
        Err(error) => {
            found.fail(Unswept::Failed(ProbeFailure::Cdp(error)));
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
    let in_targets = targets
        .into_iter()
        .filter(|target| target.frame != own_target && site.may_hold(&target.url))
        .map(|target| async move {
            let deadline = Instant::now() + FRAME_BUDGET;
            sweep_target(page, &target.frame, None, None, deadline).await
        });
    let (own, targets) = future::join(
        sweep_target(
            page,
            &own_target,
            Some(own_frames),
            Some(&script.frame),
            deadline,
        ),
        future::join_all(in_targets),
    )
    .await;
    found.merge(own);
    for target in targets {
        found.merge(target);
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
        Err(_late) => Err(Unswept::Late),
        Ok(Ok(swept)) => Ok(Some(swept)),
        Ok(Err(ProbeFailure::Gone)) => Ok(None),
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
            ProbeFailure::Gone => Ok(None),
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

/// Sends `script` to the main world. Never logs its params or result: either may hold a
/// password (STYLE P4).
async fn call_main_world(
    session: &ProtocolSession,
    document: &str,
    group: &str,
    script: &Script,
    _sent: &Sent<'_>,
) -> Result<Value, CoreError> {
    session
        .send_value(
            "Runtime.callFunctionOn",
            json!({
                "objectId": document,
                "functionDeclaration": MAIN_WORLD_FUNCTION,
                "arguments": script.arguments(),
                "returnByValue": true,
                "awaitPromise": true,
                "objectGroup": group,
            }),
        )
        .await
}

/// `Runtime.callFunctionOn`'s result.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Called {
    result: RemoteObject,
    #[serde(default)]
    exception_details: Option<ExceptionDetails>,
}

/// A by-value result. `null` arrives as a `value` of `null`, `undefined` as no value at all.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteObject {
    r#type: String,
    #[serde(default, deserialize_with = "present")]
    value: Option<Value>,
    #[serde(default)]
    unserializable_value: Option<String>,
}

#[derive(Deserialize)]
struct ExceptionDetails {
    text: String,
    #[serde(default)]
    exception: Option<Thrown>,
}

/// What the page threw, as a remote object.
#[derive(Deserialize)]
struct Thrown {
    #[serde(default)]
    description: Option<String>,
    #[serde(default, deserialize_with = "present")]
    value: Option<Value>,
}

impl RemoteObject {
    fn completion(self) -> Completion {
        if let Some(text) = self.unserializable_value {
            Completion::Unserializable(text)
        } else if self.r#type == "undefined" {
            Completion::Undefined
        } else {
            Completion::Json(self.value.unwrap_or(Value::Null))
        }
    }
}

impl ExceptionDetails {
    /// Upstream's `getExceptionMessage` without the page's stack frames: an error's
    /// description, a thrown primitive as text, else CDP's summary.
    fn message(self) -> String {
        let text = match self.exception {
            Some(Thrown {
                description: Some(description),
                ..
            }) => description,
            Some(Thrown {
                value: Some(Value::String(text)),
                ..
            }) => text,
            Some(Thrown {
                value: Some(value), ..
            }) => value.to_string(),
            _ => self.text,
        };
        text.lines()
            .filter(|line| !line.starts_with("    at "))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// How the script's call ended, from CDP's reply.
fn ran(called: Result<Value, CoreError>) -> Result<Ran, CoreError> {
    let reply = match called {
        Ok(reply) => reply,
        Err(error) => {
            let outcome = match &error {
                CoreError::Cdp(CdpError::Protocol { message, .. }) => protocol_outcome(message),
                _ => None,
            };
            return outcome.ok_or(error);
        }
    };
    let called: Called = decode(reply)?;
    Ok(Ran::Finished(match called.exception_details {
        Some(exception) => Unmasked::Threw(exception.message()),
        None => Unmasked::Returned(called.result.completion()),
    }))
}

/// The CDP errors that are an outcome of the script rather than a failure, as upstream's
/// `rewriteError` reads them.
fn protocol_outcome(message: &str) -> Option<Ran> {
    const NOT_FOUND: [&str; 3] = [
        "Cannot find context with specified id",
        "Could not find object with given id",
        "does not belong to the document",
    ];
    if message.contains("Execution context was destroyed") {
        Some(Ran::Destroyed)
    } else if NOT_FOUND.iter().any(|gone| message.contains(gone)) {
        Some(Ran::NotRun)
    } else if message.contains("Object reference chain is too long")
        || message.contains("CBOR: stack limit exceeded")
    {
        Some(Ran::Finished(Unmasked::Threw(
            "Cannot serialize result: object reference chain is too long.".to_owned(),
        )))
    } else if message.contains("Object couldn't be returned by value") {
        Some(Ran::Finished(Unmasked::Returned(Completion::Undefined)))
    } else {
        None
    }
}

/// `completion` as the op's JSON result, with every swept value masked in it. JSON has no
/// `undefined`, `NaN`, `Infinity` or `BigInt`; each becomes `null` (and `-0` becomes `0`), as
/// `JSON.stringify` writes them in an array: a listed deviation from upstream's serializer.
fn mask_completion(swept: &FieldValues, completion: Completion) -> Value {
    match completion {
        Completion::Json(mut value) => {
            swept.mask_json(&mut value);
            mask_numbers(swept, &mut value);
            value
        }
        Completion::Unserializable(text) if text == "-0" => json!(0),
        Completion::Undefined | Completion::Unserializable(_) => Value::Null,
    }
}

/// `mask_json` masks strings only, but a script can return a code as a number
/// (`Number(otp.value)`). A number whose digits hold a swept value becomes the masked string.
fn mask_numbers(swept: &FieldValues, value: &mut Value) {
    match value {
        Value::Number(number) => {
            let mut digits = Value::String(number.to_string());
            if swept.mask_json(&mut digits) {
                *value = digits;
            }
        }
        Value::Array(items) => {
            for item in items {
                mask_numbers(swept, item);
            }
        }
        Value::Object(fields) => {
            for field in fields.values_mut() {
                mask_numbers(swept, field);
            }
        }
        Value::Null | Value::Bool(_) | Value::String(_) => {}
    }
}

/// Page text with every swept value masked, as `mask_json` masks a string: a long value
/// wherever it occurs (with its newlines, unlike `mask_str`), a short one only as the whole text.
pub(crate) fn mask_text(swept: &FieldValues, text: String) -> String {
    let mut leaf = Value::String(text);
    swept.mask_json(&mut leaf);
    match leaf {
        Value::String(text) => text,
        other => other.to_string(),
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
mod tests {
    use std::error::Error;
    use std::sync::Arc;
    use std::time::Duration;

    use browseros_cdp::{CdpError, CdpEvent};
    use browseros_core::sensitive_fields::FieldValues;
    use browseros_core::{
        BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, CoreError, SessionId,
    };
    use browseros_trace::ErrorOwner;
    use futures_util::future::BoxFuture;
    use serde_json::{Value, json};
    use tokio::sync::broadcast;
    use tokio::time::{self, Instant};

    use super::{
        Completion, EvaluateGuard, FRAME_BUDGET, Ran, Unmasked, mask_completion, mask_text, ran,
    };
    use crate::op_scope::OpScope;
    use crate::probe::PROBE_PROTOCOL;
    use crate::target::CallHeader;
    use crate::test_env::TestEnv;
    use crate::test_support::RecordingHooks;
    use crate::{ErrorCode, Op, Playwright};

    type TestResult = Result<(), Box<dyn Error>>;

    fn swept() -> FieldValues {
        FieldValues::from_sweep(&json!(["hunter2-secret", "482913", "123"]))
    }

    fn finished(reply: Value) -> Result<Unmasked, Box<dyn Error>> {
        match ran(Ok(reply))? {
            Ran::Finished(unmasked) => Ok(unmasked),
            Ran::NotRun | Ran::Destroyed => Err("a reply is a finished run".into()),
        }
    }

    fn result_of(unmasked: Unmasked) -> Result<Value, Box<dyn Error>> {
        let Unmasked::Returned(completion) = unmasked else {
            return Err("the script returned".into());
        };
        Ok(mask_completion(&swept(), completion))
    }

    #[test]
    fn tests_that_a_result_is_json_and_what_json_cannot_hold_is_null() -> TestResult {
        let null =
            finished(json!({ "result": { "type": "object", "subtype": "null", "value": null } }))?;
        assert_eq!(result_of(null)?, json!(null));
        let undefined = finished(json!({ "result": { "type": "undefined" } }))?;
        assert_eq!(result_of(undefined)?, json!(null), "JSON has no undefined");
        let nan =
            finished(json!({ "result": { "type": "number", "unserializableValue": "NaN" } }))?;
        assert_eq!(result_of(nan)?, json!(null));
        let zero =
            finished(json!({ "result": { "type": "number", "unserializableValue": "-0" } }))?;
        assert_eq!(result_of(zero)?, json!(0));
        Ok(())
    }

    #[test]
    fn tests_that_a_swept_value_is_masked_in_strings_keys_and_numbers() -> TestResult {
        let echoed = finished(json!({ "result": { "type": "object", "value": {
            "typed": "you typed hunter2-secret",
            "hunter2-secret": true,
            "code": 482_913,
            "cvc": "123",
            "price": 1234,
            "plain": "untouched",
        } } }))?;
        assert_eq!(
            result_of(echoed)?,
            json!({
                "typed": "you typed [private]",
                "[private]": true,
                "code": "[private]",
                "cvc": "[private]",
                "price": 1234,
                "plain": "untouched",
            }),
            "a code returned as a number is as secret as the string; other numbers stay"
        );
        let bigint =
            finished(json!({ "result": { "type": "bigint", "unserializableValue": "482913n" } }))?;
        assert_eq!(
            result_of(bigint)?,
            json!(null),
            "a BigInt, a code perhaps, never leaves"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_thrown_message_is_upstreams_without_the_stack_and_masked() -> TestResult {
        let error = finished(json!({
            "result": { "type": "object", "subtype": "error" },
            "exceptionDetails": { "text": "Uncaught", "exception": {
                "description": "TypeError: hunter2-secret is not a function\n    at eval (eval at <anonymous> (:17:24), <anonymous>:1:7)\n    at <anonymous>:18:73" } },
        }))?;
        let Unmasked::Threw(message) = error else {
            return Err("the script threw".into());
        };
        assert_eq!(
            mask_text(&swept(), message),
            "TypeError: [private] is not a function"
        );
        let primitive = finished(json!({
            "result": { "type": "string" },
            "exceptionDetails": { "text": "Uncaught", "exception": { "type": "string", "value": "plain" } },
        }))?;
        assert!(
            matches!(&primitive, Unmasked::Threw(message) if message == "plain"),
            "a thrown string prints as itself"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_gone_document_is_an_outcome_and_other_errors_fail() -> TestResult {
        let protocol = |message: &str| {
            Err(CoreError::Cdp(CdpError::Protocol {
                code: -32000,
                message: message.to_owned(),
            }))
        };
        assert!(matches!(
            ran(protocol("Cannot find context with specified id"))?,
            Ran::NotRun
        ));
        assert!(matches!(
            ran(protocol("Execution context was destroyed."))?,
            Ran::Destroyed
        ));
        assert!(matches!(
            ran(protocol("Object couldn't be returned by value"))?,
            Ran::Finished(Unmasked::Returned(Completion::Undefined))
        ));
        assert!(matches!(
            ran(protocol("Object reference chain is too long"))?,
            Ran::Finished(Unmasked::Threw(message)) if message.starts_with("Cannot serialize result")
        ));
        assert!(
            ran(protocol("Internal error")).is_err(),
            "an unknown CDP error is the op's failure"
        );
        Ok(())
    }

    const PAGE_TARGET: &str = "page-target";
    const PAGE_SESSION: &str = "page-session";
    const OOPIF: &str = "oopif-target";
    const OOPIF_SESSION: &str = "oopif-session";

    /// How long each half of reaching and sweeping the slow frame takes: its attach plus its
    /// frame tree, then its world's install plus the sweep.
    const HALF: Duration = Duration::from_millis(600);

    /// A browser with one tab on `https://a.test/` and an out-of-process frame of the same
    /// site whose renderer is slow: its attach, frame tree, world install and sweep take
    /// [`HALF`] a pair. Every other command answers at once.
    struct SlowFrame {
        events: broadcast::Sender<CdpEvent>,
    }

    impl SlowFrame {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                events: broadcast::channel(16).0,
            })
        }

        async fn answer(&self, method: &str, params: &Value, session: Option<&SessionId>) -> Value {
            let oopif = session.is_some_and(|session| session.as_str() == OOPIF_SESSION);
            match method {
                "Browser.getTabs" => json!({ "tabs": [{ "tabId": 7, "targetId": PAGE_TARGET,
                    "windowId": 1, "url": "https://a.test/", "title": "A", "isActive": true,
                    "isLoading": false, "loadProgress": 1.0, "isPinned": false,
                    "isHidden": false }] }),
                "Target.attachToTarget" => {
                    let target = params["targetId"].as_str().unwrap_or_default();
                    stage(target == OOPIF).await;
                    let session = if target == OOPIF {
                        OOPIF_SESSION
                    } else {
                        PAGE_SESSION
                    };
                    json!({ "sessionId": session })
                }
                "Target.getTargets" => json!({ "targetInfos": [
                    { "targetId": PAGE_TARGET, "type": "page", "url": "https://a.test/" },
                    { "targetId": OOPIF, "type": "iframe", "url": "https://pay.a.test/",
                      "parentId": PAGE_TARGET },
                ] }),
                "Page.getFrameTree" => {
                    stage(oopif).await;
                    let (id, url) = if oopif {
                        (OOPIF, "https://pay.a.test/")
                    } else {
                        (PAGE_TARGET, "https://a.test/")
                    };
                    json!({ "frameTree": { "frame": { "id": id, "loaderId": "L", "url": url,
                        "domainAndRegistry": "a.test" } } })
                }
                "Page.createIsolatedWorld" => {
                    stage(oopif).await;
                    json!({ "executionContextId": 7 })
                }
                "Runtime.evaluate" => json!({ "result": { "type": "object", "objectId": "g" } }),
                "Runtime.callFunctionOn" => {
                    let request = &params["arguments"][0]["value"];
                    if request["op"] == "sweep" {
                        stage(oopif).await;
                        json!({ "result": { "type": "object",
                            "value": { "kind": "sweep", "values": [] } } })
                    } else {
                        json!({ "result": { "type": "object", "value": { "kind": "described",
                            "protocol": PROBE_PROTOCOL, "seq": request["frameSeq"], "url": "" } } })
                    }
                }
                _ => json!({}),
            }
        }
    }

    /// One stage of the slow frame, half of a [`HALF`]; nothing for any other frame.
    async fn stage(slow: bool) {
        if slow {
            time::sleep(HALF / 2).await;
        }
    }

    impl CdpConnection for SlowFrame {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            Box::pin(async move { Ok(self.answer(method, &params, session).await) })
        }

        fn send_raw_json<'a>(
            &'a self,
            method: &'a str,
            params_json: &'a str,
            session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async move {
                let params = serde_json::from_str(params_json)?;
                let reply = self.answer(method, &params, session).await;
                Ok(serde_json::to_string(&reply)?)
            })
        }

        fn events(&self) -> broadcast::Receiver<CdpEvent> {
            self.events.subscribe()
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    /// Reaching the slow frame and sweeping it each fit in a budget of their own, 600 ms
    /// against 1 s, but the frame has one budget for both: it refuses the script at 1 s
    /// rather than passing at 1.2 s.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_frame_has_one_budget_from_its_attach_to_its_sweep() -> TestResult {
        let browser = BrowserSession::new(
            SlowFrame::new() as Arc<dyn CdpConnection>,
            BrowserSessionHooks::default(),
        );
        let playwright = Playwright::new(Arc::clone(&browser), Arc::new(RecordingHooks::default()));
        let listed = browser.pages.list().await?;
        let page = listed.first().ok_or("the tab is listed")?.page_id.clone();
        let env = TestEnv::over(BrowserView::unbound(Arc::clone(&browser), None));
        let op = Op::from_name("page.evaluate").ok_or("page.evaluate is an op")?;
        let scope = OpScope::new(&env, op, CallHeader::default());
        let started = Instant::now();
        let swept = EvaluateGuard::sweep_before(&playwright, &scope, &page).await;
        let took = started.elapsed();
        let error = swept.err().ok_or(format!(
            "a frame 1.2 s from its attach to its sweep passed, after {took:?}"
        ))?;
        assert_eq!(
            (error.error_code(), error.owner()),
            (Some(ErrorCode::Unsupported), ErrorOwner::Neo),
            "{error}"
        );
        assert!(
            took >= FRAME_BUDGET && took < FRAME_BUDGET + HALF / 2,
            "refused when the frame's one budget ran out, after {took:?}"
        );
        Ok(())
    }

    #[test]
    fn tests_that_waiting_ends_on_a_truthy_value_or_a_throw() {
        let returned = |value: Value| Unmasked::Returned(Completion::Json(value)).ends_wait();
        assert!(!returned(json!(null)) && !returned(json!(0)) && !returned(json!("")));
        assert!(!returned(json!(false)));
        assert!(returned(json!({})) && returned(json!([])) && returned(json!("0")));
        assert!(returned(json!(0.5)));
        assert!(!Unmasked::Returned(Completion::Undefined).ends_wait());
        assert!(!Unmasked::Returned(Completion::Unserializable("NaN".to_owned())).ends_wait());
        assert!(Unmasked::Returned(Completion::Unserializable("Infinity".to_owned())).ends_wait());
        assert!(
            Unmasked::Threw("Error: x".to_owned()).ends_wait(),
            "a throw fails the wait"
        );
    }
}
