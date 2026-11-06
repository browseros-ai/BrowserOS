//! From a locator to the world its last chunk runs in, through every frame on the way.

use std::future::Future;

use browseros_core::FrameId;
use browseros_core::sensitive_fields::FieldValues;
use futures_util::future;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::op_scope::OpScope;
use crate::page::site_sweep::{SiteSweep, SiteValues};
use crate::page::{FRAME_BUDGET, OpPage};
use crate::probe::{
    self, ErrorClass, Hit, ParsedSelector, ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget,
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
/// time as the request, such as a read's site sweep, or the values the request of
/// [`Resolver::attempt_masked`] was masked with; none for [`Resolver::attempt_at`].
#[derive(Debug)]
pub(crate) struct Landed<T = ()> {
    pub(crate) reply: ProbeReply,
    pub(crate) world: SealedWorld,
    pub(crate) path: FramePath,
    pub(crate) alongside: T,
}

/// A request's reply, and what ran with the request in its world: a read's site sweep, or the
/// values the request was masked with. It becomes a [`Landed`]'s `alongside`.
pub(crate) struct Asked<T> {
    pub(crate) reply: ProbeReply,
    pub(crate) alongside: T,
}

/// What [`Resolver::attempt`]'s `send` got in the world a pass resolved, or why the world gave
/// none (`Gone` is the liveness rule's).
type Answered<T> = Result<Asked<T>, ProbeFailure>;

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
pub(crate) struct Resolved {
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

impl Resolved {
    /// Where an action finds an element handle's node: its document's world and that frame's
    /// parents, with the node as the root of every request. Its frames count as visible, as
    /// upstream's handle actions pass `frameVisible: true`.
    pub(crate) fn of_handle(site: &HandleSite<'_>) -> Self {
        Self {
            world: site.world().clone(),
            path: site.path().clone(),
            frame_visible: true,
            jumped: None,
            root: Some(site.node().to_owned()),
        }
    }

    /// Sends `request` in [`Resolved::world`], with the root node the target is queried from
    /// when there is one (an element handle's node, or the root of a locator rooted at a
    /// handle). A pinned target ignores the root. A reply that would hold page text the engine
    /// cut (a strict violation, a covered element's description) answers `needsMask`, and the
    /// request goes once more with the site's values ([`ask`]), so every action request is
    /// masked in the page without a sweep before the ones that hold no text.
    pub(crate) async fn call(
        &self,
        page: &OpPage<'_>,
        scope: &OpScope<'_>,
        request: &ProbeRequest,
    ) -> Result<Result<ProbeReply, ProbeFailure>, PlaywrightError> {
        ask(page, scope, &self.world, request, self.root.as_deref()).await
    }

    /// The world the last chunk runs in.
    pub(crate) fn world(&self) -> &SealedWorld {
        &self.world
    }

    /// The parent frames between the page and `world`.
    pub(crate) fn path(&self) -> &FramePath {
        &self.path
    }

    /// Every `<iframe>` on the way is visible.
    pub(crate) fn frame_visible(&self) -> bool {
        self.frame_visible
    }
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
    /// answered is kept. A request that answers `needsMask` (its reply previews page text, a
    /// strict violation say) is asked once more with its site's values ([`remask`]).
    pub(crate) async fn attempt_alongside<T, Fut>(
        &self,
        request: impl Fn(ParsedSelector, bool) -> ProbeRequest,
        alongside: impl Fn(SealedWorld) -> Fut,
    ) -> Result<Option<Landed<T>>, PlaywrightError>
    where
        Fut: Future<Output = T>,
    {
        self.attempt(request, |world, request, root| {
            let beside = alongside(world.clone());
            async move {
                let asked = self.ask(&world, &request, root.as_deref());
                let (reply, alongside) = future::join(asked, beside).await;
                Ok(reply?.map(|reply| Asked { reply, alongside }))
            }
        })
        .await
    }

    /// [`Resolver::attempt_at`] for a request whose reply always previews page text (`located`,
    /// `pins`): the landed world's site is swept first and the request carries its values, so
    /// the probe masks the previews before it cuts them and never answers `needsMask`; a field
    /// that changed meanwhile is caught by a sweep after the reply ([`masked_call`]). A frame of
    /// the site that missed a sweep fails the op closed; the landed world's own frame gone under
    /// either sweep is the request coming back gone. The values come back as `alongside`, for
    /// the op's own mask of the reply.
    pub(crate) async fn attempt_masked(
        &self,
        request: impl Fn(ParsedSelector, bool) -> ProbeRequest,
    ) -> Result<Option<Landed<SiteValues>>, PlaywrightError> {
        self.attempt(request, |world, request, root| async move {
            let site = match site_values(self.page, self.scope, &world).await? {
                Ok(site) => site,
                Err(gone) => return Ok(Err(gone)),
            };
            let root = root.as_deref();
            let Masked { reply, site } =
                masked_call(self.page, self.scope, &world, &request, root, site).await?;
            Ok(reply.map(|reply| Asked {
                reply,
                alongside: site,
            }))
        })
        .await
    }

    /// The passes of one attempt, under the liveness rule. `send` runs the request in the world
    /// a pass resolved (with the handle's node as its root, when the pass starts from one), and
    /// what ran with it comes back beside the reply.
    async fn attempt<T, Fut>(
        &self,
        request: impl Fn(ParsedSelector, bool) -> ProbeRequest,
        send: impl Fn(SealedWorld, ProbeRequest, Option<String>) -> Fut,
    ) -> Result<Option<Landed<T>>, PlaywrightError>
    where
        Fut: Future<Output = Result<Answered<T>, PlaywrightError>>,
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
            let answered = send(resolved.world.clone(), request, resolved.root.clone()).await?;
            match answered {
                Ok(Asked { reply, alongside }) => {
                    return Ok(Some(Landed {
                        reply,
                        world: resolved.world,
                        path: resolved.path,
                        alongside,
                    }));
                }
                Err(ProbeFailure::Gone(_)) => {
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

    /// Resolves the target to the world its last chunk runs in, for ops that keep talking to
    /// that world across calls (an action's measurements, interceptor and verdict). `None`
    /// while a frame on the way is not there yet. A world that turns out gone on the way is
    /// invalidated and the pass runs once more, as in [`Resolver::attempt_at`].
    pub(crate) async fn locate(&self) -> Result<Option<Resolved>, PlaywrightError> {
        for _ in 0..2 {
            match self.resolve().await? {
                Step::Ready(resolved) => return Ok(Some(resolved)),
                Step::Missing => return Ok(None),
                Step::Gone(world) => {
                    if let Some(world) = world {
                        self.page.frames().invalidate(&world);
                    }
                }
            }
        }
        Ok(None)
    }

    /// The world `resolved` named went away under a call: it is forgotten, and the op
    /// resolves again, unless a ref jump landed there, whose document is gone for good.
    pub(crate) fn lost(&self, resolved: &Resolved) -> Result<(), PlaywrightError> {
        self.page.frames().invalidate(&resolved.world);
        match resolved.jumped {
            Some(jump) => Err(self.detached(&jump.to_string())),
            None => Ok(()),
        }
    }

    /// A reply that ends the op: a ref that can never match again, a strict violation, an
    /// error thrown in the page, or a reply the request does not take.
    pub(crate) fn refused(&self, reply: ProbeReply) -> PlaywrightError {
        refused(self.scope, reply)
    }

    /// [`ask`] on this resolver's page.
    async fn ask(
        &self,
        world: &SealedWorld,
        request: &ProbeRequest,
        root: Option<&str>,
    ) -> Result<Result<ProbeReply, ProbeFailure>, PlaywrightError> {
        ask(self.page, self.scope, world, request, root).await
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
            Err(ProbeFailure::Gone(_)) => return Ok(Err(Step::Gone(None))),
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
    pub(crate) fn failed(&self, failure: ProbeFailure) -> PlaywrightError {
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
    // A hop that previews page text, a strict violation's or a non-iframe's, is asked again with
    // the parent's site's values, which mask that text here too.
    let (reply, masked) = match call_from(parent, &request, root).await {
        Ok(ProbeReply::NeedsMask) => (
            Box::pin(remask(page, scope, parent, &request, root)).await?,
            true,
        ),
        reply => (reply, false),
    };
    let (pin, key, visible) = match reply {
        Ok(ProbeReply::Frame {
            pin, key, visible, ..
        }) => (pin, key, visible),
        Ok(ProbeReply::None { stale_ref: None } | ProbeReply::Detached) => {
            return Ok(Hopped::Missing);
        }
        Ok(ProbeReply::NotFrame { .. } | ProbeReply::Strict { .. }) if !masked => {
            return Err(unmasked(scope));
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
        Err(ProbeFailure::Gone(_)) => return Ok(Hopped::ParentGone),
        Err(failure) => return Err(failed(scope, failure)),
    };
    // Boxed: every resolver pass awaits a hop, and every locator op's future holds a pass.
    Box::pin(enter_pinned(page, scope, parent, pin, key, visible)).await
}

/// Into the child frame behind the `<iframe>` that `pin` names in `parent`, as [`hop`] does
/// with a chunk: the way down to the frame a focused iframe hands keyboard input to. An
/// element that is gone or is no iframe any more is [`Hopped::Missing`].
pub(crate) async fn hop_pinned(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    parent: &SealedWorld,
    pin: Pin,
) -> Result<Hopped, PlaywrightError> {
    let request = ProbeRequest::Resolve {
        target: ProbeTarget::Pin(pin),
        strictness: Strictness::Strict,
        want: Want::Frame,
    };
    // A pin names one element, so only a not-an-iframe preview can ask for the mask
    // (`needsMask`); the preview is never shown here, so it is not asked for again.
    let (pin, key, visible) = match probe::call(parent, &request).await {
        Ok(ProbeReply::Frame {
            pin, key, visible, ..
        }) => (pin, key, visible),
        Ok(
            ProbeReply::None { stale_ref: None }
            | ProbeReply::Detached
            | ProbeReply::NotFrame { .. }
            | ProbeReply::NeedsMask,
        ) => return Ok(Hopped::Missing),
        Ok(other) => return Err(refused(scope, other)),
        Err(ProbeFailure::Gone(_)) => return Ok(Hopped::ParentGone),
        Err(failure) => return Err(failed(scope, failure)),
    };
    enter_pinned(page, scope, parent, pin, key, visible).await
}

/// A hop's iframe, pinned in `parent`, by handle, then [`enter_iframe`].
async fn enter_pinned(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    parent: &SealedWorld,
    pin: Pin,
    key: String,
    visible: bool,
) -> Result<Hopped, PlaywrightError> {
    let object = match probe::pinned_object(scope, parent, pin).await {
        Ok(Some(object)) => object,
        Ok(None) => return Ok(Hopped::Missing),
        Err(ProbeFailure::Gone(_)) => return Ok(Hopped::ParentGone),
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
                ProbeFailure::Gone(_) => Ok(Hopped::ParentGone),
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
                ProbeFailure::Gone(_) => Ok(Hopped::ChildGone),
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
        Err(ProbeFailure::Gone(_)) => Ok(Hopped::ChildGone),
        Err(failure) => Err(failed(scope, failure)),
    }
}

/// `request` in `world` (from `root` when there is one), and once more with the site's values
/// ([`remask`]) when it answers `needsMask`: the probe met page text the engine cuts, a strict
/// violation's or a covered element's say, with no mask to put on it first. Every request of a
/// locator op, an action or a matcher whose reply can hold such text goes through here,
/// [`Resolver::attempt_masked`] or [`remask`], never a bare probe call that would refuse the
/// reply.
pub(crate) async fn ask(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    world: &SealedWorld,
    request: &ProbeRequest,
    root: Option<&str>,
) -> Result<Result<ProbeReply, ProbeFailure>, PlaywrightError> {
    let asked = call_from(world, request, root).await;
    if !matches!(asked, Ok(ProbeReply::NeedsMask)) {
        return Ok(asked);
    }
    Box::pin(remask(page, scope, world, request, root)).await
}

/// `request` in `world` with its site's values, swept first, for a request whose reply always
/// previews page text (an `expect` poll's `resolved`), so it never answers `needsMask`. The
/// values come back as the reply's `alongside`, for the op's own mask of it; `world`'s frame
/// gone under a sweep is the request coming back gone.
pub(crate) async fn ask_masked(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    world: &SealedWorld,
    request: &ProbeRequest,
) -> Result<Result<Asked<SiteValues>, ProbeFailure>, PlaywrightError> {
    let site = match site_values(page, scope, world).await? {
        Ok(site) => site,
        Err(gone) => return Ok(Err(gone)),
    };
    let Masked { reply, site } = masked_call(page, scope, world, request, None, site).await?;
    Ok(reply.map(|reply| Asked {
        reply,
        alongside: site,
    }))
}

/// Sends `request` in `world` again, with the values of every sensitive field of the world's
/// site, swept now: the probe masks page text with them before Playwright's engine cuts it for
/// a preview or a hint, and the page text of a reply that ends the op (a strict violation, a
/// not-an-iframe preview) is masked with them in Rust too ([`masked_call`] checks them against
/// a sweep after the reply). A frame of the site that missed a sweep fails the op closed,
/// because the reply's text may hold that frame's value. Callers box it: it is a rare second
/// call, and every locator op's future would otherwise hold its state.
pub(super) async fn remask(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    world: &SealedWorld,
    request: &ProbeRequest,
    root: Option<&str>,
) -> Result<Result<ProbeReply, ProbeFailure>, PlaywrightError> {
    let site = match site_values(page, scope, world).await? {
        Ok(site) => site,
        Err(gone) => return Ok(Err(gone)),
    };
    let Masked { reply, site } = masked_call(page, scope, world, request, root, site).await?;
    Ok(reply.map(|reply| site.mask_reply(reply)))
}

/// A reply to a request that carried its site's values, and the values its page text was
/// masked with in the page, which the op masks the reply with again.
struct Masked {
    reply: Result<ProbeReply, ProbeFailure>,
    site: SiteValues,
}

/// `request` in `world`, carrying `site`: the values of every sensitive field of the world's
/// site as a sweep before the call found them.
///
/// The probe adds its own frame's values as they are during the call, so that frame can never
/// be newer than the mask. Another frame of the site can: typed into after its sweep, and copied
/// into this frame's text by the page's script, its new value would be cut short and match no
/// mask afterwards. So a reply that holds text the probe cut is followed by a sweep of the
/// site. A value the call lacked sends the request once more with every value found; a value
/// the second call lacked too withholds the reply, rather than chase a field still changing.
///
/// ```text
///  sweep ─ call(site) ─ sweep: nothing new ─ the reply, masked with both sweeps
///                             └ a new value ─ call(all) ─ sweep: nothing new ─ the reply
///                                                               └ new again ─ withheld
/// ```
async fn masked_call(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    world: &SealedWorld,
    request: &ProbeRequest,
    root: Option<&str>,
    site: SiteValues,
) -> Result<Masked, PlaywrightError> {
    let site = match checked_call(page, scope, world, request, root, site).await? {
        Checked::Fits(masked) => return Ok(masked),
        Checked::Changed(site) => site,
    };
    match checked_call(page, scope, world, request, root, site).await? {
        Checked::Fits(masked) => Ok(masked),
        Checked::Changed(_) => Err(scope.fail(
            ErrorCode::Unsupported,
            "a sensitive field of the frame's site changed while Neo read the page, and again \
             when it read once more, so the result is withheld; try again once the field stops \
             changing",
        )),
    }
}

/// How one masked call compares with the sweep of its site that followed it.
enum Checked {
    /// The sweep found no value the call lacked: the reply, with every value either sweep found.
    Fits(Masked),
    /// A field of the site changed after the sweep the call carried: every value found so far.
    Changed(SiteValues),
}

/// One masked call, and the sweep of the site after it when the reply holds text the probe cut.
async fn checked_call(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    world: &SealedWorld,
    request: &ProbeRequest,
    root: Option<&str>,
    site: SiteValues,
) -> Result<Checked, PlaywrightError> {
    let reply = probe::call_masked(world, request, root, site.probe_mask()).await;
    if !holds_cut_text(&reply) {
        return Ok(Checked::Fits(Masked { reply, site }));
    }
    // A reply's sweep is its frame's values as the probe added them to the mask (protocol.ts's
    // `Mask`), so a change in the probe's own frame never sends the request again.
    let own = match &reply {
        Ok(ProbeReply::Read { sweep, .. } | ProbeReply::Values { sweep, .. }) => Some(sweep),
        Ok(_) | Err(_) => None,
    };
    // The host hears them now: the sweep below can drop this reply or send the request again,
    // and either exit would lose a value only this call saw, which the next document may echo.
    if let Some(own) = own.filter(|own| !own.is_empty()) {
        page.playwright()
            .hooks()
            .values_swept(page.id().clone(), own);
    }
    let after = match site_values(page, scope, world).await? {
        Ok(after) => after,
        // The reply's frame went away before the sweep could check the site, so the reply is
        // dropped for the request coming back gone, and none of its page text leaves.
        Err(gone) => {
            return Ok(Checked::Fits(Masked {
                reply: Err(gone),
                site,
            }));
        }
    };
    let changed = site.lacks_any_of(own, &after);
    let site = site.union(after);
    Ok(if changed {
        Checked::Changed(site)
    } else {
        Checked::Fits(Masked { reply, site })
    })
}

/// Whether `reply` can hold page text the probe cut for a preview or a hint: anything but a
/// miss, an iframe (whose reply carries no preview), an action's states, measurements and
/// arming, a hit test that hit, or a failure. A kind added later counts as holding some until
/// it is listed here.
fn holds_cut_text(reply: &Result<ProbeReply, ProbeFailure>) -> bool {
    !matches!(
        reply,
        Ok(ProbeReply::None { .. }
            | ProbeReply::Detached
            | ProbeReply::Frame { .. }
            | ProbeReply::State { .. }
            | ProbeReply::Measured { .. }
            | ProbeReply::Armed { .. }
            | ProbeReply::Moves { .. }
            | ProbeReply::Focused { .. }
            | ProbeReply::Verdict { hit: Hit::Done(_) }
            | ProbeReply::FrameBox {
                hit: None | Some(Hit::Done(_)),
                ..
            })
            | Err(_)
    )
}

/// The values of every sensitive field of `world`'s site, swept now and told to the host, for a
/// request sent in `world` after the sweep: `Gone` when `world`'s own frame went away under it
/// (`SiteSweep::values_or_gone`).
async fn site_values(
    page: &OpPage<'_>,
    scope: &OpScope<'_>,
    world: &SealedWorld,
) -> Result<Result<SiteValues, ProbeFailure>, PlaywrightError> {
    SiteSweep::of(page, world.clone()).await.values_or_gone(
        page.playwright(),
        scope,
        page.id(),
        FieldValues::default(),
    )
}

/// A hop's strict violation or not-an-iframe preview from a call that carried no mask. The
/// probe answers `needsMask` instead of sending such text unmasked, so this is a probe bug,
/// and the op fails closed rather than let the text out.
pub(super) fn unmasked(scope: &OpScope<'_>) -> PlaywrightError {
    scope.fail(
        ErrorCode::Internal,
        "the probe previewed page text without the mask of its frame's site",
    )
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
            Ok(ProbeReply::None { .. } | ProbeReply::Detached) | Err(ProbeFailure::Gone(_)) => {
                false
            }
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
        ProbeFailure::Gone(_) => {
            scope.fail(ErrorCode::TargetClosed, "the frame's document went away")
        }
        ProbeFailure::Cdp(error) => scope.core_error(&error),
        ProbeFailure::Broken(why) => scope.fail(ErrorCode::Internal, &why),
        ProbeFailure::Late => scope.fail(
            ErrorCode::Unsupported,
            &format!(
                "a frame did not answer Neo's privacy scan within {}ms, so Neo cannot see every \
                 shadow root of its page and refuses the op; try again once the frame responds",
                FRAME_BUDGET.as_millis()
            ),
        ),
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
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use browseros_core::sensitive_fields::FieldValues;
    use serde_json::{Value, json};
    use tokio::time;

    use super::{Masked, Resolved, Resolver, Step, masked_call, selector_text, site_values};
    use crate::op_scope::OpScope;
    use crate::page::OpPage;
    use crate::page::site_sweep::SiteValues;
    use crate::probe::{ProbeFailure, ProbeReply, ProbeRequest, ProbeTarget, Read};
    use crate::resolve::{HopIframe, Landed};
    use crate::target::{CallHeader, Chunk, LocatorTarget, Strictness, Target};
    use crate::test_support::fixture_server::{FixtureServer, Page, nest_pages};
    use crate::test_support::{
        LiveBrowser, ProbeAnswer, ScriptedRun, ScriptedTab, Tab, TabScript, TestResult, count_op,
    };
    use crate::world::{FrameSeq, Place, SeqLookup};

    /// What the read's frame does once it answered the first read.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum AfterRead {
        /// It leaves for another document: the sweep after the reply finds it gone, and the
        /// reply is dropped.
        Leaves,
        /// Another frame of its site holds a new value from then on: the sweep after the reply
        /// finds it, and the read is sent once more.
        Changes,
    }

    /// One frame of `https://a.test/` whose first read sees [`TYPED_LATE`] and whose later
    /// reads do not.
    struct ReadOnce {
        after: AfterRead,
        reads: AtomicUsize,
    }

    /// A value in the read's own frame after the sweep before the call, which only the first
    /// call's own sweep saw.
    const TYPED_LATE: &str = "typed-after-the-sweep";

    impl ReadOnce {
        fn new(after: AfterRead) -> Self {
            Self {
                after,
                reads: AtomicUsize::new(0),
            }
        }

        fn read_once(&self) -> bool {
            self.reads.load(Ordering::SeqCst) > 0
        }
    }

    impl TabScript for ReadOnce {
        fn probe(&self, _frame: &str, request: &Value) -> ProbeAnswer {
            if self.after == AfterRead::Leaves && self.read_once() {
                return ProbeAnswer::error("Cannot find context with specified id");
            }
            if request["op"] == "read" {
                let own: &[&str] = if self.reads.fetch_add(1, Ordering::SeqCst) == 0 {
                    &[TYPED_LATE]
                } else {
                    &[]
                };
                return ProbeAnswer::reply(json!({ "kind": "read", "value": "Signed in",
                    "sensitive": null, "sweep": own }));
            }
            if self.read_once() {
                return ProbeAnswer::reply(
                    json!({ "kind": "sweep", "values": ["typed-in-another-frame"] }),
                );
            }
            ProbeAnswer::empty_sweep()
        }

        fn left(&self) -> bool {
            self.after == AfterRead::Leaves && self.read_once()
        }
    }

    /// A reply carries its own frame's values as the probe found them during the call. The
    /// sweep after it can drop the reply (its frame went away) or send the read once more
    /// (another frame changed), and either way the host still learns those values: the next
    /// document or the next reply may echo one (a form that posts the field and prints it
    /// back), and the host masks every exit with what it was told.
    #[tokio::test(start_paused = true)]
    async fn tests_that_a_dropped_or_repeated_reply_still_tells_the_host_its_own_sweep()
    -> TestResult {
        for after in [AfterRead::Leaves, AfterRead::Changes] {
            let tab = ScriptedTab::new(ReadOnce::new(after));
            let run = ScriptedRun::over(&tab).await?;
            let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
            let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
            let world = page
                .world(&page.main_endpoint(), Place::Main)
                .await
                .map_err(|failure| failure.to_string())?;
            let request = located("h1")?;
            let site = SiteValues::found(FieldValues::default());
            let Masked { reply, .. } =
                masked_call(&page, &scope, &world, &request, None, site).await?;
            match after {
                AfterRead::Leaves => assert!(
                    matches!(reply, Err(ProbeFailure::Gone(_))),
                    "the reply whose frame left before the sweep after it is dropped"
                ),
                AfterRead::Changes => assert!(
                    matches!(reply, Ok(ProbeReply::Read { .. })),
                    "the read sent once more answers"
                ),
            }
            assert!(
                run.hooks.swept().iter().any(|value| value == TYPED_LATE),
                "the first reply's own sweep reached the host when its frame {}: {:?}",
                if after == AfterRead::Leaves {
                    "left"
                } else {
                    "changed"
                },
                run.hooks.swept()
            );
        }
        Ok(())
    }

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

    /// The top page of `tests/data/site_copy/top.html` and its same-origin field frame.
    fn site_copy_pages() -> HashMap<String, Page> {
        [
            (
                "/site_copy/top.html",
                include_str!("../../tests/data/site_copy/top.html"),
            ),
            (
                "/site_copy/field.html",
                include_str!("../../tests/data/site_copy/field.html"),
            ),
        ]
        .into_iter()
        .map(|(path, body)| (path.to_owned(), Page::html(body)))
        .collect()
    }

    /// `page.$(selector, { strict: true })`'s request in the main frame.
    fn located(selector: &str) -> TestResult<ProbeRequest> {
        let parsed = json!({ "parts": [{
            "name": "css",
            "body": [{ "simples": [{ "selector": { "css": selector, "functions": [] }, "combinator": "" }] }],
            "source": selector,
        }] });
        Ok(ProbeRequest::Read {
            target: ProbeTarget::Chunk(serde_json::from_value(parsed)?),
            strictness: Strictness::Strict,
            frame_visible: true,
            read: Read::Located,
        })
    }

    /// The longest start of `secret`, three characters or more, that `shown` holds.
    fn leaked_start<'a>(shown: &str, secret: &'a str) -> Option<&'a str> {
        (3..=secret.len())
            .rev()
            .filter_map(|end| secret.get(..end))
            .find(|start| shown.contains(start))
    }

    #[tokio::test]
    #[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT (default 9005)"]
    async fn tests_that_a_field_of_the_site_typed_into_after_the_sweep_is_masked_before_the_cut()
    -> TestResult {
        const FRESH: &str = "fresh-secret-123456";
        let live = LiveBrowser::connect().await?;
        let server = FixtureServer::start(site_copy_pages()).await?;
        let tab = live
            .open(&server.url("127.0.0.1", "/site_copy/top.html?inproc"))
            .await?;
        let ready = "(() => { try { return !!field(); } catch (_) { return false; } })()";
        for _ in 0..200 {
            if tab.main_world(ready).await? == json!(true) {
                break;
            }
            time::sleep(Duration::from_millis(50)).await;
        }
        tab.main_world("field().value = 'stale-secret-0000'; copyField()")
            .await?;
        let env = live.env();
        let scope = OpScope::new(&env, count_op()?, CallHeader::default());
        let page = OpPage::admit(&live.playwright, &scope, &tab.page).await?;
        let world = page.world(&page.main_endpoint(), Place::Main).await?;
        // The site as each op's sweep finds it before its call. The page's own frame holds no
        // field, so its probe has nothing of its own to add when the user types on in the
        // same-origin child, and the page copies the new value into the top document.
        let live_world = |_gone| "the page's own frame is still there";
        let for_preview = site_values(&page, &scope, &world)
            .await?
            .map_err(live_world)?;
        let for_strict = site_values(&page, &scope, &world)
            .await?
            .map_err(live_world)?;
        tab.main_world(&format!("field().value = {FRESH:?}; copyField()"))
            .await?;

        let request = located("#long")?;
        let Masked { reply, site } =
            masked_call(&page, &scope, &world, &request, None, for_preview).await?;
        let Ok(ProbeReply::Read { value, sweep, .. }) = reply else {
            return Err(format!("a located read reads: {reply:?}").into());
        };
        let shown = site
            .with_own(&live.playwright, page.id(), sweep)
            .mask(value)
            .to_string();
        assert!(
            shown.contains("opened under is [privat"),
            "{shown}: the preview is cut inside the masked value"
        );
        assert_eq!(leaked_start(&shown, FRESH), None, "{shown}");

        let request = located(".who")?;
        let Masked { reply, site } =
            masked_call(&page, &scope, &world, &request, None, for_strict).await?;
        let Ok(ProbeReply::Strict { message }) = reply.map(|reply| site.mask_reply(reply)) else {
            return Err("two elements are .who".into());
        };
        assert!(
            message.contains("Signed in with [private]"),
            "{message}: the heading's preview holds the masked value"
        );
        assert_eq!(leaked_start(&message, FRESH), None, "{message}");
        live.close(&tab).await;
        Ok(())
    }
}
