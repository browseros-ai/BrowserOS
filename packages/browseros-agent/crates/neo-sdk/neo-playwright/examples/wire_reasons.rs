//! Prints every way an op differs from Playwright 1.64.1's protocol, with the reason its
//! route row gives: the inventory task 006's audit holds against the owner's rule that Neo's own
//! packaging is not a reason (align-plan.md, section 5). It reads the same rows `Run` and
//! `Describe` walk, so the list cannot drift from what the ops send and answer.
//!
//! ```text
//! cargo run -p neo-playwright --example wire_reasons > reasons.md
//! ```
//!
//! The generated upstream checks compare an input key by key, but a result only for void-ness
//! (`VoidIsNull`), so a second table sets each command row's answer beside upstream's result type
//! for the audit to compare by hand.

use std::collections::BTreeMap;
use std::io::{self, Write};

use neo_playwright::{Op, Upstream};

fn main() -> io::Result<()> {
    let mut out = io::stdout().lock();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut answers = Vec::new();
    let mut ops = 0;

    writeln!(out, "## Deviations by op\n")?;
    writeln!(out, "| family | op | upstream | kind | key | reason |")?;
    writeln!(out, "|---|---|---|---|---|---|")?;
    for op in Op::all() {
        let (family, name) = (op.family(), op.spec().name);
        let wire = op.wire();
        ops += 1;
        match wire.upstream {
            Upstream::Command(command) => {
                for (kind, deviations) in [
                    ("adds", command.adds),
                    ("omits", command.omits),
                    ("retypes", command.retypes),
                ] {
                    for (key, reason) in deviations {
                        *counts.entry(kind).or_default() += 1;
                        row(&mut out, &[family, name, command.name, kind, key, reason])?;
                    }
                }
                if let Some(reason) = command.result {
                    *counts.entry("result").or_default() += 1;
                    row(
                        &mut out,
                        &[
                            family,
                            name,
                            command.name,
                            "result",
                            &code(&wire.output),
                            reason,
                        ],
                    )?;
                }
                let aligned = command.adds.is_empty()
                    && command.omits.is_empty()
                    && command.retypes.is_empty()
                    && command.result.is_none();
                if aligned {
                    *counts.entry("aligned").or_default() += 1;
                    row(&mut out, &[family, name, command.name, "aligned", "", ""])?;
                }
                answers.push([
                    name.to_owned(),
                    code(&result_type(command.name)),
                    code(&wire.output),
                    command.result.unwrap_or("").to_owned(),
                ]);
            }
            Upstream::NeoOnly { reason } => {
                *counts.entry("neo_only").or_default() += 1;
                row(&mut out, &[family, name, "", "neo_only", "", reason])?;
            }
            Upstream::Replaces { command, reason } => {
                *counts.entry("replaces").or_default() += 1;
                row(&mut out, &[family, name, command, "replaces", "", reason])?;
            }
        }
    }

    let tally: Vec<String> = counts
        .iter()
        .map(|(kind, n)| format!("{kind} {n}"))
        .collect();
    writeln!(out, "\n{ops} ops: {}.", tally.join(", "))?;

    writeln!(out, "\n## Answers beside upstream's results\n")?;
    writeln!(
        out,
        "Typed checks assert only that a void command answers `null`; the rest is compared here.\n"
    )?;
    writeln!(
        out,
        "| op | upstream result | Neo answers | stated reason |"
    )?;
    writeln!(out, "|---|---|---|---|")?;
    for [name, upstream, neo, reason] in &answers {
        row(&mut out, &[name, upstream, neo, reason])?;
    }
    Ok(())
}

/// A type as a code span, so Markdown keeps `Array<string>`'s parameter instead of reading
/// `<string>` as an HTML tag.
fn code(text: &str) -> String {
    format!("`{text}`")
}

/// One Markdown table row; a `|` inside a cell (a union type) is escaped.
fn row(out: &mut impl Write, cells: &[&str]) -> io::Result<()> {
    let cells: Vec<String> = cells.iter().map(|cell| cell.replace('|', "\\|")).collect();
    writeln!(out, "| {} |", cells.join(" | "))
}

/// `Frame.evaluateExpression` → `FrameEvaluateExpressionResult`, the name `channels.d.ts` gives
/// a command's result.
fn result_type(command: &str) -> String {
    let (interface, method) = command.split_once('.').unwrap_or((command, ""));
    let mut chars = method.chars();
    let head: String = chars
        .next()
        .map(|c| c.to_ascii_uppercase())
        .into_iter()
        .collect();
    format!("{interface}{head}{}Result", chars.as_str())
}
