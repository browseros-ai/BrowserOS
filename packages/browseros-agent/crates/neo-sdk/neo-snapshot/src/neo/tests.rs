//! Neo's default format in Rust against its judges (`crate::judges`), through `render` with
//! `Call::Element` and `Call::Page`: the committed cases; the private sets of saved pages, where
//! a folder names them; and today's TypeScript renderer in QuickJS on seeded random trees with
//! every fact and every combination of options. Each is held to its text and to what it keeps.

use std::error::Error;
use std::{env, fs};

use serde_json::json;

use crate::call::Call;
use crate::cover_block::cover_block;
use crate::form::Passed;
use crate::judges::{Bundles, CaseFile, Generator, NEO_CASES, Shown, count, seed};
use crate::render::render;
use crate::snapshot::Snapshot;

use super::neo_text_as;

/// How many random trees the gate's run draws.
const GATE_TREES: usize = 300;

/// A call in Neo's default format, a locator's or a whole page's.
fn neos(call: &Call<'_>) -> bool {
    matches!(call, Call::Element(_) | Call::Page(_))
}

#[test]
fn tests_that_neos_format_prints_every_committed_case() -> Result<(), Box<dyn Error>> {
    let cases: CaseFile = serde_json::from_str(NEO_CASES)?;
    let mut judged = 0;
    let mut differing = Vec::new();
    for found in cases.rendered_cases(neos) {
        let (case, call, name) = found?;
        let answer = render(&cases.snapshot(name)?, call);
        if let Some(difference) = case.expected().difference(&call, &answer, Shown::Lines)? {
            differing.push(format!("{}: {difference}", case.name));
        }
        judged += 1;
    }
    assert!(
        judged > 0,
        "neo.json holds cases in Neo's format, or this test judges nothing"
    );
    assert!(
        differing.is_empty(),
        "{} of {judged} cases in Neo's format differ from the committed text:\n{}",
        differing.len(),
        differing.join("\n")
    );
    Ok(())
}

/// The private sets are real pages' content, so they live outside the repository and never go
/// to the remote pool: the test reads the folders `NEO_SNAPSHOT_PRIVATE_CASES` names (joined
/// with the platform's path separator), each `*.json` file in the committed cases' shape, and is
/// run by hand where they are kept. Without the variable it passes and says it was skipped, so a
/// landing that needs the proof pastes the counts from a run with it. Nothing it prints holds a
/// page's text: a difference is named by its case's number, its line and the two lengths.
#[test]
#[expect(
    clippy::print_stdout,
    reason = "the counts are the evidence a reviewer pastes; a silent skip must not read as a pass"
)]
fn tests_that_neos_format_prints_every_private_case() -> Result<(), Box<dyn Error>> {
    let Some(folders) = env::var_os("NEO_SNAPSHOT_PRIVATE_CASES") else {
        println!(
            "private sets, Neo's format: SKIPPED (NEO_SNAPSHOT_PRIVATE_CASES names no folder)"
        );
        return Ok(());
    };
    let bundles = Bundles::new()?;
    let mut differing: Vec<String> = Vec::new();
    let (mut texts, mut trees) = (Tally::default(), Tally::default());
    for file in &CaseFile::private_files(&folders)? {
        let shown = file.file_name().unwrap_or_default().to_string_lossy();
        let cases: CaseFile = serde_json::from_str(&fs::read_to_string(file)?)?;
        let file_texts = saved_texts(&cases, &shown, &mut differing)?;
        let file_trees = saved_trees(&cases, &bundles, &shown, &mut differing)?;
        println!(
            "private sets, Neo's format: {shown}: {} of {} saved texts identical, the kept \
             baseline too; {} of {} trees print today's renderer's text and covered block",
            file_texts.identical, file_texts.judged, file_trees.identical, file_trees.judged
        );
        texts.add(file_texts);
        trees.add(file_trees);
    }
    println!(
        "private sets, Neo's format: {} of {} identical, {} of {} trees as today's renderer \
         prints them",
        texts.identical, texts.judged, trees.identical, trees.judged
    );
    assert!(
        trees.judged > 0,
        "the named folders hold no case file, which proves nothing"
    );
    assert!(
        differing.is_empty(),
        "{} differences:\n{}",
        differing.len(),
        differing.join("\n")
    );
    Ok(())
}

/// How many of a set's texts were judged, and how many of those were identical.
#[derive(Clone, Copy, Default)]
struct Tally {
    judged: usize,
    identical: usize,
}

impl Tally {
    fn count(&mut self, identical: bool) {
        self.judged += 1;
        if identical {
            self.identical += 1;
        }
    }

    fn add(&mut self, other: Self) {
        self.judged += other.judged;
        self.identical += other.identical;
    }
}

