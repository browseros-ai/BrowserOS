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
//! A snapshot only reads, so it never calls `send_effect`. Default mode mints no refs and so
//! prints no iframes to follow: it reads one frame, as upstream's does.

use std::fmt;
use std::time::Duration;

use futures_util::future::{self, BoxFuture};
use serde::Deserialize;
use serde_json::Value;
use tokio::time;
use ts_rs::TS;

use super::aria_snapshot::{self, AriaNode, ChildFrame, FrameSnapshot, PartialFrame};
use crate::backoff::Backoff;
use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, PAGE, TARGET};
use crate::page::OpPage;
use crate::probe::{
    self, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget, SnapshotIframe, SnapshotMode,
};
use crate::resolve::{self, Hopped, Landed, Resolver};
use crate::target::{Chunk, PageRef, Strictness, Target};
use crate::wire::{self, Describe, OpWire, Route, Run, options};
use crate::world::{Place, SeqLookup};
use crate::{ErrorCode, Playwright, PlaywrightError};

listed_enum! {
    /// Aria snapshots of a whole page (every frame stitched in) or of one element.
    pub(crate) enum SnapshotOp {
        Page,
        Locator,
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
                "the page's aria snapshot with its frames stitched in; mode 'ai' prints refs",
            ),
            Self::Locator => OpSpec::read(
                "locator.ariaSnapshot",
                TARGET,
                "an aria snapshot rooted at one element; mode 'ai' prints refs",
            ),
        }
    }
}

/// One exhaustive table binds each snapshot's handler to the upstream command. `Run` and
/// `Describe` walk these same rows; the policy in `spec()` remains independent of the shapes.
fn route<'a, R: Route<'a>>(op: SnapshotOp, r: R) -> R::Out {
    let command = wire::command("Frame.ariaSnapshot").result(
        "Rust stitches the child frames and answers the JSON tree plus the frames it could \
         not read; the prelude renders YAML with upstream's own renderer and annotates the \
         partial frames",
    );
    match op {
        SnapshotOp::Page => r.op(command, page_snapshot),
        SnapshotOp::Locator => r.op(command, locator_snapshot),
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
}
options!(LocatorInput);

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
    let snapshotter = Snapshotter {
        scope,
        page: &page,
        mode: input.mode,
        boxes: input.boxes,
    };
    let request = ProbeRequest::Snapshot {
        root: ProbeTarget::Document,
        mode: input.mode,
        depth: input.depth,
        boxes: input.boxes,
    };
    let backoff = Backoff::outer(scope.timeout());
    for attempt in 0.. {
        if scope.sleep(backoff.wait(attempt)).await.is_err() {
            return Err(scope.timed_out(ErrorCode::WaitTimeout));
        }
        let Ok(read) = scope.within(snapshotter.main_frame(&request)).await else {
            return Err(scope.timed_out(ErrorCode::WaitTimeout));
        };
        if let Some(root) = read? {
            return snapshotter.stitched(root, input.depth).await;
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
    let snapshotter = Snapshotter {
        scope,
        page: &page,
        mode: input.mode,
        boxes: input.boxes,
    };
    let resolver = Resolver::new(&page, scope, target, input.strict);
    scope.log(format!("waiting for {}", target.display));
    let backoff = Backoff::outer(scope.timeout());
    for attempt in 0.. {
        if scope.sleep(backoff.wait(attempt)).await.is_err() {
            return Err(scope.timed_out(ErrorCode::NoMatch));
        }
        let request = |last, _frame_visible| ProbeRequest::Snapshot {
            root: ProbeTarget::Chunk(last),
            mode: input.mode,
            depth: input.depth,
            boxes: input.boxes,
        };
        let Ok(attempt) = scope.within(resolver.attempt_at(request)).await else {
            return Err(scope.timed_out(ErrorCode::NoMatch));
        };
        let Some(Landed { reply, .. }) = attempt? else {
            continue;
        };
        match reply {
            ProbeReply::Snapshot { json, iframes } => {
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

/// What every frame of one snapshot op shares.
struct Snapshotter<'op> {
    scope: &'op OpScope<'op>,
    page: &'op OpPage<'op>,
    mode: SnapshotMode,
    /// Asked of every frame, as upstream passes its options down to each child frame.
    boxes: bool,
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

impl Snapshotter<'_> {
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
        match probe::call(&world, request).await {
            Ok(ProbeReply::Snapshot { json, iframes }) => frame_reply(json, iframes)
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

    /// `root` with every frame below it read and stitched.
    async fn stitched(
        &self,
        root: FrameReply,
        depth: Option<i64>,
    ) -> Result<aria_snapshot::SnapshotResult, PlaywrightError> {
        Ok(aria_snapshot::stitch(self.below(root, depth).await?))
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
        let read = time::timeout(FRAME_BUDGET, self.enter(&iframe.r#ref, depth))
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
    /// dropped and entered once more, as the resolver's liveness rule does.
    async fn enter(&self, iframe_ref: &str, depth: Option<i64>) -> Result<FrameReply, Unread> {
        // The engine mints every ref this reads, so one that is not a ref is a probe bug.
        let Ok(chunk) = Chunk::aria_ref(iframe_ref) else {
            return Err(Unread::Failed(ErrorCode::Internal));
        };
        let Some(printer) = chunk.jump else {
            return Err(Unread::Failed(ErrorCode::Internal));
        };
        let request = ProbeRequest::Snapshot {
            root: ProbeTarget::Document,
            mode: self.mode,
            depth,
            boxes: self.boxes,
        };
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
            match probe::call(&child, &request).await {
                Ok(ProbeReply::Snapshot { json, iframes }) => {
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
    use std::error::Error;

    use serde_json::json;

    use super::{LocatorInput, PageInput, Unread, child_depth};
    use crate::ErrorCode;
    use crate::probe::SnapshotMode;

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
        let locator: LocatorInput = serde_json::from_value(json!({
            "target": { "$neo": "locator", "page": { "$neo": "page", "tab": 1007 }, "chunks": [
                { "parts": [{ "name": "aria-ref", "body": "f2e5", "source": "f2e5" }] },
            ], "display": "getByRef('f2e5')" },
            "strict": "strict",
            "mode": "default",
        }))?;
        assert_eq!((locator.mode, locator.depth), (SnapshotMode::Default, None));
        let boxed: PageInput = serde_json::from_value(
            json!({ "page": { "$neo": "page", "tab": 1007 }, "mode": "default", "boxes": true }),
        )?;
        assert!(
            boxed.boxes,
            "upstream's boxes option reaches every frame's probe"
        );
        for bad in [
            json!({ "page": { "$neo": "page", "tab": 1007 }, "mode": "codegen" }),
            json!({ "page": { "$neo": "page", "tab": 1007 }, "mode": "ai", "depth": 1.5 }),
        ] {
            assert!(
                serde_json::from_value::<PageInput>(bad.clone()).is_err(),
                "{bad} is not a page snapshot input"
            );
        }
        Ok(())
    }
}
