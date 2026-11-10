//! The snapshot ops: `page.ariaSnapshot` and `locator.ariaSnapshot`, every frame read by its
//! own probe in its own sealed world, as upstream's server reads them
//! (`ariaSnapshotJSONForFrame`, server/page.ts at v1.64.1).
//!
//! ```text
//!  root frame   snapshot { root }       the op: its own retry loop and Playwright timeout
//!     │           → json, iframes [{ ref, depth, src }], sweep (to the host at once)
//!     │         and with it SiteSweep: every frame of its site, whose values become its sweep
//!     └─ every iframe ref, all at once, each child inside its own FRAME_BUDGET:
//!          the world that printed the ref (its f<seq> prefix) ─ hop(aria-ref=<ref>)
//!            ─ the child's world ─ snapshot { root: document }, and SiteSweep with it
//!          read      → the same below it, each grandchild on a fresh budget
//!          not read  → PartialFrame { ref, frame: src, reason }; the iframe keeps its ref
//!  a frame of any read frame's site unswept → neo / unsupported, the snapshot withheld
//!  aria_snapshot::stitch: merge, the name rule, the union of every site's values masks
//!                         every string once → { json, partial }
//! ```
//!
//! Every frame read is masked with its whole site's values, not only its own document's: the
//! frame's own script can copy a value from any frame of its site, also from one no snapshot
//! reaches (a hidden iframe, one behind a closed shadow root). A snapshot only reads, so it
//! never calls `send_effect`. Default mode mints no refs and so prints no iframes to follow: it
//! reads one frame, as upstream's does.

use std::time::Duration;
use std::{fmt, mem};

use browseros_core::sensitive_fields::FieldValues;
use futures_util::future::{self, BoxFuture};
use serde::Deserialize;
use serde_json::Value;
use tokio::time;

use super::aria_snapshot::{self, AriaNode, ChildFrame, FrameSnapshot, PartialFrame};
use crate::backoff::Backoff;
use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, PAGE, TARGET};
use crate::page::OpPage;
use crate::page::site_sweep::{self, ReadMask, SiteSweep};
use crate::probe::{
    self, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget, SnapshotIframe, SnapshotMode,
};
use crate::resolve::{self, Hopped, Landed, Resolver};
use crate::target::{Chunk, PageRef, Strictness, Target};
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

/// How long one child frame's own calls may take: the hop into it, its world's install, its
/// snapshot and its site's sweep (decisions.md). A frame that misses it is reported partial and
/// the rest of the page is returned; upstream instead waits on a hung frame until the whole call
/// times out. Its children start budgets of their own, so a slow level does not eat the levels
/// below.
const FRAME_BUDGET: Duration = Duration::from_secs(2);

/// `page.ariaSnapshot`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PageInput {
    page: PageRef,
    mode: SnapshotMode,
    #[serde(default)]
    depth: Option<i64>,
}

/// `locator.ariaSnapshot`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocatorInput {
    target: Target,
    strict: Strictness,
    mode: SnapshotMode,
    #[serde(default)]
    depth: Option<i64>,
}

/// Runs one snapshot op; its output is `aria_snapshot::SnapshotResult`, `{ json, partial }`.
pub(crate) async fn invoke(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: SnapshotOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    let result = match op {
        SnapshotOp::Page => page_snapshot(pw, scope, args.decode(scope)?).await?,
        SnapshotOp::Locator => locator_snapshot(pw, scope, args.decode(scope)?).await?,
    };
    serde_json::to_value(result).map_err(|error| {
        scope.fail(
            ErrorCode::Internal,
            &format!("the snapshot did not encode: {error}"),
        )
    })
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
    let page = input.page.checked(pw.browser(), scope).await?;
    let page = OpPage::admit(pw, scope, &page).await?;
    let snapshotter = Snapshotter {
        pw,
        scope,
        page: &page,
        mode: input.mode,
    };
    let request = ProbeRequest::Snapshot {
        root: ProbeTarget::Document,
        mode: input.mode,
        depth: input.depth,
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
        pw,
        scope,
        page: &page,
        mode: input.mode,
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
        };
        let swept = |world| SiteSweep::of(&page, world);
        let Ok(attempt) = scope
            .within(resolver.attempt_alongside(request, swept))
            .await
        else {
            return Err(scope.timed_out(ErrorCode::NoMatch));
        };
        let Some(Landed {
            reply,
            world,
            alongside: site,
            ..
        }) = attempt?
        else {
            continue;
        };
        match reply {
            ProbeReply::Snapshot {
                json,
                iframes,
                sweep,
            } => {
                let reply = snapshotter
                    .frame_reply(json, iframes, sweep)
                    .map_err(|why| scope.fail(ErrorCode::Internal, why))?;
                let root = FrameRead { reply, site };
                return snapshotter.stitched(root, input.depth).await;
            }
            ProbeReply::None { stale_ref: None } | ProbeReply::Detached => {}
            other => {
                let mask = ReadMask::Site(site);
                return Err(site_sweep::refused(pw, scope, &page, world, other, mask).await);
            }
        }
    }
    Err(scope.timed_out(ErrorCode::NoMatch))
}

