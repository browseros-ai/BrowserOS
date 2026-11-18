//! Playwright 1.64.1's own lines for a snapshot's nodes, in Rust: upstream's YAML escaping and its
//! printer for one node.
//!
//! ```text
//!  one node, without its children (NodeAlone)
//!     ▼
//!  node_text: upstream's createKey and visit ─▶ YamlEscape: upstream's quoting
//! ```
//!
//! This is the crate's one copy of upstream's printer: every format prints its tree lines with
//! `node_text`, so no two formats can print a key differently. Upstream's renderer recurses; here
//! a format's walk keeps its own stack and hands the printer one node without its children, which
//! is how the TypeScript renderer already calls upstream's function.
//!
//! Ported: `createKey`, `visit`, `visitText` and `indent` of `isomorphic/ariaSnapshotRenderer.ts`,
//! and all of `isomorphic/yaml.ts`. Not ported: the regular-expression mode
//! (`convertToBestGuessRegex`, `textContributesInfo`), which only a failed `expect` uses, in the
//! page, with the page's own copy of upstream's file.
//!
//! The judge, in `tests.rs`: upstream's printer itself, in QuickJS beside the Rust, on seeded
//! random nodes.

mod node_text;
#[cfg(test)]
mod tests;
mod yaml_escape;

pub(crate) use node_text::{NodeAlone, node_text};
