//! The `aria-template` engine over the tree: Playwright 1.64.1's `_createAriaTemplateEngine`
//! (`injectedScript.ts:770-778`), which runs `getAllElementsMatchingExpectAriaTemplate`
//! (`ariaSnapshot.ts:371-375`) from each root, answered from the APC capture instead of the
//! page's JavaScript ARIA engine. The root's tree is emitted in template mode, the tree
//! `toMatchAriaSnapshot` matches, and `neo_snapshot::template` matches it, so a selector and an
//! assertion read one template the same way.
//!
//! ```text
//!  body ─ parse (once, when the stage is planned) ─ AriaTemplateNode ───────────────┐
//!  root ─ prepare_aria(template mode, one document) ─ PreparedAria ─┬ source_nodes ─┤ preorder
//!                                                                   └ AriaNode forest ─ match_template
//!                                                                         &AriaNode ─▶ NodeId
//! ```
//!
//! A match is a node of the emitted forest; `PreparedAria::source_nodes` lists, in the same
//! preorder, the tree node each forest node came from, which turns it back into the element
//! upstream's `ariaNodeElement` returns.

use std::collections::HashMap;
use std::ptr;

use neo_snapshot::template::{self, AriaTemplateNode, TemplateError, TemplateMatch};
use neo_snapshot::{AriaChild, AriaNode};
use thiserror::Error;

use crate::tree::{AriaAsk, AriaRoot, AriaTooDeep, NameCache, NodeId, SnapshotTree};

/// Why an `aria-template` part stopped its query, each worded with the engine's name first so a
/// script can tell which part of a chain failed.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum AriaTemplateError {
    /// The body is not a template. Upstream's `JSON.parse` throws on text that is not JSON and
    /// trusts any JSON it gets; Rust decodes the template's shape, so both refuse here. Only
    /// where the decode stopped is kept: serde's message quotes the text it could not read.
    #[error(
        "aria-template: the selector body is not an aria template, Playwright's parsed template JSON such as {{\"kind\":\"role\",\"role\":\"button\",\"name\":\"Pay\"}}; it stops at line {line}, column {column}"
    )]
    Body {
        /// The body's line where the decode stopped, from 1.
        line: usize,
        /// The column on that line, from 1.
        column: usize,
    },
    /// A name regex of the template does not compile, where upstream's `new RegExp` throws.
    #[error("aria-template: {0}")]
    Template(TemplateError),
    /// The root's tree is nested deeper than an aria tree may be, which a snapshot of it would
    /// refuse too.
    #[error(
        "aria-template: the tree below this root is nested about {levels} levels deep, more than a snapshot carries (about 147); match from a smaller section",
        levels = .0.levels()
    )]
    TooDeep(AriaTooDeep),
    /// A template where only the page could run it: from an element the capture has no node
    /// for, or inside a filter the page evaluates. Its matches depend on the root it starts
    /// from, so no candidate set can stand for it (see `ChunkPlan`), and the page's engine for
    /// it is a stand-in that throws.
    #[error(
        "aria-template: Neo matches a template over the accessibility snapshot, so only from the page, a frame or an element the snapshot holds; this one starts from an element it leaves out, such as a plain div, or sits inside a filter the page evaluates"
    )]
    OutsideTree,
}

/// Decodes a part's body into the template it names.
pub(super) fn parse(body: &str) -> Result<AriaTemplateNode, AriaTemplateError> {
    serde_json::from_str(body).map_err(|error| AriaTemplateError::Body {
        line: error.line(),
        column: error.column(),
    })
}

/// Every element of `root`'s tree that `template` matches, top-most first in document order:
/// `getAllElementsMatchingExpectAriaTemplate(root, template)`.
///
/// `from` is where the tree starts: the root's own node for an element, so the element itself
/// takes part as upstream's tree starts at its root element, or its document for a document
/// root. A match on the synthetic root (a `fragment` template, or text at the top) is `root`
/// itself; for a document that is the `Document`, where upstream names `<html>`, which the tree
/// has no node of its own for. A direct read of the Document reads `<html>`, but a later stage
/// starts from the Document: `css=html` after it finds `<html>`, where upstream finds nothing
/// below `<html>`. Telling the two apart would take a set member of its own in the probe.
///
/// `names` is one cache of `tree` for every root of the step, so a set of many roots builds the
/// capture's name tables once.
pub(super) fn matches(
    tree: &SnapshotTree,
    names: &NameCache<'_>,
    from: AriaRoot,
    root: NodeId,
    template: &AriaTemplateNode,
) -> Result<Vec<NodeId>, AriaTemplateError> {
    let prepared = tree
        .prepare_aria_with(AriaAsk::template_mode().with_root(from), names)
        .map_err(AriaTemplateError::TooDeep)?;
    let sources = prepared.source_nodes();
    let forest = prepared.into_aria_tree();
    let nodes = forest.nodes();
    let elements = elements_by_address(nodes, sources);
    let found = template::match_template(nodes, template).map_err(AriaTemplateError::Template)?;
    Ok(found
        .into_iter()
        .filter_map(|matched| match matched {
            TemplateMatch::Root => Some(root),
            // Every node a template can match was made from an element: a top-level text run,
            // the one node made from none, matches only as text, which reports its container.
            TemplateMatch::Node(node) => elements.get(&ptr::from_ref(node)).copied(),
        })
        .collect())
}

