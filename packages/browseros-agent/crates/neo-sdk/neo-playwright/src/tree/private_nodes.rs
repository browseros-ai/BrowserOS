//! Per-tab mirror of host-typed private input marks, keyed by site and `DomNodeId`.
//!
//! Layer 1 privacy redacts password-like and sensitive `autocomplete` controls inside Blink's APC
//! serializer. Layer 2 covers ordinary boxes the host typed a secret into via private entry: the
//! authoritative mark lives on the element in the frame's isolated world so it survives SDK
//! restarts, and [`PrivateNodes`] mirrors those marks in Rust so `SnapshotTree` masking can redact
//! `field_value` and contenteditable text without an extra page round trip on every capture.

use std::collections::{HashMap, HashSet};
use std::fmt;

use browseros_cdp::CdpError;
use browseros_core::{CoreError, ProtocolSession};
use neo_snapshot_apc::tree::{DocOrigin, DocToken, DomNodeId, TreeDoc};
use serde::Deserialize;
use serde_json::json;
use url::{Origin, Url};

use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::probe::{self, ProbeFailure};
use crate::world::{BackendNodeId, IsolatedWorld, Pin};

/// CDP protocol error returned by `DOM.describeNode` when a remote node handle is no longer backed
/// by a DOM node (for example, after collection or replacement).
const NO_NODE_WITH_ID: &str = "No node with given id found";

/// Normalized security origin or opaque nonce scoping a host-private `DomNodeId` mark.
///
/// Keying marks by `(DomNodeId, SiteKey)` instead of `(DocToken, DomNodeId)` keeps an input
/// masked when the page moves it into another same-origin document (which changes `DocToken` while
/// keeping `DomNodeId`), while preventing an unrelated cross-site renderer from matching the same
/// numeric `DomNodeId`.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct SiteKey(SiteKeyInner);

#[derive(Clone, PartialEq, Eq, Hash)]
enum SiteKeyInner {
    Origin(String),
    Opaque(String),
}

impl SiteKey {
    /// Parses and normalizes an origin or document URL into a [`SiteKey`].
    ///
    /// Tuple origins (`http`, `https`, `ws`, `wss`, `ftp`, and `blob:` wrapping them) normalize via
    /// [`Url::origin`] (`scheme://lowercased-host[:non-default-port]`). Non-tuple schemes such as
    /// `file://` or custom schemes preserve their lowercased scheme and normalized authority (or
    /// lowercased scheme prefix when no authority exists). Empty, whitespace-only, and unparseable
    /// strings return `None`.
    #[must_use]
    pub(crate) fn from_origin(origin: &str) -> Option<Self> {
        let trimmed = origin.trim();
        if trimmed.is_empty() {
            return None;
        }
        let parsed = Url::parse(trimmed).ok()?;
        let normalized = match parsed.origin() {
            Origin::Tuple(..) => parsed.origin().ascii_serialization(),
            Origin::Opaque(_) => normalize_non_tuple_origin(&parsed)?,
        };
        Some(Self(SiteKeyInner::Origin(normalized)))
    }

    /// Converts an APC [`DocOrigin`] into a [`SiteKey`].
    ///
    /// Opaque origins keep their exact non-empty nonce isolated from tuple origins and from other
    /// opaque nonces; non-opaque origins normalize through [`Self::from_origin`].
    #[must_use]
    pub(crate) fn from_doc_origin(origin: &DocOrigin) -> Option<Self> {
        if origin.opaque {
            if origin.value.trim().is_empty() {
                return None;
            }
            Some(Self(SiteKeyInner::Opaque(origin.value.clone())))
        } else {
            Self::from_origin(&origin.value)
        }
    }

    /// Resolves a stitched document's [`SiteKey`], preferring its explicit [`DocOrigin`] and falling
    /// back to its URL when origin metadata is absent.
    #[must_use]
    pub(crate) fn from_doc(doc: &TreeDoc) -> Option<Self> {
        doc.origin()
            .and_then(Self::from_doc_origin)
            .or_else(|| doc.url().and_then(Self::from_origin))
    }
}

impl fmt::Debug for SiteKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            SiteKeyInner::Origin(_) => f.write_str("SiteKey::Origin(<redacted>)"),
            SiteKeyInner::Opaque(_) => f.write_str("SiteKey::Opaque(<redacted>)"),
        }
    }
}

