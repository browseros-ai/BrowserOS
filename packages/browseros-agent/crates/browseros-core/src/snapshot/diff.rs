use super::coverage::{FrameError, FrameSpan, SnapshotCoverage, compared_text, unevenly_read};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotDiff {
    pub text: String,
    /// Lines the edit script adds and removes. Exact, except for a stretch too changed to align
    /// (see [`diff_snapshots`]), which counts every line of it as removed and every line as added.
    pub added: usize,
    pub removed: usize,
    pub changed: bool,
    /// When true, `text` is the full current snapshot for a known URL change, not a line diff,
    /// and `added` and `removed` are zero.
    pub url_changed: bool,
    pub before_url: Option<String>,
    pub after_url: Option<String>,
    /// Whether any control moved, appeared or went away.
    ///
    /// A narrower question than `changed`, which is a text diff and so reads
    /// a ticking countdown or a rotating ad as the page having changed. None
    /// when there was no baseline to compare an arrangement against, or when the two captures
    /// did not read the same frames.
    pub arrangement_changed: Option<bool>,
    /// What the current capture read, as its snapshot reports it.
    pub coverage: SnapshotCoverage,
    /// Frames left out of the comparison because the baseline and the current capture read them
    /// differently. Their lines are neither removed nor added: one side could not read them.
    pub uncompared: Vec<FrameError>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffOptions {
    pub context_radius: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotObservation {
    pub text: String,
    pub url: Option<String>,
    pub coverage: SnapshotCoverage,
    /// Where each child frame the capture read sits in `text`.
    pub frames: Vec<FrameSpan>,
}

/// The largest LCS table built, in cells of `(before + 1) × (after + 1)` past the common head.
/// That is 32 MiB of `usize`, a 2,047-line square. Unbounded, one changed line on a 285k-line page
/// asked for about 605 GiB.
const MAX_TABLE_CELLS: usize = 1 << 22;

/// The most removed plus added lines that Myers' search looks for. The walk back keeps each
/// round's `2d + 1` reaches, about 8 MiB at this limit, and the search stops after this many rounds.
const MAX_SEARCHED_EDITS: usize = 1_000;

/// One line of the edit script, borrowing its text from the snapshot it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaggedLine<'a> {
    Same(&'a str),
    Removed(&'a str),
    Added(&'a str),
    /// A stretch that differs in more than [`MAX_SEARCHED_EDITS`] lines. Its `removed` baseline
    /// lines became `added` current ones, and it prints as one line instead of aligned.
    Unaligned {
        removed: usize,
        added: usize,
    },
}

/// The line diff from `before` to `after`. Each change prints with `opts.context_radius` lines of
/// context (3 by default), and an `N added, M removed` line comes last.
///
/// Time and memory stay bounded on any page. The lines both texts start with are matched first.
/// When the rest fits `MAX_TABLE_CELLS`, the LCS table aligns it, and the text is what this diff
/// has always printed. A bigger rest also matches the lines both texts end with, and Myers' search
/// aligns the middle between them. Both find a shortest script, so the counts agree, but they can
/// break a tie between repeated lines differently. A middle with more than `MAX_SEARCHED_EDITS`
/// changed lines prints as one `…` line that says how many lines became how many.
pub fn diff_snapshots(before: &str, after: &str, opts: DiffOptions) -> SnapshotDiff {
    if before == after {
        return SnapshotDiff::default();
    }

    let tagged = diff_lines(before, after);
    let mut added = 0;
    let mut removed = 0;
    for line in &tagged {
        match line {
            TaggedLine::Same(_) => {}
            TaggedLine::Removed(_) => removed += 1,
            TaggedLine::Added(_) => added += 1,
            TaggedLine::Unaligned {
                removed: gone,
                added: new,
            } => {
                removed += gone;
                added += new;
            }
        }
    }

    let body = collapse(&tagged, opts.context_radius.unwrap_or(3));
    SnapshotDiff {
        text: format!("{body}\n{added} added, {removed} removed"),
        added,
        removed,
        changed: true,
        url_changed: false,
        before_url: None,
        after_url: None,
        // Filled in by the caller that holds both arrangements. A text diff
        // on its own cannot answer the structural question.
        arrangement_changed: None,
        coverage: SnapshotCoverage::default(),
        uncompared: Vec::new(),
    }
}

pub fn diff_snapshot_observations(
    before: Option<&SnapshotObservation>,
    after: &SnapshotObservation,
    opts: DiffOptions,
) -> SnapshotDiff {
    let before_url = before.and_then(|before| before.url.as_deref());
    let after_url = after.url.as_deref();
    if is_known_url(before_url) && is_known_url(after_url) && before_url != after_url {
        return SnapshotDiff {
            text: after.text.clone(),
            added: 0,
            removed: 0,
            changed: true,
            url_changed: true,
            before_url: before_url.map(ToString::to_string),
            after_url: after_url.map(ToString::to_string),
            // A different url is a different page by any instrument.
            arrangement_changed: Some(true),
            coverage: after.coverage.clone(),
            uncompared: Vec::new(),
        };
    }

    let uncompared = before
        .map(|before| {
            unevenly_read(
                (&before.coverage, &before.frames),
                (&after.coverage, &after.frames),
            )
        })
        .unwrap_or_default();
    let before_text = before
        .map(|before| compared_text(&before.text, &before.frames, &uncompared))
        .unwrap_or_default();
    let after_text = compared_text(&after.text, &after.frames, &uncompared);
    let mut diff = diff_snapshots(&before_text, &after_text, opts);
    if is_known_url(after_url) {
        diff.after_url = after_url.map(ToString::to_string);
    }
    diff.coverage = after.coverage.clone();
    diff.uncompared = uncompared;
    diff
}

fn is_known_url(url: Option<&str>) -> bool {
    url.is_some_and(|url| !url.is_empty() && url != "unknown")
}

fn split_lines(value: &str) -> Vec<&str> {
    if value.is_empty() {
        Vec::new()
    } else {
        value.split('\n').collect()
    }
}

fn diff_lines<'a>(before: &'a str, after: &'a str) -> Vec<TaggedLine<'a>> {
    let before_lines = split_lines(before);
    let after_lines = split_lines(after);
    // Matching the head first leaves the output unchanged. The table's walk takes equal lines
    // together, and no cell past the head depends on it. The tail is matched only past the table
    // bound, because the walk can take a tail line early, ahead of an added line equal to it.
    let head = before_lines
        .iter()
        .zip(&after_lines)
        .take_while(|(earlier, current)| earlier == current)
        .count();
    let mut tagged: Vec<_> = before_lines[..head]
        .iter()
        .map(|line| TaggedLine::Same(line))
        .collect();
    let rest_before = &before_lines[head..];
    let rest_after = &after_lines[head..];
    if (rest_before.len() + 1).saturating_mul(rest_after.len() + 1) <= MAX_TABLE_CELLS {
        tagged.extend(table_script(rest_before, rest_after));
        return tagged;
    }

    let tail = rest_before
        .iter()
        .rev()
        .zip(rest_after.iter().rev())
        .take_while(|(earlier, current)| earlier == current)
        .count();
    let middle_before = &rest_before[..rest_before.len() - tail];
    let middle_after = &rest_after[..rest_after.len() - tail];
    match shortest_script(middle_before, middle_after, MAX_SEARCHED_EDITS) {
        Some(script) => tagged.extend(script),
        None => tagged.push(TaggedLine::Unaligned {
            removed: middle_before.len(),
            added: middle_after.len(),
        }),
    }
    tagged.extend(
        rest_before[middle_before.len()..]
            .iter()
            .map(|line| TaggedLine::Same(line)),
    );
    tagged
}

/// The script from walking the LCS table forward: equal lines go together, and a tie removes
/// before it adds.
fn table_script<'a>(before_lines: &[&'a str], after_lines: &[&'a str]) -> Vec<TaggedLine<'a>> {
    let table = build_lcs_table(before_lines, after_lines);
    let mut tagged = Vec::new();
    let mut i = 0;
    let mut j = 0;

    while i < before_lines.len() && j < after_lines.len() {
        if before_lines[i] == after_lines[j] {
            tagged.push(TaggedLine::Same(before_lines[i]));
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            tagged.push(TaggedLine::Removed(before_lines[i]));
            i += 1;
        } else {
            tagged.push(TaggedLine::Added(after_lines[j]));
            j += 1;
        }
    }

    tagged.extend(
        before_lines[i..]
            .iter()
            .map(|line| TaggedLine::Removed(line)),
    );
    tagged.extend(after_lines[j..].iter().map(|line| TaggedLine::Added(line)));
    tagged
}

fn build_lcs_table(before: &[&str], after: &[&str]) -> Vec<Vec<usize>> {
    let mut table = vec![vec![0; after.len() + 1]; before.len() + 1];
    for i in (0..before.len()).rev() {
        for j in (0..after.len()).rev() {
            table[i][j] = if before[i] == after[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    table
}

/// Myers' greedy search for a shortest script, or `None` past `max_edits` removed and added lines.
///
/// `reach[slot(k)]` is the furthest `x` reached on diagonal `k = x - y`. Round `d` reads only
/// the diagonals round `d - 1` wrote, so one array serves every round, and a copy of each round's
/// `-d..=d` is kept for the walk back. A tie moves right, so a changed line is removed before
/// its replacement is added, as the table's walk orders it.
fn shortest_script<'a>(
    before: &[&'a str],
    after: &[&'a str],
    max_edits: usize,
) -> Option<Vec<TaggedLine<'a>>> {
    let (n, m) = (before.len() as isize, after.len() as isize);
    let most = max_edits as isize;
    let slot = |k: isize| (k + most + 1) as usize;
    let mut reach = vec![0_isize; slot(most + 1) + 1];
    let mut rounds = Vec::new();
    for d in 0..=most {
        for k in (-d..=d).step_by(2) {
            let down = k == -d || (k != d && reach[slot(k - 1)] < reach[slot(k + 1)]);
            let mut x = if down {
                reach[slot(k + 1)]
            } else {
                reach[slot(k - 1)] + 1
            };
            let mut y = x - k;
            while x < n && y < m && before[x as usize] == after[y as usize] {
                x += 1;
                y += 1;
            }
            reach[slot(k)] = x;
            if x >= n && y >= m {
                rounds.push(reach[slot(-d)..=slot(d)].to_vec());
                return Some(walk_back(&rounds, before, after));
            }
        }
        rounds.push(reach[slot(-d)..=slot(d)].to_vec());
    }
    None
}

/// The script that [`shortest_script`]'s rounds found, walked back from the end of both middles.
fn walk_back<'a>(
    rounds: &[Vec<isize>],
    before: &[&'a str],
    after: &[&'a str],
) -> Vec<TaggedLine<'a>> {
    let mut x = before.len() as isize;
    let mut y = after.len() as isize;
    let mut reversed = Vec::new();
    for d in (1..rounds.len() as isize).rev() {
        let previous = &rounds[(d - 1) as usize];
        let reached = |k: isize| previous[(k + d - 1) as usize];
        let k = x - y;
        let down = k == -d || (k != d && reached(k - 1) < reached(k + 1));
        let from_k = if down { k + 1 } else { k - 1 };
        let from_x = reached(from_k);
        let from_y = from_x - from_k;
        let (edit_x, edit_y) = if down {
            (from_x, from_y + 1)
        } else {
            (from_x + 1, from_y)
        };
        while x > edit_x && y > edit_y {
            x -= 1;
            y -= 1;
            reversed.push(TaggedLine::Same(before[x as usize]));
        }
        reversed.push(if down {
            TaggedLine::Added(after[from_y as usize])
        } else {
            TaggedLine::Removed(before[from_x as usize])
        });
        (x, y) = (from_x, from_y);
    }
    while x > 0 && y > 0 {
        x -= 1;
        y -= 1;
        reversed.push(TaggedLine::Same(before[x as usize]));
    }
    reversed.reverse();
    reversed
}

fn collapse(tagged: &[TaggedLine], radius: usize) -> String {
    let mut keep = vec![false; tagged.len()];
    for (index, line) in tagged.iter().enumerate() {
        if matches!(line, TaggedLine::Same(_)) {
            continue;
        }
        let lo = index.saturating_sub(radius);
        let hi = (index + radius).min(tagged.len().saturating_sub(1));
        for item in keep.iter_mut().take(hi + 1).skip(lo) {
            *item = true;
        }
    }

    let mut out = Vec::new();
    let mut prev = None;
    for (index, line) in tagged.iter().enumerate() {
        if !keep[index] {
            continue;
        }
        if prev.is_some_and(|prev| index - prev > 1) {
            out.push("…".to_string());
        }
        out.push(match line {
            TaggedLine::Same(text) => format!("  {}", strip_bullet(text)),
            TaggedLine::Removed(text) => format!("- {}", strip_bullet(text)),
            TaggedLine::Added(text) => format!("+ {}", strip_bullet(text)),
            TaggedLine::Unaligned { removed, added } => format!(
                "… {removed} lines here became {added} lines. More than {MAX_SEARCHED_EDITS} lines changed, too many to compare line by line. Take a snapshot to see them."
            ),
        });
        prev = Some(index);
    }
    out.join("\n")
}

fn strip_bullet(line: &str) -> String {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("- ") {
        let leading = line.len() - trimmed.len();
        format!("{}{}", &line[..leading], rest)
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DiffOptions, SnapshotDiff, SnapshotObservation, diff_snapshot_observations, diff_snapshots,
    };
    use crate::{
        FrameId,
        snapshot::{
            FrameError, FrameOwner, FrameReadReason, FrameReadStage, FrameSpan, SnapshotCoverage,
        },
    };

    fn frame_b_owner() -> FrameOwner {
        FrameOwner {
            parent: None,
            backend_node_id: 20,
        }
    }

    fn frame_b_error(stage: FrameReadStage) -> FrameError {
        FrameError {
            frame_id: Some(FrameId("b".to_string())),
            owner: Some(frame_b_owner()),
            stage,
            reason: FrameReadReason::Timeout,
        }
    }

    /// The main frame's `lines`, then frame b's iframe line followed by its contents when read.
    fn framed(lines: &[&str], b: Option<&[&str]>, errors: Vec<FrameError>) -> SnapshotObservation {
        let mut text = lines.iter().map(ToString::to_string).collect::<Vec<_>>();
        text.push("- iframe".to_string());
        let mut frames = Vec::new();
        if let Some(b) = b {
            let start = text.len();
            text.extend(b.iter().map(|line| format!("  {line}")));
            frames.push(FrameSpan {
                frame_id: Some(FrameId("b".to_string())),
                owner: frame_b_owner(),
                lines: start..text.len(),
            });
        }
        SnapshotObservation {
            text: text.join("\n"),
            url: Some("https://example.com/".to_string()),
            coverage: SnapshotCoverage {
                frame_errors: errors,
            },
            frames,
        }
    }

    const SAVE: &str = "- button \"Save\" [ref=e1]";
    const B: &str = "- button \"B\" [ref=e2]";

    #[test]
    fn an_unreadable_frame_is_unknown_while_the_rest_of_the_page_still_diffs() {
        let before = framed(&[SAVE], Some(&[B]), Vec::new());
        let after = framed(
            &[SAVE, "- button \"New\" [ref=e3]"],
            None,
            vec![frame_b_error(FrameReadStage::Ax)],
        );

        let diff = diff_snapshot_observations(Some(&before), &after, DiffOptions::default());

        assert_eq!((diff.added, diff.removed), (1, 0), "{}", diff.text);
        assert!(diff.text.contains("+ button \"New\" [ref=e3]"));
        assert!(!diff.text.contains("button \"B\""));
        assert_eq!(diff.uncompared, vec![frame_b_error(FrameReadStage::Ax)]);
        assert_eq!(diff.coverage, after.coverage);
    }

    #[test]
    fn a_frame_that_reads_again_is_not_new() {
        let before = framed(&[SAVE], None, vec![frame_b_error(FrameReadStage::Session)]);
        let after = framed(&[SAVE], Some(&[B]), Vec::new());

        let diff = diff_snapshot_observations(Some(&before), &after, DiffOptions::default());

        assert!(!diff.changed, "{}", diff.text);
        assert!(!diff.coverage.is_partial());
        assert_eq!(
            diff.uncompared,
            vec![frame_b_error(FrameReadStage::Session)]
        );
    }

    #[test]
    fn a_frame_without_an_id_is_matched_by_its_iframe_element() {
        let before = framed(&[SAVE], Some(&[B]), Vec::new());
        let unidentified = FrameError {
            frame_id: None,
            stage: FrameReadStage::Identify,
            ..frame_b_error(FrameReadStage::Identify)
        };
        let after = framed(&[SAVE], None, vec![unidentified.clone()]);

        let diff = diff_snapshot_observations(Some(&before), &after, DiffOptions::default());

        assert!(!diff.changed, "{}", diff.text);
        assert_eq!(diff.uncompared, vec![unidentified]);
    }

    #[test]
    fn a_frame_read_alike_on_both_sides_still_diffs() {
        let degraded = || vec![frame_b_error(FrameReadStage::CursorScan)];
        let before = framed(&[SAVE], Some(&[B]), degraded());
        let after = framed(&[SAVE], Some(&["- button \"B2\" [ref=e2]"]), degraded());

        let diff = diff_snapshot_observations(Some(&before), &after, DiffOptions::default());

        assert_eq!((diff.added, diff.removed), (1, 1), "{}", diff.text);
        assert!(diff.uncompared.is_empty());
        assert!(diff.coverage.is_partial());
    }

    #[test]
    fn a_frame_unread_before_and_gone_now_is_compared_as_usual() {
        let before = framed(&[SAVE], None, vec![frame_b_error(FrameReadStage::Ax)]);
        let after = SnapshotObservation {
            text: SAVE.to_string(),
            url: Some("https://example.com/".to_string()),
            ..SnapshotObservation::default()
        };

        let diff = diff_snapshot_observations(Some(&before), &after, DiffOptions::default());

        assert_eq!((diff.added, diff.removed), (0, 1), "{}", diff.text);
        assert!(diff.text.contains("- iframe"));
        assert!(diff.uncompared.is_empty());
    }

    #[test]
    fn an_unevenly_read_main_frame_leaves_nothing_compared() {
        let before = framed(&[SAVE], Some(&[B]), Vec::new());
        let main_degraded = FrameError {
            frame_id: None,
            owner: None,
            stage: FrameReadStage::CursorScan,
            reason: FrameReadReason::BrowserError,
        };
        let after = framed(
            &["- generic \"Was a target\""],
            Some(&[B]),
            vec![main_degraded.clone()],
        );

        let diff = diff_snapshot_observations(Some(&before), &after, DiffOptions::default());

        assert!(!diff.changed, "{}", diff.text);
        assert_eq!(diff.uncompared, vec![main_degraded]);
    }

    /// `n` lines, each unique, as refs make most snapshot lines.
    fn numbered(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| format!("- link \"comment {i}\" [ref=e{i}]"))
            .collect()
    }

    /// Line `i` of [`numbered`] as the diff prints it in context.
    fn same(i: usize) -> String {
        format!("  link \"comment {i}\" [ref=e{i}]")
    }

    /// One changed line in 285,000, the size of a long comment thread. A table over the whole
    /// page would be about 605 GiB, which no `act` survives.
    #[test]
    fn a_huge_page_with_one_changed_line_diffs_in_bounded_memory() {
        let lines = numbered(285_000);
        let mut after = lines.clone();
        after[142_500] = "- button \"Reply\" [ref=e900000]".to_string();

        let diff = diff_snapshots(&lines.join("\n"), &after.join("\n"), DiffOptions::default());

        let mut want: Vec<String> = (142_497..142_500).map(same).collect();
        want.push("- link \"comment 142500\" [ref=e142500]".to_string());
        want.push("+ button \"Reply\" [ref=e900000]".to_string());
        want.extend((142_501..142_504).map(same));
        want.push("1 added, 1 removed".to_string());
        assert_eq!(diff.text, want.join("\n"));
    }

    /// Changes at both ends leave a middle too long for the table, so Myers' search aligns it.
    #[test]
    fn far_apart_changes_on_a_big_page_each_get_their_own_hunk() {
        let lines = numbered(3_000);
        let mut after = lines.clone();
        after[10] = "- button \"Reply\" [ref=e900000]".to_string();
        after.insert(1_501, "- button \"New\" [ref=e900001]".to_string());
        after.remove(2_991);

        let diff = diff_snapshots(&lines.join("\n"), &after.join("\n"), DiffOptions::default());

        let mut want: Vec<String> = (7..10).map(same).collect();
        want.push("- link \"comment 10\" [ref=e10]".to_string());
        want.push("+ button \"Reply\" [ref=e900000]".to_string());
        want.extend((11..14).map(same));
        want.push("…".to_string());
        want.extend((1_498..1_501).map(same));
        want.push("+ button \"New\" [ref=e900001]".to_string());
        want.extend((1_501..1_504).map(same));
        want.push("…".to_string());
        want.extend((2_987..2_990).map(same));
        want.push("- link \"comment 2990\" [ref=e2990]".to_string());
        want.extend((2_991..2_994).map(same));
        want.push("2 added, 2 removed".to_string());
        assert_eq!(diff.text, want.join("\n"));
        assert_eq!((diff.added, diff.removed), (2, 2));
    }

    /// Every second line of 2,000 changed: 2,000 changed lines, past what the search looks for.
    #[test]
    fn a_big_page_changed_in_too_many_places_reports_the_stretch_as_one_line() {
        let lines = numbered(3_000);
        let mut after = lines.clone();
        for i in (10..2_010).step_by(2) {
            after[i] = format!("- button \"Changed {i}\" [ref=e{}]", 900_000 + i);
        }

        let diff = diff_snapshots(&lines.join("\n"), &after.join("\n"), DiffOptions::default());

        let mut want: Vec<String> = (7..10).map(same).collect();
        want.push(
            "… 1999 lines here became 1999 lines. More than 1000 lines changed, too many to \
             compare line by line. Take a snapshot to see them."
                .to_string(),
        );
        want.extend((2_009..2_012).map(same));
        want.push("1999 added, 1999 removed".to_string());
        assert_eq!(diff.text, want.join("\n"));
    }

    /// Matching the shared last line first would print `+ c` above `  c`. A diff that fits the
    /// table must print as it always has.
    #[test]
    fn a_small_diff_keeps_how_the_table_places_a_repeated_line() {
        let diff = diff_snapshots("- a\n- c", "- b\n- c\n- c", DiffOptions::default());
        assert_eq!(diff.text, "- a\n+ b\n  c\n+ c\n2 added, 1 removed");
    }

    #[test]
    fn identical_snapshots_short_circuit_to_no_change() {
        let snap = "- button \"Go\" [ref=e1]";
        assert_eq!(
            diff_snapshots(snap, snap, DiffOptions::default()),
            SnapshotDiff::default()
        );
    }

    #[test]
    fn state_change_shows_removed_added_pair_on_same_ref() {
        let before = "- button \"Save\" [ref=e1]";
        let after = "- button \"Save\" [ref=e1] [disabled]";
        let diff = diff_snapshots(before, after, DiffOptions::default());
        assert!(diff.changed);
        assert_eq!(diff.added, 1);
        assert_eq!(diff.removed, 1);
        assert!(diff.text.contains("- button \"Save\" [ref=e1]"));
        assert!(diff.text.contains("+ button \"Save\" [ref=e1] [disabled]"));
        assert!(diff.text.contains("1 added, 1 removed"));
    }

    #[test]
    fn pure_additions_count_only_as_added_and_strip_bullet() {
        let before = "- main\n  - link \"Home\" [ref=e1]";
        let after = "- main\n  - link \"Home\" [ref=e1]\n  - link \"About\" [ref=e2]";
        let diff = diff_snapshots(before, after, DiffOptions::default());
        assert_eq!(diff.added, 1);
        assert_eq!(diff.removed, 0);
        assert!(diff.text.contains("+   link \"About\" [ref=e2]"));
    }

    #[test]
    fn stable_refs_turn_top_insertions_into_one_added_line() {
        let before = "- button \"A\" [ref=e1]\n- link \"B\" [ref=e2]";
        let after = [
            "- button \"X\" [ref=e3]",
            "- button \"A\" [ref=e1]",
            "- link \"B\" [ref=e2]",
        ]
        .join("\n");
        let diff = diff_snapshots(before, &after, DiffOptions::default());
        assert_eq!(diff.added, 1);
        assert_eq!(diff.removed, 0);
        assert!(diff.text.contains("+ button \"X\" [ref=e3]"));
        assert!(diff.text.contains("1 added, 0 removed"));
    }

    #[test]
    fn collapses_far_apart_context_with_ellipsis() {
        let before = (0..30)
            .map(|index| format!("- item {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let after = before
            .replace("- item 0", "- item ZERO")
            .replace("- item 29", "- item LAST");
        let diff = diff_snapshots(
            &before,
            &after,
            DiffOptions {
                context_radius: Some(2),
            },
        );
        assert!(diff.text.contains('…'));
        assert!(diff.text.contains("- item 0"));
        assert!(diff.text.contains("+ item ZERO"));
        assert!(diff.text.contains("+ item LAST"));
        assert!(!diff.text.contains("item 15"));
    }

    #[test]
    fn url_changes_return_full_current_snapshot() {
        let before = SnapshotObservation {
            text: "- main\n  - button \"Old page\" [ref=e1]".to_string(),
            url: Some("https://example.com/old".to_string()),
            ..SnapshotObservation::default()
        };
        let after = SnapshotObservation {
            text: "- main\n  - heading \"New page\"".to_string(),
            url: Some("https://example.com/new".to_string()),
            ..SnapshotObservation::default()
        };
        let diff = diff_snapshot_observations(Some(&before), &after, DiffOptions::default());
        assert_eq!(diff.text, after.text);
        assert_eq!(diff.added, 0);
        assert_eq!(diff.removed, 0);
        assert!(diff.changed);
        assert!(diff.url_changed);
        assert_eq!(diff.before_url.as_deref(), before.url.as_deref());
        assert_eq!(diff.after_url.as_deref(), after.url.as_deref());
    }

    #[test]
    fn unknown_urls_keep_line_diff_behavior() {
        let diff = diff_snapshot_observations(
            Some(&SnapshotObservation {
                text: "- main\n  - button \"Old\"".to_string(),
                url: Some("unknown".to_string()),
                ..SnapshotObservation::default()
            }),
            &SnapshotObservation {
                text: "- main\n  - button \"New\"".to_string(),
                url: Some("https://example.com/new".to_string()),
                ..SnapshotObservation::default()
            },
            DiffOptions::default(),
        );
        assert!(diff.changed);
        assert!(!diff.url_changed);
        assert_eq!(diff.added, 1);
        assert_eq!(diff.removed, 1);
        assert!(diff.text.contains("-   button \"Old\""));
        assert!(diff.text.contains("+   button \"New\""));
    }

    #[test]
    fn same_url_diffs_preserve_current_url() {
        let diff = diff_snapshot_observations(
            Some(&SnapshotObservation {
                text: "- main\n  - button \"Save\" [ref=e1]".to_string(),
                url: Some("https://example.com/form".to_string()),
                ..SnapshotObservation::default()
            }),
            &SnapshotObservation {
                text: "- main\n  - button \"Saved\" [ref=e1]".to_string(),
                url: Some("https://example.com/form".to_string()),
                ..SnapshotObservation::default()
            },
            DiffOptions::default(),
        );
        assert!(diff.changed);
        assert!(!diff.url_changed);
        assert_eq!(diff.after_url.as_deref(), Some("https://example.com/form"));
    }

    /// The textual verdict and the structural one are allowed to disagree,
    /// and on a page carrying a countdown they must.
    #[test]
    fn a_text_diff_alone_does_not_answer_whether_the_page_moved() {
        let before = SnapshotObservation {
            text: "link \"Deal of the Day, Ends in 08:54:33\"".to_string(),
            url: Some("https://example.com/".to_string()),
            ..SnapshotObservation::default()
        };
        let after = SnapshotObservation {
            text: "link \"Deal of the Day, Ends in 08:54:32\"".to_string(),
            url: Some("https://example.com/".to_string()),
            ..SnapshotObservation::default()
        };
        let diff = diff_snapshot_observations(Some(&before), &after, DiffOptions::default());
        assert!(diff.changed, "the text really did change");
        assert_eq!(
            diff.arrangement_changed, None,
            "a text diff cannot answer the structural question on its own"
        );
    }

    #[test]
    fn a_url_change_is_a_change_of_page_whatever_the_controls_did() {
        let before = SnapshotObservation {
            text: "button \"Go\"".to_string(),
            url: Some("https://example.com/".to_string()),
            ..SnapshotObservation::default()
        };
        let after = SnapshotObservation {
            text: "button \"Go\"".to_string(),
            url: Some("https://example.com/next".to_string()),
            ..SnapshotObservation::default()
        };
        let diff = diff_snapshot_observations(Some(&before), &after, DiffOptions::default());
        assert_eq!(diff.arrangement_changed, Some(true));
    }
}
