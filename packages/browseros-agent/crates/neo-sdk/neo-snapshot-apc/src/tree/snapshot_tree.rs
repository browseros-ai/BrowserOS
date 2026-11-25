use std::collections::HashMap;
use std::fmt;
use std::iter::FusedIterator;

use crate::proto::AnnotatedPageContent;

use super::build_error::BuildError;
use super::frame_stitch::{self, StitchedTree};
use super::node_key::{DocId, DocSeq, DocToken, FrameToken, NodeId, NodeKey, NodeRange};
use super::tree_doc::{PartialFrame, TreeDoc};
use super::tree_node::TreeNode;
use super::{hidden, role, state};

/// Minimum BrowserOS APC contract version required to construct a [`SnapshotTree`].
pub const MIN_CONTRACT_VERSION: u32 = 1;

/// Captured viewport rectangle in device-independent pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    /// Left coordinate of the viewport.
    pub x: i32,
    /// Top coordinate of the viewport.
    pub y: i32,
    /// Viewport width in pixels.
    pub width: i32,
    /// Viewport height in pixels.
    pub height: i32,
}

type SeqAssigner<'a> = &'a dyn Fn(&DocToken, Option<&FrameToken>) -> DocSeq;

/// Configuration controlling how [`SnapshotTree::build`] validates contract metadata and assigns
/// session-scoped [`DocSeq`] identifiers.
#[must_use]
#[derive(Clone, Copy)]
pub struct BuildOptions<'a> {
    min_contract_version: u32,
    seq_of: Option<SeqAssigner<'a>>,
}

impl Default for BuildOptions<'_> {
    fn default() -> Self {
        Self {
            min_contract_version: MIN_CONTRACT_VERSION,
            seq_of: None,
        }
    }
}

impl<'a> BuildOptions<'a> {
    /// Creates default build options enforcing [`MIN_CONTRACT_VERSION`] with no `seq_of` callback.
    pub fn new() -> Self {
        Self::default()
    }

    /// Raises the minimum accepted BrowserOS contract version.
    ///
    /// Values below [`MIN_CONTRACT_VERSION`] are clamped up to [`MIN_CONTRACT_VERSION`] so callers
    /// can strengthen the contract requirement without disabling the baseline gate.
    pub fn with_min_contract_version(mut self, version: u32) -> Self {
        self.min_contract_version = version.max(MIN_CONTRACT_VERSION);
        self
    }

    /// Supplies a callback invoked for each stitched document to assign a session-scoped
    /// [`DocSeq`].
    pub fn with_seq_of(mut self, seq_of: SeqAssigner<'a>) -> Self {
        self.seq_of = Some(seq_of);
        self
    }

    /// Returns the effective minimum contract version floor.
    #[must_use]
    pub const fn min_contract_version(&self) -> u32 {
        self.min_contract_version
    }

    pub(crate) fn assign_seq(
        &self,
        token: &DocToken,
        frame_token: Option<&FrameToken>,
    ) -> Option<DocSeq> {
        self.seq_of.map(|assign| assign(token, frame_token))
    }
}

impl fmt::Debug for BuildOptions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BuildOptions")
            .field("min_contract_version", &self.min_contract_version)
            .field("has_seq_of", &self.seq_of.is_some())
            .finish()
    }
}

/// Immutable, flat-tree preorder arena normalizing a multi-frame [`AnnotatedPageContent`] snapshot.
///
/// Snapshot rendering and `getByRole` queries both read this single tree so refs, frame boundaries,
/// and containment checks cannot drift. Its manual `Debug` implementation prints counts and
/// structural metadata only, never page text, form values, labels, titles, or URLs.
pub struct SnapshotTree {
    nodes: Vec<TreeNode>,
    docs: Vec<TreeDoc>,
    partial_frames: Vec<PartialFrame>,
    by_key: HashMap<NodeKey, NodeId>,
    doc_by_token: HashMap<DocToken, DocId>,
    viewport: Option<Viewport>,
    contract_version: u32,
    subframe_timeout_ms: Option<u32>,
}

