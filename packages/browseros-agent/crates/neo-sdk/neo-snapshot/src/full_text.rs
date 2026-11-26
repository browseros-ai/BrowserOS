//! `mode: 'full'`: Playwright's lines for every node of the marked tree under Neo's head lines,
//! cut by the same cap as Neo's default format. Ported from the TypeScript renderer's
//! `prelude/snapshot.ts` (`renderFullSnapshot`, `printed`) and `prelude/snapshot-inert.ts`
//! (`inertRoots`), whose output it must equal byte for byte.
//!
//! ```text
//!  SnapshotView ─▶ on-top lines, # refs missing:, # inert: N lines …   outside the cap
//!              └─▶ visit_tree_lines (playwright/, an iframe's fallback dropped)
//!                     ─▶ LineCap: block ends by indentation, kept blocks, sections, [ref= by text
//!                     ─▶ # partial: closed roots pinned, the cut offering mode: 'full'
//! ```
//!
//! Unlike Neo's default format, nothing is left out: inert content prints in place, and the head
//! only counts the lines it takes. The lines come from Playwright's printer, which says nothing
//! of what a line is, so the cap's facts are found again from the text, as `printed` finds them:
//! a node's block is the deeper-indented lines after its own, and a line has a ref when it says
//! `[ref=`, page text included.

use crate::aria_sink::EmittedNode;
use crate::aria_source::{AriaSource, SnapshotView, SourceChild};
use crate::cap::{
    LineCap, LineMark, LineMeta, LineSink, MaxTokens, Offer, Printed, Section, cap, join,
    partial_head,
};
use crate::js;
use crate::neo::{KEPT_ROLES, SECTION, on_top_head, section_label, unmarked_head};
use crate::playwright::{IframeText, TreeLine, visit_tree_lines, yaml_lines_source};

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
pub(crate) fn full_text_source<'a, S: AriaSource<'a>>(
    view: &SnapshotView<'a, S>,
    max_tokens: MaxTokens,
) -> String {
    let roots = inert_roots(&view.tree);
    let mut count = 0usize;
    if !roots.is_empty() {
        visit_tree_lines(&view.tree, roots, IframeText::Dropped, |_, _| {
            count += 1;
        });
    }
    let inert_head = (count > 0).then(|| {
        format!(
            "# inert: {count} lines of this page cannot be clicked or typed into now. \
             page.ariaSnapshot() lists what can."
        )
    });
    let pinned = partial_head(view.closed_roots);
    let body = if view.partial.is_empty() {
        stream_full_body(&view.tree, pinned.as_slice(), max_tokens.units())
    } else {
        cap(
            &[],
            pinned.as_slice(),
            &printed(yaml_lines_source(view, IframeText::Dropped)),
            max_tokens.units(),
            Offer::Full,
        )
    };
    let unmarked = unmarked_head(view.unmarked);
    join(
        on_top_head(view.layers)
            .chain(unmarked.as_deref())
            .chain(inert_head.as_deref())
            .chain(Some(body.as_str()).filter(|body| !body.is_empty())),
    )
}

enum OpenKind<'a> {
    Kept,
    Section {
        r#ref: String,
        role: &'a str,
        name: Option<&'a str>,
    },
}

struct OpenBlock<'a> {
    indent: usize,
    mark: LineMark,
    kind: OpenKind<'a>,
}

/// Streams `mode: 'full'` directly into [`LineCap`] when no partial-frame comments need insertion,
/// avoiding `Vec<TreeLine>` and `Vec<Printed>` allocation across the full tree.
fn stream_full_body<'a, S: AriaSource<'a>>(source: &S, pinned: &[String], limit: f64) -> String {
    let mut line_cap = LineCap::new(limit);
    let mut open: Vec<OpenBlock<'a>> = Vec::new();
    visit_tree_lines(
        source,
        source.roots(),
        IframeText::Dropped,
        |line, node: Option<EmittedNode<'a>>| {
            let indent = js::units(line) - js::units(js::trim_start(line));
            while let Some(top) = open.last() {
                if top.indent < indent {
                    break;
                }
                if let Some(closed) = open.pop() {
                    close_open_block(&mut line_cap, closed);
                }
            }
            let has_ref = line.contains("[ref=");
            let starts_kept = node.is_some_and(|emitted| KEPT_ROLES.contains(&emitted.role));
            let mark = line_cap.push_line(
                line,
                LineMeta {
                    has_ref,
                    starts_kept,
                },
            );
            if starts_kept {
                open.push(OpenBlock {
                    indent,
                    mark,
                    kind: OpenKind::Kept,
                });
            } else if let Some(emitted) = node
                && SECTION.contains(&emitted.role)
                && let Some(reference) = emitted.r#ref
            {
                open.push(OpenBlock {
                    indent,
                    mark,
                    kind: OpenKind::Section {
                        r#ref: reference.to_owned_string(),
                        role: emitted.role,
                        name: emitted.name,
                    },
                });
            }
        },
    );
    while let Some(closed) = open.pop() {
        close_open_block(&mut line_cap, closed);
    }
    line_cap.finish(&[], pinned, Offer::Full)
}

fn close_open_block(line_cap: &mut LineCap, block: OpenBlock<'_>) {
    match block.kind {
        OpenKind::Kept => line_cap.end_kept(block.mark),
        OpenKind::Section { r#ref, role, name } => {
            if line_cap.line_count() > block.mark.index + 1 {
                line_cap.end_section(block.mark, &r#ref, role, name);
            }
        }
    }
}

/// `inertRoots`: the nodes marked inert whose ancestors are not, in document order. The search
/// does not enter an inert node, so the roots are disjoint, and keeps its own stack, so a tree of
/// any depth is searched.
fn inert_roots<'a, S: AriaSource<'a>>(source: &S) -> Vec<S::Id> {
    let mut roots = Vec::new();
    let mut pending: Vec<S::Id> = source.roots().rev().collect();
    while let Some(id) = pending.pop() {
        let node = source.node(id);
        if node.inert == Some(true) {
            roots.push(id);
        } else if node.has_children {
            pending.extend(source.children(id).rev().filter_map(|child| match child {
                SourceChild::Node(child_id) => Some(child_id),
                SourceChild::Text(_) => None,
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
                Some(node) if KEPT_ROLES.contains(&node.role) => (Some(end), None),
                Some(node) if SECTION.contains(&node.role) && end > index + 1 => (
                    None,
                    node.r#ref.map(|reference| Section {
                        r#ref: reference.to_owned_string(),
                        label: section_label(&node),
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