/// Normalizes a parsed non-tuple URL into `scheme://lowercased-host[:port]`, `scheme://`, or
/// `scheme:` so same-scheme non-tuple documents share a stable key without carrying paths.
fn normalize_non_tuple_origin(url: &Url) -> Option<String> {
    let scheme = url.scheme();
    if scheme.is_empty() {
        return None;
    }
    if url.has_authority() {
        let Some(host) = url.host_str() else {
            return Some(format!("{scheme}://"));
        };
        let host = host.to_ascii_lowercase();
        let normalized = match url.port() {
            Some(port) => format!("{scheme}://{host}:{port}"),
            None => format!("{scheme}://{host}"),
        };
        Some(normalized)
    } else {
        Some(format!("{scheme}:"))
    }
}

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

/// Per-tab host mirror of elements marked private by host-driven secret entry.
///
/// Marks are indexed by `DomNodeId -> HashSet<SiteKey>` so moved same-origin inputs remain masked
/// while colliding renderer IDs on other sites do not. Missing document origin metadata fails
/// closed in [`Self::is_private`] so privacy marks can never be bypassed by omitted frame data.
pub(crate) struct PrivateNodes {
    origin_state: TabOriginState,
    marks: HashMap<DomNodeId, HashSet<SiteKey>>,
    seeded_docs: HashSet<DocToken>,
}

impl PrivateNodes {
    /// Creates an empty private-node mirror for a tab with the given [`TabOriginState`].
    #[must_use]
    pub(crate) fn new(origin_state: TabOriginState) -> Self {
        Self {
            origin_state,
            marks: HashMap::new(),
            seeded_docs: HashSet::new(),
        }
    }

    /// Returns whether this tab was opened under the current instance or already existed at start.
    #[must_use]
    pub(crate) const fn origin_state(&self) -> TabOriginState {
        self.origin_state
    }

    /// Records `node` as host-private on `site`.
    pub(crate) fn mark(&mut self, site: SiteKey, node: DomNodeId) {
        self.marks.entry(node).or_default().insert(site);
    }

    /// Records every node in `nodes` as host-private on `site`.
    pub(crate) fn mark_many(&mut self, site: SiteKey, nodes: impl IntoIterator<Item = DomNodeId>) {
        let mut iter = nodes.into_iter();
        let Some(first) = iter.next() else {
            return;
        };
        for node in iter {
            self.marks.entry(node).or_default().insert(site.clone());
        }
        self.marks.entry(first).or_default().insert(site);
    }

    /// Returns whether `node` must be masked in a document on `site`.
    ///
    /// Fails closed (`true`) when `node` carries at least one mark and `site` is `None`, ensuring
    /// missing frame metadata never reveals a host-typed secret.
    #[must_use]
    pub(crate) fn is_private(&self, site: Option<&SiteKey>, node: DomNodeId) -> bool {
        let Some(sites) = self.marks.get(&node) else {
            return false;
        };
        match site {
            Some(site) => sites.contains(site),
            None => !sites.is_empty(),
        }
    }

    /// Resolves `doc`'s [`SiteKey`] and checks [`Self::is_private`] for `node`.
    #[must_use]
    pub(crate) fn is_private_in_doc(&self, doc: &TreeDoc, node: DomNodeId) -> bool {
        let site = SiteKey::from_doc(doc);
        self.is_private(site.as_ref(), node)
    }

    /// Returns whether `doc` must be seeded once from its isolated world before a snapshot may
    /// rely on this mirror alone.
    #[must_use]
    pub(crate) fn needs_seed(&self, doc: &DocToken) -> bool {
        self.origin_state == TabOriginState::ExistedAtStart && !self.seeded_docs.contains(doc)
    }

    /// Imports the pre-existing private marks read from `doc`'s isolated world and marks `doc`
    /// seeded so later snapshots skip the seed read.
    pub(crate) fn record_seeded(
        &mut self,
        doc: DocToken,
        site: SiteKey,
        nodes: impl IntoIterator<Item = DomNodeId>,
    ) {
        self.mark_many(site, nodes);
        self.seeded_docs.insert(doc);
    }

    /// Marks `doc` seeded when its isolated world reported zero marked elements (or no usable
    /// site key).
    pub(crate) fn mark_seeded_empty(&mut self, doc: DocToken) {
        self.seeded_docs.insert(doc);
    }

