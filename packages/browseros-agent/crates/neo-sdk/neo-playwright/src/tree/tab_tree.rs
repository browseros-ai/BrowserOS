//! Per-tab coordinator owning APC capture coalescing, cross-capture ref tracking, safe hydration,
//! and host-private node mirroring for `neo-playwright`.
//!
//! [`TabTree`] ties together the pieces of design sections 6, 7, and 8:
//! - [`PageCapture`] runs `Page.getAnnotatedPageContent` off the CDP reader loop, gates on
//!   `contract_version`, and coalesces overlapping attempts without caching completed trees.
//! - After each APC reply arrives, [`TabTree::capture`] probes every pre-capture cached world in
//!   parallel with `probe::describe` so only worlds proven alive across the entire capture bind to
//!   their documents; stale worlds are invalidated, retiring their documents before seq
//!   assignment.
//! - The tab's [`TabRefTable`] (`neo_snapshot_apc::refs`) maps `Ref ⇄ NodeKey` across captures.
//!   `DocTable` stays the authority on seqs and document lifetime: refs are issued under the seq
//!   `DocTable` assigned, and the table drains `DocTable`'s retirement queue, so a document's refs
//!   retire in lockstep with the document.
//! - [`PrivateMirror`] mirrors the private entry's marks by `DomNodeId` and [`SiteKey`]; every
//!   capture builds with a copy of them, so APC masking redacts host-entered values without a page
//!   round trip.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_core::FrameId;
use futures_util::future;
use neo_snapshot_apc::refs::{Ref, RefLookup, RefTarget, TabRefTable};
use neo_snapshot_apc::select::RefResolver;
use neo_snapshot_apc::tree::{
    AriaAsk, AriaRoot, AriaTooDeep, AriaTree, BuildOptions, DocSeq, DocToken, DomNodeId,
    FrameToken, NodeKey, SiteKey, SnapshotTree, TreeDoc,
};
use tokio::time;

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest};
use crate::target::AriaRefJump;
use crate::tree::{
    AttemptStart, CaptureError, CapturedTree, Hydrate, PageCapture, PrivateMirror, RefLanding,
    TabOriginState, hydrate, private_mirror,
};
#[cfg(test)]
use crate::tree::{HydrateOutcome, HydratedNode};
use crate::world::{DocContext, DocPlace, FrameSeq, FrameWorlds, IsolatedWorld, Pin, Stamp};
use crate::{ErrorCode, PlaywrightError, resolve};

/// What [`TabTree::aria`] emits: ai mode (refs) or Playwright's default, from where, how deep,
/// and with boxes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TreeAsk {
    pub(crate) ai: bool,
    pub(crate) root: AriaRoot,
    pub(crate) depth: Option<usize>,
    pub(crate) boxes: bool,
}

/// Per-tab APC capture, ref table, hydration, and private-node state.
///
/// Designed to live on `PageState` once the switch-over wires it; every internal mutex is a short
/// `std::sync::Mutex` that is never held across an `.await`.
pub(crate) struct TabTree {
    capture: PageCapture,
    refs: Arc<Mutex<TabRefTable>>,
    private: Arc<Mutex<PrivateMirror>>,
}

impl TabTree {
    /// Creates a new [`TabTree`] with the baseline BrowserOS APC contract version floor and the
    /// tab's initial [`TabOriginState`].
    #[must_use]
    pub(crate) fn new(origin_state: TabOriginState) -> Self {
        Self {
            capture: PageCapture::default(),
            refs: Arc::new(Mutex::new(TabRefTable::new())),
            private: Arc::new(Mutex::new(PrivateMirror::new(origin_state))),
        }
    }

    /// Creates a new [`TabTree`] enforcing at least `min_contract_version` (clamped up to
    /// `MIN_CONTRACT_VERSION`).
    #[must_use]
    #[cfg(test)]
    pub(crate) fn with_min_contract_version(
        origin_state: TabOriginState,
        min_contract_version: u32,
    ) -> Self {
        Self {
            capture: PageCapture::new(min_contract_version),
            refs: Arc::new(Mutex::new(TabRefTable::new())),
            private: Arc::new(Mutex::new(PrivateMirror::new(origin_state))),
        }
    }

    /// Captures and normalizes the tab's APC tree for an attempt that started at `started`.
    ///
    /// Overlapping attempts with `started < in_flight.requested_at` coalesce onto one CDP call.
    /// After the APC payload passes the contract gate, every cached world installed strictly
    /// before `requested_at` is probed with `probe::describe` in parallel:
    /// - Worlds answering `Described` proved they stayed alive across the capture and bind in
    ///   `DocTable`.
    /// - Worlds failing `Gone` are invalidated immediately and their retired documents' refs are
    ///   retired before new seqs are assigned (design section 6.3).
    /// - Worlds failing for any other reason are omitted from the proven map (fail closed).
    ///
    /// The tree is built with a copy of the private mirror's marks taken after the reply, so it
    /// masks every box marked before the reply arrived.
    pub(crate) async fn capture(
        &self,
        scope: &OpScope<'_>,
        page: &OpPage<'_>,
        started: AttemptStart,
    ) -> Result<Arc<CapturedTree>, PlaywrightError> {
        let root_session = page.root_session();
        let crash = scope.begin_protocol(&root_session);
        let page_state = Arc::clone(page.state());
        let main_frame = page.main_endpoint().frame;
        let refs = Arc::clone(&self.refs);
        let private = Arc::clone(&self.private);
        let min_contract_version = self.capture.min_contract_version();

        let outcome = self
            .capture
            .capture(root_session, started, move |apc, requested_at| async move {
                let proven = prove_pre_capture_worlds(page_state.frames(), requested_at).await;

                let main_doc_token = apc
                    .main_frame_data
                    .as_ref()
                    .and_then(|frame| frame.document_identifier.as_ref())
                    .and_then(|id| id.serialized_token.as_deref())
                    .and_then(DocToken::new);
                let main_url = apc
                    .main_frame_data
                    .as_ref()
                    .and_then(|frame| frame.url.clone());

                let seq_of = |doc_token: &DocToken, frame_token: Option<&FrameToken>| -> DocSeq {
                    let is_main = main_doc_token
                        .as_ref()
                        .is_some_and(|main| main == doc_token);
                    let child_frame = frame_token.map(|token| FrameId(token.as_str().to_owned()));
                    let (frame, place, url) = if is_main {
                        (Some(&main_frame), DocPlace::Main, main_url.as_deref())
                    } else {
                        (child_frame.as_ref(), DocPlace::Child, None)
                    };
                    let ctx = DocContext {
                        token: doc_token,
                        frame,
                        place,
                        url,
                        requested_at,
                    };
                    page_state.frames().seq_for_doc(ctx, &proven)
                };

                // Privacy layer 2's feed (design 8.1). The copy is taken after the reply: a
                // caller of `mark_private` mirrors a mark before the first host-typed character
                // reaches the page, so any APC reply that can hold that character arrived after
                // the mark, and a copy taken now has it (one taken before dispatch would miss a
                // mark made during the round trip). It is a copy, not the guard, so `mark_private`
                // on the typing path never blocks a thread on this std mutex for a whole build.
                // The copy costs one entry per box the host typed into on this tab.
                let marks = private
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .marks()
                    .clone();
                let options = BuildOptions::new()
                    .with_min_contract_version(min_contract_version)
                    .with_seq_of(&seq_of)
                    .with_private_nodes(&marks);
                let built = SnapshotTree::build(apc, options).map_err(CaptureError::Build);
                // Worlds the probe proved gone, and documents `seq_of` displaced from their frames,
                // retired in `DocTable` above; drop their refs even when the build failed.
                retire_refs_of_retired_docs(page_state.frames(), &refs);
                built
            })
            .await;

        match outcome {
            Ok(captured) => Ok(captured),
            Err(error) => {
                if let CaptureError::Cdp(core_error) = error.as_ref() {
                    crash.record(core_error);
                }
                Err(error.to_playwright_error(scope))
            }
        }
    }

