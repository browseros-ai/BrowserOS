use std::fmt;

use crate::proto::{FrameData, RedactedFrameReason, SecurityOrigin};

use super::node_key::{DocId, DocSeq, DocToken, DomNodeId, FrameToken, NodeId, NodeKey, NodeRange};

/// Normalized security origin for a [`TreeDoc`], detached from raw protobuf types.
///
/// Distinct from a document's URL because documents such as `about:blank` inherit their opener's
/// origin or carry an opaque nonce.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocOrigin {
    /// Whether this origin is opaque (serialized as a nonce rather than `scheme://host:port`).
    pub opaque: bool,
    /// Nonce string for opaque origins, or serialized `scheme://host:port` otherwise.
    pub value: String,
}

impl DocOrigin {
    pub(crate) fn from_proto(origin: SecurityOrigin) -> Self {
        Self {
            opaque: origin.opaque.unwrap_or(false),
            value: origin.value.unwrap_or_default(),
        }
    }
}

/// Explanation for why an embedded `<iframe>` could not be stitched into a [`TreeDoc`].
///
/// Rendered into snapshot diagnostics and action errors so callers distinguish BrowserOS subframe
/// timeouts from cross-site redaction or missing document identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PartialFrameReason {
    /// Frame capture hit the BrowserOS subframe deadline (`REASON_BROWSEROS_TIMEOUT`).
    Timeout {
        /// Configured subframe timeout budget in milliseconds, when reported by the browser.
        timeout_ms: Option<u32>,
    },
    /// Frame was omitted because it is cross-site (`RedactedFrameReason::CrossSite`).
    CrossSite,
    /// Frame was omitted because it is cross-origin (`RedactedFrameReason::CrossOrigin`).
    CrossOrigin,
    /// Frame was redacted with an unspecified or unrecognized open-enum reason code.
    Redacted(i32),
    /// Frame node carried no `FrameData` or `RedactedFrameMetadata` payload.
    MissingContent,
    /// Frame node carried `FrameData` without a non-empty `document_identifier.serialized_token`.
    MissingDocumentToken,
}

impl PartialFrameReason {
    #[must_use]
    pub(crate) fn from_redacted_reason(
        reason: Option<i32>,
        subframe_timeout_ms: Option<u32>,
    ) -> Self {
        match reason.and_then(|code| RedactedFrameReason::try_from(code).ok()) {
            Some(RedactedFrameReason::BrowserOsTimeout) => Self::Timeout {
                timeout_ms: subframe_timeout_ms,
            },
            Some(RedactedFrameReason::CrossSite) => Self::CrossSite,
            Some(RedactedFrameReason::CrossOrigin) => Self::CrossOrigin,
            Some(RedactedFrameReason::Unspecified) | None => Self::Redacted(reason.unwrap_or(0)),
        }
    }
}

impl fmt::Display for PartialFrameReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout {
                timeout_ms: Some(ms),
            } => write!(f, "timeout after {ms} ms"),
            Self::Timeout { timeout_ms: None } => f.write_str("timeout"),
            Self::CrossSite => f.write_str("cross-site frame redacted"),
            Self::CrossOrigin => f.write_str("cross-origin frame redacted"),
            Self::Redacted(code) => write!(f, "frame redacted ({code})"),
            Self::MissingContent => f.write_str("frame content unavailable"),
            Self::MissingDocumentToken => f.write_str("frame missing document token"),
        }
    }
}

/// Record of an embedded `<iframe>` whose child document was omitted, timed out, or unidentifiable.
///
/// Kept alongside [`TreeDoc`] entries on [`SnapshotTree`](super::SnapshotTree) so snapshot
/// rendering and callers can surface partial-frame boundaries explicitly instead of silently
/// dropping subframes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartialFrame {
    /// Arena index of the `<iframe>` node in [`Self::parent_doc`].
    pub iframe_node: NodeId,
    /// Composite key of the `<iframe>` node in [`Self::parent_doc`], when it has a `DomNodeId`.
    pub iframe_key: Option<NodeKey>,
    /// Owning document that contains this `<iframe>` element.
    pub parent_doc: DocId,
    /// `DevTools` frame token reported for this iframe when BrowserOS metadata is present.
    pub frame_token: Option<FrameToken>,
    /// Reason the iframe's child document could not be stitched.
    pub reason: PartialFrameReason,
}

pub(crate) struct TreeDocInit {
    pub(crate) id: DocId,
    pub(crate) token: DocToken,
    pub(crate) frame_token: Option<FrameToken>,
    pub(crate) seq: Option<DocSeq>,
    pub(crate) parent_doc: Option<DocId>,
    pub(crate) iframe_node: Option<NodeId>,
    pub(crate) iframe_key: Option<NodeKey>,
    pub(crate) node_range: NodeRange,
    pub(crate) frame_data: FrameData,
}

