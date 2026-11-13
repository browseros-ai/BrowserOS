//! The snapshot ops: `page.ariaSnapshot` and `locator.ariaSnapshot`, every frame read by its
//! own probe in its own isolated world, as upstream's server reads them
//! (`ariaSnapshotJSONForFrame`, server/page.ts at v1.64.1).
//!
//! ```text
//!  root frame   snapshot { root }       the op: its own retry loop and Playwright timeout
//!     │           → json, iframes [{ ref, depth, src }]
//!     └─ every iframe ref, all at once, each child inside its own FRAME_BUDGET:
//!          the world that printed the ref (its f<seq> prefix) ─ hop(aria-ref=<ref>)
//!            ─ the child's world ─ snapshot { root: document }
//!          read      → the same below it, each grandchild on a fresh budget
//!          not read  → PartialFrame { ref, frame: src, reason }; the iframe keeps its ref
//!  aria_snapshot::stitch: merge, the name rule → { json, partial }
//! ```
//!
//! With `clickable` in mode `ai` (the prelude's v1 text asks for it), each frame's snapshot call
//! also carries that frame's clickable targets and the seeds of its closed shadow roots
//! (`clickable_targets`), read before it; a locator's own frame resolves its element first, so
//! they come from the world it landed in.
//!
//! A snapshot only reads, so it never calls `send_effect`. Default mode mints no refs and so
//! prints no iframes to follow: it reads one frame, as upstream's does.
//!
//! `page.snapshotBaseline` and `page.snapshotPending` are Neo's own, behind
//! `ariaSnapshot({ delta: true })`. After the prelude renders a whole page's v1 text, a delta
//! call reads the tab's stored baseline to diff against, and every whole-page v1 call records its
//! text, and the string it returned, as pending on the run's state; the host commits or drops
//! that when the run ends (`snapshot_baselines.rs`, `pending_baselines.rs`).

use std::borrow::Cow;
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::future::{self, BoxFuture};
use serde::Deserialize;
use serde_json::Value;
use tokio::time::{self, Instant};
use ts_rs::TS;

use super::aria_snapshot::{self, AriaNode, ChildFrame, ClosedRoots, FrameSnapshot, PartialFrame};
use super::clickable_targets::{Feed, Marking};
use super::{Baseline, SnapshotForm};
use crate::backoff::Backoff;
use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, PAGE, TARGET};
use crate::page::OpPage;
use crate::probe::{
    self, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget, SnapshotIframe, SnapshotMode, Want,
};
use crate::resolve::{self, Hopped, Landed, Resolver};
use crate::target::{Chunk, PageRef, Strictness, Target};
use crate::wire::{self, Describe, Nothing, OpWire, Route, Run, options};
use crate::world::{IsolatedWorld, Pin, Place, SeqLookup};
use crate::{ErrorCode, Playwright, PlaywrightError};

listed_enum! {
    /// Aria snapshots of a whole page (every frame stitched in) or of one element, and the
    /// tab's delta baseline: read, and a run's text recorded as pending.
    pub(crate) enum SnapshotOp {
        Page,
        Locator,
        Baseline,
        Pending,
    }
}

impl SnapshotOp {
    /// Describes the same input and result types the runtime route decodes and encodes.
    pub(crate) fn wire(self) -> OpWire {
        route(self, Describe)
    }

    /// This op's registration metadata.
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::Page => OpSpec::read(
                "page.ariaSnapshot",
                PAGE,
                "the page's aria snapshot with its frames stitched in; mode 'ai' mints refs, and \
                 `clickable` marks what Chromium reports clickable",
            ),
            Self::Locator => OpSpec::read(
                "locator.ariaSnapshot",
                TARGET,
                "an aria snapshot rooted at one element; mode 'ai' mints refs, and `clickable` \
                 marks what Chromium reports clickable",
            ),
            // Reads: they touch only text Neo keeps, never the page.
            Self::Baseline => OpSpec::read(
                "page.snapshotBaseline",
                PAGE,
                "the tab's delta baseline: the last whole-page v1 snapshot text of that \
                 document its caller was given, with its form, or null",
            ),
            Self::Pending => OpSpec::read(
                "page.snapshotPending",
                PAGE,
                "records a whole-page v1 snapshot's text, its form, and the string the call \
                 returned, as the run's pending baseline for its tab",
            ),
        }
    }
}