impl SnapshotTree {
    /// Validates contract metadata and stitches `apc` into an immutable [`SnapshotTree`].
    pub fn build(apc: AnnotatedPageContent, options: BuildOptions<'_>) -> Result<Self, BuildError> {
        let meta = read_contract_meta(&apc);
        let contract_version = check_contract_version(meta.version, options.min_contract_version)?;

        let StitchedTree {
            mut nodes,
            docs,
            partial_frames,
            by_key,
            doc_by_token,
            viewport,
        } = frame_stitch::frame_stitch(apc, options, meta.subframe_timeout_ms)?;

        let hidden_flags = hidden::compute_hidden(&nodes);
        let roles = role::compute_roles(&nodes, &hidden_flags);
        let states = state::compute_states(&nodes, &docs, &roles);
        for (((node, hidden), role), state) in
            nodes.iter_mut().zip(hidden_flags).zip(roles).zip(states)
        {
            node.set_hidden(hidden);
            node.set_role(role);
            node.set_state(state);
        }

        Ok(Self {
            nodes,
            docs,
            partial_frames,
            by_key,
            doc_by_token,
            viewport,
            contract_version,
            subframe_timeout_ms: meta.subframe_timeout_ms,
        })
    }

    /// All stitched documents in preorder encounter order, starting with [`DocId::MAIN`].
    #[must_use]
    pub fn documents(&self) -> &[TreeDoc] {
        &self.docs
    }

    /// Looks up a stitched document by its arena [`DocId`].
    #[must_use]
    pub fn document(&self, id: DocId) -> Option<&TreeDoc> {
        self.docs.get(id.index())
    }

    /// Returns the top-level main document ([`DocId::MAIN`]).
    #[must_use]
    pub fn main_document(&self) -> Option<&TreeDoc> {
        self.docs.first()
    }

    /// Looks up a stitched document by its stable [`DocToken`].
    #[must_use]
    pub fn document_by_token(&self, token: &DocToken) -> Option<&TreeDoc> {
        self.doc_by_token
            .get(token)
            .and_then(|&id| self.document(id))
    }

    /// All embedded iframes whose child documents were omitted, redacted, or unidentifiable.
    #[must_use]
    pub fn partial_frames(&self) -> &[PartialFrame] {
        &self.partial_frames
    }

    /// Looks up a [`PartialFrame`] entry by index (typically from [`TreeNode::partial_frame_index`]).
    #[must_use]
    pub fn partial_frame(&self, index: usize) -> Option<&PartialFrame> {
        self.partial_frames.get(index)
    }

    /// All nodes in the stitched flat-tree preorder arena.
    #[must_use]
    pub fn nodes(&self) -> &[TreeNode] {
        &self.nodes
    }

    /// Looks up a node by its arena [`NodeId`].
    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&TreeNode> {
        self.nodes.get(id.index())
    }

    /// Looks up a node by its cross-frame composite [`NodeKey`].
    #[must_use]
    pub fn node_by_key(&self, key: &NodeKey) -> Option<&TreeNode> {
        self.by_key.get(key).and_then(|&id| self.node(id))
    }

    /// Constructs the composite [`NodeKey`] for `id`, or `None` if `id` is out of bounds or lacks
    /// a `DomNodeId`.
    #[must_use]
    pub fn node_key(&self, id: NodeId) -> Option<NodeKey> {
        let node = self.node(id)?;
        let dom = node.dom_node_id()?;
        let doc = self.document(node.doc())?;
        Some(NodeKey::new(doc.token().clone(), dom))
    }

