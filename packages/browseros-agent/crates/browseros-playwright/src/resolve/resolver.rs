//! From a locator to the world its last chunk runs in, through every frame on the way.

use std::future::Future;

use browseros_core::FrameId;
use futures_util::future;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{
    self, ErrorClass, ParsedSelector, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget,
    QueryState, Read, Want,
};
use crate::resolve::frame_start::{FrameStart, Started};
use crate::resolve::{self, FramePath, HandleSite, Hop, HopIframe};
use crate::target::{AriaRefJump, Chunk, LocatorTarget, RootHandle, Strictness};
use crate::world::{ParentLink, Pin, Place, SealedWorld, SeqLookup};
use crate::{ErrorCode, PlaywrightError};

/// Resolves one locator for one op. It never waits: "not there yet" is an attempt that lands
/// nowhere (`None`), and the op's retry loop owns the timing.
pub(crate) struct Resolver<'a> {
    page: &'a OpPage<'a>,
    scope: &'a OpScope<'a>,
    target: &'a LocatorTarget,
    /// Applies to every frame hop and to the last chunk, as upstream's `info.strict` does.
    strictness: Strictness,
    /// The child frame the locator was built from (`frame.locator(..)`), where its first chunk
    /// runs unless that chunk jumps to a ref; `None` starts in the main frame.
    start: Option<FrameStart>,
}

/// A reply, and where the request that got it ran: what an op needs when it goes on working
/// in that world (making a handle of a pinned element) or above it (placing a box in the main
/// viewport). `alongside` is what [`Resolver::attempt_alongside`] ran in that world at the same
/// time as the request, such as a read's site sweep; none for [`Resolver::attempt_at`].
#[derive(Debug)]
pub(crate) struct Landed<T = ()> {
    pub(crate) reply: ProbeReply,
    pub(crate) world: SealedWorld,
    pub(crate) path: FramePath,
    pub(crate) alongside: T,
}

/// The frames one pass went through so far: their parents, top down, and whether every
/// `<iframe>` on the way is visible (upstream's `frameVisible`).
#[derive(Debug)]
struct Way {
    path: FramePath,
    frame_visible: bool,
    /// The element handle's node (a remote object in the current world) the next chunk is
    /// queried from: set by a rooted start, gone after the first hop or a jump.
    root: Option<String>,
}

impl Way {
    /// The main frame: no parents, and nothing on the way to hide it.
    fn top() -> Self {
        Self {
            path: FramePath::default(),
            frame_visible: true,
            root: None,
        }
    }
}

/// Where the last chunk runs.
#[derive(Debug)]
struct Resolved {
    world: SealedWorld,
    /// The parents of `world`'s frame, from the main frame down.
    path: FramePath,
    /// Every `<iframe>` on the way is visible (upstream's `frameVisible`).
    frame_visible: bool,
    /// `world` is where a ref jump landed: if it is gone, so is the ref's document.
    jumped: Option<AriaRefJump>,
    /// The handle's node the last chunk is queried from, when it is also the first.
    root: Option<String>,
}

/// One pass over the chunks.
enum Step {
    Ready(Resolved),
    Missing,
    /// A world on the way turned out gone; it is invalidated (when there is one) and the pass
    /// runs again.
    Gone(Option<SealedWorld>),
}

/// An `<iframe>` a hop found in its parent's world: its remote object (in the op's group), its
/// pin, its key in the parent's ref map and whether it is visible there.
pub(super) struct Iframe {
    pub(super) object: String,
    pub(super) pin: Pin,
    pub(super) key: String,
    pub(super) visible: bool,
}

/// One frame hop.
#[expect(
    clippy::large_enum_variant,
    reason = "one value per hop, moved once into the path; boxing would only add an allocation"
)]
pub(crate) enum Hopped {
    Into {
        child: SealedWorld,
        hop: Hop,
        visible: bool,
    },
    /// The iframe, or its content frame, is not there yet.
    Missing,
    /// The parent's document went away under the hop.
    ParentGone,
    /// The child frame's session or document went away while it was being entered.
    ChildGone,
}

