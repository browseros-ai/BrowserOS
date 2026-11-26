//! Per-tab mirror of the private entry's marks: privacy layer 2's copy in Rust (design 8.1).
//!
//! Layer 1 privacy redacts password-like and sensitive `autocomplete` controls inside Blink's APC
//! serializer. Layer 2 covers ordinary boxes the host typed a secret into through the private
//! entry. The page holds the record: the entry sets its mark on the element in the frame's
//! isolated world before the first character is typed, so the mark lives exactly as long as the
//! element and survives an SDK restart, as worlds do. [`PrivateMirror`] copies those marks into
//! the [`PrivateNodes`] every capture builds with, so masking costs no page round trip:
//!
//! Nothing calls into it yet; the switch-over makes the calls on the left:
//!
//! ```text
//! private entry ─▶ TabTree::mark_private_pins ───────────┐
//! first snapshot's seed ─▶ TabTree::record_private_seed ─┴─▶ PrivateMirror::marks
//!                                                                │ copied after each APC reply
//!                                                                ▼
//!                              TabTree::capture ─▶ SnapshotTree::build masks
//! ```

use std::collections::HashSet;
use std::fmt;

use browseros_cdp::CdpError;
use browseros_core::{CoreError, ProtocolSession};
use neo_snapshot_apc::tree::{DocToken, DomNodeId, PrivateNodes, SiteKey, TreeDoc};
use serde::Deserialize;
use serde_json::json;

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{self, ProbeFailure};
use crate::world::{BackendNodeId, IsolatedWorld, Pin};

/// CDP protocol error returned by `DOM.describeNode` when a remote node handle is no longer backed
/// by a DOM node (for example, after collection or replacement).
const NO_NODE_WITH_ID: &str = "No node with given id found";

/// Whether a tab existed before this `Playwright` instance started, controlling whether documents
/// must be seeded from their isolated world before the host mirror can be trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabOriginState {
    /// Tab created after this `Playwright` started; every private mark in its documents was made
    /// by this instance, so no initial page-side seed read is needed.
    OpenedUnderInstance,
    /// Tab was already open when this `Playwright` started (or open-tab listing failed); each
    /// pre-existing document must be seeded once from its isolated world before a snapshot trusts
    /// the mirror for it.
    ExistedAtStart,
}

/// One tab's copy of the private entry's page-side marks, held as the builder's own
/// [`PrivateNodes`] so the feed into each capture is the value itself, with nothing to translate.
///
/// For a tab opened under this instance every mark was made by it, so the copy is complete. For a
/// tab that already existed, a document's earlier marks reach the copy only once its world's
/// `privateMarks` seed is recorded, and [`Self::needs_seed`] says which documents still lack one.
pub(crate) struct PrivateMirror {
    origin_state: TabOriginState,
    marks: PrivateNodes,
    seeded_docs: HashSet<DocToken>,
}

impl PrivateMirror {
    #[must_use]
    pub(crate) fn new(origin_state: TabOriginState) -> Self {
        Self {
            origin_state,
            marks: PrivateNodes::new(),
            seeded_docs: HashSet::new(),
        }
    }

    /// Returns whether this tab was opened under the current instance or already existed at start.
    #[must_use]
    #[cfg(test)]
    pub(crate) const fn origin_state(&self) -> TabOriginState {
        self.origin_state
    }

    /// Returns whether `doc` must be seeded once from its isolated world before a snapshot may
    /// rely on this mirror alone.
    #[must_use]
    pub(crate) fn needs_seed(&self, doc: &DocToken) -> bool {
        self.origin_state == TabOriginState::ExistedAtStart && !self.seeded_docs.contains(doc)
    }

    /// Records `node` as host-private on `site`. `None`: the site the mark was made on is unknown
    /// (a world's `self.origin` reads `"null"` for an opaque origin), so `node` masks in every
    /// document.
    pub(crate) fn mark(&mut self, site: Option<SiteKey>, node: DomNodeId) {
        self.marks.mark(site, node);
    }

