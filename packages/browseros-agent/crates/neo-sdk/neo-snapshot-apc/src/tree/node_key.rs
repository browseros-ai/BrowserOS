use std::fmt;
use std::num::NonZeroU32;
use std::ops::Range;
use std::sync::Arc;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

struct NonEmptyTokenVisitor {
    kind: &'static str,
}

impl Visitor<'_> for NonEmptyTokenVisitor {
    type Value = Arc<str>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "a non-empty {}", self.kind)
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value.is_empty() {
            return Err(E::custom(format!("{} must not be empty", self.kind)));
        }
        Ok(Arc::from(value))
    }
}

/// Validated non-empty document token from `FrameData.document_identifier.serialized_token`.
///
/// Uniquely identifies a loaded document across the browser's lifetime, so pairing it with a
/// [`DomNodeId`] disambiguates nodes whose renderer-assigned DOM IDs collide across iframes.
#[derive(Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocToken(Arc<str>);

impl DocToken {
    /// Wraps `token` if non-empty, returning `None` for empty strings.
    #[must_use]
    pub fn new(token: &str) -> Option<Self> {
        if token.is_empty() {
            return None;
        }
        Some(Self(Arc::from(token)))
    }

    /// Returns the underlying non-empty document token string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Clone for DocToken {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl fmt::Display for DocToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for DocToken {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for DocToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let token = deserializer.deserialize_str(NonEmptyTokenVisitor { kind: "DocToken" })?;
        Ok(Self(token))
    }
}

/// Validated non-empty `DevTools` frame token reported for a frame or redacted iframe.
///
/// Correlates an APC frame with CDP `FrameId` targets when BrowserOS extension metadata provides
/// it.
#[derive(Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FrameToken(Arc<str>);

impl FrameToken {
    /// Wraps `token` if non-empty, returning `None` for empty strings.
    #[must_use]
    pub fn new(token: &str) -> Option<Self> {
        if token.is_empty() {
            return None;
        }
        Some(Self(Arc::from(token)))
    }

    /// Returns the underlying non-empty frame token string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Clone for FrameToken {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl fmt::Display for FrameToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for FrameToken {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for FrameToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let token = deserializer.deserialize_str(NonEmptyTokenVisitor { kind: "FrameToken" })?;
        Ok(Self(token))
    }
}

/// Renderer-assigned DOM node ID (`common_ancestor_dom_node_id`), scoped to a single document.
///
/// Raw IDs are only unique within their owning document and routinely collide across cross-site
/// iframes; combine with [`DocToken`] in [`NodeKey`] for cross-frame identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DomNodeId(i32);

impl DomNodeId {
    /// Wraps a raw `common_ancestor_dom_node_id` value.
    #[must_use]
    pub const fn new(raw: i32) -> Self {
        Self(raw)
    }

    /// Returns the underlying `i32` DOM node ID.
    #[must_use]
    pub const fn raw(self) -> i32 {
        self.0
    }

    /// Widens this DOM node ID to the `i64` `backendNodeId` representation expected by CDP calls.
    #[must_use]
    pub const fn as_backend_node_id(self) -> i64 {
        self.0 as i64
    }
}

impl fmt::Display for DomNodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Composite document-and-node key uniquely identifying a DOM node across all frames in a page.
///
/// Pairing [`Self::doc`] with [`Self::node`] guarantees that snapshot references and `getByRole`
/// matches never alias across iframes even when separate renderers assign the same raw
/// [`DomNodeId`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeKey {
    /// Stable document token identifying the owning frame's document.
    pub doc: DocToken,
    /// Renderer-assigned DOM node ID within [`Self::doc`].
    pub node: DomNodeId,
}

impl NodeKey {
    /// Constructs a composite key from an owning [`DocToken`] and document-scoped [`DomNodeId`].
    #[must_use]
    pub const fn new(doc: DocToken, node: DomNodeId) -> Self {
        Self { doc, node }
    }
}

/// Caller-assigned monotonic sequence number for a document across snapshots in a session.
///
/// Assigned via [`BuildOptions::with_seq_of`](super::BuildOptions::with_seq_of) so compact ref
/// formats can identify a document without repeating its full [`DocToken`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DocSeq(u32);

impl DocSeq {
    /// Wraps a raw document sequence number.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns the underlying `u32` sequence number.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl fmt::Display for DocSeq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Arena index into [`SnapshotTree::documents`](super::SnapshotTree::documents).
///
/// Valid only within the [`SnapshotTree`](super::SnapshotTree) that minted it; slot `0`
/// ([`Self::MAIN`]) is always the top-level document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocId(u32);

impl DocId {
    /// Arena index of the top-level main document.
    pub const MAIN: Self = Self(0);

    /// Wraps a raw arena document index.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns this document ID as a `usize` slice index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Returns the raw `u32` arena index.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Arena index into [`SnapshotTree::nodes`](super::SnapshotTree::nodes) in flat-tree preorder.
///
/// Because nodes are stored in flat-tree preorder, every node's descendants occupy the contiguous
/// half-open range `[id, subtree_end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

impl NodeId {
    /// Wraps a raw arena node index.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns this node ID as a `usize` slice index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Returns the raw `u32` arena index.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Validated half-open range `[start, end)` of [`NodeId`] arena indices with `start <= end`.
///
/// Represents either a node's preorder subtree (`id..subtree_end`) or a document's entire spanned
/// arena slice (including any nested child iframes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeRange {
    start: NodeId,
    end: NodeId,
}

impl NodeRange {
    /// Constructs a half-open range `[start, end)` if `start <= end`, or `None` if inverted.
    #[must_use]
    pub const fn new(start: NodeId, end: NodeId) -> Option<Self> {
        if start.raw() <= end.raw() {
            Some(Self { start, end })
        } else {
            None
        }
    }

    /// Constructs an empty range `[at, at)` anchored at `at`.
    #[must_use]
    pub const fn empty_at(at: NodeId) -> Self {
        Self { start: at, end: at }
    }

    pub(crate) const fn from_start_and_len(start: NodeId, len: NonZeroU32) -> Self {
        Self {
            start,
            end: NodeId::new(start.raw() + len.get()),
        }
    }

    /// Inclusive start index of the range.
    #[must_use]
    pub const fn start(self) -> NodeId {
        self.start
    }

    /// Exclusive end index of the range.
    #[must_use]
    pub const fn end(self) -> NodeId {
        self.end
    }

    /// Number of node slots spanned by this range.
    #[must_use]
    pub const fn len(self) -> usize {
        (self.end.raw() - self.start.raw()) as usize
    }

    /// Whether this range spans zero nodes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start.raw() == self.end.raw()
    }

    /// Whether `id` falls within `[start, end)`.
    #[must_use]
    pub const fn contains(self, id: NodeId) -> bool {
        self.start.raw() <= id.raw() && id.raw() < self.end.raw()
    }

    /// Converts this range into a `Range<usize>` for slicing the node arena.
    #[must_use]
    pub const fn as_usize_range(self) -> Range<usize> {
        self.start.index()..self.end.index()
    }
}
