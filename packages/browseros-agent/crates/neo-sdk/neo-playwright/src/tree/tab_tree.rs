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
//! - [`PrivateNodes`] mirrors host-typed secret marks by `(SiteKey, DomNodeId)` so APC masking
//!   redacts host-entered values without extra page round trips.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use browseros_core::FrameId;
use futures_util::future;
use neo_snapshot_apc::refs::{Ref, RefLookup, TabRefTable};
use neo_snapshot_apc::tree::{
    BuildOptions, DocSeq, DocToken, DomNodeId, FrameToken, NodeKey, SnapshotTree, TreeDoc,
};

use crate::PlaywrightError;
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{self, ProbeFailure, ProbeReply};
use crate::target::AriaRefJump;
use crate::tree::{
    AttemptStart, CaptureError, CapturedTree, Hydrate, HydrateOutcome, HydratedNode, PageCapture,
    PrivateNodes, SiteKey, TabOriginState, private_nodes,
};
use crate::world::{DocContext, DocPlace, FrameSeq, FrameWorlds, IsolatedWorld, Pin, Stamp};

/// Per-tab APC capture, ref table, hydration, and private-node state.
///
/// Designed to live on `PageState` once the switch-over wires it; every internal mutex is a short
/// `std::sync::Mutex` that is never held across an `.await`.
pub(crate) struct TabTree {
    capture: PageCapture,
    refs: Arc<Mutex<TabRefTable>>,
    private: Arc<Mutex<PrivateNodes>>,
}

impl TabTree {
    /// Creates a new [`TabTree`] with the baseline BrowserOS APC contract version floor and the
    /// tab's initial [`TabOriginState`].
    #[must_use]
    pub(crate) fn new(origin_state: TabOriginState) -> Self {
        Self {
            capture: PageCapture::default(),
            refs: Arc::new(Mutex::new(TabRefTable::new())),
            private: Arc::new(Mutex::new(PrivateNodes::new(origin_state))),
        }
    }

    /// Creates a new [`TabTree`] enforcing at least `min_contract_version` (clamped up to
    /// `MIN_CONTRACT_VERSION`).
    #[must_use]
    pub(crate) fn with_min_contract_version(
        origin_state: TabOriginState,
        min_contract_version: u32,
    ) -> Self {
        Self {
            capture: PageCapture::new(min_contract_version),
            refs: Arc::new(Mutex::new(TabRefTable::new())),
            private: Arc::new(Mutex::new(PrivateNodes::new(origin_state))),
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
                    };
                    page_state.frames().seq_for_doc(ctx, &proven)
                };

                let options = BuildOptions::new()
                    .with_min_contract_version(min_contract_version)
                    .with_seq_of(&seq_of);
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

    /// Hydrates `key` (expected in `frame`) against `capture` under section 6.2's four cases.
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

    /// Hydrates a snapshot or minted ref jump under section 6.4 (zero captures when already bound).
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

    /// Mirrors a single host-private DOM node mark for `site`.
    pub(crate) fn mark_private(&self, site: SiteKey, node: DomNodeId) {
        self.private
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .mark(site, node);
    }

    /// Resolves probe `pins` in `world` into [`DomNodeId`]s and mirrors all surviving pins as
    /// host-private under `site`, returning how many nodes were mirrored.
    pub(crate) async fn mark_private_pins(
        &self,
        scope: &OpScope<'_>,
        page: &OpPage<'_>,
        world: &IsolatedWorld,
        site: SiteKey,
        pins: Vec<Pin>,
    ) -> Result<usize, ProbeFailure> {
        let dom_ids = private_nodes::describe_pins_node_ids(scope, page, world, pins).await?;
        let count = dom_ids.len();
        if count > 0 {
            self.private
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .mark_many(site, dom_ids);
        }
        Ok(count)
    }

    /// Returns whether `node` must be masked inside `doc` (failing closed on absent metadata).
    #[must_use]
    pub(crate) fn is_private_in_doc(&self, doc: &TreeDoc, node: DomNodeId) -> bool {
        self.private
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_private_in_doc(doc, node)
    }

