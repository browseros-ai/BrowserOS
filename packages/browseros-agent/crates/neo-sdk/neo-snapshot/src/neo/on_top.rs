use std::borrow::Cow;
use std::collections::HashSet;
use std::ptr;

use crate::aria_node::{AriaChild, AriaNode};
use crate::js;
use crate::snapshot::Snapshot;

use super::rule::Child;

/// What the whole-view layers hold (`onTopContent` in `snapshot-on-top.ts`): Neo's default
/// format prints it first, then the rest of the page without it.
///
/// The TypeScript projects a copy of the tree with those nodes removed and finds them again by
/// object identity. Here the walk borrows the tree as it is and leaves out the held nodes as it
/// meets them, by address: every child list the walk, the omissions and the label rules read goes
/// through [`OnTop::children`], so each sees the children the projection would hold. The search
/// never enters a held node or an inert one, so no held node lies inside either, and a printer
/// walking inside them sees the same children with or without the filter.
#[derive(Debug, Default)]
pub(super) struct OnTop<'a> {
    /// Each whole-view layer's block in layer order: the nodes marked `onTop` with its ref. Two
    /// layers with one ref each print that block, as the TypeScript's map does.
    pub(super) first: Vec<&'a AriaNode>,
    /// The same nodes, by address in the borrowed tree.
    held: HashSet<*const AriaNode>,
}

