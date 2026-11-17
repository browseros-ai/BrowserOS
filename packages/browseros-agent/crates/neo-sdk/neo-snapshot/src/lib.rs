//! A page's aria snapshot as a typed tree.
//!
//! Its place in a snapshot's path is tree in, text out:
//!
//! ```text
//!  a producer of the shape ─▶ AriaNode tree ─▶ the text
//! ```
//!
//! The tree ([`AriaNode`]) is Playwright's `AriaNodeJSON` at 1.64.1 plus Neo's two marks, `inert`
//! and `onTop`. Today the page's walk writes it, and neo-playwright reads it from each frame's
//! reply, stitches the frames and serves the result. Any producer that builds the same shape can
//! feed this crate, which takes the tree as data and never asks where it came from.
//!
//! Pure: no clock, no store, no browser and no async. The dependencies are serde and `serde_json`,
//! so the build refuses printing that waits or sends.

mod aria_node;

pub use aria_node::{AriaChild, AriaNode, Invalid, NodeBox, Tristate};