/// One exhaustive table binds each snapshot's handler to the upstream command. `Run` and
/// `Describe` walk these same rows; the policy in `spec()` remains independent of the shapes.
fn route<'a, R: Route<'a>>(op: SnapshotOp, r: R) -> R::Out {
    let command = wire::command("Frame.ariaSnapshot")
        .adds(&[
            (
                "clickable",
                "Neo also marks elements Chromium reports clickable without a pointer cursor, so \
                 the condensed snapshot keeps their refs; read over CDP, never a script in the page",
            ),
            (
                "conformance",
                "the prelude's captured spec-harness switch bypasses the masking of page-declared \
                 private fields in every frame, never of a box Neo typed a host's secret into; no \
                 public snapshot option can set it",
            ),
        ])
        .result(
            "Each frame masks page-declared private field values outside conformance; Rust \
             stitches the child frames and answers the JSON tree plus the frames it could \
             not read, with `clickable` also marking `cursor: 'pointer'` on the elements \
             Chromium reports clickable; the prelude renders the text, upstream's YAML for mode \
             'default' and Neo's condensed list of actionable lines for 'ai', each line \
             upstream's own, and annotates the partial frames; a whole page's answer also names \
             the document it read, for the delta baseline",
        );
    match op {
        SnapshotOp::Page => r.op(command, page_snapshot),
        SnapshotOp::Locator => r.op(command, locator_snapshot),
        SnapshotOp::Baseline => r.op(
            wire::neo_only(
                "Playwright keeps no snapshot between calls; Neo keeps the last whole-page v1 \
                 text per tab on the host for ariaSnapshot({ delta: true }), since each run is \
                 a fresh realm and the page's world must not hold cross-site frames' text; it \
                 answers how that text was rendered too (its text option, urls and cap), since a \
                 delta takes the options it does not pass from it and the text alone does not \
                 say them",
            ),
            read_baseline,
        ),
        SnapshotOp::Pending => r.op(
            wire::neo_only(
                "the delta baseline is the text the caller was given, which only the host \
                 knows when the run ends, so each whole-page v1 snapshot's text waits on the \
                 run's state until then, with how the prelude rendered it, which the baseline \
                 keeps for the next delta",
            ),
            record_pending,
        ),
    }
}

/// How long one child frame's own calls may take: the hop into it, its world's install and its
/// snapshot (decisions.md). A frame that misses it is reported partial and
/// the rest of the page is returned; upstream instead waits on a hung frame until the whole call
/// times out. Its children start budgets of their own, so a slow level does not eat the levels
/// below.
const FRAME_BUDGET: Duration = Duration::from_secs(2);

/// `page.ariaSnapshot`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "PageSnapshotInput")]
struct PageInput {
    page: PageRef,
    #[ts(type = "import('../../../probe/protocol').SnapshotMode")]
    mode: SnapshotMode,
    #[serde(default)]
    #[ts(optional)]
    depth: Option<i64>,
    /// Upstream's `boxes`: every node's box in its own frame's viewport.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    boxes: bool,
    /// Neo's clickable marking, in mode `ai` only: the prelude asks for it in its condensed
    /// format, never under the spec harness's conformance switch (`clickable_targets`).
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    clickable: bool,
    // Spec-harness bypass. Omission keeps normal snapshot value masking enabled.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    conformance: bool,
}
// Snapshots have always sent flat fields, not a nested `options` bag. Keep that wire shape;
// there is no options bag for the generator's option-key probe to describe.
options!(PageInput);

/// `locator.ariaSnapshot`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "LocatorSnapshotInput")]
struct LocatorInput {
    target: Target,
    strict: Strictness,
    #[ts(type = "import('../../../probe/protocol').SnapshotMode")]
    mode: SnapshotMode,
    #[serde(default)]
    #[ts(optional)]
    depth: Option<i64>,
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    boxes: bool,
    /// Neo's clickable marking, as on `page.ariaSnapshot`.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    clickable: bool,
    // Spec-harness bypass, carried unchanged into every frame's isolated-world walk.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    conformance: bool,
}
options!(LocatorInput);

/// `page.snapshotBaseline`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "SnapshotBaselineInput")]
struct BaselineInput {
    page: PageRef,
    /// The `document` the page's snapshot answered. None finds no baseline.
    #[serde(default)]
    #[ts(optional)]
    document: Option<u32>,
    /// The `snapshotCall` the page's snapshot answered: the read is held to that call's
    /// deadline. None holds it to none.
    #[serde(default, rename = "snapshotCall")]
    #[ts(optional)]
    snapshot_call: Option<u32>,
}
options!(BaselineInput);

/// `page.snapshotPending`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "SnapshotPendingInput")]
struct PendingInput {
    page: PageRef,
    /// The `document` the page's snapshot answered. A text with none is not recorded.
    #[serde(default)]
    #[ts(optional)]
    document: Option<u32>,
    /// The `snapshotCall` the page's snapshot answered: the record is held to that call's
    /// deadline. None holds it to none.
    #[serde(default, rename = "snapshotCall")]
    #[ts(optional)]
    snapshot_call: Option<u32>,
    /// The whole page's v1 text, after the notice and the cap: what the baseline becomes.
    text: String,
    /// How `text` was rendered, which the baseline keeps beside it: the form a later delta
    /// takes the options it does not pass from.
    form: SnapshotForm,
    /// The script passed `delta: true`. Without it, a stored baseline found means a delta was
    /// possible, which the run's reply turns into a tip.
    delta: bool,
    /// What the call returned, when it was not `text` itself: a delta call's delta. The text
    /// becomes the baseline only if this string reaches the caller whole.
    #[serde(default)]
    #[ts(optional)]
    returned: Option<String>,
}
options!(PendingInput);