impl<'a> OnTop<'a> {
    pub(super) fn of(snapshot: &'a Snapshot) -> Self {
        let layers: Vec<&str> = snapshot
            .layers
            .iter()
            .filter(|layer| layer.whole_view)
            .map(|layer| layer.r#ref.as_str())
            .collect();
        if layers.is_empty() {
            return Self::default();
        }
        // One block per distinct ref, as a map keyed by ref holds it.
        let mut blocks: Vec<(&str, Vec<&'a AriaNode>)> = Vec::new();
        for layer in &layers {
            if !blocks.iter().any(|(held_by, _)| held_by == layer) {
                blocks.push((layer, Vec::new()));
            }
        }
        let mut held = HashSet::new();
        // Depth first in document order, on a stack of its own.
        let mut pending: Vec<&'a AriaNode> = snapshot.tree.iter().rev().collect();
        while let Some(node) = pending.pop() {
            if node.inert == Some(true) {
                continue;
            }
            // `node.onTop ? front.get(node.onTop) : undefined`: an empty mark selects nothing.
            let block = node
                .on_top
                .as_deref()
                .filter(|mark| !mark.is_empty())
                .and_then(|mark| blocks.iter_mut().find(|(held_by, _)| *held_by == mark));
            if let Some((_, block)) = block {
                held.insert(ptr::from_ref(node));
                block.push(node);
            } else {
                pending.extend(node.children.iter().rev().filter_map(|child| match child {
                    AriaChild::Node(node) => Some(&**node),
                    AriaChild::Text(_) => None,
                }));
            }
        }
        let mut first = Vec::new();
        for layer in &layers {
            if let Some((_, block)) = blocks.iter().find(|(held_by, _)| held_by == layer) {
                first.extend(block.iter().copied());
            }
        }
        Self { first, held }
    }

    /// `node`'s children without the held nodes, in document order.
    pub(super) fn children(
        &self,
        node: &'a AriaNode,
    ) -> impl DoubleEndedIterator<Item = Child<'a>> + '_ {
        self.without_held(node.children.iter().map(Child::from))
    }

    /// The page's top-level nodes without the held nodes, in document order.
    pub(super) fn rest(
        &self,
        tree: &'a [AriaNode],
    ) -> impl DoubleEndedIterator<Item = Child<'a>> + '_ {
        self.without_held(tree.iter().map(Child::Node))
    }

    fn without_held(
        &self,
        children: impl DoubleEndedIterator<Item = Child<'a>>,
    ) -> impl DoubleEndedIterator<Item = Child<'a>> {
        children.filter(move |child| match child {
            Child::Node(node) => !self.held.contains(&ptr::from_ref(*node)),
            Child::Text(_) => true,
        })
    }
}

/// `onTopHead`: each layer's line, split at its line breaks, in layer order. Neo's formats (the
/// default format and `mode: 'full'`) print them before everything else and outside the cap.
pub(crate) fn on_top_head(snapshot: &Snapshot) -> impl Iterator<Item = &str> {
    snapshot
        .layers
        .iter()
        .flat_map(|layer| layer.line.split('\n'))
}

/// `isOnTopLine`: a line that says what is on top of the page, now or no longer. It is a fact of
/// the read, not the page: a delta leaves it out of both texts it compares, so a layer coming or
/// going alone is no change, and out of a whole page it gives back, whose head already holds it.
pub(crate) fn is_on_top_line(line: &str) -> bool {
    line.starts_with("# on top: ")
        || line.starts_with("#   its controls: ")
        || line.starts_with("# no longer on top: ")
}

/// `onTopDelta`: what a delta prints after its first line, the page's on-top lines as they are
/// now, then `# no longer on top: <role> "<name>" [ref=<ref>]` for each layer the earlier text
/// named whose ref no layer carries now.
///
/// The earlier layers are read back from `previous`, the text the caller was given, so a delta
/// can only say a layer is gone that the caller saw. neo-playwright words every layer line
/// (`action::on_top_line`), and this reads those words: a change to them is a change to both.
pub(crate) fn on_top_delta<'a>(previous: &'a str, snapshot: &'a Snapshot) -> Vec<Cow<'a, str>> {
    let mut lines: Vec<Cow<'a, str>> = on_top_head(snapshot).map(Cow::Borrowed).collect();
    for line in previous.split('\n') {
        if let Some(named) = named_layer(line)
            && !snapshot
                .layers
                .iter()
                .any(|layer| layer.r#ref == named.r#ref)
        {
            lines.push(Cow::Owned(format!("# no longer on top: {}", named.words)));
        }
    }
    lines
}

/// A layer as an on-top line names it: its words up to and with its ref, and the ref alone.
struct NamedLayer<'a> {
    /// `dialog "Cookies" [ref=e9]`.
    words: &'a str,
    r#ref: &'a str,
}

/// `/^# on top: (\S+ (?:"(?:[^"\\]|\\.)*" )?\[ref=([^\]]+)\])/.exec(line)`, character by
/// character: group 1 is the layer's words, group 2 its ref. A name is JSON-quoted, so a ref-shaped
/// word inside it never becomes the layer's ref.
///
/// The expression backtracks nowhere that could change its match, so each part is one scan:
/// - `\S+ `: a shorter run of non-space characters is followed by one of its own characters,
///   never by the space, so the run is the longest one. `\S` is JavaScript's whitespace set.
/// - the quoted name: the star reads each character alone, or a backslash with the character
///   after it (not a line terminator, which `.` refuses), so it can stop only at the first quote
///   it meets unescaped. When that quote is not followed by a space, or the name is not closed,
///   the group matches nothing and `\[` then fails on the opening quote.
/// - `[^\]]+\]`: the ref runs to the first `]`, and is not empty.
fn named_layer(line: &str) -> Option<NamedLayer<'_>> {
    let rest = line.strip_prefix("# on top: ")?;
    let role = rest.find(js::is_space).unwrap_or(rest.len());
    if role == 0 {
        return None;
    }
    let after_role = rest.get(role..)?.strip_prefix(' ')?;
    let after_name = match after_role.strip_prefix('"') {
        Some(name) => after_quoted_name(name)?,
        None => after_role,
    };
    let reference = after_name.strip_prefix("[ref=")?;
    let end = reference.find(']')?;
    if end == 0 {
        return None;
    }
    let words = rest.get(..rest.len() - reference.len() + end + 1)?;
    Some(NamedLayer {
        words,
        r#ref: reference.get(..end)?,
    })
}

/// `(?:[^"\\]|\\.)*" `: what follows a quoted name's closing quote and the space after it, from
/// just inside its opening quote; `None` when the name does not close so.
fn after_quoted_name(name: &str) -> Option<&str> {
    let mut characters = name.char_indices();
    while let Some((index, character)) = characters.next() {
        match character {
            '"' => return name.get(index + 1..)?.strip_prefix(' '),
            '\\' => match characters.next() {
                None | Some((_, '\n' | '\r' | '\u{2028}' | '\u{2029}')) => return None,
                Some(_) => {}
            },
            _ => {}
        }
    }
    None
}