    /// The marks every capture builds with, through `BuildOptions::with_private_nodes`.
    #[must_use]
    pub(crate) const fn marks(&self) -> &PrivateNodes {
        &self.marks
    }

    /// Imports the marks `doc`'s isolated world reported, on `doc`'s own site
    /// ([`SiteKey::of_doc`]; an origin-less document's marks mask everywhere), and records `doc`
    /// seeded so later snapshots skip the seed read. Zero `nodes` records an empty seed.
    pub(crate) fn record_seeded(
        &mut self,
        doc: &TreeDoc,
        nodes: impl IntoIterator<Item = DomNodeId>,
    ) {
        let site = SiteKey::of_doc(doc);
        for node in nodes {
            self.marks.mark(site.clone(), node);
        }
        self.seeded_docs.insert(doc.token().clone());
    }
}

impl fmt::Debug for PrivateMirror {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrivateMirror")
            .field("origin_state", &self.origin_state)
            .field("marks", &self.marks)
            .field("seeded_docs", &self.seeded_docs.len())
            .finish()
    }
}

/// `DOM.describeNode` reply subset carrying only the node's backend ID.
#[derive(Deserialize)]
struct Described {
    node: DescribedNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescribedNode {
    backend_node_id: BackendNodeId,
}

/// Converts CDP's 64-bit `backendNodeId` to APC's 32-bit `DomNodeId`, failing loudly on overflow
/// instead of silently truncating to an unrelated node ID.
pub(crate) fn backend_to_dom_node_id(id: BackendNodeId) -> Result<DomNodeId, ProbeFailure> {
    let raw = i32::try_from(id.0).map_err(|error| {
        ProbeFailure::Broken(format!(
            "DOM backendNodeId {} exceeds i32 range: {error}",
            id.0
        ))
    })?;
    Ok(DomNodeId::new(raw))
}

/// Describes a remote DOM object handle to obtain its `DomNodeId` for mirroring in
/// [`PrivateMirror`].
///
/// Maps CDP's `"No node with given id found"` protocol error to `Ok(None)` so a vanished node is
/// skipped cleanly, maps stale contexts or lost sessions to [`ProbeFailure::Gone`], and stamps
/// target-crash tracking through [`OpScope::begin_protocol`].
pub(crate) async fn describe_object_node_id(
    scope: &OpScope<'_>,
    session: &ProtocolSession,
    object_id: &str,
) -> Result<Option<DomNodeId>, ProbeFailure> {
    let crash = scope.begin_protocol(session);
    let described = session
        .send::<_, Described>(
            "DOM.describeNode",
            json!({ "objectId": object_id, "depth": 0 }),
        )
        .await;
    match described {
        Ok(described) => backend_to_dom_node_id(described.node.backend_node_id).map(Some),
        Err(CoreError::Cdp(CdpError::Protocol { message, .. }))
            if message.contains(NO_NODE_WITH_ID) =>
        {
            Ok(None)
        }
        Err(error) => {
            let failure = ProbeFailure::from_core(error);
            crash.probe_failure(&failure);
            Err(failure)
        }
    }
}

/// Resolves a batch of probe [`Pin`]s in `world` into their [`DomNodeId`]s for mirroring in
/// [`PrivateMirror`].
///
/// Uses the single-round-trip `probe::pinned_nodes` deep-serialization batch when every pin is
/// still live. Because `probe::pinned_nodes` returns `Ok(None)` whenever any pin in the batch was
/// evicted, multi-pin batches fall back to per-pin `probe::pinned_object` +
/// [`describe_object_node_id`] so a single evicted sibling never drops a live private mark.
pub(crate) async fn describe_pins_node_ids(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    pins: Vec<Pin>,
) -> Result<Vec<DomNodeId>, ProbeFailure> {
    if pins.is_empty() {
        return Ok(Vec::new());
    }
    let fallback_pins = (pins.len() > 1).then(|| pins.clone());
    if let Some(backend_ids) = probe::pinned_nodes(scope, page, world, pins).await? {
        let mut dom_ids = Vec::with_capacity(backend_ids.len());
        for backend_id in backend_ids {
            dom_ids.push(backend_to_dom_node_id(backend_id)?);
        }
        return Ok(dom_ids);
    }
    // Single-pin eviction needs no fallback: the sole pin is already known to be gone.
    let Some(fallback_pins) = fallback_pins else {
        return Ok(Vec::new());
    };
    let mut dom_ids = Vec::with_capacity(fallback_pins.len());
    for pin in fallback_pins {
        if let Some(object_id) = probe::pinned_object(scope, page, world, pin).await?
            && let Some(dom_id) =
                describe_object_node_id(scope, world.session(), &object_id).await?
        {
            dom_ids.push(dom_id);
        }
    }
    Ok(dom_ids)
}

/// The mirror's tests, and the test pages `TabTree`'s capture tests serve too, so both read what
/// a box printed the same way.
#[cfg(test)]
pub(super) mod tests {
    use std::sync::{Arc, Mutex, PoisonError};

