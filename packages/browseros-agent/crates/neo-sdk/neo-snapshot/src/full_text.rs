//! `mode: 'full'`: Playwright's lines for every node of the marked tree under Neo's head lines,
//! cut by the same cap as Neo's default format. Ported from the TypeScript renderer's
//! `prelude/snapshot.ts` (`renderFullSnapshot`, `printed`) and `prelude/snapshot-inert.ts`
//! (`inertRoots`), whose output it must equal byte for byte.
//!
//! ```text
//!  Snapshot ─▶ on-top lines, # refs missing:, # inert: N lines …   outside the cap
//!          └─▶ yaml_lines (playwright/, an iframe's fallback dropped)
//!                 ─▶ printed: block ends by indentation, kept blocks, sections, [ref= by text
//!                 ─▶ cap: # partial: closed roots pinned, the cut offering mode: 'full'
//! ```
//!
//! Unlike Neo's default format, nothing is left out: inert content prints in place, and the head
//! only counts the lines it takes. The lines come from Playwright's printer, which says nothing
//! of what a line is, so the cap's facts are found again from the text, as `printed` finds them:
//! a node's block is the deeper-indented lines after its own, and a line has a ref when it says
//! `[ref=`, page text included.

use crate::aria_node::{AriaChild, AriaNode};
use crate::cap::{MaxTokens, Offer, Printed, Section, cap, join, partial_head};
use crate::js;
use crate::neo::{KEPT_ROLES, SECTION, on_top_head, section_label, unmarked_head};
use crate::playwright::{IframeText, TreeLine, tree_lines, yaml_lines};
use crate::snapshot::Snapshot;

#[cfg(test)]
mod tests;

/// `renderFullSnapshot(result, maxTokens)`: in order, outside the cap, each layer's on-top lines,
/// the `# refs missing:` line when the check for clickable elements did not finish, and the
/// `# inert:` line when the page has inert roots; then the cap's text (the closed-roots line
/// pinned, then Playwright's lines with each `# partial:` comment under its iframe, cut at
/// `max_tokens` with an offer of that section in `mode: 'full'`). An empty cap's text prints no
/// line.
///
/// The `# inert:` count is the number of lines the inert roots print on their own, each at the
/// top level; the same lines print, indented, in the tree below it.
pub(crate) fn full_text(snapshot: &Snapshot, max_tokens: MaxTokens) -> String {
    let roots = inert_roots(&snapshot.tree);
    let count = if roots.is_empty() {
        0
    } else {
        tree_lines(roots, IframeText::Dropped).len()
    };
    let inert_head = (count > 0).then(|| {
        format!(
            "# inert: {count} lines of this page cannot be clicked or typed into now. \
             page.ariaSnapshot() lists what can."
        )
    });
    let pinned = partial_head(snapshot);
    let body = cap(
        &[],
        pinned.as_slice(),
        &printed(yaml_lines(snapshot, IframeText::Dropped)),
        max_tokens.units(),
        Offer::Full,
    );
    let unmarked = unmarked_head(snapshot);
    join(
        on_top_head(snapshot)
            .chain(unmarked.as_deref())
            .chain(inert_head.as_deref())
            .chain(Some(body.as_str()).filter(|body| !body.is_empty())),
    )
}

/// `inertRoots`: the nodes marked inert whose ancestors are not, in document order. The search
/// does not enter an inert node, so the roots are disjoint, and keeps its own stack, so a tree of
/// any depth is searched.
fn inert_roots(tree: &[AriaNode]) -> Vec<&AriaNode> {
    let mut roots = Vec::new();
    let mut pending: Vec<&AriaNode> = tree.iter().rev().collect();
    while let Some(node) = pending.pop() {
        if node.inert == Some(true) {
            roots.push(node);
        } else {
            pending.extend(node.children.iter().rev().filter_map(|child| match child {
                AriaChild::Node(node) => Some(&**node),
                AriaChild::Text(_) => None,
            }));
        }
    }
    roots
}

/// `printed`: what the cap needs of each line. A node's block is its line and every
/// deeper-indented line after it (its props, its text, its children and their `# partial:`
/// comments), found with one stack of the lines still open: a line closes every open line
/// indented as far as it or further. A kept block (a dialog, an alert) is never split and prints
/// past the cut; a section with a ref and a line under it is what the cut line can offer.
///
/// Indentation is JavaScript's: the UTF-16 units `trimStart` removes. A line has a ref when its
/// text holds `[ref=`, as the rule is worded, so page text that says so counts too.
fn printed(lines: Vec<TreeLine<'_>>) -> Vec<Printed> {
    let mut ends = vec![lines.len(); lines.len()];
    let mut open: Vec<(usize, usize)> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let indent = js::units(&line.text) - js::units(js::trim_start(&line.text));
        while let Some(&(opened, opened_indent)) = open.last() {
            if opened_indent < indent {
                break;
            }
            if let Some(end) = ends.get_mut(opened) {
                *end = index;
            }
            open.pop();
        }
        open.push((index, indent));
    }
    lines
        .into_iter()
        .zip(ends)
        .enumerate()
        .map(|(index, (TreeLine { text, node }, end))| {
            let has_ref = text.contains("[ref=");
            let (kept_end, section) = match node {
                Some(node) if KEPT_ROLES.contains(&node.role.as_str()) => (Some(end), None),
                Some(node) if SECTION.contains(&node.role.as_str()) && end > index + 1 => (
                    None,
                    node.r#ref.as_ref().map(|r#ref| Section {
                        r#ref: r#ref.clone(),
                        label: section_label(node),
                        end,
                    }),
                ),
                _ => (None, None),
            };
            Printed {
                text,
                has_ref,
                section,
                kept_end,
            }
        })
        .collect()
}