    /// [`Self::capture`] for every op that prints the tree or matches against it: first, each
    /// document of a tab that was open before this `Playwright` started has its private marks
    /// read from its frame's world once (design 8.1, "Keeping the copy complete").
    ///
    /// A host may have typed a secret into such a document under an earlier instance, whose
    /// mark lives only in the page's isolated world. The capture that finds the document builds
    /// before the mirror knows that mark, so when a seed brings any mark in, the tab is captured
    /// again and that tree, which masks it, is the one returned. A document whose world does not
    /// answer stays unseeded: the tab is captured again, which a frame that left no longer shows,
    /// and a tree that still holds an unseeded document is never returned (fail closed).
    pub(crate) async fn capture_seeded(
        &self,
        scope: &OpScope<'_>,
        page: &OpPage<'_>,
        started: AttemptStart,
    ) -> Result<Arc<CapturedTree>, PlaywrightError> {
        let mut captured = self.capture(scope, page, started).await?;
        for pass in 0..=SEED_RECAPTURES {
            let (marked, unread) = self.seed_unseeded(scope, page, &captured).await?;
            if unread == 0 && marked == 0 {
                return Ok(captured);
            }
            if pass == SEED_RECAPTURES {
                break;
            }
            // A mark read in makes this tree stale: the next one masks it. A document that did
            // not answer may have left with its frame, which the next tree shows.
            captured = self.capture(scope, page, AttemptStart::now()).await?;
        }
        // Fail closed: a document whose marks could not be read could print in clear what a
        // host typed into it under an earlier instance.
        Err(scope.fail(
            ErrorCode::Internal,
            "a document open before this browser session did not answer for its private fields, \
             so it is not printed",
        ))
    }

    /// Seeds every document of `captured` that still needs it: how many marks came in, and how
    /// many documents could not be read.
    async fn seed_unseeded(
        &self,
        scope: &OpScope<'_>,
        page: &OpPage<'_>,
        captured: &CapturedTree,
    ) -> Result<(usize, usize), PlaywrightError> {
        let tree = captured.tree();
        let (mut marked, mut unread) = (0, 0);
        for doc in tree.documents() {
            if !self.needs_private_seed(doc.token()) {
                continue;
            }
            match self.seed_document(scope, page, tree, doc).await? {
                Seed::Read(count) => marked += count,
                Seed::Unread => unread += 1,
            }
        }
        Ok((marked, unread))
    }

    /// Reads `doc`'s private marks from its frame's world (the probe's `privateMarks`) into the
    /// mirror, on the document's own site, and records it seeded. The document's closed shadow
    /// roots are opened to that world first, every one the capture found, since `privateMarks`
    /// walks only the roots the world can reach. A document whose frame has no world to ask, or
    /// whose world does not answer within the budget, is [`Seed::Unread`] and stays unseeded.
    async fn seed_document(
        &self,
        scope: &OpScope<'_>,
        page: &OpPage<'_>,
        tree: &SnapshotTree,
        doc: &TreeDoc,
    ) -> Result<Seed, PlaywrightError> {
        let main_frame = page.main_endpoint().frame;
        let Some(frame) = hydrate::doc_frame(doc, &main_frame) else {
            return Ok(Seed::Unread);
        };
        // Twice, as a hydration installs: a world on the way can go away under the install.
        for _ in 0..2 {
            let world = match hydrate::ensure_world(page, scope, self, &frame).await? {
                hydrate::EnsuredWorld::Ready(world) => world,
                hydrate::EnsuredWorld::Gone => continue,
                hydrate::EnsuredWorld::Missing => return Ok(Seed::Unread),
            };
            // Bounded as a world's proof is: a hung frame leaves its document unseeded.
            let opened = time::timeout(
                PROOF_BUDGET,
                hydrate::open_closed_roots(scope, page, &world, tree, doc.id(), None),
            )
            .await;
            if !matches!(opened, Ok(true)) {
                return Ok(Seed::Unread);
            }
            let asked = time::timeout(
                PROOF_BUDGET,
                probe::call(scope, page, &world, &ProbeRequest::PrivateMarks),
            )
            .await;
            let Ok(asked) = asked else {
                return Ok(Seed::Unread);
            };
            let pins = match asked {
                Ok(ProbeReply::Marks { pins }) => pins,
                Ok(other) => return Err(resolve::refused(scope, other)),
                Err(ProbeFailure::Gone(_)) => {
                    self.invalidate_world(page.frames(), &world);
                    continue;
                }
                Err(failure) => return Err(resolve::failed(scope, failure)),
            };
            let nodes =
                match private_mirror::describe_pins_node_ids(scope, page, &world, pins).await {
                    Ok(nodes) => nodes,
                    Err(ProbeFailure::Gone(_)) => {
                        self.invalidate_world(page.frames(), &world);
                        continue;
                    }
                    Err(failure) => return Err(resolve::failed(scope, failure)),
                };
            let count = nodes.len();
            self.record_private_seed(doc, nodes);
            return Ok(Seed::Read(count));
        }
        Ok(Seed::Unread)
    }

    /// Hydrates `key` (expected in `frame`) against `capture` under section 6.2's four cases.
    #[cfg(test)]
    pub(crate) async fn hydrate_key(
        &self,
        scope: &OpScope<'_>,
        page: &OpPage<'_>,
        capture: &CapturedTree,
        key: &NodeKey,
        frame: &FrameId,
    ) -> Result<HydrateOutcome, PlaywrightError> {
        Hydrate::new(page, scope, self)
            .hydrate_key(capture, key, frame)
            .await
    }

    /// Lands a snapshot or minted ref jump under section 6.4 (zero captures when already bound):
    /// its element, or its live document's world when the element left it.
    pub(crate) async fn land_ref(
        &self,
        scope: &OpScope<'_>,
        page: &OpPage<'_>,
        jump: AriaRefJump,
    ) -> Result<RefLanding, PlaywrightError> {
        Hydrate::new(page, scope, self).land_ref(jump).await
    }

    /// [`Self::land_ref`] for the element itself: one that left fails `element_detached`.
    #[cfg(test)]
    pub(crate) async fn hydrate_ref(
        &self,
        scope: &OpScope<'_>,
        page: &OpPage<'_>,
        jump: AriaRefJump,
    ) -> Result<HydratedNode, PlaywrightError> {
        Hydrate::new(page, scope, self).hydrate_ref(jump).await
    }

    /// Invalidates `world` in `frames` and retires every document bound to that install in both
    /// `DocTable` and the ref table.
    pub(crate) fn invalidate_world(&self, frames: &FrameWorlds, world: &IsolatedWorld) {
        frames.invalidate(world);
        retire_refs_of_retired_docs(frames, &self.refs);
    }

    /// Retires `doc` in both `frames` (`DocTable`) and the ref table, returning its seq if it was
    /// previously active in `DocTable`.
    pub(crate) fn retire_doc(&self, frames: &FrameWorlds, doc: &DocToken) -> Option<FrameSeq> {
        let seq = frames.retire_doc(doc);
        retire_refs_of_retired_docs(frames, &self.refs);
        seq
    }

    /// `tree`'s aria tree under `ask`, which a snapshot prints. In ai mode the tab's ref table
    /// assigns the refs the text prints, under the seqs `DocTable` gave the tree's documents, so
    /// every printed ref is one `aria-ref` resolves; the lock is held for the emission only,
    /// which is synchronous.
    pub(crate) fn aria(&self, tree: &SnapshotTree, ask: TreeAsk) -> Result<AriaTree, AriaTooDeep> {
        let TreeAsk {
            ai,
            root,
            depth,
            boxes,
        } = ask;
        if ai {
            let mut refs = self.refs.lock().unwrap_or_else(PoisonError::into_inner);
            tree.aria(
                AriaAsk::ai(&mut refs)
                    .with_root(root)
                    .with_depth(depth)
                    .with_boxes(boxes),
            )
        } else {
            tree.aria(
                AriaAsk::default_mode()
                    .with_root(root)
                    .with_depth(depth)
                    .with_boxes(boxes),
            )
        }
    }

    /// Resolves `r` against the tab's ref table.
    #[must_use]
    pub(crate) fn resolve_ref(&self, r: &Ref) -> RefLookup {
        self.refs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .resolve(r)
    }

    /// Returns `key`'s printed ref, else its minted `n`-ref, minting one under `seq` if needed
    /// (`locator.ref()`).
    ///
    /// `seq` is the one `DocTable` assigned to `key.doc` in the capture that found `key`. Passing it
    /// keeps the table from numbering documents itself, which would print refs no `DocTable` seq
    /// resolves.
    pub(crate) fn ref_for(&self, seq: DocSeq, key: &NodeKey) -> Ref {
        self.refs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .ref_for(key, Some(seq))
    }

