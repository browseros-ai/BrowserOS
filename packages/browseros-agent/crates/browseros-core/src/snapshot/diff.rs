#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotDiff {
    pub text: String,
    pub added: usize,
    pub removed: usize,
    pub changed: bool,
    /// When true, `text` is the full current snapshot for a known URL change, not a line diff,
    /// and `added` and `removed` are zero.
    pub url_changed: bool,
    pub before_url: Option<String>,
    pub after_url: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffOptions {
    pub context_radius: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotObservation {
    pub text: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TaggedLine {
    gutter: char,
    text: String,
}

pub fn diff_snapshots(before: &str, after: &str, opts: DiffOptions) -> SnapshotDiff {
    if before == after {
        return SnapshotDiff::default();
    }

    let mut tagged = Vec::new();
    let mut added = 0;
    let mut removed = 0;
    for line in diff_lines(before, after) {
        if line.gutter == '+' {
            added += 1;
        } else if line.gutter == '-' {
            removed += 1;
        }
        tagged.push(line);
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
        };
    }

    let before_text = before.map(|before| before.text.as_str()).unwrap_or("");
    let mut diff = diff_snapshots(before_text, &after.text, opts);
    if is_known_url(after_url) {
        diff.after_url = after_url.map(ToString::to_string);
    }
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

fn diff_lines(before: &str, after: &str) -> Vec<TaggedLine> {
    let before_lines = split_lines(before);
    let after_lines = split_lines(after);
    let table = build_lcs_table(&before_lines, &after_lines);
    let mut tagged = Vec::new();
    let mut i = 0;
    let mut j = 0;

    while i < before_lines.len() && j < after_lines.len() {
        if before_lines[i] == after_lines[j] {
            tagged.push(TaggedLine {
                gutter: ' ',
                text: before_lines[i].to_string(),
            });
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            tagged.push(TaggedLine {
                gutter: '-',
                text: before_lines[i].to_string(),
            });
            i += 1;
        } else {
            tagged.push(TaggedLine {
                gutter: '+',
                text: after_lines[j].to_string(),
            });
            j += 1;
        }
    }

    while i < before_lines.len() {
        tagged.push(TaggedLine {
            gutter: '-',
            text: before_lines[i].to_string(),
        });
        i += 1;
    }
    while j < after_lines.len() {
        tagged.push(TaggedLine {
            gutter: '+',
            text: after_lines[j].to_string(),
        });
        j += 1;
    }

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

fn collapse(tagged: &[TaggedLine], radius: usize) -> String {
    let mut keep = vec![false; tagged.len()];
    for (index, line) in tagged.iter().enumerate() {
        if line.gutter == ' ' {
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
        out.push(format!("{} {}", line.gutter, strip_bullet(&line.text)));
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
        };
        let after = SnapshotObservation {
            text: "- main\n  - heading \"New page\"".to_string(),
            url: Some("https://example.com/new".to_string()),
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
            }),
            &SnapshotObservation {
                text: "- main\n  - button \"New\"".to_string(),
                url: Some("https://example.com/new".to_string()),
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
            }),
            &SnapshotObservation {
                text: "- main\n  - button \"Saved\" [ref=e1]".to_string(),
                url: Some("https://example.com/form".to_string()),
            },
            DiffOptions::default(),
        );
        assert!(diff.changed);
        assert!(!diff.url_changed);
        assert_eq!(diff.after_url.as_deref(), Some("https://example.com/form"));
    }
}
