//! A page's aria snapshot as a typed tree, and the JavaScript semantics its text is printed with.
//!
//! Its place in a snapshot's path is tree in, text out:
//!
//! ```text
//!  a producer of the shape ─▶ AriaNode tree ─▶ printed by JavaScript's rules (js) ─▶ the text
//! ```
//!
//! The tree ([`AriaNode`]) is Playwright's `AriaNodeJSON` at 1.64.1 plus Neo's two marks, `inert`
//! and `onTop`. Today the page's walk writes it, and neo-playwright reads it from each frame's
//! reply, stitches the frames and serves the result. Any producer that builds the same shape can
//! feed this crate, which takes the tree as data and never asks where it came from.
//!
//! The text must be, byte for byte, what the TypeScript renderer prints in the script engine.
//! Where JavaScript's own rules decide a byte (UTF-16 lengths, its whitespace set,
//! `Number(string)`, number printing, `JSON.stringify`, `Math.round`), the printing calls `js`,
//! never Rust's own string or float behaviour, and `js`'s tests hold each rule against QuickJS,
//! that engine.
//!
//! Pure: no clock, no store, no browser and no async. The dependencies are serde and `serde_json`,
//! so the build refuses printing that waits or sends.

mod aria_node;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        unused_imports,
        reason = "the renderers that print a snapshot's text call these and are not in this \
                  crate yet; until they are, only the QuickJS tests do"
    )
)]
mod js;

pub use aria_node::{AriaChild, AriaNode, Invalid, NodeBox, Tristate};