    use browseros_cdp::{CdpError, CdpEvent, TargetedEvents};
    use browseros_core::{CdpConnection, ProtocolSession, SessionId};
    use futures_util::future::BoxFuture;
    use neo_snapshot::{AriaChild, AriaNode};
    use neo_snapshot_apc::proto::{AnnotatedPageContent, FormControlType};
    use neo_snapshot_apc::tree::{
        ApcBuilder, AriaAsk, BuildOptions, DocToken, DomNodeId, NodeBuilder, SiteKey, SnapshotTree,
    };
    use serde_json::{Value, json};
    use tokio::sync::{broadcast, mpsc};

    use super::{PrivateMirror, TabOriginState, describe_object_node_id, describe_pins_node_ids};
    use crate::op_scope::OpScope;
    use crate::page::OpPage;
    use crate::probe::{ProbeFailure, Ran};
    use crate::target::CallHeader;
    use crate::test_env::TestEnv;
    use crate::test_support::{
        ProbeAnswer, SCRIPTED_MAIN, ScriptedRun, ScriptedTab, TabScript, TestResult, count_op,
    };
    use crate::world::{IsolatedWorld, Pin};

    /// Prefix of every test box's value, so a value that prints is easy to spot.
    const CANARY: &str = "NEO-PRIVATE-CANARY";

    /// What a box printed in a built tree.
    #[derive(Debug, PartialEq, Eq)]
    pub(in crate::tree) enum Shown {
        /// One bullet per UTF-16 unit of the value: the box was masked.
        Bullets,
        /// The value itself.
        Value,
    }

    /// A page whose main document is `token` on `origin`, with one text box per id, named
    /// `box {id}` and holding the canary value `{CANARY}-{id}`.
    pub(in crate::tree) fn page(
        token: &str,
        origin: Option<(&str, bool)>,
        ids: &[i32],
    ) -> AnnotatedPageContent {
        let boxes = ids.iter().map(|&id| {
            NodeBuilder::form_control(FormControlType::InputText)
                .dom_id(id)
                .tag("input")
                .aria_label(&format!("box {id}"))
                .field_value(&canary(id))
                .bounding_box(0, id * 30, 180, 24)
        });
        let mut builder =
            ApcBuilder::new(token).root(NodeBuilder::root().dom_id(1).children(boxes.collect()));
        if let Some((value, opaque)) = origin {
            builder = builder.main_origin(value, opaque);
        }
        builder.build()
    }

    /// Builds `apc` with the mirror's marks, as `TabTree::capture` does.
    fn build(apc: AnnotatedPageContent, mirror: &PrivateMirror) -> TestResult<SnapshotTree> {
        Ok(SnapshotTree::build(
            apc,
            BuildOptions::new().with_private_nodes(mirror.marks()),
        )?)
    }