/// Normalized metadata and arena boundaries for one stitched document in a
/// [`SnapshotTree`](super::SnapshotTree).
///
/// Slot `0` ([`DocId::MAIN`]) is the main document; subsequent entries correspond to embedded
/// iframes whose `FrameData` carried a valid [`DocToken`]. Its manual `Debug` implementation
/// redacts `url`, `title`, and `origin` strings so logging a document never leaks page content.
pub struct TreeDoc {
    id: DocId,
    token: DocToken,
    frame_token: Option<FrameToken>,
    seq: Option<DocSeq>,
    parent_doc: Option<DocId>,
    iframe_node: Option<NodeId>,
    iframe_key: Option<NodeKey>,
    root_node: Option<NodeId>,
    node_range: NodeRange,
    node_count: usize,
    url: Option<String>,
    title: Option<String>,
    origin: Option<DocOrigin>,
    focused_node_id: Option<DomNodeId>,
}

impl TreeDoc {
    pub(crate) fn from_frame_data(init: TreeDocInit) -> Self {
        let focused_node_id = init
            .frame_data
            .frame_interaction_info
            .and_then(|info| info.focused_node_id)
            .map(DomNodeId::new);
        let origin = init.frame_data.security_origin.map(DocOrigin::from_proto);

        Self {
            id: init.id,
            token: init.token,
            frame_token: init.frame_token,
            seq: init.seq,
            parent_doc: init.parent_doc,
            iframe_node: init.iframe_node,
            iframe_key: init.iframe_key,
            root_node: None,
            node_range: init.node_range,
            node_count: 0,
            url: init.frame_data.url,
            title: init.frame_data.title,
            origin,
            focused_node_id,
        }
    }

    /// Arena index of this document in [`SnapshotTree::documents`](super::SnapshotTree::documents).
    #[must_use]
    pub const fn id(&self) -> DocId {
        self.id
    }

    /// Stable non-empty document token identifying this document across APC captures.
    #[must_use]
    pub const fn token(&self) -> &DocToken {
        &self.token
    }

    /// `DevTools` frame token when provided by BrowserOS frame metadata.
    #[must_use]
    pub const fn frame_token(&self) -> Option<&FrameToken> {
        self.frame_token.as_ref()
    }

    /// Caller-assigned session sequence number for this document, if a `seq_of` callback was set.
    #[must_use]
    pub const fn seq(&self) -> Option<DocSeq> {
        self.seq
    }

    /// Whether this document is the top-level main document ([`DocId::MAIN`]).
    #[must_use]
    pub const fn is_main(&self) -> bool {
        self.id.raw() == DocId::MAIN.raw()
    }

    /// Owning parent document containing this document's `<iframe>`, or `None` for the main doc.
    #[must_use]
    pub const fn parent_doc(&self) -> Option<DocId> {
        self.parent_doc
    }

    /// Arena index of the `<iframe>` node in [`Self::parent_doc`], or `None` for the main doc.
    #[must_use]
    pub const fn iframe_node(&self) -> Option<NodeId> {
        self.iframe_node
    }

    /// Composite key of the `<iframe>` node in [`Self::parent_doc`], when available.
    #[must_use]
    pub const fn iframe_key(&self) -> Option<&NodeKey> {
        self.iframe_key.as_ref()
    }

    /// First arena node directly owned by this document, or `None` if the document has zero nodes.
    #[must_use]
    pub const fn root_node(&self) -> Option<NodeId> {
        self.root_node
    }

    /// Contiguous arena range spanning this document's entire subtree, including nested iframes.
    ///
    /// For the main document this covers `0..nodes.len()`; for a child document it covers
    /// `(iframe_node + 1)..iframe.subtree_end`.
    #[must_use]
    pub const fn node_range(&self) -> NodeRange {
        self.node_range
    }

    /// Exact count of nodes directly owned by this document, excluding nested child documents.
    #[must_use]
    pub const fn node_count(&self) -> usize {
        self.node_count
    }

    /// Current URL reported for this frame, if present.
    #[must_use]
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    /// Document title reported for this frame, if present.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Normalized security origin of this document, if present.
    #[must_use]
    pub const fn origin(&self) -> Option<&DocOrigin> {
        self.origin.as_ref()
    }

    /// DOM node ID holding focus within this frame, if reported.
    #[must_use]
    pub const fn focused_node_id(&self) -> Option<DomNodeId> {
        self.focused_node_id
    }

    pub(crate) fn record_node(&mut self, node_id: NodeId) {
        if self.root_node.is_none() {
            self.root_node = Some(node_id);
        }
        self.node_count += 1;
    }

    pub(crate) fn set_node_range(&mut self, range: NodeRange) {
        self.node_range = range;
    }
}

impl fmt::Debug for TreeDoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeDoc")
            .field("id", &self.id)
            .field("token", &self.token)
            .field("frame_token", &self.frame_token)
            .field("seq", &self.seq)
            .field("parent_doc", &self.parent_doc)
            .field("iframe_node", &self.iframe_node)
            .field("iframe_key", &self.iframe_key)
            .field("root_node", &self.root_node)
            .field("node_range", &self.node_range)
            .field("node_count", &self.node_count)
            .field("has_url", &self.url.is_some())
            .field("has_title", &self.title.is_some())
            .field("has_origin", &self.origin.is_some())
            .field("focused_node_id", &self.focused_node_id)
            .finish()
    }
}
