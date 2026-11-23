use std::collections::HashSet;
use std::ptr;

use crate::aria_node::{AriaChild, AriaNode};
use crate::snapshot::Snapshot;

use super::rule::Child;

/// What the whole-view layers hold (`onTopContent` in `snapshot-on-top.ts`): Neo's default
/// format prints it first, then the rest of the page without it.
///
/// The TypeScript projects a copy of the tree with those nodes removed and finds them again by
/// object identity. Here the walk borrows the tree as it is and leaves out the held nodes as it
/// meets them, by address: every child list the walk, the omissions and the label rules read goes
/// through [`OnTop::children`], so each sees the children the projection would hold. The search
/// never enters a held node or an inert one, so no held node lies inside either, and a printer
/// walking inside them sees the same children with or without the filter.
#[derive(Debug, Default)]
pub(super) struct OnTop<'a> {
    /// Each whole-view layer's block in layer order: the nodes marked `onTop` with its ref. Two
    /// layers with one ref each print that block, as the TypeScript's map does.
    pub(super) first: Vec<&'a AriaNode>,
    /// The same nodes, by address in the borrowed tree.
    held: HashSet<*const AriaNode>,
}

impl<'a> OnTop<'a> {
    pub(super) fn of(snapshot: &'a Snapshot) -> Self {
        let layers: Vec<&str> = snapshot
            .layers
            .iter()
            .filter(|layer| layer.whole_view)
            .map(|layer| layer.r#ref.as_str())
            .collect();
        if layers.is_empty() {
            return Self::default();
        }
        // One block per distinct ref, as a map keyed by ref holds it.
        let mut blocks: Vec<(&str, Vec<&'a AriaNode>)> = Vec::new();
        for layer in &layers {
            if !blocks.iter().any(|(held_by, _)| held_by == layer) {
                blocks.push((layer, Vec::new()));
            }
        }
        let mut held = HashSet::new();
        // Depth first in document order, on a stack of its own.
        let mut pending: Vec<&'a AriaNode> = snapshot.tree.iter().rev().collect();
        while let Some(node) = pending.pop() {
            if node.inert == Some(true) {
                continue;
            }
            // `node.onTop ? front.get(node.onTop) : undefined`: an empty mark selects nothing.
            let block = node
                .on_top
                .as_deref()
                .filter(|mark| !mark.is_empty())
                .and_then(|mark| blocks.iter_mut().find(|(held_by, _)| *held_by == mark));
            if let Some((_, block)) = block {
                held.insert(ptr::from_ref(node));
                block.push(node);
            } else {
                pending.extend(node.children.iter().rev().filter_map(|child| match child {
                    AriaChild::Node(node) => Some(&**node),
                    AriaChild::Text(_) => None,
                }));
            }
        }
        let mut first = Vec::new();
        for layer in &layers {
            if let Some((_, block)) = blocks.iter().find(|(held_by, _)| held_by == layer) {
                first.extend(block.iter().copied());
            }
        }
        Self { first, held }
    }

    /// `node`'s children without the held nodes, in document order.
    pub(super) fn children(
        &self,
        node: &'a AriaNode,
    ) -> impl DoubleEndedIterator<Item = Child<'a>> + '_ {
        self.without_held(node.children.iter().map(Child::from))
    }

    /// The page's top-level nodes without the held nodes, in document order.
    pub(super) fn rest(
        &self,
        tree: &'a [AriaNode],
    ) -> impl DoubleEndedIterator<Item = Child<'a>> + '_ {
        self.without_held(tree.iter().map(Child::Node))
    }

    fn without_held(
        &self,
        children: impl DoubleEndedIterator<Item = Child<'a>>,
    ) -> impl DoubleEndedIterator<Item = Child<'a>> {
        children.filter(move |child| match child {
            Child::Node(node) => !self.held.contains(&ptr::from_ref(*node)),
            Child::Text(_) => true,
        })
    }
}

/// `onTopHead`: each layer's line, split at its line breaks, in layer order. Neo's formats (the
/// default format and `mode: 'full'`) print them before everything else and outside the cap.
pub(crate) fn on_top_head(snapshot: &Snapshot) -> impl Iterator<Item = &str> {
    snapshot
        .layers
        .iter()
        .flat_map(|layer| layer.line.split('\n'))
}
