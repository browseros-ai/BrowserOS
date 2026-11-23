//! The covered click's block, ported from the TypeScript prelude's `cover.ts` (`snapshotLines`).
//! A pointer action whose element is covered fails with an error that names the cover under an
//! "On top" line; the block is the cover's own snapshot printed under that line, so the agent
//! reads the cover's refs as its own `ariaSnapshot()` would print them.
//!
//! ```text
//!  neo-playwright action/cover.rs: the cover's tree ─▶ cover_block ─▶ the error's cover, beside
//!  Rust's message; the prelude places the block under "On top" with today's placement, unchanged
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

/// The lines under a covered click's "On top" line: the cover's tree in Neo's default format with
/// every default, without the lines blank after JavaScript's trim, four spaces before each line,
/// at most 1,500 units. The first line that would pass the limit is cut to the room left, when
/// there is any, and ends in `…`; then `    … (the rest of its snapshot is cut)`. `None` when
/// nothing is left to print.
///
/// The cut is JavaScript's own `slice`: inside a character it keeps half of it, which a Rust
/// string cannot hold, so the half is U+FFFD, the character every reader outside the script
/// engine receives for it today. A tree that does not decode never reaches here: the caller has
/// no block to place, as the TypeScript printed none when its renderer threw.
pub(crate) fn cover_block(tree: Vec<AriaNode>) -> Option<String> {
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
