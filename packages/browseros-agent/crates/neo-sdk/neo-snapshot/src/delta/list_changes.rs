use crate::cap::join;
use crate::js;
use crate::neo::{is_on_top_line, is_unmarked_line};

use super::hunks::hunks;
use super::line_diff::{EditKind, line_diff};

/// The first line when nothing but read and size facts differ.
const NO_CHANGE: &str = "# no change since your last snapshot of this tab";

/// `listChanges`: the changes from `previous` to `current`, two texts rendered in one form:
/// `NO_CHANGE`, or the changes' first line and hunks; `None` when they would be longer than half
/// of `current`, too much to list.
///
/// "Half" measures the whole current text, its head lines included, so the same one-line change
/// is listed when the text carries a `# refs missing:` or `# not listed:` line and comes back as
/// the whole page when it does not. Lengths are UTF-16 units, and `x > length / 2` is written
/// `2 * x > length`, the same comparison in integers when the length is odd.
pub(super) fn list_changes(previous: &str, current: &str) -> Option<String> {
    let before: Vec<&str> = previous
        .split('\n')
        .filter(|line| !size_line(line))
        .collect();
    let after: Vec<&str> = current
        .split('\n')
        .filter(|line| !size_line(line))
        .collect();
    let current_units = js::units(current);
    let edits = line_diff(&before, &after, current_units)?;
    if edits.iter().all(|edit| edit.kind == EditKind::Same) {
        return Some(NO_CHANGE.to_owned());
    }
    let changes = join(hunks(&edits, &before, &after).iter().map(String::as_str));
    if 2 * js::units(&changes) > current_units {
        return None;
    }
    let added = edits
        .iter()
        .filter(|edit| edit.kind == EditKind::Added)
        .count();
    let removed = edits
        .iter()
        .filter(|edit| edit.kind == EditKind::Removed)
        .count();
    Some(format!(
        "# changes since your last snapshot of this tab: {added} added, {removed} removed\n\
         {changes}"
    ))
}

/// `sizeLine`: a fact of the read or of the text's size, not of the page, on either side of the
/// comparison: the on-top lines, `# refs missing:`, the notice, the cut line and `# not listed:`.
/// The closed-roots line and `mode: 'full'`'s `# inert:` are compared like any other line.
fn size_line(line: &str) -> bool {
    is_on_top_line(line)
        || is_unmarked_line(line)
        || line.starts_with("# text left out: ")
        || line.starts_with("# cut: ")
        || line.starts_with("# not listed: ")
}
