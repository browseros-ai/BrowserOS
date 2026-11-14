//! Clickable marking for hosted Neo's aria snapshot: elements Chromium reports clickable without
//! a pointer cursor (a `div` with only a click listener, a `tabindex` card, an image tile) keep
//! their refs in the prelude's condensed format, which prints refs only on what the agent can act
//! on. The same read opens closed shadow roots to Neo's engine (a Cloudflare Turnstile widget
//! sits in one), which no script can enter from the host.
//!
//! ```text
//!  each frame the snapshot reads, BEFORE its snapshot call: its isolated world, and where its
//!  document sits in its session's DOM capture (the session's root, or a same-process child's
//!  document node)
//!    ├─ browseros-core find_clickables: one DOMSnapshot.captureSnapshot { computedStyles:
//!    │    [cursor] } per CDP session (DomSnapshots), desktop Neo's rules over the document
//!    ├─ the hits that are not pointer-cursor hits (the engine already treats those as targets)
//!    ├─ browseros-core find_closed_shadow_seeds over the same capture: elements inside closed
//!    │    roots, at least one per root, the outermost first, at most SEED_CAP of them
//!    └─ DOM.resolveNode { backendNodeId, executionContextId: that frame's isolated world }
//!  then that frame's `snapshot` probe call carries the resolved seeds, then the targets, after
//!  its root. The probe takes each seed's root (`getRootNode()`) and from then on its world's
//!  engine enters that root as an open one (probe/shadow-roots.ts); for that one call the engine
//!  reads the targets as `cursor: pointer` (probe/pointer-targets.ts), so its own ai rules print,
//!  keep and fold them as they do the page's own pointer-cursor elements.
//! ```
//!
//! Nothing runs in, or is written to, the page's own world: the capture is Chromium's, and the
//! only script is the engine's own snapshot in Neo's isolated world. No call here turns `Runtime`
//! on. Desktop's rules are called, not copied, so both products pick the same elements.
//!
//! The camera uses the same capture and rules once per document, outside any op
//! ([`closed_root_seeds`], protected-fill ruling R45's closed-root limit): it hands the seeds to
//! its own `privateShown` question, which keeps the roots in a map of its own and never opens
//! them to the engine, so a snapshot's marking and the camera's walk do not see each other. Its
//! captures are shared by the frames of one camera check ([`CameraCaptures`]), its resolves live
//! in an object group of the walk's own, and its budget is the camera's (`picture_check.rs`).
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
use std::task::Poll;
use std::time::Duration;
use std::{future, pin};

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

/// Seeds of closed shadow roots resolved per frame at most. Core gives one per root whose host is
/// not inside a closed root, first, then one per element inside a closed root that could host
/// another (a root nested in a closed root cannot be told from an ordinary element there), so a
/// page of many closed components needs one seed each. A cut is not silent: the snapshot's text
/// says how many were sent of how many found (`SeedCount`), and the op's call log names the frame.
/// Set from the cost of one seed against [`MARK_BUDGET`]: resolved eight at a time, a seed costs
/// about 50 µs (measured 2026-11-12 on pages of closed cards: 100 seeds add 6 ms to marking, 999
/// add 52 ms), so 1000 take about a twentieth of the frame's second, and a few times that on a
/// loaded machine, beside a capture that took up to 950 ms on the largest page measured.
const SEED_CAP: usize = 1000;

/// One snapshot attempt's marking: the DOM captures its frames share, one per CDP session,
/// core's bound on their reads in flight, the sessions whose read ran out of time, and the
/// closed-root seeds of every frame it read.
pub(super) struct Marking {
    snapshots: DomSnapshots,
    budget: SnapshotBudget,
    /// By session id (`None` for the browser's own session), never locked across an await.
    timed_out: Mutex<HashSet<Option<String>>>,
    /// Never locked across an await.
    seeds: Mutex<SeedCount>,
}

/// Closed-root seeds over every frame of one snapshot attempt: found in the frames' captures, and
/// sent to their probes. A frame counts its seeds as found once its capture is read, and as sent
/// only once each is resolved into its world, so a frame whose read failed or ran out of time
/// after that, a seed removed from the page since the capture, or seeds the cap cut leave `sent`
/// short: some closed roots may be unread, and the snapshot's text says so.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct SeedCount {
    pub(super) found: usize,
    pub(super) sent: usize,
}