/// What every frame of one snapshot op shares.
struct Snapshotter<'op> {
    pw: &'op Playwright,
    scope: &'op OpScope<'op>,
    page: &'op OpPage<'op>,
    mode: SnapshotMode,
}

/// One frame's `snapshot` reply, decoded at the boundary.
struct FrameReply {
    json: Vec<AriaNode>,
    iframes: Vec<SnapshotIframe>,
    sweep: FieldValues,
}

impl FrameReply {
    /// A frame whose document has no body yet. Upstream reads it as no nodes, so its iframe
    /// stays a leaf without a partial line.
    fn empty() -> Self {
        Self {
            json: Vec::new(),
            iframes: Vec::new(),
            sweep: FieldValues::default(),
        }
    }
}

/// One frame read: its reply, and the sweep of its frame's site taken in the same world at the
/// same time. Every frame the snapshot reads comes as one, so no frame's text is stitched
/// without its site's values.
struct FrameRead {
    reply: FrameReply,
    site: SiteSweep,
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
    /// One read of the main frame, with the sweep of its site taken at the same time: its
    /// reply, or `None` to try again.
    async fn main_frame(
        &self,
        request: &ProbeRequest,
    ) -> Result<Option<FrameRead>, PlaywrightError> {
        let world = match self
            .page
            .world(&self.page.main_endpoint(), Place::Main)
            .await
        {
            Ok(world) => world,
            Err(ProbeFailure::Gone(_)) => return Ok(None),
            Err(failure) => return Err(resolve::failed(self.scope, failure)),
        };
        let (reply, site) = future::join(
            probe::call(&world, request),
            SiteSweep::of(self.page, world.clone()),
        )
        .await;
        match reply {
            Ok(ProbeReply::Snapshot {
                json,
                iframes,
                sweep,
            }) => self
                .frame_reply(json, iframes, sweep)
                .map(|reply| Some(FrameRead { reply, site }))
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

    /// Decodes one frame's reply. Its sweep goes to the host at once, while those values are
    /// on the page; the stitched tree is masked with it too. The decode error is left out of
    /// the message, because serde quotes the value it could not read, which is page text.
    fn frame_reply(
        &self,
        json: Value,
        iframes: Vec<SnapshotIframe>,
        sweep: FieldValues,
    ) -> Result<FrameReply, &'static str> {
        let json = serde_json::from_value(json).map_err(|_unreadable| {
            "the probe's snapshot has a node this build cannot read; rebuild \
             crates/neo-sdk/neo-playwright-js"
        })?;
        if !sweep.is_empty() {
            self.pw.hooks().values_swept(self.page.id().clone(), &sweep);
        }
        Ok(FrameReply {
            json,
            iframes,
            sweep,
        })
    }

    /// `root` with every frame below it read, stitched and masked. Every frame read, the root
    /// and each child, takes its whole site's values as its sweep, and the stitch masks every
    /// string and partial entry once with the union of them all: a value one frame's script
    /// copied from any frame of its site is masked in every mode, also from a frame no snapshot
    /// reads (a hidden iframe, one behind a closed shadow root). Masking once, not frame by
    /// frame, never leaves the tail of a long value after a shorter one inside it was masked.
    async fn stitched(
        &self,
        root: FrameRead,
        depth: Option<i64>,
    ) -> Result<aria_snapshot::SnapshotResult, PlaywrightError> {
        let root = self.site_masked(root)?;
        Ok(aria_snapshot::stitch(self.below(root, depth).await?))
    }

    /// `read`'s reply with its sweep widened to the values of every frame of its site, all of
    /// them told to the host. A frame of that site that could not be swept fails the whole
    /// snapshot closed (`neo / unsupported`), not as a partial frame: this frame's text, already
    /// read, may hold the unswept frame's value.
    fn site_masked(&self, read: FrameRead) -> Result<FrameReply, PlaywrightError> {
        let FrameRead { mut reply, site } = read;
        let own = mem::take(&mut reply.sweep);
        reply.sweep = site
            .values(self.pw, self.scope, self.page.id(), own)?
            .into_values();
        Ok(reply)
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
            let FrameReply {
                json,
                iframes,
                sweep,
            } = frame;
            let children =
                future::try_join_all(iframes.into_iter().map(|iframe| self.child(iframe, depth)))
                    .await?;
            Ok(FrameSnapshot {
                json,
                sweep,
                children,
            })
        })
    }

    /// One child frame: entered, read and its site swept inside [`FRAME_BUDGET`], then its own
    /// children on budgets of their own. A frame that was not read is a partial frame, so one
    /// frame never costs the rest of the page; a read frame whose site was not swept withholds
    /// the snapshot. The host's cancel and the run deadline still drop the op.
    async fn child(
        &self,
        iframe: SnapshotIframe,
        depth: Option<i64>,
    ) -> Result<ChildFrame, PlaywrightError> {
        let depth = child_depth(depth, iframe.depth);
        // A dropped read leaves nothing behind: an install interrupted inside its cell
        // leaves the cell empty for the next op, and the op's objects go with its scope. A read
        // dropped with its sweep unfinished returns none of the frame's text.
        let read = time::timeout(FRAME_BUDGET, self.enter(&iframe.r#ref, depth))
            .await
            .unwrap_or(Err(Unread::Timeout));
        match read {
            Ok(read) => {
                let frame = self.site_masked(read)?;
                Ok(ChildFrame::Read {
                    iframe_ref: iframe.r#ref,
                    snapshot: self.below(frame, depth).await?,
                })
            }
            Err(unread) => Ok(ChildFrame::Partial(PartialFrame {
                r#ref: iframe.r#ref,
                frame: iframe.src,
                reason: unread.to_string(),
            })),
        }
    }

    /// Enters the frame behind the iframe `iframe_ref` and reads it at its body, sweeping its
    /// site at the same time. The hop starts in the world that printed the ref, named by its
    /// prefix (upstream jumps there the same way, `_jumpToAriaRefFrameIfNeeded`), and goes
    /// through `aria-ref=<ref>`, so it records the child's parent link like any resolver hop. A
    /// child world that went away under the read (its frame navigated since the install) is
    /// dropped and entered once more, as the resolver's liveness rule does.
    async fn enter(&self, iframe_ref: &str, depth: Option<i64>) -> Result<FrameRead, Unread> {
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
            let (reply, site) = future::join(
                probe::call(&child, &request),
                SiteSweep::of(self.page, child.clone()),
            )
            .await;
            match reply {
                Ok(ProbeReply::Snapshot {
                    json,
                    iframes,
                    sweep,
                }) => {
                    return self
                        .frame_reply(json, iframes, sweep)
                        .map(|reply| FrameRead { reply, site })
                        .map_err(|_why| Unread::Failed(ErrorCode::Internal));
                }
                Ok(ProbeReply::None { .. }) => {
                    return Ok(FrameRead {
                        reply: FrameReply::empty(),
                        site,
                    });
                }
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
                Err(ProbeFailure::Late) => {
                    return Err(Unread::Failed(ErrorCode::Unsupported));
                }
            }
        }
        Err(Unread::Detached)
    }
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
            "page": { "$neo": "page", "id": 7, "tab": 1007 }, "mode": "ai", "depth": 3,
        }))?;
        assert_eq!((page.mode, page.depth), (SnapshotMode::Ai, Some(3)));
        let locator: LocatorInput = serde_json::from_value(json!({
            "target": { "$neo": "locator", "page": { "$neo": "page", "id": 7, "tab": 1007 }, "chunks": [
                { "parts": [{ "name": "aria-ref", "body": "f2e5", "source": "f2e5" }] },
            ], "display": "getByRef('f2e5')" },
            "strict": "strict",
            "mode": "default",
        }))?;
        assert_eq!((locator.mode, locator.depth), (SnapshotMode::Default, None));
        for bad in [
            json!({ "page": { "$neo": "page", "id": 7, "tab": 1007 }, "mode": "ai", "boxes": true }),
            json!({ "page": { "$neo": "page", "id": 7, "tab": 1007 }, "mode": "codegen" }),
            json!({ "page": { "$neo": "page", "id": 7, "tab": 1007 }, "mode": "ai", "depth": 1.5 }),
        ] {
            assert!(
                serde_json::from_value::<PageInput>(bad.clone()).is_err(),
                "{bad} is not a page snapshot input"
            );
        }
        Ok(())
    }
}
