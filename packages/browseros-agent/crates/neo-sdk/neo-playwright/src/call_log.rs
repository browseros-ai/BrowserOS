//! Playwright's call log: the lines under `Call log:` in an error message.
//!
//! Leaves record lines as Playwright's server logs them, leading spaces nesting a line under
//! the one before (`waiting for getByRole('button')`, `  locator resolved to <button>`,
//! `attempting click action`, `  waiting for element to be visible, enabled and stable`).
//! Rendering ports upstream's `compressCallLog` (server/callLog.ts at v1.64.1): a block of lines
//! repeated back to back prints once, as `N × <first line>`. Neo adds one bound, at most 12
//! rendered lines (the first and the latest 11), so a long retry keeps its context line and its
//! latest state.
//!
//! The lines are mirrored into the op's trace record (`OpEnv::note`).

/// Rendered lines kept in an error: the first one and the latest `MAX_LINES - 1`.
const MAX_LINES: usize = 12;

/// The longest repeated block the compression looks for. Upstream's retry blocks are 2 to 5
/// lines; the bound keeps compression linear in the log's length on the minutes-long waits a
/// `timeout: 0` op can run, where upstream's unbounded search grows with its cube.
const MAX_BLOCK: usize = 32;

/// One op's log, in the order its lines were recorded.
#[derive(Debug, Default)]
pub(crate) struct CallLog {
    lines: Vec<String>,
}

impl CallLog {
    /// Records `line` and says whether it differs from the line before it, so the trace mirror
    /// gets each change once while the log keeps the repeats it compresses.
    pub(crate) fn push(&mut self, line: &str) -> bool {
        let changed = self.lines.last().is_none_or(|last| last != line);
        self.lines.push(line.to_owned());
        changed
    }

    /// The recorded lines as they were logged, uncompressed: Playwright's `matcherResult.log`,
    /// which upstream fills from the server's raw log. The same bound as [`CallLog::render`]
    /// applies, to the raw lines (the first one and the latest `MAX_LINES - 1`), so a
    /// minutes-long wait cannot make the script's error as large as its log.
    pub(crate) fn kept_lines(&self) -> Vec<String> {
        if self.lines.len() <= MAX_LINES {
            return self.lines.clone();
        }
        self.lines
            .iter()
            .take(1)
            .chain(self.lines.iter().skip(self.lines.len() - (MAX_LINES - 1)))
            .cloned()
            .collect()
    }

    /// What follows an error's first line, as upstream's client appends it:
    /// `"\nCall log:\n  - waiting for ...\n"`, or nothing when no line was recorded.
    pub(crate) fn render(&self) -> String {
        let lines = compress(&self.lines);
        if lines.is_empty() {
            return String::new();
        }
        let kept: Vec<&str> = if lines.len() > MAX_LINES {
            lines
                .iter()
                .take(1)
                .chain(lines.iter().skip(lines.len() - (MAX_LINES - 1)))
                .map(String::as_str)
                .collect()
        } else {
            lines.iter().map(String::as_str).collect()
        };
        format!("\nCall log:\n{}\n", kept.join("\n"))
    }
}

/// A run of `count` back-to-back copies of `lines`.
#[derive(Debug, PartialEq, Eq)]
struct Block<'a> {
    lines: &'a [String],
    count: usize,
}

/// Upstream's `compressCallLog`: each line becomes `"  " + its indent + "- " + its text`, and
/// a repeated block's first line carries the count instead of the dash.
fn compress(lines: &[String]) -> Vec<String> {
    let mut rendered = Vec::new();
    for block in repeated_blocks(lines) {
        // `×` is one UTF-16 unit in JavaScript's `length`, so count chars, not bytes.
        let count_prefix = format!("{} × ", block.count);
        let continuation = " ".repeat(count_prefix.chars().count() - 2);
        for (position, line) in block.lines.iter().enumerate() {
            let text = line.trim();
            let indent = line
                .get(..line.len() - line.trim_start().len())
                .unwrap_or_default();
            rendered.push(match (block.count > 1, position) {
                (true, 0) => format!("  {indent}{count_prefix}{text}"),
                (true, _) => format!("  {indent}{continuation}- {text}"),
                (false, _) => format!("  {indent}- {text}"),
            });
        }
    }
    rendered
}

