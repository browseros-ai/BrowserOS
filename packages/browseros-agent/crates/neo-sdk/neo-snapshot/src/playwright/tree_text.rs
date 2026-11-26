use crate::aria_sink::EmittedNode;
use crate::aria_source::{AriaSource, SnapshotView, SourceChild};
use crate::cap::push_indent;
use crate::js;
use crate::neo::shown_text;
use crate::snapshot::PartialFrame;

use super::node_text::{NodeAlone, node_text_into, text_line};

/// One line of Playwright's text, with the node whose key line it is. A text line, the lines
/// under a key and a comment have none.
pub(crate) struct TreeLine<'a> {
    pub(crate) text: String,
    pub(crate) node: Option<EmittedNode<'a>>,
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

/// Playwright's own text for a snapshot, as `ariaSnapshot()` returns it with `mode: 'default'`
/// and in every mode under the conformance switch: upstream's lines for every node, an iframe's
/// fallback text kept, and each frame that could not be read as a `# partial:` comment under its
/// iframe. No head line and no cap: none of Neo's facts about the page prints in this format.
///
/// Ported from the TypeScript renderer's `renderSnapshot` (neo-playwright-js
/// `prelude/snapshot.ts`), whose output it must equal byte for byte.
pub(crate) fn tree_text_source<'a, S: AriaSource<'a>>(view: &SnapshotView<'a, S>) -> String {
    if view.partial.is_empty() {
        let mut text = String::new();
        let mut first = true;
        visit_tree_lines(
            &view.tree,
            view.tree.roots(),
            IframeText::Fallback,
            |line, _| {
                if !first {
                    text.push('\n');
                }
                first = false;
                text.push_str(line);
            },
        );
        return text;
    }
    let lines = yaml_lines_source(view, IframeText::Fallback);
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
pub(crate) fn yaml_lines_source<'a, S: AriaSource<'a>>(
    view: &SnapshotView<'a, S>,
    iframe: IframeText,
) -> Vec<TreeLine<'a>> {
    with_partial_frames(
        tree_lines_source(&view.tree, view.tree.roots(), iframe),
        view.partial,
    )
}

/// `treeLines`: upstream's lines for `roots` in `source`, each element's key line with its node,
/// `iframe` saying what an iframe's text is.
pub(crate) fn tree_lines_source<'a, S, Roots>(
    source: &S,
    roots: Roots,
    iframe: IframeText,
) -> Vec<TreeLine<'a>>
where
    S: AriaSource<'a>,
    Roots: IntoIterator<Item = S::Id>,
    Roots::IntoIter: DoubleEndedIterator,
{
    let mut lines = Vec::new();
    visit_tree_lines(source, roots, iframe, |text, node| {
        lines.push(TreeLine {
            text: text.to_owned(),
            node,
        });
    });
    lines
}

/// Streams Playwright's lines for `roots` in `source` into `emit`, reusing scratch buffers across
/// every node so callers that only join or count lines never allocate per-line `String`s.
///
/// Upstream's renderer recurses once per level. This walk keeps its pending children on its own
/// stack, as the TypeScript does, so no tree is too deep for it. A node's own lines are
/// `node_text`'s for the node alone, split at each line break: the first is indented and gains the
/// colon that opens the node's block when its children follow, the rest are indented. That split
/// is the TypeScript's and changes bytes: where a role or a ref holds a line break, upstream's own
/// whole-tree text keeps the colon at the end of the quoted key and the key's later lines at the
/// margin, while this text puts the colon after the key's first line and indents the rest.
pub(crate) fn visit_tree_lines<'a, S, Roots, F>(
    source: &S,
    roots: Roots,
    iframe: IframeText,
    mut emit: F,
) where
    S: AriaSource<'a>,
    Roots: IntoIterator<Item = S::Id>,
    Roots::IntoIter: DoubleEndedIterator,
    F: FnMut(&str, Option<EmittedNode<'a>>),
{
    let mut key_buf = String::new();
    let mut own = String::new();
    let mut line_buf = String::new();
    let mut steps: Vec<(SourceChild<'a, S::Id>, usize)> = roots
        .into_iter()
        .rev()
        .map(|node| (SourceChild::Node(node), 0))
        .collect();
    while let Some((step, depth)) = steps.pop() {
        own.clear();
        let (id, node) = match step {
            SourceChild::Text(text) => {
                line_buf.clear();
                push_indent(&mut line_buf, depth);
                text_line(text, &mut line_buf);
                emit(&line_buf, None);
                continue;
            }
            SourceChild::Node(id) => {
                let node = source.node(id);
                // A text node prints as its text alone, and its line is no element's.
                if node.role == "text" {
                    line_buf.clear();
                    push_indent(&mut line_buf, depth);
                    text_line(node.text.unwrap_or_default(), &mut line_buf);
                    emit(&line_buf, None);
                    continue;
                }
                (id, node)
            }
        };
        let text = match iframe {
            IframeText::Fallback => node.text,
            IframeText::Dropped => shown_text(&node),
        };
        node_text_into(
            &NodeAlone {
                text,
                ..NodeAlone::from_emitted(&node)
            },
            &mut key_buf,
            &mut own,
        );
        // Alone, a node whose children print under it has no text, url or placeholder, so
        // upstream printed it as a leaf, without the colon.
        let opens_children =
            text.is_none() && node.url.is_none() && node.placeholder.is_none() && node.has_children;
        let mut own_lines = own.split('\n');
        let key = own_lines.next().unwrap_or_default();
        line_buf.clear();
        push_indent(&mut line_buf, depth);
        line_buf.push_str(key);
        if opens_children {
            line_buf.push(':');
        }
        emit(&line_buf, Some(node));
        for line in own_lines {
            line_buf.clear();
            push_indent(&mut line_buf, depth);
            line_buf.push_str(line);
            emit(&line_buf, None);
        }
        // Upstream prints no child of a node with a text.
        if text.is_none() && node.has_children {
            steps.extend(source.children(id).rev().map(|child| (child, depth + 1)));
        }
    }
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
        .position(|line| {
            line.node
                .and_then(|node| node.r#ref)
                .is_some_and(|reference| reference.eq_str(iframe_ref))
        })
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
