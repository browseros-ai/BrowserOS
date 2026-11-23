use crate::aria_node::AriaNode;

use super::on_top::OnTop;
use super::rule::{Child, actionable, field_without_ref, named, shown_text};

/// `ownText`: every text piece under `node` outside a nested actionable node or field without a
/// ref, joined with one space. Those controls print their own lines, so their names and values
/// cannot label their parent too. A content-only name counts as a piece (an image's alt text, a
/// cell's text), and an iframe's fallback markup does not (`shown_text`). An inert subtree counts
/// only where the call lists inert content.
///
/// Depth first in document order, on a stack of its own.
pub(super) fn own_text<'a>(node: &'a AriaNode, inert_listed: bool, on_top: &OnTop<'a>) -> String {
    let mut pieces: Vec<&'a str> = Vec::new();
    let mut stack: Vec<Child<'a>> = vec![Child::Node(node)];
    while let Some(item) = stack.pop() {
        let item = match item {
            Child::Text(text) => {
                pieces.push(text);
                continue;
            }
            Child::Node(item) => item,
        };
        if let Some(shown) = shown_text(item) {
            pieces.push(shown);
        }
        for child in on_top.children(item).rev() {
            match child {
                Child::Node(node) if node.inert == Some(true) && !inert_listed => {}
                Child::Text(_) => stack.push(child),
                Child::Node(node) if node.role == "text" => stack.push(child),
                Child::Node(node) if !actionable(node) && !field_without_ref(node) => {
                    stack.push(name_as_content(node, on_top).map_or(child, Child::Text));
                }
                Child::Node(_) => {}
            }
        }
    }
    let mut text = String::new();
    for piece in pieces.into_iter().filter(|piece| !piece.is_empty()) {
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
pub(super) fn name_as_content<'a>(node: &'a AriaNode, on_top: &OnTop<'a>) -> Option<&'a str> {
    let name = named(node.name.as_deref())?;
    if node.text.is_some() || node.role == "heading" || field_without_ref(node) {
        return None;
    }
    says_nothing(node, on_top).then_some(name)
}

/// `saysNothing(node.children)`: no string and no node with a printed text or a name at any
/// depth under `node`, an empty one printing nothing. Stops at the first that says something.
fn says_nothing<'a>(node: &'a AriaNode, on_top: &OnTop<'a>) -> bool {
    let mut pending: Vec<Child<'a>> = on_top.children(node).collect();
    while let Some(item) = pending.pop() {
        match item {
            Child::Text(text) => {
                if !text.is_empty() {
                    return false;
                }
            }
            Child::Node(node) => {
                if named(shown_text(node)).is_some() || named(node.name.as_deref()).is_some() {
                    return false;
                }
                pending.extend(on_top.children(node));
            }
        }
    }
    true
}