    /// Returns `true` when no `(site, node)` marks have been recorded.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    /// Returns the total number of distinct `(site, node)` marks recorded for this tab.
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.marks.values().map(HashSet::len).sum()
    }
}

impl fmt::Debug for PrivateNodes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrivateNodes")
            .field("origin_state", &self.origin_state)
            .field("marked_nodes", &self.marks.len())
            .field("total_marks", &self.len())
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
fn backend_to_dom_node_id(id: BackendNodeId) -> Result<DomNodeId, ProbeFailure> {
    let raw = i32::try_from(id.0).map_err(|error| {
        ProbeFailure::Broken(format!(
            "DOM backendNodeId {} exceeds i32 range: {error}",
            id.0
        ))
    })?;
    Ok(DomNodeId::new(raw))
}

/// Describes a remote DOM object handle to obtain its `DomNodeId` for mirroring in
/// [`PrivateNodes`].
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
/// [`PrivateNodes`].
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

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::{Arc, Mutex, PoisonError};

    use browseros_cdp::{CdpError, CdpEvent, TargetedEvents};
    use browseros_core::{CdpConnection, ProtocolSession, SessionId};
    use futures_util::future::BoxFuture;
    use neo_snapshot_apc::tree::{
        ApcBuilder, BuildOptions, DocOrigin, DocToken, DomNodeId, NodeBuilder, SnapshotTree,
    };
    use serde_json::{Value, json};
    use tokio::sync::{broadcast, mpsc};

    use super::{SiteKey, TabOriginState, describe_object_node_id, describe_pins_node_ids};
    use crate::op_scope::OpScope;
    use crate::page::OpPage;
    use crate::probe::{ProbeFailure, Ran};
    use crate::target::CallHeader;
    use crate::test_env::TestEnv;
    use crate::test_support::{
        ProbeAnswer, SCRIPTED_MAIN, ScriptedRun, ScriptedTab, TabScript, TestResult, count_op,
    };
    use crate::tree::PrivateNodes;
    use crate::world::{IsolatedWorld, Pin};

    fn doc_with(
        token: &str,
        url: Option<&str>,
        origin: Option<(&str, bool)>,
    ) -> Result<SnapshotTree, Box<dyn Error>> {
        let mut builder = ApcBuilder::new(token).root(NodeBuilder::root().dom_id(1));
        if let Some(url) = url {
            builder = builder.main_url(url);
        }
        if let Some((value, opaque)) = origin {
            builder = builder.main_origin(value, opaque);
        }
        Ok(SnapshotTree::build(builder.build(), BuildOptions::new())?)
    }

    #[test]
    fn normalized_same_origin_marks_follow_moved_nodes_without_cross_site_bleed()
    -> Result<(), Box<dyn Error>> {
        let mut mirror = PrivateNodes::new(TabOriginState::OpenedUnderInstance);
        assert!(mirror.is_empty());
        assert_eq!(mirror.len(), 0);
        assert_eq!(mirror.origin_state(), TabOriginState::OpenedUnderInstance);

        let site_a_default_port =
            SiteKey::from_origin("https://A.TEST:443/login?next=/home").ok_or("valid origin")?;
        let site_a_canonical = SiteKey::from_origin("https://a.test").ok_or("valid origin")?;
        let site_a_alt_port = SiteKey::from_origin("https://a.test:8443").ok_or("valid origin")?;
        let site_b = SiteKey::from_origin("https://b.test").ok_or("valid origin")?;

        assert_eq!(
            site_a_default_port, site_a_canonical,
            "default HTTPS port and host case must normalize to the same SiteKey"
        );
        assert_ne!(
            site_a_canonical, site_a_alt_port,
            "non-default ports represent distinct origins"
        );
        assert!(
            SiteKey::from_origin("").is_none()
                && SiteKey::from_origin("   \t ").is_none()
                && SiteKey::from_origin("not a url").is_none(),
            "empty, whitespace-only, and invalid strings must be rejected"
        );

        let file_a = SiteKey::from_origin("file:///tmp/one.html").ok_or("file url")?;
        let file_b = SiteKey::from_origin("FILE:///var/two.html").ok_or("file url")?;
        assert_eq!(
            file_a, file_b,
            "non-tuple file URLs without authority must normalize to a stable file:// key"
        );
        let ext_upper =
            SiteKey::from_origin("chrome-extension://ABC123/popup.html").ok_or("extension url")?;
        let ext_lower = SiteKey::from_origin("chrome-extension://abc123/options.html")
            .ok_or("extension url")?;
        assert_eq!(
            ext_upper, ext_lower,
            "custom scheme authority hosts must normalize case-insensitively"
        );
        assert!(
            SiteKey::from_origin("about:blank").is_some(),
            "valid non-authority scheme URLs must retain their lowercased scheme prefix"
        );

        let moved_node = DomNodeId::new(42);
        mirror.mark(site_a_default_port, moved_node);
        assert!(!mirror.is_empty());
        assert_eq!(mirror.len(), 1);
        assert!(
            mirror.is_private(Some(&site_a_canonical), moved_node),
            "normalized https://a.test must match a mark recorded under https://A.TEST:443"
        );
        assert!(
            !mirror.is_private(Some(&site_b), moved_node),
            "same numeric DomNodeId on https://b.test must not inherit https://a.test's mark"
        );

        let moved_doc = doc_with("doc-frame-2", Some("https://a.test/frame"), None)?;
        let other_site_doc = doc_with(
            "doc-site-b",
            Some("https://b.test/form"),
            Some(("https://b.test", false)),
        )?;
        let moved_tree_doc = moved_doc.main_document().ok_or("main doc")?;
        let other_tree_doc = other_site_doc.main_document().ok_or("main doc")?;

        assert!(
            mirror.is_private_in_doc(moved_tree_doc, moved_node),
            "a node moved to another same-origin document must stay masked"
        );
        assert!(
            !mirror.is_private_in_doc(other_tree_doc, moved_node),
            "an unrelated cross-site document with the same DomNodeId must stay unmasked"
        );

        mirror.mark_many(site_b.clone(), [DomNodeId::new(42), DomNodeId::new(99)]);
        assert_eq!(
            mirror.len(),
            3,
            "len must count distinct (site, node) pairs across both origins"
        );
        assert!(mirror.is_private(Some(&site_b), DomNodeId::new(42)));
        assert!(mirror.is_private(Some(&site_b), DomNodeId::new(99)));
        Ok(())
    }

    #[test]
    fn opaque_origin_marks_match_only_the_exact_opaque_nonce() -> Result<(), Box<dyn Error>> {
        let nonce_a = DocOrigin {
            opaque: true,
            value: "opaque-nonce-alpha-111".to_owned(),
        };
        let nonce_b = DocOrigin {
            opaque: true,
            value: "opaque-nonce-beta-222".to_owned(),
        };
        let empty_opaque = DocOrigin {
            opaque: true,
            value: "   ".to_owned(),
        };
        assert!(
            SiteKey::from_doc_origin(&empty_opaque).is_none(),
            "empty opaque nonces must be rejected"
        );

        let key_a = SiteKey::from_doc_origin(&nonce_a).ok_or("opaque key a")?;
        let key_a_same = SiteKey::from_doc_origin(&nonce_a).ok_or("opaque key a repeat")?;
        let key_b = SiteKey::from_doc_origin(&nonce_b).ok_or("opaque key b")?;
        let tuple_key = SiteKey::from_origin("https://a.test").ok_or("tuple key")?;

        let mut mirror = PrivateNodes::new(TabOriginState::OpenedUnderInstance);
        let node = DomNodeId::new(7);
        mirror.mark(key_a, node);

        assert!(
            mirror.is_private(Some(&key_a_same), node),
            "exact opaque nonce must match its own mark"
        );
        assert!(
            !mirror.is_private(Some(&key_b), node),
            "a different opaque nonce must not match"
        );
        assert!(
            !mirror.is_private(Some(&tuple_key), node),
            "a tuple origin must not match an opaque origin mark"
        );

        let opaque_tree = doc_with(
            "doc-opaque",
            Some("https://a.test/sandboxed"),
            Some(("opaque-nonce-alpha-111", true)),
        )?;
        let opaque_doc = opaque_tree.main_document().ok_or("main doc")?;
        assert!(
            mirror.is_private_in_doc(opaque_doc, node),
            "SiteKey::from_doc must prefer the document's opaque DocOrigin over its URL"
        );
        Ok(())
    }

    #[test]
    fn missing_origin_and_url_fails_closed_only_for_marked_nodes() -> Result<(), Box<dyn Error>> {
        let mut mirror = PrivateNodes::new(TabOriginState::OpenedUnderInstance);
        let site = SiteKey::from_origin("https://secret.example").ok_or("site")?;
        let marked = DomNodeId::new(15);
        let unmarked = DomNodeId::new(16);
        mirror.mark(site, marked);

        let bare_tree = doc_with("doc-no-metadata", None, None)?;
        let bare_doc = bare_tree.main_document().ok_or("main doc")?;
        assert!(
            SiteKey::from_doc(bare_doc).is_none(),
            "document with neither origin nor URL yields no SiteKey"
        );
        assert!(
            mirror.is_private_in_doc(bare_doc, marked),
            "missing document metadata must fail closed for a marked DomNodeId"
        );
        assert!(
            !mirror.is_private_in_doc(bare_doc, unmarked),
            "missing document metadata must still return false for an unmarked DomNodeId"
        );
        Ok(())
    }

    #[test]
    fn seeding_policy_tracks_preexisting_documents_once_and_skips_new_tabs()
    -> Result<(), Box<dyn Error>> {
        let doc_1 = DocToken::new("doc-1").ok_or("doc token")?;
        let doc_2 = DocToken::new("doc-2").ok_or("doc token")?;
        let site = SiteKey::from_origin("https://bank.example").ok_or("site")?;

        let new_tab = PrivateNodes::new(TabOriginState::OpenedUnderInstance);
        assert!(
            !new_tab.needs_seed(&doc_1),
            "tabs opened under this Playwright instance never need page-side seeding"
        );

        let mut old_tab = PrivateNodes::new(TabOriginState::ExistedAtStart);
        assert_eq!(old_tab.origin_state(), TabOriginState::ExistedAtStart);
        assert!(
            old_tab.needs_seed(&doc_1),
            "pre-existing tabs must seed each unseen document once"
        );
        assert!(old_tab.needs_seed(&doc_2));

        old_tab.record_seeded(doc_1.clone(), site.clone(), [DomNodeId::new(3)]);
        assert!(
            !old_tab.needs_seed(&doc_1),
            "record_seeded must stop further seed reads for that DocToken"
        );
        assert!(
            old_tab.is_private(Some(&site), DomNodeId::new(3)),
            "record_seeded must import the seeded nodes under the document's site"
        );
        assert!(
            old_tab.needs_seed(&doc_2),
            "seeding doc-1 must not mark an unrelated document seeded"
        );

        old_tab.mark_seeded_empty(doc_2.clone());
        assert!(
            !old_tab.needs_seed(&doc_2),
            "mark_seeded_empty must stop repeat seed reads for an empty frame"
        );
        Ok(())
    }

    #[test]
    fn debug_output_redacts_origins_urls_and_opaque_nonces() -> Result<(), Box<dyn Error>> {
        let origin_secret = "https://private-tenant-xyz.example:8443/account";
        let nonce_secret = "opaque-nonce-super-secret-999";

        let origin_key = SiteKey::from_origin(origin_secret).ok_or("origin key")?;
        let opaque_key = SiteKey::from_doc_origin(&DocOrigin {
            opaque: true,
            value: nonce_secret.to_owned(),
        })
        .ok_or("opaque key")?;

        let origin_debug = format!("{origin_key:?}");
        let opaque_debug = format!("{opaque_key:?}");
        assert_eq!(origin_debug, "SiteKey::Origin(<redacted>)");
        assert_eq!(opaque_debug, "SiteKey::Opaque(<redacted>)");

        let mut mirror = PrivateNodes::new(TabOriginState::ExistedAtStart);
        mirror.mark(origin_key, DomNodeId::new(11));
        mirror.mark(opaque_key, DomNodeId::new(11));
        mirror.mark_seeded_empty(DocToken::new("doc-secret-token").ok_or("doc token")?);

        let mirror_debug = format!("{mirror:?}");
        for forbidden in [
            "private-tenant-xyz",
            "8443",
            "account",
            nonce_secret,
            "doc-secret-token",
        ] {
            assert!(
                !mirror_debug.contains(forbidden),
                "PrivateNodes Debug output leaked {forbidden:?}: {mirror_debug}"
            );
        }
        assert!(
            mirror_debug.contains("origin_state: ExistedAtStart")
                && mirror_debug.contains("marked_nodes: 1")
                && mirror_debug.contains("total_marks: 2")
                && mirror_debug.contains("seeded_docs: 1"),
            "PrivateNodes Debug must report only state and counts: {mirror_debug}"
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