    /// Builds `apc` unmasked, for the document a seed is recorded against: the mirror only ever
    /// seeds a document APC reported.
    fn seed_doc(apc: AnnotatedPageContent) -> TestResult<SnapshotTree> {
        Ok(SnapshotTree::build(apc, BuildOptions::new())?)
    }

    fn canary(id: i32) -> String {
        format!("{CANARY}-{id}")
    }

    /// What `box {id}` printed in `tree`'s default ARIA output.
    pub(in crate::tree) fn shown(tree: &SnapshotTree, id: i32) -> TestResult<Shown> {
        let aria = tree.aria(AriaAsk::default_mode())?;
        let name = format!("box {id}");
        let mut stack: Vec<&AriaNode> = aria.nodes().iter().collect();
        while let Some(node) = stack.pop() {
            if node.role == "textbox" && node.name.as_deref() == Some(name.as_str()) {
                let value = canary(id);
                let bullets = "•".repeat(value.encode_utf16().count());
                return match node.text.as_deref() {
                    Some(text) if text == value => Ok(Shown::Value),
                    Some(text) if text == bullets => Ok(Shown::Bullets),
                    other => Err(format!("{name} printed {other:?}").into()),
                };
            }
            stack.extend(node.children.iter().filter_map(|child| match child {
                AriaChild::Node(node) => Some(node.as_ref()),
                AriaChild::Text(_) => None,
            }));
        }
        Err(format!("{name} did not print").into())
    }

    fn site(origin: &str) -> TestResult<Option<SiteKey>> {
        let key = SiteKey::from_origin(origin).ok_or_else(|| format!("{origin:?} is an origin"))?;
        Ok(Some(key))
    }

    #[test]
    fn a_tuple_mark_in_any_spelling_masks_on_its_site_only() -> TestResult {
        let mut mirror = PrivateMirror::new(TabOriginState::OpenedUnderInstance);
        mirror.mark(site("https://A.TEST:443")?, DomNodeId::new(7));

        let home = build(
            page("doc-a", Some(("https://a.test", false)), &[7, 8]),
            &mirror,
        )?;
        assert_eq!(
            shown(&home, 7)?,
            Shown::Bullets,
            "APC's https://a.test is the site the mark spelled https://A.TEST:443"
        );
        assert_eq!(shown(&home, 8)?, Shown::Value, "an unmarked box prints");
        let moved = build(
            page("doc-moved", Some(("https://a.test", false)), &[7]),
            &mirror,
        )?;
        assert_eq!(
            shown(&moved, 7)?,
            Shown::Bullets,
            "a box moved into another same-origin document keeps its id and its mask"
        );
        let other = build(
            page("doc-b", Some(("https://b.test", false)), &[7]),
            &mirror,
        )?;
        assert_eq!(
            shown(&other, 7)?,
            Shown::Value,
            "the same id on another site is another node"
        );
        Ok(())
    }

    #[test]
    fn non_tuple_marks_mask_on_their_own_scheme_and_host() -> TestResult {
        let mut mirror = PrivateMirror::new(TabOriginState::OpenedUnderInstance);
        mirror.mark(site("file://")?, DomNodeId::new(7));
        mirror.mark(site("chrome-extension://abc")?, DomNodeId::new(8));

        let file = build(page("doc-file", Some(("file://", false)), &[7, 8]), &mirror)?;
        assert_eq!(
            shown(&file, 7)?,
            Shown::Bullets,
            "a file: mark masks in a file: document"
        );
        assert_eq!(
            shown(&file, 8)?,
            Shown::Value,
            "an extension's mark is not file:'s"
        );
        let extension = build(
            page("doc-ext", Some(("chrome-extension://abc", false)), &[7, 8]),
            &mirror,
        )?;
        assert_eq!(
            shown(&extension, 7)?,
            Shown::Value,
            "file:'s mark is not the extension's"
        );
        assert_eq!(
            shown(&extension, 8)?,
            Shown::Bullets,
            "an extension's mark masks in its own pages"
        );
        let other = build(
            page("doc-ext-2", Some(("chrome-extension://xyz", false)), &[8]),
            &mirror,
        )?;
        assert_eq!(
            shown(&other, 8)?,
            Shown::Value,
            "another extension is another site"
        );
        Ok(())
    }

