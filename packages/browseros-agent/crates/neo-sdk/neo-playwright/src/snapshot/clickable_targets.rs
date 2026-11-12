//! Clickable marking for hosted Neo's aria snapshot: elements Chromium reports clickable without
//! a pointer cursor (a `div` with only a click listener, a `tabindex` card, an image tile) keep
//! their refs in the prelude's condensed format, which prints refs only on what the agent can act
//! on.
//!
//! ```text
//!  each frame the snapshot reads, BEFORE its snapshot call: its isolated world, and where its
//!  document sits in its session's DOM capture (the session's root, or a same-process child's
//!  document node)
//!    ├─ browseros-core find_clickables: one DOMSnapshot.captureSnapshot { computedStyles:
//!    │    [cursor] } per CDP session (DomSnapshots), desktop Neo's rules over the document
//!    ├─ the hits that are not pointer-cursor hits (the engine already treats those as targets)
//!    └─ DOM.resolveNode { backendNodeId, executionContextId: that frame's isolated world }
//!  then that frame's `snapshot` probe call carries the resolved nodes after its root: for that
//!  one call the engine reads them as `cursor: pointer` (probe/pointer-targets.ts), so its own ai
//!  rules print, keep and fold them as they do the page's own pointer-cursor elements.
//! ```
//!
//! Nothing runs in, or is written to, the page's own world: the capture is Chromium's, and the
//! only script is the engine's own snapshot in Neo's isolated world. No call here turns `Runtime`
//! on. Desktop's rules are called, not copied, so both products pick the same elements.
//!
//! Best effort, by design: a frame whose capture, document or nodes cannot be read is snapshotted
//! without targets, and the op's call log says so; it never fails the snapshot. Each frame's read
//! has its own budget ([`frame_budget`]), so a slow frame costs only its own marks, and never more
//! than half of what is left of the op's timeout (and of a child frame's own budget), so the
//! snapshot itself keeps time to finish. A session whose read ran out of time is not read again
//! in the same attempt: a capture cut off by its frame's budget would otherwise start over for
//! each frame of that session.

