use crate::js;

use super::line_diff::{Edit, EditKind};

/// Unchanged lines printed on each side of a change.
const CONTEXT: usize = 3;

/// `hunks`: the changes in hunks, each one `@@` line naming the block it is in, then its lines with
/// `+`, `-` or a space glued in front, [`CONTEXT`] unchanged lines on each side. Changes whose
/// context would touch or overlap, six unchanged lines apart or closer, share one hunk.
pub(super) fn hunks(edits: &[Edit], before: &[&str], after: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut index = 0;
    while let Some(edit) = edits.get(index) {
        if edit.kind == EditKind::Same {
            index += 1;
            continue;
        }
        let start = index.saturating_sub(CONTEXT);
        // Extend over changes until a run of more than twice the context separates the next one.
        let mut end = index;
        let mut quiet = 0;
        for (next, edit) in edits.iter().enumerate().skip(index) {
            if edit.kind == EditKind::Same {
                quiet += 1;
                if quiet > 2 * CONTEXT {
                    break;
                }
            } else {
                quiet = 0;
                end = next;
            }
        }
        let stop = edits.len().min(end + 1 + CONTEXT);
        let lines = edits.get(start..stop).unwrap_or_default();
        out.push(block_line(lines, before, after));
        for edit in lines {
            let mark = match edit.kind {
                EditKind::Same => ' ',
                EditKind::Removed => '-',
                EditKind::Added => '+',
            };
            out.push(format!("{mark}{}", edit_text(edit, before, after)));
        }
        index = stop;
    }
    out
}

/// `blockLine`: `@@` and the block the hunk's first change is in, the nearest line above that
/// change with less indentation, without its indentation, its list dash and its trailing colon
/// (`@@ main [ref=e293]`), so the agent can snapshot that block by its ref. Looked up in the
/// current text, or in the earlier one for a hunk that only removes lines; `@@` alone at the top
/// level.
fn block_line(lines: &[Edit], before: &[&str], after: &[&str]) -> String {
    // Kept as the TypeScript has it, though no input reaches it: a hunk starts at a change.
    let Some(first) = lines.iter().find(|edit| edit.kind != EditKind::Same) else {
        return "@@".to_owned();
    };
    let removes_only = lines.iter().all(|edit| edit.kind != EditKind::Added);
    let (text, position) = if removes_only {
        (before, first.before)
    } else {
        (after, first.after)
    };
    let depth = indent_of(edit_text(first, before, after));
    for line in text.iter().take(position).rev() {
        if indent_of(line) < depth {
            // `.trimStart().replace(/^- /, '').replace(/:$/, '')`: `$` without the `m` flag is
            // the end of the line only.
            let name = js::trim_start(line);
            let name = name.strip_prefix("- ").unwrap_or(name);
            let name = name.strip_suffix(':').unwrap_or(name);
            return format!("@@ {name}");
        }
    }
    "@@".to_owned()
}

/// The line an edit prints: the current text's for an added line, the earlier text's otherwise.
fn edit_text<'a>(edit: &Edit, before: &[&'a str], after: &[&'a str]) -> &'a str {
    let line = match edit.kind {
        EditKind::Added => after.get(edit.after),
        EditKind::Same | EditKind::Removed => before.get(edit.before),
    };
    line.copied().unwrap_or_default()
}

/// `indentOf`, `line.length - line.trimStart().length`: the JavaScript whitespace a line starts
/// with, in UTF-16 units. Counted from the start alone, the same number without measuring the
/// whole line: a hunk in a long section looks back over every line above it for its block.
fn indent_of(line: &str) -> usize {
    line.chars()
        .take_while(|character| js::is_space(*character))
        .map(char::len_utf16)
        .sum()
}