/// Runs one snapshot op; its output is `aria_snapshot::SnapshotResult`, `{ json, partial }`.
pub(crate) async fn invoke(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: SnapshotOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    route(op, Run { pw, scope, args }).await
}

/// The page's snapshot from the main frame's body. The main frame is the op: while its
/// document has no body yet, or its world went away under a navigation, it tries again on the
/// outer backoff until the op's timeout, as upstream polls for the body. Its child frames never
/// make it wait.
async fn page_snapshot(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PageInput,
) -> Result<aria_snapshot::SnapshotResult, PlaywrightError> {
    let page = input.page.tab();
    let page = OpPage::admit(pw, scope, page).await?;
    if input.mode == SnapshotMode::Ai {
        scope.run_state().opened_tabs.refs_issued(page.tab());
    }
    // Before the tree: a document that commits after this makes the text stale for the
    // baseline, so a run that ends later cannot hand the new document the old one's text.
    let document = pw.snapshot_baselines().generation(page.target_id());
    let request = ProbeRequest::Snapshot {
        root: ProbeTarget::Document,
        mode: input.mode,
        depth: input.depth,
        boxes: input.boxes,
        closed_roots: 0,
        conformance: input.conformance,
        same_process_neighbour: true,
    };
    let backoff = Backoff::outer(scope.timeout());
    for attempt in 0.. {
        if scope.sleep(backoff.wait(attempt)).await.is_err() {
            return Err(scope.timed_out(ErrorCode::WaitTimeout));
        }
        // Each attempt captures afresh: a document read before its body existed has no targets.
        let snapshotter = Snapshotter {
            scope,
            page: &page,
            mode: input.mode,
            boxes: input.boxes,
            conformance: input.conformance,
            marking: marks(input.mode, input.clickable).then(Marking::new),
            reading: Mutex::default(),
        };
        let Ok(read) = scope.within(snapshotter.main_frame(&request)).await else {
            snapshotter.cut_off();
            return Err(scope.timed_out(ErrorCode::WaitTimeout));
        };
        if let Some(root) = read? {
            let stitched = snapshotter.stitched(root, input.depth).await?;
            // The call this tree answers, and when its timeout passes, for the baseline ops
            // the prelude may send after it (`snapshot_calls.rs`).
            let call = scope
                .run_state()
                .snapshot_calls
                .taken(scope.timeout(), scope.deadline());
            return Ok(aria_snapshot::SnapshotResult {
                document: Some(document),
                snapshot_call: Some(call),
                ..stitched
            });
        }
    }
    Err(scope.timed_out(ErrorCode::WaitTimeout))
}

/// One element's snapshot, with the frames inside it. It waits for the locator's one element
/// in both modes, as upstream's default mode does, through the resolver's frame hops and ref
/// jumps; a strict violation, a stale ref and an error the page throws end it at once.
async fn locator_snapshot(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: LocatorInput,
) -> Result<aria_snapshot::SnapshotResult, PlaywrightError> {
    let Target::Locator(target) = &input.target else {
        return Err(scope.not_built());
    };
    let page = OpPage::admit_handle(pw, scope, &target.page).await?;
    if input.mode == SnapshotMode::Ai {
        scope.run_state().opened_tabs.refs_issued(page.tab());
    }
    let marked = marks(input.mode, input.clickable);
    let resolver = Resolver::new(&page, scope, target, input.strict);
    scope.log(format!("waiting for {}", target.display));
    let backoff = Backoff::outer(scope.timeout());
    for attempt in 0.. {
        if scope.sleep(backoff.wait(attempt)).await.is_err() {
            return Err(scope.timed_out(ErrorCode::NoMatch));
        }
        let snapshotter = Snapshotter {
            scope,
            page: &page,
            mode: input.mode,
            boxes: input.boxes,
            conformance: input.conformance,
            marking: marked.then(Marking::new),
            reading: Mutex::default(),
        };
        // Marked, the element is pinned first: its frame's targets are read in the world it
        // landed in, then the snapshot is taken there with them. Unmarked, one call does both.
        let request = |last, _frame_visible, world: &IsolatedWorld| {
            let root = ProbeTarget::Chunk(last);
            if marked {
                ProbeRequest::Resolve {
                    target: root,
                    strictness: Strictness::Strict,
                    want: Want::Element,
                }
            } else {
                with_frame_context(Cow::Owned(snapshotter.request(root, input.depth)), || {
                    page.same_process_neighbour(scope, world)
                })
                .into_owned()
            }
        };
        let Ok(attempt) = scope.within(resolver.attempt_at_world(request)).await else {
            return Err(scope.timed_out(ErrorCode::NoMatch));
        };
        let Some(Landed { reply, world, path }) = attempt? else {
            continue;
        };
        let reply = match reply {
            ProbeReply::Element { pin, .. } => {
                let parent = path.hops().last().map(|hop| &hop.parent);
                let pinned = snapshotter.pinned(&world, parent, pin, input.depth);
                let Ok(pinned) = scope.within(pinned).await else {
                    return Err(scope.timed_out(ErrorCode::NoMatch));
                };
                pinned?
            }
            other => other,
        };
        match reply {
            ProbeReply::Snapshot { json, iframes, .. } => {
                let root = frame_reply(json, iframes)
                    .map_err(|why| scope.fail(ErrorCode::Internal, why))?;
                return snapshotter.stitched(root, input.depth).await;
            }
            ProbeReply::None { stale_ref: None } | ProbeReply::Detached => {}
            other => return Err(resolver.refused(other)),
        }
    }
    Err(scope.timed_out(ErrorCode::NoMatch))
}