/// What one frame's read hands its snapshot call, as remote objects of that frame's world in the
/// op's object group: the call carries the seeds first, then the targets.
#[derive(Default)]
pub(super) struct Feed {
    /// Nodes inside closed shadow roots, at least one per root the cap kept.
    pub(super) seeds: Vec<String>,
    /// Elements Chromium reports clickable without a pointer cursor.
    pub(super) targets: Vec<String>,
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
            seeds: Mutex::default(),
        }
    }

    /// The attempt's seeds when some were found that never reached a probe; `None` when every
    /// one did, or none was found.
    pub(super) fn unsent_seeds(&self) -> Option<SeedCount> {
        let count = *self.seeds.lock().unwrap_or_else(PoisonError::into_inner);
        (count.sent < count.found).then_some(count)
    }

    fn count_seeds(&self, change: impl FnOnce(&mut SeedCount)) {
        change(&mut self.seeds.lock().unwrap_or_else(PoisonError::into_inner));
    }

    /// What `world`'s frame passes to its snapshot call. `parent` is the world of the frame that
    /// holds its iframe, `None` for the main frame; `frame_deadline` is when a child frame's own
    /// budget ends. Empty when the frame has nothing to pass; `None` when it could not be read,
    /// which is noted, with the time it took, on the op's call log. One read, one budget: a
    /// frame that misses it passes neither its targets nor its seeds.
    pub(super) async fn feed(
        &self,
        scope: &OpScope<'_>,
        world: &IsolatedWorld,
        parent: Option<&IsolatedWorld>,
        frame_deadline: Option<Instant>,
    ) -> Option<Feed> {
        let session = world.session_id().map(|id| id.0.clone());
        let timed_out = || {
            self.timed_out
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
        };
        let started = Instant::now();
        let read = if timed_out().contains(&session) {
            Err(Skipped::Timeout)
        } else {
            let read = within(
                started + frame_budget(scope, frame_deadline),
                self.read(scope, world, parent),
            )
            .await;
            if matches!(read, Err(Skipped::Timeout)) {
                timed_out().insert(session);
            }
            read
        };
        match read {
            Ok(feed) => Some(feed),
            Err(skipped) => {
                scope.log(format!(
                    "  clickable marking skipped a frame ({}) after {} ms",
                    skipped.words(),
                    started.elapsed().as_millis()
                ));
                None
            }
        }
    }

    async fn read(
        &self,
        scope: &OpScope<'_>,
        world: &IsolatedWorld,
        parent: Option<&IsolatedWorld>,
    ) -> Result<Feed, Skipped> {
        let shares_parent_session =
            parent.is_some_and(|parent| parent.session().same_session(world.session()));
        let document = capture_document(world, shares_parent_session).await;
        let session = world.session();
        let hits = observer::find_clickables(&self.snapshots, session, document, &self.budget)
            .await
            .map_err(Skipped::Read)?;
        // The capture is cached for the attempt, so this reads the same one again.
        let mut seeds =
            observer::find_closed_shadow_seeds(&self.snapshots, session, document, &self.budget)
                .await
                .map_err(Skipped::Read)?;
        self.count_seeds(|count| count.found += seeds.len());
        if seeds.len() > SEED_CAP {
            scope.log(format!(
                "  closed shadow roots: a frame has {} seeds, the first {SEED_CAP} sent",
                seeds.len()
            ));
            seeds.truncate(SEED_CAP);
        }
        let targets = hits
            .into_iter()
            .filter(|(_, reasons)| !reasons.iter().any(|reason| reason == POINTER_REASON))
            .map(|(node, _)| (Fed::Target, node));
        let nodes = seeds
            .into_iter()
            .map(|node| (Fed::Seed, node))
            .chain(targets);
        let group = scope.object_group(world.session());
        let resolved: Vec<(Fed, Option<String>)> = stream::iter(nodes)
            .map(|(fed, node)| async move {
                resolve(group, world, node)
                    .await
                    .map(|object| (fed, object))
            })
            .buffer_unordered(RESOLVES_IN_FLIGHT)
            .try_collect()
            .await?;
        let mut feed = Feed::default();
        // A node that left its document since the capture resolved to nothing and is skipped. A
        // seed that did is not sent: its root may still hold the rest of what it held.
        for (fed, object) in resolved {
            let Some(object) = object else { continue };
            match fed {
                Fed::Seed => feed.seeds.push(object),
                Fed::Target => feed.targets.push(object),
            }
        }
        self.count_seeds(|count| count.sent += feed.seeds.len());
        Ok(feed)
    }
}

/// The DOM captures one camera check shares, one per CDP session, and core's bound on their
/// reads in flight: frames of one renderer share a session, and its capture holds all their
/// documents, so a check captures each session once however many of its frames it walks.
pub(crate) struct CameraCaptures {
    snapshots: DomSnapshots,
    budget: SnapshotBudget,
}

impl CameraCaptures {
    pub(crate) fn new() -> Self {
        Self {
            snapshots: DomSnapshots::default(),
            budget: SnapshotBudget::new(),
        }
    }
}

