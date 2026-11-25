//! Normalized, immutable multi-frame [`SnapshotTree`] built from Chromium's `AnnotatedPageContent`.
//!
//! This subsystem turns the nested, frame-heterogeneous protobuf tree into a flat-tree preorder
//! arena where every node belongs to one validated document, carries a stable composite
//! [`NodeKey`] (`(DocToken, DomNodeId)`), and precomputes its [`Hidden`], [`Role`], and [`State`]:
//!
//! ```text
//!  AnnotatedPageContent ─▶ contract check ─▶ frame_stitch ─▶ hidden ─▶ roles ─▶ states ─▶ SnapshotTree
//!                                                │                                          ├─ nodes: Vec<TreeNode>
//!                                                ├─ populated frame ──▶ TreeDoc             ├─ docs: Vec<TreeDoc>
//!                                                └─ omitted/redacted ─▶ PartialFrame        └─ by_key / doc_by_token
//! ```
//!
//! Key invariants enforced here:
//!
//! - **Zero recursion:** [`frame_stitch`], [`hidden`], [`role`], and [`state`] run iteratively so
//!   arbitrarily deep DOM and iframe trees cannot overflow the call stack.
//! - **Document-scoped identity:** raw `DomNodeId` values collide across iframes, so external
//!   lookups, inheritance walks, and containment checks always stay scoped to one document.
//! - **Privacy-safe diagnostics:** [`SnapshotTree`], [`TreeDoc`], [`TreeNode`], [`Hidden`],
//!   [`Role`], and [`State`] print only structural IDs, tokens, closed enums, booleans, and counts.

#[cfg(any(test, feature = "test-support"))]
mod apc_builder;
mod build_error;
mod frame_stitch;
mod hidden;
mod node_key;
mod role;
mod snapshot_tree;
mod state;
mod tree_doc;
mod tree_node;

#[cfg(any(test, feature = "test-support"))]
pub use apc_builder::{ApcBuilder, FrameBuilder, NodeBuilder, PartialIframeSpec};
pub use build_error::BuildError;
pub use hidden::Hidden;
pub use node_key::{DocId, DocSeq, DocToken, DomNodeId, FrameToken, NodeId, NodeKey, NodeRange};
pub use role::Role;
pub use snapshot_tree::{
    Ancestors, BuildOptions, Children, DocNodes, MIN_CONTRACT_VERSION, SnapshotTree, Viewport,
};
pub use state::{InvalidState, State, TriState};
pub use tree_doc::{DocOrigin, PartialFrame, PartialFrameReason, TreeDoc};
pub use tree_node::TreeNode;