/// `request` as it goes with `seeds` closed-root seeds first among its nodes; borrowed when it
/// carries none, which is every request but a marked snapshot's on a page with a closed root.
fn with_closed_roots(request: &ProbeRequest, seeds: usize) -> Cow<'_, ProbeRequest> {
    let mut request = Cow::Borrowed(request);
    if seeds > 0
        && let ProbeRequest::Snapshot { closed_roots, .. } = request.to_mut()
    {
        *closed_roots = u32::try_from(seeds).unwrap_or(u32::MAX);
    }
    request
}

/// Bind the hint lazily to the world about to be read, after marking or a resolver retry.
/// Keeping the read inside this closure makes conformance perform no topology work at all.
fn with_frame_context<'request>(
    mut request: Cow<'request, ProbeRequest>,
    neighbour: impl FnOnce() -> bool,
) -> Cow<'request, ProbeRequest> {
    if let ProbeRequest::Snapshot {
        conformance: false,
        same_process_neighbour,
        ..
    } = request.as_ref()
    {
        let neighbour = neighbour();
        if neighbour != *same_process_neighbour
            && let ProbeRequest::Snapshot {
                same_process_neighbour,
                ..
            } = request.to_mut()
        {
            *same_process_neighbour = neighbour;
        }
    }
    request
}

/// The tab's stored baseline for the snapshot's document, or `null`: none yet, its main frame
/// committed a new document since, or no `document` was given. Never a text pending in this run,
/// so a delta compares against what the caller held when the run started. The page is admitted
/// like any op's, so a tab the run may not touch keeps its baseline unread.
async fn read_baseline(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: BaselineInput,
) -> Result<Option<Baseline>, PlaywrightError> {
    let page = OpPage::admit(pw, scope, input.page.tab()).await?;
    within_snapshot_call(scope, input.snapshot_call, "reading its delta baseline")?;
    Ok(input
        .document
        .and_then(|document| pw.snapshot_baselines().stored(page.target_id(), document)))
}

/// Records the snapshot's text as pending on the run's state; the host settles it when the run
/// ends (`RunState::settle_snapshot_baselines`). A full snapshot that finds a stored baseline is
/// noted for the reply's tip.
async fn record_pending(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PendingInput,
) -> Result<Nothing, PlaywrightError> {
    let page = OpPage::admit(pw, scope, input.page.tab()).await?;
    within_snapshot_call(scope, input.snapshot_call, "recording its delta baseline")?;
    let Some(document) = input.document else {
        return Ok(());
    };
    let store = pw.snapshot_baselines();
    if !input.delta && store.has_stored(page.target_id(), document) {
        scope.run_state().note_full_snapshot_repeat(page.tab());
    }
    scope.run_state().pending_baselines.record(
        store,
        page.target_id().clone(),
        document,
        input.text,
        input.form,
        input.returned,
    );
    Ok(())
}

/// Fails a baseline op once the timeout of the snapshot call it names has passed, by Rust's clock
/// and the tree op's own record (`snapshot_calls.rs`): that call's `wait_timeout`, naming the
/// `phase` it ran out in, with this op's id. Nothing the realm sends decides it. A script's abort
/// of this op comes first, as in `OpScope::timed_out`.
fn within_snapshot_call(
    scope: &OpScope<'_>,
    call: Option<u32>,
    phase: &str,
) -> Result<(), PlaywrightError> {
    if let Some(aborted) = scope.aborted_error() {
        return Err(aborted);
    }
    let passed = call.and_then(|call| {
        scope
            .run_state()
            .snapshot_calls
            .passed(call, Instant::now())
    });
    match passed {
        Some(timeout) => Err(scope.fail(
            ErrorCode::WaitTimeout,
            &format!(
                "Timeout {}ms exceeded after the snapshot was taken, while {phase}.",
                timeout.as_millis()
            ),
        )),
        None => Ok(()),
    }
}