    #[test]
    fn an_opaque_seed_masks_under_its_own_nonce_only() -> TestResult {
        let mut mirror = PrivateMirror::new(TabOriginState::ExistedAtStart);
        let sandboxed = seed_doc(page("doc-sandboxed", Some(("nonce-1", true)), &[7]))?;
        let doc = sandboxed.main_document().ok_or("main doc")?;
        mirror.record_seeded(doc, [DomNodeId::new(7)]);
        assert!(!mirror.needs_seed(doc.token()), "the seed is recorded");

        let same = build(
            page("doc-sandboxed", Some(("nonce-1", true)), &[7]),
            &mirror,
        )?;
        assert_eq!(
            shown(&same, 7)?,
            Shown::Bullets,
            "the seeded nonce masks its box"
        );
        let other_nonce = build(
            page("doc-sandboxed-2", Some(("nonce-2", true)), &[7]),
            &mirror,
        )?;
        assert_eq!(
            shown(&other_nonce, 7)?,
            Shown::Value,
            "another nonce is another site"
        );
        let tuple = build(
            page("doc-a", Some(("https://a.test", false)), &[7]),
            &mirror,
        )?;
        assert_eq!(
            shown(&tuple, 7)?,
            Shown::Value,
            "an opaque mark never matches a tuple site"
        );
        Ok(())
    }

    #[test]
    fn an_unknown_site_mark_masks_in_every_document() -> TestResult {
        let mut mirror = PrivateMirror::new(TabOriginState::OpenedUnderInstance);
        mirror.mark(None, DomNodeId::new(7));

        for apc in [
            page("doc-a", Some(("https://a.test", false)), &[7, 8]),
            page("doc-opaque", Some(("nonce-1", true)), &[7, 8]),
            page("doc-bare", None, &[7, 8]),
        ] {
            let tree = build(apc, &mirror)?;
            assert_eq!(
                shown(&tree, 7)?,
                Shown::Bullets,
                "a mark whose site was unreadable masks its id wherever it is read"
            );
            assert_eq!(shown(&tree, 8)?, Shown::Value, "only the marked id masks");
        }
        Ok(())
    }

    #[test]
    fn seeds_mark_on_the_seeded_documents_site_and_fail_closed_without_one() -> TestResult {
        let mut mirror = PrivateMirror::new(TabOriginState::ExistedAtStart);
        let tuple_seed = seed_doc(page("doc-seed-a", Some(("https://a.test", false)), &[7]))?;
        let bare_seed = seed_doc(page("doc-seed-bare", None, &[8]))?;
        let tuple_doc = tuple_seed.main_document().ok_or("main doc")?;
        let bare_doc = bare_seed.main_document().ok_or("main doc")?;
        assert!(mirror.needs_seed(tuple_doc.token()) && mirror.needs_seed(bare_doc.token()));

        mirror.record_seeded(tuple_doc, [DomNodeId::new(7)]);
        mirror.record_seeded(bare_doc, [DomNodeId::new(8)]);
        assert!(
            !mirror.needs_seed(tuple_doc.token()) && !mirror.needs_seed(bare_doc.token()),
            "a recorded seed is read once per document"
        );

        let on_a = build(
            page("doc-a", Some(("https://a.test", false)), &[7, 8]),
            &mirror,
        )?;
        assert_eq!(
            shown(&on_a, 7)?,
            Shown::Bullets,
            "a tuple seed marks on its document's site"
        );
        assert_eq!(
            shown(&on_a, 8)?,
            Shown::Bullets,
            "an origin-less document's seed marks on an unknown site, which masks everywhere"
        );
        let on_b = build(
            page("doc-b", Some(("https://b.test", false)), &[7, 8]),
            &mirror,
        )?;
        assert_eq!(
            shown(&on_b, 7)?,
            Shown::Value,
            "a tuple seed stays on its own site"
        );
        assert_eq!(
            shown(&on_b, 8)?,
            Shown::Bullets,
            "an unknown-site seed masks on any site"
        );
        Ok(())
    }