    /// Mirrors one host-private DOM node mark made on `site` (`None`: site unknown, so `node`
    /// masks in every document).
    ///
    /// The caller mirrors the mark before the first character is typed into the box: a capture
    /// copies the marks after its APC reply arrives, which masks only what was mirrored by then.
    #[cfg(test)]
    pub(crate) fn mark_private(&self, site: Option<SiteKey>, node: DomNodeId) {
        self.private
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .mark(site, node);
    }

    /// Resolves probe `pins` in `world` into [`DomNodeId`]s and mirrors every surviving pin as
    /// host-private on `site`, returning how many nodes were mirrored.
    ///
    /// As with [`Self::mark_private`], the caller mirrors the marks before the first character is
    /// typed into any of the boxes.
    pub(crate) async fn mark_private_pins(
        &self,
        scope: &OpScope<'_>,
        page: &OpPage<'_>,
        world: &IsolatedWorld,
        site: Option<SiteKey>,
        pins: Vec<Pin>,
    ) -> Result<usize, ProbeFailure> {
        let dom_ids = private_mirror::describe_pins_node_ids(scope, page, world, pins).await?;
        let count = dom_ids.len();
        let mut mirror = self.private.lock().unwrap_or_else(PoisonError::into_inner);
        for node in dom_ids {
            mirror.mark(site.clone(), node);
        }
        Ok(count)
    }

    /// Returns whether `doc` still needs a one-time page-side seed read for pre-existing tabs.
    #[must_use]
    pub(crate) fn needs_private_seed(&self, doc: &DocToken) -> bool {
        self.private
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .needs_seed(doc)
    }

    /// Imports the pre-existing private marks `doc`'s isolated world reported, on `doc`'s own
    /// site, and records `doc` seeded; zero `nodes` records an empty seed.
    pub(crate) fn record_private_seed(
        &self,
        doc: &TreeDoc,
        nodes: impl IntoIterator<Item = DomNodeId>,
    ) {
        self.private
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record_seeded(doc, nodes);
    }
}

impl fmt::Debug for TabTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("TabTree");
        debug.field("capture", &self.capture);
        {
            let refs = self.refs.lock().unwrap_or_else(PoisonError::into_inner);
            debug.field("refs", &*refs);
        }
        {
            let private = self.private.lock().unwrap_or_else(PoisonError::into_inner);
            debug.field("private", &*private);
        }
        debug.finish()
    }
}

/// A chunk run's `aria-ref` parts read the tab's live table, one short lock per lookup: a run
/// holding `&TabTree` across awaits never holds the lock across one, and a ref retired between
/// two of its stages answers `None` in the later one, as the page's live ref map would.
impl RefResolver for TabTree {
    fn resolve(&self, body: &str) -> Option<NodeKey> {
        let lookup = self
            .refs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .resolve_str(body);
        match lookup {
            // A click proxy (a zero-size control's shown label, design 3.5) changes how an
            // action presses the control, not which node the ref names.
            RefLookup::Key(RefTarget { key, .. }) => Some(key),
            RefLookup::Retired | RefLookup::Unknown => None,
        }
    }
}

/// How many more times [`TabTree::capture_seeded`] captures when a seed brought marks in or a
/// document did not answer, before it fails closed.
const SEED_RECAPTURES: usize = 2;

/// What reading one document's private marks came to.
enum Seed {
    /// The marks were read into the mirror, and the document recorded seeded: how many.
    Read(usize),
    /// No world of the document's frame answered in time, or the frame is gone: the document
    /// stays unseeded, and no tree that holds it may be returned.
    Unread,
}

/// How long a cached world has to answer the proof that it outlived a capture: the browser's own
/// deadline for a subframe's content (contract P6, `subframe_timeout_ms`), so a hung frame costs a
/// capture no more than it costs the capture itself.
const PROOF_BUDGET: Duration = Duration::from_secs(2);

/// Probes every cached world installed strictly before `requested_at` with `probe::describe` in
/// parallel after the APC reply arrives (design sections 6.2 and 6.3).
///
/// Worlds answering `Described` proved they stayed alive across the capture interval. Worlds
/// answering `Gone` are invalidated, which retires their documents in `DocTable` before `seq_of`
/// runs, so a navigated frame's new document takes a fresh seq instead of the dead one's.
async fn prove_pre_capture_worlds(
    frames: &FrameWorlds,
    requested_at: Stamp,
) -> HashMap<FrameId, IsolatedWorld> {
    let eligible: Vec<IsolatedWorld> = frames
        .cached_worlds()
        .into_iter()
        .filter(|world| world.installed_at() < requested_at)
        .collect();

    // Each proof waits at most as long as the browser waits for a subframe's content: a world
    // whose renderer hangs never answers, and without a bound every capture of its tab would
    // wait on it forever. A world that does not answer in time is left unproven, not gone.
    let checks = eligible.into_iter().map(|world| async move {
        let reply = time::timeout(PROOF_BUDGET, probe::describe(&world)).await;
        (world, reply)
    });
    let results = future::join_all(checks).await;

    let mut proven: HashMap<FrameId, IsolatedWorld> = HashMap::new();
    for (world, reply) in results {
        let Ok(reply) = reply else {
            continue;
        };
        match reply {
            Ok(ProbeReply::Described { .. }) => {
                let frame = world.frame().clone();
                let replace = proven
                    .get(&frame)
                    .is_none_or(|existing| world.installed_at() > existing.installed_at());
                if replace {
                    proven.insert(frame, world);
                }
            }
            Err(ProbeFailure::Gone(_)) => frames.invalidate(&world),
            // Fail closed on unproven worlds: leave them out of `proven` so the document reserves
            // a seq rather than binding an unverified install.
            Ok(_) | Err(_) => {}
        }
    }
    proven
}

