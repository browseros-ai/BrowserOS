//! Shared frame- and node-counting helpers for `neo-snapshot-apc` tests and dev tools.

use neo_snapshot_apc::AnnotatedPageContent;
use neo_snapshot_apc::proto::{ContentData, ContentNode};

/// Frame and content-node totals for a decoded [`AnnotatedPageContent`] tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TreeCounts {
    /// Main frame (when `main_frame_data` is set) plus every embedded `Iframe` node.
    pub(crate) frames: usize,
    /// Total [`ContentNode`] entries across `root_node`.
    pub(crate) nodes: usize,
}

/// Counts frames and nodes in `apc` iteratively so deeply nested trees never risk stack overflow.
#[must_use]
pub(crate) fn count_frames_and_nodes(apc: &AnnotatedPageContent) -> TreeCounts {
    let mut frames = usize::from(apc.main_frame_data.is_some());
    let mut nodes = 0;
    let mut stack: Vec<&ContentNode> = apc.root_node.iter().collect();

    while let Some(node) = stack.pop() {
        nodes += 1;
        if let Some(attrs) = node.content_attributes.as_ref()
            && matches!(attrs.content_data, Some(ContentData::Iframe(_)))
        {
            frames += 1;
        }
        stack.extend(node.children_nodes.iter().rev());
    }

    TreeCounts { frames, nodes }
}