    #[test]
    fn a_zero_node_seed_marks_the_document_seeded_and_masks_nothing() -> TestResult {
        let new_tab = PrivateMirror::new(TabOriginState::OpenedUnderInstance);
        let token = DocToken::new("doc-1").ok_or("doc token")?;
        assert!(
            !new_tab.needs_seed(&token),
            "every mark of a tab opened under this instance was made by it"
        );

        let mut old_tab = PrivateMirror::new(TabOriginState::ExistedAtStart);
        assert_eq!(old_tab.origin_state(), TabOriginState::ExistedAtStart);
        let first = seed_doc(page("doc-1", Some(("https://a.test", false)), &[7]))?;
        let second = seed_doc(page("doc-2", Some(("https://a.test", false)), &[7]))?;
        let first_doc = first.main_document().ok_or("main doc")?;
        let second_doc = second.main_document().ok_or("main doc")?;
        old_tab.record_seeded(first_doc, []);
        assert!(
            !old_tab.needs_seed(first_doc.token()),
            "an empty seed is still a seed"
        );
        assert!(
            old_tab.needs_seed(second_doc.token()),
            "seeding one document does not seed another"
        );
        assert!(old_tab.marks().is_empty(), "an empty seed marks nothing");
        let tree = build(
            page("doc-1", Some(("https://a.test", false)), &[7]),
            &old_tab,
        )?;
        assert_eq!(
            shown(&tree, 7)?,
            Shown::Value,
            "an empty seed masks nothing"
        );
        Ok(())
    }

    #[test]
    fn debug_prints_state_and_counts_only() -> TestResult {
        let mut mirror = PrivateMirror::new(TabOriginState::ExistedAtStart);
        mirror.mark(
            site("https://private-tenant-xyz.example:8443")?,
            DomNodeId::new(4242),
        );
        let opaque = seed_doc(page(
            "doc-secret-token",
            Some(("opaque-nonce-super-secret-999", true)),
            &[7],
        ))?;
        mirror.record_seeded(
            opaque.main_document().ok_or("main doc")?,
            [DomNodeId::new(4242)],
        );

        let debug = format!("{mirror:?}");
        for forbidden in [
            "private-tenant-xyz",
            "8443",
            "opaque-nonce-super-secret-999",
            "doc-secret-token",
            "4242",
        ] {
            assert!(
                !debug.contains(forbidden),
                "PrivateMirror Debug leaked {forbidden:?}: {debug}"
            );
        }
        assert_eq!(
            debug,
            "PrivateMirror { origin_state: ExistedAtStart, marks: PrivateNodes { marked_ids: 1, \
             site_marks: 2, anywhere_marks: 0 }, seeded_docs: 1 }",
            "Debug reports the tab's state and counts only"
        );
        Ok(())
    }

    type CdpResponder = Box<dyn Fn(&str, &Value) -> Result<Value, CdpError> + Send + Sync>;

    struct StubCdp {
        sent: Mutex<Vec<(String, Value)>>,
        responder: CdpResponder,
        targeted: TargetedEvents,
    }

    impl StubCdp {
        fn new(
            responder: impl Fn(&str, &Value) -> Result<Value, CdpError> + Send + Sync + 'static,
        ) -> Arc<Self> {
            Arc::new(Self {
                sent: Mutex::default(),
                responder: Box::new(responder),
                targeted: TargetedEvents::default(),
            })
        }

        fn session(self: &Arc<Self>) -> ProtocolSession {
            ProtocolSession::for_session(
                Arc::clone(self) as Arc<dyn CdpConnection>,
                SessionId::from("S-private"),
            )
        }