/// Upstream's `findRepeatedSubsequences`: from each position, the block length whose
/// back-to-back copies cover the most lines (ties go to the shorter block), else one line.
fn repeated_blocks(lines: &[String]) -> Vec<Block<'_>> {
    let mut blocks = Vec::new();
    let mut start = 0;
    while let Some(rest) = lines.get(start..).filter(|rest| !rest.is_empty()) {
        let mut best = Block {
            lines: rest.get(..1).unwrap_or_default(),
            count: 1,
        };
        for length in 1..=rest.len().min(MAX_BLOCK) {
            let count = copies(rest, length);
            if count > 1 && count * length > best.count * best.lines.len() {
                best = Block {
                    lines: rest.get(..length).unwrap_or_default(),
                    count,
                };
            }
        }
        start += best.lines.len() * best.count;
        blocks.push(best);
    }
    blocks
}

/// How many back-to-back copies of `rest[..length]` start `rest`.
fn copies(rest: &[String], length: usize) -> usize {
    let mut chunks = rest.chunks_exact(length);
    let Some(first) = chunks.next() else {
        return 0;
    };
    1 + chunks.take_while(|chunk| chunk == &first).count()
}

#[cfg(test)]
mod tests {
    use super::{Block, CallLog, repeated_blocks};

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|line| (*line).to_owned()).collect()
    }

    /// Each block as `(its lines, its count)`, the shape upstream's tests compare.
    fn blocks(input: &[&str]) -> Vec<(Vec<String>, usize)> {
        let input = lines(input);
        repeated_blocks(&input)
            .into_iter()
            .map(|Block { lines, count }| (lines.to_vec(), count))
            .collect()
    }

    fn block(text: &[&str], count: usize) -> (Vec<String>, usize) {
        (lines(text), count)
    }

    /// Upstream's tests/library/unit/sequence.spec.ts at v1.64.1, case by case.
    #[test]
    fn tests_that_repeated_blocks_match_upstream() {
        assert_eq!(blocks(&[]), [], "empty");
        assert_eq!(blocks(&["a"]), [block(&["a"], 1)], "single element");
        assert_eq!(
            blocks(&["a", "b", "c"]),
            [block(&["a"], 1), block(&["b"], 1), block(&["c"], 1)],
            "no repeats"
        );
        assert_eq!(
            blocks(&["a", "a", "a", "b", "b", "c"]),
            [block(&["a"], 3), block(&["b"], 2), block(&["c"], 1)],
            "contiguous repeats of single elements"
        );
        assert_eq!(
            blocks(&["a", "b", "a", "b", "a", "b"]),
            [block(&["a", "b"], 3)],
            "longer repeating substrings"
        );
        assert_eq!(
            blocks(&["a", "a", "b", "b", "a", "a", "b", "b"]),
            [block(&["a", "a", "b", "b"], 2)],
            "multiple repeating substrings"
        );
        assert_eq!(
            blocks(&["a", "a", "a", "a"]),
            [block(&["a"], 4)],
            "overlapping repeats"
        );
        assert_eq!(
            blocks(&["a", "a", "b", "b", "a", "a", "b", "b", "c", "c", "c", "c"]),
            [block(&["a", "a", "b", "b"], 2), block(&["c"], 4)],
            "multiple possible repeats"
        );
        assert_eq!(
            blocks(&["a", "b", "a", "b", "a", "b", "c", "c", "c", "c"]),
            [block(&["a", "b"], 3), block(&["c"], 4)],
            "maximal repeating substring"
        );
        assert_eq!(
            blocks(&["a", "a", "b", "b", "b", "b", "a", "a"]),
            [block(&["a"], 2), block(&["b"], 4), block(&["a"], 2)],
            "repeats with varying lengths"
        );
        assert_eq!(
            blocks(&["a", "b", "a", "b", "c"]),
            [block(&["a", "b"], 2), block(&["c"], 1)],
            "a repeat count of one"
        );
        assert_eq!(
            blocks(&["x", "y", "x", "y", "x", "y", "z"]),
            [block(&["x", "y"], 3), block(&["z"], 1)],
            "repeats at the end"
        );
        assert_eq!(
            blocks(&["m", "n", "m", "n", "m"]),
            [block(&["m", "n"], 2), block(&["m"], 1)],
            "an incomplete last repeat"
        );
        assert_eq!(
            blocks(&["a", "b", "c", "a", "b", "d"]),
            [
                block(&["a"], 1),
                block(&["b"], 1),
                block(&["c"], 1),
                block(&["a"], 1),
                block(&["b"], 1),
                block(&["d"], 1),
            ],
            "single repeats longer than one"
        );
    }

    fn log(text: &[&str]) -> CallLog {
        let mut log = CallLog::default();
        for line in text {
            log.push(line);
        }
        log
    }

    /// A timeout real Playwright 1.64.1 printed in the look-then-act eval
    /// (spikes/results/eval/eval-final.json).
    #[test]
    fn tests_that_a_waiting_click_renders_as_upstream() {
        assert_eq!(
            log(&["waiting for locator('aria-ref=e7')"]).render(),
            "\nCall log:\n  - waiting for locator('aria-ref=e7')\n"
        );
    }

    /// An `expect` that polled 24 times, from the same eval: a repeated two-line block with
    /// its continuation line aligned under the count.
    #[test]
    fn tests_that_a_repeated_block_renders_as_upstream() {
        let button = "<button data-id=\"pay\">Pay $42.00</button>";
        let mut text = vec![
            "Expect \"not toHaveText\" locator('[data-id=\"pay\"]') with timeout 10000ms"
                .to_owned(),
            "waiting for locator('[data-id=\"pay\"]')".to_owned(),
        ];
        for _ in 0..24 {
            text.push(format!("  locator resolved to {button}"));
            text.push("  unexpected value \"Pay $42.00\"".to_owned());
        }
        let mut log = CallLog::default();
        for line in &text {
            log.push(line);
        }
        assert_eq!(
            log.render(),
            format!(
                "\nCall log:\n  - Expect \"not toHaveText\" locator('[data-id=\"pay\"]') with timeout \
                 10000ms\n  - waiting for locator('[data-id=\"pay\"]')\n    24 × locator resolved to \
                 {button}\n       - unexpected value \"Pay $42.00\"\n"
            )
        );
    }

    #[test]
    fn tests_that_an_empty_log_renders_nothing() {
        assert_eq!(CallLog::default().render(), "");
    }

    #[test]
    fn tests_that_a_long_log_keeps_its_first_line_and_the_latest_eleven() {
        let text: Vec<String> = (0..30).map(|line| format!("step {line}")).collect();
        let mut log = CallLog::default();
        for line in &text {
            log.push(line);
        }
        let rendered = log.render();
        let kept: Vec<&str> = rendered.lines().skip(2).collect();
        assert_eq!(kept.len(), 12, "at most 12 rendered lines: {rendered}");
        assert_eq!(kept.first(), Some(&"  - step 0"), "the context line stays");
        assert_eq!(kept.get(1), Some(&"  - step 19"), "then the latest 11");
        assert_eq!(kept.last(), Some(&"  - step 29"));
    }

    #[test]
    fn tests_that_push_reports_only_changes() {
        let mut log = CallLog::default();
        assert!(
            log.push("attempting click action"),
            "a first line is a change"
        );
        assert!(log.push("  element is not visible"));
        assert!(!log.push("  element is not visible"), "a repeat is not");
        assert!(log.push("retrying click action"));
    }
}