    /// Returns whether `doc` still needs a one-time page-side seed read for pre-existing tabs.
    #[must_use]
    pub(crate) fn needs_private_seed(&self, doc: &DocToken) -> bool {
        self.private
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .needs_seed(doc)
    }

    /// Imports pre-existing private marks seeded from `doc`'s isolated world and marks `doc`
    /// seeded.
    pub(crate) fn record_private_seed(
        &self,
        doc: DocToken,
        site: SiteKey,
        nodes: impl IntoIterator<Item = DomNodeId>,
    ) {
        self.private
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record_seeded(doc, site, nodes);
    }

    /// Marks `doc` seeded when its isolated world reported zero host-private marks.
    pub(crate) fn mark_private_seeded_empty(&self, doc: DocToken) {
        self.private
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .mark_seeded_empty(doc);
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

    let checks = eligible.into_iter().map(|world| async move {
        let reply = probe::describe(&world).await;
        (world, reply)
    });
    let results = future::join_all(checks).await;

    let mut proven: HashMap<FrameId, IsolatedWorld> = HashMap::new();
    for (world, reply) in results {
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

/// Drops the refs of every document `DocTable` retired since the last drain, so the ref table
/// retires in lockstep with `DocTable`, its authority on document lifetime (design 2.3).
///
/// The queue also carries retirements made outside `TabTree`, such as the resolver invalidating a
/// world that failed `Gone`; draining after every capture catches those before refs are printed.
/// The `FrameWorlds` lock is released before the ref table's is taken, so the two never nest.
fn retire_refs_of_retired_docs(frames: &FrameWorlds, refs: &Mutex<TabRefTable>) {
    let retired = frames.take_retired_docs();
    if retired.is_empty() {
        return;
    }
    let mut table = refs.lock().unwrap_or_else(PoisonError::into_inner);
    for doc in &retired {
        table.retire_doc(doc);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use browseros_core::FrameId;
    use neo_snapshot::{AriaChild, AriaNode};
    use neo_snapshot_apc::refs::{Ref, RefLookup, RefTarget};
    use neo_snapshot_apc::tree::{
        ApcBuilder, AriaAsk, BuildOptions, DocSeq, DocToken, DomNodeId, MIN_CONTRACT_VERSION,
        NodeBuilder, NodeKey, SnapshotTree,
    };
    use serde_json::{Value, json};

    use crate::ErrorCode;
    use crate::op_scope::OpScope;
    use crate::page::OpPage;
    use crate::probe::PROBE_PROTOCOL;
    use crate::target::{AriaRefJump, CallHeader};
    use crate::test_support::{
        ProbeAnswer, SCRIPTED_CHILD, SCRIPTED_MAIN, ScriptedRun, ScriptedTab, TabScript,
        TestResult, count_op,
    };
    use crate::tree::{AttemptStart, HydrateOutcome, SiteKey, TabOriginState, TabTree};
    use crate::world::{DocLookup, FrameSeq, Pin, Place};

    // --- Minimal protobuf wire encoder for hand-crafted offline APC fixtures ---

    fn encode_varint(mut value: u64, out: &mut Vec<u8>) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    fn encode_varint_field(tag: u32, value: u64, out: &mut Vec<u8>) {
        encode_varint(u64::from(tag) << 3, out);
        encode_varint(value, out);
    }

    fn encode_len_delimited(tag: u32, payload: &[u8], out: &mut Vec<u8>) {
        encode_varint((u64::from(tag) << 3) | 2, out);
        encode_varint(payload.len() as u64, out);
        out.extend_from_slice(payload);
    }

    fn encode_frame_data(doc_token: &str, url: Option<&str>, frame_token: Option<&str>) -> Vec<u8> {
        let mut frame = Vec::new();
        let mut doc_id = Vec::new();
        encode_len_delimited(1, doc_token.as_bytes(), &mut doc_id);
        encode_len_delimited(3, &doc_id, &mut frame);
        if let Some(url) = url {
            encode_len_delimited(4, url.as_bytes(), &mut frame);
        }
        if let Some(token) = frame_token {
            let mut browseros = Vec::new();
            encode_len_delimited(1, token.as_bytes(), &mut browseros);
            encode_len_delimited(1000, &browseros, &mut frame);
        }
        frame
    }

    fn encode_leaf_node(dom_id: i32) -> Vec<u8> {
        let mut attrs = Vec::new();
        encode_varint_field(2, u64::from(dom_id.cast_unsigned()), &mut attrs);
        encode_varint_field(3, 24, &mut attrs); // FormControl
        let mut node = Vec::new();
        encode_len_delimited(2, &attrs, &mut node);
        node
    }

    /// A `<button>` named `label` with a box: the ai-mode emitter gives it a ref.
    fn encode_button_node(dom_id: i32, label: &str) -> Vec<u8> {
        let mut rect = Vec::new();
        encode_varint_field(3, 80, &mut rect); // width
        encode_varint_field(4, 20, &mut rect); // height
        let mut geometry = Vec::new();
        encode_len_delimited(1, &rect, &mut geometry); // outer_bounding_box
        let mut browseros = Vec::new();
        encode_len_delimited(1, b"button", &mut browseros); // tag
        encode_len_delimited(11, label.as_bytes(), &mut browseros); // aria_label
        let mut attrs = Vec::new();
        encode_varint_field(2, u64::from(dom_id.cast_unsigned()), &mut attrs);
        encode_varint_field(3, 2, &mut attrs); // Container
        encode_len_delimited(4, &geometry, &mut attrs);
        encode_len_delimited(1000, &browseros, &mut attrs);
        let mut node = Vec::new();
        encode_len_delimited(2, &attrs, &mut node);
        node
    }

    struct ChildFrameSpec<'a> {
        iframe_dom_id: i32,
        doc_token: &'a str,
        frame_token: &'a str,
        url: Option<&'a str>,
        child_dom_ids: &'a [i32],
    }

    fn encode_apc_reply(
        main_doc_token: &str,
        main_url: Option<&str>,
        main_dom_ids: &[i32],
        child: Option<&ChildFrameSpec<'_>>,
        contract_version: Option<u32>,
    ) -> Value {
        let main_nodes: Vec<Vec<u8>> = main_dom_ids
            .iter()
            .map(|&dom_id| encode_leaf_node(dom_id))
            .collect();
        encode_apc_reply_nodes(
            main_doc_token,
            main_url,
            &main_nodes,
            child,
            contract_version,
        )
    }

    fn encode_apc_reply_nodes(
        main_doc_token: &str,
        main_url: Option<&str>,
        main_nodes: &[Vec<u8>],
        child: Option<&ChildFrameSpec<'_>>,
        contract_version: Option<u32>,
    ) -> Value {
        let mut root_node = Vec::new();
        for node in main_nodes {
            encode_len_delimited(1, node, &mut root_node);
        }
        if let Some(child) = child {
            let mut iframe_node = Vec::new();
            for &dom_id in child.child_dom_ids {
                let leaf = encode_leaf_node(dom_id);
                encode_len_delimited(1, &leaf, &mut iframe_node);
            }
            let child_frame_data =
                encode_frame_data(child.doc_token, child.url, Some(child.frame_token));
            let mut iframe_data = Vec::new();
            encode_len_delimited(3, &child_frame_data, &mut iframe_data);

            let mut iframe_attrs = Vec::new();
            encode_varint_field(
                2,
                u64::from(child.iframe_dom_id.cast_unsigned()),
                &mut iframe_attrs,
            );
            encode_varint_field(3, 3, &mut iframe_attrs); // Iframe
            encode_len_delimited(9, &iframe_data, &mut iframe_attrs);
            encode_len_delimited(2, &iframe_attrs, &mut iframe_node);
            encode_len_delimited(1, &iframe_node, &mut root_node);
        }

        let mut root_attrs = Vec::new();
        encode_varint_field(2, 1, &mut root_attrs);
        encode_varint_field(3, 1, &mut root_attrs); // Root
        encode_len_delimited(2, &root_attrs, &mut root_node);

        let main_frame_data = encode_frame_data(main_doc_token, main_url, None);

        let mut apc = Vec::new();
        encode_varint_field(1, 1, &mut apc); // Version10
        encode_len_delimited(2, &root_node, &mut apc);
        encode_len_delimited(3, &main_frame_data, &mut apc);
        if let Some(version) = contract_version {
            let mut browseros = Vec::new();
            encode_varint_field(1, u64::from(version), &mut browseros);
            encode_len_delimited(1000, &browseros, &mut apc);
        }
        json!({ "content": STANDARD.encode(apc) })
    }

    fn key(doc: &str, dom_id: i32) -> TestResult<NodeKey> {
        let token = DocToken::new(doc).ok_or("non-empty doc token")?;
        Ok(NodeKey::new(token, DomNodeId::new(dom_id)))
    }

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

    /// Programmable fake CDP script backing `ScriptedTab` for offline `TabTree` + `Hydrate` tests.
    ///
    /// Tracks distinct per-install execution context IDs and global object IDs, per-global stale
    /// status (so `W_old` can go `Gone` after navigation while `W_new` succeeds), queued APC
    /// replies, `DOM.resolveNode` behaviors, and probe replies.
    struct TreeScript {
        has_child: bool,
        apc_replies: Mutex<VecDeque<Value>>,
        default_apc: Mutex<Option<Value>>,
        /// Hook run right after an isolated world finishes booting, letting tests simulate a
        /// navigation occurring between Case 3's world install and its subsequent recapture.
        after_world_install: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
        next_context: AtomicI64,
        context_to_global: Mutex<HashMap<i64, String>>,
        global_info: Mutex<HashMap<String, WorldStub>>,
        stale_globals: Mutex<HashMap<String, &'static str>>,
        resolve_node_error: Mutex<Option<&'static str>>,
        resolve_probe_error: Mutex<Option<&'static str>>,
        resolve_probe_reply: Mutex<Option<Value>>,
        pinned_nodes_reply: Mutex<Option<Value>>,
        next_pin: AtomicU32,
        rekey_first_main: AtomicBool,
        rekey_retired_seq: u32,
        resolve_node_calls: Mutex<Vec<Value>>,
        apc_calls: AtomicUsize,
    }

    #[derive(Clone)]
    struct WorldStub {
        frame: String,
        seq: u32,
        url: String,
    }

    impl TreeScript {
        fn new(default_apc: Value) -> Self {
            Self {
                has_child: false,
                apc_replies: Mutex::default(),
                default_apc: Mutex::new(Some(default_apc)),
                after_world_install: Mutex::default(),
                next_context: AtomicI64::new(100),
                context_to_global: Mutex::default(),
                global_info: Mutex::default(),
                stale_globals: Mutex::default(),
                resolve_node_error: Mutex::default(),
                resolve_probe_error: Mutex::default(),
                resolve_probe_reply: Mutex::default(),
                pinned_nodes_reply: Mutex::default(),
                next_pin: AtomicU32::new(1),
                rekey_first_main: AtomicBool::new(false),
                rekey_retired_seq: 0,
                resolve_node_calls: Mutex::default(),
                apc_calls: AtomicUsize::new(0),
            }
        }

        fn with_child(mut self) -> Self {
            self.has_child = true;
            self
        }

        fn with_rekey_retired_seq(mut self, retired: u32) -> Self {
            self.rekey_first_main = AtomicBool::new(true);
            self.rekey_retired_seq = retired;
            self
        }

        fn push_apc(&self, reply: Value) {
            self.apc_replies
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push_back(reply);
        }

        fn set_default_apc(&self, reply: Value) {
            *self
                .default_apc
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(reply);
        }

        fn mark_global_stale(&self, global: &str, message: &'static str) {
            self.stale_globals
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(global.to_owned(), message);
        }

        fn set_resolve_node_error(&self, message: Option<&'static str>) {
            *self
                .resolve_node_error
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = message;
        }

        fn set_resolve_probe_error(&self, message: Option<&'static str>) {
            *self
                .resolve_probe_error
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = message;
        }

        fn set_resolve_probe_reply(&self, reply: Option<Value>) {
            *self
                .resolve_probe_reply
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = reply;
        }

        fn set_pinned_nodes_reply(&self, reply: Value) {
            *self
                .pinned_nodes_reply
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(reply);
        }

        fn set_after_world_install(&self, hook: impl Fn() + Send + Sync + 'static) {
            *self
                .after_world_install
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(Box::new(hook));
        }

        fn apc_count(&self) -> usize {
            self.apc_calls.load(Ordering::SeqCst)
        }

        fn resolve_node_calls(&self) -> Vec<Value> {
            self.resolve_node_calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }

        fn call_function_on_reply(&self, params: &Value) -> Option<Value> {
            let object_id = params["objectId"].as_str().unwrap_or_default();
            let first = &params["arguments"][0]["value"];

            // Batch pinned_nodes call (array of envelopes) used by mark_private_pins.
            if first.is_array() {
                return self
                    .pinned_nodes_reply
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
            }

            // Boot install call carrying `{ frameSeq, epoch }`.
            if let Some(seq) = first["frameSeq"].as_u64() {
                return Some(self.boot_reply(object_id, u32::try_from(seq).unwrap_or(0)));
            }

            // Normal probe request envelope `{ request, policy }`.
            let request = first.get("request")?;
            let op = request["op"].as_str()?;
            let reply = match op {
                "describe" => {
                    let stub = self
                        .global_info
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(object_id)
                        .cloned();
                    let (seq, url) = stub.map_or_else(
                        || (0, "https://a.test/".to_owned()),
                        |stub| (stub.seq, stub.url),
                    );
                    json!({
                        "kind": "described",
                        "protocol": PROBE_PROTOCOL,
                        "seq": seq,
                        "url": url,
                    })
                }
                "resolve" if request["want"] == "frame" => {
                    let pin = self.next_pin.fetch_add(1, Ordering::SeqCst);
                    json!({
                        "kind": "frame",
                        "pin": pin,
                        "key": format!("#{pin}"),
                        "visible": true,
                        "src": "https://a.test/child",
                    })
                }
                "resolve" => self
                    .resolve_probe_reply
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone()
                    .unwrap_or_else(|| {
                        let pin = self.next_pin.fetch_add(1, Ordering::SeqCst);
                        let root_obj = params["arguments"][1]["objectId"]
                            .as_str()
                            .unwrap_or("node");
                        json!({
                            "kind": "element",
                            "pin": pin,
                            "preview": format!("<button data-obj=\"{root_obj}\">Go</button>"),
                        })
                    }),
                _ => return None,
            };
            Some(json!({
                "result": { "type": "string", "value": reply.to_string() }
            }))
        }

        fn boot_reply(&self, object_id: &str, seq: u32) -> Value {
            let mut retired = None;
            let url = {
                let mut info = self
                    .global_info
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if let Some(stub) = info.get_mut(object_id) {
                    stub.seq = seq;
                    if stub.frame == SCRIPTED_MAIN
                        && self.rekey_first_main.swap(false, Ordering::SeqCst)
                    {
                        retired = Some(self.rekey_retired_seq);
                    }
                    stub.url.clone()
                } else {
                    "https://a.test/".to_owned()
                }
            };
            let hook = self
                .after_world_install
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(hook) = hook {
                hook();
            }
            let mut reply = json!({
                "kind": "described",
                "protocol": PROBE_PROTOCOL,
                "seq": seq,
                "url": url,
            });
            if let Some(retired_seq) = retired {
                reply["retired"] = json!(retired_seq);
            }
            json!({ "result": { "type": "string", "value": reply.to_string() } })
        }
    }

    impl TabScript for TreeScript {
        fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
            // `Runtime.callFunctionOn` is handled directly in `cdp_reply` / `cdp_error` below so
            // we can inspect `params["objectId"]` per world install.
            ProbeAnswer::unexpected()
        }

        fn child(&self) -> bool {
            self.has_child
        }

        fn cdp_error(&self, method: &str, params: &Value) -> Option<&'static str> {
            match method {
                "DOM.resolveNode" => {
                    if let Some(error) = *self
                        .resolve_node_error
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                    {
                        return Some(error);
                    }
                    if let Some(ctx) = params["executionContextId"].as_i64() {
                        let global = self
                            .context_to_global
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .get(&ctx)
                            .cloned();
                        if let Some(global) = global
                            && let Some(msg) = self
                                .stale_globals
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .get(&global)
                                .copied()
                        {
                            return Some(msg);
                        }
                    }
                    None
                }
                "Runtime.callFunctionOn" => {
                    if let Some(object_id) = params["objectId"].as_str()
                        && let Some(msg) = self
                            .stale_globals
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .get(object_id)
                            .copied()
                    {
                        return Some(msg);
                    }
                    let op = params["arguments"][0]["value"]["request"]["op"].as_str();
                    if op == Some("resolve") {
                        return *self
                            .resolve_probe_error
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner);
                    }
                    None
                }
                _ => None,
            }
        }

        fn cdp_reply(&self, method: &str, params: &Value) -> Option<Value> {
            match method {
                // Report no pre-existing targets at Playwright start so the scripted tab models a
                // newly opened tab starting at FrameSeq(0) (bare `e1`, then `f1e1`).
                "Target.getTargets" => Some(json!({ "targetInfos": [] })),
                "Page.getAnnotatedPageContent" => {
                    self.apc_calls.fetch_add(1, Ordering::SeqCst);
                    let queued = self
                        .apc_replies
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .pop_front();
                    queued.or_else(|| {
                        self.default_apc
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .clone()
                    })
                }
                "Page.createIsolatedWorld" => {
                    let frame = params["frameId"]
                        .as_str()
                        .unwrap_or(SCRIPTED_MAIN)
                        .to_owned();
                    let ctx = self.next_context.fetch_add(1, Ordering::SeqCst);
                    let global = format!("g-{frame}-{ctx}");
                    self.context_to_global
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(ctx, global.clone());
                    let url = if frame == SCRIPTED_CHILD {
                        "https://a.test/child".to_owned()
                    } else {
                        "https://a.test/".to_owned()
                    };
                    self.global_info
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(global, WorldStub { frame, seq: 0, url });
                    Some(json!({ "executionContextId": ctx }))
                }
                "Runtime.evaluate" => {
                    let ctx = params["contextId"].as_i64()?;
                    let global = self
                        .context_to_global
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(&ctx)
                        .cloned()?;
                    Some(json!({ "result": { "type": "object", "objectId": global } }))
                }
                "DOM.getFrameOwner" => {
                    if params["frameId"].as_str() == Some(SCRIPTED_CHILD) {
                        Some(json!({ "backendNodeId": 500 }))
                    } else {
                        None
                    }
                }
                "DOM.resolveNode" => {
                    let backend_id = params["backendNodeId"].as_i64().unwrap_or_default();
                    let ctx = params["executionContextId"].as_i64().unwrap_or_default();
                    self.resolve_node_calls
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(params.clone());
                    Some(json!({
                        "object": {
                            "type": "object",
                            "subtype": "node",
                            "objectId": format!("node-{ctx}-{backend_id}"),
                        }
                    }))
                }
                "DOM.describeNode" => {
                    let object_id = params["objectId"].as_str().unwrap_or_default();
                    if object_id.ends_with("-500") {
                        return Some(json!({
                            "node": {
                                "backendNodeId": 500,
                                "nodeName": "IFRAME",
                                "frameId": SCRIPTED_CHILD,
                                "contentDocument": { "nodeId": 2, "backendNodeId": 2, "nodeName": "#document" },
                            }
                        }));
                    }
                    None
                }
                "Runtime.callFunctionOn" => self.call_function_on_reply(params),
                _ => None,
            }
        }
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

        // Unknown never-issued seq and unknown ref number on live seq fail InvalidInput with 0
        // captures.
        for unknown in ["f99e1", "e99"] {
            let err = tree
                .hydrate_ref(&scope, &page, jump(unknown)?)
                .await
                .err()
                .ok_or("unknown ref must fail")?;
            assert_eq!(
                err.error_code(),
                Some(ErrorCode::InvalidInput),
                "unknown ref {unknown} must fail InvalidInput: {err}"
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
    async fn private_nodes_mirror_and_seed_through_tab_tree() -> TestResult {
        let apc = encode_apc_reply(
            "doc-private",
            Some("https://a.test/"),
            &[42],
            None,
            Some(MIN_CONTRACT_VERSION),
        );
        let tab = ScriptedTab::new(TreeScript::new(apc));
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let tree = TabTree::new(TabOriginState::ExistedAtStart);

        tab.script().set_pinned_nodes_reply(json!({
            "result": {
                "type": "object",
                "deepSerializedValue": {
                    "type": "array",
                    "value": [
                        { "type": "node", "value": { "backendNodeId": 42 } },
                        { "type": "node", "value": { "backendNodeId": 84 } }
                    ]
                }
            }
        }));
        let main_world = page.world(&page.main_endpoint(), Place::Main).await?;
        let site_a = SiteKey::from_origin("https://A.TEST:443/login").ok_or("site a")?;
        let mirrored = tree
            .mark_private_pins(
                &scope,
                &page,
                &main_world,
                site_a.clone(),
                vec![Pin(1), Pin(2)],
            )
            .await?;
        assert_eq!(
            mirrored, 2,
            "mark_private_pins must mirror every surviving pinned DomNodeId"
        );
        tree.mark_private(site_a.clone(), DomNodeId::new(99));

        let same_origin_moved = SnapshotTree::build(
            ApcBuilder::new("doc-moved")
                .main_url("https://a.test/iframe")
                .root(NodeBuilder::root().dom_id(1))
                .build(),
            BuildOptions::new(),
        )?;
        let other_site = SnapshotTree::build(
            ApcBuilder::new("doc-other")
                .main_url("https://b.test/form")
                .main_origin("https://b.test", false)
                .root(NodeBuilder::root().dom_id(1))
                .build(),
            BuildOptions::new(),
        )?;
        let moved_doc = same_origin_moved.main_document().ok_or("moved doc")?;
        let other_doc = other_site.main_document().ok_or("other doc")?;

        for id in [42, 84, 99] {
            assert!(
                tree.is_private_in_doc(moved_doc, DomNodeId::new(id)),
                "host-private node {id} must stay masked after a same-origin document move"
            );
            assert!(
                !tree.is_private_in_doc(other_doc, DomNodeId::new(id)),
                "host-private node {id} must not bleed onto a cross-site document"
            );
        }

        // ExistedAtStart one-time seeding through TabTree.
        let doc_a = DocToken::new("doc-preexisting-a").ok_or("doc a")?;
        let doc_b = DocToken::new("doc-preexisting-b").ok_or("doc b")?;
        assert!(tree.needs_private_seed(&doc_a));
        assert!(tree.needs_private_seed(&doc_b));

        tree.record_private_seed(doc_a.clone(), site_a, [DomNodeId::new(123)]);
        assert!(!tree.needs_private_seed(&doc_a));
        assert!(tree.is_private_in_doc(moved_doc, DomNodeId::new(123)));
        assert!(tree.needs_private_seed(&doc_b));

        tree.mark_private_seeded_empty(doc_b.clone());
        assert!(!tree.needs_private_seed(&doc_b));
        Ok(())
    }

    #[test]
    fn ref_for_and_debug_redaction_through_tab_tree() -> TestResult {
        let tree = TabTree::new(TabOriginState::ExistedAtStart);
        let site = SiteKey::from_origin("https://secret-tenant.example:8443/account")
            .ok_or("valid site")?;
        tree.mark_private(site, DomNodeId::new(7));
        tree.mark_private_seeded_empty(DocToken::new("doc-preexisting-secret").ok_or("doc")?);

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
                && debug.contains("PrivateNodes"),
            "TabTree Debug must summarize all subcomponents: {debug}"
        );
        Ok(())
    }
}
