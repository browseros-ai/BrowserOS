use std::borrow::Cow;
use std::collections::HashSet;
use std::hash::Hash;

use crate::aria_source::{AriaSource, SnapshotView, SourceChild};
use crate::js;
use crate::snapshot::Layer;

use super::hidden::Hidden;

/// What the whole-view layers hold (`onTopContent` in `snapshot-on-top.ts`): Neo's default
/// format prints it first, then the rest of the page without it.
///
/// The TypeScript projects a copy of the tree with those nodes removed and finds them again by
/// object identity. Here the walk borrows the tree as it is and leaves out the held nodes as it
/// meets them, by identity key: every child list the walk, the omissions and the label rules read
/// goes through [`OnTop::children`], so each sees the children the projection would hold. The
/// search never enters a held node or an inert one, so no held node lies inside either, and a
/// printer walking inside them sees the same children with or without the filter.
#[derive(Debug)]
pub(super) struct OnTop<Id, Key> {
    /// Each whole-view layer's block in layer order: the nodes marked `onTop` with its ref. Two
    /// layers with one ref each print that block, as the TypeScript's map does.
    pub(super) first: Vec<Held<Id>>,
    /// The same nodes, by identity key in the source.
    held: HashSet<Key>,
}

/// A node a whole-view layer holds, printed first and so outside every line around it in the page.
#[derive(Clone, Copy, Debug)]
pub(super) struct Held<Id> {
    pub(super) id: Id,
    /// [`Hidden::Carried`] when an `aria-hidden` node is above it in its document, whose line no
    /// longer encloses it; otherwise [`Hidden::No`].
    pub(super) hidden: Hidden,
}

impl<Id, Key> Default for OnTop<Id, Key> {
    fn default() -> Self {
        Self {
            first: Vec::new(),
            held: HashSet::new(),
        }
    }
}

impl<Id: Copy, Key: Copy + Eq + Hash> OnTop<Id, Key> {
    pub(super) fn of<'a, S: AriaSource<'a, Id = Id, Key = Key>>(
        view: &SnapshotView<'a, S>,
    ) -> Self {
        let layers: Vec<&str> = view
            .layers
            .iter()
            .filter(|layer| layer.whole_view)
            .map(|layer| layer.r#ref.as_str())
            .collect();
        if layers.is_empty() {
            return Self::default();
        }
        // One block per distinct ref, as a map keyed by ref holds it.
        let mut blocks: Vec<(&str, Vec<Held<Id>>)> = Vec::new();
        for layer in &layers {
            if !blocks.iter().any(|(held_by, _)| held_by == layer) {
                blocks.push((layer, Vec::new()));
            }
        }
        let mut held = HashSet::new();
        // Depth first in document order, on a stack of its own, each node with what it inherits
        // of an `aria-hidden` node above it once no line encloses it.
        let mut pending: Vec<Held<Id>> = view
            .tree
            .roots()
            .rev()
            .map(|id| Held {
                id,
                hidden: Hidden::No,
            })
            .collect();
        while let Some(candidate) = pending.pop() {
            let Held { id, hidden } = candidate;
            let node = view.tree.node(id);
            if node.inert == Some(true) {
                continue;
            }
            // `node.onTop ? front.get(node.onTop) : undefined`: an empty mark selects nothing.
            let block = node
                .on_top
                .filter(|mark| !mark.is_empty())
                .and_then(|mark| blocks.iter_mut().find(|(held_by, _)| *held_by == mark));
            if let Some((_, block)) = block {
                held.insert(view.tree.key(id));
                block.push(candidate);
            } else if node.has_children {
                let below = hidden.beside(&node);
                pending.extend(
                    view.tree
                        .children(id)
                        .rev()
                        .filter_map(|child| match child {
                            SourceChild::Node(child_id) => Some(Held {
                                id: child_id,
                                hidden: below,
                            }),
                            SourceChild::Text(_) => None,
                        }),
                );
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

    /// `id`'s children without the held nodes, in document order.
    pub(super) fn children<'a, 's, S: AriaSource<'a, Id = Id, Key = Key>>(
        &'s self,
        source: &'s S,
        id: Id,
    ) -> impl DoubleEndedIterator<Item = SourceChild<'a, Id>> + 's {
        self.without_held(source, source.children(id))
    }

    /// The page's top-level nodes without the held nodes, in document order.
    pub(super) fn rest<'a, 's, S: AriaSource<'a, Id = Id, Key = Key>>(
        &'s self,
        source: &'s S,
    ) -> impl DoubleEndedIterator<Item = SourceChild<'a, Id>> + 's {
        self.without_held(source, source.roots().map(SourceChild::Node))
    }

    fn without_held<'a, 's, S: AriaSource<'a, Id = Id, Key = Key>>(
        &'s self,
        source: &'s S,
        children: impl DoubleEndedIterator<Item = SourceChild<'a, Id>> + 's,
    ) -> impl DoubleEndedIterator<Item = SourceChild<'a, Id>> + 's {
        let has_held = !self.held.is_empty();
        children.filter(move |child| match child {
            SourceChild::Node(id) => !has_held || !self.held.contains(&source.key(*id)),
            SourceChild::Text(_) => true,
        })
    }
}

/// `onTopHead`: each layer's line, split at its line breaks, in layer order. Neo's formats (the
/// default format and `mode: 'full'`) print them before everything else and outside the cap.
pub(crate) fn on_top_head(layers: &[Layer]) -> impl Iterator<Item = &str> {
    layers.iter().flat_map(|layer| layer.line.split('\n'))
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
pub(crate) fn on_top_delta<'a>(previous: &'a str, layers: &'a [Layer]) -> Vec<Cow<'a, str>> {
    let mut lines: Vec<Cow<'a, str>> = on_top_head(layers).map(Cow::Borrowed).collect();
    for line in previous.split('\n') {
        if let Some(named) = named_layer(line)
            && !layers.iter().any(|layer| layer.r#ref == named.r#ref)
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
