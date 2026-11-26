use crate::aria_sink::EmittedNode;
use crate::aria_source::{AriaSource, SourceChild};

use super::on_top::OnTop;
use super::rule::{actionable, field_without_ref, named, shown_text};

/// Reusable scratch buffers for [`own_text`] and [`name_as_content`], kept on `Printer` so
/// walking large trees does not allocate temporary stacks per node.
pub(super) struct TextScratch<'a, Id> {
    pieces: Vec<&'a str>,
    stack: Vec<SourceChild<'a, Id>>,
    says_pending: Vec<SourceChild<'a, Id>>,
}

impl<Id> Default for TextScratch<'_, Id> {
    fn default() -> Self {
        Self {
            pieces: Vec::new(),
            stack: Vec::new(),
            says_pending: Vec::new(),
        }
    }
}

/// `ownText`: every text piece under `id` outside a nested actionable node or field without a
/// ref, joined with one space. Those controls print their own lines, so their names and values
/// cannot label their parent too. A content-only name counts as a piece (an image's alt text, a
/// cell's text), and an iframe's fallback markup does not (`shown_text`). An inert subtree counts
/// only where the call lists inert content.
///
/// Depth first in document order, on a stack of its own.
pub(super) fn own_text<'a, S: AriaSource<'a>>(
    source: &S,
    id: S::Id,
    inert_listed: bool,
    on_top: &OnTop<S::Id, S::Key>,
    scratch: &mut TextScratch<'a, S::Id>,
) -> String {
    scratch.pieces.clear();
    scratch.stack.clear();
    scratch.stack.push(SourceChild::Node(id));
    while let Some(item) = scratch.stack.pop() {
        let item_id = match item {
            SourceChild::Text(text) => {
                scratch.pieces.push(text);
                continue;
            }
            SourceChild::Node(item_id) => item_id,
        };
        let item_node = source.node(item_id);
        if let Some(shown) = shown_text(&item_node) {
            scratch.pieces.push(shown);
        }
        if !item_node.has_children {
            continue;
        }
        for child in on_top.children(source, item_id).rev() {
            match child {
                SourceChild::Text(_) => scratch.stack.push(child),
                SourceChild::Node(child_id) => {
                    let child_node = source.node(child_id);
                    if child_node.inert == Some(true) && !inert_listed {
                        continue;
                    }
                    if child_node.role == "text" {
                        scratch.stack.push(child);
                    } else if !actionable(&child_node) && !field_without_ref(&child_node) {
                        let next = name_as_content_with(
                            source,
                            child_id,
                            &child_node,
                            on_top,
                            &mut scratch.says_pending,
                        )
                        .map_or(child, SourceChild::Text);
                        scratch.stack.push(next);
                    }
                }
            }
        }
    }
    let mut text = String::new();
    for piece in scratch
        .pieces
        .iter()
        .copied()
        .filter(|piece| !piece.is_empty())
    {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(piece);
    }
    text
}

/// `nameAsContent`: the name of a node under which nothing prints text, which is then its
/// content. The engine names a cell, a list item or a group from its text and drops the text
/// child that repeats the name (`<td>$7.49</td>` is `cell "$7.49"` with no children), so without
/// this the price would print nowhere. A node with a child that prints text keeps that child's
/// lines instead, so its name, derived from them, never prints twice. A node with a `text` has
/// that as its content; a heading prints its own line; a field without a ref keeps its name on
/// its own line.
pub(super) fn name_as_content<'a, S: AriaSource<'a>>(
    source: &S,
    id: S::Id,
    node: &EmittedNode<'a>,
    on_top: &OnTop<S::Id, S::Key>,
    scratch: &mut TextScratch<'a, S::Id>,
) -> Option<&'a str> {
    name_as_content_with(source, id, node, on_top, &mut scratch.says_pending)
}

fn name_as_content_with<'a, S: AriaSource<'a>>(
    source: &S,
    id: S::Id,
    node: &EmittedNode<'a>,
    on_top: &OnTop<S::Id, S::Key>,
    pending: &mut Vec<SourceChild<'a, S::Id>>,
) -> Option<&'a str> {
    let name = named(node.name)?;
    if node.text.is_some() || node.role == "heading" || field_without_ref(node) {
        return None;
    }
    if !node.has_children {
        return Some(name);
    }
    says_nothing(source, id, on_top, pending).then_some(name)
}

/// `saysNothing(node.children)`: no string and no node with a printed text or a name at any
/// depth under `id`, an empty one printing nothing. Stops at the first that says something.
fn says_nothing<'a, S: AriaSource<'a>>(
    source: &S,
    id: S::Id,
    on_top: &OnTop<S::Id, S::Key>,
    pending: &mut Vec<SourceChild<'a, S::Id>>,
) -> bool {
    pending.clear();
    pending.extend(on_top.children(source, id));
    while let Some(item) = pending.pop() {
        match item {
            SourceChild::Text(text) => {
                if !text.is_empty() {
                    return false;
                }
            }
            SourceChild::Node(child_id) => {
                let child_node = source.node(child_id);
                if named(shown_text(&child_node)).is_some() || named(child_node.name).is_some() {
                    return false;
                }
                if child_node.has_children {
                    pending.extend(on_top.children(source, child_id));
                }
            }
        }
    }
    true
}
