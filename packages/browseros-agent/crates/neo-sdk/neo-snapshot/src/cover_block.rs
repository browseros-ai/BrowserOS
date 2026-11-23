//! The covered click's block, ported from the prelude's `snapshotLines`, which now lives on as
//! this port's test oracle in neo-playwright-js `scripts/snapshot-cases/snapshot-lines.ts`. A
//! pointer action whose element is covered fails with an error that names the cover under an
//! "On top" line; the block is the cover's own snapshot printed under that line, so the agent
//! reads the cover's refs as its own `ariaSnapshot()` would print them.
//!
//! ```text
//!  neo-playwright action/cover.rs: the cover's tree ─▶ cover_block ─▶ the error's cover as
//!  `block`, beside Rust's message ─▶ the prelude's `shapeCovered` places it under "On top"
//! ```
//!
//! The block never goes into Rust's message: the operation's record keeps its message without it,
//! so a trace's clip cannot push the cover's controls and the call log out of the record.

use crate::aria_node::AriaNode;
use crate::form::Passed;
use crate::js;
use crate::neo::neo_text;
use crate::snapshot::Snapshot;

/// The most of the cover's snapshot the block carries, in UTF-16 units, each line counted with
/// its newline.
const BLOCK_UNITS: usize = 1_500;

/// The lines a covered click's error prints under its "On top" line: the cover's tree in Neo's
/// default format with every default (a script's own `ariaSnapshot()`), `None` when that prints
/// nothing after the blank lines are dropped.
///
/// - Each line carries four spaces before it and is joined by `\n`, with no newline before the
///   first or after the last. The placement under "On top" is the caller's; a test of the
///   acceptance suite finds the block by this shape.
/// - At most 1,500 UTF-16 units of the snapshot, each line counted with its newline. The first
///   line that would pass the limit is cut to the room left, when there is any, and ends in `…`;
///   then the last line is `    … (the rest of its snapshot is cut)`.
/// - Where the cut falls inside a character, the half kept is U+FFFD: JavaScript's `slice` keeps
///   half of it, which a Rust string cannot hold, and U+FFFD is what every reader outside the
///   script engine received for it when the prelude cut the block.
///
/// It cannot fail. A tree that does not decode as [`AriaNode`]s is the caller's to leave out, as
/// the prelude printed no block when its renderer threw.
#[must_use]
pub fn cover_block(tree: Vec<AriaNode>) -> Option<String> {
    let snapshot = Snapshot {
        tree,
        ..Snapshot::default()
    };
    let text = neo_text(&snapshot, &Passed::default()).text;
    let mut kept: Vec<String> = Vec::new();
    let mut used = 0;
    for line in text.split('\n') {
        if js::trim(line).is_empty() {
            continue;
        }
        let units = js::units(line);
        if used + units + 1 > BLOCK_UNITS {
            // A long text prints as one line: keep its start up to the limit, not nothing.
            let room = BLOCK_UNITS.saturating_sub(used + 1);
            if room > 0 {
                kept.push(format!("    {}…", js::prefix_as_javascript(line, room)));
            }
            kept.push("    … (the rest of its snapshot is cut)".to_owned());
            break;
        }
        used += units + 1;
        kept.push(format!("    {line}"));
    }
    (!kept.is_empty()).then(|| kept.join("\n"))
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::{env, fs};

    use serde_json::json;

    use crate::judges::{Bundles, CaseFile, Generator, NEO_CASES, count, first_difference, seed};
    use crate::snapshot::Snapshot;

    use super::cover_block;

    const GATE_TREES: usize = 300;

    #[test]
    fn tests_that_the_covered_block_prints_every_committed_case() -> Result<(), Box<dyn Error>> {
        let cases: CaseFile = serde_json::from_str(NEO_CASES)?;
        let mut judged = 0;
        let mut differing = Vec::new();
        for case in cases.cover_cases() {
            // A cover with no tree has no block to place, and the caller never asks for one.
            let printed = match case.snapshot.as_deref() {
                Some(name) => cover_block(cases.snapshot(name)?.tree),
                None => None,
            };
            if printed != case.text {
                let line = match (printed.as_deref(), case.text.as_deref()) {
                    (Some(printed), Some(expected)) => {
                        first_difference(printed, expected).map_or(0, |(line, _, _)| line)
                    }
                    _ => 0,
                };
                differing.push(format!(
                    "{}: printed {:?}, expected {:?} (first differing line {line})",
                    case.name,
                    printed.as_deref().map(str::len),
                    case.text.as_deref().map(str::len)
                ));
            }
            judged += 1;
        }
        assert!(
            judged > 0,
            "neo.json holds cover cases, or this judges nothing"
        );
        assert!(
            differing.is_empty(),
            "{} of {judged} covered blocks differ from the committed text:\n{}",
            differing.len(),
            differing.join("\n")
        );
        Ok(())
    }

    /// Every tree of the private sets (real pages' content, kept on one Mac and never in the
    /// repository or on the pool): the block Rust prints for it as the cover's tree, beside the
    /// block today's renderer prints in QuickJS. Without `NEO_SNAPSHOT_PRIVATE_CASES` it passes
    /// and says it was skipped; nothing it prints holds a page's text.
    #[test]
    #[expect(
        clippy::print_stdout,
        reason = "the counts are the evidence a reviewer pastes; a silent skip must not read as a \
                  pass"
    )]
    fn tests_that_the_covered_block_is_todays_on_every_private_tree() -> Result<(), Box<dyn Error>>
    {
        let Some(folders) = env::var_os("NEO_SNAPSHOT_PRIVATE_CASES") else {
            println!(
                "private sets, covered block: SKIPPED (NEO_SNAPSHOT_PRIVATE_CASES names no folder)"
            );
            return Ok(());
        };
        let bundles = Bundles::new()?;
        let (mut judged, mut identical, mut printed) = (0, 0, 0);
        let mut differing = Vec::new();
        for file in &CaseFile::private_files(&folders)? {
            let shown = file.file_name().unwrap_or_default().to_string_lossy();
            let cases: CaseFile = serde_json::from_str(&fs::read_to_string(file)?)?;
            for (index, name) in cases.snapshot_names().enumerate() {
                let snapshot = cases.snapshot(name)?;
                let todays = bundles
                    .todays_answer(&json!({ "cover": {} }), &snapshot)?
                    .text;
                let block = cover_block(snapshot.tree);
                judged += 1;
                printed += usize::from(block.is_some());
                if block == todays {
                    identical += 1;
                } else {
                    differing.push(format!(
                        "{shown}: tree {index}: {:?} bytes printed, {:?} by today's renderer",
                        block.as_deref().map(str::len),
                        todays.as_deref().map(str::len)
                    ));
                }
            }
        }
        println!(
            "private sets, covered block: {identical} of {judged} trees print today's block \
             ({printed} with a block)"
        );
        assert!(
            judged > 0,
            "the named folders hold no tree, which proves nothing"
        );
        assert!(
            differing.is_empty(),
            "{} differences:\n{}",
            differing.len(),
            differing.join("\n")
        );
        Ok(())
    }

    #[test]
    fn tests_that_the_covered_block_is_todays_on_generated_trees() -> Result<(), Box<dyn Error>> {
        let seed = seed()?;
        let bundles = Bundles::new()?;
        let mut generator = Generator::new(seed);
        for index in 0..count("NEO_SNAPSHOT_DIFFERENTIAL_TREES", GATE_TREES)? {
            let tree = generator.snapshot().tree;
            let todays = bundles
                .todays_answer(
                    &json!({ "cover": {} }),
                    &Snapshot {
                        tree: tree.clone(),
                        ..Snapshot::default()
                    },
                )?
                .text;
            let json = serde_json::to_string(&tree)?;
            assert_eq!(
                cover_block(tree),
                todays,
                "seed {seed:#x}, tree {index} prints today's covered block: {json}"
            );
        }
        Ok(())
    }
}