/// The camera's walk of one document's closed shadow roots (protected-fill design, the camera's
/// closed-root limit): the elements a marked snapshot would hand as seeds, from the same capture,
/// the same rules (`find_closed_shadow_seeds`) and the same cap ([`SEED_CAP`]), resolved into
/// `world` in the camera's object group `group`. No op runs, so nothing goes to a call log: an
/// `Err` names why for the camera's own note. Bounded by `deadline` as marking's read is
/// ([`within`]): a capture that comes back after it is dropped as it arrives.
pub(crate) async fn closed_root_seeds(
    captures: &CameraCaptures,
    world: &IsolatedWorld,
    shares_parent_session: bool,
    group: &str,
    deadline: Instant,
) -> Result<Vec<String>, &'static str> {
    let read = async {
        let document = capture_document(world, shares_parent_session).await;
        let mut seeds = observer::find_closed_shadow_seeds(
            &captures.snapshots,
            world.session(),
            document,
            &captures.budget,
        )
        .await
        .map_err(Skipped::Read)?;
        seeds.truncate(SEED_CAP);
        let resolved: Vec<Option<String>> = stream::iter(seeds)
            .map(|node| resolve(group, world, node))
            .buffer_unordered(RESOLVES_IN_FLIGHT)
            .try_collect()
            .await?;
        Ok(resolved.into_iter().flatten().collect())
    };
    within(deadline, read)
        .await
        .map_err(|skipped| skipped.words())
}

/// Which list of the [`Feed`] a resolved node goes to.
#[derive(Clone, Copy)]
enum Fed {
    Seed,
    Target,
}

/// `read`'s answer, unless `deadline` passes first; then `Skipped::Timeout`, and `read` is
/// dropped where it stands. The deadline is asked before `read` on every poll, so a read whose
/// budget is already spent sends no capture at all, and a capture that comes back after its
/// budget is dropped as it arrives. `time::timeout` asks its future first: a late capture was
/// still decoded and run through core's rules (about 0.1 s for the 12.8 MB capture of a
/// 75,000-element page) and its first resolves sent before the timer was asked. A capture
/// already sent cannot be called back: Chromium finishes it, and the frame's tree walk queues
/// behind it in the page, so dropping what comes back is all a deadline can still save.
///
/// The clock is read on every poll, not only the timer: tokio reports a timer as expired only
/// once its driver has turned, so a read polled in between would still run. The timer stays
/// for the wake-up at the deadline.
async fn within<T>(
    deadline: Instant,
    read: impl Future<Output = Result<T, Skipped>>,
) -> Result<T, Skipped> {
    let mut timer = pin::pin!(time::sleep_until(deadline));
    let mut read = pin::pin!(read);
    future::poll_fn(|cx| {
        if Instant::now() >= deadline || timer.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(Skipped::Timeout));
        }
        read.as_mut().poll(cx)
    })
    .await
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
/// iframe are their session's root document. A same-process child shares its parent's session
/// (`shares_parent_session`), so its capture holds the parent's documents too, and its own is
/// named by the document node inside its `<iframe>` (core's frame acquisition picks it the same
/// way).
async fn capture_document(world: &IsolatedWorld, shares_parent_session: bool) -> ClickableDocument {
    if !shares_parent_session {
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

/// The node as a remote object in `world`, in object group `group` (the op's, released when the
/// op ends, or the camera's own), or `None` when the browser names no such node in that document
/// any more (removed since the capture), as `HandleSite` reads the same answer. A context id a
/// later document took over fails the probe call instead (`send`), before anything runs.
async fn resolve(group: &str, world: &IsolatedWorld, node: i64) -> Result<Option<String>, Skipped> {
    let params = json!({
        "backendNodeId": node,
        "executionContextId": world.context(),
        "objectGroup": group,
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use tokio::time::{self, Instant};

    use super::{MARK_BUDGET, Skipped, budget_within, within};

    #[tokio::test(start_paused = true)]
    async fn tests_that_a_spent_budget_drops_the_read_before_it_runs_and_a_late_one_as_it_lands() {
        let polled = AtomicBool::new(false);
        let read = async {
            polled.store(true, Ordering::SeqCst);
            Ok::<_, Skipped>(())
        };
        let spent = within(Instant::now() - Duration::from_millis(1), read).await;
        assert!(matches!(spent, Err(Skipped::Timeout)));
        assert!(
            !polled.load(Ordering::SeqCst),
            "no budget left: the read never starts, so no capture is sent"
        );
        let deadline = Instant::now() + Duration::from_millis(100);
        let late = within(deadline, async {
            time::sleep_until(deadline + Duration::from_millis(1)).await;
            Ok::<_, Skipped>(())
        });
        assert!(matches!(late.await, Err(Skipped::Timeout)));
        let ready = within(Instant::now() + Duration::from_millis(100), async {
            time::sleep(Duration::from_millis(10)).await;
            Ok::<_, Skipped>(7)
        });
        assert!(
            matches!(ready.await, Ok(7)),
            "a read inside its budget answers"
        );
    }

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