        fn sent(&self) -> Vec<(String, Value)> {
            self.sent
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    impl CdpConnection for StubCdp {
        fn send<'a>(
            &'a self,
            method: &'a str,
            params: Value,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<Value, CdpError>> {
            self.sent
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((method.to_owned(), params.clone()));
            let reply = (self.responder)(method, &params);
            Box::pin(async move { reply })
        }

        fn send_raw_json<'a>(
            &'a self,
            _method: &'a str,
            _params_json: &'a str,
            _session: Option<&'a SessionId>,
        ) -> BoxFuture<'a, Result<String, CdpError>> {
            Box::pin(async { Err(CdpError::NotConnected) })
        }

        fn events(&self) -> broadcast::Receiver<Arc<CdpEvent>> {
            broadcast::channel(1).1
        }

        fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
            self.targeted.subscribe(method)
        }

        fn is_connected(&self) -> bool {
            true
        }

        fn connection_epoch(&self) -> u64 {
            1
        }
    }

    #[tokio::test]
    async fn describe_object_node_id_maps_backend_id_missing_node_gone_and_overflow() -> TestResult
    {
        let env = TestEnv::new();
        let scope = OpScope::new(&env, count_op()?, CallHeader::default());

        let ok_cdp = StubCdp::new(|method, params| {
            assert_eq!(method, "DOM.describeNode");
            assert_eq!(params, &json!({ "objectId": "obj-42", "depth": 0 }));
            Ok(json!({ "node": { "backendNodeId": 42 } }))
        });
        let found = describe_object_node_id(&scope, &ok_cdp.session(), "obj-42").await?;
        assert_eq!(
            found,
            Some(DomNodeId::new(42)),
            "valid DOM.describeNode reply must yield DomNodeId"
        );
        assert_eq!(ok_cdp.sent().len(), 1);

        let missing_cdp = StubCdp::new(|_, _| {
            Err(CdpError::Protocol {
                code: -32000,
                message: "No node with given id found".to_owned(),
            })
        });
        let missing = describe_object_node_id(&scope, &missing_cdp.session(), "obj-gone").await?;
        assert_eq!(
            missing, None,
            "missing DOM node protocol error must map to Ok(None)"
        );

        let stale_cdp = StubCdp::new(|_, _| {
            Err(CdpError::Protocol {
                code: -32000,
                message: "Cannot find context with specified id".to_owned(),
            })
        });
        let stale = describe_object_node_id(&scope, &stale_cdp.session(), "obj-stale").await;
        assert!(
            matches!(stale, Err(ProbeFailure::Gone(Ran::No))),
            "stale context must map to ProbeFailure::Gone(Ran::No): {stale:?}"
        );

        let overflow_cdp = StubCdp::new(|_, _| {
            Ok(json!({ "node": { "backendNodeId": i64::from(i32::MAX) + 1 } }))
        });
        let overflow = describe_object_node_id(&scope, &overflow_cdp.session(), "obj-big").await;
        assert!(
            matches!(overflow, Err(ProbeFailure::Broken(_))),
            "backendNodeId overflowing i32 must return ProbeFailure::Broken: {overflow:?}"
        );
        Ok(())
    }

    struct UnusedProbe;