/// The forest's nodes by address, each to the tree node it was made from. `match_template`
/// borrows its matches from `nodes`, so an address is the identity of a match, as it is for
/// `AriaSource` over a node slice; the walk is the preorder `sources` is listed in.
fn elements_by_address(
    nodes: &[AriaNode],
    sources: Vec<Option<NodeId>>,
) -> HashMap<*const AriaNode, NodeId> {
    let mut elements = HashMap::with_capacity(sources.len());
    let mut sources = sources.into_iter();
    let mut pending: Vec<&AriaNode> = nodes.iter().rev().collect();
    while let Some(node) = pending.pop() {
        if let Some(Some(source)) = sources.next() {
            elements.insert(ptr::from_ref(node), source);
        }
        pending.extend(node.children.iter().rev().filter_map(|child| match child {
            AriaChild::Node(child) => Some(&**child),
            AriaChild::Text(_) => None,
        }));
    }
    elements
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use neo_snapshot::{AriaChild, AriaNode};

    use crate::tree::{
        ApcBuilder, AriaAsk, BuildOptions, FrameBuilder, NodeBuilder, NodeId, SnapshotTree,
    };

    /// The forest and the source list are two walks of one prepared tree; if they ever disagree
    /// on preorder, every match after the first difference names the wrong element.
    #[test]
    fn every_forest_node_traces_to_the_tree_node_it_was_made_from() -> Result<(), Box<dyn Error>> {
        let button = |dom: i32, label: &str| {
            NodeBuilder::element("button")
                .dom_id(dom)
                .child(NodeBuilder::text(label))
        };
        let child = FrameBuilder::new("doc-child").root(NodeBuilder::root().child(button(1, "In")));
        let apc = ApcBuilder::new("doc-main")
            .root(NodeBuilder::root().children(vec![
                NodeBuilder::text("Loose"),
                button(10, "Pay"),
                NodeBuilder::element("ul").dom_id(11).children(vec![
                    NodeBuilder::element("li")
                        .dom_id(12)
                        .children(vec![NodeBuilder::text("Item "), button(13, "Edit")]),
                    NodeBuilder::element("li").dom_id(14),
                ]),
                NodeBuilder::iframe(child).tag("iframe").dom_id(15),
                NodeBuilder::element("h2").dom_id(16).child(NodeBuilder::text("End")),
            ]))
            .build();
        let tree = SnapshotTree::build(apc, BuildOptions::default())?;
        let prepared = tree.prepare_aria(AriaAsk::template_mode())?;
        let sources = prepared.source_nodes();
        let forest = prepared.into_aria_tree();
        let mut walked: Vec<(&str, Option<i32>)> = Vec::new();
        let mut pending: Vec<&AriaNode> = forest.nodes().iter().rev().collect();
        let mut sources = sources.into_iter();
        while let Some(node) = pending.pop() {
            let source = sources.next().ok_or("a forest node past the source list")?;
            let dom = source
                .and_then(|id: NodeId| tree.node_key(id))
                .map(|key| key.node.raw());
            walked.push((node.role.as_str(), dom));
            pending.extend(node.children.iter().rev().filter_map(|child| match child {
                AriaChild::Node(child) => Some(&**child),
                AriaChild::Text(_) => None,
            }));
        }
        assert!(
            sources.next().is_none(),
            "a source past the forest's last node"
        );
        assert_eq!(
            walked,
            [
                ("text", None),
                ("button", Some(10)),
                ("list", Some(11)),
                ("listitem", Some(12)),
                ("button", Some(13)),
                ("listitem", Some(14)),
                ("iframe", Some(15)),
                ("heading", Some(16)),
            ],
            "each node is its element's, the top-level text none, and the frame's content is \
             another document's"
        );
        Ok(())
    }
}
