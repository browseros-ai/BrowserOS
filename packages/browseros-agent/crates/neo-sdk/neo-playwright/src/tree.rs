//! Per-tab APC capture ownership, request-time coalescing, safe hydration, and host-private
//! node mirroring for `neo-playwright`.
//!
//! ```text
//!  attempt ─▶ TabTree ─▶ PageCapture (Page.getAnnotatedPageContent + contract gate + coalescing)
//!                ├─▶ FrameWorlds / DocTable (DocToken ⇄ FrameSeq + section 6.2 world proof)
//!                ├─▶ TabRefTable (neo_snapshot_apc::refs: NodeKey ⇄ Ref across captures)
//!                ├─▶ PrivateMirror (host-typed secret marks by DomNodeId and site, copied into
//!                │   every build as neo_snapshot_apc's PrivateNodes)
//!                ├─▶ Hydrate (DOM.resolveNode + probe pin under a proven pre-capture world)
//!                └─▶ ChunkWorld (a chunk run's page side: its tree nodes handed to the frame's
//!                    world under the same proof, and the tab's refs as its RefResolver)
//! ```

mod chunk_world;
mod hydrate;
#[cfg(test)]
mod live_tests;
mod page_capture;
mod private_mirror;
mod tab_tree;
#[cfg(test)]
mod tree_script;

pub(crate) use chunk_world::{ChunkWorld, RunAttempt, RunRoot, StartedRun, Unproven};
pub(crate) use hydrate::{Hydrate, RefLanding, ensure_frame_world, frame_doc, open_closed_roots};
#[cfg(test)]
pub(crate) use hydrate::{HydrateOutcome, HydratedNode};
pub(crate) use page_capture::{AttemptStart, CaptureError, CapturedTree, PageCapture};
pub(crate) use private_mirror::{
    PrivateMirror, TabOriginState, backend_to_dom_node_id, describe_object_node_id,
};
pub(crate) use tab_tree::{TabTree, TreeAsk};
