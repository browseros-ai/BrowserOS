//! Normalized, immutable multi-frame [`SnapshotTree`] built from Chromium's `AnnotatedPageContent`.
//!
//! This subsystem turns the nested, frame-heterogeneous protobuf tree into a flat-tree preorder
//! arena where every node belongs to one validated document and carries a stable composite
//! [`NodeKey`] (`(DocToken, DomNodeId)`):
//!
//! ```text
//!  AnnotatedPageContent ─▶ contract check ─▶ frame_stitch (iterative Enter/Exit) ─▶ SnapshotTree
//!                                                │                                    ├─ nodes: Vec<TreeNode>
//!                                                ├─ populated frame ──▶ TreeDoc       ├─ docs: Vec<TreeDoc>
//!                                                └─ omitted/redacted ─▶ PartialFrame  └─ by_key / doc_by_token
//! ```
//!
//! Key invariants enforced here:
//!
//! - **Zero recursion:** [`frame_stitch`] walks `ContentNode` with an explicit heap stack so
//!   arbitrarily deep DOM and iframe trees cannot overflow the call stack.
//! - **Document-scoped identity:** raw `DomNodeId` values collide across iframes, so external
//!   lookups and containment checks always use `(DocToken, DomNodeId)` via [`NodeKey`].
//! - **Privacy-safe diagnostics:** [`SnapshotTree`], [`TreeDoc`], and [`TreeNode`] implement
//!   redacting `Debug` formatting that prints only structural IDs, tokens, booleans, and counts.

#[cfg(any(test, feature = "test-support"))]
mod apc_builder;
mod build_error;
mod frame_stitch;
mod node_key;
mod snapshot_tree;
mod tree_doc;
mod tree_node;

#[cfg(any(test, feature = "test-support"))]
pub use apc_builder::{ApcBuilder, FrameBuilder, NodeBuilder, PartialIframeSpec};
pub use build_error::BuildError;
pub use node_key::{DocId, DocSeq, DocToken, DomNodeId, FrameToken, NodeId, NodeKey, NodeRange};
pub use snapshot_tree::{
    Ancestors, BuildOptions, Children, DocNodes, MIN_CONTRACT_VERSION, SnapshotTree, Viewport,
};
pub use tree_doc::{DocOrigin, PartialFrame, PartialFrameReason, TreeDoc};
pub use tree_node::TreeNode;