/// What every frame of one snapshot op shares.
struct Snapshotter<'op> {
    scope: &'op OpScope<'op>,
    page: &'op OpPage<'op>,
    mode: SnapshotMode,
    /// Asked of every frame, as upstream passes its options down to each child frame.
    boxes: bool,
    /// The prelude's captured spec switch. Every descendant uses the same value so a hop or
    /// retry cannot change whether field values are masked before crossing the protocol.
    conformance: bool,
    /// Clickable marking, for `clickable` in mode `ai`, the only mode that mints refs.
    marking: Option<Marking>,
    /// When the main frame's tree read began, while it runs: what a timeout logs as the phase it
    /// cut off ([`Snapshotter::cut_off`]).
    reading: Mutex<Option<Instant>>,
}

/// One frame's `snapshot` reply, decoded at the boundary.
struct FrameReply {
    json: Vec<AriaNode>,
    iframes: Vec<SnapshotIframe>,
}

impl FrameReply {
    /// A frame whose document has no body yet. Upstream reads it as no nodes, so its iframe
    /// stays a leaf without a partial line.
    fn empty() -> Self {
        Self {
            json: Vec::new(),
            iframes: Vec::new(),
        }
    }
}

/// Why a child frame was not read: the reason its `# partial` line prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unread {
    /// Its calls missed [`FRAME_BUDGET`], as an unresponsive renderer's do.
    Timeout,
    /// The iframe, its document or its session went away during the read.
    Detached,
    /// The iframe has no frame to enter yet.
    Loading,
    /// CDP or the probe failed there in the way the code names.
    Failed(ErrorCode),
}

impl fmt::Display for Unread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => write!(f, "timeout after {} ms", FRAME_BUDGET.as_millis()),
            Self::Detached => f.write_str("frame detached"),
            Self::Loading => f.write_str("frame not loaded"),
            Self::Failed(code) => f.write_str(&code.as_str().replace('_', " ")),
        }
    }
}

/// Whether a snapshot is marked: `clickable` asks for it, and only mode `ai` mints the refs
/// marking names. Default mode ignores the key, as it prints no refs.
fn marks(mode: SnapshotMode, clickable: bool) -> bool {
    clickable && mode == SnapshotMode::Ai
}

