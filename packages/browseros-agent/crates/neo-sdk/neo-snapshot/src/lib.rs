//! A page's aria snapshot as a typed tree, and the text a script receives for it, printed with
//! JavaScript's semantics.
//!
//! Its place in a snapshot's path is tree in, text out:
//!
//! ```text
//!  a producer of the shape ─▶ Snapshot: the AriaNode tree, the frames not read
//!                                 │
//!                                 ▼
//!                       playwright: Playwright's own text ─▶ the text
//!                                 │ every length, space, number and quote by
//!                                 ▼
//!                       js: JavaScript's rules
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
//! One rule has one home, and a diff on either source maps onto one file here:
//!
//! | Here | Ported from |
//! |---|---|
//! | `aria_node.rs` | vendored `isomorphic/ariaSnapshot.ts` (the shape) and `injected/src/ariaSnapshot.ts` (its writer) |
//! | `playwright/yaml_escape.rs` | vendored `isomorphic/yaml.ts` |
//! | `playwright/node_text.rs` | vendored `isomorphic/ariaSnapshotRenderer.ts` |
//! | `playwright/tree_text.rs` | neo-playwright-js `prelude/snapshot.ts` (`treeLines`, `yamlLines`, `renderSnapshot`) |
//! | `js/*.rs` | ECMAScript itself; QuickJS is the test oracle |
//!
//! [`PORTED_FROM`] holds the vendored files' hashes at the port, which neo-playwright's tests
//! compare with the vendored tree at every engine upgrade.
//!
//! Pure: no clock, no store, no browser and no async. The dependencies are serde and `serde_json`,
//! so the build refuses printing that waits or sends.

mod aria_node;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        unused_imports,
        reason = "only Playwright's printer calls these, and no production code calls it before \
                  the snapshot operation renders through this crate; until then the tests do"
    )
)]
mod js;
#[cfg(test)]
mod judges;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        unused_imports,
        reason = "no production code prints a snapshot's text through this crate before the \
                  snapshot operation calls it; until then the tests are its only callers"
    )
)]
mod playwright;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "a snapshot is built by the snapshot operation once it renders through this \
                  crate; until then only the tests build one"
    )
)]
mod snapshot;
#[cfg(test)]
mod split_mix64;

pub use aria_node::{AriaChild, AriaNode, Invalid, NodeBox, Tristate};
pub use playwright::{PORTED_FROM, PortedFile};
