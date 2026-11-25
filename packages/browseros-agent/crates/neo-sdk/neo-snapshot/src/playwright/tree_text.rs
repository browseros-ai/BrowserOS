use crate::aria_node::{AriaChild, AriaNode};
use crate::js;
use crate::neo::shown_text;
use crate::snapshot::{PartialFrame, Snapshot};

use super::node_text::{NodeAlone, node_text, text_line};

/// One line of Playwright's text, with the node whose key line it is. A text line, the lines
/// under a key and a comment have none.
pub(crate) struct TreeLine<'a> {
    pub(crate) text: String,
    pub(crate) node: Option<&'a AriaNode>,
}

/// What an iframe's own text is in the two formats that print Playwright's lines for every node.
#[derive(Clone, Copy, Debug)]
pub(crate) enum IframeText {
    /// Playwright's text: the fallback markup written between the iframe's tags is the node's
    /// text and prints on its line, and, as upstream prints no child of a node with text, the
    /// frame's content stitched under it does not.
    Fallback,
    /// `mode: 'full'`: Neo's rule (`shown_text`) drops that markup, which no browser shows, so the
    /// iframe opens a block of its frame's content as an iframe without fallback does.
    Dropped,
}

/// One entry of the walk's own stack: a child still to print, at its depth.
enum Step<'a> {
    Text(&'a str),
    Node(&'a AriaNode),
}

/// Playwright's own text for a snapshot, as `ariaSnapshot()` returns it with `mode: 'default'`
/// and in every mode under the conformance switch: upstream's lines for every node, an iframe's
/// fallback text kept, and each frame that could not be read as a `# partial:` comment under its
/// iframe. No head line and no cap: none of Neo's facts about the page prints in this format.
///
/// Ported from the TypeScript renderer's `renderSnapshot` (neo-playwright-js
/// `prelude/snapshot.ts`), whose output it must equal byte for byte.
pub(crate) fn tree_text(snapshot: &Snapshot) -> String {
    let lines = yaml_lines(snapshot, IframeText::Fallback);
    let mut text = String::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        text.push_str(&line.text);
    }
    text
}

/// `yamlLines`: upstream's lines for the snapshot's tree, `iframe` saying what an iframe's text
/// is, with each frame that could not be read placed as a `# partial:` comment.
pub(crate) fn yaml_lines(snapshot: &Snapshot, iframe: IframeText) -> Vec<TreeLine<'_>> {
    with_partial_frames(tree_lines(&snapshot.tree, iframe), &snapshot.partial)
}