impl Snapshotter<'_> {
    /// Each locator, pin and child frame inherits this op's rendering and conformance policy.
    fn request(&self, root: ProbeTarget, depth: Option<i64>) -> ProbeRequest {
        ProbeRequest::Snapshot {
            root,
            mode: self.mode,
            depth,
            boxes: self.boxes,
            closed_roots: 0,
            conformance: self.conformance,
            same_process_neighbour: true,
        }
    }

    /// `request` in `world`, with that frame's clickable targets and closed-root seeds when the
    /// snapshot is marked.
    /// `parent` is the world of the frame that holds its iframe, `None` for the main frame;
    /// `frame_deadline` is when a child frame's [`FRAME_BUDGET`] ends.
    ///
    /// A whole page's main frame logs its two phases, the two that grow with the page: Neo's
    /// clickable marking, then Playwright's walk of the page with the tree's transfer. An op
    /// that times out shows them in its error: one that ran out during the walk ends with
    /// `reading the main frame's tree: unfinished after N ms` ([`Snapshotter::cut_off`]), under
    /// the marking line that says how much of the timeout marking took.
    async fn call(
        &self,
        world: &IsolatedWorld,
        parent: Option<&IsolatedWorld>,
        frame_deadline: Option<Instant>,
        request: &ProbeRequest,
    ) -> Result<ProbeReply, ProbeFailure> {
        let main = parent.is_none()
            && matches!(
                request,
                ProbeRequest::Snapshot {
                    root: ProbeTarget::Document,
                    ..
                }
            );
        let feed = match &self.marking {
            Some(marking) => {
                let started = Instant::now();
                let feed = marking
                    .feed(self.scope, world, parent, frame_deadline)
                    .await;
                // A frame marking skipped has a line of its own, with its time.
                if main && let Some(feed) = &feed {
                    let seeds = match feed.seeds.len() {
                        0 => String::new(),
                        seeds => format!(", closed shadow root seeds: {seeds}"),
                    };
                    self.scope.log(format!(
                        "  clickable marking: {} ms, {} elements to mark{seeds}",
                        started.elapsed().as_millis(),
                        feed.targets.len()
                    ));
                }
                feed.unwrap_or_default()
            }
            None => Feed::default(),
        };
        let request = with_closed_roots(request, feed.seeds.len());
        let request = with_frame_context(request, || {
            self.page.same_process_neighbour(self.scope, world)
        });
        let nodes = [feed.seeds, feed.targets].concat();
        let started = Instant::now();
        if main {
            *self.reading() = Some(started);
        }
        let reply = probe::call_with_nodes(world, &request, &nodes).await;
        if main {
            *self.reading() = None;
            if matches!(reply, Ok(ProbeReply::Snapshot { .. })) {
                self.scope.log(format!(
                    "  read the main frame's tree in {} ms",
                    started.elapsed().as_millis()
                ));
            }
        }
        if let Ok(ProbeReply::Snapshot {
            marks_skipped: true,
            ..
        }) = &reply
        {
            self.scope
                .log("  clickable marking skipped a frame (page shape)".to_owned());
        }
        reply
    }

    /// The op's deadline dropped the main frame's read: logs the tree read it cut off, with how
    /// long it had run, so the timeout's error ends at the phase that ran out.
    fn cut_off(&self) {
        if let Some(started) = self.reading().take() {
            self.scope.log(format!(
                "  reading the main frame's tree: unfinished after {} ms",
                started.elapsed().as_millis()
            ));
        }
    }

    fn reading(&self) -> MutexGuard<'_, Option<Instant>> {
        self.reading.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A marked locator's snapshot, from the element its resolve pinned in `world`. A world
    /// that went away answers `detached`, so the locator tries again.
    async fn pinned(
        &self,
        world: &IsolatedWorld,
        parent: Option<&IsolatedWorld>,
        pin: Pin,
        depth: Option<i64>,
    ) -> Result<ProbeReply, PlaywrightError> {
        let request = self.request(ProbeTarget::Pin(pin), depth);
        match self.call(world, parent, None, &request).await {
            Ok(reply) => Ok(reply),
            Err(ProbeFailure::Gone(_)) => {
                self.page.frames().invalidate(world);
                Ok(ProbeReply::Detached)
            }
            Err(failure) => Err(resolve::failed(self.scope, failure)),
        }
    }

    /// One read of the main frame: its reply, or `None` to try again.
    async fn main_frame(
        &self,
        request: &ProbeRequest,
    ) -> Result<Option<FrameReply>, PlaywrightError> {
        let world = match self
            .page
            .world(&self.page.main_endpoint(), Place::Main)
            .await
        {
            Ok(world) => world,
            Err(ProbeFailure::Gone(_)) => return Ok(None),
            Err(failure) => return Err(resolve::failed(self.scope, failure)),
        };
        match self.call(&world, None, None, request).await {
            Ok(ProbeReply::Snapshot { json, iframes, .. }) => frame_reply(json, iframes)
                .map(Some)
                .map_err(|why| self.scope.fail(ErrorCode::Internal, why)),
            Ok(ProbeReply::None { stale_ref: None }) => Ok(None),
            Ok(other) => Err(resolve::refused(self.scope, other)),
            // A navigation replaced the document: the next try installs in the new one.
            Err(ProbeFailure::Gone(_)) => {
                self.page.frames().invalidate(&world);
                Ok(None)
            }
            Err(failure) => Err(resolve::failed(self.scope, failure)),
        }
    }

    /// `root` with every frame below it read and stitched, and, once every frame is read, the
    /// closed-root seeds marking found but could not send.
    async fn stitched(
        &self,
        root: FrameReply,
        depth: Option<i64>,
    ) -> Result<aria_snapshot::SnapshotResult, PlaywrightError> {
        let mut result = aria_snapshot::stitch(self.below(root, depth).await?);
        let count = |seeds: usize| u32::try_from(seeds).unwrap_or(u32::MAX);
        result.closed_roots = self
            .marking
            .as_ref()
            .and_then(Marking::unsent_seeds)
            .map(|seeds| ClosedRoots {
                sent: count(seeds.sent),
                found: count(seeds.found),
            });
        Ok(result)
    }

    /// `frame`'s tree: every iframe it rendered, read at once (upstream's `Promise.all`). The
    /// first child that withholds the snapshot drops the others. Boxed, because each child
    /// below is read the same way.
    fn below(
        &self,
        frame: FrameReply,
        depth: Option<i64>,
    ) -> BoxFuture<'_, Result<FrameSnapshot, PlaywrightError>> {
        Box::pin(async move {
            let FrameReply { json, iframes } = frame;
            let children =
                future::try_join_all(iframes.into_iter().map(|iframe| self.child(iframe, depth)))
                    .await?;
            Ok(FrameSnapshot { json, children })
        })
    }

    /// One child frame: entered and read inside [`FRAME_BUDGET`], then its own children on
    /// budgets of their own. A frame that was not read is a partial frame, so one frame never
    /// costs the rest of the page. The host's cancel and the run deadline still drop the op.
    async fn child(
        &self,
        iframe: SnapshotIframe,
        depth: Option<i64>,
    ) -> Result<ChildFrame, PlaywrightError> {
        let depth = child_depth(depth, iframe.depth);
        // A dropped read leaves nothing behind: an install interrupted inside its cell
        // leaves the cell empty for the next op, and the op's objects go with its scope.
        let deadline = Instant::now() + FRAME_BUDGET;
        let read = time::timeout_at(deadline, self.enter(&iframe.r#ref, depth, deadline))
            .await
            .unwrap_or(Err(Unread::Timeout));
        match read {
            Ok(frame) => Ok(ChildFrame::Read {
                iframe_ref: iframe.r#ref,
                snapshot: self.below(frame, depth).await?,
            }),
            Err(unread) => Ok(ChildFrame::Partial(PartialFrame {
                r#ref: iframe.r#ref,
                frame: iframe.src,
                reason: unread.to_string(),
            })),
        }
    }

    /// Enters the frame behind the iframe `iframe_ref` and reads it at its body. The hop starts in the world that printed the ref, named by its
    /// prefix (upstream jumps there the same way, `_jumpToAriaRefFrameIfNeeded`), and goes
    /// through `aria-ref=<ref>`, so it records the child's parent link like any resolver hop. A
    /// child world that went away under the read (its frame navigated since the install) is
    /// dropped and entered once more, as the resolver's liveness rule does. `deadline`, when
    /// the frame's [`FRAME_BUDGET`] ends, bounds its clickable marking too.
    async fn enter(
        &self,
        iframe_ref: &str,
        depth: Option<i64>,
        deadline: Instant,
    ) -> Result<FrameReply, Unread> {
        // The engine mints every ref this reads, so one that is not a ref is a probe bug.
        let Ok(chunk) = Chunk::aria_ref(iframe_ref) else {
            return Err(Unread::Failed(ErrorCode::Internal));
        };
        let Some(printer) = chunk.jump else {
            return Err(Unread::Failed(ErrorCode::Internal));
        };
        let request = self.request(ProbeTarget::Document, depth);
        for _ in 0..2 {
            let SeqLookup::Live { world: parent, .. } = self.page.frames().by_seq(printer.seq)
            else {
                return Err(Unread::Detached);
            };
            let hopped =
                resolve::hop(self.page, self.scope, &parent, &chunk, Strictness::Strict).await;
            let child = match hopped {
                Ok(Hopped::Into { child, .. }) => child,
                Ok(Hopped::Missing) => return Err(Unread::Loading),
                Ok(Hopped::ParentGone) => {
                    self.page.frames().invalidate(&parent);
                    return Err(Unread::Detached);
                }
                Ok(Hopped::ChildGone) => continue,
                Err(error) => {
                    return Err(Unread::Failed(
                        error.error_code().unwrap_or(ErrorCode::Internal),
                    ));
                }
            };
            match self
                .call(&child, Some(&parent), Some(deadline), &request)
                .await
            {
                Ok(ProbeReply::Snapshot { json, iframes, .. }) => {
                    return frame_reply(json, iframes)
                        .map_err(|_why| Unread::Failed(ErrorCode::Internal));
                }
                Ok(ProbeReply::None { .. }) => return Ok(FrameReply::empty()),
                Ok(other) => {
                    let refused = resolve::refused(self.scope, other);
                    return Err(Unread::Failed(
                        refused.error_code().unwrap_or(ErrorCode::Internal),
                    ));
                }
                Err(ProbeFailure::Gone(_)) => self.page.frames().invalidate(&child),
                Err(ProbeFailure::Cdp(error)) => {
                    return Err(Unread::Failed(ErrorCode::from_core(&error)));
                }
                Err(ProbeFailure::Broken(_why)) => {
                    return Err(Unread::Failed(ErrorCode::Internal));
                }
            }
        }
        Err(Unread::Detached)
    }
}

/// Decodes one frame's reply. The decode error is left out of the message, because serde quotes
/// the value it could not read, which is page text.
fn frame_reply(json: Value, iframes: Vec<SnapshotIframe>) -> Result<FrameReply, &'static str> {
    let json = serde_json::from_value(json).map_err(|_unreadable| {
        "the probe's snapshot has a node this build cannot read; rebuild \
         crates/neo-sdk/neo-playwright-js"
    })?;
    Ok(FrameReply { json, iframes })
}

