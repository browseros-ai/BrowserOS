use std::fmt;
use std::num::NonZeroU32;

use crate::proto::ContentAttributes;

use super::hidden::Hidden;
use super::mask::NodeMask;
use super::node_key::{DocId, DomNodeId, NodeId, NodeRange};
use super::role::Role;
use super::state::State;

/// Where an arena node comes from, and so how the page reaches it.
///
/// A node APC sent carries its own DOM id. An option of a collapsed `<select>` has none: APC sends
/// it as data on its select (`FormControlData.select_options`), and the page reaches it as
/// `select.options[index]` ([`OptionSlot`](super::OptionSlot)). The enum keeps a DOM id off such a
/// node, so nothing can resolve it as if APC had sent it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeOrigin {
    /// A node APC sent; `None` for one with no DOM node, such as CSS-generated text.
    Apc(Option<DomNodeId>),
    /// The `index`th entry of the parent `<select>`'s `select_options`, appended by frame
    /// stitching as one of the select's last children.
    SelectOption { index: u32 },
    /// A select option whose entry the mask removed with its select's whole list. It stands for
    /// nothing, so no pass, matcher, emitter or join may reveal that it exists.
    WithheldOption,
}

pub(crate) struct TreeNodeInit {
    pub(crate) id: NodeId,
    pub(crate) doc: DocId,
    pub(crate) origin: NodeOrigin,
    pub(crate) parent: Option<NodeId>,
    pub(crate) child_doc: Option<DocId>,
    pub(crate) partial_frame: Option<usize>,
    pub(crate) raw: Option<ContentAttributes>,
}

/// Single node stored in a [`SnapshotTree`](super::SnapshotTree)'s flat-tree preorder arena.
///
/// Owns its parent/child/sibling arena links, non-empty preorder subtree extent, iframe boundary
/// link, precomputed [`Hidden`], [`Role`], and [`State`], and internal raw `ContentAttributes`
/// (with `ContentData::Iframe` already extracted). Its manual `Debug` implementation prints only
/// structural IDs, closed enums, and booleans so debug logs never expose page text, form values,
/// labels, or URLs.
pub struct TreeNode {
    id: NodeId,
    doc: DocId,
    origin: NodeOrigin,
    parent: Option<NodeId>,
    first_child: Option<NodeId>,
    last_child: Option<NodeId>,
    next_sibling: Option<NodeId>,
    subtree_len: NonZeroU32,
    child_doc: Option<DocId>,
    partial_frame: Option<usize>,
    hidden: Hidden,
    role: Option<Role>,
    state: State,
    mask: NodeMask,
    raw: Option<ContentAttributes>,
}

impl TreeNode {
    pub(crate) fn new(init: TreeNodeInit) -> Self {
        Self {
            id: init.id,
            doc: init.doc,
            origin: init.origin,
            parent: init.parent,
            first_child: None,
            last_child: None,
            next_sibling: None,
            subtree_len: NonZeroU32::MIN,
            child_doc: init.child_doc,
            partial_frame: init.partial_frame,
            hidden: Hidden::default(),
            role: None,
            state: State::default(),
            mask: NodeMask::Clear,
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

    /// Renderer-assigned DOM node ID within [`Self::doc`], or `None` for a node APC sent with no
    /// DOM node (CSS-generated text) and for an option of a collapsed `<select>`, which the page
    /// reaches through its select ([`SnapshotTree::option_slot`](super::SnapshotTree::option_slot)).
    #[must_use]
    pub const fn dom_node_id(&self) -> Option<DomNodeId> {
        match self.origin {
            NodeOrigin::Apc(dom) => dom,
            NodeOrigin::SelectOption { .. } | NodeOrigin::WithheldOption => None,
        }
    }

    /// The entry of its select's `select_options` this node stands for, when it is a live option
    /// slot.
    pub(crate) const fn option_index(&self) -> Option<u32> {
        match self.origin {
            NodeOrigin::SelectOption { index } => Some(index),
            NodeOrigin::Apc(_) | NodeOrigin::WithheldOption => None,
        }
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

    /// Precomputed ARIA visibility, box visibility, and folding inputs for this node.
    #[must_use]
    pub const fn hidden(&self) -> Hidden {
        self.hidden
    }

    /// Precomputed WAI-ARIA role for this node, or `None` for unroled, text, or `ua_shadow` nodes.
    #[must_use]
    pub const fn role(&self) -> Option<Role> {
        self.role
    }

    /// Precomputed WAI-ARIA states, focus flag, and inert flag for this node.
    #[must_use]
    pub const fn state(&self) -> State {
        self.state
    }

    /// Precomputed privacy mask outcome for this node (Chromium redaction or host-private mark).
    #[must_use]
    pub const fn mask(&self) -> NodeMask {
        self.mask
    }

    /// Whether this element is the first visible element inside a closed shadow root the page
    /// made (contract field 31, retention rule 4). The page's own CSS and text engines cannot see
    /// into such a root; handed this element, the probe opens the root to them.
    #[must_use]
    pub fn is_closed_root_seed(&self) -> bool {
        self.raw
            .as_ref()
            .and_then(|raw| raw.browseros.as_deref())
            .and_then(|facts| facts.closed_root_seed)
            == Some(true)
    }

    pub(crate) const fn raw_attributes(&self) -> Option<&ContentAttributes> {
        self.raw.as_ref()
    }

    pub(crate) fn raw_attributes_mut(&mut self) -> Option<&mut ContentAttributes> {
        self.raw.as_mut()
    }

    /// Turns an option slot whose entry the mask removed into a node that stands for nothing: no
    /// facts, no role, default state, hidden for ARIA. It keeps its arena place, since every
    /// range and link around it is already final.
    pub(crate) fn withhold(&mut self) {
        self.origin = NodeOrigin::WithheldOption;
        self.raw = None;
        self.role = None;
        self.state = State::default();
        self.hidden = Hidden::WITHHELD;
        self.mask = NodeMask::Clear;
    }

    pub(crate) fn set_hidden(&mut self, hidden: Hidden) {
        self.hidden = hidden;
    }

    pub(crate) fn set_role(&mut self, role: Option<Role>) {
        self.role = role;
    }

    pub(crate) fn set_state(&mut self, state: State) {
        self.state = state;
    }

    pub(crate) fn set_mask(&mut self, mask: NodeMask) {
        self.mask = mask;
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
            .field("origin", &self.origin)
            .field("parent", &self.parent)
            .field("first_child", &self.first_child)
            .field("last_child", &self.last_child)
            .field("next_sibling", &self.next_sibling)
            .field("subtree_len", &self.subtree_len)
            .field("child_doc", &self.child_doc)
            .field("partial_frame", &self.partial_frame)
            .field("role", &self.role)
            .field("hidden", &self.hidden)
            .field("state", &self.state)
            .field("mask", &self.mask)
            .field("has_raw_attributes", &self.raw.is_some())
            .finish()
    }
}
