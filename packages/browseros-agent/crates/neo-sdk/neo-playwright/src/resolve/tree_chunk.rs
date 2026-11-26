//! What one chunk of a locator names in one world, for one attempt: the switch from the page's
//! JavaScript ARIA engine to the APC tree (apc-engine-design 5.4, 5.7 and 13.5).
//!
//! The page keeps every part it can answer alone (CSS, text, label, attribute, test id, layout).
//! A chunk made only of those goes to the page as it always has, with no capture. A `role`,
//! `internal:role` or `aria-ref` part cannot run there any more: the page's role engine is a stub
//! that throws, and only Rust's ref table knows the refs a snapshot printed. So such a chunk runs
//! through [`ChunkRun`] over the attempt's capture, and the op's own request then names what it
//! found as a pinned set, which the page's `resolveTarget` and `queryAll` answer exactly as they
//! would have answered the chunk:
//!
//! ```text
//!  chunk ─ ChunkPlan::of ─ page only ─────────────────────▶ ProbeTarget::Chunk  (no capture)
//!                        └ tree parts ─ the attempt's capture (PassTree)
//!                            ─ ChunkWorld::start (design 6.2's proof for the world)
//!                            ─ ChunkRun::run ─┬ Matched(set) ─ pinned ─▶ ProbeTarget::Set
//!                                             ├ Strict, Thrown ─▶ Answered: the reply the
//!                                             │    page would have given
//!                                             └ Stale ─▶ Again: the attempt lands nowhere
//! ```
//!
//! Both engines answer one question per attempt from one source (design 7.1): every tree chunk
//! of a pass reads the pass's capture, so the names a snapshot printed and the names a locator
//! matches come from the same tree. A world installed after that capture was requested cannot
//! prove the capture's documents (6.2 case 3), so its chunk takes a second capture, once.

use std::sync::Arc;

use neo_snapshot_apc::select::{self, ChunkPlan};
use neo_snapshot_apc::tree::{DocId, NameCache, NodeId, NodeKey, SnapshotTree, TreeNode};
use serde_json::{Value, json};

use crate::PlaywrightError;
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{
    self, ErrorClass, ParsedSelector, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget,
    ResolvedSet, SetMember, StageMatch,
};
use crate::resolve::{self, ChunkOutcome, ChunkSet, First, Pinned};
use crate::target::{Chunk, Strictness};
use crate::tree::{
    AttemptStart, CapturedTree, ChunkWorld, RunAttempt, RunRoot, StartedRun, Unproven,
    describe_object_node_id, frame_doc,
};
use crate::world::{IsolatedWorld, Pin};

/// The attempt's capture, shared by every tree chunk of one resolver pass.
pub(crate) struct PassTree {
    capture: Option<Arc<CapturedTree>>,
}

impl PassTree {
    pub(crate) const fn new() -> Self {
        Self { capture: None }
    }

    /// The pass's capture when `world` was installed before it was requested, else a new one,
    /// which every later chunk of the pass reads. A new capture is requested after `world` was
    /// installed, which is what lets `ChunkWorld::start` prove the world against it.
    async fn for_world(
        &mut self,
        scope: &OpScope<'_>,
        page: &OpPage<'_>,
        world: &IsolatedWorld,
    ) -> Result<Arc<CapturedTree>, PlaywrightError> {
        if let Some(capture) = &self.capture
            && world.installed_at() < capture.requested_at()
        {
            return Ok(Arc::clone(capture));
        }
        let capture = page
            .tree()
            .capture_seeded(scope, page, AttemptStart::now())
            .await?;
        self.capture = Some(Arc::clone(&capture));
        Ok(capture)
    }
}

/// What a chunk names for the op's request.
#[derive(Debug)]
pub(crate) enum Aim {
    /// Send the request with this target: the chunk itself, or the set Rust resolved it to.
    Target(ProbeTarget),
    /// Rust decided the chunk's answer: the reply the page would have given to any request on
    /// it (a strict-mode violation, or an error the chunk throws).
    Answered(ProbeReply),
    /// A member left the page since the capture, or the world's frame shows a document the
    /// capture did not see: this attempt lands nowhere, and the op's loop tries again.
    Again,
}