    /// Iterates over the immediate children of `id` in flat-tree preorder.
    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            nodes: &self.nodes,
            next: self.node(id).and_then(TreeNode::first_child),
        }
    }

    /// Returns an O(1) slice covering `id` and every descendant in its flat-tree preorder subtree.
    #[must_use]
    pub fn subtree(&self, id: NodeId) -> &[TreeNode] {
        let Some(node) = self.node(id) else {
            return &[];
        };
        self.nodes
            .get(node.subtree_range().as_usize_range())
            .unwrap_or(&[])
    }

    /// Iterates over the ancestors of `id` from `node.parent()` up to the main root.
    pub fn ancestors(&self, id: NodeId) -> Ancestors<'_> {
        Ancestors {
            nodes: &self.nodes,
            next: self.node(id).and_then(TreeNode::parent),
        }
    }

    /// Iterates over nodes directly owned by `doc` in flat-tree document order, jumping over nested
    /// stitched iframe subtrees in O(1) via each iframe's `subtree_end`.
    pub fn doc_nodes(&self, doc: DocId) -> DocNodes<'_> {
        let range = self
            .document(doc)
            .map_or_else(|| NodeRange::empty_at(NodeId::new(0)), TreeDoc::node_range);
        DocNodes {
            nodes: &self.nodes,
            cursor: range.start().index(),
            end: range.end().index(),
        }
    }

    /// Whether `ancestor` inclusively contains `node` within the same document.
    ///
    /// Containment never crosses iframe boundaries so scoped queries stay confined to one document.
    #[must_use]
    pub fn contains(&self, ancestor: &NodeKey, node: &NodeKey) -> bool {
        let (Some(anc), Some(target)) = (self.node_by_key(ancestor), self.node_by_key(node)) else {
            return false;
        };
        anc.doc() == target.doc() && anc.subtree_range().contains(target.id())
    }

    /// Whether `node` is a strict descendant of `ancestor` within the same document.
    #[must_use]
    pub fn is_strict_descendant(&self, ancestor: &NodeKey, node: &NodeKey) -> bool {
        let (Some(anc), Some(target)) = (self.node_by_key(ancestor), self.node_by_key(node)) else {
            return false;
        };
        anc.doc() == target.doc() && anc.id() < target.id() && target.id() < anc.subtree_end()
    }

    /// Captured viewport rectangle, if reported on the top-level APC message.
    #[must_use]
    pub const fn viewport(&self) -> Option<Viewport> {
        self.viewport
    }

    /// Validated BrowserOS APC contract version accepted when building this tree.
    #[must_use]
    pub const fn contract_version(&self) -> u32 {
        self.contract_version
    }

    /// Configured subframe capture timeout budget in milliseconds, if reported by the browser.
    #[must_use]
    pub const fn subframe_timeout_ms(&self) -> Option<u32> {
        self.subframe_timeout_ms
    }
}

impl fmt::Debug for SnapshotTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnapshotTree")
            .field("node_count", &self.nodes.len())
            .field("doc_count", &self.docs.len())
            .field("partial_frame_count", &self.partial_frames.len())
            .field("indexed_key_count", &self.by_key.len())
            .field("indexed_doc_count", &self.doc_by_token.len())
            .field("viewport", &self.viewport)
            .field("contract_version", &self.contract_version)
            .field("subframe_timeout_ms", &self.subframe_timeout_ms)
            .finish()
    }
}

/// Iterative walker over a node's direct children in flat-tree preorder.
#[must_use]
#[derive(Clone)]
pub struct Children<'a> {
    nodes: &'a [TreeNode],
    next: Option<NodeId>,
}

impl<'a> Iterator for Children<'a> {
    type Item = &'a TreeNode;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.next?;
        let node = self.nodes.get(current.index())?;
        self.next = node.next_sibling();
        Some(node)
    }
}

impl FusedIterator for Children<'_> {}

impl fmt::Debug for Children<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Children")
            .field("next", &self.next)
            .finish()
    }
}

/// Iterative walker over a node's ancestors from its parent up to the main root.
#[must_use]
#[derive(Clone)]
pub struct Ancestors<'a> {
    nodes: &'a [TreeNode],
    next: Option<NodeId>,
}

impl<'a> Iterator for Ancestors<'a> {
    type Item = &'a TreeNode;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.next?;
        let node = self.nodes.get(current.index())?;
        self.next = node.parent();
        Some(node)
    }
}

impl FusedIterator for Ancestors<'_> {}

impl fmt::Debug for Ancestors<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ancestors")
            .field("next", &self.next)
            .finish()
    }
}

/// Iterative walker over nodes directly owned by a single [`TreeDoc`], skipping nested stitched
/// child documents in O(1) per iframe boundary.
#[must_use]
#[derive(Clone)]
pub struct DocNodes<'a> {
    nodes: &'a [TreeNode],
    cursor: usize,
    end: usize,
}

impl<'a> Iterator for DocNodes<'a> {
    type Item = &'a TreeNode;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor >= self.end {
            return None;
        }
        let node = self.nodes.get(self.cursor)?;
        self.cursor = if node.child_doc().is_some() {
            node.subtree_end().index()
        } else {
            self.cursor.saturating_add(1)
        };
        Some(node)
    }
}

impl FusedIterator for DocNodes<'_> {}

impl fmt::Debug for DocNodes<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocNodes")
            .field("cursor", &self.cursor)
            .field("end", &self.end)
            .finish()
    }
}

struct ContractMeta {
    version: Option<u32>,
    subframe_timeout_ms: Option<u32>,
}