/// Every saved call in Neo's format in `cases`, against its saved text and kept baseline. A
/// difference is named by `shown`, the case's number, its line and the two lengths.
fn saved_texts(
    cases: &CaseFile,
    shown: &str,
    differing: &mut Vec<String>,
) -> Result<Tally, Box<dyn Error>> {
    let mut tally = Tally::default();
    for (index, found) in cases.rendered_cases(neos).enumerate() {
        let (case, call, name) = found?;
        let answer = render(&cases.snapshot(name)?, call);
        let difference = case.expected().difference(&call, &answer, Shown::Lengths)?;
        if let Some(difference) = &difference {
            differing.push(format!("{shown}: Neo-format case {index}: {difference}"));
        }
        tally.count(difference.is_none());
    }
    Ok(tally)
}

/// Every saved tree in `cases`, not only those a Neo-format call was saved for: Rust's whole page
/// with every default, its text and what it keeps, and its covered block, beside today's
/// renderer's in QuickJS.
fn saved_trees(
    cases: &CaseFile,
    bundles: &Bundles,
    shown: &str,
    differing: &mut Vec<String>,
) -> Result<Tally, Box<dyn Error>> {
    let mut tally = Tally::default();
    let passed = Passed::default();
    let call = Call::Page(&passed);
    for (index, name) in cases.snapshot_names().enumerate() {
        let snapshot = cases.snapshot(name)?;
        let answer = render(&snapshot, call);
        let todays = bundles.todays(&call, &snapshot)?;
        let block = cover_block(snapshot.tree.clone());
        let todays_block = bundles
            .todays_answer(&json!({ "cover": {} }), &snapshot)?
            .text;
        let difference = if let Some(difference) =
            todays
                .expected()
                .difference(&call, &answer, Shown::Lengths)?
        {
            Some(format!("against today's renderer: {difference}"))
        } else if block != todays_block {
            Some(format!(
                "the covered block differs; {:?} bytes printed, {:?} by today's renderer",
                block.as_deref().map(str::len),
                todays_block.as_deref().map(str::len)
            ))
        } else {
            None
        };
        if let Some(difference) = &difference {
            differing.push(format!("{shown}: tree {index}: {difference}"));
        }
        tally.count(difference.is_none());
    }
    Ok(tally)
}

/// Seeded random trees, each in Neo's format as a whole page and as a locator's snapshot with
/// the same options, against today's renderer: the text, and what each keeps.
#[test]
fn tests_that_neos_format_is_todays_on_generated_trees() -> Result<(), Box<dyn Error>> {
    let seed = seed()?;
    let bundles = Bundles::new()?;
    let mut generator = Generator::new(seed);
    for index in 0..count("NEO_SNAPSHOT_DIFFERENTIAL_TREES", GATE_TREES)? {
        let snapshot = generator.snapshot();
        let passed = generator.passed();
        for call in [Call::Page(&passed), Call::Element(&passed)] {
            let answer = render(&snapshot, call);
            let todays = bundles.todays(&call, &snapshot)?;
            if let Some(difference) = todays.expected().difference(&call, &answer, Shown::Lines)? {
                panic!("seed {seed:#x}, tree {index}, {call:?}: {difference}\n{snapshot:?}");
            }
        }
    }
    Ok(())
}

/// What a delta stands on: a whole page's text rendered again in the form it keeps is the same
/// text, the length rule not asked again and the text form walked again for the notice.
#[test]
fn tests_that_a_text_rendered_again_in_its_own_form_is_the_same_text() -> Result<(), Box<dyn Error>>
{
    let cases: CaseFile = serde_json::from_str(NEO_CASES)?;
    let mut judged = 0;
    let rendered_again = |snapshot: &Snapshot, passed: &Passed| -> Result<(), String> {
        let answer = render(snapshot, Call::Page(passed));
        let kept = answer.pending().ok_or("a whole page keeps itself")?;
        (neo_text_as(snapshot, &kept.form) == kept.text)
            .then_some(())
            .ok_or_else(|| format!("renders again in its form {:?} as it was", kept.form))
    };
    for found in cases.rendered_cases(|call| matches!(call, Call::Page(_))) {
        let (case, call, name) = found?;
        let Call::Page(passed) = call else {
            continue;
        };
        rendered_again(&cases.snapshot(name)?, passed)
            .map_err(|failed| format!("{}: {failed}", case.name))?;
        judged += 1;
    }
    let seed = seed()?;
    let mut generator = Generator::new(seed);
    for index in 0..count("NEO_SNAPSHOT_DIFFERENTIAL_TREES", GATE_TREES)? {
        let snapshot: Snapshot = generator.snapshot();
        rendered_again(&snapshot, &generator.passed())
            .map_err(|failed| format!("seed {seed:#x}, tree {index}: {failed}"))?;
        judged += 1;
    }
    assert!(judged > 0, "texts were rendered again");
    Ok(())
}
