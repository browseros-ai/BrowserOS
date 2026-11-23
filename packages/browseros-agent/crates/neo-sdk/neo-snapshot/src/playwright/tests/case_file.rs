//! One file of snapshot text cases, as neo-playwright-js's case writer writes it
//! (`scripts/snapshot-cases/model.ts` documents the shape): named snapshots, then calls on them
//! with the text today's renderer printed. The committed files and the private sets of saved
//! pages share the shape.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Value;

use crate::aria_node::{AriaChild, AriaNode};
use crate::snapshot::{PartialFrame, Snapshot};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CaseFile {
    #[serde(rename = "about")]
    _about: IgnoredAny,
    snapshots: BTreeMap<String, CaseSnapshot>,
    cases: Vec<Case>,
}

/// One call on a named snapshot, and the text it printed (`null` only for a cover with nothing to
/// place).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Case {
    pub(super) name: String,
    pub(super) snapshot: Option<String>,
    /// Which text, by its one key (`playwright`, `full`, `element`, `page`, `delta`, `cover`) with
    /// that call's options.
    call: BTreeMap<String, Value>,
    pub(super) text: Option<String>,
    /// What a whole page in Neo's format keeps as the tab's baseline; not Playwright's format's.
    #[serde(rename = "pending")]
    _pending: Option<IgnoredAny>,
}

/// A snapshot as the file holds it: the tree as JSON, or as a chain built here because it nests
/// deeper than a JSON file may.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseSnapshot {
    json: Option<Vec<AriaNode>>,
    chain: Option<Chain>,
    partial: Vec<CasePartial>,
    // Neo's facts about the page. Playwright's format prints none of them, which the cases that
    // carry one, in that format, hold.
    #[serde(rename = "onTop")]
    _on_top: Option<IgnoredAny>,
    #[serde(rename = "closedRoots")]
    _closed_roots: Option<IgnoredAny>,
    #[serde(rename = "unmarked")]
    _unmarked: Option<IgnoredAny>,
}

/// `depth` nodes, each of the first `depth - 1` a copy of `link` whose one child is the next, the
/// last one `leaf`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Chain {
    depth: usize,
    link: AriaNode,
    leaf: AriaNode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CasePartial {
    r#ref: String,
    frame: String,
    reason: String,
}

impl CaseFile {
    /// The cases that ask for Playwright's format.
    pub(super) fn playwright_cases(&self) -> impl Iterator<Item = &Case> {
        self.cases
            .iter()
            .filter(|case| case.call.contains_key("playwright"))
    }

    /// Every snapshot's name.
    pub(super) fn snapshot_names(&self) -> impl Iterator<Item = &str> {
        self.snapshots.keys().map(String::as_str)
    }

    /// The snapshot named `name`, built as the renderer is handed it.
    pub(super) fn snapshot(&self, name: &str) -> Result<Snapshot, String> {
        let found = self
            .snapshots
            .get(name)
            .ok_or_else(|| format!("no snapshot is named {name}"))?;
        let tree = match (&found.json, &found.chain) {
            (Some(json), None) => json.clone(),
            (None, Some(chain)) => vec![chain.built()],
            _ => return Err(format!("{name} holds neither a tree nor a chain, or both")),
        };
        let partial = found
            .partial
            .iter()
            .map(|frame| PartialFrame {
                r#ref: frame.r#ref.clone(),
                frame: frame.frame.clone(),
                reason: frame.reason.clone(),
            })
            .collect();
        Ok(Snapshot { tree, partial })
    }
}

impl Chain {
    fn built(&self) -> AriaNode {
        let mut node = self.leaf.clone();
        for _ in 1..self.depth {
            node = AriaNode {
                children: vec![AriaChild::Node(Box::new(node))],
                ..self.link.clone()
            };
        }
        node
    }
}
