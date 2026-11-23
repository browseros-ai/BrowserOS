//! Playwright 1.64.1's own text for a snapshot, in Rust: upstream's YAML escaping, its printer
//! for one node, and the whole tree with the frames that could not be read.
//!
//! ```text
//!  Snapshot ─▶ tree_text: walks the tree on its own stack, places each # partial: comment;
//!                 │ yaml_lines, with an iframe's fallback text dropped, feeds mode: 'full'
//!                 │ one node at a time, without its children (NodeAlone)
//!                 ▼
//!              node_text: upstream's createKey and visit ─▶ YamlEscape: upstream's quoting
//! ```
//!
//! This is the crate's one copy of upstream's printer: every format prints its tree lines with
//! `node_text`, so no two formats can print a key differently. Upstream's renderer recurses; here
//! the walk keeps its own stack and hands the printer one node without its children, which is how
//! the TypeScript renderer already calls upstream's function.
//!
//! Ported: `createKey`, `visit`, `visitText` and `indent` of `isomorphic/ariaSnapshotRenderer.ts`,
//! and all of `isomorphic/yaml.ts`. Not ported: the regular-expression mode
//! (`convertToBestGuessRegex`, `textContributesInfo`), which only a failed `expect` uses, in the
//! page, with the page's own copy of upstream's file. `PORTED_FROM` records the vendored files'
//! hashes, and neo-playwright's `engine_sources` test fails when an upgrade changes one.
//!
//! The judges, in `tests.rs` through `crate::judges`: the committed cases, written by
//! neo-playwright-js's build.ts from upstream's printer and today's TypeScript renderer, and those
//! two again in QuickJS beside the Rust, on seeded random nodes and trees.

mod node_text;
mod ported_from;
#[cfg(test)]
mod tests;
mod tree_text;
mod yaml_escape;

pub(crate) use node_text::{NodeAlone, node_text, text_line};
pub use ported_from::{PORTED_FROM, PortedFile};
pub(crate) use tree_text::{IframeText, TreeLine, tree_lines, tree_text, yaml_lines};
