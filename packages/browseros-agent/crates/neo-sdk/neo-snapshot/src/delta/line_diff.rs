use std::collections::HashMap;

use crate::js;

/// The most changed lines one comparison looks for. The search keeps, for each edit count `d` it
/// tries, the `2d + 1` diagonals it reached, so this bounds its memory (about 4 MB of `i32` at the
/// limit) and its time. A page that changed more than this is far past the half rule anyway, and
/// comes back whole.
const MAX_CHANGED_LINES: i32 = 1_000;

/// One line of the edit script. `before` and `after` are where the script stands in each text at
/// this line: the line's own index on its side, and on the other side the index of the next line
/// there.
#[derive(Clone, Copy, Debug)]
pub(super) struct Edit {
    pub(super) kind: EditKind,
    pub(super) before: usize,
    pub(super) after: usize,
}

/// The mark a hunk glues before the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EditKind {
    /// ` `: in both texts.
    Same,
    /// `-`: in the earlier text only.
    Removed,
    /// `+`: in the current text only.
    Added,
}

/// `diff`: the shortest edit script from `before` to `after` (Myers' O(ND) diff), or `None` when
/// it would hold more changed lines than [`MAX_CHANGED_LINES`] or than can fit in half of `limit`
/// units, each changed line costing at least its text, its mark and a newline.
///
/// The common head and tail are matched first, since a delta usually changes one place; a page
/// that changed nearly everywhere is refused by [`unmatched_cost`] before the search starts. That
/// early refusal and the half rule after the hunks are both kept, in that order: some pages are
/// decided by the first and not the second.
pub(super) fn line_diff(before: &[&str], after: &[&str], limit: usize) -> Option<Vec<Edit>> {
    let head = before
        .iter()
        .zip(after)
        .take_while(|(earlier, current)| earlier == current)
        .count();
    let rest_before = before.get(head..).unwrap_or_default();
    let rest_after = after.get(head..).unwrap_or_default();
    // Matched from the end of what the head left, so a line is never taken by both.
    let tail = rest_before
        .iter()
        .rev()
        .zip(rest_after.iter().rev())
        .take_while(|(earlier, current)| earlier == current)
        .count();
    let middle_before = rest_before
        .get(..rest_before.len() - tail)
        .unwrap_or_default();
    let middle_after = rest_after
        .get(..rest_after.len() - tail)
        .unwrap_or_default();
    // `unmatchedCost(…) > limit / 2`, in integers.
    if 2 * unmatched_cost(middle_before, middle_after) > limit {
        return None;
    }
    let changed = middle_before.len() + middle_after.len();
    // `Math.floor(limit / 2 / (shortest + 2))`. Integer division is the same number: `limit / 2`
    // is exact as a double, and the quotient's rounding could reach the next whole number only
    // for a text of 2^52 units or more.
    let fit = middle_before
        .iter()
        .chain(middle_after)
        .map(|line| js::units(line))
        .min()
        .map_or(0, |shortest| limit / (2 * (shortest + 2)));
    let most = i32::try_from(changed.min(fit))
        .unwrap_or(MAX_CHANGED_LINES)
        .min(MAX_CHANGED_LINES);
    let middle = middle_script(middle_before, middle_after, head, most)?;
    let mut edits = Vec::with_capacity(head + middle.len() + tail);
    edits.extend((0..head).map(|index| Edit {
        kind: EditKind::Same,
        before: index,
        after: index,
    }));
    edits.extend(middle);
    edits.extend((0..tail).map(|index| Edit {
        kind: EditKind::Same,
        before: head + middle_before.len() + index,
        after: head + middle_after.len() + index,
    }));
    Some(edits)
}

/// `unmatchedCost`: a lower bound on the changes' length. A middle line with no equal partner left
/// on the other side is printed as a change whatever the script, at its length plus mark and
/// newline. It costs one pass, where the search would spend [`MAX_CHANGED_LINES`] rounds to learn
/// the same.
fn unmatched_cost(before: &[&str], after: &[&str]) -> usize {
    let mut spare: HashMap<&str, usize> = HashMap::new();
    for line in after {
        *spare.entry(line).or_default() += 1;
    }
    let mut cost = 0;
    for line in before {
        match spare.get_mut(line) {
            Some(left) if *left > 0 => *left -= 1,
            _ => cost += js::units(line) + 2,
        }
    }
    for (line, left) in &spare {
        cost += left * (js::units(line) + 2);
    }
    cost
}