/// A child frame's depth, as upstream computes it: the parent's minus the iframe's own depth
/// minus one, and none when the parent has none or zero. It can come out zero or negative;
/// the engine then renders the child whole, as upstream's does.
fn child_depth(depth: Option<i64>, iframe_depth: u32) -> Option<i64> {
    depth
        .filter(|depth| *depth != 0)
        .map(|depth| depth - i64::from(iframe_depth) - 1)
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::cell::Cell;
    use std::error::Error;

    use serde_json::json;

    use super::{LocatorInput, PageInput, Unread, child_depth, with_frame_context};
    use crate::ErrorCode;
    use crate::probe::{ProbeRequest, SnapshotMode};

    #[test]
    fn conformance_skips_even_the_lazy_frame_metadata_read() -> Result<(), Box<dyn Error>> {
        let mut request: ProbeRequest = serde_json::from_value(json!({
            "op": "snapshot", "root": "document", "mode": "ai", "conformance": true,
        }))?;
        let read = Cell::new(false);
        let contextual = with_frame_context(Cow::Borrowed(&request), || {
            read.set(true);
            false
        });
        assert!(!read.get());
        assert!(matches!(contextual, Cow::Borrowed(_)));
        let ProbeRequest::Snapshot { conformance, .. } = &mut request else {
            return Err("expected snapshot".into());
        };
        *conformance = false;
        let contextual = with_frame_context(Cow::Borrowed(&request), || {
            read.set(true);
            false
        });
        assert!(read.get());
        assert!(matches!(
            contextual.as_ref(),
            ProbeRequest::Snapshot {
                same_process_neighbour: false,
                ..
            }
        ));
        Ok(())
    }

    #[test]
    fn tests_that_a_child_frame_s_depth_follows_upstream_s_arithmetic() {
        // `options.depth ? options.depth - iframeDepths[ref] - 1 : undefined`
        assert_eq!(child_depth(None, 2), None, "no depth stays unlimited");
        assert_eq!(child_depth(Some(0), 2), None, "zero is falsy upstream");
        assert_eq!(child_depth(Some(5), 1), Some(3));
        assert_eq!(child_depth(Some(2), 1), Some(0));
        assert_eq!(
            child_depth(Some(1), 1),
            Some(-1),
            "an iframe at the limit still sends its child a depth"
        );
    }

    #[test]
    fn tests_that_partial_reasons_read_as_the_prelude_prints_them() {
        assert_eq!(Unread::Timeout.to_string(), "timeout after 2000 ms");
        assert_eq!(Unread::Detached.to_string(), "frame detached");
        assert_eq!(
            Unread::Failed(ErrorCode::TargetClosed).to_string(),
            "target closed"
        );
    }

    #[test]
    fn tests_that_snapshot_inputs_decode_as_wire_ts_declares_them() -> Result<(), Box<dyn Error>> {
        let page: PageInput = serde_json::from_value(json!({
            "page": { "$neo": "page", "tab": 1007 }, "mode": "ai", "depth": 3,
        }))?;
        assert_eq!((page.mode, page.depth), (SnapshotMode::Ai, Some(3)));
        assert!(!page.conformance, "normal page inputs mask by default");
        let locator: LocatorInput = serde_json::from_value(json!({
            "target": { "$neo": "locator", "page": { "$neo": "page", "tab": 1007 }, "chunks": [
                { "parts": [{ "name": "aria-ref", "body": "f2e5", "source": "f2e5" }] },
            ], "display": "getByRef('f2e5')" },
            "strict": "strict",
            "mode": "default",
        }))?;
        assert_eq!((locator.mode, locator.depth), (SnapshotMode::Default, None));
        assert!(
            !locator.conformance,
            "normal locator inputs mask by default"
        );
        let boxed: PageInput = serde_json::from_value(
            json!({ "page": { "$neo": "page", "tab": 1007 }, "mode": "default", "boxes": true }),
        )?;
        assert!(
            boxed.boxes,
            "upstream's boxes option reaches every frame's probe"
        );
        let upstream_page: PageInput = serde_json::from_value(json!({
            "page": { "$neo": "page", "tab": 1007 }, "mode": "ai", "conformance": true,
        }))?;
        assert!(upstream_page.conformance);
        let upstream_locator: LocatorInput = serde_json::from_value(json!({
            "target": { "$neo": "locator", "page": { "$neo": "page", "tab": 1007 }, "chunks": [
                { "parts": [{ "name": "css", "body": "body", "source": "body" }] },
            ], "display": "locator('body')" },
            "strict": "strict", "mode": "default", "conformance": true,
        }))?;
        assert!(upstream_locator.conformance);
        for bad in [
            json!({ "page": { "$neo": "page", "tab": 1007 }, "mode": "codegen" }),
            json!({ "page": { "$neo": "page", "tab": 1007 }, "mode": "ai", "depth": 1.5 }),
            json!({ "page": { "$neo": "page", "tab": 1007 }, "mode": "ai", "sameProcessNeighbour": false }),
        ] {
            assert!(
                serde_json::from_value::<PageInput>(bad.clone()).is_err(),
                "{bad} is not a page snapshot input"
            );
        }
        Ok(())
    }
}
