use std::hash::Hash;
use std::ptr;

use crate::aria_node::{AriaChild, AriaNode};
use crate::aria_sink::EmittedNode;
use crate::snapshot::{ClosedRoots, Layer, PartialFrame, Snapshot, Unmarked};

/// One direct child yielded by an [`AriaSource`]: either a text run or a child node handle.
#[derive(Clone, Copy, Debug)]
pub enum SourceChild<'a, Id> {
    /// Text run between elements.
    Text(&'a str),
    /// Child node handle in the same [`AriaSource`].
    Node(Id),
}

/// Zero-allocation read-only view over a stitched ARIA tree.
///
/// Implemented for `&[AriaNode]` (used by [`render`](crate::render)) and for distilled draft
/// arenas (`neo-snapshot-apc`) so every text format (`playwright`, `full`, Neo default, and
/// `delta`) runs one shared printer without materializing `AriaNode`s.
pub trait AriaSource<'a> {
    /// Lightweight handle identifying one node in the tree.
    type Id: Copy;
    /// Stable identity key for set membership (such as whole-view on-top filtering).
    type Key: Copy + Eq + Hash;

    /// Returns the identity key for `id`.
    fn key(&self, id: Self::Id) -> Self::Key;

    /// Top-level nodes in document order.
    fn roots(&self) -> impl DoubleEndedIterator<Item = Self::Id> + '_;

    /// Borrowed properties of `id`.
    fn node(&self, id: Self::Id) -> EmittedNode<'a>;

    /// Direct children of `id` in document order.
    fn children(
        &self,
        id: Self::Id,
    ) -> impl DoubleEndedIterator<Item = SourceChild<'a, Self::Id>> + '_;
}

impl<'a> AriaSource<'a> for &'a [AriaNode] {
    type Id = &'a AriaNode;
    type Key = *const AriaNode;

    fn key(&self, id: Self::Id) -> Self::Key {
        ptr::from_ref(id)
    }

    fn roots(&self) -> impl DoubleEndedIterator<Item = Self::Id> + '_ {
        self.iter()
    }

    fn node(&self, id: Self::Id) -> EmittedNode<'a> {
        EmittedNode::from(id)
    }

    fn children(
        &self,
        id: Self::Id,
    ) -> impl DoubleEndedIterator<Item = SourceChild<'a, Self::Id>> + '_ {
        id.children.iter().map(|child| match child {
            AriaChild::Text(text) => SourceChild::Text(text.as_str()),
            AriaChild::Node(node) => SourceChild::Node(node.as_ref()),
        })
    }
}

/// Borrowed snapshot facts paired with any [`AriaSource`] tree, accepted by
/// [`render_source`](crate::render_source).
#[derive(Debug)]
pub struct SnapshotView<'a, S> {
    /// Stitched tree view.
    pub tree: S,
    /// Frames that could not be read, in stitch order.
    pub partial: &'a [PartialFrame],
    /// Whole-view and partial layers covering the main page.
    pub layers: &'a [Layer],
    /// Closed shadow root notice, if not all seeds reached the page.
    pub closed_roots: Option<&'a ClosedRoots>,
    /// Unmarked clickable-check notice, if the root check did not finish.
    pub unmarked: Option<&'a Unmarked>,
}

impl<'a> SnapshotView<'a, &'a [AriaNode]> {
    /// Borrows an existing owned [`Snapshot`] as a [`SnapshotView`].
    #[must_use]
    pub fn from_snapshot(snapshot: &'a Snapshot) -> Self {
        Self {
            tree: snapshot.tree.as_slice(),
            partial: snapshot.partial.as_slice(),
            layers: snapshot.layers.as_slice(),
            closed_roots: snapshot.closed_roots.as_ref(),
            unmarked: snapshot.unmarked.as_ref(),
        }
    }
}
