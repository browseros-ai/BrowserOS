//! A page's aria snapshot as a typed tree, and every text a script receives for it, printed with
//! JavaScript's semantics.
//!
//! Its place in a snapshot's path is tree in, text out, through one interface:
//!
//! ```text
//!  neo-playwright (effects)                              this crate (pure)
//!  frames read, stitched ─▶ Snapshot ─┐
//!  the script's options  ─▶ Call ─────┴─▶ render(&snapshot, call) ─▶ Answer ─▶ text()
//!  the tab's pending baseline ◀────────────────────────────── pending() ◀──┘
//!
//!  render: Call::Playwright ─▶ playwright: Playwright's own text
//!          Call::Full ───────▶ full_text: Playwright's lines under Neo's heads, cut
//!          Call::Element ────▶ neo: Neo's default format
//!          Call::Page ───────▶ neo: the same, kept in its Form
//!          Call::Delta ──────▶ delta: what changed since the tab's stored text, from the
//!                              page rendered in that text's Form, kept in it
//!  cover_block: a covered click's lines, by neo's format
//!                                 │ every element line by playwright's printer; the size cap by
//!                                 ▼ cap; every length, space, number and quote by
//!                       js: JavaScript's rules
//! ```
//!
//! [`render`] is the only way to a snapshot's text, and only it builds an [`Answer`]: so the
//! snapshot operation can keep for the tab only a text this crate printed, in the [`Form`] it was
//! printed in. Every walk, printer and line is private; the tests judge the crate through
//! `render`, except two judges whose oracle has a finer grain: upstream's printer one node at a
//! time, and `js` one rule at a time.
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
//! | `full_text.rs` | neo-playwright-js `prelude/snapshot.ts` (`renderFullSnapshot`, `printed`), `snapshot-inert.ts` |
//! | `neo/*.rs` | neo-playwright-js `prelude/snapshot-v1.ts`, `snapshot-on-top.ts`, `snapshot-unmarked.ts` |
//! | `cap.rs` | neo-playwright-js `prelude/snapshot-cap.ts` |
//! | `render.rs`, `call.rs`, `answer.rs` | `Frame._ariaSnapshot` in neo-playwright-js `prelude/frame.ts`: which text a call gets, and what it keeps |
//! | `cover_block.rs` | neo-playwright-js `scripts/snapshot-cases/snapshot-lines.ts` (`snapshotLines`, the prelude's `cover.ts` until Rust rendered the block) |
//! | `delta/*.rs` | neo-playwright-js `prelude/snapshot-delta.ts`, and `onTopDelta` in `neo/on_top.rs` |
//! | `js/*.rs` | ECMAScript itself; QuickJS is the test oracle |
//!
//! [`PORTED_FROM`] holds the vendored files' hashes at the port, which neo-playwright's tests
//! compare with the vendored tree at every engine upgrade.
//!
//! Pure: no clock, no store, no browser and no async. The dependencies are serde and `serde_json`,
//! so the build refuses printing that waits or sends.

mod answer;
mod aria_node;
mod call;
mod cap;
mod cover_block;
mod delta;
mod form;
mod full_text;
mod js;
#[cfg(test)]
mod judges;
mod neo;
mod playwright;
mod render;
mod snapshot;
#[cfg(test)]
mod split_mix64;

pub use answer::{Answer, Pending};
pub use aria_node::{AriaChild, AriaNode, Invalid, MAX_TREE_DEPTH, NodeBox, Tristate};
pub use call::Call;
pub use cap::{DEFAULT_MAX_TOKENS, MaxTokens};
pub use cover_block::cover_block;
pub use delta::{Baseline, Earlier};
pub use form::{Form, Passed, TextForm};
pub use neo::{KEPT_ROLES, TEXT_LIMIT_TOKENS};
pub use playwright::{PORTED_FROM, PortedFile};
pub use render::render;
pub use snapshot::{ClosedRoots, Layer, PartialFrame, Snapshot, Unmarked};