/// Drops the refs of every document life `DocTable` retired since the last drain, so the ref
/// table retires in lockstep with `DocTable`, its authority on document lifetime (design 2.3).
///
/// The queue also carries retirements made outside `TabTree`, such as the resolver invalidating a
/// world that failed `Gone`; draining after every capture catches those before refs are printed.
/// It names the retired seq, not the token: a page back from the back/forward cache has a second
/// life under a fresh seq, which another capture may print refs for between this drain's two
/// locks, and retiring by token would delete them. The `FrameWorlds` lock is released before the
/// ref table's is taken, so the two never nest.
fn retire_refs_of_retired_docs(frames: &FrameWorlds, refs: &Mutex<TabRefTable>) {
    let retired = frames.take_retired_seqs();
    if retired.is_empty() {
        return;
    }
    let mut table = refs.lock().unwrap_or_else(PoisonError::into_inner);
    for seq in retired {
        table.retire_seq(DocSeq::new(seq.0));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, PoisonError};

    use browseros_core::FrameId;
    use neo_snapshot::{AriaChild, AriaNode};
    use neo_snapshot_apc::refs::{Ref, RefLookup, RefTarget, TabRefTable};
    use neo_snapshot_apc::select::RefResolver;
    use neo_snapshot_apc::tree::{
        ApcBuilder, AriaAsk, BuildOptions, DocSeq, DocToken, DomNodeId, MIN_CONTRACT_VERSION,
        NodeBuilder, NodeKey, SiteKey, SnapshotTree,
    };
    use serde_json::json;

    use crate::ErrorCode;
    use crate::op_scope::OpScope;
    use crate::page::OpPage;
    use crate::target::{AriaRefJump, CallHeader};
    use crate::test_support::{SCRIPTED_CHILD, ScriptedRun, ScriptedTab, TestResult, count_op};
    use crate::tree::private_mirror::tests::{Shown, page, shown};
    use crate::tree::tree_script::{
        ChildFrameSpec, TreeScript, apc_reply, encode_apc_reply, encode_apc_reply_nodes,
        encode_button_node, key,
    };
    use crate::tree::{AttemptStart, HydrateOutcome, TabOriginState, TabTree};
    use crate::world::{DocContext, DocLookup, DocPlace, FrameSeq, FrameWorlds, Pin, Place, Stamp};

    fn jump(raw: &str) -> TestResult<AriaRefJump> {
        Ok(AriaRefJump::parse(raw).ok_or("valid canonical ref")?)
    }

    fn parsed_ref(raw: &str) -> TestResult<Ref> {
        Ok(Ref::parse(raw).ok_or("valid canonical ref")?)
    }

    /// The ref the snapshot printed on the node named `name`.
    fn printed_ref(nodes: &[AriaNode], name: &str) -> Option<String> {
        let mut stack: Vec<&AriaNode> = nodes.iter().collect();
        while let Some(node) = stack.pop() {
            if node.name.as_deref() == Some(name) {
                return node.r#ref.clone();
            }
            stack.extend(node.children.iter().filter_map(|child| match child {
                AriaChild::Node(node) => Some(node.as_ref()),
                AriaChild::Text(_) => None,
            }));
        }
        None
    }

    /// What a ref resolves to when it names `key` and no click proxy stands in for it.
    fn live(key: &NodeKey) -> RefLookup {
        RefLookup::Key(RefTarget {
            key: key.clone(),
            click_proxy: None,
        })
    }

    /// Records `key` in the tab's ref table as a snapshot printing it would, and checks the table
    /// issued exactly `raw`: refs count per document seq, so a test reads which ref is which.
    fn assign_ref(tree: &TabTree, raw: &str, key: &NodeKey) -> TestResult<Ref> {
        let printed = parsed_ref(raw)?;
        let assigned = tree
            .refs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .assign_snapshot_ref(printed.seq(), key, "button", "", None);
        assert_eq!(assigned, printed, "the ref table must issue {raw} next");
        Ok(assigned)
    }

    #[tokio::test]
    async fn case_1_pre_capture_world_hydrates_without_recapture_and_subsequent_ref_needs_zero_captures()
    -> TestResult {
        let apc = encode_apc_reply(
            "doc-1",
            Some("https://a.test/"),
            &[10, 20],
            None,
            Some(MIN_CONTRACT_VERSION),
        );
        let tab = ScriptedTab::new(TreeScript::new(apc));
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let tree = TabTree::new(TabOriginState::OpenedUnderInstance);

        // Install main world strictly BEFORE taking Capture 1.
        let main_endpoint = page.main_endpoint();
        let main_world = page.world(&main_endpoint, Place::Main).await?;
        let captured = tree.capture(&scope, &page, AttemptStart::now()).await?;
        assert!(
            main_world.installed_at() < captured.requested_at(),
            "world installed before capture must strictly predate capture.requested_at"
        );
        assert_eq!(tab.script().apc_count(), 1);

        let node_key = key("doc-1", 10)?;
        let outcome = tree
            .hydrate_key(&scope, &page, &captured, &node_key, &main_endpoint.frame)
            .await?;
        let HydrateOutcome::Hydrated(hydrated) = outcome else {
            return Err(format!("expected Hydrated in case 1, got {outcome:?}").into());
        };
        assert_eq!(
            tab.script().apc_count(),
            1,
            "case 1 must hydrate through the pre-capture world with zero extra captures"
        );
        assert!(hydrated.world.same_install(&main_world));
        assert_eq!(hydrated.seq, FrameSeq(0));
        assert_eq!(hydrated.key, node_key);
        assert_eq!(hydrated.pin, Pin(1));
        assert!(
            hydrated.object_id.ends_with("-10"),
            "object_id must come from DOM.resolveNode for backendNodeId 10: {}",
            hydrated.object_id
        );
        assert!(
            hydrated.preview.contains(&hydrated.object_id),
            "preview must echo the pinned remote node handle: {}",
            hydrated.preview
        );
        assert!(hydrated.path.hops().is_empty());

        let resolve_calls = tab.script().resolve_node_calls();
        assert_eq!(resolve_calls.len(), 1);
        assert_eq!(resolve_calls[0]["backendNodeId"], 10);
        assert_eq!(
            resolve_calls[0]["executionContextId"],
            main_world.context().0
        );

        // Record a ref and hydrate via hydrate_ref: must succeed with ZERO captures.
        assign_ref(&tree, "e1", &node_key)?;
        let by_ref = tree.hydrate_ref(&scope, &page, jump("e1")?).await?;
        assert_eq!(by_ref.key, node_key);
        assert_eq!(by_ref.seq, FrameSeq(0));
        assert_eq!(
            tab.script().apc_count(),
            1,
            "hydrate_ref on a bound document must perform zero APC captures"
        );
        Ok(())
    }

    #[tokio::test]
    async fn case_2_stale_world_on_resolve_node_or_cross_world_probe_invalidates_and_retires_doc()
    -> TestResult {
        // (2a) DOM.resolveNode fails with stale execution context.
        let apc = encode_apc_reply(
            "doc-1",
            Some("https://a.test/"),
            &[10],
            None,
            Some(MIN_CONTRACT_VERSION),
        );
        let tab = ScriptedTab::new(TreeScript::new(apc.clone()));
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let tree = TabTree::new(TabOriginState::OpenedUnderInstance);

        let main_endpoint = page.main_endpoint();
        let main_world = page.world(&main_endpoint, Place::Main).await?;
        let captured = tree.capture(&scope, &page, AttemptStart::now()).await?;
        let node_key = key("doc-1", 10)?;
        let e1 = assign_ref(&tree, "e1", &node_key)?;

        tab.script()
            .mark_global_stale(main_world.global(), "Cannot find context with specified id");
        let outcome = tree
            .hydrate_key(&scope, &page, &captured, &node_key, &main_endpoint.frame)
            .await?;
        assert!(
            matches!(outcome, HydrateOutcome::WorldStale),
            "stale context on DOM.resolveNode must return WorldStale: {outcome:?}"
        );
        assert!(
            page.frames()
                .cached_world_for_frame(&main_endpoint.frame)
                .is_none(),
            "case 2 must invalidate the stale world from FrameWorlds"
        );
        assert!(
            matches!(page.frames().lookup_doc(&node_key.doc), DocLookup::Retired),
            "case 2 must retire the stale world's bound DocToken"
        );
        assert_eq!(
            tree.resolve_ref(&e1),
            RefLookup::Retired,
            "case 2 must retire the stale document's refs"
        );

        // (2b) DOM.resolveNode succeeds against a recycled context ID, and probe resolve fails
        // with V8's cross-world refusal ("same JavaScript world").
        let tab_b = ScriptedTab::new(TreeScript::new(apc));
        let run_b = ScriptedRun::over(&tab_b).await?;
        let scope_b = OpScope::new(&run_b.env, count_op()?, CallHeader::default());
        let page_b = OpPage::admit_handle(&run_b.playwright, &scope_b, &run_b.page).await?;
        let tree_b = TabTree::new(TabOriginState::OpenedUnderInstance);

        let main_b = page_b.main_endpoint();
        let _world_b = page_b.world(&main_b, Place::Main).await?;
        let captured_b = tree_b
            .capture(&scope_b, &page_b, AttemptStart::now())
            .await?;
        let key_b = key("doc-1", 10)?;
        assign_ref(&tree_b, "e1", &key_b)?;

        tab_b.script().set_resolve_probe_error(Some(
            "Argument should belong to the same JavaScript world as target object",
        ));
        let outcome_b = tree_b
            .hydrate_key(&scope_b, &page_b, &captured_b, &key_b, &main_b.frame)
            .await?;
        assert!(
            matches!(outcome_b, HydrateOutcome::WorldStale),
            "cross-world V8 refusal on probe call_on must map to WorldStale: {outcome_b:?}"
        );
        assert!(
            page_b
                .frames()
                .cached_world_for_frame(&main_b.frame)
                .is_none(),
            "cross-world refusal must invalidate the stale world"
        );
        assert_eq!(
            tree_b.resolve_ref(&e1),
            RefLookup::Retired,
            "cross-world refusal must retire the stale document's refs"
        );
        Ok(())
    }

    #[tokio::test]
    async fn case_3_missing_world_installs_and_recaptures_once() -> TestResult {
        let apc = encode_apc_reply(
            "doc-1",
            Some("https://a.test/"),
            &[10],
            None,
            Some(MIN_CONTRACT_VERSION),
        );
        let tab = ScriptedTab::new(TreeScript::new(apc));
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let tree = TabTree::new(TabOriginState::OpenedUnderInstance);

        let main_frame = page.main_endpoint().frame;
        let captured = tree.capture(&scope, &page, AttemptStart::now()).await?;
        let doc_1 = DocToken::new("doc-1").ok_or("doc token")?;
        assert!(
            matches!(
                page.frames().lookup_doc(&doc_1),
                DocLookup::Reserved {
                    seq: FrameSeq(0),
                    ..
                }
            ),
            "worldless capture must reserve seq 0 for main document"
        );
        assert_eq!(tab.script().apc_count(), 1);

        let node_key = NodeKey::new(doc_1.clone(), DomNodeId::new(10));
        let outcome = tree
            .hydrate_key(&scope, &page, &captured, &node_key, &main_frame)
            .await?;
        let HydrateOutcome::Hydrated(hydrated) = outcome else {
            return Err(format!("expected Hydrated in case 3a, got {outcome:?}").into());
        };
        assert_eq!(
            tab.script().apc_count(),
            2,
            "case 3a must install the world and take exactly one fresh recapture"
        );
        assert_eq!(
            hydrated.seq,
            FrameSeq(0),
            "binding after case 3a must preserve the document's reserved seq"
        );
        assert!(
            matches!(
                page.frames().lookup_doc(&doc_1),
                DocLookup::Bound {
                    seq: FrameSeq(0),
                    ..
                }
            ),
            "doc-1 must promote from Reserved to Bound at seq 0"
        );
        Ok(())
    }

    #[tokio::test]
    async fn case_3_post_capture_world_refuses_direct_hydration_and_recaptures_once() -> TestResult
    {
        let apc = encode_apc_reply(
            "doc-1",
            Some("https://a.test/"),
            &[10],
            None,
            Some(MIN_CONTRACT_VERSION),
        );
        let tab = ScriptedTab::new(TreeScript::new(apc));
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let tree = TabTree::new(TabOriginState::OpenedUnderInstance);

        let main_endpoint = page.main_endpoint();
        let capture_1 = tree.capture(&scope, &page, AttemptStart::now()).await?;
        let world_after = page.world(&main_endpoint, Place::Main).await?;
        assert!(
            world_after.installed_at() >= capture_1.requested_at(),
            "world installed after capture_1 must not satisfy pre-capture proof"
        );
        assert_eq!(tab.script().apc_count(), 1);

        let node_key = key("doc-1", 10)?;
        let outcome = tree
            .hydrate_key(&scope, &page, &capture_1, &node_key, &main_endpoint.frame)
            .await?;
        let HydrateOutcome::Hydrated(hydrated) = outcome else {
            return Err(format!("expected Hydrated in case 3b, got {outcome:?}").into());
        };
        assert_eq!(
            tab.script().apc_count(),
            2,
            "case 3b must recapture when the cached world was installed after the capture"
        );
        assert!(hydrated.world.same_install(&world_after));
        assert_eq!(hydrated.seq, FrameSeq(0));
        Ok(())
    }

    #[tokio::test]
    async fn case_3_child_frame_enters_via_frame_start_recaptures_and_preserves_reserved_seq()
    -> TestResult {
        let child_apc = encode_apc_reply(
            "doc-main",
            Some("https://a.test/"),
            &[10],
            Some(&ChildFrameSpec {
                iframe_dom_id: 500,
                doc_token: "doc-child",
                frame_token: SCRIPTED_CHILD,
                url: Some("https://a.test/child"),
                child_dom_ids: &[33],
            }),
            Some(MIN_CONTRACT_VERSION),
        );
        let tab = ScriptedTab::new(TreeScript::new(child_apc).with_child());
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let tree = TabTree::new(TabOriginState::OpenedUnderInstance);

        let child_capture = tree.capture(&scope, &page, AttemptStart::now()).await?;
        let child_key = key("doc-child", 33)?;
        let child_frame = FrameId(SCRIPTED_CHILD.to_owned());
        let outcome = tree
            .hydrate_key(&scope, &page, &child_capture, &child_key, &child_frame)
            .await?;
        let HydrateOutcome::Hydrated(hydrated) = outcome else {
            return Err(format!("expected Hydrated child in case 3c, got {outcome:?}").into());
        };
        assert_eq!(
            tab.script().apc_count(),
            2,
            "child frame case 3 must install via FrameStart and recapture exactly once"
        );
        assert_eq!(hydrated.world.frame(), &child_frame);
        assert_eq!(
            hydrated.seq,
            FrameSeq(1),
            "child document must keep its reserved seq 1 when bound"
        );
        assert_eq!(
            hydrated.path.hops().len(),
            1,
            "child frame hydration must carry its parent iframe hop"
        );
        Ok(())
    }

    #[tokio::test]
    async fn case_4_stale_document_and_cross_frame_mismatch_refuse_without_resolve_node()
    -> TestResult {
        let with_child = encode_apc_reply(
            "doc-1",
            Some("https://a.test/"),
            &[10],
            Some(&ChildFrameSpec {
                iframe_dom_id: 500,
                doc_token: "doc-child",
                frame_token: SCRIPTED_CHILD,
                url: Some("https://a.test/child"),
                child_dom_ids: &[20],
            }),
            Some(MIN_CONTRACT_VERSION),
        );
        // The page then navigates: a new main document, and the iframe is gone.
        let navigated = encode_apc_reply(
            "doc-2",
            Some("https://b.test/"),
            &[10],
            None,
            Some(MIN_CONTRACT_VERSION),
        );
        let tab = ScriptedTab::new(TreeScript::new(navigated));
        tab.script().push_apc(with_child);
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let tree = TabTree::new(TabOriginState::OpenedUnderInstance);
        let main_frame = page.main_endpoint().frame;
        let child_frame = FrameId(SCRIPTED_CHILD.to_owned());

        let before = tree.capture(&scope, &page, AttemptStart::now()).await?;
        let main_key = key("doc-1", 10)?;
        let main_ref = assign_ref(&tree, "e1", &main_key)?;
        let child_key = key("doc-child", 20)?;
        let child_ref = assign_ref(&tree, "f1e1", &child_key)?;

        let mismatch = tree
            .hydrate_key(&scope, &page, &before, &child_key, &main_frame)
            .await?;
        assert!(
            matches!(mismatch, HydrateOutcome::StaleDocument),
            "key belonging to another frame must return StaleDocument: {mismatch:?}"
        );
        assert_eq!(
            tree.resolve_ref(&child_ref),
            live(&child_key),
            "frame mismatch for a document still live in another frame must not retire it"
        );

        let after = tree.capture(&scope, &page, AttemptStart::now()).await?;
        let outcome = tree
            .hydrate_key(&scope, &page, &after, &main_key, &main_frame)
            .await?;
        assert!(
            matches!(outcome, HydrateOutcome::StaleDocument),
            "key whose doc is absent from capture must return StaleDocument: {outcome:?}"
        );
        assert_eq!(
            tree.resolve_ref(&main_ref),
            RefLookup::Retired,
            "a main document displaced by navigation must retire its refs"
        );

        // Nothing displaced doc-child: its frame simply left. Case 4 must find it absent and
        // retire it, refs included.
        assert_eq!(
            tree.resolve_ref(&child_ref),
            live(&child_key),
            "doc-child stays live until a hydration proves it absent"
        );
        let gone = tree
            .hydrate_key(&scope, &page, &after, &child_key, &child_frame)
            .await?;
        assert!(
            matches!(gone, HydrateOutcome::StaleDocument),
            "key whose frame left the capture must return StaleDocument: {gone:?}"
        );
        assert!(
            matches!(page.frames().lookup_doc(&child_key.doc), DocLookup::Retired),
            "case 4 must retire the absent document in DocTable"
        );
        assert_eq!(
            tree.resolve_ref(&child_ref),
            RefLookup::Retired,
            "absent document detected in case 4 must retire its refs"
        );
        assert!(
            tab.script().resolve_node_calls().is_empty(),
            "case 4 must never send DOM.resolveNode for a stale or mismatched key"
        );
        Ok(())
    }

    #[tokio::test]
    async fn case_4_navigation_between_case_3_install_and_recapture_returns_stale_document()
    -> TestResult {
        let apc_nav_1 = encode_apc_reply(
            "doc-before",
            Some("https://a.test/"),
            &[10],
            None,
            Some(MIN_CONTRACT_VERSION),
        );
        let apc_nav_2 = encode_apc_reply(
            "doc-after",
            Some("https://b.test/"),
            &[10],
            None,
            Some(MIN_CONTRACT_VERSION),
        );
        let tab = ScriptedTab::new(TreeScript::new(apc_nav_1.clone()));
        tab.script().push_apc(apc_nav_1);
        let tab_hook = Arc::clone(&tab);
        tab.script().set_after_world_install(move || {
            tab_hook.script().set_default_apc(apc_nav_2.clone());
        });
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let tree = TabTree::new(TabOriginState::OpenedUnderInstance);

        let main_frame = page.main_endpoint().frame;
        let cap_before = tree.capture(&scope, &page, AttemptStart::now()).await?;
        let before_key = key("doc-before", 10)?;
        let before_ref = assign_ref(&tree, "e1", &before_key)?;

        let outcome = tree
            .hydrate_key(&scope, &page, &cap_before, &before_key, &main_frame)
            .await?;
        assert!(
            matches!(outcome, HydrateOutcome::StaleDocument),
            "navigation between case 3 install and recapture must return StaleDocument: {outcome:?}"
        );
        assert!(
            tab.script().resolve_node_calls().is_empty(),
            "navigation before recapture must prevent DOM.resolveNode into the new document"
        );
        assert_eq!(tree.resolve_ref(&before_ref), RefLookup::Retired);
        Ok(())
    }

    #[tokio::test]
    async fn case_4_reserved_ref_whose_doc_navigated_away_fails_detached_without_resolve_node()
    -> TestResult {
        let apc_r1 = encode_apc_reply(
            "doc-r1",
            Some("https://a.test/"),
            &[10],
            None,
            Some(MIN_CONTRACT_VERSION),
        );
        let apc_r2 = encode_apc_reply(
            "doc-r2",
            Some("https://b.test/"),
            &[10],
            None,
            Some(MIN_CONTRACT_VERSION),
        );
        let tab = ScriptedTab::new(TreeScript::new(apc_r2));
        tab.script().push_apc(apc_r1);
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let tree = TabTree::new(TabOriginState::OpenedUnderInstance);

        let _cap_r1 = tree.capture(&scope, &page, AttemptStart::now()).await?;
        let r1_key = key("doc-r1", 10)?;
        assign_ref(&tree, "e1", &r1_key)?;

        let err = tree
            .hydrate_ref(&scope, &page, jump("e1")?)
            .await
            .err()
            .ok_or("reserved ref whose doc navigated away must fail")?;
        assert_eq!(
            err.error_code(),
            Some(ErrorCode::ElementDetached),
            "stale reserved ref must fail ElementDetached: {err}"
        );
        assert!(
            tab.script().resolve_node_calls().is_empty(),
            "stale reserved ref must never send DOM.resolveNode into doc-r2"
        );
        assert!(
            matches!(page.frames().lookup_doc(&r1_key.doc), DocLookup::Retired),
            "stale reserved doc must retire in DocTable"
        );
        Ok(())
    }

    #[tokio::test]
    async fn section_6_3_snapshot_right_after_navigation_retires_old_world_and_reserves_new_seq()
    -> TestResult {
        let apc_1 = encode_apc_reply(
            "doc-1",
            Some("https://a.test/"),
            &[10],
            None,
            Some(MIN_CONTRACT_VERSION),
        );
        let apc_2 = encode_apc_reply(
            "doc-2",
            Some("https://b.test/"),
            &[10, 20],
            None,
            Some(MIN_CONTRACT_VERSION),
        );
        let tab = ScriptedTab::new(TreeScript::new(apc_1));
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let tree = TabTree::new(TabOriginState::OpenedUnderInstance);

        // 1. Install W_old (seq 0) on doc-1 and take Capture 1 -> Describe succeeds -> binds doc-1
        //    to seq 0.
        let main_endpoint = page.main_endpoint();
        let w_old = page.world(&main_endpoint, Place::Main).await?;
        assert_eq!(w_old.seq(), FrameSeq(0));

        let cap_1 = tree.capture(&scope, &page, AttemptStart::now()).await?;
        let doc_1 = cap_1.tree().main_document().ok_or("doc 1")?;
        assert_eq!(doc_1.token().as_str(), "doc-1");
        assert_eq!(doc_1.seq(), Some(DocSeq::new(0)));
        let e1 = assign_ref(&tree, "e1", &key("doc-1", 10)?)?;

        // 2. Simulate navigation to doc-2 (https://b.test/): APC returns doc-2, and Describe on
        //    W_old fails Gone.
        tab.script().set_default_apc(apc_2);
        tab.script()
            .mark_global_stale(w_old.global(), "Cannot find context with specified id");

        // 3. Take Capture 2: W_old is invalidated, doc-1 + e1 retire, doc-2 reserves seq 1 (NOT 0).
        let cap_2 = tree.capture(&scope, &page, AttemptStart::now()).await?;
        let doc_2 = cap_2.tree().main_document().ok_or("doc 2")?;
        assert_eq!(doc_2.token().as_str(), "doc-2");
        assert_eq!(
            doc_2.seq(),
            Some(DocSeq::new(1)),
            "new document after navigation must reserve seq 1, never reuse dead doc-1's seq 0"
        );
        assert!(
            page.frames()
                .cached_world_for_frame(&main_endpoint.frame)
                .is_none(),
            "Capture 2's failed Describe must invalidate W_old"
        );
        assert_eq!(
            tree.resolve_ref(&e1),
            RefLookup::Retired,
            "Capture 2 must retire old doc-1's ref e1"
        );

        assign_ref(&tree, "f1e1", &key("doc-2", 10)?)?;
        assign_ref(&tree, "f1e2", &key("doc-2", 20)?)?;

        // 4. Old ref e1 fails ElementDetached with ZERO extra captures.
        let captures_before_old = tab.script().apc_count();
        let old_err = tree
            .hydrate_ref(&scope, &page, jump("e1")?)
            .await
            .err()
            .ok_or("old e1 must fail after navigation")?;
        assert_eq!(old_err.error_code(), Some(ErrorCode::ElementDetached));
        assert_eq!(
            tab.script().apc_count(),
            captures_before_old,
            "retired ref e1 must fail without capturing"
        );

        // 5. Clicking f1e1 runs Case 3 once, binds W_new to reserved seq 1.
        let first_hydrated = tree.hydrate_ref(&scope, &page, jump("f1e1")?).await?;
        assert_eq!(
            tab.script().apc_count(),
            captures_before_old + 1,
            "first click on reserved f1e1 must run Case 3 with exactly one recapture"
        );
        assert_eq!(
            first_hydrated.seq,
            FrameSeq(1),
            "W_new must answer to doc-2's reserved seq 1"
        );
        assert!(!first_hydrated.world.same_install(&w_old));
        assert_eq!(first_hydrated.key, key("doc-2", 10)?);

        // 6. Second ref f1e2 hydrates with ZERO additional captures.
        let captures_before_second = tab.script().apc_count();
        let second_hydrated = tree.hydrate_ref(&scope, &page, jump("f1e2")?).await?;
        assert_eq!(
            tab.script().apc_count(),
            captures_before_second,
            "subsequent ref f1e2 in now-bound doc-2 must hydrate with zero captures"
        );
        assert_eq!(second_hydrated.seq, FrameSeq(1));
        assert_eq!(second_hydrated.key, key("doc-2", 20)?);
        assert!(second_hydrated.world.same_install(&first_hydrated.world));
        Ok(())
    }

    #[tokio::test]
    async fn a_ref_printed_right_after_navigation_reaches_the_element_it_names() -> TestResult {
        let apc_1 = encode_apc_reply_nodes(
            "doc-1",
            Some("https://a.test/"),
            &[encode_button_node(10, "Back")],
            None,
            Some(MIN_CONTRACT_VERSION),
        );
        let apc_2 = encode_apc_reply_nodes(
            "doc-2",
            Some("https://b.test/"),
            &[encode_button_node(10, "Pay")],
            None,
            Some(MIN_CONTRACT_VERSION),
        );
        let tab = ScriptedTab::new(TreeScript::new(apc_1));
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let tree = TabTree::new(TabOriginState::OpenedUnderInstance);

        let w_old = page.world(&page.main_endpoint(), Place::Main).await?;
        let _before = tree.capture(&scope, &page, AttemptStart::now()).await?;
        tab.script().set_default_apc(apc_2);
        tab.script()
            .mark_global_stale(w_old.global(), "Cannot find context with specified id");

        let after = tree.capture(&scope, &page, AttemptStart::now()).await?;
        let snapshot = {
            let mut refs = tree.refs.lock().unwrap_or_else(PoisonError::into_inner);
            after.tree().aria(AriaAsk::ai(&mut refs))?
        };
        let printed = printed_ref(snapshot.nodes(), "Pay").ok_or("Pay must print a ref")?;
        assert_eq!(
            printed, "f1e1",
            "the new document's refs must print under the seq DocTable reserved for it"
        );

        let captures = tab.script().apc_count();
        let hydrated = tree.hydrate_ref(&scope, &page, jump(&printed)?).await?;
        assert_eq!(
            hydrated.key,
            key("doc-2", 10)?,
            "aria-ref={printed} must reach the element the snapshot printed it on"
        );
        assert_eq!(
            hydrated.seq,
            FrameSeq(1),
            "the world installed for doc-2 must answer to the seq its refs printed under"
        );
        assert!(
            !hydrated.world.same_install(&w_old),
            "the dead document's world must never host the new document's element"
        );
        assert_eq!(
            tab.script().apc_count(),
            captures + 1,
            "the first use of a reserved document's ref proves it with one capture"
        );
        Ok(())
    }

    #[tokio::test]
    async fn section_6_4_refs_without_capture_cover_bound_retired_foreign_unknown_and_detached_node()
    -> TestResult {
        let apc = encode_apc_reply(
            "doc-1",
            Some("https://a.test/"),
            &[10, 20],
            None,
            Some(MIN_CONTRACT_VERSION),
        );
        // Boot reports a retired foreign seq 77 from an earlier Playwright instance.
        let tab = ScriptedTab::new(TreeScript::new(apc).with_rekey_retired_seq(77));
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let tree = TabTree::new(TabOriginState::OpenedUnderInstance);

        let main_endpoint = page.main_endpoint();
        let main_world = page.world(&main_endpoint, Place::Main).await?;
        let _captured = tree.capture(&scope, &page, AttemptStart::now()).await?;
        let key_10 = key("doc-1", 10)?;
        let key_20 = key("doc-1", 20)?;
        assign_ref(&tree, "e1", &key_10)?;
        assign_ref(&tree, "e2", &key_20)?;
        let baseline_captures = tab.script().apc_count();

        // Bound ref hydrates with 0 captures.
        let hydrated = tree.hydrate_ref(&scope, &page, jump("e1")?).await?;
        assert_eq!(hydrated.key, key_10);
        assert_eq!(tab.script().apc_count(), baseline_captures);

        // Foreign seq (from an earlier Playwright process) fails ElementDetached with 0 captures.
        let foreign_err = tree
            .hydrate_ref(&scope, &page, jump("f77e1")?)
            .await
            .err()
            .ok_or("foreign seq must fail")?;
        assert_eq!(
            foreign_err.error_code(),
            Some(ErrorCode::ElementDetached),
            "foreign retired seq must fail ElementDetached: {foreign_err}"
        );
        assert_eq!(tab.script().apc_count(), baseline_captures);

        // A never-issued seq is no ref at all (InvalidInput); an unknown ref number under a live
        // document's seq names an element that is not on the page (ElementDetached), as the
        // page's own ref map answered before the switch-over (frames_live). Neither captures.
        for (unknown, code) in [
            ("f99e1", ErrorCode::InvalidInput),
            ("e99", ErrorCode::ElementDetached),
        ] {
            let err = tree
                .hydrate_ref(&scope, &page, jump(unknown)?)
                .await
                .err()
                .ok_or("unknown ref must fail")?;
            assert_eq!(
                err.error_code(),
                Some(code),
                "unknown ref {unknown} must fail {code:?}: {err}"
            );
            assert_eq!(tab.script().apc_count(), baseline_captures);
        }

        // Node removed in live doc (DOM.resolveNode "No node with given id found") fails
        // ElementDetached WITHOUT invalidating the live world.
        tab.script()
            .set_resolve_node_error(Some("No node with given id found"));
        let missing_err = tree
            .hydrate_ref(&scope, &page, jump("e1")?)
            .await
            .err()
            .ok_or("removed DOM node must fail")?;
        assert_eq!(missing_err.error_code(), Some(ErrorCode::ElementDetached));
        tab.script().set_resolve_node_error(None);
        assert!(
            page.frames()
                .cached_world_for_frame(&main_endpoint.frame)
                .is_some_and(|w| w.same_install(&main_world)),
            "missing DOM node in live doc must not invalidate the live world"
        );

        // Node detached in probe reply (`Detached`) also fails ElementDetached WITHOUT
        // invalidating the live world.
        tab.script()
            .set_resolve_probe_reply(Some(json!({ "kind": "detached" })));
        let detached_err = tree
            .hydrate_ref(&scope, &page, jump("e1")?)
            .await
            .err()
            .ok_or("detached probe node must fail")?;
        assert_eq!(detached_err.error_code(), Some(ErrorCode::ElementDetached));
        tab.script().set_resolve_probe_reply(None);

        // Sibling live ref e2 still hydrates immediately with 0 captures.
        let survivor = tree.hydrate_ref(&scope, &page, jump("e2")?).await?;
        assert_eq!(survivor.key, key_20);
        assert_eq!(tab.script().apc_count(), baseline_captures);

        // Explicitly retiring the doc makes its ref fail ElementDetached with 0 captures.
        let retired_seq = tree.retire_doc(page.frames(), &key_10.doc);
        assert_eq!(retired_seq, Some(FrameSeq(0)));
        let retired_err = tree
            .hydrate_ref(&scope, &page, jump("e2")?)
            .await
            .err()
            .ok_or("retired ref must fail")?;
        assert_eq!(retired_err.error_code(), Some(ErrorCode::ElementDetached));
        assert_eq!(tab.script().apc_count(), baseline_captures);
        Ok(())
    }

    #[tokio::test]
    async fn refs_follow_a_retirement_made_outside_tab_tree_at_the_next_capture() -> TestResult {
        let apc_1 = encode_apc_reply(
            "doc-1",
            Some("https://a.test/"),
            &[10],
            None,
            Some(MIN_CONTRACT_VERSION),
        );
        let apc_2 = encode_apc_reply(
            "doc-2",
            Some("https://b.test/"),
            &[10],
            None,
            Some(MIN_CONTRACT_VERSION),
        );
        let tab = ScriptedTab::new(TreeScript::new(apc_1));
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let tree = TabTree::new(TabOriginState::OpenedUnderInstance);

        let world = page.world(&page.main_endpoint(), Place::Main).await?;
        let _bound = tree.capture(&scope, &page, AttemptStart::now()).await?;
        let doc_1_key = key("doc-1", 10)?;
        assert!(
            matches!(
                page.frames().lookup_doc(&doc_1_key.doc),
                DocLookup::Bound { .. }
            ),
            "the pre-capture world must bind doc-1"
        );
        let e1 = assign_ref(&tree, "e1", &doc_1_key)?;

        // The resolver invalidates a world that failed `Gone` through `FrameWorlds` directly,
        // never through `TabTree`; the page has navigated to doc-2.
        page.frames().invalidate(&world);
        tab.script().set_default_apc(apc_2);
        let _next = tree.capture(&scope, &page, AttemptStart::now()).await?;
        assert_eq!(
            tree.resolve_ref(&e1),
            RefLookup::Retired,
            "a document retired outside TabTree must lose its refs by the next capture"
        );
        Ok(())
    }

    /// A page back from the back/forward cache prints refs under a fresh seq while its first
    /// visit's retirement may still wait in the queue. Draining that retirement must drop the first
    /// visit's refs and spare the restored page's.
    #[test]
    fn draining_a_retired_visit_spares_the_restored_pages_refs() -> TestResult {
        let frames = FrameWorlds::new(FrameSeq(0));
        let refs = Mutex::new(TabRefTable::new());
        let main = FrameId("main".to_owned());
        let button = key("doc-page", 10)?;
        let other = DocToken::new("doc-other-page").ok_or("non-empty doc token")?;
        let report = |token: &DocToken| {
            frames.seq_for_doc(
                DocContext {
                    token,
                    frame: Some(&main),
                    place: DocPlace::Main,
                    url: Some("https://a.test/"),
                    requested_at: Stamp::next(),
                },
                &HashMap::new(),
            )
        };
        let assign = |seq: DocSeq| {
            refs.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .assign_snapshot_ref(seq, &button, "button", "Pay", None)
        };

        let first_ref = assign(report(&button.doc));
        let _ = report(&other);
        let restored_ref = assign(report(&button.doc));
        assert_ne!(
            restored_ref.seq(),
            first_ref.seq(),
            "the restored page prints under a fresh seq"
        );
        super::retire_refs_of_retired_docs(&frames, &refs);

        let table = refs.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(
            table.resolve(&restored_ref),
            live(&button),
            "a drain of the first visit must not delete the restored page's refs"
        );
        assert_eq!(
            table.resolve(&first_ref),
            RefLookup::Retired,
            "the first visit's refs must answer element_detached"
        );
        Ok(())
    }

    /// A chunk run's `aria-ref` parts resolve through the tab's table as it stands at each
    /// lookup, retirements included.
    #[test]
    fn a_runs_ref_parts_resolve_through_the_tabs_live_table() -> TestResult {
        let frames = FrameWorlds::new(FrameSeq(0));
        let tree = TabTree::new(TabOriginState::OpenedUnderInstance);
        let button = key("doc-page", 10)?;
        let checkbox = key("doc-page", 20)?;
        let label = key("doc-page", 21)?;
        let located = key("doc-page", 30)?;
        let seq = frames.seq_for_doc(
            DocContext {
                token: &button.doc,
                frame: Some(&FrameId("main".to_owned())),
                place: DocPlace::Main,
                url: Some("https://a.test/"),
                requested_at: Stamp::next(),
            },
            &HashMap::new(),
        );
        let (printed, proxied) = {
            let mut table = tree.refs.lock().unwrap_or_else(PoisonError::into_inner);
            let printed = table.assign_snapshot_ref(seq, &button, "button", "Pay", None);
            let proxied =
                table.assign_snapshot_ref(seq, &checkbox, "checkbox", "Accept", Some(label));
            (printed, proxied)
        };
        let minted = tree.ref_for(seq, &located);
        let refs: &(dyn RefResolver + Sync) = &tree;

        assert_eq!(
            refs.resolve(&printed.to_string()),
            Some(button.clone()),
            "a printed ref names the node the snapshot printed it on"
        );
        assert_eq!(
            refs.resolve(&minted.to_string()),
            Some(located),
            "a minted ref names the node `locator.ref()` minted it for"
        );
        assert_eq!(
            refs.resolve(&proxied.to_string()),
            Some(checkbox),
            "a click proxy changes how an action presses the control, not which node it is"
        );
        for body in ["e99", "x1", "e01", ""] {
            assert_eq!(
                refs.resolve(body),
                None,
                "{body:?} names no live node: never issued, or not a ref at all"
            );
        }
        assert_eq!(
            tree.retire_doc(&frames, &button.doc),
            Some(FrameSeq(seq.raw()))
        );
        assert_eq!(
            refs.resolve(&printed.to_string()),
            None,
            "a ref retired with its document names no live node at the next lookup"
        );
        Ok(())
    }

    #[tokio::test]
    async fn contract_gate_refuses_stock_browser_apc_payload_as_unsupported() -> TestResult {
        let stock_apc = encode_apc_reply("doc-stock", Some("https://a.test/"), &[10], None, None);
        let tab = ScriptedTab::new(TreeScript::new(stock_apc));
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let tree = TabTree::with_min_contract_version(
            TabOriginState::ExistedAtStart,
            MIN_CONTRACT_VERSION,
        );

        let err = tree
            .capture(&scope, &page, AttemptStart::now())
            .await
            .err()
            .ok_or("stock APC must fail contract gate")?;
        assert_eq!(
            err.error_code(),
            Some(ErrorCode::Unsupported),
            "stock Chromium APC payload must fail with ErrorCode::Unsupported: {err}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn private_marks_mirror_and_seed_into_captured_trees() -> TestResult {
        let ids = [42, 84, 99, 123, 7];
        let on_a = Some(("https://a.test", false));
        let tab = ScriptedTab::new(TreeScript::new(apc_reply(&page("doc-private", on_a, &ids))));
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let op_page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let tree = TabTree::new(TabOriginState::ExistedAtStart);

        tab.script().hold_pin(Pin(1), 42);
        tab.script().hold_pin(Pin(2), 84);
        let main_world = op_page.world(&op_page.main_endpoint(), Place::Main).await?;
        // The private entry's world reads its origin in another spelling than APC's.
        let site_a = SiteKey::from_origin("https://A.TEST:443");
        let mirrored = tree
            .mark_private_pins(
                &scope,
                &op_page,
                &main_world,
                site_a.clone(),
                vec![Pin(1), Pin(2)],
            )
            .await?;
        assert_eq!(
            mirrored, 2,
            "mark_private_pins must mirror every surviving pinned DomNodeId"
        );
        tree.mark_private(site_a, DomNodeId::new(99));

        let first = tree.capture(&scope, &op_page, AttemptStart::now()).await?;
        for id in [42, 84, 99] {
            assert_eq!(
                shown(first.tree(), id)?,
                Shown::Bullets,
                "box {id}, mirrored before the capture, must print bullets"
            );
        }
        for id in [123, 7] {
            assert_eq!(
                shown(first.tree(), id)?,
                Shown::Value,
                "box {id} carries no mirrored mark yet"
            );
        }

        // The tab predates this instance: the first snapshot seeds its document's earlier marks.
        let doc = first.tree().main_document().ok_or("main doc")?;
        assert!(tree.needs_private_seed(doc.token()));
        tree.record_private_seed(doc, [DomNodeId::new(123)]);
        assert!(
            !tree.needs_private_seed(doc.token()),
            "a document is seeded once"
        );

        tab.script()
            .push_apc(apc_reply(&page("doc-moved", on_a, &ids)));
        tab.script().push_apc(apc_reply(&page(
            "doc-other",
            Some(("https://b.test", false)),
            &ids,
        )));
        let moved = tree.capture(&scope, &op_page, AttemptStart::now()).await?;
        for id in [42, 84, 99, 123] {
            assert_eq!(
                shown(moved.tree(), id)?,
                Shown::Bullets,
                "box {id} must stay masked after the page moves it into another same-origin \
                 document"
            );
        }
        assert_eq!(
            shown(moved.tree(), 7)?,
            Shown::Value,
            "box 7 was never marked"
        );
        let other = tree.capture(&scope, &op_page, AttemptStart::now()).await?;
        for id in ids {
            assert_eq!(
                shown(other.tree(), id)?,
                Shown::Value,
                "box {id} on another site is another node, so it must print its value"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn a_mark_mirrored_while_the_apc_request_is_in_flight_is_masked_in_that_capture()
    -> TestResult {
        let apc = page("doc-typing", Some(("https://a.test", false)), &[7, 8]);
        let tab = ScriptedTab::new(TreeScript::new(apc_reply(&apc)));
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let op_page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let tree = Arc::new(TabTree::new(TabOriginState::OpenedUnderInstance));

        // The private entry marks a box on the typing path while the capture's APC request is
        // between the browser and its reply.
        let typing = Arc::clone(&tree);
        let site = SiteKey::from_origin("https://a.test");
        tab.script()
            .set_during_apc(move || typing.mark_private(site.clone(), DomNodeId::new(7)));

        let captured = tree.capture(&scope, &op_page, AttemptStart::now()).await?;
        assert_eq!(tab.script().apc_count(), 1, "one capture, one APC request");
        assert_eq!(
            shown(captured.tree(), 7)?,
            Shown::Bullets,
            "the marks are copied after the reply, so a mark made during the round trip masks"
        );
        assert_eq!(
            shown(captured.tree(), 8)?,
            Shown::Value,
            "box 8 was never marked"
        );
        Ok(())
    }

    #[test]
    fn ref_for_and_debug_redaction_through_tab_tree() -> TestResult {
        let tree = TabTree::new(TabOriginState::ExistedAtStart);
        let site =
            SiteKey::from_origin("https://secret-tenant.example:8443").ok_or("valid site")?;
        tree.mark_private(Some(site), DomNodeId::new(7));
        let preexisting = SnapshotTree::build(
            ApcBuilder::new("doc-preexisting-secret")
                .root(NodeBuilder::root().dom_id(1))
                .build(),
            BuildOptions::new(),
        )?;
        tree.record_private_seed(preexisting.main_document().ok_or("doc")?, []);

        let ref_key = key("doc-debug-secret", 5)?;
        let minted = tree.ref_for(DocSeq::new(0), &ref_key);
        assert_eq!(minted.to_string(), "n1");
        assert_eq!(
            tree.ref_for(DocSeq::new(0), &ref_key),
            minted,
            "repeated ref_for on the same key must reuse its recorded ref"
        );
        assert_eq!(tree.resolve_ref(&parsed_ref("n1")?), live(&ref_key));
        let debug = format!("{tree:?}");
        for forbidden in [
            "secret-tenant",
            "8443",
            "doc-debug-secret",
            "doc-preexisting-secret",
        ] {
            assert!(
                !debug.contains(forbidden),
                "TabTree Debug leaked {forbidden:?}: {debug}"
            );
        }
        assert!(
            debug.contains("TabTree")
                && debug.contains("PageCapture")
                && debug.contains("TabRefTable")
                && debug.contains("PrivateMirror"),
            "TabTree Debug must summarize all subcomponents: {debug}"
        );
        Ok(())
    }
}