/// Why aiming stopped short of an answer.
#[derive(Debug)]
pub(crate) enum AimFailure {
    /// The world went away under a call: the resolver's liveness rule, as for any request.
    Gone,
    /// What ends the op.
    Failed(PlaywrightError),
}

impl From<PlaywrightError> for AimFailure {
    fn from(error: PlaywrightError) -> Self {
        Self::Failed(error)
    }
}

/// What a chunk's request reads of its matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reads {
    /// Every match, in order: the last chunk, whose op may read all of them.
    All,
    /// How many there are, alone: `count()`'s last chunk. A last stage that ran over the tree is
    /// answered from its matches, with nothing hydrated (design 5.4).
    Count,
    /// The first match alone: a frame hop's chunk, whose one element is the iframe it enters
    /// (design 5.7), hydrated alone.
    First,
}

/// What `chunk` names in `world` for this attempt, queried from `root` (an element handle's node
/// in `world`) or from the document, under `strictness`.
#[expect(
    clippy::too_many_arguments,
    reason = "one call per chunk of a resolver pass, each argument a fact of that pass"
)]
pub(crate) async fn aim(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    pass: &mut PassTree,
    world: &IsolatedWorld,
    chunk: &Chunk,
    strictness: Strictness,
    root: Option<&str>,
    reads: Reads,
) -> Result<Aim, AimFailure> {
    let as_sent = || Ok(Aim::Target(ProbeTarget::Chunk(ParsedSelector::of(chunk))));
    let plan = match plan_of(chunk) {
        Some(Ok(plan)) => plan,
        Some(Err(answer)) => return Ok(Aim::Answered(answer)),
        // A chunk the engine's typed view does not read goes to the page as it always went:
        // the page parses every part itself, and its role stand-in throws on a role part, so a
        // tree leaf hidden in such a chunk fails loudly instead of matching wrongly.
        None => return as_sent(),
    };
    if plan.is_page_only() {
        return as_sent();
    }
    // Twice: a world installed after the pass's capture takes one capture of its own.
    for _ in 0..2 {
        let capture = pass.for_world(scope, page, world).await?;
        let run = Run {
            plan: &plan,
            strictness,
            root,
            reads,
        };
        match run_over(page, scope, &capture, world, run).await? {
            Ran::Aimed(aim) => return Ok(aim),
            Ran::WorldAfterCapture | Ran::Stale => pass.capture = None,
        }
    }
    Ok(Aim::Again)
}

/// A chunk that leads with a ref and goes on only in parts the page runs (`getByRef('e5')
/// .locator('input')`, `.and(..)`, `.filter({ hasText })`): the rest runs as one `stage` from
/// the element the ref was hydrated into (`pin`, design 6.4), with no capture. A ref's document
/// is proven by its hydration, so this holds where a capture would not read the document at all,
/// such as a frame its iframe hides (`visibility: hidden`). `None` when the chunk is not of that
/// shape: a captured part, or a tree part after the ref.
pub(crate) async fn aim_from_ref(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    world: &IsolatedWorld,
    chunk: &Chunk,
    pin: Pin,
    strictness: Strictness,
) -> Option<Result<Aim, AimFailure>> {
    let rest = rest_after_ref(chunk)?;
    let typed: select::ParsedSelector = serde_json::from_value(rest.clone()).ok()?;
    if !ChunkPlan::of(&typed).ok()?.is_page_only() {
        return None;
    }
    let selector: ParsedSelector = serde_json::from_value(rest).ok()?;
    let strict = match strictness {
        Strictness::Strict => Some(ParsedSelector::of(chunk)),
        Strictness::FirstMatch => None,
    };
    let request = ProbeRequest::Stage {
        selector,
        from: Some(vec![SetMember::Pin(pin)]),
        sets: Vec::new(),
        strict,
    };
    let reply = match probe::call(scope, page, world, &request).await {
        Ok(reply) => reply,
        Err(ProbeFailure::Gone(_)) => return Some(Err(AimFailure::Gone)),
        Err(failure) => return Some(Err(AimFailure::Failed(resolve::failed(scope, failure)))),
    };
    Some(Ok(match reply {
        ProbeReply::Staged { matches, stale_ref } => Aim::Target(ProbeTarget::Set(ResolvedSet {
            members: matches.into_iter().map(set_member).collect(),
            stale_ref,
        })),
        ProbeReply::Strict { message, .. } => Aim::Answered(ProbeReply::Strict {
            message,
            listing: None,
        }),
        ProbeReply::Error { message, class, .. } => Aim::Answered(error_reply(class, message)),
        ProbeReply::Detached => Aim::Again,
        other => Aim::Answered(error_reply(
            ErrorClass::Internal,
            format!(
                "the probe answered `{}` to a stage from a ref",
                other.kind()
            ),
        )),
    }))
}

