//! Playwright's text in Rust against its judges: the committed cases; the private sets of saved
//! pages, where a folder names them; and, in QuickJS on seeded random nodes and trees, upstream's
//! printer and today's TypeScript renderer.
//!
//! The random runs are small here, as the gate runs them on every change. A long run sets
//! `NEO_SNAPSHOT_DIFFERENTIAL_NODES`, `NEO_SNAPSHOT_DIFFERENTIAL_TREES` and
//! `NEO_SNAPSHOT_DIFFERENTIAL_SEED` (decimal, or hex after `0x`).

mod bundles;
mod case_file;
mod generator;

use std::error::Error;
use std::path::PathBuf;
use std::{env, fs};

use serde_json::json;

use crate::aria_node::AriaNode;
use crate::snapshot::Snapshot;

use self::bundles::Bundles;
use self::case_file::CaseFile;
use self::generator::Generator;
use super::{NodeAlone, node_text, tree_text};

/// Upstream's printer's own text on trees with no Neo fact; kept for as long as the vendored
/// printer is, and rewritten from it at every engine upgrade.
const PLAYWRIGHT_CASES: &str = include_str!("../../tests/data/cases/playwright.json");

/// Every other case, by today's renderer, Playwright's format on trees with Neo's facts among them.
const NEO_CASES: &str = include_str!("../../tests/data/cases/neo.json");

/// The seed of the random runs unless `NEO_SNAPSHOT_DIFFERENTIAL_SEED` names another.
const SEED: u64 = 0x5eed_7a11_0000_0002;

/// How many random nodes and trees the gate's run draws.
const GATE_NODES: usize = 3_000;
const GATE_TREES: usize = 300;