/// `treeLines`: upstream's lines for `tree`, each element's key line with its node, `iframe`
/// saying what an iframe's text is.
///
/// Upstream's renderer recurses once per level. This walk keeps its pending children on its own
/// stack, as the TypeScript does, so no tree is too deep for it. A node's own lines are
/// `node_text`'s for the node alone, split at each line break: the first is indented and gains the
/// colon that opens the node's block when its children follow, the rest are indented. That split
/// is the TypeScript's and changes bytes: where a role or a ref holds a line break, upstream's own
/// whole-tree text keeps the colon at the end of the quoted key and the key's later lines at the
/// margin, while this text puts the colon after the key's first line and indents the rest.
pub(crate) fn tree_lines<'a, Tree>(tree: Tree, iframe: IframeText) -> Vec<TreeLine<'a>>
where
    Tree: IntoIterator<Item = &'a AriaNode>,
    Tree::IntoIter: DoubleEndedIterator,
{
    let mut lines = Vec::new();
    // One node's own text, reused for every node.
    let mut own = String::new();
    let mut steps: Vec<(Step<'_>, usize)> = tree
        .into_iter()
        .rev()
        .map(|node| (Step::Node(node), 0))
        .collect();
    while let Some((step, depth)) = steps.pop() {
        let indent = "  ".repeat(depth);
        own.clear();
        let node = match step {
            Step::Text(text) => {
                text_line(text, &mut own);
                lines.push(TreeLine {
                    text: format!("{indent}{own}"),
                    node: None,
                });
                continue;
            }
            // A text node prints as its text alone, and its line is no element's.
            Step::Node(node) if node.role == "text" => {
                text_line(node.text.as_deref().unwrap_or_default(), &mut own);
                lines.push(TreeLine {
                    text: format!("{indent}{own}"),
                    node: None,
                });
                continue;
            }
            Step::Node(node) => node,
        };
        let text = match iframe {
            IframeText::Fallback => node.text.as_deref(),
            IframeText::Dropped => shown_text(node),
        };
        node_text(
            &NodeAlone {
                text,
                ..NodeAlone::from(node)
            },
            &mut own,
        );
        // Alone, a node whose children print under it has no text, url or placeholder, so
        // upstream printed it as a leaf, without the colon.
        let opens_children = text.is_none()
            && node.url.is_none()
            && node.placeholder.is_none()
            && !node.children.is_empty();
        let mut own_lines = own.split('\n');
        let key = own_lines.next().unwrap_or_default();
        let colon = if opens_children { ":" } else { "" };
        lines.push(TreeLine {
            text: format!("{indent}{key}{colon}"),
            node: Some(node),
        });
        for line in own_lines {
            lines.push(TreeLine {
                text: format!("{indent}{line}"),
                node: None,
            });
        }
        // Upstream prints no child of a node with a text.
        if text.is_none() {
            steps.extend(node.children.iter().rev().map(|child| {
                let step = match child {
                    AriaChild::Text(text) => Step::Text(text),
                    AriaChild::Node(node) => Step::Node(node),
                };
                (step, depth + 1)
            }));
        }
    }
    lines
}

/// `yamlLines`: `lines` with each partial frame's comment two spaces under its iframe's line, and
/// the comments with no line at the end.
///
/// Every iframe's line is found before any comment goes in. The comments then go in from the
/// bottom up, after a stable sort by that line, so two for one iframe each go directly under it
/// and end reversed. The unplaced ones are put in front of each other, reversed too. Both
/// reversals are bytes of the TypeScript's text.
fn with_partial_frames<'a>(
    mut lines: Vec<TreeLine<'a>>,
    partial: &[PartialFrame],
) -> Vec<TreeLine<'a>> {
    let mut placed: Vec<(Option<usize>, &PartialFrame)> = partial
        .iter()
        .map(|frame| (iframe_line(&lines, &frame.r#ref), frame))
        .collect();
    // `placed.sort((a, b) => b.line - a.line)`, with `None` for the TypeScript's -1: last.
    placed.sort_by(|(above, _), (below, _)| below.cmp(above));
    let mut unplaced = Vec::new();
    for (line, frame) in placed {
        let comment = format!("# partial: {} not read ({})", frame.frame, frame.reason);
        let Some(index) = line else {
            unplaced.insert(
                0,
                TreeLine {
                    text: comment,
                    node: None,
                },
            );
            continue;
        };
        let indent = lines
            .get(index)
            .map_or("", |iframe| leading_space(&iframe.text))
            .to_owned();
        lines.insert(
            index + 1,
            TreeLine {
                text: format!("{indent}  {comment}"),
                node: None,
            },
        );
    }
    lines.extend(unplaced);
    lines
}

/// `iframeLine`: the line of the node whose ref is `iframe_ref`; else the first element line
/// (`/^\s*- /`) whose text holds `[ref=<iframe_ref>]`, which page text can also hold.
fn iframe_line(lines: &[TreeLine<'_>], iframe_ref: &str) -> Option<usize> {
    lines
        .iter()
        .position(|line| line.node.and_then(|node| node.r#ref.as_deref()) == Some(iframe_ref))
        .or_else(|| {
            let tag = format!("[ref={iframe_ref}]");
            lines.iter().position(|line| {
                js::trim_start(&line.text).starts_with("- ") && line.text.contains(&tag)
            })
        })
}

/// `/^\s*/.exec(text)[0]`: the JavaScript whitespace `text` starts with.
fn leading_space(text: &str) -> &str {
    text.strip_suffix(js::trim_start(text)).unwrap_or_default()
}