/// `chunk`'s parts after its leading ref, as the vendored parser's JSON, when the chunk has no
/// captured part: `None` for a chunk with one part or a capture.
fn rest_after_ref(chunk: &Chunk) -> Option<Value> {
    let parsed: Value = serde_json::from_str(chunk.parsed.get()).ok()?;
    if parsed
        .get("capture")
        .is_some_and(|capture| !capture.is_null())
    {
        return None;
    }
    let parts = parsed.get("parts")?.as_array()?;
    let rest: Vec<Value> = parts.iter().skip(1).cloned().collect();
    (!rest.is_empty()).then(|| json!({ "parts": rest }))
}

/// The prelude's chunk as the engine's typed selector, planned: `None` when the typed view does
/// not read it. An engine the page lacks is upstream's selector error, as the probe's own engine
/// check words it.
fn plan_of(chunk: &Chunk) -> Option<Result<ChunkPlan, ProbeReply>> {
    let parsed: select::ParsedSelector = serde_json::from_str(chunk.parsed.get()).ok()?;
    Some(
        ChunkPlan::of(&parsed)
            .map_err(|error| error_reply(ErrorClass::Selector, error.to_string())),
    )
}

enum Ran {
    Aimed(Aim),
    /// The world was installed at or after the capture was requested.
    WorldAfterCapture,
    /// A node the capture matched left its document before it was hydrated: a fresh capture
    /// matches what is there now.
    Stale,
}

/// One chunk's run: its plan, and what its request asks of it.
struct Run<'r> {
    plan: &'r ChunkPlan,
    strictness: Strictness,
    root: Option<&'r str>,
    reads: Reads,
}

