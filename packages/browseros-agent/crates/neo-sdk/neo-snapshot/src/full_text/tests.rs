//! `mode: 'full'` in Rust against its judges (`crate::judges`), through `render` with
//! `Call::Full`: the committed cases; the private sets of saved pages, where a folder names them;
//! and today's TypeScript renderer in QuickJS on seeded random trees, at every cap the prelude
//! accepts and on pages long enough to be cut at the default cap.

use std::error::Error;
use std::{env, fs};

use crate::call::Call;
use crate::cap::{DEFAULT_MAX_TOKENS, MaxTokens};
use crate::form::Passed;
use crate::judges::{Bundles, CaseFile, Generator, NEO_CASES, Shown, count, seed};
use crate::render::render;

/// How many random trees the gate's run draws.
const GATE_TREES: usize = 300;

fn full(call: &Call<'_>) -> bool {
    matches!(call, Call::Full { .. })
}

#[test]
fn tests_that_mode_full_prints_every_committed_case() -> Result<(), Box<dyn Error>> {
    let cases: CaseFile = serde_json::from_str(NEO_CASES)?;
    let mut judged = 0;
    let mut differing = Vec::new();
    for found in cases.rendered_cases(full) {
        let (case, call, name) = found?;
        let answer = render(&cases.snapshot(name)?, call);
        if let Some(difference) = case.expected().difference(&call, &answer, Shown::Lines)? {
            differing.push(format!("{}: {difference}", case.name));
        }
        judged += 1;
    }
    assert!(
        judged > 0,
        "neo.json holds mode: 'full' cases, or this test judges nothing"
    );
    assert!(
        differing.is_empty(),
        "{} of {judged} mode: 'full' cases differ from the committed text:\n{}",
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
fn tests_that_mode_full_prints_every_private_case() -> Result<(), Box<dyn Error>> {
    let Some(folders) = env::var_os("NEO_SNAPSHOT_PRIVATE_CASES") else {
        println!(
            "private sets, mode: 'full': SKIPPED (NEO_SNAPSHOT_PRIVATE_CASES names no folder)"
        );
        return Ok(());
    };
    let bundles = Bundles::new()?;
    let mut differing: Vec<String> = Vec::new();
    let (mut texts, mut texts_identical, mut trees, mut trees_identical) = (0, 0, 0, 0);
    // Every saved tree at the cap a script that passes none gets.
    let by_default = Call::Full { max_tokens: None };
    for file in &CaseFile::private_files(&folders)? {
        let shown = file.file_name().unwrap_or_default().to_string_lossy();
        let cases: CaseFile = serde_json::from_str(&fs::read_to_string(file)?)?;
        let (mut file_texts, mut file_texts_identical) = (0, 0);
        for (index, found) in cases.rendered_cases(full).enumerate() {
            let (case, call, name) = found?;
            let answer = render(&cases.snapshot(name)?, call);
            match case.expected().difference(&call, &answer, Shown::Lengths)? {
                None => file_texts_identical += 1,
                Some(difference) => {
                    differing.push(format!("{shown}: mode: 'full' case {index}: {difference}"));
                }
            }
            file_texts += 1;
        }
        let (mut file_trees, mut file_trees_identical) = (0, 0);
        for (index, name) in cases.snapshot_names().enumerate() {
            let snapshot = cases.snapshot(name)?;
            let answer = render(&snapshot, by_default);
            let todays = bundles.todays(&by_default, &snapshot)?;
            match todays
                .expected()
                .difference(&by_default, &answer, Shown::Lengths)?
            {
                None => file_trees_identical += 1,
                Some(difference) => differing.push(format!(
                    "{shown}: tree {index}, against today's renderer: {difference}"
                )),
            }
            file_trees += 1;
        }
        println!(
            "private sets, mode: 'full': {shown}: {file_texts_identical} of {file_texts} saved \
             texts identical; {file_trees_identical} of {file_trees} trees print today's \
             renderer's text at the default cap"
        );
        texts += file_texts;
        texts_identical += file_texts_identical;
        trees += file_trees;
        trees_identical += file_trees_identical;
    }
    println!(
        "private sets, mode: 'full': {texts_identical} of {texts} identical, \
         {trees_identical} of {trees} trees as today's renderer prints them"
    );
    assert!(
        trees > 0,
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

#[test]
fn tests_that_mode_full_is_todays_on_generated_trees() -> Result<(), Box<dyn Error>> {
    let seed = seed()?;
    let bundles = Bundles::new()?;
    let mut generator = Generator::new(seed);
    for index in 0..count("NEO_SNAPSHOT_DIFFERENTIAL_TREES", GATE_TREES)? {
        let snapshot = generator.full_snapshot();
        let call = Call::Full {
            max_tokens: generator.max_tokens(),
        };
        let answer = render(&snapshot, call);
        let todays = bundles.todays(&call, &snapshot)?;
        if let Some(difference) = todays.expected().difference(&call, &answer, Shown::Lines)? {
            panic!("seed {seed:#x}, tree {index}, {call:?}: {difference}\n{snapshot:?}");
        }
    }
    Ok(())
}

/// A cap the script left out reaches the crate as `None`, and the crate cuts at
/// `DEFAULT_MAX_TOKENS`, its one owner: in `mode: 'full'`, and in the form a whole page in Neo's
/// format keeps. The generated pages long enough to pass it are cut, so the default is met.
#[test]
fn tests_that_a_cap_left_out_is_the_default_cap() -> Result<(), Box<dyn Error>> {
    let default = MaxTokens::new(f64::from(DEFAULT_MAX_TOKENS)).ok_or("the default is a cap")?;
    assert_eq!(
        MaxTokens::default(),
        default,
        "the crate's default is 8,000 tokens"
    );
    let seed = seed()?;
    let mut generator = Generator::new(seed);
    let mut cut = 0;
    for index in 0..count("NEO_SNAPSHOT_DIFFERENTIAL_TREES", GATE_TREES)? {
        let snapshot = generator.full_snapshot();
        let left_out = render(&snapshot, Call::Full { max_tokens: None });
        let passed = render(
            &snapshot,
            Call::Full {
                max_tokens: Some(default),
            },
        );
        assert!(
            left_out.text() == passed.text(),
            "seed {seed:#x}, tree {index}: a cap left out prints as 8,000 tokens"
        );
        if left_out.text().contains("# cut: ") {
            cut += 1;
        }
        let page = render(&snapshot, Call::Page(&Passed::default()));
        let kept = page.pending().ok_or("a whole page keeps itself")?;
        assert_eq!(
            kept.form.max_tokens, default,
            "seed {seed:#x}, tree {index}: a whole page keeps the default cap in its form"
        );
    }
    assert!(
        cut > 0,
        "some generated pages pass the default cap and are cut"
    );
    Ok(())
}