/// `DOM.describeNode`'s answer for an `<iframe>`: its content frame, and whether that frame's
/// document lives in this process (same-process children carry their `contentDocument`).
#[derive(Deserialize)]
struct Described {
    node: DescribedNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescribedNode {
    #[serde(default)]
    frame_id: Option<String>,
    #[serde(default)]
    content_document: Option<Value>,
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(
        page: &'a OpPage<'a>,
        scope: &'a OpScope<'a>,
        target: &'a LocatorTarget,
        strictness: Strictness,
    ) -> Self {
        let main = page.main_endpoint().frame;
        let start = target
            .frame
            .as_ref()
            .filter(|frame| **frame != main)
            .map(|frame| FrameStart::new(frame.clone()));
        Self {
            page,
            scope,
            target,
            strictness,
            start,
        }
    }

    /// Resolves the target and sends the op's request, built by `request` from the last chunk
    /// and whether every frame on the way is visible, in the last chunk's world. The reply comes
    /// back with that world and its parents; `None` is "a frame on the way is not there yet".
    ///
    /// The liveness rule: a world that turns out gone (a stale handle, a lost session, an
    /// `uninstalled` probe) is invalidated, and the whole pass runs once more, installing
    /// fresh worlds where documents changed; if that fails too, the frame is mid-navigation
    /// and the attempt lands nowhere. A world a ref jump landed in is the exception: its
    /// document is the ref's, so the op fails at once with `element_detached`.
    pub(crate) async fn attempt_at(
        &self,
        request: impl Fn(ParsedSelector, bool) -> ProbeRequest,
    ) -> Result<Option<Landed>, PlaywrightError> {
        self.attempt_alongside(request, |_world| future::ready(()))
            .await
    }

    /// [`Resolver::attempt_at`], with `alongside` run on the world the request lands in, at the
    /// same time as the request, so a read's sweep of its frame's site (`page::SiteSweep`)
    /// overlaps the read instead of following it. Only the output of the pass whose request was
    /// answered is kept.
    pub(crate) async fn attempt_alongside<T, Fut>(
        &self,
        request: impl Fn(ParsedSelector, bool) -> ProbeRequest,
        alongside: impl Fn(SealedWorld) -> Fut,
    ) -> Result<Option<Landed<T>>, PlaywrightError>
    where
        Fut: Future<Output = T>,
    {
        for _ in 0..2 {
            let resolved = match self.resolve().await? {
                Step::Ready(resolved) => resolved,
                Step::Missing => return Ok(None),
                Step::Gone(world) => {
                    if let Some(world) = world {
                        self.page.frames().invalidate(&world);
                    }
                    continue;
                }
            };
            let last = ParsedSelector::of(self.target.chunks.last());
            let request = request(last, resolved.frame_visible);
            let (called, beside) = future::join(
                call_from(&resolved.world, &request, resolved.root.as_deref()),
                alongside(resolved.world.clone()),
            )
            .await;
            match called {
                Ok(reply) => {
                    return Ok(Some(Landed {
                        reply,
                        world: resolved.world,
                        path: resolved.path,
                        alongside: beside,
                    }));
                }
                Err(ProbeFailure::Gone) => {
                    self.page.frames().invalidate(&resolved.world);
                    if let Some(jump) = resolved.jumped {
                        return Err(self.detached(&jump.to_string()));
                    }
                }
                Err(failure) => return Err(self.failed(failure)),
            }
        }
        Ok(None)
    }

    /// A reply that ends the op: a ref that can never match again, a strict violation, an
    /// error thrown in the page, or a reply the request does not take.
    pub(crate) fn refused(&self, reply: ProbeReply) -> PlaywrightError {
        refused(self.scope, reply)
    }

    /// One pass: the start world, then one hop per chunk but the last. A chunk that leads
    /// with a ref jumps to the document that printed it first, as upstream's
    /// `_jumpToAriaRefFrameIfNeeded` does, so no start world is needed for it.
    async fn resolve(&self) -> Result<Step, PlaywrightError> {
        let mut world: Option<SealedWorld> = None;
        let mut way = Way::top();
        let mut jumped = None;
        for chunk in self.target.chunks.hops() {
            let parent = match self.enter(chunk, world.take(), &mut way).await? {
                Ok(parent) => parent,
                Err(step) => return Ok(step),
            };
            if chunk.jump.is_some() {
                jumped = chunk.jump;
            }
            let root = way.root.take();
            let hopped = hop_from(
                self.page,
                self.scope,
                &parent,
                chunk,
                self.strictness,
                root.as_deref(),
            );
            match hopped.await? {
                Hopped::Into {
                    child,
                    hop,
                    visible,
                } => {
                    way.path.push(hop);
                    way.frame_visible &= visible;
                    world = Some(child);
                    jumped = None;
                }
                Hopped::Missing => return Ok(Step::Missing),
                Hopped::ParentGone => {
                    return match jumped {
                        Some(jump) => {
                            self.page.frames().invalidate(&parent);
                            Err(self.detached(&jump.to_string()))
                        }
                        None => Ok(Step::Gone(Some(parent))),
                    };
                }
                Hopped::ChildGone => return Ok(Step::Gone(None)),
            }
        }
        let last = self.target.chunks.last();
        let world = match self.enter(last, world, &mut way).await? {
            Ok(world) => world,
            Err(step) => return Ok(step),
        };
        if last.jump.is_some() {
            jumped = last.jump;
        }
        Ok(Step::Ready(Resolved {
            world,
            path: way.path,
            frame_visible: way.frame_visible,
            jumped,
            root: way.root,
        }))
    }

    /// The world `chunk` runs in: the document its leading ref names, else `current`, else
    /// the start world (the main frame's, or the locator's own frame's). A jump or a frame
    /// start replaces the way with the frames above its world.
    async fn enter(
        &self,
        chunk: &Chunk,
        current: Option<SealedWorld>,
        way: &mut Way,
    ) -> Result<Result<SealedWorld, Step>, PlaywrightError> {
        if let Some(jump) = chunk.jump {
            return match self.page.frames().by_seq(jump.seq) {
                SeqLookup::Live { world, parents } => {
                    let path = FramePath::from_parents(parents);
                    *way = Way {
                        frame_visible: iframes_visible(self.scope, &path).await?,
                        path,
                        root: None,
                    };
                    Ok(Ok(*world))
                }
                SeqLookup::Retired => Err(self.detached(&jump.to_string())),
                SeqLookup::Unknown => Err(self.scope.fail(
                    ErrorCode::InvalidInput,
                    &format!("{jump} is not a ref on this page; take a new snapshot"),
                )),
            };
        }
        if let Some(world) = current {
            return Ok(Ok(world));
        }
        if let Some(RootHandle::Element(root)) = &self.target.root {
            // A handle's frames count as visible, as upstream's handle queries start from
            // `frameVisible: true` (and as the handle's own reads do).
            let site = match HandleSite::find(self.page, self.scope, root).await? {
                Some(site) if site.attached().await? => site,
                Some(_) | None => return Err(resolve::handle_detached(self.scope)),
            };
            *way = Way {
                path: site.path().clone(),
                frame_visible: true,
                root: Some(site.node().to_owned()),
            };
            return Ok(Ok(site.world().clone()));
        }
        let main = match self
            .page
            .world(&self.page.main_endpoint(), Place::Main)
            .await
        {
            Ok(world) => world,
            Err(ProbeFailure::Gone) => return Ok(Err(Step::Gone(None))),
            Err(failure) => return Err(self.failed(failure)),
        };
        let Some(start) = &self.start else {
            return Ok(Ok(main));
        };
        // Boxed: a walk of hops, inside every attempt of every locator op's future.
        let started = Box::pin(start.enter(self.page, self.scope, main)).await?;
        Ok(match started {
            Started::Ready {
                world,
                path,
                visible,
            } => {
                *way = Way {
                    path,
                    frame_visible: visible,
                    root: None,
                };
                Ok(*world)
            }
            Started::Missing => Err(Step::Missing),
            Started::Gone(world) => Err(Step::Gone(world)),
        })
    }

    /// `website / element_detached`: the ref's element or document is gone, for good.
    fn detached(&self, stale: &str) -> PlaywrightError {
        detached(self.scope, stale)
    }

    /// A probe failure that is not the liveness rule's to handle.
    fn failed(&self, failure: ProbeFailure) -> PlaywrightError {
        failed(self.scope, failure)
    }
}

/// One frame hop out of `parent`, through the `<iframe>` that `chunk` selects there:
///
/// ```text
///  resolve { chunk, want: frame } ─ pin ─ pinned (by handle) ─ DOM.describeNode
///    ─ child frame id, cross-process? ─ core's FrameRegistry ─ the child's world
/// ```
///
/// The way into a child frame: the resolver hops along a locator's chunks, and a snapshot
/// hops through each iframe ref it printed (`aria-ref=<ref>`). A frame start (`frame_start`)
/// finds its iframes another way and shares the second half, [`enter_iframe`]. The child's
/// parent link is recorded with the iframe's key, so a later ref jump into the frame finds
/// its parents. Core owns the OOPIF sessions (decisions.md): the hop asks its registry and
/// keeps no table of its own.
pub(crate) async fn hop(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    parent: &SealedWorld,
    chunk: &Chunk,
    strictness: Strictness,
) -> Result<Hopped, PlaywrightError> {
    hop_from(page, scope, parent, chunk, strictness, None).await
}

/// [`hop`], with `chunk` queried from `root` (an element handle's node in `parent`'s world)
/// instead of the document.
async fn hop_from(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    parent: &SealedWorld,
    chunk: &Chunk,
    strictness: Strictness,
    root: Option<&str>,
) -> Result<Hopped, PlaywrightError> {
    let request = ProbeRequest::Resolve {
        target: ProbeTarget::Chunk(ParsedSelector::of(chunk)),
        strictness,
        want: Want::Frame,
    };
    let (pin, key, visible) = match call_from(parent, &request, root).await {
        Ok(ProbeReply::Frame {
            pin, key, visible, ..
        }) => (pin, key, visible),
        Ok(ProbeReply::None { stale_ref: None } | ProbeReply::Detached) => {
            return Ok(Hopped::Missing);
        }
        // Upstream throws this from the page, so it is never retried.
        Ok(ProbeReply::NotFrame { preview }) => {
            return Err(scope.fail(
                ErrorCode::InvalidInput,
                &format!(
                    "Error: Selector \"{}\" resolved to {preview}, <iframe> was expected",
                    selector_text(chunk)
                ),
            ));
        }
        Ok(other) => return Err(refused(scope, other)),
        Err(ProbeFailure::Gone) => return Ok(Hopped::ParentGone),
        Err(failure) => return Err(failed(scope, failure)),
    };
    let object = match probe::pinned_object(scope, parent, pin).await {
        Ok(Some(object)) => object,
        Ok(None) => return Ok(Hopped::Missing),
        Err(ProbeFailure::Gone) => return Ok(Hopped::ParentGone),
        Err(failure) => return Err(failed(scope, failure)),
    };
    let iframe = Iframe {
        object,
        pin,
        key,
        visible,
    };
    enter_iframe(page, scope, parent, iframe).await
}

/// The second half of every frame hop, from the `<iframe>` found in `parent`'s world to the
/// child's world: the content frame and whether it lives in another process
/// (`DOM.describeNode`), its session from core's frame registry, and the child's world, which
/// records how it was reached.
pub(super) async fn enter_iframe(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    parent: &SealedWorld,
    iframe: Iframe,
) -> Result<Hopped, PlaywrightError> {
    let Iframe {
        object,
        pin,
        key,
        visible,
    } = iframe;
    let described: Described = match parent
        .session()
        .send(
            "DOM.describeNode",
            json!({ "objectId": object, "depth": 1 }),
        )
        .await
    {
        Ok(described) => described,
        Err(error) => {
            return match ProbeFailure::from_core(error) {
                ProbeFailure::Gone => Ok(Hopped::ParentGone),
                failure => Err(failed(scope, failure)),
            };
        }
    };
    // Measured: `frameId` is set for same-process and out-of-process iframes alike, and
    // `contentDocument` only for the first.
    let Some(frame) = described.node.frame_id.map(FrameId) else {
        return Ok(Hopped::Missing);
    };
    let cross_process = described.node.content_document.is_none();
    let target = match page
        .browser()
        .frames()
        .resolve_frame_target(
            page.id().clone(),
            Some(frame.clone()),
            Some(parent.session()),
            cross_process,
        )
        .await
    {
        Ok(target) => target,
        Err(error) => {
            return match ProbeFailure::from_core(error) {
                ProbeFailure::Gone => Ok(Hopped::ChildGone),
                failure => Err(failed(scope, failure)),
            };
        }
    };
    let endpoint = page.child_endpoint(target.session, frame);
    let link = ParentLink {
        parent: parent.seq(),
        iframe_key: key,
    };
    match page.world(&endpoint, Place::Child(link)).await {
        Ok(child) => Ok(Hopped::Into {
            child,
            hop: Hop {
                parent: parent.clone(),
                iframe: HopIframe::Pin(pin),
            },
            visible,
        }),
        Err(ProbeFailure::Gone) => Ok(Hopped::ChildGone),
        Err(failure) => Err(failed(scope, failure)),
    }
}

/// `request` in `world`: from the document, or from `root` (an element handle's node there),
/// which the probe reads as the call's root element.
async fn call_from(
    world: &SealedWorld,
    request: &ProbeRequest,
    root: Option<&str>,
) -> Result<ProbeReply, ProbeFailure> {
    match root {
        Some(root) => probe::call_on(world, request, root).await,
        None => probe::call(world, request).await,
    }
}

/// Whether every `<iframe>` on `path` is visible in its parent, as a hop checks the one it goes
/// through (`isElementVisible`). A ref jump lands below parents it did not walk, and without
/// this an element inside a `visibility: hidden` iframe would read visible. Neo counts the
/// whole path wherever the frame was reached from; upstream starts a jump at `true`. An iframe
/// that is gone, or a parent world that is, is not visible: the request in the jumped world
/// decides whether the ref's document still lives.
async fn iframes_visible(scope: &OpScope<'_>, path: &FramePath) -> Result<bool, PlaywrightError> {
    for hop in path.hops() {
        let iframe = hop
            .iframe
            .target()
            .map_err(|error| scope.fail(ErrorCode::Internal, &error.to_string()))?;
        let request = ProbeRequest::Read {
            target: iframe,
            strictness: Strictness::Strict,
            frame_visible: true,
            read: Read::State {
                state: QueryState::Visible,
            },
        };
        let visible = match probe::call(&hop.parent, &request).await {
            Ok(ProbeReply::Read { value, .. }) => value.textless() == Value::Bool(true),
            Ok(ProbeReply::None { .. } | ProbeReply::Detached) | Err(ProbeFailure::Gone) => false,
            Ok(other) => return Err(refused(scope, other)),
            Err(failure) => return Err(failed(scope, failure)),
        };
        if !visible {
            return Ok(false);
        }
    }
    Ok(true)
}

/// A reply that ends the op: a ref that can never match again, a strict violation, an error
/// thrown in the page, or a reply the request does not take.
pub(crate) fn refused(scope: &OpScope<'_>, reply: ProbeReply) -> PlaywrightError {
    match reply {
        ProbeReply::None {
            stale_ref: Some(stale),
        } => detached(scope, &stale),
        // Upstream's message comes from an error thrown in the page, hence "Error: ".
        ProbeReply::Strict { message } => {
            scope.fail(ErrorCode::StrictModeViolation, &format!("Error: {message}"))
        }
        ProbeReply::Error { message, class } => match class {
            ErrorClass::Selector => scope.fail(ErrorCode::InvalidInput, &message),
            ErrorClass::Script => scope.fail(ErrorCode::InvalidInput, &format!("Error: {message}")),
            ErrorClass::Unsupported => scope.fail(ErrorCode::Unsupported, &message),
            ErrorClass::Internal => scope.fail(ErrorCode::Internal, &message),
        },
        other => scope.fail(
            ErrorCode::Internal,
            &format!(
                "the probe answered `{}`, which this request does not take",
                other.kind()
            ),
        ),
    }
}

/// `website / element_detached`: the ref's element or document is gone, for good.
fn detached(scope: &OpScope<'_>, stale: &str) -> PlaywrightError {
    scope.fail(
        ErrorCode::ElementDetached,
        &format!("{stale} is not on the page any more; take a new snapshot"),
    )
}

/// A probe failure that is not the liveness rule's to handle.
pub(crate) fn failed(scope: &OpScope<'_>, failure: ProbeFailure) -> PlaywrightError {
    match failure {
        ProbeFailure::Gone => scope.fail(ErrorCode::TargetClosed, "the frame's document went away"),
        ProbeFailure::Cdp(error) => scope.core_error(&error),
        ProbeFailure::Broken(why) => scope.fail(ErrorCode::Internal, &why),
    }
}

/// Upstream's `stringifySelector` for one chunk, for the not-an-iframe message: each part as
/// `name=source`, a `css` part (and an `xpath` one that reads as a path) as its source alone,
/// and the capture marked `*`.
fn selector_text(chunk: &Chunk) -> String {
    #[derive(Deserialize)]
    struct Parsed {
        parts: Vec<Part>,
        #[serde(default)]
        capture: Option<usize>,
    }
    #[derive(Deserialize)]
    struct Part {
        name: String,
        source: String,
    }
    let Ok(parsed) = serde_json::from_str::<Parsed>(chunk.parsed.get()) else {
        return String::new();
    };
    let parts: Vec<String> = parsed
        .parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            let captured = parsed.capture == Some(index);
            let bare = !captured
                && (part.name == "css"
                    || (part.name == "xpath" && part.source.starts_with("//"))
                    || part.source.starts_with(".."));
            match (captured, bare) {
                (true, _) => format!("*{}={}", part.name, part.source),
                (false, true) => part.source.clone(),
                (false, false) => format!("{}={}", part.name, part.source),
            }
        })
        .collect();
    parts.join(" >> ")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use browseros_core::sensitive_fields::FieldValues;
    use serde_json::{Value, json};
    use tokio::time;

    use super::{Resolved, Resolver, Step, selector_text};
    use crate::op_scope::OpScope;
    use crate::page::OpPage;
    use crate::probe::{ProbeReply, ProbeRequest, ProbeTarget, Read};
    use crate::resolve::{HopIframe, Landed};
    use crate::target::{CallHeader, Chunk, LocatorTarget, Strictness, Target};
    use crate::test_support::fixture_server::{FixtureServer, nest_pages};
    use crate::test_support::{LiveBrowser, Tab, TestResult, count_op};
    use crate::world::{FrameSeq, SeqLookup};

    #[test]
    fn tests_that_a_chunk_prints_as_upstreams_stringify_selector() -> TestResult {
        let chunk: Chunk = serde_json::from_value(json!({ "parts": [
            { "name": "css", "body": [], "source": "#pay" },
            { "name": "internal:has-text", "body": "\"Go\"i", "source": "\"Go\"i" },
        ] }))?;
        assert_eq!(selector_text(&chunk), "#pay >> internal:has-text=\"Go\"i");
        let captured: Chunk = serde_json::from_value(json!({ "parts": [
            { "name": "css", "body": [], "source": "li" },
            { "name": "xpath", "body": "//b", "source": "//b" },
        ], "capture": 0 }))?;
        assert_eq!(selector_text(&captured), "*css=li >> //b");
        Ok(())
    }

    /// A locator down the nested fixture's iframes by title, ending on `last`.
    fn down(tab: &Tab, titles: &[&str], last: Value) -> TestResult<LocatorTarget> {
        let mut chunks: Vec<Value> = titles
            .iter()
            .map(|title| {
                let body = format!("[title=\"{title}\"s]");
                json!({ "parts": [{ "name": "internal:attr", "body": body, "source": body }] })
            })
            .collect();
        chunks.push(last);
        let target: Target = serde_json::from_value(json!({
            "$neo": "locator", "page": tab.brand(), "chunks": chunks, "display": titles.join(" >> "),
        }))?;
        let Target::Locator(target) = target else {
            return Err("a locator brand is a locator".into());
        };
        Ok(target)
    }

    fn button() -> Value {
        json!({ "parts": [{ "name": "internal:role", "body": "button", "source": "button" }] })
    }

    /// Resolves until every frame on the way has loaded.
    async fn resolved(resolver: &Resolver<'_>) -> TestResult<Resolved> {
        for _ in 0..100 {
            match resolver.resolve().await? {
                Step::Ready(resolved) => return Ok(resolved),
                Step::Missing | Step::Gone(_) => time::sleep(Duration::from_millis(50)).await,
            }
        }
        Err("the frames did not load within 5 s".into())
    }

    #[tokio::test]
    #[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
    async fn tests_that_every_hop_records_its_parent_link_for_a_later_ref_jump() -> TestResult {
        let live = LiveBrowser::connect().await?;
        let server = FixtureServer::start(nest_pages()).await?;
        let tab = live
            .open(&server.url("127.0.0.1", "/nest/top.html"))
            .await?;
        let env = live.env();
        let scope = OpScope::new(&env, count_op()?, CallHeader::default());
        let page = OpPage::admit(&live.playwright, &scope, &tab.page).await?;
        let deepest = down(&tab, &["b1", "c2", "a3", "c4"], button())?;
        let resolver = Resolver::new(&page, &scope, &deepest, Strictness::Strict);
        let c4 = resolved(&resolver).await?;
        assert_eq!(c4.path.hops().len(), 4, "four parents above c4");
        assert!(
            c4.path
                .hops()
                .iter()
                .all(|hop| matches!(hop.iframe, HopIframe::Pin(_))),
            "a hop pins each iframe it went through"
        );
        assert!(c4.frame_visible && c4.jumped.is_none());
        let SeqLookup::Live { world, parents } = page.frames().by_seq(c4.world.seq()) else {
            return Err("c4's seq is live".into());
        };
        assert!(world.same_install(&c4.world));
        assert_eq!(parents.len(), 4, "every hop recorded its link");
        for (parent, hop) in parents.iter().zip(c4.path.hops()) {
            assert!(
                parent.world.same_install(&hop.parent),
                "the same parents, top down"
            );
            assert!(parent.iframe_key.starts_with('#'), "{}", parent.iframe_key);
        }

        // A jump into c4's document needs no hop: the links give its parents.
        let jump_ref = format!("f{}e1", c4.world.seq().0);
        let jump = down(
            &tab,
            &[],
            json!({ "parts": [{ "name": "aria-ref", "body": jump_ref, "source": jump_ref }] }),
        )?;
        let jumper = Resolver::new(&page, &scope, &jump, Strictness::Strict);
        let landed = resolved(&jumper).await?;
        assert!(landed.world.same_install(&c4.world));
        assert_eq!(landed.jumped.map(|jump| jump.seq), Some(c4.world.seq()));
        assert!(
            landed
                .path
                .hops()
                .iter()
                .all(|hop| matches!(&hop.iframe, HopIframe::Key(key) if key.starts_with('#'))),
            "a jump finds each parent's iframe by its recorded key"
        );
        assert_eq!(landed.path.hops().len(), 4);
        live.close(&tab).await;
        Ok(())
    }

    #[tokio::test]
    #[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
    async fn tests_that_an_iframe_that_navigated_gets_a_new_world_and_seq_on_the_next_hop()
    -> TestResult {
        let live = LiveBrowser::connect().await?;
        let server = FixtureServer::start(nest_pages()).await?;
        let tab = live
            .open(&server.url("127.0.0.1", "/nest/top.html"))
            .await?;
        let env = live.env();
        let scope = OpScope::new(&env, count_op()?, CallHeader::default());
        let page = OpPage::admit(&live.playwright, &scope, &tab.page).await?;
        let a1 = down(&tab, &["a1"], button())?;
        let resolver = Resolver::new(&page, &scope, &a1, Strictness::Strict);
        let before = resolved(&resolver).await?.world;

        tab.main_world("document.querySelector('iframe[title=a1]').src = '/nest/b3.html'")
            .await?;
        let read = |last, frame_visible| ProbeRequest::Read {
            target: ProbeTarget::Chunk(last),
            strictness: Strictness::Strict,
            frame_visible,
            read: Read::TextContent,
        };
        let mut text = Value::Null;
        for _ in 0..100 {
            if let Some(Landed {
                reply: ProbeReply::Read { value, .. },
                ..
            }) = resolver.attempt_at(read).await?
            {
                text = value.masked(&FieldValues::default());
                if text == "B3 button" {
                    break;
                }
            }
            time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(text, "B3 button", "the frame's new document answers");
        let after = resolved(&resolver).await?.world;
        assert_eq!(
            after.frame(),
            before.frame(),
            "the same frame, a new document"
        );
        assert!(
            after.seq() > before.seq(),
            "{:?} after {:?}: a new document takes a new seq",
            after.seq(),
            before.seq()
        );
        assert!(matches!(
            page.frames().by_seq(before.seq()),
            SeqLookup::Retired
        ));
        assert!(matches!(
            page.frames().by_seq(FrameSeq(after.seq().0 + 1)),
            SeqLookup::Unknown
        ));
        live.close(&tab).await;
        Ok(())
    }
}
