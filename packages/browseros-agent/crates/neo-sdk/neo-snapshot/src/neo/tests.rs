//! Neo's default format in Rust against its judges (`crate::judges`): the committed cases; the
//! private sets of saved pages, where a folder names them; and today's TypeScript renderer in
//! QuickJS on seeded random trees with every fact and every combination of options.

use std::error::Error;
use std::{env, fs};

use serde_json::json;

use crate::form::{Form, Passed};
use crate::judges::{
    Bundles, Case, CaseFile, Generator, NEO_CASES, count, first_difference, passed_json, seed,
};
use crate::snapshot::Snapshot;

use super::{neo_text, neo_text_as};

/// How many random trees the gate's run draws.
const GATE_TREES: usize = 300;

#[test]
fn tests_that_neos_format_prints_every_committed_case() -> Result<(), Box<dyn Error>> {
    let cases: CaseFile = serde_json::from_str(NEO_CASES)?;
    let mut judged = 0;
    let mut differing = Vec::new();
    for (case, passed) in cases.neo_cases() {
        let snapshot = cases.snapshot(case.snapshot.as_deref().ok_or("a case names its tree")?)?;
        let printed = neo_text(&snapshot, &passed.passed()?);
        let expected = case
            .text
            .as_deref()
            .ok_or("a case in Neo's format has a text")?;
        if let Some((line, printed_line, expected_line)) = first_difference(&printed.text, expected)
        {
            differing.push(format!(
                "{}: line {line}\n  printed:  {printed_line:?}\n  expected: {expected_line:?}",
                case.name
            ));
        } else if let Some(kept) = kept_differs(case, &printed.text, printed.form)? {
            differing.push(format!("{}: {kept}", case.name));
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
             baseline too; {} of {} trees print today's renderer's text",
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
    for (index, (case, passed)) in cases.neo_cases().enumerate() {
        let snapshot = cases.snapshot(case.snapshot.as_deref().ok_or("a case names its tree")?)?;
        let printed = neo_text(&snapshot, &passed.passed()?);
        let expected = case
            .text
            .as_deref()
            .ok_or("a case in Neo's format has a text")?;
        let difference = match first_difference(&printed.text, expected) {
            Some((line, _, _)) => Some(format!(
                "line {line} differs; {} bytes printed, {} saved",
                printed.text.len(),
                expected.len()
            )),
            None => kept_differs(case, &printed.text, printed.form)?,
        };
        if let Some(difference) = &difference {
            differing.push(format!("{shown}: Neo-format case {index}: {difference}"));
        }
        tally.count(difference.is_none());
    }
    Ok(tally)
}

/// Every saved tree in `cases`, not only those a Neo-format call was saved for: Rust's text with
/// every default, beside today's renderer's in QuickJS.
fn saved_trees(
    cases: &CaseFile,
    bundles: &Bundles,
    shown: &str,
    differing: &mut Vec<String>,
) -> Result<Tally, Box<dyn Error>> {
    let mut tally = Tally::default();
    for (index, name) in cases.snapshot_names().enumerate() {
        let snapshot = cases.snapshot(name)?;
        let printed = neo_text(&snapshot, &Passed::default()).text;
        let todays = bundles
            .todays_answer(&json!({ "page": {} }), &snapshot)?
            .text
            .unwrap_or_default();
        let difference = first_difference(&printed, &todays).map(|(line, _, _)| {
            format!(
                "line {line} differs; {} bytes printed, {} by today's renderer",
                printed.len(),
                todays.len()
            )
        });
        if let Some(difference) = &difference {
            differing.push(format!("{shown}: tree {index}: {difference}"));
        }
        tally.count(difference.is_none());
    }
    Ok(tally)
}

#[test]
fn tests_that_neos_format_is_todays_on_generated_trees() -> Result<(), Box<dyn Error>> {
    let seed = seed()?;
    let bundles = Bundles::new()?;
    let mut generator = Generator::new(seed);
    for index in 0..count("NEO_SNAPSHOT_DIFFERENTIAL_TREES", GATE_TREES)? {
        let snapshot = generator.snapshot();
        let passed = generator.passed();
        let printed = neo_text(&snapshot, &passed);
        let todays = bundles.todays_answer(&json!({ "page": passed_json(&passed) }), &snapshot)?;
        let context = format!("seed {seed:#x}, tree {index}, {passed:?}: {snapshot:?}");
        assert_eq!(
            Some(&printed.text),
            todays.text.as_ref(),
            "prints today's renderer's text; {context}"
        );
        let pending = todays.pending.ok_or("a whole page keeps a baseline")?;
        assert_eq!(
            (&pending.text, pending.form()?),
            (&printed.text, printed.form),
            "keeps the text in today's form; {context}"
        );
    }
    Ok(())
}

/// What a delta stands on: a whole page's text rendered again in the form it was kept in is the
/// same text, the length rule not asked again and the text form walked again for the notice.
#[test]
fn tests_that_a_text_rendered_again_in_its_own_form_is_the_same_text() -> Result<(), Box<dyn Error>>
{
    let cases: CaseFile = serde_json::from_str(NEO_CASES)?;
    let mut judged = 0;
    for (case, passed) in cases.neo_cases() {
        let snapshot = cases.snapshot(case.snapshot.as_deref().ok_or("a case names its tree")?)?;
        let printed = neo_text(&snapshot, &passed.passed()?);
        assert!(
            neo_text_as(&snapshot, &printed.form) == printed.text,
            "{} renders again in its form {:?} as it was",
            case.name,
            printed.form
        );
        judged += 1;
    }
    let seed = seed()?;
    let mut generator = Generator::new(seed);
    for index in 0..count("NEO_SNAPSHOT_DIFFERENTIAL_TREES", GATE_TREES)? {
        let snapshot: Snapshot = generator.snapshot();
        let printed = neo_text(&snapshot, &generator.passed());
        assert!(
            neo_text_as(&snapshot, &printed.form) == printed.text,
            "seed {seed:#x}, tree {index} renders again in its form {:?} as it was",
            printed.form
        );
        judged += 1;
    }
    assert!(judged > 0, "texts were rendered again");
    Ok(())
}

/// How what `case` kept differs from what Rust would keep for `text` in `form`, in words that hold
/// no page text; `None` when it is the same. A whole page keeps its text in its form; a locator's
/// snapshot keeps nothing.
fn kept_differs(case: &Case, text: &str, form: Form) -> Result<Option<String>, String> {
    Ok(match (&case.pending, case.keeps_pending()) {
        (Some(pending), true) if pending.text != text => {
            Some("the kept text is not the text returned".to_owned())
        }
        (Some(pending), true) => {
            let kept = pending.form()?;
            (kept != form).then(|| format!("the form {form:?} printed, {kept:?} kept"))
        }
        (None, false) => None,
        (Some(_), false) => Some("a locator's snapshot kept a baseline".to_owned()),
        (None, true) => Some("a whole page kept no baseline".to_owned()),
    })
}