/// One run of a chunk's plan over `capture` in `world`.
async fn run_over(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    capture: &CapturedTree,
    world: &IsolatedWorld,
    chunk: Run<'_>,
) -> Result<Ran, AimFailure> {
    let Run {
        plan,
        strictness,
        root,
        reads,
    } = chunk;
    let tree = capture.tree();
    let main_frame = page.main_endpoint().frame;
    let Some(doc) = frame_doc(tree, &main_frame, world.frame()) else {
        return Ok(Ran::Aimed(Aim::Again));
    };
    let run_root = match root {
        None => RunRoot::Document,
        Some(object) => RunRoot::Handle {
            object: object.to_owned(),
            node: handle_node(scope, world, object, tree, doc).await?,
        },
    };
    let names = NameCache::new(tree);
    let attempt = RunAttempt {
        scope,
        page,
        tab: page.tree(),
        capture,
        names: &names,
    };
    let StartedRun {
        run,
        page: mut side,
    } = match ChunkWorld::start(&attempt, world.clone(), doc, run_root) {
        Ok(started) => started,
        Err(Unproven::WorldAfterCapture) => return Ok(Ran::WorldAfterCapture),
        Err(Unproven::DocumentNotInFrame) => return Ok(Ran::Aimed(Aim::Again)),
    };
    let outcome = run
        .run(plan, strictness, &mut side)
        .await
        .map_err(|failure| probe_failure(scope, failure))?;
    let matched = match outcome {
        ChunkOutcome::Matched(matched) => matched,
        ChunkOutcome::Strict(message) => {
            return Ok(Ran::Aimed(Aim::Answered(ProbeReply::Strict {
                message,
                listing: None,
            })));
        }
        ChunkOutcome::Thrown { message, class } => {
            return Ok(Ran::Aimed(Aim::Answered(error_reply(class, message))));
        }
        ChunkOutcome::Stale => return Ok(Ran::Stale),
    };
    // A stale ref stays with the set, as the page's own chunk keeps it: an op that wants one
    // element fails `element_detached` on it, a read of all counts nothing.
    let stale_ref = matched.stale_ref;
    if reads == Reads::First {
        let first = run
            .first(&matched.set, &mut side)
            .await
            .map_err(|failure| probe_failure(scope, failure))?;
        return Ok(Ran::Aimed(match first {
            First::Pin(pin) => Aim::Target(ProbeTarget::Pin(pin)),
            First::Document => Aim::Target(ProbeTarget::Document),
            First::Missing => Aim::Target(ProbeTarget::Set(ResolvedSet {
                members: Vec::new(),
                stale_ref,
            })),
            First::Stale => return Ok(Ran::Stale),
        }));
    }
    if let (Reads::Count, ChunkSet::Keys(matches)) = (reads, &matched.set) {
        let count = u32::try_from(matches.nodes().len()).unwrap_or(u32::MAX);
        return Ok(Ran::Aimed(Aim::Answered(ProbeReply::Values {
            count,
            values: Vec::new(),
        })));
    }
    let pinned = run
        .pinned(matched.set, &mut side)
        .await
        .map_err(|failure| probe_failure(scope, failure))?;
    Ok(Ran::Aimed(match pinned {
        Pinned::Members(matches) => Aim::Target(ProbeTarget::Set(ResolvedSet {
            members: matches.into_iter().map(set_member).collect(),
            stale_ref,
        })),
        Pinned::Stale => return Ok(Ran::Stale),
        Pinned::Broken(why) => Aim::Answered(error_reply(ErrorClass::Internal, why)),
    }))
}

/// The tree node of an element handle's node, when the capture has one: a plain container the
/// retention rule left out has none, and its chunk runs in the page from the handle
/// (`QueryRoot::Untracked`).
async fn handle_node(
    scope: &OpScope<'_>,
    world: &IsolatedWorld,
    object: &str,
    tree: &SnapshotTree,
    doc: DocId,
) -> Result<Option<NodeId>, AimFailure> {
    let Some(document) = tree.document(doc) else {
        return Ok(None);
    };
    let dom = describe_object_node_id(scope, world.session(), object)
        .await
        .map_err(|failure| probe_failure(scope, failure))?;
    Ok(dom.and_then(|dom| {
        tree.node_by_key(&NodeKey::new(document.token().clone(), dom))
            .map(TreeNode::id)
    }))
}

fn set_member(matched: StageMatch) -> SetMember {
    match matched {
        StageMatch::Pin(pin) => SetMember::Pin(pin),
        StageMatch::Document => SetMember::Document,
    }
}

fn error_reply(class: ErrorClass, message: String) -> ProbeReply {
    ProbeReply::Error {
        message,
        class,
        tree_depth: None,
        resolved: None,
    }
}

fn probe_failure(scope: &OpScope<'_>, failure: ProbeFailure) -> AimFailure {
    match failure {
        ProbeFailure::Gone(_) => AimFailure::Gone,
        failure => AimFailure::Failed(resolve::failed(scope, failure)),
    }
}
