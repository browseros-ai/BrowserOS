use std::fmt;
use std::num::NonZeroU32;

use crate::proto::ContentAttributes;

use super::node_key::{DocId, DomNodeId, NodeId, NodeRange};

pub(crate) struct TreeNodeInit {
    pub(crate) id: NodeId,
    pub(crate) doc: DocId,
    pub(crate) dom_node_id: Option<DomNodeId>,
    pub(crate) parent: Option<NodeId>,
    pub(crate) child_doc: Option<DocId>,
    pub(crate) partial_frame: Option<usize>,
    pub(crate) raw: Option<ContentAttributes>,
}

/// Single node stored in a [`SnapshotTree`](super::SnapshotTree)'s flat-tree preorder arena.
///
/// Owns its parent/child/sibling arena links, non-empty preorder subtree extent, iframe boundary
/// link, and internal raw `ContentAttributes` (with `ContentData::Iframe` already extracted). Its
/// manual `Debug` implementation prints only structural IDs and booleans so debug logs never expose
/// page text, form values, labels, or URLs.
pub struct TreeNode {
    id: NodeId,
    doc: DocId,
    dom_node_id: Option<DomNodeId>,
    parent: Option<NodeId>,
    first_child: Option<NodeId>,
    last_child: Option<NodeId>,
    next_sibling: Option<NodeId>,
    subtree_len: NonZeroU32,
    child_doc: Option<DocId>,
    partial_frame: Option<usize>,
    raw: Option<ContentAttributes>,
}

impl TreeNode {
    pub(crate) fn new(init: TreeNodeInit) -> Self {
        Self {
            id: init.id,
            doc: init.doc,
            dom_node_id: init.dom_node_id,
            parent: init.parent,
            first_child: None,
            last_child: None,
            next_sibling: None,
            subtree_len: NonZeroU32::MIN,
            child_doc: init.child_doc,
            partial_frame: init.partial_frame,
            raw: init.raw,
        }
    }

    /// Arena index of this node in [`SnapshotTree::nodes`](super::SnapshotTree::nodes).
    #[must_use]
    pub const fn id(&self) -> NodeId {
        self.id
    }

    /// Owning document of this node.
    #[must_use]
    pub const fn doc(&self) -> DocId {
        self.doc
    }

    /// Renderer-assigned DOM node ID within [`Self::doc`], or `None` for synthetic layout nodes.
    #[must_use]
    pub const fn dom_node_id(&self) -> Option<DomNodeId> {
        self.dom_node_id
    }

    /// Parent node in the stitched flat tree, or `None` for the top-level root node.
    #[must_use]
    pub const fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    /// First child in flat-tree preorder, if any.
    #[must_use]
    pub const fn first_child(&self) -> Option<NodeId> {
        self.first_child
    }

    /// Last child in flat-tree preorder, if any.
    #[must_use]
    pub const fn last_child(&self) -> Option<NodeId> {
        self.last_child
    }

    /// Next sibling under [`Self::parent`], if any.
    #[must_use]
    pub const fn next_sibling(&self) -> Option<NodeId> {
        self.next_sibling
    }

    /// Exclusive end index of this node's flat-tree preorder subtree (`id < subtree_end`).
    #[must_use]
    pub const fn subtree_end(&self) -> NodeId {
        NodeId::new(self.id.raw() + self.subtree_len.get())
    }

    /// Half-open arena range `[id, subtree_end)` covering this node and all of its descendants.
    #[must_use]
    pub const fn subtree_range(&self) -> NodeRange {
        NodeRange::from_start_and_len(self.id, self.subtree_len)
    }

    /// Stitched child document when this node is a populated `<iframe>` boundary.
    #[must_use]
    pub const fn child_doc(&self) -> Option<DocId> {
        self.child_doc
    }

    /// Index into [`SnapshotTree::partial_frames`](super::SnapshotTree::partial_frames) when this
    /// node is an omitted, redacted, or unidentifiable `<iframe>` boundary.
    #[must_use]
    pub const fn partial_frame_index(&self) -> Option<usize> {
        self.partial_frame
    }

    /// Whether this node marks an iframe boundary (either a stitched child doc or a partial frame).
    #[must_use]
    pub const fn is_iframe_boundary(&self) -> bool {
        self.child_doc.is_some() || self.partial_frame.is_some()
    }

    pub(crate) fn set_first_child(&mut self, child: NodeId) {
        self.first_child = Some(child);
    }

    pub(crate) fn set_last_child(&mut self, child: NodeId) {
        self.last_child = Some(child);
    }

    pub(crate) fn set_next_sibling(&mut self, sibling: NodeId) {
        self.next_sibling = Some(sibling);
    }

    pub(crate) fn set_subtree_end(&mut self, end: NodeId) {
        if let Some(len) = end
            .raw()
            .checked_sub(self.id.raw())
            .and_then(NonZeroU32::new)
        {
            self.subtree_len = len;
        }
    }
}

impl fmt::Debug for TreeNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeNode")
            .field("id", &self.id)
            .field("doc", &self.doc)
            .field("dom_node_id", &self.dom_node_id)
            .field("parent", &self.parent)
            .field("first_child", &self.first_child)
            .field("last_child", &self.last_child)
            .field("next_sibling", &self.next_sibling)
            .field("subtree_len", &self.subtree_len)
            .field("child_doc", &self.child_doc)
            .field("partial_frame", &self.partial_frame)
            .field("has_raw_attributes", &self.raw.is_some())
            .finish()
    }
}