use std::collections::HashSet;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use browseros_cdp::CdpError;
use browseros_core::CoreError;
use browseros_core::observer::{self, ClickableDocument, DomSnapshots, SnapshotBudget};
use browseros_core::snapshot::FrameReadReason;
use futures_util::stream::{self, StreamExt, TryStreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::time::{self, Instant};

use crate::op_scope::OpScope;
use crate::probe::ProbeFailure;
use crate::world::IsolatedWorld;

/// How long one frame's read may take at most. It measured 6 to 85 ms on real pages (85 ms on a
/// 15,000-node product page); this leaves room for a loaded machine and caps what a pathological
/// page adds to a snapshot.
const MARK_BUDGET: Duration = Duration::from_secs(1);

/// `DOM.resolveNode` calls in flight per frame, as many as core's own snapshot budget allows.
const RESOLVES_IN_FLIGHT: usize = 8;

/// The reason core's rules give a hit for its computed `cursor: pointer` (`find_clickables`).
/// The engine already treats those as click targets itself, so only the other hits are passed.
const POINTER_REASON: &str = "cursor:pointer";

/// One snapshot attempt's marking: the DOM captures its frames share, one per CDP session,
/// core's bound on their reads in flight, and the sessions whose read ran out of time.
pub(super) struct Marking {
    snapshots: DomSnapshots,
    budget: SnapshotBudget,
    /// By session id (`None` for the browser's own session), never locked across an await.
    timed_out: Mutex<HashSet<Option<String>>>,
}

/// Why a frame was snapshotted without its targets, as the call log words it.
enum Skipped {
    /// Core's capture, or the frame's document in it, could not be read.
    Read(FrameReadReason),
    /// A hit's node could not be resolved into the frame's world, other than by being gone.
    Resolve,
    /// The frame's read missed its budget.
    Timeout,
}

/// `DOM.resolveNode`'s answer.
#[derive(Deserialize)]
struct Resolved {
    object: ResolvedObject,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolvedObject {
    object_id: String,
}

/// `DOM.getFrameOwner`'s answer: the `<iframe>` in the parent document.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameOwner {
    backend_node_id: i64,
}

/// `DOM.describeNode`'s answer for that `<iframe>`, as far as its content document goes.
#[derive(Deserialize)]
struct Described {
    node: DescribedNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescribedNode {
    #[serde(default)]
    content_document: Option<ContentDocument>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContentDocument {
    backend_node_id: i64,
}

impl Marking {
    pub(super) fn new() -> Self {
        Self {
            snapshots: DomSnapshots::default(),
            budget: SnapshotBudget::new(),
            timed_out: Mutex::default(),
        }
    }

    /// The elements of `world`'s frame to pass to its snapshot call, as remote objects of that
    /// world in the op's object group. `parent` is the world of the frame that holds its iframe,
    /// `None` for the main frame; `frame_deadline` is when a child frame's own budget ends.
    /// Empty when the frame has none or could not be read; the latter is noted on the op's call
    /// log.
    pub(super) async fn targets(
        &self,
        scope: &OpScope<'_>,
        world: &IsolatedWorld,
        parent: Option<&IsolatedWorld>,
        frame_deadline: Option<Instant>,
    ) -> Vec<String> {
        let session = world.session_id().map(|id| id.0.clone());
        let timed_out = || {
            self.timed_out
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
        };
        let read = if timed_out().contains(&session) {
            Err(Skipped::Timeout)
        } else {
            let budget = frame_budget(scope, frame_deadline);
            let read = time::timeout(budget, self.read(scope, world, parent))
                .await
                .unwrap_or(Err(Skipped::Timeout));
            if matches!(read, Err(Skipped::Timeout)) {
                timed_out().insert(session);
            }
            read
        };
        match read {
            Ok(targets) => targets,
            Err(skipped) => {
                scope.log(format!(
                    "  clickable marking skipped a frame ({})",
                    skipped.words()
                ));
                Vec::new()
            }
        }
    }

    async fn read(
        &self,
        scope: &OpScope<'_>,
        world: &IsolatedWorld,
        parent: Option<&IsolatedWorld>,
    ) -> Result<Vec<String>, Skipped> {
        let document = capture_document(world, parent).await;
        let hits =
            observer::find_clickables(&self.snapshots, world.session(), document, &self.budget)
                .await
                .map_err(Skipped::Read)?;
        let nodes = hits
            .into_iter()
            .filter(|(_, reasons)| !reasons.iter().any(|reason| reason == POINTER_REASON))
            .map(|(node, _)| node);
        let resolved: Vec<Option<String>> = stream::iter(nodes)
            .map(|node| resolve(scope, world, node))
            .buffer_unordered(RESOLVES_IN_FLIGHT)
            .try_collect()
            .await?;
        // A node that left its document since the capture resolved to nothing and is skipped.
        Ok(resolved.into_iter().flatten().collect())
    }
}

/// One frame's read budget: [`MARK_BUDGET`], or half of what is left of the op's timeout, or of
/// a child frame's own budget, when that is less, so the frame's snapshot keeps time to finish.
fn frame_budget(scope: &OpScope<'_>, frame_deadline: Option<Instant>) -> Duration {
    let now = Instant::now();
    let left = |deadline: Instant| deadline.saturating_duration_since(now);
    budget_within([scope.deadline().map(left), frame_deadline.map(left)])
}

/// [`frame_budget`] for what is `left` of each bound (`None`: no such bound).
fn budget_within(left: [Option<Duration>; 2]) -> Duration {
    left.into_iter()
        .flatten()
        .fold(MARK_BUDGET, |budget, left| budget.min(left / 2))
}

impl Skipped {
    fn words(&self) -> &'static str {
        match self {
            Self::Read(FrameReadReason::Timeout) | Self::Timeout => "timeout",
            Self::Read(FrameReadReason::Detached) => "frame detached",
            Self::Read(FrameReadReason::Refused) => "refused",
            Self::Read(FrameReadReason::NotFound) => "document not found",
            Self::Read(FrameReadReason::BrowserError) => "browser error",
            Self::Resolve => "node resolve failed",
        }
    }
}

/// Which document of its session's capture is `world`'s. The main frame and an out-of-process
/// iframe are their session's root document. A same-process child shares its parent's session,
/// so its capture holds the parent's documents too, and its own is named by the document node
/// inside its `<iframe>` (core's frame acquisition picks it the same way).
async fn capture_document(
    world: &IsolatedWorld,
    parent: Option<&IsolatedWorld>,
) -> ClickableDocument {
    let Some(parent) = parent else {
        return ClickableDocument::SessionDefault;
    };
    if !parent.session().same_session(world.session()) {
        return ClickableDocument::SessionDefault;
    }
    let session = world.session();
    let owner = session
        .send::<_, FrameOwner>("DOM.getFrameOwner", json!({ "frameId": world.frame() }))
        .await;
    let Ok(owner) = owner else {
        return ClickableDocument::Unavailable;
    };
    let described = session
        .send::<_, Described>(
            "DOM.describeNode",
            json!({ "backendNodeId": owner.backend_node_id, "depth": 1 }),
        )
        .await;
    match described.map(|described| described.node.content_document) {
        Ok(Some(document)) => ClickableDocument::BackendNode(document.backend_node_id),
        _ => ClickableDocument::Unavailable,
    }
}

/// The node as a remote object in `world`, in the op's object group (released when the op
/// ends), or `None` when the browser names no such node in that document any more (removed
/// since the capture), as `HandleSite` reads the same answer. A context id a later document took
/// over fails the snapshot call instead (`call_with_nodes`), before anything runs.
async fn resolve(
    scope: &OpScope<'_>,
    world: &IsolatedWorld,
    node: i64,
) -> Result<Option<String>, Skipped> {
    let params = json!({
        "backendNodeId": node,
        "executionContextId": world.context(),
        "objectGroup": scope.object_group(world.session()),
    });
    let resolved = world
        .session()
        .send::<_, Resolved>("DOM.resolveNode", params)
        .await;
    match resolved {
        Ok(resolved) => Ok(Some(resolved.object.object_id)),
        Err(error) => match ProbeFailure::from_core(error) {
            ProbeFailure::Gone(_) => Err(Skipped::Read(FrameReadReason::Detached)),
            // "No node with given id found": the backend id names nothing here any more.
            ProbeFailure::Cdp(CoreError::Cdp(CdpError::Protocol { .. })) => Ok(None),
            ProbeFailure::Cdp(_) | ProbeFailure::Broken(_) => Err(Skipped::Resolve),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{MARK_BUDGET, budget_within};

    #[test]
    fn tests_that_a_frame_read_never_takes_more_than_half_of_what_is_left() {
        let ms = Duration::from_millis;
        assert_eq!(budget_within([None, None]), MARK_BUDGET, "no bound at all");
        assert_eq!(
            budget_within([Some(Duration::from_secs(30)), None]),
            MARK_BUDGET
        );
        assert_eq!(
            budget_within([Some(ms(100)), None]),
            ms(50),
            "a short op keeps half its time for the snapshot itself"
        );
        assert_eq!(
            budget_within([Some(Duration::from_secs(30)), Some(ms(600))]),
            ms(300),
            "a child frame entered late keeps half of its own budget for its snapshot"
        );
        assert_eq!(budget_within([Some(Duration::ZERO), None]), Duration::ZERO);
    }
}