    impl TabScript for UnusedProbe {
        fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
            ProbeAnswer::unexpected()
        }
    }

    #[tokio::test]
    async fn describe_pins_node_ids_handles_empty_live_batch_and_overflow() -> TestResult {
        let tab = ScriptedTab::new(UnusedProbe);
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;

        let empty_cdp = StubCdp::new(|_, _| Err(CdpError::NotConnected));
        let empty_world = IsolatedWorld::for_tests(empty_cdp.session(), SCRIPTED_MAIN, 0, "g");
        let empty = describe_pins_node_ids(&scope, &page, &empty_world, Vec::new()).await?;
        assert!(empty.is_empty());
        assert!(
            empty_cdp.sent().is_empty(),
            "empty pin list must return without sending any CDP command"
        );

        let batch_cdp = StubCdp::new(|_, _| {
            Ok(json!({
                "result": {
                    "type": "object",
                    "deepSerializedValue": {
                        "type": "array",
                        "value": [
                            { "type": "node", "value": { "backendNodeId": 10 } },
                            { "type": "node", "value": { "backendNodeId": 20 } }
                        ]
                    }
                }
            }))
        });
        let batch_world = IsolatedWorld::for_tests(batch_cdp.session(), SCRIPTED_MAIN, 0, "g");
        let ids = describe_pins_node_ids(&scope, &page, &batch_world, vec![Pin(1), Pin(2)]).await?;
        assert_eq!(
            ids,
            vec![DomNodeId::new(10), DomNodeId::new(20)],
            "live batch pins must decode in one pinned_nodes round trip"
        );
        assert_eq!(batch_cdp.sent().len(), 1);

        let overflow_cdp = StubCdp::new(|_, _| {
            Ok(json!({
                "result": {
                    "type": "object",
                    "deepSerializedValue": {
                        "type": "array",
                        "value": [
                            { "type": "node", "value": { "backendNodeId": i64::from(i32::MAX) + 5 } }
                        ]
                    }
                }
            }))
        });
        let overflow_world =
            IsolatedWorld::for_tests(overflow_cdp.session(), SCRIPTED_MAIN, 0, "g");
        let overflow = describe_pins_node_ids(&scope, &page, &overflow_world, vec![Pin(9)]).await;
        assert!(
            matches!(overflow, Err(ProbeFailure::Broken(_))),
            "overflowing backendNodeId in batch must fail as ProbeFailure::Broken: {overflow:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn describe_pins_node_ids_filters_evicted_pins_without_dropping_survivors() -> TestResult
    {
        let tab = ScriptedTab::new(UnusedProbe);
        let run = ScriptedRun::over(&tab).await?;
        let scope = OpScope::new(&run.env, count_op()?, CallHeader::default());
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;

        let mixed_evicted_cdp = StubCdp::new(|method, params| {
            if method == "DOM.describeNode" {
                assert_eq!(params["objectId"], "pin-obj-2");
                return Ok(json!({ "node": { "backendNodeId": 77 } }));
            }
            let first = &params["arguments"][0]["value"];
            if first.is_array() {
                // Batch call sees pin 1 evicted and pin 2 live.
                return Ok(json!({
                    "result": {
                        "type": "object",
                        "deepSerializedValue": {
                            "type": "array",
                            "value": [
                                { "type": "null" },
                                { "type": "node", "value": { "backendNodeId": 77 } }
                            ]
                        }
                    }
                }));
            }
            if first["request"]["pin"] == 1 {
                Ok(json!({ "result": { "type": "object", "subtype": "null" } }))
            } else {
                Ok(json!({
                    "result": { "type": "object", "subtype": "node", "objectId": "pin-obj-2" }
                }))
            }
        });
        let mixed_world =
            IsolatedWorld::for_tests(mixed_evicted_cdp.session(), SCRIPTED_MAIN, 0, "g");
        let surviving =
            describe_pins_node_ids(&scope, &page, &mixed_world, vec![Pin(1), Pin(2)]).await?;
        assert_eq!(
            surviving,
            vec![DomNodeId::new(77)],
            "an evicted sibling pin must be filtered out without losing surviving private marks"
        );

        let single_evicted_cdp = StubCdp::new(|_, _| {
            Ok(json!({
                "result": {
                    "type": "object",
                    "deepSerializedValue": {
                        "type": "array",
                        "value": [{ "type": "null" }]
                    }
                }
            }))
        });
        let single_world =
            IsolatedWorld::for_tests(single_evicted_cdp.session(), SCRIPTED_MAIN, 0, "g");
        let single_evicted =
            describe_pins_node_ids(&scope, &page, &single_world, vec![Pin(5)]).await?;
        assert!(
            single_evicted.is_empty(),
            "a sole evicted pin must return empty without an extra round trip"
        );
        assert_eq!(single_evicted_cdp.sent().len(), 1);
        Ok(())
    }
}