/// `middleScript`: Myers' greedy search over the middle, `offset` lines into both texts, keeping
/// each round's furthest reaches so the path can be walked back; `None` past `most` edits.
///
/// Round `d` touches only diagonals `-d` to `d` (`k = x - y`), so only those are kept,
/// `rounds[d][k + d]`. `reach` is the TypeScript's `Int32Array`, zeroed once and never reset:
/// every read is of a diagonal the previous round wrote (`k ± 1`, of the other parity), or of the
/// zero round 0 starts from.
///
/// The tie-break fixes the shape of every hunk: a strict `<` moves down (an insertion) only when
/// that reaches further, so a tie removes first.
fn middle_script(before: &[&str], after: &[&str], offset: usize, most: i32) -> Option<Vec<Edit>> {
    // A text of more than `i32::MAX` lines cannot exist in the script engine, whose strings are
    // shorter than that many units.
    let n = i32::try_from(before.len()).ok()?;
    let m = i32::try_from(after.len()).ok()?;
    // `reach[at(k)]` is the furthest x on diagonal k; `at(k) = k + most + 1` lies in 0..width for
    // every k in -(most + 1)..=most + 1.
    let width = 2 * most + 3;
    let mut reach = vec![0_i32; index(width)];
    let at = |k: i32| index(k + most + 1);
    let mut rounds: Vec<Vec<i32>> = Vec::new();
    for d in 0..=most {
        for k in (-d..=d).step_by(2) {
            // Down (an insertion) from diagonal k + 1, or right (a deletion) from k - 1.
            let down =
                k == -d || (k != d && reached(&reach, at(k - 1)) < reached(&reach, at(k + 1)));
            let mut x = if down {
                reached(&reach, at(k + 1))
            } else {
                reached(&reach, at(k - 1)) + 1
            };
            let mut y = x - k;
            while x < n && y < m && line(before, x) == line(after, y) {
                x += 1;
                y += 1;
            }
            if let Some(slot) = reach.get_mut(at(k)) {
                *slot = x;
            }
            if x >= n && y >= m {
                rounds.push(diagonals(&reach, at(-d), at(d)));
                return Some(walk_back(&rounds, n, m, offset));
            }
        }
        rounds.push(diagonals(&reach, at(-d), at(d)));
    }
    None
}

/// `walkBack`: the edit script from the rounds' reaches, walked from the end back to the start.
#[expect(
    clippy::many_single_char_names,
    reason = "Myers' own names, as snapshot-delta.ts has them: x and y in the two middles, k the \
              diagonal, n and m their lengths; the port is read beside it"
)]
fn walk_back(rounds: &[Vec<i32>], n: i32, m: i32, offset: usize) -> Vec<Edit> {
    let edit = |kind, x: i32, y: i32| Edit {
        kind,
        before: offset.saturating_add(index(x)),
        after: offset.saturating_add(index(y)),
    };
    let mut reversed: Vec<Edit> = Vec::new();
    let mut x = n;
    let mut y = m;
    let last = i32::try_from(rounds.len()).unwrap_or_default() - 1;
    for d in (1..=last).rev() {
        let k = x - y;
        // Round d - 1 kept diagonals -(d - 1) to d - 1; a step into round d came from one of them.
        let previous = rounds.get(index(d - 1)).map_or(&[][..], Vec::as_slice);
        let at = |diagonal: i32| index(diagonal + d - 1);
        let down =
            k == -d || (k != d && reached(previous, at(k - 1)) < reached(previous, at(k + 1)));
        let from_k = if down { k + 1 } else { k - 1 };
        let from_x = reached(previous, at(from_k));
        let from_y = from_x - from_k;
        while x > (if down { from_x } else { from_x + 1 })
            && y > (if down { from_y + 1 } else { from_y })
        {
            x -= 1;
            y -= 1;
            reversed.push(edit(EditKind::Same, x, y));
        }
        if down {
            y -= 1;
            reversed.push(edit(EditKind::Added, x, y));
        } else {
            x -= 1;
            reversed.push(edit(EditKind::Removed, x, y));
        }
    }
    // Round 0 is only the snake down the main diagonal. Kept as the TypeScript has it, though no
    // input reaches it: the head was matched first, so that snake is empty.
    while x > 0 && y > 0 {
        x -= 1;
        y -= 1;
        reversed.push(edit(EditKind::Same, x, y));
    }
    reversed.reverse();
    reversed
}

/// `reach.slice(from, to + 1)`: the diagonals one round kept.
fn diagonals(reach: &[i32], from: usize, to: usize) -> Vec<i32> {
    reach.get(from..=to).unwrap_or_default().to_vec()
}

/// `reach[at]`. Every index the search reads lies within the round (see [`middle_script`]), so the
/// zero for one outside it is never read.
fn reached(reach: &[i32], at: usize) -> i32 {
    reach.get(at).copied().unwrap_or_default()
}

/// Line `at` of a middle, `None` outside it.
fn line<'a>(lines: &[&'a str], at: i32) -> Option<&'a str> {
    usize::try_from(at)
        .ok()
        .and_then(|at| lines.get(at))
        .copied()
}

/// A position or a diagonal's slot as an index; never negative where the search uses it, and
/// `usize::MAX`, outside every slice, if it were.
fn index(value: i32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}