fn read_contract_meta(apc: &AnnotatedPageContent) -> ContractMeta {
    match apc.browseros {
        Some(data) => ContractMeta {
            version: data.contract_version,
            subframe_timeout_ms: data.subframe_timeout_ms,
        },
        None => ContractMeta {
            version: None,
            subframe_timeout_ms: None,
        },
    }
}

pub(crate) fn check_contract_version(found: Option<u32>, required: u32) -> Result<u32, BuildError> {
    match found {
        Some(version) if version >= required => Ok(version),
        _ => Err(BuildError::Unpatched { found, required }),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use crate::proto::{REASON_BROWSEROS_TIMEOUT, RedactedFrameReason};

    use super::{BuildOptions, MIN_CONTRACT_VERSION};
    use crate::tree::{BuildError, PartialFrameReason};

    #[test]
    fn contract_version_gate_refuses_missing_zero_and_lower_versions() -> Result<(), Box<dyn Error>>
    {
        assert_eq!(
            super::check_contract_version(None, MIN_CONTRACT_VERSION),
            Err(BuildError::Unpatched {
                found: None,
                required: MIN_CONTRACT_VERSION,
            }),
            "stock Chromium payloads with no contract version must be refused"
        );
        assert_eq!(
            super::check_contract_version(Some(0), MIN_CONTRACT_VERSION),
            Err(BuildError::Unpatched {
                found: Some(0),
                required: MIN_CONTRACT_VERSION,
            }),
            "contract version 0 must be refused below MIN_CONTRACT_VERSION"
        );
        assert_eq!(
            super::check_contract_version(Some(1), 2),
            Err(BuildError::Unpatched {
                found: Some(1),
                required: 2,
            }),
            "versions below a raised floor must be refused"
        );
        assert_eq!(
            super::check_contract_version(Some(1), 1)?,
            1,
            "exact floor match must be accepted"
        );
        assert_eq!(
            super::check_contract_version(Some(3), 2)?,
            3,
            "versions above the floor must be accepted"
        );
        Ok(())
    }

    #[test]
    fn build_options_never_lowers_contract_floor_below_minimum() {
        let clamped = BuildOptions::default().with_min_contract_version(0);
        assert_eq!(
            clamped.min_contract_version(),
            MIN_CONTRACT_VERSION,
            "with_min_contract_version(0) must clamp up to MIN_CONTRACT_VERSION"
        );

        let raised = BuildOptions::default().with_min_contract_version(4);
        assert_eq!(
            raised.min_contract_version(),
            4,
            "callers must still be able to raise the contract version floor"
        );
    }

    #[test]
    fn redacted_reason_mapping_formats_timeout_with_and_without_budget() {
        let with_budget =
            PartialFrameReason::from_redacted_reason(Some(REASON_BROWSEROS_TIMEOUT), Some(2_000));
        assert_eq!(
            with_budget,
            PartialFrameReason::Timeout {
                timeout_ms: Some(2_000),
            },
            "REASON_BROWSEROS_TIMEOUT must map to Timeout carrying the page's timeout budget"
        );
        assert_eq!(
            with_budget.to_string(),
            "timeout after 2000 ms",
            "Timeout with budget must render exact millisecond phrasing"
        );

        let without_budget =
            PartialFrameReason::from_redacted_reason(Some(REASON_BROWSEROS_TIMEOUT), None);
        assert_eq!(
            without_budget.to_string(),
            "timeout",
            "Timeout without budget must render plain timeout phrasing"
        );

        assert_eq!(
            PartialFrameReason::from_redacted_reason(
                Some(i32::from(RedactedFrameReason::CrossSite)),
                Some(2_000),
            ),
            PartialFrameReason::CrossSite,
            "CrossSite enum code must map to PartialFrameReason::CrossSite"
        );
        assert_eq!(
            PartialFrameReason::from_redacted_reason(
                Some(i32::from(RedactedFrameReason::CrossOrigin)),
                None,
            ),
            PartialFrameReason::CrossOrigin,
            "CrossOrigin enum code must map to PartialFrameReason::CrossOrigin"
        );
        assert_eq!(
            PartialFrameReason::from_redacted_reason(Some(77), None),
            PartialFrameReason::Redacted(77),
            "unrecognized open-enum reason codes must preserve their numeric code"
        );
        assert_eq!(
            PartialFrameReason::from_redacted_reason(None, None),
            PartialFrameReason::Redacted(0),
            "absent reason code must fall back to Redacted(0)"
        );
    }
}