#[test]
fn tests_that_playwrights_format_prints_every_committed_case() -> Result<(), Box<dyn Error>> {
    for (file, text) in [
        ("playwright.json", PLAYWRIGHT_CASES),
        ("neo.json", NEO_CASES),
    ] {
        let cases: CaseFile = serde_json::from_str(text)?;
        let mut printed = 0;
        let mut differing = Vec::new();
        for case in cases.playwright_cases() {
            let name = case
                .snapshot
                .as_deref()
                .ok_or("a Playwright case names its snapshot")?;
            let rust = tree_text(&cases.snapshot(name)?);
            let expected = case.text.as_deref().ok_or("a Playwright case has a text")?;
            if let Some((line, printed_line, expected_line)) = first_difference(&rust, expected) {
                differing.push(format!(
                    "{}: line {line}\n  printed:  {printed_line:?}\n  expected: {expected_line:?}",
                    case.name
                ));
            }
            printed += 1;
        }
        assert!(
            printed > 0,
            "{file} holds Playwright-format cases, or this test judges nothing"
        );
        assert!(
            differing.is_empty(),
            "{file}: {} of {printed} Playwright-format cases differ from the committed text:\n{}",
            differing.len(),
            differing.join("\n")
        );
    }
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
fn tests_that_playwrights_format_prints_every_private_case() -> Result<(), Box<dyn Error>> {
    let Some(folders) = env::var_os("NEO_SNAPSHOT_PRIVATE_CASES") else {
        println!("private sets: SKIPPED (NEO_SNAPSHOT_PRIVATE_CASES names no folder)");
        return Ok(());
    };
    let bundles = Bundles::new()?;
    let mut differing: Vec<String> = Vec::new();
    let (mut texts, mut texts_identical, mut trees, mut trees_identical) = (0, 0, 0, 0);
    for folder in env::split_paths(&folders) {
        let mut files: Vec<PathBuf> = Vec::new();
        for entry in fs::read_dir(&folder)? {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                files.push(path);
            }
        }
        files.sort();
        for file in &files {
            let shown = file.file_name().unwrap_or_default().to_string_lossy();
            let cases: CaseFile = serde_json::from_str(&fs::read_to_string(file)?)?;
            let (mut file_texts, mut file_texts_identical) = (0, 0);
            for (index, case) in cases.playwright_cases().enumerate() {
                let name = case
                    .snapshot
                    .as_deref()
                    .ok_or("a Playwright case names its snapshot")?;
                let rust = tree_text(&cases.snapshot(name)?);
                let expected = case.text.as_deref().ok_or("a Playwright case has a text")?;
                match first_difference(&rust, expected) {
                    None => file_texts_identical += 1,
                    Some((line, _, _)) => differing.push(format!(
                        "{shown}: Playwright-format case {index}: line {line} differs; {} bytes \
                         printed, {} saved",
                        rust.len(),
                        expected.len()
                    )),
                }
                file_texts += 1;
            }
            // Every saved tree, not only those a Playwright-format call was saved for: Rust's
            // text beside today's renderer's, in QuickJS.
            let (mut file_trees, mut file_trees_identical) = (0, 0);
            for (index, name) in cases.snapshot_names().enumerate() {
                let snapshot = cases.snapshot(name)?;
                let rust = tree_text(&snapshot);
                let todays = bundles.todays_playwright_text(&snapshot)?;
                match first_difference(&rust, &todays) {
                    None => file_trees_identical += 1,
                    Some((line, _, _)) => differing.push(format!(
                        "{shown}: tree {index}: line {line} differs; {} bytes printed, {} by \
                         today's renderer",
                        rust.len(),
                        todays.len()
                    )),
                }
                file_trees += 1;
            }
            println!(
                "private sets: {shown}: {file_texts_identical} of {file_texts} saved \
                 Playwright-format texts identical; {file_trees_identical} of {file_trees} trees \
                 print today's renderer's text"
            );
            texts += file_texts;
            texts_identical += file_texts_identical;
            trees += file_trees;
            trees_identical += file_trees_identical;
        }
    }
    println!(
        "private sets: {texts_identical} of {texts} identical (Playwright's format), \
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
fn tests_that_each_generated_node_prints_upstreams_own_lines() -> Result<(), Box<dyn Error>> {
    let seed = seed()?;
    let bundles = Bundles::new()?;
    let mut generator = Generator::new(seed);
    for index in 0..count("NEO_SNAPSHOT_DIFFERENTIAL_NODES", GATE_NODES)? {
        let node = generator.node();
        let mut rust = String::new();
        node_text(&NodeAlone::from(&node), &mut rust);
        assert_eq!(
            rust,
            bundles.vendored_node_text(&node)?,
            "seed {seed:#x}, node {index} prints upstream's own text for it: {}",
            serde_json::to_string(&node)?
        );
    }
    Ok(())
}

#[test]
fn tests_that_playwrights_format_is_todays_on_generated_trees() -> Result<(), Box<dyn Error>> {
    let seed = seed()?;
    let bundles = Bundles::new()?;
    let mut generator = Generator::new(seed);
    for index in 0..count("NEO_SNAPSHOT_DIFFERENTIAL_TREES", GATE_TREES)? {
        let snapshot = generator.snapshot();
        assert_eq!(
            tree_text(&snapshot),
            bundles.todays_playwright_text(&snapshot)?,
            "seed {seed:#x}, tree {index} prints today's renderer's text: {snapshot:?}"
        );
    }
    Ok(())
}

/// The one place today's renderer and upstream's whole-tree text part, which the differential's
/// trees reach only by chance: a role or a ref with a line break. No page writes one, but the
/// tree's type admits it, and Rust prints today's text.
#[test]
fn tests_that_a_line_break_in_a_key_moves_its_colon_as_todays_renderer_does()
-> Result<(), Box<dyn Error>> {
    let tree: Vec<AriaNode> = serde_json::from_value(json!([{
        "role": "button",
        "ref": "a\nb",
        "children": [{ "role": "img" }],
    }]))?;
    let snapshot = Snapshot {
        tree,
        partial: Vec::new(),
    };
    let todays = "- 'button [ref=a:\nb]'\n  - img";
    assert_eq!(
        tree_text(&snapshot),
        todays,
        "the key's first line takes the colon, as the TypeScript splits a node's text at line breaks"
    );
    let bundles = Bundles::new()?;
    assert_eq!(
        bundles.todays_playwright_text(&snapshot)?,
        todays,
        "today's renderer prints the same"
    );
    assert_eq!(
        bundles.vendored_text(&snapshot.tree)?,
        "- 'button [ref=a\nb]':\n  - img",
        "upstream's own walk keeps the colon at the end of the quoted key"
    );
    Ok(())
}

/// Where `printed` first differs from `expected`: the line's number from 1, and that line of each
/// (empty past a text's end). `None` when they are equal.
fn first_difference<'a>(printed: &'a str, expected: &'a str) -> Option<(usize, &'a str, &'a str)> {
    let mut printed_lines = printed.split('\n');
    let mut expected_lines = expected.split('\n');
    let mut line = 0;
    loop {
        line += 1;
        match (printed_lines.next(), expected_lines.next()) {
            (None, None) => return None,
            (Some(left), Some(right)) if left == right => {}
            (left, right) => {
                return Some((line, left.unwrap_or_default(), right.unwrap_or_default()));
            }
        }
    }
}

fn seed() -> Result<u64, Box<dyn Error>> {
    let Ok(text) = env::var("NEO_SNAPSHOT_DIFFERENTIAL_SEED") else {
        return Ok(SEED);
    };
    Ok(match text.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16)?,
        None => text.parse()?,
    })
}

fn count(variable: &str, gate: usize) -> Result<usize, Box<dyn Error>> {
    match env::var(variable) {
        Ok(text) => Ok(text.parse()?),
        Err(_) => Ok(gate),
    }
}
