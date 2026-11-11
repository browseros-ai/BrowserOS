//! From a locator to the world its element lives in, nested frames included.
//!
//! ```text
//!  chunks[0]              chunks[1]               chunks[2] (last)
//!  #pay ─ hop ─▶ iframe ─▶ iframe ─ hop ─▶ iframe ─▶ getByRole('button') ─ the op's request
//!   main world             child world (seq 4)     grandchild world (seq 9)
//!
//!  hop = resolve { want: frame } → pin ─▶ pinned (by handle) ─▶ DOM.describeNode
//!        → child frame id + cross-process? ─▶ core's FrameRegistry::resolve_frame_target
//!        → FrameWorlds::world, recording the child's ParentLink
//!
//!  aria-ref=f9e3 leading a chunk: jump to document 9 (FrameWorlds::by_seq) with the parent
//!  links recorded on the hops into it; retired is element_detached, unknown invalid_input.
//!
//!  element handle { seq, node }: document seq (FrameWorlds::by_seq) ─▶ DOM.resolveNode in
//!  its world ─▶ the request, sent to the world's global with the node as its root
//!
//!  frame.locator(..) { frame }: the page's frame tree ─▶ main, f1, .., frame; each level a
//!  hop through the iframe DOM.getFrameOwner names in its parent (frame_start)
//! ```
//!
//! The resolver never waits. "Not there yet" is a value, and each op's loop owns the timing
//! on Rust's clock (`OpScope::sleep`), never the page's timers.

mod frame_path;
mod frame_start;
mod handle_site;
mod resolver;

pub(crate) use frame_path::{FramePath, Hop, HopIframe};
pub(crate) use frame_start::{FrameStart, Started};
pub(crate) use handle_site::{HandleSite, handle_detached};
pub(crate) use resolver::{
    Hopped, Landed, Resolved, Resolver, ask, failed, hop, hop_pinned, refused,
};
